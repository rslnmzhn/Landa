use std::convert::Infallible;
use std::io::SeekFrom;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, Limited};
use hyper::body::{Body, Frame, Incoming};
use hyper::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, COOKIE, LOCATION,
    RANGE, SET_COOKIE,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use landa_core::network::enumerate_lan_interfaces;
use landa_core::{CoreError, LandaModule, OpenSourceHook, PeerContext};
use landa_proto::wire::StreamKind;
use quinn::{RecvStream, SendStream};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::debug;

pub const DEFAULT_VIDEOLINK_PORT: u16 = 40444;
pub const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_FORM_BODY_BYTES: usize = 4 * 1024;
const MAX_PASSWORD_BYTES: usize = 256;
const STREAM_CHUNK_BYTES: usize = 64 * 1024;
const SESSION_TTL_SECS: u64 = 12 * 60 * 60;
const SESSION_COOKIE_NAME: &str = "landa_vl_session";

type RespBody = BoxBody<Bytes, std::io::Error>;

#[derive(Debug, Clone)]
pub enum VideoSource {
    File(PathBuf),
    ContentUri {
        uri: String,
        file_name: String,
        size_bytes: u64,
        mime_type: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoLinkStatus {
    pub running: bool,
    pub url: Option<String>,
    pub port: Option<u16>,
    pub file_name: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Clone)]
struct ServerSessionState {
    source: VideoSource,
    file_name: String,
    mime_type: String,
    size_bytes: u64,
    password_hash: [u8; 32],
    cookie_secret: [u8; 32],
    open_source_hook: Option<OpenSourceHook>,
}

struct ActiveServer {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    bound_addr: SocketAddr,
}

struct LifecycleState {
    enabled: bool,
    active_cancel: Option<CancellationToken>,
    status: VideoLinkStatus,
}

impl Default for LifecycleState {
    fn default() -> Self {
        Self {
            enabled: true,
            active_cancel: None,
            status: VideoLinkStatus::default(),
        }
    }
}

pub struct VideoLinkModule {
    open_source_hook: RwLock<Option<OpenSourceHook>>,
    active: Mutex<Option<ActiveServer>>,
    lifecycle: StdMutex<LifecycleState>,
}

impl Default for VideoLinkModule {
    fn default() -> Self {
        Self {
            open_source_hook: RwLock::new(None),
            active: Mutex::new(None),
            lifecycle: StdMutex::new(LifecycleState::default()),
        }
    }
}

impl VideoLinkModule {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_open_source_hook(&self, hook: OpenSourceHook) {
        *self
            .open_source_hook
            .write()
            .expect("open_source_hook lock poisoned") = Some(hook);
    }

    pub fn status(&self) -> VideoLinkStatus {
        self.lifecycle
            .lock()
            .expect("videolink lifecycle lock poisoned")
            .status
            .clone()
    }

    pub async fn start(
        &self,
        source: VideoSource,
        password: String,
    ) -> Result<VideoLinkStatus, CoreError> {
        self.start_with_bind(Ipv4Addr::UNSPECIFIED, DEFAULT_VIDEOLINK_PORT, source, password)
            .await
    }

    pub async fn start_with_bind(
        &self,
        bind_ip: Ipv4Addr,
        preferred_port: u16,
        source: VideoSource,
        password: String,
    ) -> Result<VideoLinkStatus, CoreError> {
        if password.trim().is_empty() {
            return Err(CoreError::InvalidInput(
                "video link password cannot be empty".into(),
            ));
        }
        if password.len() > MAX_PASSWORD_BYTES {
            return Err(CoreError::InvalidInput(
                "video link password is too long".into(),
            ));
        }

        // Serialize all start/stop transitions so concurrent starts can never orphan a listener.
        let mut active_guard = self.active.lock().await;

        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("videolink".into()));
        }

        // Stop any previously running server before binding a new one.
        if let Some(prev) = active_guard.take() {
            prev.cancel.cancel();
            let _ = prev.task.await;
        }
        {
            let mut lc = self
                .lifecycle
                .lock()
                .expect("videolink lifecycle lock poisoned");
            lc.active_cancel = None;
            lc.status = VideoLinkStatus::default();
            if !lc.enabled {
                return Err(CoreError::ModuleDisabled("videolink".into()));
            }
        }

