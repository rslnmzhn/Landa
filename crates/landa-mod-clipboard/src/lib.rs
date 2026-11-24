use std::collections::{HashSet, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use image::{ImageFormat, ImageReader, Limits, RgbaImage};
use landa_core::{CoreError, CoreEvent, LandaEngine, LandaModule, PeerContext, Storage};
use landa_proto::wire::{read_frame, write_frame, StreamKind, MAX_CONTROL_FRAME_BYTES};
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

const CLIPBOARD_SETTING_KEY: &str = "clipboard_history";
const MAX_HISTORY_ITEMS: usize = 30;
const MAX_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 8192;
const STREAM_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardContentKind {
    #[default]
    Text,
    Image,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardItem {
    pub id: String,
    #[serde(default)]
    pub kind: ClipboardContentKind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub image_path: Option<String>,
    #[serde(default)]
    pub image_width: Option<u32>,
    #[serde(default)]
    pub image_height: Option<u32>,
    #[serde(default)]
    pub size_bytes: u64,
    pub content_hash: String,
    pub source_peer_id: Option<String>,
    pub source_peer_name: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
enum ClipboardRequest {
    FetchLatest,
    PushText {
        text: String,
    },
    PushImage {
        size_bytes: u64,
        width: u32,
        height: u32,
        content_hash: String,
    },
    FetchImage {
        item_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
enum ClipboardResponse {
    Items(Vec<ClipboardItem>),
    ImageHeader {
        size_bytes: u64,
        width: u32,
        height: u32,
        content_hash: String,
    },
    Ack,
    Error(String),
}

pub struct ClipboardModule {
    storage: Arc<Storage>,
    events: broadcast::Sender<CoreEvent>,
    images_dir: PathBuf,
    enabled: AtomicBool,
    items: Mutex<VecDeque<ClipboardItem>>,
}

impl ClipboardModule {
    pub fn new(
        storage: Arc<Storage>,
        events: broadcast::Sender<CoreEvent>,
        images_dir: PathBuf,
    ) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&images_dir);
        let mut initial: Vec<ClipboardItem> = storage
            .get_setting_json::<Vec<ClipboardItem>>(CLIPBOARD_SETTING_KEY)
            .ok()
            .flatten()
            .unwrap_or_default();

        initial.retain(|item| match item.kind {
            ClipboardContentKind::Text => true,
            ClipboardContentKind::Image => item
                .image_path
                .as_deref()
                .map(|p| Path::new(p).is_file())
                .unwrap_or(false),
        });

        sweep_orphan_images(&images_dir, &initial);

        Arc::new(Self {
            storage,
            events,
            images_dir,
            enabled: AtomicBool::new(true),
            items: Mutex::new(VecDeque::from(initial)),
        })
    }

    pub fn images_dir(&self) -> &Path {
        &self.images_dir
    }

    pub fn list_local_items(&self) -> Vec<ClipboardItem> {
        self.items
            .lock()
            .expect("clipboard lock poisoned")
            .iter()
            .cloned()
            .collect()
    }

    pub fn get_local_item(&self, id: &str) -> Option<ClipboardItem> {
        self.items
            .lock()
            .expect("clipboard lock poisoned")
            .iter()
            .find(|item| item.id == id)
            .cloned()
    }

    pub fn push_local_text(
        &self,
        text: String,
        source_peer_id: Option<String>,
        source_peer_name: Option<String>,
    ) -> Result<ClipboardItem, CoreError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(CoreError::InvalidInput("clipboard text is empty".into()));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(CoreError::InvalidInput(
                "clipboard text exceeds 256 KB".into(),
            ));
        }

        let hash = blake3::hash(trimmed.as_bytes()).to_hex().to_string();
        let size_bytes = text.len() as u64;
        let item = ClipboardItem {
            id: Uuid::new_v4().to_string(),
            kind: ClipboardContentKind::Text,
            text,
            image_path: None,
            image_width: None,
            image_height: None,
            size_bytes,
            content_hash: hash.clone(),
            source_peer_id,
            source_peer_name,
            created_at: Utc::now().timestamp_millis(),
        };

        self.insert_item(item.clone());
        Ok(item)
    }

    pub fn push_local_image(
        &self,
        raw_bytes: &[u8],
        source_peer_id: Option<String>,
        source_peer_name: Option<String>,
    ) -> Result<ClipboardItem, CoreError> {
        let (png_bytes, width, height) = normalize_to_png(raw_bytes)?;
        self.store_verified_png(png_bytes, width, height, source_peer_id, source_peer_name)
    }

    pub fn push_local_rgba_image(
        &self,
        width: u32,
        height: u32,
        rgba_bytes: &[u8],
    ) -> Result<ClipboardItem, CoreError> {
        if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION
        {
            return Err(CoreError::InvalidInput(
                "clipboard image dimensions out of bounds".into(),
            ));
        }
        let img = RgbaImage::from_raw(width, height, rgba_bytes.to_vec())
            .ok_or_else(|| CoreError::InvalidInput("invalid RGBA clipboard image buffer".into()))?;
        let mut png_bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
            .map_err(|e| CoreError::InvalidInput(format!("failed to encode PNG: {e}")))?;
        if png_bytes.len() > MAX_IMAGE_BYTES {
            return Err(CoreError::InvalidInput(
                "clipboard image exceeds 10 MB limit".into(),
            ));
        }
        self.store_verified_png(png_bytes, width, height, None, None)
    }

    fn store_verified_png(
        &self,
        png_bytes: Vec<u8>,
        width: u32,
        height: u32,
        source_peer_id: Option<String>,
        source_peer_name: Option<String>,
    ) -> Result<ClipboardItem, CoreError> {
        std::fs::create_dir_all(&self.images_dir)?;
        let id = Uuid::new_v4().to_string();
        let hash = blake3::hash(&png_bytes).to_hex().to_string();
        let size_bytes = png_bytes.len() as u64;
        let file_path = self.images_dir.join(format!("{id}.png"));
        std::fs::write(&file_path, &png_bytes)?;

        let item = ClipboardItem {
            id,
            kind: ClipboardContentKind::Image,
            text: format!("PNG {width}×{height}"),
            image_path: Some(file_path.to_string_lossy().to_string()),
            image_width: Some(width),
            image_height: Some(height),
            size_bytes,
            content_hash: hash,
            source_peer_id,
            source_peer_name,
            created_at: Utc::now().timestamp_millis(),
        };

        self.insert_item(item.clone());
        Ok(item)
    }

    fn insert_item(&self, item: ClipboardItem) {
        let mut removed_paths = Vec::new();
        {
            let mut guard = self.items.lock().expect("clipboard lock poisoned");
            guard.retain(|existing| {
                if existing.content_hash == item.content_hash {
                    if let Some(ref p) = existing.image_path {
                        if item.image_path.as_deref() != Some(p.as_str()) {
                            removed_paths.push(PathBuf::from(p));
                        }
                    }
                    false
                } else {
                    true
                }
            });
            guard.push_front(item.clone());
            while guard.len() > MAX_HISTORY_ITEMS {
                if let Some(evicted) = guard.pop_back() {
                    if let Some(p) = evicted.image_path {
                        removed_paths.push(PathBuf::from(p));
                    }
                }
            }
            let snapshot: Vec<ClipboardItem> = guard.iter().cloned().collect();
            let _ = self
                .storage
                .set_setting_json(CLIPBOARD_SETTING_KEY, &snapshot);
        }

        for path in removed_paths {
            self.remove_image_file_if_owned(&path);
        }

        let _ = self.events.send(CoreEvent::ModuleEvent {
            module_id: "clipboard".to_string(),
            event_name: "updated".to_string(),
            data: serde_json::to_value(&item).unwrap_or(serde_json::Value::Null),
        });
    }

    pub fn delete_local_item(&self, id: &str) {
        let mut removed_paths = Vec::new();
        {
            let mut guard = self.items.lock().expect("clipboard lock poisoned");
            guard.retain(|item| {
                if item.id == id {
                    if let Some(ref p) = item.image_path {
                        removed_paths.push(PathBuf::from(p));
                    }
                    false
                } else {
                    true
                }
            });
            let snapshot: Vec<ClipboardItem> = guard.iter().cloned().collect();
            let _ = self
                .storage
                .set_setting_json(CLIPBOARD_SETTING_KEY, &snapshot);
        }

        for path in removed_paths {
            self.remove_image_file_if_owned(&path);
        }

        let _ = self.events.send(CoreEvent::ModuleEvent {
            module_id: "clipboard".to_string(),
            event_name: "updated".to_string(),
            data: serde_json::Value::Null,
        });
    }

    fn remove_image_file_if_owned(&self, path: &Path) {
        if let (Ok(canon_dir), Ok(canon_file)) =
            (self.images_dir.canonicalize(), path.canonicalize())
        {
            if canon_file.starts_with(&canon_dir) {
                let _ = std::fs::remove_file(canon_file);
            }
        } else if path.parent() == Some(self.images_dir.as_path()) {
            let _ = std::fs::remove_file(path);
        }
    }

    pub async fn send_text_to_peer(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        text: String,
    ) -> Result<(), CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("clipboard".into()));
        }
        if text.trim().is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(CoreError::InvalidInput(
                "invalid clipboard text size".into(),
            ));
        }
        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Clipboard)
            .await?;
        write_frame(&mut send, &ClipboardRequest::PushText { text }).await?;
        let _ = send.finish();

        let resp: ClipboardResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        match resp {
            ClipboardResponse::Ack => Ok(()),
            ClipboardResponse::Error(msg) => Err(CoreError::InvalidInput(msg)),
            _ => Err(CoreError::InvalidInput(
                "unexpected clipboard response".into(),
            )),
        }
    }

    pub async fn send_local_image_to_peer(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        item_id: &str,
    ) -> Result<(), CoreError> {
        let item = self
            .get_local_item(item_id)
            .ok_or_else(|| CoreError::NotFound(format!("clipboard item {item_id}")))?;
        if item.kind != ClipboardContentKind::Image {
            return Err(CoreError::InvalidInput(
                "clipboard item is not an image".into(),
            ));
        }
        let path = item
            .image_path
            .ok_or_else(|| CoreError::NotFound("clipboard image path missing".into()))?;
        let bytes = tokio::fs::read(&path).await?;
        self.send_image_bytes_to_peer(engine, peer_id, &bytes).await
    }

    pub async fn send_image_bytes_to_peer(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        raw_bytes: &[u8],
    ) -> Result<(), CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("clipboard".into()));
        }
        let (png_bytes, width, height) = normalize_to_png(raw_bytes)?;
        let content_hash = blake3::hash(&png_bytes).to_hex().to_string();
        let size_bytes = png_bytes.len() as u64;

        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Clipboard)
            .await?;
        write_frame(
            &mut send,
            &ClipboardRequest::PushImage {
                size_bytes,
                width,
                height,
                content_hash,
            },
        )
        .await?;

        stream_bytes_with_digest(&mut send, &png_bytes).await?;
        let _ = send.finish();

        let resp: ClipboardResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        match resp {
            ClipboardResponse::Ack => Ok(()),
            ClipboardResponse::Error(msg) => Err(CoreError::InvalidInput(msg)),
            _ => Err(CoreError::InvalidInput(
                "unexpected clipboard response".into(),
            )),
        }
    }

    pub async fn fetch_remote_items(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
    ) -> Result<Vec<ClipboardItem>, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("clipboard".into()));
        }
        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Clipboard)
            .await?;
        write_frame(&mut send, &ClipboardRequest::FetchLatest).await?;
        let _ = send.finish();

        let resp: ClipboardResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        match resp {
            ClipboardResponse::Items(items) => Ok(items),
            ClipboardResponse::Error(msg) => Err(CoreError::InvalidInput(msg)),
            _ => Err(CoreError::InvalidInput(
                "unexpected clipboard response".into(),
            )),
        }
    }

    pub async fn fetch_remote_image(
        &self,
        engine: &LandaEngine,
        peer_id: &str,
        item_id: &str,
    ) -> Result<Vec<u8>, CoreError> {
        if !self.is_enabled() {
            return Err(CoreError::ModuleDisabled("clipboard".into()));
        }
        let (_, mut send, mut recv) = engine
            .open_trusted_module_stream(peer_id, StreamKind::Clipboard)
            .await?;
        write_frame(
            &mut send,
            &ClipboardRequest::FetchImage {
                item_id: item_id.to_string(),
            },
        )
        .await?;
        let _ = send.finish();

        let resp: ClipboardResponse = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;
        let (size_bytes, content_hash) = match resp {
            ClipboardResponse::ImageHeader {
                size_bytes,
                content_hash,
                ..
            } => (size_bytes, content_hash),
            ClipboardResponse::Error(msg) => return Err(CoreError::InvalidInput(msg)),
            _ => {
                return Err(CoreError::InvalidInput(
                    "unexpected clipboard response".into(),
                ))
            }
        };

        read_verified_image_stream(&mut recv, size_bytes, &content_hash).await
    }
}

