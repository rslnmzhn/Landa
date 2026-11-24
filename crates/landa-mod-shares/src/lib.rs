use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Cursor, Seek};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine as _;
use chrono::Utc;
use image::codecs::jpeg::JpegEncoder;
use image::ImageFormat;
use landa_core::{
    CoreError, CoreEvent, LandaEngine, LandaModule, PeerContext, Storage, TransferHistoryRecord,
    TransferProgressInfo,
};
use landa_proto::path_policy::{
    resolve_within_root, sanitize_relative_path, validate_relative_path,
};
use landa_proto::wire::{read_frame, write_frame, StreamKind, MAX_CONTROL_FRAME_BYTES};
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const SHARES_SETTING_KEY: &str = "shares_roots";
const MAX_DIR_ENTRIES: usize = 5_000;
const MAX_TREE_DEPTH: usize = 32;
const MAX_TREE_FILES: usize = 10_000;
const CHUNK_SIZE: usize = 64 * 1024;
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(100);
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareFolderFileMeta {
    pub rel_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareFolderDownloadResult {
    pub folder_name: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub saved_paths: Vec<String>,
    pub destination: String,
}

#[derive(Debug, Serialize, Deserialize)]
enum ShareRequest {
    ListRoots,
    ListDir {
        root_id: String,
        rel_path: String,
    },
    DownloadFile {
        root_id: String,
        rel_path: String,
    },
    GetThumbnail {
        root_id: String,
        rel_path: String,
        max_size: u32,
    },
    DownloadFolder {
        root_id: String,
        rel_path: String,
    },
}

#[doc(hidden)]
#[derive(Debug, Serialize, Deserialize)]
pub enum ShareResponse {
    Roots(Vec<ShareRootInfo>),
    Entries(Vec<ShareEntry>),
    FileHeader {
        file_name: String,
        size_bytes: u64,
    },
    Thumbnail {
        mime_type: String,
        bytes: Vec<u8>,
    },
    Error(String),
    FolderManifest {
        folder_name: String,
        dirs: Vec<String>,
        files: Vec<ShareFolderFileMeta>,
        total_bytes: u64,
    },
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

    fn list_tree(
        &self,
        root_uri: &str,
        is_dir: bool,
        rel_path: &str,
        max_depth: usize,
        max_files: usize,
    ) -> Result<Vec<ShareEntry>, CoreError> {
        if !is_dir {
            return Err(CoreError::InvalidInput("target is not a directory".into()));
        }
        let mut out = Vec::new();
        let mut dir_count = 0usize;
        let mut file_count = 0usize;
        let mut stack: Vec<(String, usize)> = vec![(rel_path.to_string(), 0)];
        while let Some((dir_rel, depth)) = stack.pop() {
            let entries = self.list_dir(root_uri, true, &dir_rel)?;
            for entry in entries {
                let clean = validate_relative_path(&entry.rel_path)?;
                let clean_str = clean.to_string_lossy().replace('\\', "/");
                if !dir_rel.is_empty() {
                    let expected_prefix = format!("{dir_rel}/");
                    if !clean_str.starts_with(&expected_prefix) {
                        return Err(CoreError::InvalidInput(
                            "share entry path escapes requested directory".into(),
                        ));
                    }
                }
                if entry.is_dir {
                    if depth >= max_depth {
                        return Err(CoreError::InvalidInput(format!(
                            "folder exceeds maximum depth ({max_depth})"
                        )));
                    }
                    if dir_count >= max_files {
                        return Err(CoreError::InvalidInput(format!(
                            "folder exceeds maximum directory count ({max_files})"
                        )));
                    }
                    dir_count += 1;
                    out.push(ShareEntry {
                        rel_path: clean_str.clone(),
                        ..entry
                    });
                    stack.push((clean_str, depth + 1));
                } else {
                    if file_count >= max_files {
                        return Err(CoreError::InvalidInput(format!(
                            "folder exceeds maximum file count ({max_files})"
                        )));
                    }
                    file_count += 1;
                    out.push(ShareEntry {
                        rel_path: clean_str,
                        ..entry
                    });
                }
            }
        }
        out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        Ok(out)
    }
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
                CoreError::InvalidInput(
                    "content URI shares are not supported on this platform".into(),
                )
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
            return Err(CoreError::InvalidInput(
                "path must be a file or directory".into(),
            ));
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
            return Err(CoreError::InvalidInput(
                "share URI must start with content://".into(),
            ));
        }
        let clean_name = name.trim().to_string();
        let display_name = if clean_name.is_empty() {
            if is_dir {
                "Folder".to_string()
            } else {
                "File".to_string()
            }
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
            let n = recv.read(&mut buf[..to_read]).await?.ok_or_else(|| {
                CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "share download truncated",
                ))
            })?;
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
            relative_paths: vec![safe_rel.to_string_lossy().replace('\\', "/")],
            status: "completed".to_string(),
            error_message: None,
            created_at: Utc::now().timestamp_millis(),
        };
        let _ = engine.storage().insert_transfer_history(&record);
        let _ = engine.events().send(CoreEvent::HistoryChanged);

        Ok(saved_str)
    }

    /// Downloads a remote shared folder (or share root) recursively, preserving
    /// subdirectories (including empty subdirectories and empty root folders) in
    /// `download_dir`.
    ///
    /// On Android, when `receive_publish_hook` (`MediaStore.Downloads`) is active:
    /// `MediaStore` only indexes file entries and cannot represent empty directories.
    /// Empty subdirectories alongside files are skipped while all files are published
    /// under `Download/Landa/<folder>/<relPath>`; downloading a completely empty folder
    /// with zero files returns an explicit error because `MediaStore` cannot store it.
    pub async fn download_remote_folder(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        root_id: &str,
        rel_path: &str,
    ) -> Result<ShareFolderDownloadResult, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("shares".into()));
        }
        let clean_req_rel = Self::normalize_share_rel_path(true, rel_path)?;

        let (peer_ctx, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Shares)
            .await?;
        write_frame(
            &mut send,
            &ShareRequest::DownloadFolder {
                root_id: root_id.to_string(),
                rel_path: clean_req_rel,
            },
        )
        .await?;
        let _ = send.finish();

        let resp: ShareResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        let (folder_name, dirs, files, manifest_total_bytes) = match resp {
            ShareResponse::FolderManifest {
                folder_name,
                dirs,
                files,
                total_bytes,
            } => (folder_name, dirs, files, total_bytes),
            ShareResponse::Error(msg) => return Err(CoreError::InvalidInput(msg)),
            _ => return Err(CoreError::InvalidInput("unexpected share response".into())),
        };

        let validated =
            validate_folder_manifest(&folder_name, &dirs, &files, manifest_total_bytes)?;
        let safe_folder = validated.safe_folder;
        let safe_folder_str = validated.safe_folder_str;
        let combined_dirs = validated.combined_dirs;
        let combined_paths = validated.combined_paths;
        let computed_total = validated.total_bytes;

        let transfer_id = Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        engine.register_transfer_cancellation(&transfer_id, cancel.clone());

        let temp_dir = engine
            .app_data_dir()
            .join("incoming_temp")
            .join(format!("share-{}", Uuid::new_v4()));
        let download_dir = PathBuf::from(engine.get_settings().download_dir);
        let _guard = FolderDownloadGuard {
            engine,
            transfer_id: transfer_id.clone(),
            temp_dir: temp_dir.clone(),
        };

        let receive_res = receive_folder_inner(
            engine,
            &transfer_id,
            &peer_ctx,
            &mut recv,
            &safe_folder,
            &combined_dirs,
            &files,
            &combined_paths,
            computed_total,
            &temp_dir,
            &download_dir,
            &cancel,
        )
        .await;

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        match receive_res {
            Ok((saved_paths, published_via_hook)) => {
                let relative_paths: Vec<String> = combined_paths
                    .iter()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .collect();
                let record = TransferHistoryRecord {
                    id: transfer_id.clone(),
                    direction: "receive".to_string(),
                    peer_id: peer_ctx.peer_id.to_hex(),
                    peer_name: peer_ctx.display_name.clone(),
                    file_count: saved_paths.len(),
                    total_bytes: computed_total,
                    saved_paths: saved_paths.clone(),
                    relative_paths,
                    status: "completed".to_string(),
                    error_message: None,
                    created_at: Utc::now().timestamp_millis(),
                };
                let _ = engine.storage().insert_transfer_history(&record);
                let _ = engine.events().send(CoreEvent::HistoryChanged);
                let _ = engine
                    .events()
                    .send(CoreEvent::TransferProgress(TransferProgressInfo {
                        transfer_id,
                        direction: "receive".to_string(),
                        peer_id: peer_ctx.peer_id.to_hex(),
                        peer_name: peer_ctx.display_name,
                        current_file: saved_paths.first().cloned().unwrap_or_default(),
                        files_done: saved_paths.len(),
                        file_count: files.len(),
                        bytes_transferred: computed_total,
                        total_bytes: computed_total,
                        speed_bps: 0,
                        eta_secs: None,
                        status: "completed".to_string(),
                        error: None,
                        saved_paths: saved_paths.clone(),
                    }));

                let destination = if published_via_hook {
                    format!("Downloads/Landa/{safe_folder_str}")
                } else {
                    download_dir
                        .join(&safe_folder)
                        .to_string_lossy()
                        .to_string()
                };

                Ok(ShareFolderDownloadResult {
                    folder_name: safe_folder_str,
                    file_count: saved_paths.len(),
                    total_bytes: computed_total,
                    saved_paths,
                    destination,
                })
            }
            Err(err) => {
                let status = if matches!(err, CoreError::Cancelled) {
                    "cancelled"
                } else {
                    "failed"
                };
                let record = TransferHistoryRecord {
                    id: transfer_id.clone(),
                    direction: "receive".to_string(),
                    peer_id: peer_ctx.peer_id.to_hex(),
                    peer_name: peer_ctx.display_name.clone(),
                    file_count: files.len(),
                    total_bytes: computed_total,
                    saved_paths: Vec::new(),
                    relative_paths: Vec::new(),
                    status: status.to_string(),
                    error_message: Some(err.to_string()),
                    created_at: Utc::now().timestamp_millis(),
                };
                let _ = engine.storage().insert_transfer_history(&record);
                let _ = engine.events().send(CoreEvent::HistoryChanged);
                let _ = engine
                    .events()
                    .send(CoreEvent::TransferProgress(TransferProgressInfo {
                        transfer_id,
                        direction: "receive".to_string(),
                        peer_id: peer_ctx.peer_id.to_hex(),
                        peer_name: peer_ctx.display_name,
                        current_file: String::new(),
                        files_done: 0,
                        file_count: files.len(),
                        bytes_transferred: 0,
                        total_bytes: computed_total,
                        speed_bps: 0,
                        eta_secs: None,
                        status: status.to_string(),
                        error: Some(err.to_string()),
                        saved_paths: Vec::new(),
                    }));
                Err(err)
            }
        }
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
            let n = recv.read(&mut buf[..to_read]).await?.ok_or_else(|| {
                CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "share preview truncated",
                ))
            })?;
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
            return Err(CoreError::InvalidInput(
                "cannot traverse into file share".into(),
            ));
        }
        let safe_rel = validate_relative_path(trimmed)?;
        Ok(safe_rel.to_string_lossy().replace('\\', "/"))
    }

    fn resolve_root_target(
        &self,
        root_id: &str,
        rel_path: &str,
    ) -> Result<(LocalShareRoot, PathBuf), CoreError> {
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

    async fn handle_download_folder(
        &self,
        root_id: &str,
        rel_path: &str,
        send: &mut SendStream,
    ) -> Result<(), CoreError> {
        let root = match self.find_root(root_id) {
            Ok(r) => r,
            Err(err) => {
                send_response(send, &ShareResponse::Error(err.to_string())).await?;
                return Ok(());
            }
        };

        if !root.is_dir {
            send_response(
                send,
                &ShareResponse::Error("target is not a directory".into()),
            )
            .await?;
            return Ok(());
        }

        let clean_rel = match Self::normalize_share_rel_path(root.is_dir, rel_path) {
            Ok(r) => r,
            Err(err) => {
                send_response(send, &ShareResponse::Error(err.to_string())).await?;
                return Ok(());
            }
        };

        let folder_name = if clean_rel.is_empty() {
            sanitize_single_segment(&root.name, "Share")
        } else {
            let last = clean_rel.rsplit('/').next().unwrap_or("Folder");
            sanitize_single_segment(last, "Folder")
        };

        let is_content_uri = root.path.starts_with("content://");
        let (local_root_canon, entries) = match self
            .collect_folder_entries(&root, &clean_rel, is_content_uri)
            .await
        {
            Ok(res) => res,
            Err(err) => {
                send_response(send, &ShareResponse::Error(err.to_string())).await?;
                return Ok(());
            }
        };

        let built = match build_folder_manifest(entries, &clean_rel) {
            Ok(built) => built,
            Err(err) => {
                send_response(send, &ShareResponse::Error(err.to_string())).await?;
                return Ok(());
            }
        };

        let manifest = ShareResponse::FolderManifest {
            folder_name,
            dirs: built.dirs,
            files: built.files.clone(),
            total_bytes: built.total_bytes,
        };
        if let Err(err) = send_response(send, &manifest).await {
            let _ = send_response(send, &ShareResponse::Error(err.to_string())).await;
            return Ok(());
        }

        for (meta, source_rel) in built.files.iter().zip(built.source_rel_paths.iter()) {
            let stream_res: Result<(), CoreError> = async {
                let mut file = if is_content_uri {
                    let provider = self.get_content_provider()?;
                    let uri = root.path.clone();
                    let rel_owned = source_rel.clone();
                    let opened = tokio::task::spawn_blocking(move || {
                        provider.open_file(&uri, true, &rel_owned)
                    })
                    .await
                    .map_err(|e| CoreError::InvalidInput(e.to_string()))??;
                    tokio::fs::File::from_std(opened.file)
                } else {
                    let root_canon = local_root_canon.clone().ok_or_else(|| {
                        CoreError::InvalidInput("missing local share root".into())
                    })?;
                    let rel_owned = source_rel.clone();
                    let std_file = tokio::task::spawn_blocking(move || {
                        open_local_share_file(&root_canon, &rel_owned)
                    })
                    .await
                    .map_err(|e| CoreError::InvalidInput(e.to_string()))??;
                    tokio::fs::File::from_std(std_file)
                };
                stream_exact_file_with_blake3(&mut file, meta.size_bytes, send).await
            }
            .await;

            if let Err(err) = stream_res {
                let _ = send.reset(1u32.into());
                return Err(err);
            }
        }

        Ok(())
    }

    async fn collect_folder_entries(
        &self,
        root: &LocalShareRoot,
        clean_rel: &str,
        is_content_uri: bool,
    ) -> Result<(Option<PathBuf>, Vec<ShareEntry>), CoreError> {
        if is_content_uri {
            let provider = self.get_content_provider()?;
            let uri = root.path.clone();
            let rel_clone = clean_rel.to_string();
            let entries = tokio::task::spawn_blocking(move || {
                provider.list_tree(&uri, true, &rel_clone, MAX_TREE_DEPTH, MAX_TREE_FILES)
            })
            .await
            .map_err(|e| CoreError::InvalidInput(e.to_string()))??;
            let validated =
                validate_tree_entries(entries, clean_rel, MAX_TREE_DEPTH, MAX_TREE_FILES)?;
            return Ok((None, validated));
        }

        let root_path = root.path.clone();
        let rel_clone = clean_rel.to_string();
        let (root_canon, entries) = tokio::task::spawn_blocking(move || {
            let root_canon = std::fs::canonicalize(&root_path)?;
            let target = if rel_clone.is_empty() {
                root_canon.clone()
            } else {
                let candidate = root_canon.join(&rel_clone);
                let target_canon = std::fs::canonicalize(&candidate)?;
                if !target_canon.starts_with(&root_canon) {
                    return Err(CoreError::InvalidInput("path escapes share root".into()));
                }
                target_canon
            };
            if !target.is_dir() {
                return Err(CoreError::InvalidInput("target is not a directory".into()));
            }
            let entries = collect_local_tree(&root_canon, &target, MAX_TREE_DEPTH, MAX_TREE_FILES)?;
            Ok((root_canon, entries))
        })
        .await
        .map_err(|e| CoreError::InvalidInput(e.to_string()))??;

        Ok((Some(root_canon), entries))
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
                            send_response(&mut send, &ShareResponse::Error(err.to_string()))
                                .await?;
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
                            send_response(&mut send, &ShareResponse::Error(err.to_string()))
                                .await?;
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
                            send_response(&mut send, &ShareResponse::Error(err.to_string()))
                                .await?;
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
                            send_response(&mut send, &ShareResponse::Error(err.to_string()))
                                .await?;
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
                            send_response(&mut send, &ShareResponse::Error(err.to_string()))
                                .await?;
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
                                        &ShareResponse::Error(
                                            "image too large for thumbnail".into(),
                                        ),
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
                            send_response(&mut send, &ShareResponse::Error(err.to_string()))
                                .await?;
                        }
                    }
                }
            }
            ShareRequest::DownloadFolder { root_id, rel_path } => {
                self.handle_download_folder(&root_id, &rel_path, &mut send)
                    .await?;
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

struct FolderDownloadGuard<'a> {
    engine: &'a LandaEngine,
    transfer_id: String,
    temp_dir: PathBuf,
}

