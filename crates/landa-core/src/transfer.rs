use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use landa_proto::path_policy::{resolve_within_root, sanitize_relative_path, validate_relative_path};
use landa_proto::wire::{
    read_frame, write_frame, TransferDecision, TransferFileMeta, TransferOffer, TransferResultAck,
    MAX_CONTROL_FRAME_BYTES,
};
use quinn::{RecvStream, SendStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::CoreError;
use crate::events::{
    CoreEvent, IncomingTransferOfferInfo, TransferHistoryRecord, TransferProgressInfo,
};
use crate::module::PeerContext;
use crate::storage::Storage;

const CHUNK_SIZE: usize = 64 * 1024;
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(100);
const OFFER_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct OutgoingFileItem {
    pub source_path: PathBuf,
    pub source_uri: Option<String>,
    pub relative_path: String,
    pub size_bytes: u64,
}

pub type ReceivePublishHook = Arc<dyn Fn(&Path, &Path) -> Option<String> + Send + Sync>;
pub type OpenSourceHook = Arc<dyn Fn(&str) -> Option<std::fs::File> + Send + Sync>;

pub struct TransferManager {
    storage: Arc<Storage>,
    events: broadcast::Sender<CoreEvent>,
    temp_root: PathBuf,
    pending_offers: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    active_cancellations: Mutex<HashMap<String, CancellationToken>>,
    publish_hook: RwLock<Option<ReceivePublishHook>>,
    open_source_hook: RwLock<Option<OpenSourceHook>>,
}

impl TransferManager {
    pub fn new(
        storage: Arc<Storage>,
        events: broadcast::Sender<CoreEvent>,
        temp_root: PathBuf,
    ) -> Self {
        Self {
            storage,
            events,
            temp_root,
            pending_offers: Mutex::new(HashMap::new()),
            active_cancellations: Mutex::new(HashMap::new()),
            publish_hook: RwLock::new(None),
            open_source_hook: RwLock::new(None),
        }
    }

    pub fn set_publish_hook(&self, hook: ReceivePublishHook) {
        *self.publish_hook.write().expect("publish_hook lock poisoned") = Some(hook);
    }

    pub fn set_open_source_hook(&self, hook: OpenSourceHook) {
        *self
            .open_source_hook
            .write()
            .expect("open_source_hook lock poisoned") = Some(hook);
    }

    pub fn respond_to_offer(&self, transfer_id: &str, accept: bool) -> Result<(), CoreError> {
        let tx = self
            .pending_offers
            .lock()
            .expect("pending_offers lock poisoned")
            .remove(transfer_id)
            .ok_or_else(|| CoreError::NotFound(format!("offer {transfer_id}")))?;
        let _ = tx.send(accept);
        Ok(())
    }

    pub fn cancel_transfer(&self, transfer_id: &str) -> bool {
        if let Some(token) = self
            .active_cancellations
            .lock()
            .expect("active_cancellations lock poisoned")
            .get(transfer_id)
        {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub fn collect_outgoing_items(paths: &[PathBuf]) -> Result<Vec<OutgoingFileItem>, CoreError> {
        let mut items = Vec::new();
        for path in paths {
            if !path.exists() {
                return Err(CoreError::NotFound(path.display().to_string()));
            }
            if path.is_file() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| CoreError::InvalidInput("invalid file name".into()))?;
                let rel = sanitize_relative_path(name);
                let meta = std::fs::metadata(path)?;
                items.push(OutgoingFileItem {
                    source_path: path.clone(),
                    source_uri: None,
                    relative_path: rel.to_string_lossy().replace('\\', "/"),
                    size_bytes: meta.len(),
                });
            } else if path.is_dir() {
                let root_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("folder");
                collect_dir_recursive(path, path, root_name, &mut items)?;
            }
        }
        if items.is_empty() {
            return Err(CoreError::InvalidInput("no files selected".into()));
        }
        Ok(items)
    }

    pub async fn send_files(
        &self,
        peer: &PeerContext,
        mut send: SendStream,
        mut recv: RecvStream,
        items: Vec<OutgoingFileItem>,
    ) -> Result<TransferHistoryRecord, CoreError> {
        let transfer_id = Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        self.active_cancellations
            .lock()
            .expect("active_cancellations lock poisoned")
            .insert(transfer_id.clone(), cancel.clone());

        let result = self
            .send_files_inner(&transfer_id, peer, &mut send, &mut recv, &items, &cancel)
            .await;

        self.active_cancellations
            .lock()
            .expect("active_cancellations lock poisoned")
            .remove(&transfer_id);

        let total_bytes: u64 = items.iter().map(|i| i.size_bytes).sum();
        let saved_paths: Vec<String> = items
            .iter()
            .map(|i| i.source_path.to_string_lossy().to_string())
            .collect();

        let (status, error_message) = match &result {
            Ok(()) => ("completed".to_string(), None),
            Err(CoreError::Cancelled) => ("cancelled".to_string(), Some("Cancelled".into())),
            Err(CoreError::Rejected) => ("rejected".to_string(), Some("Rejected by peer".into())),
            Err(err) => ("failed".to_string(), Some(err.to_string())),
        };

        let record = TransferHistoryRecord {
            id: transfer_id.clone(),
            direction: "send".to_string(),
            peer_id: peer.peer_id.to_hex(),
            peer_name: peer.display_name.clone(),
            file_count: items.len(),
            total_bytes,
            saved_paths,
            status: status.clone(),
            error_message: error_message.clone(),
            created_at: Utc::now().timestamp_millis(),
        };
        let _ = self.storage.insert_transfer_history(&record);
        let _ = self.events.send(CoreEvent::HistoryChanged);

        let _ = self.events.send(CoreEvent::TransferProgress(TransferProgressInfo {
            transfer_id,
            direction: "send".to_string(),
            peer_id: peer.peer_id.to_hex(),
            peer_name: peer.display_name.clone(),
            current_file: String::new(),
            files_done: if result.is_ok() { items.len() } else { 0 },
            file_count: items.len(),
            bytes_transferred: if result.is_ok() { total_bytes } else { 0 },
            total_bytes,
            speed_bps: 0,
            eta_secs: None,
            status,
            error: error_message,
            saved_paths: Vec::new(),
        }));

        result.map(|()| record)
    }

    async fn send_files_inner(
        &self,
        transfer_id: &str,
        peer: &PeerContext,
        send: &mut SendStream,
        recv: &mut RecvStream,
        items: &[OutgoingFileItem],
        cancel: &CancellationToken,
    ) -> Result<(), CoreError> {
        let total_bytes: u64 = items.iter().map(|i| i.size_bytes).sum();
        let offer = TransferOffer {
            transfer_id: transfer_id.to_string(),
            files: items
                .iter()
                .map(|i| TransferFileMeta {
                    relative_path: i.relative_path.clone(),
                    size_bytes: i.size_bytes,
                })
                .collect(),
            total_bytes,
        };
        write_frame(send, &offer).await?;

        let _ = self.events.send(CoreEvent::TransferProgress(TransferProgressInfo {
            transfer_id: transfer_id.to_string(),
            direction: "send".to_string(),
            peer_id: peer.peer_id.to_hex(),
            peer_name: peer.display_name.clone(),
            current_file: items.first().map(|i| i.relative_path.clone()).unwrap_or_default(),
            files_done: 0,
            file_count: items.len(),
            bytes_transferred: 0,
            total_bytes,
            speed_bps: 0,
            eta_secs: None,
            status: "waiting_approval".to_string(),
            error: None,
            saved_paths: Vec::new(),
        }));

        let decision: TransferDecision = tokio::select! {
            _ = cancel.cancelled() => return Err(CoreError::Cancelled),
            res = read_frame(recv, MAX_CONTROL_FRAME_BYTES) => res?,
        };
        if !decision.accepted {
            return Err(CoreError::Rejected);
        }

        let started = Instant::now();
        let mut last_emit = Instant::now();
        let mut bytes_sent: u64 = 0;
        let mut buf = vec![0u8; CHUNK_SIZE];

        for (idx, item) in items.iter().enumerate() {
            let mut file = if let Some(ref uri) = item.source_uri {
                let hook = self
                    .open_source_hook
                    .read()
                    .expect("open_source_hook lock poisoned")
                    .clone()
                    .ok_or_else(|| {
                        CoreError::InvalidInput("content URI source hook not configured".into())
                    })?;
                let uri_owned = uri.clone();
                let std_file = tokio::task::spawn_blocking(move || hook(&uri_owned))
                    .await
                    .map_err(|e| CoreError::Io(std::io::Error::other(e)))?
                    .ok_or_else(|| CoreError::NotFound(format!("unable to open {uri}")))?;
                tokio::fs::File::from_std(std_file)
            } else {
                tokio::fs::File::open(&item.source_path).await?
            };
            let mut hasher = blake3::Hasher::new();
            let mut remaining = item.size_bytes;

            while remaining > 0 {
                if cancel.is_cancelled() {
                    return Err(CoreError::Cancelled);
                }
                let to_read = (remaining as usize).min(buf.len());
                let n = file.read(&mut buf[..to_read]).await?;
                if n == 0 {
                    return Err(CoreError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "file truncated during send",
                    )));
                }
                hasher.update(&buf[..n]);
                send.write_all(&buf[..n]).await?;
                remaining -= n as u64;
                bytes_sent += n as u64;

                if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                    last_emit = Instant::now();
                    emit_progress(
                        &self.events,
                        transfer_id,
                        "send",
                        peer,
                        &item.relative_path,
                        idx,
                        items.len(),
                        bytes_sent,
                        total_bytes,
                        started,
                    );
                }
            }

            let digest = hasher.finalize();
            send.write_all(digest.as_bytes()).await?;
        }

        send.finish().map_err(|e| CoreError::InvalidInput(e.to_string()))?;

        let ack: TransferResultAck = read_frame(recv, MAX_CONTROL_FRAME_BYTES).await?;
        if !ack.ok {
            return Err(CoreError::IntegrityMismatch(
                ack.message.unwrap_or_else(|| "peer verification failed".into()),
            ));
        }

        Ok(())
    }

    pub async fn handle_incoming_transfer(
        &self,
        peer: PeerContext,
        mut send: SendStream,
        mut recv: RecvStream,
        download_dir: PathBuf,
        auto_accept_from_friends: bool,
    ) -> Result<(), CoreError> {
        let offer: TransferOffer = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;

        if offer.files.is_empty() {
            let _ = send_decision(&mut send, false, Some("empty file list".into())).await;
            return Err(CoreError::InvalidInput("empty transfer offer".into()));
        }

        // Validate all relative paths upfront before asking the user
        let mut sanitized_paths = Vec::with_capacity(offer.files.len());
        let mut computed_total: u64 = 0;
        for meta in &offer.files {
            let rel = validate_relative_path(&meta.relative_path)?;
            computed_total = computed_total
                .checked_add(meta.size_bytes)
                .ok_or_else(|| CoreError::InvalidInput("total size overflow".into()))?;
            sanitized_paths.push(rel);
        }

        let transfer_id = sanitize_transfer_id(&offer.transfer_id);
        let local_temp_id = Uuid::new_v4().to_string();
        let accepted = if auto_accept_from_friends && peer.is_trusted {
            true
        } else {
            let (tx, rx) = oneshot::channel();
            self.pending_offers
                .lock()
                .expect("pending_offers lock poisoned")
                .insert(transfer_id.clone(), tx);

            let _ = self.events.send(CoreEvent::IncomingTransferOffer(
                IncomingTransferOfferInfo {
                    transfer_id: transfer_id.clone(),
                    peer_id: peer.peer_id.to_hex(),
                    peer_name: peer.display_name.clone(),
                    file_count: offer.files.len(),
                    total_bytes: computed_total,
                    file_names: offer
                        .files
                        .iter()
                        .take(5)
                        .map(|f| f.relative_path.clone())
                        .collect(),
                },
            ));

            let decision = tokio::time::timeout(OFFER_TIMEOUT, rx).await;
            self.pending_offers
                .lock()
                .expect("pending_offers lock poisoned")
                .remove(&transfer_id);

            matches!(decision, Ok(Ok(true)))
        };

        if !accepted {
            let _ = send_decision(&mut send, false, Some("declined".into())).await;
            return Err(CoreError::Rejected);
        }

        send_decision(&mut send, true, None).await?;

        let cancel = CancellationToken::new();
        self.active_cancellations
            .lock()
            .expect("active_cancellations lock poisoned")
            .insert(transfer_id.clone(), cancel.clone());

        let temp_dir = self.temp_root.join(&local_temp_id);
        let receive_res = self
            .receive_files_inner(
                &transfer_id,
                &peer,
                &mut recv,
                &offer,
                &sanitized_paths,
                computed_total,
                &temp_dir,
                &download_dir,
                &cancel,
            )
            .await;

        self.active_cancellations
            .lock()
            .expect("active_cancellations lock poisoned")
            .remove(&transfer_id);

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        match &receive_res {
            Ok(saved_paths) => {
                let ack = TransferResultAck {
                    ok: true,
                    message: None,
                };
                let _ = write_frame(&mut send, &ack).await;
                let _ = send.finish();

                let record = TransferHistoryRecord {
                    id: transfer_id.clone(),
                    direction: "receive".to_string(),
                    peer_id: peer.peer_id.to_hex(),
                    peer_name: peer.display_name.clone(),
                    file_count: saved_paths.len(),
                    total_bytes: computed_total,
                    saved_paths: saved_paths.clone(),
                    status: "completed".to_string(),
                    error_message: None,
                    created_at: Utc::now().timestamp_millis(),
                };
                let _ = self.storage.insert_transfer_history(&record);
                let _ = self.events.send(CoreEvent::HistoryChanged);
                let _ = self.events.send(CoreEvent::TransferProgress(TransferProgressInfo {
                    transfer_id,
                    direction: "receive".to_string(),
                    peer_id: peer.peer_id.to_hex(),
                    peer_name: peer.display_name.clone(),
                    current_file: saved_paths.first().cloned().unwrap_or_default(),
                    files_done: saved_paths.len(),
                    file_count: offer.files.len(),
                    bytes_transferred: computed_total,
                    total_bytes: computed_total,
                    speed_bps: 0,
                    eta_secs: None,
                    status: "completed".to_string(),
                    error: None,
                    saved_paths: saved_paths.clone(),
                }));
            }
            Err(err) => {
                let ack = TransferResultAck {
                    ok: false,
                    message: Some(err.to_string()),
                };
                let _ = write_frame(&mut send, &ack).await;
                let _ = send.finish();

                let status = if matches!(err, CoreError::Cancelled) {
                    "cancelled"
                } else {
                    "failed"
                };
                let record = TransferHistoryRecord {
                    id: transfer_id.clone(),
                    direction: "receive".to_string(),
                    peer_id: peer.peer_id.to_hex(),
                    peer_name: peer.display_name.clone(),
                    file_count: offer.files.len(),
                    total_bytes: computed_total,
                    saved_paths: Vec::new(),
                    status: status.to_string(),
                    error_message: Some(err.to_string()),
                    created_at: Utc::now().timestamp_millis(),
                };
                let _ = self.storage.insert_transfer_history(&record);
                let _ = self.events.send(CoreEvent::HistoryChanged);
                let _ = self.events.send(CoreEvent::TransferProgress(TransferProgressInfo {
                    transfer_id,
                    direction: "receive".to_string(),
                    peer_id: peer.peer_id.to_hex(),
                    peer_name: peer.display_name.clone(),
                    current_file: String::new(),
                    files_done: 0,
                    file_count: offer.files.len(),
                    bytes_transferred: 0,
                    total_bytes: computed_total,
                    speed_bps: 0,
                    eta_secs: None,
                    status: status.to_string(),
                    error: Some(err.to_string()),
                    saved_paths: Vec::new(),
                }));
            }
        }

        receive_res.map(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    async fn receive_files_inner(
        &self,
        transfer_id: &str,
        peer: &PeerContext,
        recv: &mut RecvStream,
        offer: &TransferOffer,
        sanitized_paths: &[PathBuf],
        total_bytes: u64,
        temp_dir: &Path,
        download_dir: &Path,
        cancel: &CancellationToken,
    ) -> Result<Vec<String>, CoreError> {
        tokio::fs::create_dir_all(temp_dir).await?;
        tokio::fs::create_dir_all(download_dir).await?;

        let started = Instant::now();
        let mut last_emit = Instant::now();
        let mut bytes_received: u64 = 0;
        let mut buf = vec![0u8; CHUNK_SIZE];
        let mut staged_files: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(offer.files.len());

        if let Some(first) = offer.files.first() {
            emit_progress(
                &self.events,
                transfer_id,
                "receive",
                peer,
                &first.relative_path,
                0,
                offer.files.len(),
                0,
                total_bytes,
                started,
            );
        }

        for (idx, (meta, rel_path)) in offer.files.iter().zip(sanitized_paths.iter()).enumerate() {
            let temp_file_path = resolve_within_root(temp_dir, rel_path)?;
            if let Some(parent) = temp_file_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            let mut file = tokio::fs::File::create(&temp_file_path).await?;
            let mut hasher = blake3::Hasher::new();
            let mut remaining = meta.size_bytes;

            while remaining > 0 {
                if cancel.is_cancelled() {
                    return Err(CoreError::Cancelled);
                }
                let to_read = (remaining as usize).min(buf.len());
                let n = recv
                    .read(&mut buf[..to_read])
                    .await?
                    .ok_or_else(|| CoreError::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "stream ended before file completed",
                    )))?;
                hasher.update(&buf[..n]);
                file.write_all(&buf[..n]).await?;
                remaining -= n as u64;
                bytes_received += n as u64;

                if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                    last_emit = Instant::now();
                    emit_progress(
                        &self.events,
                        transfer_id,
                        "receive",
                        peer,
                        &meta.relative_path,
                        idx,
                        offer.files.len(),
                        bytes_received,
                        total_bytes,
                        started,
                    );
                }
            }
            file.flush().await?;

            let mut expected_digest = [0u8; 32];
            recv.read_exact(&mut expected_digest)
                .await
                .map_err(|e| CoreError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, e)))?;
            let actual_digest = hasher.finalize();
            if actual_digest.as_bytes() != &expected_digest {
                return Err(CoreError::IntegrityMismatch(meta.relative_path.clone()));
            }

            staged_files.push((temp_file_path, rel_path.clone()));
        }

        let hook = self
            .publish_hook
            .read()
            .expect("publish_hook lock poisoned")
            .clone();

        let mut saved_paths = Vec::with_capacity(staged_files.len());
        for (temp_path, rel_path) in staged_files {
            if let Some(ref publish) = hook {
                let publish_fn = Arc::clone(publish);
                let src = temp_path.clone();
                let rel = rel_path.clone();
                if let Ok(Some(published)) =
                    tokio::task::spawn_blocking(move || publish_fn(&src, &rel)).await
                {
                    saved_paths.push(published);
                    continue;
                }
            }

            let target = allocate_unique_destination(download_dir, &rel_path)?;
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            if tokio::fs::rename(&temp_path, &target).await.is_err() {
                tokio::fs::copy(&temp_path, &target).await?;
                let _ = tokio::fs::remove_file(&temp_path).await;
            }
            saved_paths.push(target.to_string_lossy().to_string());
        }

        Ok(saved_paths)
    }
}

