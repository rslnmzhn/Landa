use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use chrono::Utc;
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

#[derive(Debug, Serialize, Deserialize)]
enum ShareRequest {
    ListRoots,
    ListDir { root_id: String, rel_path: String },
    DownloadFile { root_id: String, rel_path: String },
}

#[derive(Debug, Serialize, Deserialize)]
enum ShareResponse {
    Roots(Vec<ShareRootInfo>),
    Entries(Vec<ShareEntry>),
    FileHeader { file_name: String, size_bytes: u64 },
    Error(String),
}

pub struct SharesModule {
    storage: Arc<Storage>,
    enabled: AtomicBool,
}

impl SharesModule {
    pub fn new(storage: Arc<Storage>) -> Arc<Self> {
        Arc::new(Self {
            storage,
            enabled: AtomicBool::new(true),
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

    fn resolve_root_target(&self, root_id: &str, rel_path: &str) -> Result<(LocalShareRoot, PathBuf), CoreError> {
        let roots = self.list_local_shares()?;
        let root = roots
            .into_iter()
            .find(|r| r.id == root_id)
            .ok_or_else(|| CoreError::NotFound("share root".into()))?;
        let root_canon = std::fs::canonicalize(&root.path)?;

        let trimmed = rel_path.trim();
        if trimmed.is_empty() || trimmed == "." || trimmed == "/" {
            return Ok((root, root_canon));
        }
        if !root.is_dir {
            return Err(CoreError::InvalidInput("cannot traverse into file share".into()));
        }

        let safe_rel = validate_relative_path(trimmed)?;
        let candidate = root_canon.join(&safe_rel);
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
            ShareRequest::DownloadFile { root_id, rel_path } => {
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

        let _ = send.finish();
        Ok(())
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
