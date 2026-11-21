use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use landa_proto::wire::StreamKind;
use landa_proto::PeerId;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleStatus {
    pub id: String,
    pub enabled: bool,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct PeerContext {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub remote_addr: SocketAddr,
    pub is_trusted: bool,
    pub display_name: String,
}

#[async_trait]
pub trait LandaModule: Send + Sync {
    fn id(&self) -> &'static str;

    fn stream_kinds(&self) -> &'static [StreamKind];

    fn requires_trusted_peer(&self) -> bool {
        true
    }

    fn is_enabled(&self) -> bool;

    fn set_enabled(&self, enabled: bool);

    async fn handle_stream(
        &self,
        peer: PeerContext,
        kind: StreamKind,
        send: SendStream,
        recv: RecvStream,
    ) -> Result<(), CoreError>;
}

#[derive(Default)]
pub struct ModuleRegistry {
    modules: RwLock<HashMap<String, Arc<dyn LandaModule>>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, module: Arc<dyn LandaModule>) {
        let mut guard = self.modules.write().expect("module registry lock poisoned");
        guard.insert(module.id().to_string(), module);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn LandaModule>> {
        let guard = self.modules.read().expect("module registry lock poisoned");
        guard.get(id).cloned()
    }

    pub fn find_for_stream(&self, kind: StreamKind) -> Option<Arc<dyn LandaModule>> {
        let guard = self.modules.read().expect("module registry lock poisoned");
        guard
            .values()
            .find(|m| m.stream_kinds().contains(&kind))
            .cloned()
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> bool {
        let guard = self.modules.read().expect("module registry lock poisoned");
        if let Some(module) = guard.get(id) {
            module.set_enabled(enabled);
            true
        } else {
            false
        }
    }

    pub fn statuses(&self) -> Vec<ModuleStatus> {
        let guard = self.modules.read().expect("module registry lock poisoned");
        let mut list: Vec<ModuleStatus> = guard
            .values()
            .map(|m| ModuleStatus {
                id: m.id().to_string(),
                enabled: m.is_enabled(),
                available: true,
            })
            .collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    pub async fn dispatch_stream(
        &self,
        peer: PeerContext,
        kind: StreamKind,
        send: SendStream,
        recv: RecvStream,
    ) -> Result<(), CoreError> {
        let module = self
            .find_for_stream(kind)
            .ok_or_else(|| CoreError::ModuleDisabled(format!("{kind:?}")))?;

        if !module.is_enabled() {
            return Err(CoreError::ModuleDisabled(module.id().to_string()));
        }
        if module.requires_trusted_peer() && !peer.is_trusted {
            return Err(CoreError::UntrustedPeer);
        }

        module.handle_stream(peer, kind, send, recv).await
    }
}
