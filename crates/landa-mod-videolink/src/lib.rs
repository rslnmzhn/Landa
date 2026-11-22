use std::io::SeekFrom;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use landa_core::network::enumerate_lan_interfaces;
use landa_core::{CoreError, LandaModule, OpenSourceHook, PeerContext};
use landa_proto::wire::StreamKind;
use quinn::{RecvStream, SendStream};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::debug;

pub const DEFAULT_VIDEOLINK_PORT: u16 = 40444;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const MAX_FORM_BODY_BYTES: usize = 4 * 1024;
const MAX_PASSWORD_BYTES: usize = 256;
const STREAM_CHUNK_BYTES: usize = 64 * 1024;
const SESSION_TTL_SECS: u64 = 12 * 60 * 60;
const SESSION_COOKIE_NAME: &str = "landa_vl_session";

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

pub struct VideoLinkModule {
    enabled: AtomicBool,
    open_source_hook: RwLock<Option<OpenSourceHook>>,
    active: Mutex<Option<ActiveServer>>,
    active_cancel: RwLock<Option<CancellationToken>>,
    status_snapshot: RwLock<VideoLinkStatus>,
}

impl Default for VideoLinkModule {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            open_source_hook: RwLock::new(None),
            active: Mutex::new(None),
            active_cancel: RwLock::new(None),
            status_snapshot: RwLock::new(VideoLinkStatus::default()),
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
        self.status_snapshot
            .read()
            .expect("videolink status lock poisoned")
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
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("videolink".into()));
        }

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

        // Stop any previously running server before binding a new one.
        self.stop().await;

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

        let cancel = CancellationToken::new();
        let loop_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            run_http_server(listener, session_state, loop_cancel).await;
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

        {
            let mut active_guard = self.active.lock().await;
            *active_guard = Some(ActiveServer {
                cancel: cancel.clone(),
                task,
                bound_addr,
            });
        }
        *self
            .active_cancel
            .write()
            .expect("active_cancel lock poisoned") = Some(cancel);
        *self
            .status_snapshot
            .write()
            .expect("videolink status lock poisoned") = status.clone();

        Ok(status)
    }

    pub async fn stop(&self) -> VideoLinkStatus {
        let prev = {
            let mut active_guard = self.active.lock().await;
            active_guard.take()
        };
        *self
            .active_cancel
            .write()
            .expect("active_cancel lock poisoned") = None;

        if let Some(server) = prev {
            server.cancel.cancel();
            let _ = server.task.await;
        }

        let stopped = VideoLinkStatus::default();
        *self
            .status_snapshot
            .write()
            .expect("videolink status lock poisoned") = stopped.clone();
        stopped
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
        if let Some(cancel) = self
            .active_cancel
            .write()
            .expect("active_cancel lock poisoned")
            .take()
        {
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
        self.enabled.load(Ordering::Relaxed)
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        if !enabled {
            if let Some(cancel) = self
                .active_cancel
                .write()
                .expect("active_cancel lock poisoned")
                .take()
            {
                cancel.cancel();
            }
            *self
                .status_snapshot
                .write()
                .expect("videolink status lock poisoned") = VideoLinkStatus::default();
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
                    tokio::select! {
                        _ = conn_cancel.cancelled() => {}
                        res = handle_http_connection(stream, st) => {
                            if let Err(err) = res {
                                debug!("videolink connection error: {err}");
                            }
                        }
                    }
                });
            }
        }
    }
    drop(listener);
}

