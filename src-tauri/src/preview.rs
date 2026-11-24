use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use uuid::Uuid;

use crate::commands::AppState;

const MAX_PREVIEW_ENTRIES: usize = 256;
const MAX_TEXT_PREVIEW_BYTES: usize = 512 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const STREAM_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_REMOTE_PREVIEW_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewDescriptor {
    pub kind: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub url: Option<String>,
    pub text_content: Option<String>,
    pub truncated: bool,
    pub source_path: String,
}

#[derive(Clone)]
enum PreviewSource {
    File(PathBuf),
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    ContentUri(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedPreviewTarget {
    File { canonical: PathBuf, size_bytes: u64 },
    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    ContentUri(String),
}

#[derive(Clone)]
struct PreviewEntry {
    source: PreviewSource,
    mime_type: String,
    size_bytes: u64,
}

#[derive(Default)]
struct RegistryInner {
    entries: HashMap<String, PreviewEntry>,
    order: VecDeque<String>,
}

type ContentUriOpener = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Option<std::fs::File>> + Send>> + Send + Sync,
>;

pub struct PreviewServer {
    port: u16,
    registry: Arc<RwLock<RegistryInner>>,
}

impl PreviewServer {
    pub async fn start(app: AppHandle) -> Result<Arc<Self>, String> {
        #[cfg(target_os = "android")]
        let opener: Option<ContentUriOpener> = Some(Arc::new(move |uri: String| {
            let app_handle = app.clone();
            Box::pin(async move {
                let bridge = app_handle
                    .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()?;
                bridge.open_content_uri_file_async(&uri).await
            })
        }));
        #[cfg(not(target_os = "android"))]
        let opener: Option<ContentUriOpener> = {
            let _ = app;
            None
        };
        Self::start_inner(opener).await
    }

    #[cfg(test)]
    pub async fn start_without_app() -> Result<Arc<Self>, String> {
        Self::start_inner(None).await
    }

    async fn start_inner(opener: Option<ContentUriOpener>) -> Result<Arc<Self>, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind preview server: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to read preview server port: {e}"))?
            .port();

        let server = Arc::new(Self {
            port,
            registry: Arc::new(RwLock::new(RegistryInner::default())),
        });

        let registry = Arc::clone(&server.registry);
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let reg = Arc::clone(&registry);
                let opener_clone = opener.clone();
                tokio::spawn(async move {
                    let _ = handle_http_connection(stream, reg, opener_clone).await;
                });
            }
        });

        Ok(server)
    }

    fn register(&self, entry: PreviewEntry) -> String {
        let token = Uuid::new_v4().to_string();
        let mut guard = self.registry.write().expect("preview registry lock poisoned");
        while guard.order.len() >= MAX_PREVIEW_ENTRIES {
            if let Some(oldest) = guard.order.pop_front() {
                guard.entries.remove(&oldest);
            }
        }
        guard.order.push_back(token.clone());
        guard.entries.insert(token.clone(), entry);
        format!("http://127.0.0.1:{}/preview/{token}", self.port)
    }
}

pub fn resolve_preview_cache_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_cache_dir()
        .or_else(|_| app.path().app_data_dir())
        .map_err(|e| format!("Failed to resolve cache directory: {e}"))?;
    Ok(base.join("preview_cache"))
}

pub async fn sweep_old_preview_cache(app: &AppHandle) {
    let Ok(dir) = resolve_preview_cache_dir(app) else {
        return;
    };
    if !dir.exists() {
        return;
    }
    let cutoff = SystemTime::now() - Duration::from_secs(24 * 60 * 60);
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if let Ok(meta) = entry.metadata().await {
            if meta.is_file() {
                if let Ok(modified) = meta.modified() {
                    if modified < cutoff {
                        let _ = tokio::fs::remove_file(&path).await;
                    }
                }
            }
        }
    }
}

pub fn strip_windows_verbatim_prefix(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.into_owned()
    }
}