impl Drop for FolderDownloadGuard<'_> {
    fn drop(&mut self) {
        self.engine
            .unregister_transfer_cancellation(&self.transfer_id);
        let _ = std::fs::remove_dir_all(&self.temp_dir);
    }
}

struct BuiltFolderManifest {
    dirs: Vec<String>,
    files: Vec<ShareFolderFileMeta>,
    source_rel_paths: Vec<String>,
    total_bytes: u64,
}

struct ValidatedFolderManifest {
    safe_folder: PathBuf,
    safe_folder_str: String,
    combined_dirs: Vec<PathBuf>,
    combined_paths: Vec<PathBuf>,
    total_bytes: u64,
}

fn build_folder_manifest(
    entries: Vec<ShareEntry>,
    clean_rel: &str,
) -> Result<BuiltFolderManifest, CoreError> {
    let prefix = if clean_rel.is_empty() {
        String::new()
    } else {
        format!("{clean_rel}/")
    };

    let mut manifest_dirs = Vec::new();
    let mut manifest_files = Vec::new();
    let mut source_rel_paths = Vec::new();
    let mut seen_dirs = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut total_bytes: u64 = 0;

    for entry in entries {
        let manifest_rel = if prefix.is_empty() {
            entry.rel_path.clone()
        } else {
            entry
                .rel_path
                .strip_prefix(&prefix)
                .ok_or_else(|| {
                    CoreError::InvalidInput("entry path escapes requested folder".into())
                })?
                .to_string()
        };
        let safe_rel = validate_relative_path(&manifest_rel)?;
        let clean_manifest_rel = safe_rel.to_string_lossy().replace('\\', "/");
        if entry.is_dir {
            if !seen_dirs.insert(clean_manifest_rel.clone()) {
                return Err(CoreError::InvalidInput(
                    "duplicate directory path in folder manifest".into(),
                ));
            }
            manifest_dirs.push(clean_manifest_rel);
        } else {
            if !seen_files.insert(clean_manifest_rel.clone()) {
                return Err(CoreError::InvalidInput(
                    "duplicate path in folder manifest".into(),
                ));
            }
            total_bytes = total_bytes
                .checked_add(entry.size_bytes)
                .ok_or_else(|| CoreError::InvalidInput("total size overflow".into()))?;
            manifest_files.push(ShareFolderFileMeta {
                rel_path: clean_manifest_rel,
                size_bytes: entry.size_bytes,
            });
            source_rel_paths.push(entry.rel_path);
        }
    }

    Ok(BuiltFolderManifest {
        dirs: manifest_dirs,
        files: manifest_files,
        source_rel_paths,
        total_bytes,
    })
}