        let (file_name, mime_type, size_bytes, resolved_source) = match source {
            VideoSource::File(path) => {
                let canonical = tokio::fs::canonicalize(&path).await.map_err(|_| {
                    CoreError::NotFound(format!("video file not found: {}", path.display()))
                })?;
                let meta = tokio::fs::metadata(&canonical).await?;
                if !meta.is_file() {
                    return Err(CoreError::InvalidInput(
                        "selected path is not a regular file".into(),
                    ));
                }
                let name = canonical
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("video.mp4")
                    .to_string();
                let mime = detect_video_mime(&name, None);
                (name, mime, meta.len(), VideoSource::File(canonical))
            }
            VideoSource::ContentUri {
                uri,
                file_name,
                size_bytes,
                mime_type,
            } => {
                if !uri.starts_with("content://") {
                    return Err(CoreError::InvalidInput("invalid content URI".into()));
                }
                let clean_name = if file_name.trim().is_empty() {
                    "video.mp4".to_string()
                } else {
                    file_name
                };
                let mime = detect_video_mime(&clean_name, mime_type.as_deref());
                (
                    clean_name.clone(),
                    mime,
                    size_bytes,
                    VideoSource::ContentUri {
                        uri,
                        file_name: clean_name,
                        size_bytes,
                        mime_type,
                    },
                )
            }
        };

        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("videolink".into()));
        }

        let listener = match TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(
            bind_ip,
            preferred_port,
        )))
        .await
        {
            Ok(l) => l,
            Err(_) if preferred_port != 0 => {
                TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(bind_ip, 0))).await?
            }
            Err(err) => return Err(CoreError::Io(err)),
        };

        let bound_addr = listener.local_addr()?;
        let port = bound_addr.port();

        let mut cookie_secret = [0u8; 32];
        OsRng.fill_bytes(&mut cookie_secret);
        let password_hash = *blake3::hash(password.as_bytes()).as_bytes();
        let hook = self
            .open_source_hook
            .read()
            .expect("open_source_hook lock poisoned")
            .clone();

        let session_state = Arc::new(ServerSessionState {
            source: resolved_source,
            file_name: file_name.clone(),
            mime_type,
            size_bytes,
            password_hash,
            cookie_secret,
            open_source_hook: hook,
        });

        let host_ip = if bind_ip.is_unspecified() {
            resolve_lan_ipv4().unwrap_or(Ipv4Addr::LOCALHOST)
        } else {
            bind_ip
        };
        let url = format!("http://{host_ip}:{port}/watch");

        let status = VideoLinkStatus {
            running: true,
            url: Some(url),
            port: Some(port),
            file_name: Some(file_name),
            size_bytes: Some(size_bytes),
        };

        let cancel = CancellationToken::new();
        {
            let mut lc = self
                .lifecycle
                .lock()
                .expect("videolink lifecycle lock poisoned");
            if !lc.enabled {
                drop(listener);
                return Err(CoreError::ModuleDisabled("videolink".into()));
            }
            lc.active_cancel = Some(cancel.clone());
            lc.status = status.clone();
        }

        let loop_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            run_http_server(listener, session_state, loop_cancel).await;
        });

        *active_guard = Some(ActiveServer {
            cancel,
            task,
            bound_addr,
        });

        Ok(status)
    }

    pub async fn stop(&self) -> VideoLinkStatus {
        let mut active_guard = self.active.lock().await;
        if let Some(server) = active_guard.take() {
            server.cancel.cancel();
            let _ = server.task.await;
        }

        let mut lc = self
            .lifecycle
            .lock()
            .expect("videolink lifecycle lock poisoned");
        if let Some(cancel) = lc.active_cancel.take() {
            cancel.cancel();
        }
        lc.status = VideoLinkStatus::default();
        lc.status.clone()
    }

    pub async fn bound_addr(&self) -> Option<SocketAddr> {
        self.active
            .lock()
            .await
            .as_ref()
            .map(|s| s.bound_addr)
    }
}

