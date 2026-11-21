use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use landa_core::{CoreError, CoreEvent, LandaEngine, LandaModule, PeerContext, Storage};
use landa_proto::wire::{read_frame, write_frame, StreamKind, MAX_CONTROL_FRAME_BYTES};
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

const CLIPBOARD_SETTING_KEY: &str = "clipboard_history";
const MAX_HISTORY_ITEMS: usize = 30;
const MAX_TEXT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardItem {
    pub id: String,
    pub text: String,
    pub content_hash: String,
    pub source_peer_id: Option<String>,
    pub source_peer_name: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
enum ClipboardRequest {
    FetchLatest,
    PushText { text: String },
}

#[derive(Debug, Serialize, Deserialize)]
enum ClipboardResponse {
    Items(Vec<ClipboardItem>),
    Ack,
    Error(String),
}

pub struct ClipboardModule {
    storage: Arc<Storage>,
    events: broadcast::Sender<CoreEvent>,
    enabled: AtomicBool,
    items: Mutex<VecDeque<ClipboardItem>>,
}

impl ClipboardModule {
    pub fn new(storage: Arc<Storage>, events: broadcast::Sender<CoreEvent>) -> Arc<Self> {
        let initial: Vec<ClipboardItem> = storage
            .get_setting_json::<Vec<ClipboardItem>>(CLIPBOARD_SETTING_KEY)
            .ok()
            .flatten()
            .unwrap_or_default();
        Arc::new(Self {
            storage,
            events,
            enabled: AtomicBool::new(true),
            items: Mutex::new(VecDeque::from(initial)),
        })
    }

    pub fn list_local_items(&self) -> Vec<ClipboardItem> {
        self.items
            .lock()
            .expect("clipboard lock poisoned")
            .iter()
            .cloned()
            .collect()
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
            return Err(CoreError::InvalidInput("clipboard text exceeds 256 KB".into()));
        }

        let hash = blake3::hash(trimmed.as_bytes()).to_hex().to_string();
        let item = ClipboardItem {
            id: Uuid::new_v4().to_string(),
            text,
            content_hash: hash.clone(),
            source_peer_id,
            source_peer_name,
            created_at: Utc::now().timestamp_millis(),
        };

        {
            let mut guard = self.items.lock().expect("clipboard lock poisoned");
            guard.retain(|existing| existing.content_hash != hash);
            guard.push_front(item.clone());
            while guard.len() > MAX_HISTORY_ITEMS {
                guard.pop_back();
            }
            let snapshot: Vec<ClipboardItem> = guard.iter().cloned().collect();
            let _ = self.storage.set_setting_json(CLIPBOARD_SETTING_KEY, &snapshot);
        }

        let _ = self.events.send(CoreEvent::ModuleEvent {
            module_id: "clipboard".to_string(),
            event_name: "updated".to_string(),
            data: serde_json::to_value(&item).unwrap_or(serde_json::Value::Null),
        });

        Ok(item)
    }

    pub fn delete_local_item(&self, id: &str) {
        let mut guard = self.items.lock().expect("clipboard lock poisoned");
        guard.retain(|item| item.id != id);
        let snapshot: Vec<ClipboardItem> = guard.iter().cloned().collect();
        let _ = self.storage.set_setting_json(CLIPBOARD_SETTING_KEY, &snapshot);
        let _ = self.events.send(CoreEvent::ModuleEvent {
            module_id: "clipboard".to_string(),
            event_name: "updated".to_string(),
            data: serde_json::Value::Null,
        });
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
            return Err(CoreError::InvalidInput("invalid clipboard text size".into()));
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
            _ => Err(CoreError::InvalidInput("unexpected clipboard response".into())),
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
            _ => Err(CoreError::InvalidInput("unexpected clipboard response".into())),
        }
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

        let resp = match req {
            ClipboardRequest::FetchLatest => ClipboardResponse::Items(self.list_local_items()),
            ClipboardRequest::PushText { text } => {
                match self.push_local_text(
                    text,
                    Some(peer.peer_id.to_hex()),
                    Some(peer.display_name),
                ) {
                    Ok(_) => ClipboardResponse::Ack,
                    Err(err) => ClipboardResponse::Error(err.to_string()),
                }
            }
        };

        write_frame(&mut send, &resp).await?;
        let _ = send.finish();
        Ok(())
    }
}
