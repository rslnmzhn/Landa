use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use landa_proto::tls::{
    build_client_config, build_server_config, extract_verified_peer_pubkey,
};
use landa_proto::wire::{
    read_frame, write_frame, FriendMessage, StreamKind, MAX_CONTROL_FRAME_BYTES,
};
use landa_proto::{DeviceIdentity, PeerId};
use quinn::{Connection, Endpoint, EndpointConfig, RecvStream, SendStream, TransportConfig};
use rustls::pki_types::CertificateDer;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::discovery::{current_category, current_os};
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::module::{ModuleRegistry, PeerContext};
#[cfg(unix)]
use crate::network::bind_socket_to_lan_network;
use crate::network::is_ip_in_local_lan;
use crate::storage::{AppSettings, Storage};
use crate::transfer::TransferManager;

pub const DEFAULT_QUIC_PORT: u16 = 40406;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone)]
pub struct TransportHandlerCtx {
    pub storage: Arc<Storage>,
    pub modules: Arc<ModuleRegistry>,
    pub transfers: Arc<TransferManager>,
    pub events: broadcast::Sender<CoreEvent>,
    pub settings: Arc<RwLock<AppSettings>>,
}

pub struct QuicTransport {
    identity: Arc<DeviceIdentity>,
    endpoint: Endpoint,
    fallback_endpoint: Option<Endpoint>,
    local_port: u16,
    #[cfg(unix)]
    raw_fd: i32,
    connections: Mutex<HashMap<PeerId, Connection>>,
    handler_ctx: OnceLock<TransportHandlerCtx>,
}

impl QuicTransport {
    pub fn bind(identity: Arc<DeviceIdentity>) -> Result<Self, CoreError> {
        Self::bind_ip(identity, Ipv4Addr::UNSPECIFIED)
    }