impl Drop for VideoLinkModule {
    fn drop(&mut self) {
        let mut lc = self
            .lifecycle
            .lock()
            .expect("videolink lifecycle lock poisoned");
        if let Some(cancel) = lc.active_cancel.take() {
            cancel.cancel();
        }
    }
}

#[async_trait]
impl LandaModule for VideoLinkModule {
    fn id(&self) -> &'static str {
        "videolink"
    }

    fn stream_kinds(&self) -> &'static [StreamKind] {
        &[]
    }

    fn requires_trusted_peer(&self) -> bool {
        false
    }

    fn is_enabled(&self) -> bool {
        self.lifecycle
            .lock()
            .expect("videolink lifecycle lock poisoned")
            .enabled
    }

    fn set_enabled(&self, enabled: bool) {
        let mut lc = self
            .lifecycle
            .lock()
            .expect("videolink lifecycle lock poisoned");
        lc.enabled = enabled;
        if !enabled {
            if let Some(cancel) = lc.active_cancel.take() {
                cancel.cancel();
            }
            lc.status = VideoLinkStatus::default();
        }
    }

    async fn handle_stream(
        &self,
        _peer: PeerContext,
        _kind: StreamKind,
        _send: SendStream,
        _recv: RecvStream,
    ) -> Result<(), CoreError> {
        Err(CoreError::InvalidInput(
            "videolink does not accept QUIC streams".into(),
        ))
    }
}

fn resolve_lan_ipv4() -> Option<Ipv4Addr> {
    enumerate_lan_interfaces().into_iter().map(|i| i.ip).next()
}

async fn run_http_server(
    listener: TcpListener,
    state: Arc<ServerSessionState>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            }
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    break;
                };
                let st = Arc::clone(&state);
                let conn_cancel = cancel.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let mut builder = http1::Builder::new();
                    builder
                        .timer(TokioTimer::new())
                        .max_header_size(MAX_HTTP_HEADER_BYTES)
                        .max_buf_size(MAX_HTTP_HEADER_BYTES)
                        .keep_alive(true);

                    let svc = service_fn(move |req| {
                        let st = Arc::clone(&st);
                        async move { handle_http_request(req, st).await }
                    });

                    let mut conn = builder.serve_connection(io, svc);
                    tokio::select! {
                        _ = conn_cancel.cancelled() => {
                            Pin::new(&mut conn).graceful_shutdown();
                        }
                        res = &mut conn => {
                            if let Err(err) = res {
                                debug!("videolink hyper connection error: {err}");
                            }
                        }
                    }
                });
            }
        }
    }
    drop(listener);
}

struct ChannelBody {
    rx: mpsc::Receiver<Result<Frame<Bytes>, std::io::Error>>,
}

impl Body for ChannelBody {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        self.rx.poll_recv(cx)
    }
}

fn full_body(data: impl Into<Bytes>) -> RespBody {
    Full::new(data.into())
        .map_err(|never| match never {})
        .boxed()
}

