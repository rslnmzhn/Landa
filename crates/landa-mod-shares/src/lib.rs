use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Cursor, Seek};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::Utc;
use image::codecs::jpeg::JpegEncoder;
use image::ImageFormat;
use landa_core::{
    CoreError, CoreEvent, LandaEngine, LandaModule, PeerContext, Storage, TransferHistoryRecord,
};
use landa_proto::path_policy::{resolve_within_root, validate_relative_path};
use landa_proto::wire::{read_frame, write_frame, StreamKind, MAX_CONTROL_FRAME_BYTES};
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const SHARES_SETTING_KEY: &str = "shares_roots";
const MAX_DIR_ENTRIES: usize = 5_000;
const CHUNK_SIZE: usize = 64 * 1024;
const THUMB_CACHE_MAX_BYTES: usize = 50 * 1024 * 1024;
const MAX_THUMB_SOURCE_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_THUMB_SIZE: u32 = 160;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalShareRoot {
    pub id: String,
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareRootInfo {
    pub id: String,
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareEntry {
    pub name: String,
    pub rel_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareThumbnail {
    pub mime_type: String,
    pub data_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
enum ShareRequest {
    ListRoots,
    ListDir { root_id: String, rel_path: String },
    DownloadFile { root_id: String, rel_path: String },
    GetThumbnail { root_id: String, rel_path: String, max_size: u32 },
}

#[derive(Debug, Serialize, Deserialize)]
enum ShareResponse {
    Roots(Vec<ShareRootInfo>),
    Entries(Vec<ShareEntry>),
    FileHeader { file_name: String, size_bytes: u64 },
    Thumbnail { mime_type: String, bytes: Vec<u8> },
    Error(String),
}

#[derive(Clone)]
struct CachedThumb {
    mime_type: String,
    bytes: Vec<u8>,
}

struct ThumbnailCacheInner {
    map: HashMap<String, CachedThumb>,
    order: VecDeque<String>,
    total_bytes: usize,
}

struct ThumbnailLruCache {
    max_bytes: usize,
    inner: Mutex<ThumbnailCacheInner>,
}

impl ThumbnailLruCache {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            inner: Mutex::new(ThumbnailCacheInner {
                map: HashMap::new(),
                order: VecDeque::new(),
                total_bytes: 0,
            }),
        }
    }

    fn get(&self, key: &str) -> Option<CachedThumb> {
        let mut guard = self.inner.lock().expect("thumb cache lock poisoned");
        let val = guard.map.get(key).cloned()?;
        if let Some(pos) = guard.order.iter().position(|k| k == key) {
            guard.order.remove(pos);
        }
        guard.order.push_back(key.to_string());
        Some(val)
    }

    fn insert(&self, key: String, value: CachedThumb) {
        let entry_size = value.bytes.len();
        if entry_size > self.max_bytes {
            return;
        }
        let mut guard = self.inner.lock().expect("thumb cache lock poisoned");
        if let Some(old) = guard.map.remove(&key) {
            guard.total_bytes = guard.total_bytes.saturating_sub(old.bytes.len());
            if let Some(pos) = guard.order.iter().position(|k| k == &key) {
                guard.order.remove(pos);
            }
        }
        while guard.total_bytes + entry_size > self.max_bytes {
            let Some(oldest_key) = guard.order.pop_front() else {
                break;
            };
            if let Some(removed) = guard.map.remove(&oldest_key) {
                guard.total_bytes = guard.total_bytes.saturating_sub(removed.bytes.len());
            }
        }
        guard.total_bytes += entry_size;
        guard.order.push_back(key.clone());
        guard.map.insert(key, value);
    }
}

pub struct ShareOpenedFile {
    pub file: std::fs::File,
    pub file_name: String,
    pub size_bytes: u64,
    pub modified_ms: i64,
}

pub trait ShareContentProvider: Send + Sync {
    fn list_dir(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError>;

    fn open_file(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
    ) -> Result<ShareOpenedFile, CoreError>;
}

pub struct SharesModule {
    storage: Arc<Storage>,
    enabled: AtomicBool,
    local_thumb_cache: ThumbnailLruCache,
    remote_thumb_cache: ThumbnailLruCache,
    content_provider: RwLock<Option<Arc<dyn ShareContentProvider>>>,
}

impl SharesModule {
    pub fn new(storage: Arc<Storage>) -> Arc<Self> {
        Arc::new(Self {
            storage,
            enabled: AtomicBool::new(true),
            local_thumb_cache: ThumbnailLruCache::new(THUMB_CACHE_MAX_BYTES),
            remote_thumb_cache: ThumbnailLruCache::new(THUMB_CACHE_MAX_BYTES),
            content_provider: RwLock::new(None),
        })
    }

    pub fn set_content_provider(&self, provider: Arc<dyn ShareContentProvider>) {
        *self
            .content_provider
            .write()
            .expect("share content provider lock poisoned") = Some(provider);
    }

    fn get_content_provider(&self) -> Result<Arc<dyn ShareContentProvider>, CoreError> {
        self.content_provider
            .read()
            .expect("share content provider lock poisoned")
            .clone()
            .ok_or_else(|| {
                CoreError::InvalidInput("content URI shares are not supported on this platform".into())
            })
    }

    pub fn list_local_shares(&self) -> Result<Vec<LocalShareRoot>, CoreError> {
        Ok(self
            .storage
            .get_setting_json::<Vec<LocalShareRoot>>(SHARES_SETTING_KEY)?
            .unwrap_or_default())
    }

    pub fn add_local_share(
        &self,
        path: PathBuf,
        custom_name: Option<String>,
    ) -> Result<LocalShareRoot, CoreError> {
        let raw_str = path.to_string_lossy();
        if raw_str.starts_with("content://") {
            return Err(CoreError::InvalidInput(
                "use add_local_share_uri for content:// URIs".into(),
            ));
        }
        let canonical = std::fs::canonicalize(&path)?;
        let is_dir = canonical.is_dir();
        if !is_dir && !canonical.is_file() {
            return Err(CoreError::InvalidInput("path must be a file or directory".into()));
        }
        let name = custom_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                canonical
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Share")
                    .to_string()
            });

        let mut roots = self.list_local_shares()?;
        let canonical_str = canonical.to_string_lossy().to_string();
        if let Some(existing) = roots.iter().find(|r| r.path == canonical_str) {
            return Ok(existing.clone());
        }

        let root = LocalShareRoot {
            id: Uuid::new_v4().to_string(),
            name,
            path: canonical_str,
            is_dir,
        };
        roots.push(root.clone());
        self.storage.set_setting_json(SHARES_SETTING_KEY, &roots)?;
        Ok(root)
    }

    pub fn add_local_share_uri(
        &self,
        uri: String,
        name: String,
        is_dir: bool,
    ) -> Result<LocalShareRoot, CoreError> {
        let trimmed_uri = uri.trim().to_string();
        if !trimmed_uri.starts_with("content://") {
            return Err(CoreError::InvalidInput("share URI must start with content://".into()));
        }
        let clean_name = name.trim().to_string();
        let display_name = if clean_name.is_empty() {
            if is_dir { "Folder".to_string() } else { "File".to_string() }
        } else {
            clean_name
        };

        let mut roots = self.list_local_shares()?;
        if let Some(existing) = roots.iter().find(|r| r.path == trimmed_uri) {
            return Ok(existing.clone());
        }

        let root = LocalShareRoot {
            id: Uuid::new_v4().to_string(),
            name: display_name,
            path: trimmed_uri,
            is_dir,
        };
        roots.push(root.clone());
        self.storage.set_setting_json(SHARES_SETTING_KEY, &roots)?;
        Ok(root)
    }

    pub fn remove_local_share(&self, id: &str) -> Result<(), CoreError> {
        let mut roots = self.list_local_shares()?;
        roots.retain(|r| r.id != id);
        self.storage.set_setting_json(SHARES_SETTING_KEY, &roots)?;
        Ok(())
    }

    pub async fn browse_remote_roots(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
    ) -> Result<Vec<ShareRootInfo>, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("shares".into()));
        }
        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Shares)
            .await?;
        write_frame(&mut send, &ShareRequest::ListRoots).await?;
        let _ = send.finish();

        let resp: ShareResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        match resp {
            ShareResponse::Roots(roots) => Ok(roots),
            ShareResponse::Error(msg) => Err(CoreError::InvalidInput(msg)),
            _ => Err(CoreError::InvalidInput("unexpected share response".into())),
        }
    }

    pub async fn browse_remote_dir(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        root_id: &str,
        rel_path: &str,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("shares".into()));
        }
        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Shares)
            .await?;
        write_frame(
            &mut send,
            &ShareRequest::ListDir {
                root_id: root_id.to_string(),
                rel_path: rel_path.to_string(),
            },
        )
        .await?;
        let _ = send.finish();

        let resp: ShareResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        match resp {
            ShareResponse::Entries(entries) => Ok(entries),
            ShareResponse::Error(msg) => Err(CoreError::InvalidInput(msg)),
            _ => Err(CoreError::InvalidInput("unexpected share response".into())),
        }
    }

    pub async fn download_remote_file(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        root_id: &str,
        rel_path: &str,
    ) -> Result<String, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("shares".into()));
        }
        let (peer_ctx, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Shares)
            .await?;
        write_frame(
            &mut send,
            &ShareRequest::DownloadFile {
                root_id: root_id.to_string(),
                rel_path: rel_path.to_string(),
            },
        )
        .await?;
        let _ = send.finish();

        let resp: ShareResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        let (file_name, size_bytes) = match resp {
            ShareResponse::FileHeader {
                file_name,
                size_bytes,
            } => (file_name, size_bytes),
            ShareResponse::Error(msg) => return Err(CoreError::InvalidInput(msg)),
            _ => return Err(CoreError::InvalidInput("unexpected share response".into())),
        };

        let safe_rel = validate_relative_path(&file_name)?;
        let download_dir = PathBuf::from(engine.get_settings().download_dir);
        tokio::fs::create_dir_all(&download_dir).await?;
        let dest = resolve_within_root(&download_dir, &safe_rel)?;
        let temp_path = download_dir.join(format!(".landa-part-{}", Uuid::new_v4()));

        let mut file = tokio::fs::File::create(&temp_path).await?;
        let mut hasher = blake3::Hasher::new();
        let mut remaining = size_bytes;
        let mut buf = vec![0u8; CHUNK_SIZE];

        while remaining > 0 {
            let to_read = (remaining as usize).min(buf.len());
            let n = recv
                .read(&mut buf[..to_read])
                .await?
                .ok_or_else(|| CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "share download truncated",
                )))?;
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
        file.flush().await?;
        drop(file);

        let mut expected_digest = [0u8; 32];
        if let Err(err) = recv.read_exact(&mut expected_digest).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                err,
            )));
        }
        if hasher.finalize().as_bytes() != &expected_digest {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(CoreError::IntegrityMismatch(file_name));
        }

        let final_dest = unique_path(&dest);
        if tokio::fs::rename(&temp_path, &final_dest).await.is_err() {
            tokio::fs::copy(&temp_path, &final_dest).await?;
            let _ = tokio::fs::remove_file(&temp_path).await;
        }

        let saved_str = final_dest.to_string_lossy().to_string();
        let record = TransferHistoryRecord {
            id: Uuid::new_v4().to_string(),
            direction: "receive".to_string(),
            peer_id: peer_ctx.peer_id.to_hex(),
            peer_name: peer_ctx.display_name,
            file_count: 1,
            total_bytes: size_bytes,
            saved_paths: vec![saved_str.clone()],
            status: "completed".to_string(),
            error_message: None,
            created_at: Utc::now().timestamp_millis(),
        };
        let _ = engine.storage().insert_transfer_history(&record);
        let _ = engine.events().send(CoreEvent::HistoryChanged);

        Ok(saved_str)
    }

    pub async fn preview_remote_file(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        root_id: &str,
        rel_path: &str,
        cache_dir: &Path,
        max_bytes: u64,
    ) -> Result<String, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("shares".into()));
        }
        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Shares)
            .await?;
        write_frame(
            &mut send,
            &ShareRequest::DownloadFile {
                root_id: root_id.to_string(),
                rel_path: rel_path.to_string(),
            },
        )
        .await?;
        let _ = send.finish();

        let resp: ShareResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        let (file_name, size_bytes) = match resp {
            ShareResponse::FileHeader {
                file_name,
                size_bytes,
            } => (file_name, size_bytes),
            ShareResponse::Error(msg) => return Err(CoreError::InvalidInput(msg)),
            _ => return Err(CoreError::InvalidInput("unexpected share response".into())),
        };

        if size_bytes > max_bytes {
            return Err(CoreError::InvalidInput(format!(
                "File exceeds preview limit ({} MB)",
                max_bytes / (1024 * 1024)
            )));
        }

        let safe_rel = validate_relative_path(&file_name)?;
        let safe_name = safe_rel
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("preview.bin");

        let mut key_hasher = blake3::Hasher::new();
        key_hasher.update(peer_id.as_bytes());
        key_hasher.update(b"\0");
        key_hasher.update(root_id.as_bytes());
        key_hasher.update(b"\0");
        key_hasher.update(rel_path.as_bytes());
        let key_hex = &key_hasher.finalize().to_hex()[..16];

        tokio::fs::create_dir_all(cache_dir).await?;
        let cache_path = cache_dir.join(format!("{key_hex}_{safe_name}"));
        if let Ok(meta) = tokio::fs::metadata(&cache_path).await {
            if meta.is_file() && meta.len() == size_bytes {
                return Ok(cache_path.to_string_lossy().to_string());
            }
        }

        let temp_path = cache_dir.join(format!(".preview-part-{}", Uuid::new_v4()));
        let mut file = tokio::fs::File::create(&temp_path).await?;
        let mut hasher = blake3::Hasher::new();
        let mut remaining = size_bytes;
        let mut buf = vec![0u8; CHUNK_SIZE];

        while remaining > 0 {
            let to_read = (remaining as usize).min(buf.len());
            let n = recv
                .read(&mut buf[..to_read])
                .await?
                .ok_or_else(|| CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "share preview truncated",
                )))?;
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).await?;
            remaining -= n as u64;
        }
        file.flush().await?;
        drop(file);

        let mut expected_digest = [0u8; 32];
        if let Err(err) = recv.read_exact(&mut expected_digest).await {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                err,
            )));
        }
        if hasher.finalize().as_bytes() != &expected_digest {
            let _ = tokio::fs::remove_file(&temp_path).await;
            return Err(CoreError::IntegrityMismatch(file_name));
        }

        if tokio::fs::rename(&temp_path, &cache_path).await.is_err() {
            tokio::fs::copy(&temp_path, &cache_path).await?;
            let _ = tokio::fs::remove_file(&temp_path).await;
        }

        Ok(cache_path.to_string_lossy().to_string())
    }

    pub async fn get_remote_thumbnail(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        root_id: &str,
        rel_path: &str,
        max_size: u32,
    ) -> Result<ShareThumbnail, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("shares".into()));
        }
        let clamped_size = max_size.clamp(64, 320);
        let cache_key = format!("{peer_id}:{root_id}:{rel_path}:{clamped_size}");
        if let Some(cached) = self.remote_thumb_cache.get(&cache_key) {
            return Ok(to_share_thumbnail(&cached));
        }

        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Shares)
            .await?;
        write_frame(
            &mut send,
            &ShareRequest::GetThumbnail {
                root_id: root_id.to_string(),
                rel_path: rel_path.to_string(),
                max_size: clamped_size,
            },
        )
        .await?;
        let _ = send.finish();

        let resp: ShareResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        match resp {
            ShareResponse::Thumbnail { mime_type, bytes } => {
                let entry = CachedThumb { mime_type, bytes };
                self.remote_thumb_cache.insert(cache_key, entry.clone());
                Ok(to_share_thumbnail(&entry))
            }
            ShareResponse::Error(msg) => Err(CoreError::InvalidInput(msg)),
            _ => Err(CoreError::InvalidInput("unexpected share response".into())),
        }
    }

    fn find_root(&self, root_id: &str) -> Result<LocalShareRoot, CoreError> {
        self.list_local_shares()?
            .into_iter()
            .find(|r| r.id == root_id)
            .ok_or_else(|| CoreError::NotFound("share root".into()))
    }

    fn normalize_share_rel_path(is_dir: bool, rel_path: &str) -> Result<String, CoreError> {
        let trimmed = rel_path.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == "/" {
            return Ok(String::new());
        }
        if !is_dir {
            return Err(CoreError::InvalidInput("cannot traverse into file share".into()));
        }
        let safe_rel = validate_relative_path(trimmed)?;
        Ok(safe_rel.to_string_lossy().replace('\\', "/"))
    }

    fn resolve_root_target(&self, root_id: &str, rel_path: &str) -> Result<(LocalShareRoot, PathBuf), CoreError> {
        let root = self.find_root(root_id)?;
        let root_canon = std::fs::canonicalize(&root.path)?;

        let clean_rel = Self::normalize_share_rel_path(root.is_dir, rel_path)?;
        if clean_rel.is_empty() {
            return Ok((root, root_canon));
        }

        let candidate = root_canon.join(&clean_rel);
        let target_canon = std::fs::canonicalize(&candidate)?;
        if !target_canon.starts_with(&root_canon) {
            return Err(CoreError::InvalidInput("path escapes share root".into()));
        }
        Ok((root, target_canon))
    }
}