async fn handle_http_connection(
    stream: TcpStream,
    state: Arc<ServerSessionState>,
) -> Result<(), String> {
    let (reader_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader_half);

    let mut request_line = String::new();
    let n = reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(());
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let raw_path = parts.next().unwrap_or("");
    let path_only = raw_path.split('?').next().unwrap_or("");

    let mut header_bytes = n;
    let mut range_header: Option<String> = None;
    let mut cookie_header: Option<String> = None;
    let mut content_length: usize = 0;

    loop {
        let mut line = String::new();
        let read_n = reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
        if read_n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        header_bytes += read_n;
        if header_bytes > MAX_HTTP_HEADER_BYTES {
            write_text_response(
                &mut writer,
                "431 Request Header Fields Too Large",
                "Header too large",
            )
            .await;
            return Ok(());
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("range") {
                range_header = Some(val.to_string());
            } else if key.eq_ignore_ascii_case("cookie") {
                cookie_header = Some(val.to_string());
            } else if key.eq_ignore_ascii_case("content-length") {
                content_length = val.parse::<usize>().unwrap_or(usize::MAX);
            }
        }
    }

    let is_authenticated = cookie_header
        .as_deref()
        .and_then(|hdr| extract_cookie(hdr, SESSION_COOKIE_NAME))
        .map(|token| verify_session_token(&state.cookie_secret, token))
        .unwrap_or(false);

    match (method, path_only) {
        ("GET", "/") | ("HEAD", "/") => {
            write_redirect(&mut writer, "302 Found", "/watch", None).await;
        }
        ("GET", "/watch") | ("HEAD", "/watch") => {
            if is_authenticated {
                let html = render_player_html(&state.file_name);
                write_html_response(&mut writer, "200 OK", &html, method == "HEAD").await;
            } else {
                let html = render_login_html(&state.file_name, false);
                write_html_response(&mut writer, "200 OK", &html, method == "HEAD").await;
            }
        }
        ("POST", "/login") => {
            if content_length > MAX_FORM_BODY_BYTES {
                write_text_response(&mut writer, "413 Payload Too Large", "Payload too large").await;
                return Ok(());
            }
            let mut body = vec![0u8; content_length];
            if content_length > 0 && reader.read_exact(&mut body).await.is_err() {
                write_text_response(&mut writer, "400 Bad Request", "Invalid request body").await;
                return Ok(());
            }
            let body_str = String::from_utf8_lossy(&body);
            let submitted = extract_form_field(&body_str, "password").unwrap_or_default();
            let submitted_hash = *blake3::hash(submitted.as_bytes()).as_bytes();

            if constant_time_eq(&submitted_hash, &state.password_hash) {
                let token = issue_session_token(&state.cookie_secret);
                let cookie = format!(
                    "{SESSION_COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECS}"
                );
                write_redirect(&mut writer, "303 See Other", "/watch", Some(&cookie)).await;
            } else {
                let html = render_login_html(&state.file_name, true);
                write_html_response(&mut writer, "401 Unauthorized", &html, false).await;
            }
        }
        ("POST", "/logout") => {
            let clear_cookie =
                format!("{SESSION_COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
            write_redirect(&mut writer, "303 See Other", "/watch", Some(&clear_cookie)).await;
        }
        ("GET", "/stream") | ("HEAD", "/stream") => {
            if !is_authenticated {
                write_text_response(&mut writer, "401 Unauthorized", "Unauthorized").await;
                return Ok(());
            }
            serve_video_stream(
                &mut writer,
                &state,
                range_header.as_deref(),
                method == "HEAD",
            )
            .await;
        }
        ("GET", _) | ("HEAD", _) | ("POST", _) => {
            write_text_response(&mut writer, "404 Not Found", "Not found").await;
        }
        _ => {
            write_text_response(&mut writer, "405 Method Not Allowed", "Method not allowed").await;
        }
    }

    Ok(())
}

async fn serve_video_stream(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    state: &ServerSessionState,
    range_header: Option<&str>,
    head_only: bool,
) {
    let mut file = match &state.source {
        VideoSource::File(path) => match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(_) => {
                write_text_response(writer, "404 Not Found", "Video file unavailable").await;
                return;
            }
        },
        VideoSource::ContentUri { uri, .. } => {
            let Some(ref hook) = state.open_source_hook else {
                write_text_response(writer, "500 Internal Server Error", "Source hook missing")
                    .await;
                return;
            };
            let uri_owned = uri.clone();
            let hook_clone = Arc::clone(hook);
            let std_file = tokio::task::spawn_blocking(move || hook_clone(&uri_owned))
                .await
                .ok()
                .flatten();
            let Some(std_file) = std_file else {
                write_text_response(writer, "404 Not Found", "Video URI unavailable").await;
                return;
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
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: 0\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            state.mime_type
        );
        let _ = writer.write_all(headers.as_bytes()).await;
        return;
    }

    let (status_line, start, end) = if let Some(range_val) = range_header {
        match parse_byte_range(range_val, total) {
            Some((s, e)) => ("206 Partial Content", s, e),
            None => {
                let resp = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nConnection: close\r\n\r\n"
                );
                let _ = writer.write_all(resp.as_bytes()).await;
                return;
            }
        }
    } else {
        ("200 OK", 0, total - 1)
    };

    let content_len = end - start + 1;
    let mut headers = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {}\r\nContent-Length: {content_len}\r\nAccept-Ranges: bytes\r\nCache-Control: no-store\r\nConnection: close\r\n",
        state.mime_type
    );
    if status_line.starts_with("206") {
        headers.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    headers.push_str("\r\n");

    if writer.write_all(headers.as_bytes()).await.is_err() {
        return;
    }
    if head_only {
        return;
    }

    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return;
    }

    let mut remaining = content_len;
    let mut buf = vec![0u8; STREAM_CHUNK_BYTES];
    while remaining > 0 {
        let to_read = (remaining as usize).min(buf.len());
        let Ok(n) = file.read(&mut buf[..to_read]).await else {
            break;
        };
        if n == 0 {
            break;
        }
        if writer.write_all(&buf[..n]).await.is_err() {
            break;
        }
        remaining -= n as u64;
    }
    let _ = writer.flush().await;
}

async fn write_text_response(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: &str,
    body: &str,
) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = writer.write_all(resp.as_bytes()).await;
}

async fn write_html_response(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: &str,
    html: &str,
    head_only: bool,
) {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        html.len()
    );
    if writer.write_all(headers.as_bytes()).await.is_err() {
        return;
    }
    if !head_only {
        let _ = writer.write_all(html.as_bytes()).await;
    }
}

async fn write_redirect(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: &str,
    location: &str,
    set_cookie: Option<&str>,
) {
    let mut resp = format!(
        "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nConnection: close\r\n"
    );
    if let Some(cookie) = set_cookie {
        resp.push_str(&format!("Set-Cookie: {cookie}\r\n"));
    }
    resp.push_str("\r\n");
    let _ = writer.write_all(resp.as_bytes()).await;
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