fn empty_body() -> RespBody {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn text_response(status: StatusCode, body: &'static str) -> Response<RespBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(CACHE_CONTROL, "no-store")
        .body(full_body(body))
        .expect("valid text response")
}

fn html_response(status: StatusCode, html: String, head_only: bool) -> Response<RespBody> {
    let len = html.len();
    let body = if head_only {
        empty_body()
    } else {
        full_body(html)
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CONTENT_LENGTH, len.to_string())
        .header(CACHE_CONTROL, "no-store")
        .body(body)
        .expect("valid html response")
}

fn redirect_response(
    status: StatusCode,
    location: &'static str,
    set_cookie: Option<&str>,
) -> Response<RespBody> {
    let mut builder = Response::builder()
        .status(status)
        .header(LOCATION, location)
        .header(CACHE_CONTROL, "no-store");
    if let Some(cookie) = set_cookie {
        builder = builder.header(SET_COOKIE, cookie);
    }
    builder.body(empty_body()).expect("valid redirect response")
}

async fn handle_http_request(
    req: Request<Incoming>,
    state: Arc<ServerSessionState>,
) -> Result<Response<RespBody>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let cookie_header = req
        .headers()
        .get(COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let range_header = req
        .headers()
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let is_authenticated = cookie_header
        .as_deref()
        .and_then(|hdr| extract_cookie(hdr, SESSION_COOKIE_NAME))
        .map(|token| verify_session_token(&state.cookie_secret, token))
        .unwrap_or(false);

    let resp = match (method.clone(), path.as_str()) {
        (Method::GET | Method::HEAD, "/") => redirect_response(StatusCode::FOUND, "/watch", None),
        (Method::GET | Method::HEAD, "/watch") => {
            let head_only = method == Method::HEAD;
            if is_authenticated {
                html_response(StatusCode::OK, render_player_html(&state.file_name), head_only)
            } else {
                html_response(
                    StatusCode::OK,
                    render_login_html(&state.file_name, false),
                    head_only,
                )
            }
        }
        (Method::POST, "/login") => {
            let limited = Limited::new(req.into_body(), MAX_FORM_BODY_BYTES);
            let collected = match limited.collect().await {
                Ok(c) => c.to_bytes(),
                Err(_) => {
                    return Ok(text_response(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "Payload too large",
                    ));
                }
            };
            let body_str = String::from_utf8_lossy(&collected);
            let submitted = extract_form_field(&body_str, "password").unwrap_or_default();
            let submitted_hash = *blake3::hash(submitted.as_bytes()).as_bytes();

            if constant_time_eq(&submitted_hash, &state.password_hash) {
                let token = issue_session_token(&state.cookie_secret);
                let cookie = format!(
                    "{SESSION_COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECS}"
                );
                redirect_response(StatusCode::SEE_OTHER, "/watch", Some(&cookie))
            } else {
                html_response(
                    StatusCode::UNAUTHORIZED,
                    render_login_html(&state.file_name, true),
                    false,
                )
            }
        }
        (Method::POST, "/logout") => {
            let clear_cookie =
                format!("{SESSION_COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
            redirect_response(StatusCode::SEE_OTHER, "/watch", Some(&clear_cookie))
        }
        (Method::GET | Method::HEAD, "/stream") => {
            if !is_authenticated {
                text_response(StatusCode::UNAUTHORIZED, "Unauthorized")
            } else {
                serve_video_stream(&state, range_header.as_deref(), method == Method::HEAD).await
            }
        }
        (Method::GET | Method::HEAD | Method::POST, _) => {
            text_response(StatusCode::NOT_FOUND, "Not found")
        }
        _ => text_response(StatusCode::METHOD_NOT_ALLOWED, "Method not allowed"),
    };

    Ok(resp)
}

async fn serve_video_stream(
    state: &ServerSessionState,
    range_header: Option<&str>,
    head_only: bool,
) -> Response<RespBody> {
    let mut file = match &state.source {
        VideoSource::File(path) => match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(_) => return text_response(StatusCode::NOT_FOUND, "Video file unavailable"),
        },
        VideoSource::ContentUri { uri, .. } => {
            let Some(ref hook) = state.open_source_hook else {
                return text_response(StatusCode::INTERNAL_SERVER_ERROR, "Source hook missing");
            };
            let uri_owned = uri.clone();
            let hook_clone = Arc::clone(hook);
            let std_file = tokio::task::spawn_blocking(move || hook_clone(&uri_owned))
                .await
                .ok()
                .flatten();
            let Some(std_file) = std_file else {
                return text_response(StatusCode::NOT_FOUND, "Video URI unavailable");
            };
            tokio::fs::File::from_std(std_file)
        }
    };

    let total = if state.size_bytes > 0 {
        state.size_bytes
    } else {
        file.metadata().await.map(|m| m.len()).unwrap_or(0)
    };

    if total == 0 {
        return Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, state.mime_type.as_str())
            .header(CONTENT_LENGTH, "0")
            .header(ACCEPT_RANGES, "bytes")
            .header(CACHE_CONTROL, "no-store")
            .body(empty_body())
            .expect("valid empty stream response");
    }

    let (status, start, end) = if let Some(range_val) = range_header {
        match parse_byte_range(range_val, total) {
            Some((s, e)) => (StatusCode::PARTIAL_CONTENT, s, e),
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(CONTENT_RANGE, format!("bytes */{total}"))
                    .header(CACHE_CONTROL, "no-store")
                    .body(empty_body())
                    .expect("valid 416 response");
            }
        }
    } else {
        (StatusCode::OK, 0, total - 1)
    };

    let content_len = end - start + 1;
    let mut builder = Response::builder()
        .status(status)
        .header(CONTENT_TYPE, state.mime_type.as_str())
        .header(CONTENT_LENGTH, content_len.to_string())
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "no-store");

    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(CONTENT_RANGE, format!("bytes {start}-{end}/{total}"));
    }

    if head_only {
        return builder.body(empty_body()).expect("valid HEAD stream response");
    }

    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return text_response(StatusCode::INTERNAL_SERVER_ERROR, "Seek failed");
    }

    let (tx, rx) = mpsc::channel::<Result<Frame<Bytes>, std::io::Error>>(4);
    tokio::spawn(async move {
        let mut remaining = content_len;
        let mut buf = vec![0u8; STREAM_CHUNK_BYTES];
        while remaining > 0 {
            let to_read = (remaining as usize).min(buf.len());
            match file.read(&mut buf[..to_read]).await {
                Ok(0) => break,
                Ok(n) => {
                    remaining -= n as u64;
                    let chunk = Bytes::copy_from_slice(&buf[..n]);
                    if tx.send(Ok(Frame::data(chunk))).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });

    let body = ChannelBody { rx }.boxed();
    builder.body(body).expect("valid stream response")
}

fn issue_session_token(secret: &[u8; 32]) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let exp = now.saturating_add(SESSION_TTL_SECS);
    let mut nonce = [0u8; 16];
    OsRng.fill_bytes(&mut nonce);
    let nonce_hex = hex_encode(&nonce);
    let payload = format!("{exp}.{nonce_hex}");
    let mac = blake3::keyed_hash(secret, payload.as_bytes());
    let mac_hex = hex_encode(mac.as_bytes());
    format!("{payload}.{mac_hex}")
}

fn verify_session_token(secret: &[u8; 32], token: &str) -> bool {
    let mut parts = token.split('.');
    let Some(exp_str) = parts.next() else {
        return false;
    };
    let Some(nonce_hex) = parts.next() else {
        return false;
    };
    let Some(mac_hex) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let Ok(exp) = exp_str.parse::<u64>() else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if exp < now {
        return false;
    }
    let Some(provided_mac) = hex_decode_32(mac_hex) else {
        return false;
    };
    let payload = format!("{exp_str}.{nonce_hex}");
    let expected_mac = blake3::keyed_hash(secret, payload.as_bytes());
    constant_time_eq(&provided_mac, expected_mac.as_bytes())
}

fn extract_cookie<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    for part in header.split(';') {
        let trimmed = part.trim();
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim() == name {
                return Some(v.trim());
            }
        }
    }
    None
}

