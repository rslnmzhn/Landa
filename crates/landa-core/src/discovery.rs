use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use landa_proto::{
    decode_and_verify_beacon, encode_signed_beacon, DeviceCategory, DeviceIdentity, DeviceOs,
    PeerId, DISCOVERY_PORT,
};
use rand::RngCore;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::error::CoreError;
use crate::events::{CoreEvent, PeerInfo};
#[cfg(unix)]
use crate::network::bind_socket_to_lan_network;
use crate::network::{
    enumerate_all_lan_interfaces, enumerate_lan_interfaces, is_ip_in_excluded_subnet,
    DISCOVERY_MULTICAST_ADDR,
};
use crate::storage::Storage;

const PEER_OFFLINE_TIMEOUT: Duration = Duration::from_secs(25);
const UNICAST_REPLY_COOLDOWN: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct LivePeer {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub device_name: String,
    pub os: DeviceOs,
    pub category: DeviceCategory,
    pub ip: Ipv4Addr,
    pub quic_port: u16,
    pub last_seen: Instant,
}

pub struct DiscoveryService {
    identity: Arc<DeviceIdentity>,
    storage: Arc<Storage>,
    live_peers: Arc<RwLock<HashMap<PeerId, LivePeer>>>,
    last_unicast_reply: Mutex<HashMap<PeerId, Instant>>,
    events: broadcast::Sender<CoreEvent>,
    scan_notify: Arc<Notify>,
    device_name: Arc<RwLock<String>>,
    quic_port: u16,
}

impl DiscoveryService {
    pub fn new(
        identity: Arc<DeviceIdentity>,
        storage: Arc<Storage>,
        events: broadcast::Sender<CoreEvent>,
        device_name: String,
        quic_port: u16,
    ) -> Self {
        Self {
            identity,
            storage,
            live_peers: Arc::new(RwLock::new(HashMap::new())),
            last_unicast_reply: Mutex::new(HashMap::new()),
            events,
            scan_notify: Arc::new(Notify::new()),
            device_name: Arc::new(RwLock::new(device_name)),
            quic_port,
        }
    }

    pub fn set_device_name(&self, name: String) {
        *self.device_name.write().expect("device_name lock poisoned") = name;
        self.scan_notify.notify_one();
    }

    pub fn trigger_scan(&self) {
        self.scan_notify.notify_one();
    }

    pub fn get_live_peer(&self, peer_id: &PeerId) -> Option<LivePeer> {
        let guard = self.live_peers.read().expect("live_peers lock poisoned");
        guard.get(peer_id).cloned()
    }

    pub fn register_direct_peer(
        &self,
        peer_id: PeerId,
        public_key: [u8; 32],
        device_name: &str,
        ip: Ipv4Addr,
        quic_port: u16,
    ) -> Result<(), CoreError> {
        self.storage.upsert_peer(
            &peer_id,
            &public_key,
            device_name,
            DeviceOs::Unknown,
            DeviceCategory::Unknown,
            Some(&ip.to_string()),
            Some(quic_port),
        )?;
        {
            let mut guard = self.live_peers.write().expect("live_peers lock poisoned");
            let existing = guard.get(&peer_id).cloned();
            guard.insert(
                peer_id,
                LivePeer {
                    peer_id,
                    public_key,
                    device_name: device_name.to_string(),
                    os: existing.as_ref().map(|e| e.os).unwrap_or(DeviceOs::Unknown),
                    category: existing
                        .as_ref()
                        .map(|e| e.category)
                        .unwrap_or(DeviceCategory::Unknown),
                    ip,
                    quic_port,
                    last_seen: Instant::now(),
                },
            );
        }
        let _ = self.events.send(CoreEvent::PeersChanged);
        Ok(())
    }

