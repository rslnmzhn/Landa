use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use landa_proto::wire::{read_frame, write_frame, FriendMessage, StreamKind, MAX_CONTROL_FRAME_BYTES};
use landa_proto::{DeviceIdentity, PeerId};
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::discovery::{current_category, current_os, DiscoveryService};
use crate::error::CoreError;
use crate::events::{
    CoreEvent, FriendRequestInfo, ManualTarget, PeerInfo, TransferHistoryRecord,
};
use crate::module::{LandaModule, ModuleRegistry, ModuleStatus, PeerContext};
use crate::storage::{AppSettings, Storage};
use crate::transfer::{OpenSourceHook, OutgoingFileItem, ReceivePublishHook, TransferManager};
use crate::transport::{QuicTransport, TransportHandlerCtx};

#[derive(Debug, Clone)]
pub struct LandaConfig {
    pub app_data_dir: PathBuf,
    pub default_download_dir: PathBuf,
    pub default_device_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalIdentityInfo {
    pub peer_id: String,
    pub public_key_hex: String,
    pub device_name: String,
    pub quic_port: u16,
    pub os: landa_proto::DeviceOs,
}

pub struct LandaEngine {
    app_data_dir: PathBuf,
    identity: Arc<DeviceIdentity>,
    storage: Arc<Storage>,
    modules: Arc<ModuleRegistry>,
    discovery: Arc<DiscoveryService>,
    transport: Arc<QuicTransport>,
    transfers: Arc<TransferManager>,
    events: broadcast::Sender<CoreEvent>,
    settings: Arc<RwLock<AppSettings>>,
    discovery_interval_secs: Arc<RwLock<u64>>,
    cancel: CancellationToken,
}

impl LandaEngine {
    pub fn new(config: LandaConfig) -> Result<Arc<Self>, CoreError> {
        Self::new_with_bind_ip(config, std::net::Ipv4Addr::UNSPECIFIED)
    }

    pub fn new_loopback(config: LandaConfig) -> Result<Arc<Self>, CoreError> {
        Self::new_with_bind_ip(config, std::net::Ipv4Addr::LOCALHOST)
    }

    fn new_with_bind_ip(
        config: LandaConfig,
        bind_ip: std::net::Ipv4Addr,
    ) -> Result<Arc<Self>, CoreError> {
        std::fs::create_dir_all(&config.app_data_dir)?;
        let identity = Arc::new(DeviceIdentity::load_or_generate(
            &config.app_data_dir.join("identity.key"),
        )?);
        let storage = Arc::new(Storage::open(&config.app_data_dir.join("landa.sqlite"))?);
        let defaults = AppSettings::default_with(
            config.default_device_name,
            config.default_download_dir,
        );
        let settings_val = storage.load_settings(defaults)?;
        let interval = settings_val.discovery_interval_secs;
        let device_name = settings_val.device_name.clone();

        let (events, _) = broadcast::channel(256);
        let transport = Arc::new(QuicTransport::bind_ip(Arc::clone(&identity), bind_ip)?);
        let discovery = Arc::new(DiscoveryService::new(
            Arc::clone(&identity),
            Arc::clone(&storage),
            events.clone(),
            device_name,
            transport.local_port(),
        ));
        let transfers = Arc::new(TransferManager::new(
            Arc::clone(&storage),
            events.clone(),
            config.app_data_dir.join("incoming_temp"),
        ));

        Ok(Arc::new(Self {
            app_data_dir: config.app_data_dir,
            identity,
            storage,
            modules: Arc::new(ModuleRegistry::new()),
            discovery,
            transport,
            transfers,
            events,
            settings: Arc::new(RwLock::new(settings_val)),
            discovery_interval_secs: Arc::new(RwLock::new(interval)),
            cancel: CancellationToken::new(),
        }))
    }

    pub fn register_module(&self, module: Arc<dyn LandaModule>) {
        let enabled = self
            .settings
            .read()
            .expect("settings lock poisoned")
            .enabled_modules
            .get(module.id())
            .copied()
            .unwrap_or(true);
        module.set_enabled(enabled);
        self.modules.register(module);
    }

    pub fn start_loops(self: &Arc<Self>) {
        let discovery = Arc::clone(&self.discovery);
        let interval = Arc::clone(&self.discovery_interval_secs);
        let disc_cancel = self.cancel.clone();
        tokio::spawn(async move {
            if let Err(err) = discovery.run(interval, disc_cancel).await {
                tracing::error!("discovery loop exited: {err}");
            }
        });

        self.start_transport_loop();
    }