fn validate_folder_manifest(
    folder_name: &str,
    dirs: &[String],
    files: &[ShareFolderFileMeta],
    manifest_total_bytes: u64,
) -> Result<ValidatedFolderManifest, CoreError> {
    if dirs.len() > MAX_TREE_FILES {
        return Err(CoreError::InvalidInput(format!(
            "folder exceeds maximum directory count ({MAX_TREE_FILES})"
        )));
    }
    if files.len() > MAX_TREE_FILES {
        return Err(CoreError::InvalidInput(format!(
            "folder exceeds maximum file count ({MAX_TREE_FILES})"
        )));
    }

    let safe_folder = validate_relative_path(folder_name)?;
    if safe_folder.components().count() != 1 {
        return Err(CoreError::InvalidInput(
            "invalid folder name in manifest".into(),
        ));
    }
    let safe_folder_str = safe_folder.to_string_lossy().to_string();

    let mut combined_dirs = Vec::with_capacity(dirs.len());
    let mut seen_dirs = HashSet::with_capacity(dirs.len());
    for dir_rel in dirs {
        let safe_dir_rel = validate_relative_path(dir_rel)?;
        if safe_dir_rel.components().count() > MAX_TREE_DEPTH {
            return Err(CoreError::InvalidInput(format!(
                "folder exceeds maximum depth ({MAX_TREE_DEPTH})"
            )));
        }
        let norm_dir = safe_dir_rel.to_string_lossy().replace('\\', "/");
        if !seen_dirs.insert(norm_dir) {
            return Err(CoreError::InvalidInput(
                "duplicate directory path in folder manifest".into(),
            ));
        }
        combined_dirs.push(safe_folder.join(safe_dir_rel));
    }

    let mut combined_paths = Vec::with_capacity(files.len());
    let mut seen_files = HashSet::with_capacity(files.len());
    let mut computed_total: u64 = 0;
    for meta in files {
        let safe_file_rel = validate_relative_path(&meta.rel_path)?;
        if safe_file_rel.components().count() > MAX_TREE_DEPTH + 1 {
            return Err(CoreError::InvalidInput(format!(
                "folder exceeds maximum depth ({MAX_TREE_DEPTH})"
            )));
        }
        let norm_rel = safe_file_rel.to_string_lossy().replace('\\', "/");
        if seen_dirs.contains(&norm_rel) {
            return Err(CoreError::InvalidInput(
                "path conflicts with directory in folder manifest".into(),
            ));
        }
        if !seen_files.insert(norm_rel) {
            return Err(CoreError::InvalidInput(
                "duplicate path in folder manifest".into(),
            ));
        }
        computed_total = computed_total
            .checked_add(meta.size_bytes)
            .ok_or_else(|| CoreError::InvalidInput("total size overflow".into()))?;
        combined_paths.push(safe_folder.join(safe_file_rel));
    }

    for dir_rel in &seen_dirs {
        let mut prefix = String::new();
        for seg in dir_rel.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(seg);
            if seen_files.contains(&prefix) {
                return Err(CoreError::InvalidInput(
                    "directory path conflicts with file in folder manifest".into(),
                ));
            }
        }
    }
    for file_rel in &seen_files {
        let mut prefix = String::new();
        let parts: Vec<&str> = file_rel.split('/').collect();
        for seg in parts.iter().take(parts.len().saturating_sub(1)) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(seg);
            if seen_files.contains(&prefix) {
                return Err(CoreError::InvalidInput(
                    "file path conflicts with ancestor file in folder manifest".into(),
                ));
            }
        }
    }

    if computed_total != manifest_total_bytes {
        return Err(CoreError::InvalidInput(
            "folder manifest size mismatch".into(),
        ));
    }

    Ok(ValidatedFolderManifest {
        safe_folder,
        safe_folder_str,
        combined_dirs,
        combined_paths,
        total_bytes: computed_total,
    })
}