fn sanitize_transfer_id(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if cleaned.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        cleaned
    }
}

async fn send_decision(
    send: &mut SendStream,
    accepted: bool,
    reason: Option<String>,
) -> Result<(), CoreError> {
    let decision = TransferDecision { accepted, reason };
    write_frame(send, &decision).await?;
    Ok(())
}

fn collect_dir_recursive(
    base_dir: &Path,
    current_dir: &Path,
    root_name: &str,
    out: &mut Vec<OutgoingFileItem>,
) -> Result<(), CoreError> {
    for entry in std::fs::read_dir(current_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_dir_recursive(base_dir, &path, root_name, out)?;
        } else if file_type.is_file() {
            let rel_from_base = path
                .strip_prefix(base_dir)
                .map_err(|e| CoreError::InvalidInput(e.to_string()))?;
            let combined = format!(
                "{}/{}",
                root_name,
                rel_from_base.to_string_lossy().replace('\\', "/")
            );
            let sanitized = sanitize_relative_path(&combined);
            let meta = entry.metadata()?;
            out.push(OutgoingFileItem {
                source_path: path,
                source_uri: None,
                relative_path: sanitized.to_string_lossy().replace('\\', "/"),
                size_bytes: meta.len(),
            });
        }
    }
    Ok(())
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
    Err(CoreError::InvalidInput("too many filename collisions".into()))
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    events: &broadcast::Sender<CoreEvent>,
    transfer_id: &str,
    direction: &str,
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
    let _ = events.send(CoreEvent::TransferProgress(TransferProgressInfo {
        transfer_id: transfer_id.to_string(),
        direction: direction.to_string(),
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