    pub fn list_peers(&self) -> Result<Vec<PeerInfo>, CoreError> {
        let stored = self.storage.list_peers()?;
        let live = self.live_peers.read().expect("live_peers lock poisoned");
        let now = Instant::now();

        let mut result = Vec::with_capacity(stored.len());
        for peer in stored {
            let live_entry = live.get(&peer.peer_id);
            let is_online = live_entry
                .map(|l| now.duration_since(l.last_seen) <= PEER_OFFLINE_TIMEOUT)
                .unwrap_or(false);
            let ip = live_entry.map(|l| l.ip.to_string()).or(peer.last_ip);
            let quic_port = live_entry.map(|l| l.quic_port).or(peer.last_port);
            let display_name = peer
                .alias_name
                .clone()
                .unwrap_or_else(|| peer.device_name.clone());

            result.push(PeerInfo {
                peer_id: peer.peer_id.to_hex(),
                public_key_hex: hex_encode(&peer.public_key),
                sas_code: DeviceIdentity::sas_code(
                    &self.identity.public_key_bytes(),
                    &peer.public_key,
                ),
                device_name: peer.device_name,
                alias_name: peer.alias_name,
                display_name,
                os: peer.os,
                category: peer.category,
                ip,
                quic_port,
                is_trusted: peer.is_trusted,
                is_online,
                last_seen_at: peer.last_seen_at,
            });
        }

        result.sort_by(|a, b| {
            b.is_online
                .cmp(&a.is_online)
                .then_with(|| b.is_trusted.cmp(&a.is_trusted))
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        Ok(result)
    }

    pub async fn run(
        self: Arc<Self>,
        interval_secs: Arc<RwLock<u64>>,
        cancel: CancellationToken,
    ) -> Result<(), CoreError> {
        let socket = Arc::new(bind_discovery_socket()?);
        join_multicast_on_lan(&socket);

        let recv_socket = Arc::clone(&socket);
        let recv_self = Arc::clone(&self);
        let recv_cancel = cancel.clone();

        let recv_task = tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            loop {
                tokio::select! {
                    _ = recv_cancel.cancelled() => break,
                    res = recv_socket.recv_from(&mut buf) => {
                        match res {
                            Ok((len, addr)) => {
                                recv_self.handle_datagram(&buf[..len], addr, &recv_socket).await;
                            }
                            Err(err) => {
                                warn!("discovery recv error: {err}");
                                tokio::time::sleep(Duration::from_millis(200)).await;
                            }
                        }
                    }
                }
            }
        });

        let reaper_self = Arc::clone(&self);
        let reaper_cancel = cancel.clone();
        let reaper_task = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = reaper_cancel.cancelled() => break,
                    _ = ticker.tick() => {
                        let now = Instant::now();
                        let mut changed = false;
                        {
                            let mut guard = reaper_self.live_peers.write().expect("live_peers lock poisoned");
                            let before = guard.len();
                            guard.retain(|_, p| now.duration_since(p.last_seen) <= PEER_OFFLINE_TIMEOUT);
                            if guard.len() != before {
                                changed = true;
                            }
                        }
                        if changed {
                            let _ = reaper_self.events.send(CoreEvent::PeersChanged);
                        }
                    }
                }
            }
        });

        // Initial burst so peers appear immediately on launch
        self.send_beacon(&socket).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        self.send_beacon(&socket).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
        self.send_beacon(&socket).await;

        loop {
            let secs = (*interval_secs.read().expect("interval lock poisoned")).clamp(2, 3600);
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = self.scan_notify.notified() => {
                    self.send_beacon(&socket).await;
                }
                _ = tokio::time::sleep(Duration::from_secs(secs)) => {
                    self.send_beacon(&socket).await;
                }
            }
        }

        recv_task.abort();
        reaper_task.abort();
        Ok(())
    }

    fn build_beacon_packet(&self) -> Option<Vec<u8>> {
        let name = self
            .device_name
            .read()
            .expect("device_name lock poisoned")
            .clone();
        encode_signed_beacon(
            &self.identity,
            self.quic_port,
            &name,
            Utc::now().timestamp_millis(),
            rand::thread_rng().next_u64(),
            Vec::new(),
        )
        .ok()
    }

    async fn send_beacon(&self, socket: &UdpSocket) {
        bind_socket_to_lan(socket);
        join_multicast_on_lan(socket);

        let Some(packet) = self.build_beacon_packet() else {
            return;
        };

        let interfaces = enumerate_lan_interfaces();
        let global_bcast = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DISCOVERY_PORT));
        let multicast_addr =
            SocketAddr::V4(SocketAddrV4::new(DISCOVERY_MULTICAST_ADDR, DISCOVERY_PORT));

        // Send from each physical LAN interface IP so VPN default routes cannot hijack LAN broadcasts
        for iface in &interfaces {
            let bcast_addr = SocketAddr::V4(SocketAddrV4::new(iface.broadcast, DISCOVERY_PORT));
            if let Ok(iface_sock) = bind_interface_sender_socket(iface.ip) {
                let _ = iface_sock.send_to(&packet, bcast_addr).await;
                let _ = iface_sock.send_to(&packet, global_bcast).await;
                let _ = iface_sock.send_to(&packet, multicast_addr).await;
            } else {
                let _ = socket.send_to(&packet, bcast_addr).await;
            }
        }

        if interfaces.is_empty() && enumerate_all_lan_interfaces().is_empty() {
            let _ = socket.send_to(&packet, global_bcast).await;
            let _ = socket.send_to(&packet, multicast_addr).await;
        }

        // Manual targets (e.g. VPN / routed subnets) go through the default socket
        if let Ok(manual) = self.storage.list_manual_targets() {
            for target in manual {
                if let Ok(ip) = target.ip.parse::<Ipv4Addr>() {
                    let addr = SocketAddr::V4(SocketAddrV4::new(ip, DISCOVERY_PORT));
                    if let Err(err) = socket.send_to(&packet, addr).await {
                        debug!("discovery send_to manual {addr} failed: {err}");
                    }
                }
            }
        }
    }

    async fn handle_datagram(&self, data: &[u8], addr: SocketAddr, socket: &UdpSocket) {
        let Ok(beacon) = decode_and_verify_beacon(data) else {
            return;
        };
        if beacon.peer_id == self.identity.peer_id() {
            return;
        }
        let SocketAddr::V4(v4) = addr else {
            return;
        };
        let ip = *v4.ip();
        let ip_str = ip.to_string();
        if is_ip_in_excluded_subnet(ip) {
            let is_manual = self
                .storage
                .list_manual_targets()
                .map(|targets| targets.iter().any(|t| t.ip == ip_str))
                .unwrap_or(false);
            if !is_manual {
                return;
            }
        }

        let is_new_or_changed = {
            let mut guard = self.live_peers.write().expect("live_peers lock poisoned");
            let changed = match guard.get(&beacon.peer_id) {
                Some(existing) => {
                    existing.ip != ip
                        || existing.quic_port != beacon.quic_port
                        || existing.device_name != beacon.device_name
                }
                None => true,
            };
            guard.insert(
                beacon.peer_id,
                LivePeer {
                    peer_id: beacon.peer_id,
                    public_key: beacon.public_key,
                    device_name: beacon.device_name.clone(),
                    os: beacon.os,
                    category: beacon.category,
                    ip,
                    quic_port: beacon.quic_port,
                    last_seen: Instant::now(),
                },
            );
            changed
        };

        if let Err(err) = self.storage.upsert_peer(
            &beacon.peer_id,
            &beacon.public_key,
            &beacon.device_name,
            beacon.os,
            beacon.category,
            Some(&ip_str),
            Some(beacon.quic_port),
        ) {
            warn!("failed to persist discovered peer: {err}");
        }

        if is_new_or_changed {
            let _ = self.events.send(CoreEvent::PeersChanged);
        }

        // Reply directly via unicast (rate-limited) so bidirectional discovery works immediately
        let should_reply = {
            let mut guard = self
                .last_unicast_reply
                .lock()
                .expect("last_unicast_reply lock poisoned");
            let now = Instant::now();
            match guard.get(&beacon.peer_id) {
                Some(prev) if now.duration_since(*prev) < UNICAST_REPLY_COOLDOWN => false,
                _ => {
                    guard.insert(beacon.peer_id, now);
                    true
                }
            }
        };

        if should_reply {
            if let Some(packet) = self.build_beacon_packet() {
                let reply_addr = SocketAddr::V4(SocketAddrV4::new(ip, DISCOVERY_PORT));
                let _ = socket.send_to(&packet, reply_addr).await;
            }
        }
    }
}