fn open_local_share_file(root_canon: &Path, rel_path: &str) -> Result<std::fs::File, CoreError> {
    let safe_rel = validate_relative_path(rel_path)?;
    let candidate = root_canon.join(safe_rel);
    let target_canon = std::fs::canonicalize(&candidate)?;
    if !target_canon.starts_with(root_canon) {
        return Err(CoreError::InvalidInput("path escapes share root".into()));
    }
    if !target_canon.is_file() {
        return Err(CoreError::InvalidInput("target is not a file".into()));
    }
    Ok(std::fs::File::open(&target_canon)?)
}

#[allow(clippy::too_many_arguments)]
async fn receive_folder_inner(
    engine: &LandaEngine,
    transfer_id: &str,
    peer: &PeerContext,
    recv: &mut RecvStream,
    safe_folder: &Path,
    combined_dirs: &[PathBuf],
    files: &[ShareFolderFileMeta],
    combined_paths: &[PathBuf],
    total_bytes: u64,
    temp_dir: &Path,
    download_dir: &Path,
    cancel: &CancellationToken,
) -> Result<(Vec<String>, bool), CoreError> {
    tokio::fs::create_dir_all(temp_dir).await?;
    tokio::fs::create_dir_all(download_dir).await?;

    let started = Instant::now();
    let mut last_emit = Instant::now();
    let mut bytes_received: u64 = 0;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut staged_files: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(files.len());

    if let Some(first_rel) = combined_paths.first() {
        emit_folder_progress(
            engine,
            transfer_id,
            peer,
            &first_rel.to_string_lossy().replace('\\', "/"),
            0,
            files.len(),
            0,
            total_bytes,
            started,
        );
    }

    for (idx, (meta, combined_rel)) in files.iter().zip(combined_paths.iter()).enumerate() {
        if cancel.is_cancelled() {
            return Err(CoreError::Cancelled);
        }
        let temp_file_path = resolve_within_root(temp_dir, combined_rel)?;
        if let Some(parent) = temp_file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::File::create(&temp_file_path).await?;
        let mut hasher = blake3::Hasher::new();
        let mut remaining = meta.size_bytes;
        let display_rel = combined_rel.to_string_lossy().replace('\\', "/");

        while remaining > 0 {
            let to_read = (remaining as usize).min(buf.len());
            let read_res = tokio::select! {
                _ = cancel.cancelled() => return Err(CoreError::Cancelled),
                res = recv.read(&mut buf[..to_read]) => res,
            };
            let n = read_res?.ok_or_else(|| {
                CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "share folder download truncated",
                ))
            })?;
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n]).await?;
            remaining -= n as u64;
            bytes_received += n as u64;

            if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                last_emit = Instant::now();
                emit_folder_progress(
                    engine,
                    transfer_id,
                    peer,
                    &display_rel,
                    idx,
                    files.len(),
                    bytes_received,
                    total_bytes,
                    started,
                );
            }
        }
        file.flush().await?;

        let mut expected_digest = [0u8; 32];
        let digest_res = tokio::select! {
            _ = cancel.cancelled() => return Err(CoreError::Cancelled),
            res = recv.read_exact(&mut expected_digest) => res,
        };
        digest_res.map_err(|e| {
            CoreError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, e))
        })?;
        if hasher.finalize().as_bytes() != &expected_digest {
            return Err(CoreError::IntegrityMismatch(meta.rel_path.clone()));
        }

        staged_files.push((temp_file_path, combined_rel.clone()));
    }

    publish_staged_files(
        engine,
        download_dir,
        safe_folder,
        combined_dirs,
        staged_files,
    )
    .await
}