fn extract_form_field(body: &str, field: &str) -> Option<String> {
    for pair in body.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if url_decode(k) == field {
                return Some(url_decode(v));
            }
        }
    }
    None
}

fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn hex_decode_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let (chunks, _) = hex.as_bytes().as_chunks::<2>();
    for (i, chunk) in chunks.iter().enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

fn constant_time_eq(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for i in 0..32 {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn parse_byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start_str, end_str) = spec.split_once('-')?;
    let start_str = start_str.trim();
    let end_str = end_str.trim();

    if start_str.is_empty() {
        let suffix: u64 = end_str.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }

    let start: u64 = start_str.parse().ok()?;
    if start >= total {
        return None;
    }
    let end = if end_str.is_empty() {
        total - 1
    } else {
        let parsed_end: u64 = end_str.parse().ok()?;
        if parsed_end < start {
            return None;
        }
        parsed_end.min(total - 1)
    };
    Some((start, end))
}

fn detect_video_mime(file_name: &str, hint: Option<&str>) -> String {
    if let Some(h) = hint {
        let trimmed = h.trim();
        if trimmed.starts_with("video/") {
            return trimmed.to_string();
        }
    }
    let ext = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "ogv" => "video/ogg",
        "ts" | "m2ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn render_login_html(file_name: &str, invalid_password: bool) -> String {
    let safe_name = escape_html(file_name);
    let error_block = if invalid_password {
        r#"<div class="err">Invalid password / Неверный пароль</div>"#
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Landa Video · {safe_name}</title>
<style>
  :root {{ color-scheme: dark; }}
  body {{ margin:0; min-height:100vh; display:flex; align-items:center; justify-content:center; padding:20px; font-family:-apple-system,BlinkMacSystemFont,"SF Pro Display","Segoe UI",sans-serif; background:radial-gradient(circle at 20% 20%,#1e1b4b,#070913 65%); color:#f8fafc; }}
  .card {{ width:100%; max-width:380px; padding:28px; border-radius:24px; background:rgba(255,255,255,0.08); border:1px solid rgba(255,255,255,0.18); backdrop-filter:blur(24px); box-shadow:0 20px 50px rgba(0,0,0,0.4); }}
  h1 {{ margin:0 0 6px; font-size:20px; font-weight:600; }}
  .sub {{ margin:0 0 18px; font-size:13px; color:rgba(255,255,255,0.65); word-break:break-word; }}
  .err {{ margin-bottom:14px; padding:10px 12px; border-radius:12px; background:rgba(244,63,94,0.2); border:1px solid rgba(244,63,94,0.4); color:#fecdd3; font-size:13px; }}
  input {{ width:100%; box-sizing:border-box; min-height:44px; padding:10px 14px; border-radius:14px; border:1px solid rgba(255,255,255,0.2); background:rgba(255,255,255,0.08); color:#fff; font-size:15px; outline:none; }}
  input:focus {{ border-color:#818cf8; }}
  button {{ margin-top:12px; width:100%; min-height:44px; border:0; border-radius:999px; background:linear-gradient(180deg,#818cf8,#4f46e5); color:#fff; font-size:15px; font-weight:600; cursor:pointer; }}
</style>
</head>
<body>
  <form class="card" method="POST" action="/login">
    <h1>Landa Video</h1>
    <p class="sub">{safe_name}</p>
    {error_block}
    <input type="password" name="password" placeholder="Password / Пароль" autocomplete="current-password" required autofocus>
    <button type="submit">Watch / Смотреть</button>
  </form>
</body>
</html>"#
    )
}

fn render_player_html(file_name: &str) -> String {
    let safe_name = escape_html(file_name);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Landa Video · {safe_name}</title>
<style>
  :root {{ color-scheme: dark; }}
  body {{ margin:0; min-height:100vh; display:flex; flex-direction:column; background:#05070f; color:#f8fafc; font-family:-apple-system,BlinkMacSystemFont,"SF Pro Display","Segoe UI",sans-serif; }}
  header {{ display:flex; align-items:center; justify-content:space-between; gap:12px; padding:14px 20px; background:rgba(255,255,255,0.05); border-bottom:1px solid rgba(255,255,255,0.12); }}
  .title {{ font-size:14px; font-weight:600; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }}
  .logout {{ min-height:36px; padding:0 14px; border-radius:999px; border:1px solid rgba(255,255,255,0.2); background:rgba(255,255,255,0.08); color:#fff; font-size:13px; cursor:pointer; }}
  main {{ flex:1; display:flex; align-items:center; justify-content:center; padding:16px; }}
  video {{ width:100%; max-width:1120px; max-height:82vh; border-radius:18px; background:#000; box-shadow:0 20px 60px rgba(0,0,0,0.6); }}
</style>
</head>
<body>
  <header>
    <div class="title">{safe_name}</div>
    <form method="POST" action="/logout" style="margin:0">
      <button class="logout" type="submit">Lock</button>
    </form>
  </header>
  <main>
    <video controls autoplay playsinline preload="metadata" src="/stream"></video>
  </main>
</body>
</html>"#
    )
}