    pub fn bind_ip(identity: Arc<DeviceIdentity>, ip: Ipv4Addr) -> Result<Self, CoreError> {
        let server_tls = build_server_config(&identity)?;
        let quic_server_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_tls)
            .map_err(|e| CoreError::InvalidInput(format!("quic server config: {e}")))?;
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server_crypto));
        server_config.transport_config(build_transport_config());

        let std_sock = if ip == Ipv4Addr::UNSPECIFIED {
            std::net::UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(ip, DEFAULT_QUIC_PORT)))
                .or_else(|_| std::net::UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(ip, 0))))?
        } else {
            std::net::UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(ip, 0)))?
        };

        #[cfg(unix)]
        let raw_fd = {
            use std::os::unix::io::AsRawFd;
            let fd = std_sock.as_raw_fd();
            bind_socket_to_lan_network(fd);
            fd
        };

        let runtime = quinn::default_runtime()
            .ok_or_else(|| CoreError::Io(std::io::Error::other("no async runtime found")))?;
        let endpoint = Endpoint::new(
            EndpointConfig::default(),
            Some(server_config),
            std_sock,
            runtime,
        )?;
        let local_port = endpoint.local_addr()?.port();

        // Unbound fallback client endpoint for manual VPN targets outside local LAN subnets
        let fallback_endpoint = if ip == Ipv4Addr::UNSPECIFIED {
            Endpoint::client(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))).ok()
        } else {
            None
        };

        Ok(Self {
            identity,
            endpoint,
            fallback_endpoint,
            local_port,
            #[cfg(unix)]
            raw_fd,
            connections: Mutex::new(HashMap::new()),
            handler_ctx: OnceLock::new(),
        })
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    pub fn rebind_network(&self) {
        #[cfg(unix)]
        {
            bind_socket_to_lan_network(self.raw_fd);
        }
    }

    pub fn set_handler_ctx(&self, ctx: TransportHandlerCtx) {
        let _ = self.handler_ctx.set(ctx);
    }

    pub fn disconnect_peer(&self, peer_id: &PeerId) {
        if let Some(conn) = self
            .connections
            .lock()
            .expect("connections lock poisoned")
            .remove(peer_id)
        {
            conn.close(0u32.into(), b"peer forgotten");
        }
    }

    pub async fn connect_peer(
        self: &Arc<Self>,
        addr: SocketAddr,
        expected_peer_id: PeerId,
    ) -> Result<Connection, CoreError> {
        if let Some(existing) = self
            .connections
            .lock()
            .expect("connections lock poisoned")
            .get(&expected_peer_id)
            .cloned()
        {
            if existing.close_reason().is_none() && existing.remote_address() == addr {
                return Ok(existing);
            }
            existing.close(0u32.into(), b"reconnecting");
            self.connections
                .lock()
                .expect("connections lock poisoned")
                .remove(&expected_peer_id);
        }

        self.rebind_network();

        let client_tls = build_client_config(&self.identity, Some(expected_peer_id))?;
        let quic_client_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(client_tls)
            .map_err(|e| CoreError::InvalidInput(format!("quic client config: {e}")))?;
        let mut client_config = quinn::ClientConfig::new(Arc::new(quic_client_crypto));
        client_config.transport_config(build_transport_config());

        let use_fallback = match addr.ip() {
            IpAddr::V4(v4) if !v4.is_loopback() && !is_ip_in_local_lan(v4) => {
                self.fallback_endpoint.is_some()
            }
            _ => false,
        };

        let ep = if use_fallback {
            self.fallback_endpoint.as_ref().unwrap_or(&self.endpoint)
        } else {
            &self.endpoint
        };

        let connecting = ep.connect_with(client_config, addr, "landa.peer")?;
        let conn = tokio::time::timeout(CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| {
                CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("QUIC connection to {addr} timed out"),
                ))
            })??;

        self.connections
            .lock()
            .expect("connections lock poisoned")
            .insert(expected_peer_id, conn.clone());

        if let Some(ctx) = self.handler_ctx.get().cloned() {
            let identity = Arc::clone(&self.identity);
            let transport = Arc::clone(self);
            let conn_clone = conn.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_connection(conn_clone, identity, transport, ctx).await {
                    debug!("outgoing peer connection closed: {err}");
                }
            });
        }

        Ok(conn)
    }

    pub async fn open_stream(
        self: &Arc<Self>,
        addr: SocketAddr,
        expected_peer_id: PeerId,
        kind: StreamKind,
    ) -> Result<(SendStream, RecvStream), CoreError> {
        let conn = self.connect_peer(addr, expected_peer_id).await?;
        match conn.open_bi().await {
            Ok((mut send, recv)) => {
                send.write_all(&[kind as u8]).await?;
                Ok((send, recv))
            }
            Err(_) => {
                // Cached connection may have gone stale; evict and reconnect once
                self.connections
                    .lock()
                    .expect("connections lock poisoned")
                    .remove(&expected_peer_id);
                let fresh = self.connect_peer(addr, expected_peer_id).await?;
                let (mut send, recv) = fresh.open_bi().await?;
                send.write_all(&[kind as u8]).await?;
                Ok((send, recv))
            }
        }
    }

    pub async fn run_accept_loop(
        self: Arc<Self>,
        ctx: TransportHandlerCtx,
        cancel: CancellationToken,
    ) {
        self.set_handler_ctx(ctx.clone());
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                incoming = self.endpoint.accept() => {
                    let Some(incoming) = incoming else {
                        break;
                    };
                    let ctx = ctx.clone();
                    let identity = Arc::clone(&self.identity);
                    let transport = Arc::clone(&self);

                    tokio::spawn(async move {
                        let Ok(conn) = incoming.await else {
                            return;
                        };
                        if let Err(err) = handle_connection(
                            conn,
                            identity,
                            transport,
                            ctx,
                        ).await {
                            debug!("peer connection closed: {err}");
                        }
                    });
                }
            }
        }
    }
}

fn build_transport_config() -> Arc<TransportConfig> {
    let mut cfg = TransportConfig::default();
    cfg.keep_alive_interval(Some(Duration::from_secs(4)));
    if let Ok(idle) = Duration::from_secs(15).try_into() {
        cfg.max_idle_timeout(Some(idle));
    }
    Arc::new(cfg)
}