#[async_trait]
impl LandaModule for ClipboardModule {
    fn id(&self) -> &'static str {
        "clipboard"
    }

    fn stream_kinds(&self) -> &'static [StreamKind] {
        &[StreamKind::Clipboard]
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    async fn handle_stream(
        &self,
        peer: PeerContext,
        _kind: StreamKind,
        mut send: SendStream,
        mut recv: RecvStream,
    ) -> Result<(), CoreError> {
        let req: ClipboardRequest = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;

        match req {
            ClipboardRequest::FetchLatest => {
                let items = self
                    .list_local_items()
                    .into_iter()
                    .map(|mut item| {
                        // Never leak local filesystem paths to remote peers
                        item.image_path = None;
                        item
                    })
                    .collect();
                write_frame(&mut send, &ClipboardResponse::Items(items)).await?;
                let _ = send.finish();
            }
            ClipboardRequest::PushText { text } => {
                let resp = match self.push_local_text(
                    text,
                    Some(peer.peer_id.to_hex()),
                    Some(peer.display_name),
                ) {
                    Ok(_) => ClipboardResponse::Ack,
                    Err(err) => ClipboardResponse::Error(err.to_string()),
                };
                write_frame(&mut send, &resp).await?;
                let _ = send.finish();
            }
            ClipboardRequest::PushImage {
                size_bytes,
                width,
                height,
                content_hash,
            } => {
                let resp = if width == 0
                    || height == 0
                    || width > MAX_IMAGE_DIMENSION
                    || height > MAX_IMAGE_DIMENSION
                {
                    ClipboardResponse::Error("invalid image dimensions".into())
                } else {
                    match read_verified_image_stream(&mut recv, size_bytes, &content_hash).await {
                        Ok(bytes) => match self.push_local_image(
                            &bytes,
                            Some(peer.peer_id.to_hex()),
                            Some(peer.display_name),
                        ) {
                            Ok(_) => ClipboardResponse::Ack,
                            Err(err) => ClipboardResponse::Error(err.to_string()),
                        },
                        Err(err) => ClipboardResponse::Error(err.to_string()),
                    }
                };
                write_frame(&mut send, &resp).await?;
                let _ = send.finish();
            }
            ClipboardRequest::FetchImage { item_id } => {
                let Some(item) = self.get_local_item(&item_id) else {
                    write_frame(
                        &mut send,
                        &ClipboardResponse::Error("clipboard image not found".into()),
                    )
                    .await?;
                    let _ = send.finish();
                    return Ok(());
                };
                if item.kind != ClipboardContentKind::Image {
                    write_frame(
                        &mut send,
                        &ClipboardResponse::Error("clipboard item is not an image".into()),
                    )
                    .await?;
                    let _ = send.finish();
                    return Ok(());
                }
                let Some(path) = item.image_path else {
                    write_frame(
                        &mut send,
                        &ClipboardResponse::Error("clipboard image missing".into()),
                    )
                    .await?;
                    let _ = send.finish();
                    return Ok(());
                };
                let Ok(bytes) = tokio::fs::read(&path).await else {
                    write_frame(
                        &mut send,
                        &ClipboardResponse::Error("failed to read clipboard image".into()),
                    )
                    .await?;
                    let _ = send.finish();
                    return Ok(());
                };
                if bytes.len() > MAX_IMAGE_BYTES {
                    write_frame(
                        &mut send,
                        &ClipboardResponse::Error("clipboard image exceeds limit".into()),
                    )
                    .await?;
                    let _ = send.finish();
                    return Ok(());
                }
                let content_hash = blake3::hash(&bytes).to_hex().to_string();
                write_frame(
                    &mut send,
                    &ClipboardResponse::ImageHeader {
                        size_bytes: bytes.len() as u64,
                        width: item.image_width.unwrap_or(0),
                        height: item.image_height.unwrap_or(0),
                        content_hash,
                    },
                )
                .await?;
                stream_bytes_with_digest(&mut send, &bytes).await?;
                let _ = send.finish();
            }
        }

        Ok(())
    }
}