async fn publish_staged_files(
    engine: &LandaEngine,
    download_dir: &Path,
    safe_folder: &Path,
    combined_dirs: &[PathBuf],
    staged_files: Vec<(PathBuf, PathBuf)>,
) -> Result<(Vec<String>, bool), CoreError> {
    let hook = engine.receive_publish_hook();
    let mut saved_paths = Vec::with_capacity(staged_files.len());
    let published_via_hook = hook.is_some();

    if let Some(ref publish) = hook {
        // Android MediaStore.Downloads only indexes file entries and cannot represent
        // empty directories. Empty subdirectories alongside files are skipped, while
        // an empty folder with zero files fails explicitly.
        if staged_files.is_empty() {
            return Err(CoreError::InvalidInput(
                "Android MediaStore cannot store empty folders without files".into(),
            ));
        }
        for (temp_path, combined_rel) in staged_files {
            let publish_fn = Arc::clone(publish);
            let src = temp_path.clone();
            let rel = combined_rel.clone();
            let published = tokio::task::spawn_blocking(move || publish_fn(&src, &rel))
                .await
                .map_err(|e| CoreError::Io(std::io::Error::other(e)))?
                .ok_or_else(|| {
                    CoreError::Io(std::io::Error::other(format!(
                        "failed to publish downloaded file {}",
                        combined_rel.display()
                    )))
                })?;
            saved_paths.push(published);
        }
        return Ok((saved_paths, published_via_hook));
    }

    let root_target = resolve_within_root(download_dir, safe_folder)?;
    tokio::fs::create_dir_all(&root_target).await?;
    for dir_rel in combined_dirs {
        let dir_target = resolve_within_root(download_dir, dir_rel)?;
        tokio::fs::create_dir_all(&dir_target).await?;
    }

    for (temp_path, combined_rel) in staged_files {
        let target = allocate_unique_destination(download_dir, &combined_rel)?;
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if tokio::fs::rename(&temp_path, &target).await.is_err() {
            tokio::fs::copy(&temp_path, &target).await?;
            let _ = tokio::fs::remove_file(&temp_path).await;
        }
        saved_paths.push(target.to_string_lossy().to_string());
    }

    Ok((saved_paths, published_via_hook))
}