fn bind_discovery_socket() -> Result<UdpSocket, CoreError> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT);
    socket.bind(&addr.into())?;
    let std_sock: std::net::UdpSocket = socket.into();
    let tokio_sock = UdpSocket::from_std(std_sock)?;
    bind_socket_to_lan(&tokio_sock);
    Ok(tokio_sock)
}

fn bind_interface_sender_socket(local_ip: Ipv4Addr) -> Result<UdpSocket, CoreError> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_broadcast(true)?;
    socket.set_nonblocking(true)?;
    let addr = SocketAddrV4::new(local_ip, 0);
    socket.bind(&addr.into())?;
    let std_sock: std::net::UdpSocket = socket.into();
    let tokio_sock = UdpSocket::from_std(std_sock)?;
    bind_socket_to_lan(&tokio_sock);
    Ok(tokio_sock)
}

fn join_multicast_on_lan(socket: &UdpSocket) {
    let all_interfaces = enumerate_all_lan_interfaces();
    for iface in &all_interfaces {
        if is_ip_in_excluded_subnet(iface.ip) {
            let _ = socket.leave_multicast_v4(DISCOVERY_MULTICAST_ADDR, iface.ip);
        }
    }
    let interfaces = enumerate_lan_interfaces();
    if interfaces.is_empty() && all_interfaces.is_empty() {
        let _ = socket.join_multicast_v4(DISCOVERY_MULTICAST_ADDR, Ipv4Addr::UNSPECIFIED);
    } else {
        for iface in interfaces {
            let _ = socket.join_multicast_v4(DISCOVERY_MULTICAST_ADDR, iface.ip);
        }
    }
}

fn bind_socket_to_lan(socket: &UdpSocket) {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        bind_socket_to_lan_network(socket.as_raw_fd());
    }
    #[cfg(not(unix))]
    {
        let _ = socket;
    }
}

pub fn current_os() -> DeviceOs {
    #[cfg(target_os = "windows")]
    {
        DeviceOs::Windows
    }
    #[cfg(target_os = "linux")]
    {
        DeviceOs::Linux
    }
    #[cfg(target_os = "macos")]
    {
        DeviceOs::MacOs
    }
    #[cfg(target_os = "android")]
    {
        DeviceOs::Android
    }
    #[cfg(target_os = "ios")]
    {
        DeviceOs::Ios
    }
    #[cfg(not(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "macos",
        target_os = "android",
        target_os = "ios"
    )))]
    {
        DeviceOs::Unknown
    }
}

pub fn current_category() -> DeviceCategory {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        DeviceCategory::Phone
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        DeviceCategory::Desktop
    }
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