fn normalize_to_png(raw_bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), CoreError> {
    if raw_bytes.is_empty() {
        return Err(CoreError::InvalidInput("clipboard image is empty".into()));
    }
    if raw_bytes.len() > MAX_IMAGE_BYTES {
        return Err(CoreError::InvalidInput(
            "clipboard image exceeds 10 MB limit".into(),
        ));
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(128 * 1024 * 1024);

    let mut reader = ImageReader::new(Cursor::new(raw_bytes))
        .with_guessed_format()
        .map_err(|e| CoreError::InvalidInput(format!("invalid image data: {e}")))?;
    reader.limits(limits.clone());
    let format = reader.format();

    let mut dim_reader = ImageReader::new(Cursor::new(raw_bytes))
        .with_guessed_format()
        .map_err(|e| CoreError::InvalidInput(format!("invalid image data: {e}")))?;
    dim_reader.limits(limits);
    let (width, height) = dim_reader
        .into_dimensions()
        .map_err(|e| CoreError::InvalidInput(format!("invalid image dimensions: {e}")))?;

    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(CoreError::InvalidInput(
            "clipboard image dimensions out of bounds".into(),
        ));
    }

    let decoded = reader
        .decode()
        .map_err(|e| CoreError::InvalidInput(format!("failed to decode image: {e}")))?;

    if format == Some(ImageFormat::Png) {
        return Ok((raw_bytes.to_vec(), width, height));
    }

    let mut png_bytes = Vec::new();
    decoded
        .write_to(&mut Cursor::new(&mut png_bytes), ImageFormat::Png)
        .map_err(|e| CoreError::InvalidInput(format!("failed to encode PNG: {e}")))?;
    if png_bytes.len() > MAX_IMAGE_BYTES {
        return Err(CoreError::InvalidInput(
            "converted PNG exceeds 10 MB limit".into(),
        ));
    }
    Ok((png_bytes, width, height))
}