#[allow(clippy::too_many_arguments)]
fn emit_folder_progress(
    engine: &LandaEngine,
    transfer_id: &str,
    peer: &PeerContext,
    current_file: &str,
    files_done: usize,
    file_count: usize,
    bytes_transferred: u64,
    total_bytes: u64,
    started: Instant,
) {
    let elapsed = started.elapsed().as_secs_f64().max(0.001);
    let speed_bps = (bytes_transferred as f64 / elapsed) as u64;
    let eta_secs = if speed_bps > 0 && total_bytes > bytes_transferred {
        Some((total_bytes - bytes_transferred) / speed_bps)
    } else {
        None
    };
    let _ = engine
        .events()
        .send(CoreEvent::TransferProgress(TransferProgressInfo {
            transfer_id: transfer_id.to_string(),
            direction: "receive".to_string(),
            peer_id: peer.peer_id.to_hex(),
            peer_name: peer.display_name.clone(),
            current_file: current_file.to_string(),
            files_done,
            file_count,
            bytes_transferred,
            total_bytes,
            speed_bps,
            eta_secs,
            status: "transferring".to_string(),
            error: None,
            saved_paths: Vec::new(),
        }));
}

async fn stream_exact_file_with_blake3(
    file: &mut tokio::fs::File,
    size_bytes: u64,
    send: &mut SendStream,
) -> Result<(), CoreError> {
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut remaining = size_bytes;
    while remaining > 0 {
        let to_read = (remaining as usize).min(buf.len());
        let n = file.read(&mut buf[..to_read]).await?;
        if n == 0 {
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "shared file truncated during folder stream",
            )));
        }
        hasher.update(&buf[..n]);
        send.write_all(&buf[..n]).await?;
        remaining -= n as u64;
    }
    send.write_all(hasher.finalize().as_bytes()).await?;
    Ok(())
}