    pub fn start_transport_loop(self: &Arc<Self>) {
        let transport = Arc::clone(&self.transport);
        let ctx = TransportHandlerCtx {
            storage: Arc::clone(&self.storage),
            modules: Arc::clone(&self.modules),
            transfers: Arc::clone(&self.transfers),
            events: self.events.clone(),
            settings: Arc::clone(&self.settings),
        };
        let trans_cancel = self.cancel.clone();
        tokio::spawn(async move {
            transport.run_accept_loop(ctx, trans_cancel).await;
        });
    }

    pub fn shutdown(&self) {
        self.cancel.cancel();
    }

    pub fn app_data_dir(&self) -> &std::path::Path {
        &self.app_data_dir
    }

    pub fn storage(&self) -> &Arc<Storage> {
        &self.storage
    }

    pub fn events(&self) -> &broadcast::Sender<CoreEvent> {
        &self.events
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<CoreEvent> {
        self.events.subscribe()
    }

    pub fn set_receive_publish_hook(&self, hook: ReceivePublishHook) {
        self.transfers.set_publish_hook(hook);
    }

    pub fn set_open_source_hook(&self, hook: OpenSourceHook) {
        self.transfers.set_open_source_hook(hook);
    }

    pub fn local_identity_info(&self) -> LocalIdentityInfo {
        let device_name = self
            .settings
            .read()
            .expect("settings lock poisoned")
            .device_name
            .clone();
        LocalIdentityInfo {
            peer_id: self.identity.peer_id().to_hex(),
            public_key_hex: hex_encode(&self.identity.public_key_bytes()),
            device_name,
            quic_port: self.transport.local_port(),
            os: current_os(),
        }
    }

    pub fn list_peers(&self) -> Result<Vec<PeerInfo>, CoreError> {
        self.discovery.list_peers()
    }

    pub fn trigger_scan(&self) {
        self.transport.rebind_network();
        self.discovery.trigger_scan();
    }

    pub fn apply_system_device_name_if_placeholder(&self, real_name: &str) {
        let trimmed = real_name.trim();
        if trimmed.is_empty() || trimmed == "Android Device" || trimmed == "Landa Device" {
            return;
        }
        let mut next = self.get_settings();
        if next.device_name.trim().is_empty() || next.device_name == "Android Device" {
            next.device_name = trimmed.to_string();
            let _ = self.update_settings(next);
        }
    }

    pub fn rename_peer(&self, peer_id_hex: &str, alias: Option<&str>) -> Result<(), CoreError> {
        let peer_id = parse_peer_id(peer_id_hex)?;
        self.storage.set_peer_alias(&peer_id, alias)?;
        let _ = self.events.send(CoreEvent::PeersChanged);
        Ok(())
    }

    pub fn list_friend_requests(&self) -> Result<Vec<FriendRequestInfo>, CoreError> {
        self.storage.list_friend_requests()
    }

    pub async fn send_friend_request(&self, peer_id_hex: &str) -> Result<(), CoreError> {
        let peer_id = parse_peer_id(peer_id_hex)?;
        let (peer_ctx, addr) = self.resolve_peer_endpoint(&peer_id)?;
        let (mut send, mut recv) = self
            .transport
            .open_stream(addr, peer_id, StreamKind::FriendHandshake)
            .await?;

        let device_name = self
            .settings
            .read()
            .expect("settings lock poisoned")
            .device_name
            .clone();
        let msg = FriendMessage::Request {
            device_name,
            os: current_os(),
            category: current_category(),
        };
        write_frame(&mut send, &msg).await?;
        let _ = send.finish();
        let _: Result<FriendMessage, _> = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await;

        let stored = self.storage.get_peer(&peer_id)?;
        if let Some(p) = stored {
            let sas = DeviceIdentity::sas_code(&self.identity.public_key_bytes(), &p.public_key);
            self.storage.upsert_friend_request(
                &peer_id,
                &p.public_key,
                &peer_ctx.display_name,
                p.os,
                p.category,
                &sas,
                "outgoing",
            )?;
            let _ = self.events.send(CoreEvent::FriendRequestsChanged);
        }
        Ok(())
    }

    pub async fn respond_friend_request(
        &self,
        peer_id_hex: &str,
        accept: bool,
    ) -> Result<(), CoreError> {
        let peer_id = parse_peer_id(peer_id_hex)?;
        if accept {
            self.storage.set_peer_trusted(&peer_id, true)?;
        }
        self.storage.remove_friend_request(&peer_id)?;
        let _ = self.events.send(CoreEvent::FriendRequestsChanged);
        let _ = self.events.send(CoreEvent::PeersChanged);

        if let Ok((_, addr)) = self.resolve_peer_endpoint(&peer_id) {
            if let Ok((mut send, mut recv)) = self
                .transport
                .open_stream(addr, peer_id, StreamKind::FriendHandshake)
                .await
            {
                let device_name = self
                    .settings
                    .read()
                    .expect("settings lock poisoned")
                    .device_name
                    .clone();
                let msg = if accept {
                    FriendMessage::Accept {
                        device_name,
                        os: current_os(),
                        category: current_category(),
                    }
                } else {
                    FriendMessage::Reject
                };
                let _ = write_frame(&mut send, &msg).await;
                let _ = send.finish();
                let _: Result<FriendMessage, _> =
                    read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await;
            }
        }
        Ok(())
    }

    pub async fn unfriend(&self, peer_id_hex: &str) -> Result<(), CoreError> {
        let peer_id = parse_peer_id(peer_id_hex)?;
        self.storage.set_peer_trusted(&peer_id, false)?;
        self.storage.remove_friend_request(&peer_id)?;
        let _ = self.events.send(CoreEvent::PeersChanged);
        let _ = self.events.send(CoreEvent::FriendRequestsChanged);

        if let Ok((_, addr)) = self.resolve_peer_endpoint(&peer_id) {
            if let Ok((mut send, _)) = self
                .transport
                .open_stream(addr, peer_id, StreamKind::FriendHandshake)
                .await
            {
                let _ = write_frame(&mut send, &FriendMessage::Unfriend).await;
                let _ = send.finish();
            }
        }
        Ok(())
    }

    pub async fn send_files(
        &self,
        peer_id_hex: &str,
        paths: Vec<PathBuf>,
    ) -> Result<TransferHistoryRecord, CoreError> {
        let items = TransferManager::collect_outgoing_items(&paths)?;
        self.send_items(peer_id_hex, items).await
    }

    pub async fn send_items(
        &self,
        peer_id_hex: &str,
        items: Vec<OutgoingFileItem>,
    ) -> Result<TransferHistoryRecord, CoreError> {
        let peer_id = parse_peer_id(peer_id_hex)?;
        let (peer_ctx, addr) = self.resolve_peer_endpoint(&peer_id)?;
        let (send, recv) = self
            .transport
            .open_stream(addr, peer_id, StreamKind::FileTransfer)
            .await?;
        self.transfers.send_files(&peer_ctx, send, recv, items).await
    }

    pub fn respond_transfer_offer(&self, transfer_id: &str, accept: bool) -> Result<(), CoreError> {
        self.transfers.respond_to_offer(transfer_id, accept)
    }

    pub fn cancel_transfer(&self, transfer_id: &str) -> bool {
        self.transfers.cancel_transfer(transfer_id)
    }

    pub fn list_history(&self, limit: usize) -> Result<Vec<TransferHistoryRecord>, CoreError> {
        self.storage.list_transfer_history(limit)
    }

    pub fn clear_history(&self) -> Result<(), CoreError> {
        self.storage.clear_transfer_history()?;
        let _ = self.events.send(CoreEvent::HistoryChanged);
        Ok(())
    }

    pub fn get_settings(&self) -> AppSettings {
        self.settings
            .read()
            .expect("settings lock poisoned")
            .clone()
    }

    pub fn update_settings(&self, next: AppSettings) -> Result<AppSettings, CoreError> {
        self.storage.save_settings(&next)?;
        self.discovery.set_device_name(next.device_name.clone());
        *self
            .discovery_interval_secs
            .write()
            .expect("interval lock poisoned") = next.discovery_interval_secs;
        for (id, enabled) in &next.enabled_modules {
            self.modules.set_enabled(id, *enabled);
        }
        *self.settings.write().expect("settings lock poisoned") = next.clone();
        Ok(next)
    }

    pub fn list_modules(&self) -> Vec<ModuleStatus> {
        self.modules.statuses()
    }

    pub fn set_module_enabled(&self, id: &str, enabled: bool) -> Result<Vec<ModuleStatus>, CoreError> {
        if !self.modules.set_enabled(id, enabled) {
            return Err(CoreError::NotFound(format!("module {id}")));
        }
        let mut next = self.get_settings();
        next.enabled_modules.insert(id.to_string(), enabled);
        self.storage.save_settings(&next)?;
        *self.settings.write().expect("settings lock poisoned") = next;
        Ok(self.modules.statuses())
    }

    pub fn list_manual_targets(&self) -> Result<Vec<ManualTarget>, CoreError> {
        self.storage.list_manual_targets()
    }

    pub fn add_manual_target(&self, ip: &str, label: Option<&str>) -> Result<(), CoreError> {
        let parsed: IpAddr = ip
            .trim()
            .parse()
            .map_err(|_| CoreError::InvalidInput("invalid IP address".into()))?;
        self.storage.add_manual_target(&parsed.to_string(), label)?;
        self.discovery.trigger_scan();
        Ok(())
    }

    pub fn remove_manual_target(&self, ip: &str) -> Result<(), CoreError> {
        self.storage.remove_manual_target(ip.trim())?;
        Ok(())
    }

    pub fn local_qr_connect_uri(&self) -> Result<(String, String, u16), CoreError> {
        let ip = crate::qr::pick_primary_lan_ip().ok_or_else(|| {
            CoreError::NotFound("no active LAN IPv4 interface found".into())
        })?;
        let port = self.transport.local_port();
        let device_name = self
            .settings
            .read()
            .expect("settings lock poisoned")
            .device_name
            .clone();
        let uri = crate::qr::build_qr_connect_uri(
            ip,
            port,
            self.identity.peer_id(),
            &self.identity.public_key_bytes(),
            &device_name,
        );
        Ok((uri, ip.to_string(), port))
    }

    pub async fn connect_via_qr_uri(&self, uri: &str) -> Result<PeerInfo, CoreError> {
        let payload = crate::qr::parse_qr_connect_uri(uri)?;
        if payload.peer_id == self.identity.peer_id() {
            return Err(CoreError::InvalidInput(
                "cannot connect to your own device".into(),
            ));
        }

        self.discovery.register_direct_peer(
            payload.peer_id,
            payload.public_key,
            &payload.device_name,
            payload.ip,
            payload.port,
        )?;
        let _ = self
            .storage
            .add_manual_target(&payload.ip.to_string(), Some(&payload.device_name));

        let addr = SocketAddr::new(IpAddr::V4(payload.ip), payload.port);
        self.transport.connect_peer(addr, payload.peer_id).await?;
        self.discovery.trigger_scan();

        let target_hex = payload.peer_id.to_hex();
        self.list_peers()?
            .into_iter()
            .find(|p| p.peer_id == target_hex)
            .ok_or_else(|| CoreError::NotFound("connected peer missing from storage".into()))
    }

    pub async fn open_trusted_module_stream(
        &self,
        peer_id_hex: &str,
        kind: StreamKind,
    ) -> Result<(PeerContext, SendStream, RecvStream), CoreError> {
        let peer_id = parse_peer_id(peer_id_hex)?;
        let (peer_ctx, addr) = self.resolve_peer_endpoint(&peer_id)?;
        if !peer_ctx.is_trusted {
            return Err(CoreError::UntrustedPeer);
        }
        let (send, recv) = self.transport.open_stream(addr, peer_id, kind).await?;
        Ok((peer_ctx, send, recv))
    }

    fn resolve_peer_endpoint(&self, peer_id: &PeerId) -> Result<(PeerContext, SocketAddr), CoreError> {
        let stored = self
            .storage
            .get_peer(peer_id)?
            .ok_or_else(|| CoreError::NotFound(format!("peer {peer_id}")))?;
        let live = self.discovery.get_live_peer(peer_id);

        let (ip_str, port) = match live {
            Some(l) => (l.ip.to_string(), l.quic_port),
            None => {
                let last_ip = stored
                    .last_ip
                    .clone()
                    .ok_or_else(|| CoreError::NotFound("peer IP unknown".into()))?;
                let last_port = stored
                    .last_port
                    .ok_or_else(|| CoreError::NotFound("peer port unknown".into()))?;
                let is_loopback = last_ip
                    .parse::<IpAddr>()
                    .map(|ip| ip.is_loopback())
                    .unwrap_or(false);
                let is_manual = self
                    .storage
                    .list_manual_targets()
                    .map(|targets| targets.iter().any(|t| t.ip == last_ip))
                    .unwrap_or(false);
                if !is_manual && !is_loopback {
                    self.trigger_scan();
                    return Err(CoreError::NotFound(
                        "device is offline or unreachable on LAN".into(),
                    ));
                }
                (last_ip, last_port)
            }
        };

        let ip: IpAddr = ip_str
            .parse()
            .map_err(|_| CoreError::InvalidInput("invalid peer IP".into()))?;
        let addr = SocketAddr::new(ip, port);
        let display_name = stored
            .alias_name
            .clone()
            .unwrap_or_else(|| stored.device_name.clone());

        let ctx = PeerContext {
            peer_id: *peer_id,
            public_key: stored.public_key,
            remote_addr: addr,
            is_trusted: stored.is_trusted,
            display_name,
        };
        Ok((ctx, addr))
    }
}

fn parse_peer_id(hex: &str) -> Result<PeerId, CoreError> {
    PeerId::from_hex(hex.trim()).map_err(CoreError::from)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