async fn resolve_posix_or_content_uri<F, Fut>(
    path: &str,
    fallback: F,
) -> Result<ResolvedPreviewTarget, String>
where
    F: FnOnce(&str) -> Fut,
    Fut: std::future::Future<Output = Option<String>>,
{
    let posix_result: Result<ResolvedPreviewTarget, String> = async {
        let canonical = PathBuf::from(path)
            .canonicalize()
            .map_err(|e| format!("File not found: {e}"))?;
        let meta = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e| format!("Unable to read file metadata: {e}"))?;
        if !meta.is_file() {
            return Err("Target is not a file".into());
        }
        tokio::fs::File::open(&canonical)
            .await
            .map_err(|e| format!("Unable to open file: {e}"))?;
        Ok(ResolvedPreviewTarget::File {
            canonical,
            size_bytes: meta.len(),
        })
    }
    .await;

    match posix_result {
        Ok(target) => Ok(target),
        Err(err) => {
            if let Some(uri) = fallback(path).await {
                Ok(ResolvedPreviewTarget::ContentUri(uri))
            } else {
                Err(err)
            }
        }
    }
}

async fn prepare_content_uri_preview(
    app: &AppHandle,
    server: &PreviewServer,
    uri: &str,
) -> Result<PreviewDescriptor, String> {
    #[cfg(target_os = "android")]
    {
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        let meta = bridge.get_content_uri_meta_async(uri).await?;
        let (kind, mime_type) = classify_file(&meta.name, Some(&meta.mime_type));

        if kind == "text" {
            let std_file = bridge
                .open_content_uri_file_async(uri)
                .await
                .ok_or_else(|| "Unable to open content URI".to_string())?;
            let mut file = tokio::fs::File::from_std(std_file);
            let (text, truncated) = read_text_preview(&mut file, meta.size_bytes).await?;
            return Ok(PreviewDescriptor {
                kind,
                file_name: meta.name,
                mime_type,
                size_bytes: meta.size_bytes,
                url: None,
                text_content: Some(text),
                truncated,
                source_path: uri.to_string(),
            });
        }

        let url = if matches!(kind.as_str(), "image" | "video" | "audio") {
            Some(server.register(PreviewEntry {
                source: PreviewSource::ContentUri(uri.to_string()),
                mime_type: mime_type.clone(),
                size_bytes: meta.size_bytes,
            }))
        } else {
            None
        };

        Ok(PreviewDescriptor {
            kind,
            file_name: meta.name,
            mime_type,
            size_bytes: meta.size_bytes,
            url,
            text_content: None,
            truncated: false,
            source_path: uri.to_string(),
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, server, uri);
        Err("content:// URIs are only supported on Android".into())
    }
}

pub async fn prepare_file_preview(
    app: &AppHandle,
    state: &AppState,
    server: &PreviewServer,
    path_or_uri: &str,
) -> Result<PreviewDescriptor, String> {
    let trimmed = path_or_uri.trim();
    if trimmed.is_empty() {
        return Err("Empty file path".into());
    }

    authorize_preview_target(app, state, trimmed)?;

    if trimmed.starts_with("content://") {
        return prepare_content_uri_preview(app, server, trimmed).await;
    }

    let resolved = resolve_posix_or_content_uri(trimmed, |path| {
        let app_handle = app.clone();
        let path_owned = path.to_string();
        async move {
            #[cfg(target_os = "android")]
            {
                let bridge = app_handle
                    .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()?;
                bridge.resolve_content_uri_for_path_async(&path_owned).await
            }
            #[cfg(not(target_os = "android"))]
            {
                let _ = (app_handle, path_owned);
                None
            }
        }
    })
    .await?;

    let (canonical, size_bytes) = match resolved {
        ResolvedPreviewTarget::ContentUri(uri) => {
            return prepare_content_uri_preview(app, server, &uri).await;
        }
        ResolvedPreviewTarget::File {
            canonical,
            size_bytes,
        } => (canonical, size_bytes),
    };

    let file_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let (kind, mime_type) = classify_file(&file_name, None);
    let source_path = strip_windows_verbatim_prefix(&canonical);

    if kind == "text" {
        let mut file = tokio::fs::File::open(&canonical)
            .await
            .map_err(|e| format!("Failed to open text file: {e}"))?;
        let (text, truncated) = read_text_preview(&mut file, size_bytes).await?;
        return Ok(PreviewDescriptor {
            kind,
            file_name,
            mime_type,
            size_bytes,
            url: None,
            text_content: Some(text),
            truncated,
            source_path,
        });
    }

    let url = if matches!(kind.as_str(), "image" | "video" | "audio") {
        Some(server.register(PreviewEntry {
            source: PreviewSource::File(canonical),
            mime_type: mime_type.clone(),
            size_bytes,
        }))
    } else {
        None
    };

    Ok(PreviewDescriptor {
        kind,
        file_name,
        mime_type,
        size_bytes,
        url,
        text_content: None,
        truncated: false,
        source_path,
    })
}

pub async fn open_file_external(app: &AppHandle, path_or_uri: &str) -> Result<(), String> {
    let trimmed = path_or_uri.trim();
    if trimmed.is_empty() {
        return Err("Empty file path".into());
    }

    #[cfg(target_os = "android")]
    {
        let bridge = app
            .try_state::<crate::android_bridge::AndroidBridge<tauri::Wry>>()
            .ok_or_else(|| "Android bridge unavailable".to_string())?;
        return bridge.open_file_external_async(trimmed).await;
    }

    #[cfg(not(target_os = "android"))]
    {
        use tauri_plugin_opener::OpenerExt;
        let clean_path = strip_windows_verbatim_prefix(Path::new(trimmed));
        app.opener()
            .open_path(clean_path, None::<&str>)
            .map_err(|e| format!("Failed to open file: {e}"))
    }
}

fn authorize_preview_target(
    app: &AppHandle,
    state: &AppState,
    path_or_uri: &str,
) -> Result<(), String> {
    let history = state
        .engine
        .list_history(500)
        .map_err(|e| e.to_string())?;
    if history
        .iter()
        .any(|rec| rec.saved_paths.iter().any(|p| p == path_or_uri))
    {
        return Ok(());
    }

    if path_or_uri.starts_with("content://") {
        #[cfg(feature = "mod-shares")]
        if let Ok(roots) = state.shares.list_local_shares() {
            if roots.iter().any(|r| !r.is_dir && r.path == path_or_uri) {
                return Ok(());
            }
        }
        return Err("Unauthorized content URI".into());
    }

    let canonical = PathBuf::from(path_or_uri)
        .canonicalize()
        .map_err(|e| format!("File not found: {e}"))?;

    let download_dir = PathBuf::from(state.engine.get_settings().download_dir);
    if let Ok(dl_canon) = download_dir.canonicalize() {
        if canonical.starts_with(&dl_canon) {
            return Ok(());
        }
    }

    if let Ok(cache_dir) = resolve_preview_cache_dir(app) {
        if let Ok(cache_canon) = cache_dir.canonicalize() {
            if canonical.starts_with(&cache_canon) {
                return Ok(());
            }
        }
    }

    #[cfg(feature = "mod-shares")]
    if let Ok(roots) = state.shares.list_local_shares() {
        for root in roots {
            if let Ok(root_canon) = PathBuf::from(&root.path).canonicalize() {
                if canonical == root_canon || canonical.starts_with(&root_canon) {
                    return Ok(());
                }
            }
        }
    }

    #[cfg(feature = "mod-clipboard")]
    if let Ok(clip_canon) = state.clipboard.images_dir().canonicalize() {
        if canonical.starts_with(&clip_canon) {
            return Ok(());
        }
    }

    Err("File is outside allowed preview directories".into())
}

async fn read_text_preview(
    file: &mut tokio::fs::File,
    size_bytes: u64,
) -> Result<(String, bool), String> {
    let to_read = (size_bytes as usize).min(MAX_TEXT_PREVIEW_BYTES);
    let mut buf = vec![0u8; to_read];
    let mut read_total = 0usize;
    while read_total < to_read {
        let n = file
            .read(&mut buf[read_total..])
            .await
            .map_err(|e| format!("Failed to read text preview: {e}"))?;
        if n == 0 {
            break;
        }
        read_total += n;
    }
    buf.truncate(read_total);
    let truncated = size_bytes > MAX_TEXT_PREVIEW_BYTES as u64;
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

async fn handle_http_connection(
    stream: TcpStream,
    registry: Arc<RwLock<RegistryInner>>,
    opener: Option<ContentUriOpener>,
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

    let mut header_bytes = n;
    let mut range_header: Option<String> = None;
    loop {
        let mut line = String::new();
        let read_n = reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
        if read_n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        header_bytes += read_n;
        if header_bytes > MAX_HTTP_HEADER_BYTES {
            write_simple_response(&mut writer, "431 Request Header Fields Too Large", "Header too large").await;
            return Ok(());
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range_header = Some(v.trim().to_string());
            }
        }
    }

    if method != "GET" && method != "HEAD" {
        write_simple_response(&mut writer, "405 Method Not Allowed", "Method not allowed").await;
        return Ok(());
    }

    let path_only = raw_path.split('?').next().unwrap_or("");
    let Some(token) = path_only.strip_prefix("/preview/") else {
        write_simple_response(&mut writer, "404 Not Found", "Not found").await;
        return Ok(());
    };

    let entry = {
        let guard = registry.read().expect("preview registry lock poisoned");
        guard.entries.get(token).cloned()
    };
    let Some(entry) = entry else {
        write_simple_response(&mut writer, "404 Not Found", "Preview expired").await;
        return Ok(());
    };

    let mut file = match &entry.source {
        PreviewSource::File(path) => match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(_) => {
                write_simple_response(&mut writer, "404 Not Found", "File unavailable").await;
                return Ok(());
            }
        },
        PreviewSource::ContentUri(uri) => {
            #[cfg(target_os = "android")]
            {
                let Some(open_fn) = opener.as_ref() else {
                    write_simple_response(&mut writer, "500 Internal Server Error", "Bridge missing").await;
                    return Ok(());
                };
                let Some(std_file) = open_fn(uri.clone()).await else {
                    write_simple_response(&mut writer, "404 Not Found", "URI unavailable").await;
                    return Ok(());
                };
                tokio::fs::File::from_std(std_file)
            }
            #[cfg(not(target_os = "android"))]
            {
                let _ = (&opener, uri);
                write_simple_response(&mut writer, "404 Not Found", "Unsupported").await;
                return Ok(());
            }
        }
    };

    let total = if entry.size_bytes > 0 {
        entry.size_bytes
    } else {
        file.metadata().await.map(|m| m.len()).unwrap_or(0)
    };

    if total == 0 {
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: 0\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            entry.mime_type
        );
        let _ = writer.write_all(headers.as_bytes()).await;
        return Ok(());
    }

    let (status_line, start, end) = if let Some(ref range_val) = range_header {
        match parse_byte_range(range_val, total) {
            Some((s, e)) => ("206 Partial Content", s, e),
            None => {
                let resp = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{total}\r\nConnection: close\r\n\r\n"
                );
                let _ = writer.write_all(resp.as_bytes()).await;
                return Ok(());
            }
        }
    } else {
        ("200 OK", 0, total - 1)
    };

    let content_len = end - start + 1;
    let mut headers = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: {}\r\nContent-Length: {content_len}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n",
        entry.mime_type
    );
    if status_line.starts_with("206") {
        headers.push_str(&format!("Content-Range: bytes {start}-{end}/{total}\r\n"));
    }
    headers.push_str("\r\n");

    if writer.write_all(headers.as_bytes()).await.is_err() {
        return Ok(());
    }

    if method == "HEAD" {
        return Ok(());
    }

    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return Ok(());
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
    Ok(())
}