async fn stream_bytes_with_digest(send: &mut SendStream, bytes: &[u8]) -> Result<(), CoreError> {
    let mut hasher = blake3::Hasher::new();
    for chunk in bytes.chunks(STREAM_CHUNK_BYTES) {
        hasher.update(chunk);
        send.write_all(chunk).await?;
    }
    let digest = hasher.finalize();
    send.write_all(digest.as_bytes()).await?;
    Ok(())
}

async fn read_verified_image_stream(
    recv: &mut RecvStream,
    size_bytes: u64,
    expected_hash_hex: &str,
) -> Result<Vec<u8>, CoreError> {
    if size_bytes == 0 || size_bytes > MAX_IMAGE_BYTES as u64 {
        return Err(CoreError::InvalidInput(
            "clipboard image size out of bounds (max 10 MB)".into(),
        ));
    }

    let total = size_bytes as usize;
    let mut out = Vec::with_capacity(total);
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; STREAM_CHUNK_BYTES];
    let mut remaining = total;

    while remaining > 0 {
        let to_read = remaining.min(buf.len());
        let n = recv.read(&mut buf[..to_read]).await?.ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "clipboard image stream truncated",
            ))
        })?;
        hasher.update(&buf[..n]);
        out.extend_from_slice(&buf[..n]);
        remaining -= n;
    }

    let mut expected_digest = [0u8; 32];
    recv.read_exact(&mut expected_digest)
        .await
        .map_err(|e| CoreError::Io(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, e)))?;

    let actual = hasher.finalize();
    if actual.as_bytes() != &expected_digest || actual.to_hex().as_str() != expected_hash_hex {
        return Err(CoreError::IntegrityMismatch("clipboard_image.png".into()));
    }

    Ok(out)
}

fn sweep_orphan_images(images_dir: &Path, items: &[ClipboardItem]) {
    let referenced: HashSet<PathBuf> = items
        .iter()
        .filter_map(|i| i.image_path.as_deref())
        .map(PathBuf::from)
        .collect();
    let Ok(entries) = std::fs::read_dir(images_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && !referenced.contains(&path) {
            let _ = std::fs::remove_file(path);
        }
    }
}