fn validate_tree_entries(
    entries: Vec<ShareEntry>,
    requested_rel: &str,
    max_depth: usize,
    max_files: usize,
) -> Result<Vec<ShareEntry>, CoreError> {
    let expected_prefix = if requested_rel.is_empty() {
        String::new()
    } else {
        format!("{requested_rel}/")
    };
    let mut out = Vec::with_capacity(entries.len());
    let mut seen_dirs = HashSet::new();
    let mut seen_files = HashSet::new();
    let mut dir_count = 0usize;
    let mut file_count = 0usize;

    for entry in entries {
        let clean = validate_relative_path(&entry.rel_path)?;
        let clean_str = clean.to_string_lossy().replace('\\', "/");
        let rel_to_requested = if expected_prefix.is_empty() {
            clean_str.as_str()
        } else {
            clean_str.strip_prefix(&expected_prefix).ok_or_else(|| {
                CoreError::InvalidInput("share entry path escapes requested directory".into())
            })?
        };
        let rel_components = validate_relative_path(rel_to_requested)?
            .components()
            .count();

        if entry.is_dir {
            if rel_components > max_depth {
                return Err(CoreError::InvalidInput(format!(
                    "folder exceeds maximum depth ({max_depth})"
                )));
            }
            dir_count += 1;
            if dir_count > max_files {
                return Err(CoreError::InvalidInput(format!(
                    "folder exceeds maximum directory count ({max_files})"
                )));
            }
            if !seen_dirs.insert(clean_str.clone()) {
                return Err(CoreError::InvalidInput(
                    "duplicate directory path in share tree".into(),
                ));
            }
        } else {
            if rel_components > max_depth + 1 {
                return Err(CoreError::InvalidInput(format!(
                    "folder exceeds maximum depth ({max_depth})"
                )));
            }
            file_count += 1;
            if file_count > max_files {
                return Err(CoreError::InvalidInput(format!(
                    "folder exceeds maximum file count ({max_files})"
                )));
            }
            if !seen_files.insert(clean_str.clone()) {
                return Err(CoreError::InvalidInput(
                    "duplicate path in share tree".into(),
                ));
            }
        }

        out.push(ShareEntry {
            rel_path: clean_str,
            ..entry
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

fn collect_local_tree(
    root_canon: &Path,
    target_dir: &Path,
    max_depth: usize,
    max_files: usize,
) -> Result<Vec<ShareEntry>, CoreError> {
    let mut out = Vec::new();
    let mut dir_count = 0usize;
    let mut file_count = 0usize;
    let mut stack: Vec<(PathBuf, usize)> = vec![(target_dir.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let canon = std::fs::canonicalize(&path)?;
            if !canon.starts_with(root_canon) {
                return Err(CoreError::InvalidInput("path escapes share root".into()));
            }
            let rel = canon
                .strip_prefix(root_canon)
                .map_err(|_| CoreError::InvalidInput("path escapes share root".into()))?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let safe_rel = validate_relative_path(&rel_str)?;
            let clean_rel = safe_rel.to_string_lossy().replace('\\', "/");

            if file_type.is_dir() {
                if depth >= max_depth {
                    return Err(CoreError::InvalidInput(format!(
                        "folder exceeds maximum depth ({max_depth})"
                    )));
                }
                if dir_count >= max_files {
                    return Err(CoreError::InvalidInput(format!(
                        "folder exceeds maximum directory count ({max_files})"
                    )));
                }
                dir_count += 1;
                let meta = entry.metadata()?;
                out.push(ShareEntry {
                    name: entry.file_name().to_string_lossy().to_string(),
                    rel_path: clean_rel,
                    is_dir: true,
                    size_bytes: 0,
                    modified_ms: modified_ms(&meta),
                });
                stack.push((canon, depth + 1));
            } else if file_type.is_file() {
                if file_count >= max_files {
                    return Err(CoreError::InvalidInput(format!(
                        "folder exceeds maximum file count ({max_files})"
                    )));
                }
                file_count += 1;
                let meta = entry.metadata()?;
                out.push(ShareEntry {
                    name: entry.file_name().to_string_lossy().to_string(),
                    rel_path: clean_rel,
                    is_dir: false,
                    size_bytes: meta.len(),
                    modified_ms: modified_ms(&meta),
                });
            }
        }
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

fn sanitize_single_segment(raw: &str, fallback: &str) -> String {
    let replaced = raw.trim().replace(['/', '\\'], "_");
    if replaced.is_empty() || replaced == "." || replaced == ".." {
        return fallback.to_string();
    }
    let sanitized = sanitize_relative_path(&replaced);
    let seg = sanitized
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or(fallback)
        .to_string();
    if seg.is_empty() {
        fallback.to_string()
    } else {
        seg
    }
}

fn allocate_unique_destination(root: &Path, rel_path: &Path) -> Result<PathBuf, CoreError> {
    let candidate = resolve_within_root(root, rel_path)?;
    if !candidate.exists() {
        return Ok(candidate);
    }
    let parent = candidate.parent().unwrap_or(root);
    let stem = candidate
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = candidate.extension().and_then(|e| e.to_str());

    for idx in 1..10_000 {
        let name = match ext {
            Some(e) if !e.is_empty() => format!("{stem} ({idx}).{e}"),
            _ => format!("{stem} ({idx})"),
        };
        let next = parent.join(name);
        if !next.exists() {
            return Ok(next);
        }
    }
    Err(CoreError::InvalidInput(
        "too many filename collisions".into(),
    ))
}