async fn write_simple_response(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    status: &str,
    body: &str,
) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = writer.write_all(resp.as_bytes()).await;
}

pub fn parse_byte_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    let first_range = spec.split(',').next()?.trim();
    let (start_str, end_str) = first_range.split_once('-')?;

    if start_str.is_empty() {
        let suffix = end_str.trim().parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }

    let start = start_str.trim().parse::<u64>().ok()?;
    if start >= total {
        return None;
    }
    let end = if end_str.trim().is_empty() {
        total - 1
    } else {
        end_str.trim().parse::<u64>().ok()?.min(total - 1)
    };
    if end < start {
        return None;
    }
    Some((start, end))
}

pub fn classify_file(file_name: &str, hint_mime: Option<&str>) -> (String, String) {
    let ext = Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "png" => return ("image".into(), "image/png".into()),
        "jpg" | "jpeg" => return ("image".into(), "image/jpeg".into()),
        "webp" => return ("image".into(), "image/webp".into()),
        "gif" => return ("image".into(), "image/gif".into()),
        "svg" => return ("image".into(), "image/svg+xml".into()),
        "bmp" => return ("image".into(), "image/bmp".into()),
        "ico" => return ("image".into(), "image/x-icon".into()),
        "avif" => return ("image".into(), "image/avif".into()),

        "mp4" | "m4v" => return ("video".into(), "video/mp4".into()),
        "webm" => return ("video".into(), "video/webm".into()),
        "mov" => return ("video".into(), "video/quicktime".into()),
        "ogv" => return ("video".into(), "video/ogg".into()),
        "mkv" => return ("video".into(), "video/x-matroska".into()),
        "3gp" => return ("video".into(), "video/3gpp".into()),

        "mp3" => return ("audio".into(), "audio/mpeg".into()),
        "wav" => return ("audio".into(), "audio/wav".into()),
        "ogg" | "opus" => return ("audio".into(), "audio/ogg".into()),
        "flac" => return ("audio".into(), "audio/flac".into()),
        "m4a" | "aac" => return ("audio".into(), "audio/mp4".into()),
        "weba" => return ("audio".into(), "audio/webm".into()),

        "txt" | "log" | "md" | "markdown" | "json" | "rs" | "ts" | "tsx" | "js" | "jsx"
        | "css" | "html" | "htm" | "xml" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf"
        | "csv" | "sh" | "bash" | "ps1" | "kt" | "java" | "py" | "go" | "c" | "cpp" | "h"
        | "hpp" | "sql" | "env" | "gitignore" | "lock" => {
            return ("text".into(), "text/plain; charset=utf-8".into());
        }
        _ => {}
    }

    if let Some(mime) = hint_mime {
        let lower = mime.trim().to_ascii_lowercase();
        if lower.starts_with("image/") {
            return ("image".into(), lower);
        }
        if lower.starts_with("video/") {
            return ("video".into(), lower);
        }
        if lower.starts_with("audio/") {
            return ("audio".into(), lower);
        }
        if lower.starts_with("text/") || lower == "application/json" {
            return ("text".into(), lower);
        }
    }

    (
        "unsupported".into(),
        hint_mime
            .unwrap_or("application/octet-stream")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_byte_ranges() {
        assert_eq!(parse_byte_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_byte_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_byte_range("bytes=-200", 1000), Some((800, 999)));
        assert_eq!(parse_byte_range("bytes=1000-", 1000), None);
        assert_eq!(parse_byte_range("bytes=200-100", 1000), None);
    }

    #[test]
    fn classifies_common_extensions() {
        assert_eq!(classify_file("photo.JPG", None).0, "image");
        assert_eq!(classify_file("clip.mp4", None).0, "video");
        assert_eq!(classify_file("movie.mkv", None).0, "video");
        assert_eq!(classify_file("mobile.3gp", None).0, "video");
        assert_eq!(classify_file("track.flac", None).0, "audio");
        assert_eq!(classify_file("main.rs", None).0, "text");
        assert_eq!(classify_file("archive.zip", None).0, "unsupported");
    }

    #[test]
    fn strips_windows_verbatim_prefix_cleanly() {
        assert_eq!(
            strip_windows_verbatim_prefix(Path::new(r"\\?\C:\Users\test\photo.png")),
            r"C:\Users\test\photo.png"
        );
        assert_eq!(
            strip_windows_verbatim_prefix(Path::new(r"\\?\UNC\nas\share\clip.mp4")),
            r"\\nas\share\clip.mp4"
        );
        assert_eq!(
            strip_windows_verbatim_prefix(Path::new("/tmp/landa/note.txt")),
            "/tmp/landa/note.txt"
        );
    }

    #[tokio::test]
    async fn preview_server_serves_full_and_range_requests() {
        let temp_dir = std::env::temp_dir().join(format!("landa-preview-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("sample.bin");
        std::fs::write(&file_path, b"0123456789abcdef").unwrap();

        let server = PreviewServer::start_without_app().await.unwrap();
        let url = server.register(PreviewEntry {
            source: PreviewSource::File(file_path.clone()),
            mime_type: "application/octet-stream".into(),
            size_bytes: 16,
        });

        let addr = format!("127.0.0.1:{}", server.port);
        let path = url.strip_prefix(&format!("http://{addr}")).unwrap();

        // Full GET
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).await.unwrap();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(resp_str.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(resp_str.ends_with("0123456789abcdef"));

        // Range GET
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nRange: bytes=2-5\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).await.unwrap();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(resp_str.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(resp_str.contains("Content-Range: bytes 2-5/16\r\n"));
        assert!(resp_str.ends_with("2345"));

        // Unknown token -> 404
        let mut stream = TcpStream::connect(&addr).await.unwrap();
        let req = format!("GET /preview/nonexistent HTTP/1.1\r\nHost: {addr}\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).await.unwrap();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(resp_str.starts_with("HTTP/1.1 404 Not Found\r\n"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn falls_back_to_content_uri_when_posix_path_is_unreadable() {
        let missing_path = "/storage/emulated/0/Download/Landa/missing_scoped.jpg";
        let resolved = resolve_posix_or_content_uri(missing_path, |p| {
            let p = p.to_string();
            async move {
                if p == missing_path {
                    Some("content://media/external/downloads/42".to_string())
                } else {
                    None
                }
            }
        })
        .await
        .unwrap();

        assert_eq!(
            resolved,
            ResolvedPreviewTarget::ContentUri("content://media/external/downloads/42".to_string())
        );
    }
}