#[async_trait]
impl LandaModule for SharesModule {
    fn id(&self) -> &'static str {
        "shares"
    }

    fn stream_kinds(&self) -> &'static [StreamKind] {
        &[StreamKind::Shares]
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    async fn handle_stream(
        &self,
        _peer: PeerContext,
        _kind: StreamKind,
        mut send: SendStream,
        mut recv: RecvStream,
    ) -> Result<(), CoreError> {
        let req: ShareRequest = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;

        match req {
            ShareRequest::ListRoots => {
                let roots = self
                    .list_local_shares()?
                    .into_iter()
                    .map(|r| ShareRootInfo {
                        id: r.id,
                        name: r.name,
                        is_dir: r.is_dir,
                    })
                    .collect();
                send_response(&mut send, &ShareResponse::Roots(roots)).await?;
            }
            ShareRequest::ListDir { root_id, rel_path } => {
                let root = match self.find_root(&root_id) {
                    Ok(r) => r,
                    Err(err) => {
                        send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        let _ = send.finish();
                        return Ok(());
                    }
                };

                if root.path.starts_with("content://") {
                    let res = match (
                        Self::normalize_share_rel_path(root.is_dir, &rel_path),
                        self.get_content_provider(),
                    ) {
                        (Ok(clean_rel), Ok(provider)) => {
                            let uri = root.path.clone();
                            let is_dir = root.is_dir;
                            tokio::task::spawn_blocking(move || {
                                provider.list_dir(&uri, is_dir, &clean_rel)
                            })
                            .await
                            .map_err(|e| CoreError::InvalidInput(e.to_string()))?
                        }
                        (Err(err), _) | (_, Err(err)) => Err(err),
                    };
                    match res {
                        Ok(entries) => {
                            send_response(&mut send, &ShareResponse::Entries(entries)).await?;
                        }
                        Err(err) => {
                            send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        }
                    }
                } else {
                    match self.resolve_root_target(&root_id, &rel_path) {
                        Ok((root, target)) => {
                            let entries = if !root.is_dir {
                                let meta = std::fs::metadata(&target)?;
                                vec![ShareEntry {
                                    name: root.name,
                                    rel_path: String::new(),
                                    is_dir: false,
                                    size_bytes: meta.len(),
                                    modified_ms: modified_ms(&meta),
                                }]
                            } else {
                                read_dir_entries(&PathBuf::from(&root.path), &target)?
                            };
                            send_response(&mut send, &ShareResponse::Entries(entries)).await?;
                        }
                        Err(err) => {
                            send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        }
                    }
                }
            }
            ShareRequest::DownloadFile { root_id, rel_path } => {
                let root = match self.find_root(&root_id) {
                    Ok(r) => r,
                    Err(err) => {
                        send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        let _ = send.finish();
                        return Ok(());
                    }
                };

                if root.path.starts_with("content://") {
                    let opened_res = match (
                        Self::normalize_share_rel_path(root.is_dir, &rel_path),
                        self.get_content_provider(),
                    ) {
                        (Ok(clean_rel), Ok(provider)) => {
                            let uri = root.path.clone();
                            let is_dir = root.is_dir;
                            tokio::task::spawn_blocking(move || {
                                provider.open_file(&uri, is_dir, &clean_rel)
                            })
                            .await
                            .map_err(|e| CoreError::InvalidInput(e.to_string()))?
                        }
                        (Err(err), _) | (_, Err(err)) => Err(err),
                    };
                    match opened_res {
                        Ok(opened) => {
                            send_response(
                                &mut send,
                                &ShareResponse::FileHeader {
                                    file_name: opened.file_name,
                                    size_bytes: opened.size_bytes,
                                },
                            )
                            .await?;

                            let mut file = tokio::fs::File::from_std(opened.file);
                            let mut hasher = blake3::Hasher::new();
                            let mut buf = vec![0u8; CHUNK_SIZE];
                            let mut remaining = opened.size_bytes;
                            while remaining > 0 {
                                let to_read = (remaining as usize).min(buf.len());
                                let n = file.read(&mut buf[..to_read]).await?;
                                if n == 0 {
                                    break;
                                }
                                hasher.update(&buf[..n]);
                                send.write_all(&buf[..n]).await?;
                                remaining -= n as u64;
                            }
                            send.write_all(hasher.finalize().as_bytes()).await?;
                        }
                        Err(err) => {
                            send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        }
                    }
                } else {
                    match self.resolve_root_target(&root_id, &rel_path) {
                        Ok((_, target)) if target.is_file() => {
                            let meta = tokio::fs::metadata(&target).await?;
                            let file_name = target
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("file.bin")
                                .to_string();
                            send_response(
                                &mut send,
                                &ShareResponse::FileHeader {
                                    file_name,
                                    size_bytes: meta.len(),
                                },
                            )
                            .await?;

                            let mut file = tokio::fs::File::open(&target).await?;
                            let mut hasher = blake3::Hasher::new();
                            let mut buf = vec![0u8; CHUNK_SIZE];
                            loop {
                                let n = file.read(&mut buf).await?;
                                if n == 0 {
                                    break;
                                }
                                hasher.update(&buf[..n]);
                                send.write_all(&buf[..n]).await?;
                            }
                            send.write_all(hasher.finalize().as_bytes()).await?;
                        }
                        Ok(_) => {
                            send_response(
                                &mut send,
                                &ShareResponse::Error("target is not a file".into()),
                            )
                            .await?;
                        }
                        Err(err) => {
                            send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        }
                    }
                }
            }
            ShareRequest::GetThumbnail {
                root_id,
                rel_path,
                max_size,
            } => {
                let target_size = if max_size == 0 {
                    DEFAULT_THUMB_SIZE
                } else {
                    max_size.clamp(64, 320)
                };
                let root = match self.find_root(&root_id) {
                    Ok(r) => r,
                    Err(err) => {
                        send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        let _ = send.finish();
                        return Ok(());
                    }
                };

                if root.path.starts_with("content://") {
                    let opened_res = match (
                        Self::normalize_share_rel_path(root.is_dir, &rel_path),
                        self.get_content_provider(),
                    ) {
                        (Ok(clean_rel), Ok(provider)) => {
                            let uri = root.path.clone();
                            let is_dir = root.is_dir;
                            tokio::task::spawn_blocking(move || {
                                provider.open_file(&uri, is_dir, &clean_rel)
                            })
                            .await
                            .map_err(|e| CoreError::InvalidInput(e.to_string()))?
                        }
                        (Err(err), _) | (_, Err(err)) => Err(err),
                    };
                    match opened_res {
                        Ok(opened) => {
                            if !is_supported_thumb_image_name(&opened.file_name) {
                                send_response(
                                    &mut send,
                                    &ShareResponse::Error("unsupported thumbnail format".into()),
                                )
                                .await?;
                            } else if opened.size_bytes > MAX_THUMB_SOURCE_BYTES {
                                send_response(
                                    &mut send,
                                    &ShareResponse::Error("image too large for thumbnail".into()),
                                )
                                .await?;
                            } else {
                                let cache_key = format!(
                                    "{root_id}:{rel_path}:{}:{}:{target_size}",
                                    opened.modified_ms, opened.size_bytes
                                );
                                if let Some(cached) = self.local_thumb_cache.get(&cache_key) {
                                    send_response(
                                        &mut send,
                                        &ShareResponse::Thumbnail {
                                            mime_type: cached.mime_type,
                                            bytes: cached.bytes,
                                        },
                                    )
                                    .await?;
                                } else {
                                    let gen_res = tokio::task::spawn_blocking(move || {
                                        generate_image_thumbnail_from_reader(
                                            BufReader::new(opened.file),
                                            target_size,
                                        )
                                    })
                                    .await
                                    .map_err(|e| CoreError::InvalidInput(e.to_string()))?;

                                    match gen_res {
                                        Ok((mime_type, bytes)) => {
                                            self.local_thumb_cache.insert(
                                                cache_key,
                                                CachedThumb {
                                                    mime_type: mime_type.clone(),
                                                    bytes: bytes.clone(),
                                                },
                                            );
                                            send_response(
                                                &mut send,
                                                &ShareResponse::Thumbnail { mime_type, bytes },
                                            )
                                            .await?;
                                        }
                                        Err(err) => {
                                            send_response(
                                                &mut send,
                                                &ShareResponse::Error(err.to_string()),
                                            )
                                            .await?;
                                        }
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        }
                    }
                } else {
                    match self.resolve_root_target(&root_id, &rel_path) {
                        Ok((_, target)) if target.is_file() => {
                            if !is_supported_thumb_image(&target) {
                                send_response(
                                    &mut send,
                                    &ShareResponse::Error("unsupported thumbnail format".into()),
                                )
                                .await?;
                            } else {
                                let meta = tokio::fs::metadata(&target).await?;
                                if meta.len() > MAX_THUMB_SOURCE_BYTES {
                                    send_response(
                                        &mut send,
                                        &ShareResponse::Error("image too large for thumbnail".into()),
                                    )
                                    .await?;
                                } else {
                                    let mod_ms = modified_ms(&meta);
                                    let cache_key =
                                        format!("{root_id}:{rel_path}:{mod_ms}:{target_size}");
                                    if let Some(cached) = self.local_thumb_cache.get(&cache_key) {
                                        send_response(
                                            &mut send,
                                            &ShareResponse::Thumbnail {
                                                mime_type: cached.mime_type,
                                                bytes: cached.bytes,
                                            },
                                        )
                                        .await?;
                                    } else {
                                        let target_clone = target.clone();
                                        let gen_res = tokio::task::spawn_blocking(move || {
                                            generate_image_thumbnail(&target_clone, target_size)
                                        })
                                        .await
                                        .map_err(|e| CoreError::InvalidInput(e.to_string()))?;

                                        match gen_res {
                                            Ok((mime_type, bytes)) => {
                                                self.local_thumb_cache.insert(
                                                    cache_key,
                                                    CachedThumb {
                                                        mime_type: mime_type.clone(),
                                                        bytes: bytes.clone(),
                                                    },
                                                );
                                                send_response(
                                                    &mut send,
                                                    &ShareResponse::Thumbnail { mime_type, bytes },
                                                )
                                                .await?;
                                            }
                                            Err(err) => {
                                                send_response(
                                                    &mut send,
                                                    &ShareResponse::Error(err.to_string()),
                                                )
                                                .await?;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(_) => {
                            send_response(
                                &mut send,
                                &ShareResponse::Error("target is not a file".into()),
                            )
                            .await?;
                        }
                        Err(err) => {
                            send_response(&mut send, &ShareResponse::Error(err.to_string())).await?;
                        }
                    }
                }
            }
        }

        let _ = send.finish();
        Ok(())
    }
}

fn to_share_thumbnail(entry: &CachedThumb) -> ShareThumbnail {
    let b64 = base64::engine::general_purpose::STANDARD.encode(&entry.bytes);
    ShareThumbnail {
        mime_type: entry.mime_type.clone(),
        data_url: format!("data:{};base64,{b64}", entry.mime_type),
    }
}

fn is_supported_thumb_image_name(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
}

fn is_supported_thumb_image(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
}

fn generate_image_thumbnail(path: &Path, max_size: u32) -> Result<(String, Vec<u8>), CoreError> {
    let file = std::fs::File::open(path)?;
    generate_image_thumbnail_from_reader(BufReader::new(file), max_size)
}

fn generate_image_thumbnail_from_reader<R: BufRead + Seek>(
    reader: R,
    max_size: u32,
) -> Result<(String, Vec<u8>), CoreError> {
    let reader = image::ImageReader::new(reader)
        .with_guessed_format()
        .map_err(|e| CoreError::InvalidInput(format!("image format error: {e}")))?;
    let img = reader
        .decode()
        .map_err(|e| CoreError::InvalidInput(format!("image decode error: {e}")))?;
    let thumb = img.thumbnail(max_size, max_size);

    let mut out = Vec::new();
    if thumb.color().has_alpha() {
        thumb
            .write_to(&mut Cursor::new(&mut out), ImageFormat::WebP)
            .map_err(|e| CoreError::InvalidInput(format!("webp encode error: {e}")))?;
        Ok(("image/webp".to_string(), out))
    } else {
        let rgb = thumb.to_rgb8();
        let encoder = JpegEncoder::new_with_quality(&mut out, 80);
        rgb.write_with_encoder(encoder)
            .map_err(|e| CoreError::InvalidInput(format!("jpeg encode error: {e}")))?;
        Ok(("image/jpeg".to_string(), out))
    }
}

async fn send_response(send: &mut SendStream, resp: &ShareResponse) -> Result<(), CoreError> {
    write_frame(send, resp).await?;
    Ok(())
}

fn read_dir_entries(root_canon: &Path, dir: &Path) -> Result<Vec<ShareEntry>, CoreError> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        if entries.len() >= MAX_DIR_ENTRIES {
            break;
        }
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root_canon) else {
            continue;
        };
        let meta = entry.metadata()?;
        entries.push(ShareEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            rel_path: rel.to_string_lossy().replace('\\', "/"),
            is_dir: file_type.is_dir(),
            size_bytes: if file_type.is_dir() { 0 } else { meta.len() },
            modified_ms: modified_ms(&meta),
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

fn modified_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|e| e.to_str());
    for idx in 1..10_000 {
        let name = match ext {
            Some(e) if !e.is_empty() => format!("{stem} ({idx}).{e}"),
            _ => format!("{stem} ({idx})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}