async fn handle_connection(
    conn: Connection,
    identity: Arc<DeviceIdentity>,
    transport: Arc<QuicTransport>,
    ctx: TransportHandlerCtx,
) -> Result<(), CoreError> {
    let remote_addr = conn.remote_address();
    let certs = conn
        .peer_identity()
        .and_then(|any| any.downcast::<Vec<CertificateDer<'static>>>().ok())
        .ok_or(CoreError::UntrustedPeer)?;
    let first_cert = certs.first().ok_or(CoreError::UntrustedPeer)?;
    let public_key = extract_verified_peer_pubkey(first_cert.as_ref())?;
    let peer_id = PeerId::from_public_key(&public_key);

    transport
        .connections
        .lock()
        .expect("connections lock poisoned")
        .insert(peer_id, conn.clone());

    loop {
        let (send, mut recv) = conn.accept_bi().await?;
        let mut tag = [0u8; 1];
        if recv.read_exact(&mut tag).await.is_err() {
            continue;
        }
        let Some(kind) = StreamKind::from_u8(tag[0]) else {
            warn!("unknown stream kind {} from {}", tag[0], peer_id);
            continue;
        };

        let stored = ctx.storage.get_peer(&peer_id).ok().flatten();
        let is_trusted = stored.as_ref().map(|p| p.is_trusted).unwrap_or(false);
        let display_name = stored
            .as_ref()
            .and_then(|p| p.alias_name.clone().or_else(|| Some(p.device_name.clone())))
            .unwrap_or_else(|| format!("Peer {}", &peer_id.to_hex()[..8]));

        let peer_ctx = PeerContext {
            peer_id,
            public_key,
            remote_addr,
            is_trusted,
            display_name,
        };

        let ctx = ctx.clone();
        let identity = Arc::clone(&identity);

        tokio::spawn(async move {
            let res = match kind {
                StreamKind::FriendHandshake => {
                    handle_friend_stream(peer_ctx, send, recv, &identity, &ctx.storage, &ctx.events)
                        .await
                }
                StreamKind::FileTransfer => {
                    let (download_dir, auto_accept) = {
                        let guard = ctx.settings.read().expect("settings lock poisoned");
                        (
                            PathBuf::from(&guard.download_dir),
                            guard.auto_accept_from_friends,
                        )
                    };
                    ctx.transfers
                        .handle_incoming_transfer(peer_ctx, send, recv, download_dir, auto_accept)
                        .await
                }
                StreamKind::Shares | StreamKind::Clipboard => {
                    ctx.modules.dispatch_stream(peer_ctx, kind, send, recv).await
                }
            };
            if let Err(err) = res {
                debug!("stream {kind:?} error: {err}");
            }
        });
    }
}

async fn handle_friend_stream(
    peer: PeerContext,
    mut send: SendStream,
    mut recv: RecvStream,
    identity: &DeviceIdentity,
    storage: &Storage,
    events: &broadcast::Sender<CoreEvent>,
) -> Result<(), CoreError> {
    let msg: FriendMessage = read_frame(&mut recv, MAX_CONTROL_FRAME_BYTES).await?;

    match msg {
        FriendMessage::Request {
            device_name,
            os,
            category,
        } => {
            let sas = DeviceIdentity::sas_code(&identity.public_key_bytes(), &peer.public_key);
            storage.upsert_peer(
                &peer.peer_id,
                &peer.public_key,
                &device_name,
                os,
                category,
                Some(&peer.remote_addr.ip().to_string()),
                None,
            )?;
            storage.upsert_friend_request(
                &peer.peer_id,
                &peer.public_key,
                &device_name,
                os,
                category,
                &sas,
                "incoming",
            )?;
            let _ = events.send(CoreEvent::FriendRequestsChanged);
            let _ = events.send(CoreEvent::PeersChanged);
        }
        FriendMessage::Accept {
            device_name,
            os,
            category,
        } => {
            storage.upsert_peer(
                &peer.peer_id,
                &peer.public_key,
                &device_name,
                os,
                category,
                Some(&peer.remote_addr.ip().to_string()),
                None,
            )?;
            storage.set_peer_trusted(&peer.peer_id, true)?;
            storage.remove_friend_request(&peer.peer_id)?;
            let _ = events.send(CoreEvent::FriendRequestsChanged);
            let _ = events.send(CoreEvent::PeersChanged);
        }
        FriendMessage::Reject | FriendMessage::Unfriend => {
            storage.set_peer_trusted(&peer.peer_id, false)?;
            storage.remove_friend_request(&peer.peer_id)?;
            let _ = events.send(CoreEvent::FriendRequestsChanged);
            let _ = events.send(CoreEvent::PeersChanged);
        }
    }

    let ack = FriendMessage::Accept {
        device_name: String::new(),
        os: current_os(),
        category: current_category(),
    };
    let _ = write_frame(&mut send, &ack).await;
    let _ = send.finish();
    Ok(())
}
