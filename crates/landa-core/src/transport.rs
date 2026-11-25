use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use landa_proto::tls::{build_client_config, build_server_config, extract_verified_peer_pubkey};
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
const OPEN_STREAM_TIMEOUT: Duration = Duration::from_secs(4);

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
        self.connections
            .lock()
            .expect("connections lock poisoned")
            .retain(|_, conn| conn.close_reason().is_none());
    }

    pub fn set_handler_ctx(&self, ctx: TransportHandlerCtx) {
        let _ = self.handler_ctx.set(ctx);
    }

    pub fn disconnect_peer(&self, peer_id: &PeerId) {
        self.evict_connection(peer_id, None);
    }

    fn evict_connection(&self, peer_id: &PeerId, expected: Option<&Connection>) {
        let removed = {
            let mut guard = self.connections.lock().expect("connections lock poisoned");
            let should_remove = match (guard.get(peer_id), expected) {
                (Some(current), Some(exp)) => current.stable_id() == exp.stable_id(),
                (Some(_), None) => true,
                (None, _) => false,
            };
            if should_remove {
                guard.remove(peer_id)
            } else {
                None
            }
        };
        if let Some(conn) = removed {
            conn.close(0u32.into(), b"reconnecting");
        }
    }

    pub async fn connect_peer(
        self: &Arc<Self>,
        addr: SocketAddr,
        expected_peer_id: PeerId,
    ) -> Result<Connection, CoreError> {
        let existing = {
            self.connections
                .lock()
                .expect("connections lock poisoned")
                .get(&expected_peer_id)
                .cloned()
        };
        if let Some(existing) = existing {
            if existing.close_reason().is_none() && existing.remote_address() == addr {
                return Ok(existing);
            }
            self.evict_connection(&expected_peer_id, Some(&existing));
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
        self.open_stream_with_timeout(addr, expected_peer_id, kind, OPEN_STREAM_TIMEOUT)
            .await
    }

    async fn open_stream_with_timeout(
        self: &Arc<Self>,
        addr: SocketAddr,
        expected_peer_id: PeerId,
        kind: StreamKind,
        stream_timeout: Duration,
    ) -> Result<(SendStream, RecvStream), CoreError> {
        let conn = self.connect_peer(addr, expected_peer_id).await?;
        match open_bi_with_kind(&conn, kind, stream_timeout).await {
            Ok(streams) => Ok(streams),
            Err(_) => {
                // Cached connection may have gone stale; evict and reconnect once
                self.evict_connection(&expected_peer_id, Some(&conn));
                let fresh = self.connect_peer(addr, expected_peer_id).await?;
                match open_bi_with_kind(&fresh, kind, stream_timeout).await {
                    Ok(streams) => Ok(streams),
                    Err(err) => {
                        self.evict_connection(&expected_peer_id, Some(&fresh));
                        Err(err)
                    }
                }
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
                    ctx.modules
                        .dispatch_stream(peer_ctx, kind, send, recv)
                        .await
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

async fn open_bi_with_kind(
    conn: &Connection,
    kind: StreamKind,
    stream_timeout: Duration,
) -> Result<(SendStream, RecvStream), CoreError> {
    let fut = async {
        let (mut send, recv) = conn.open_bi().await?;
        send.write_all(&[kind as u8]).await?;
        Ok::<_, CoreError>((send, recv))
    };
    tokio::time::timeout(stream_timeout, fut)
        .await
        .map_err(|_| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "QUIC open_bi timed out",
            ))
        })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::ModuleRegistry;
    use crate::transfer::TransferManager;
    use landa_proto::{DeviceCategory, DeviceOs};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_test_ctx(dir: &std::path::Path) -> TransportHandlerCtx {
        let storage = Arc::new(Storage::open(&dir.join("test.sqlite")).unwrap());
        let (events, _) = broadcast::channel(16);
        let transfers = Arc::new(TransferManager::new(
            Arc::clone(&storage),
            events.clone(),
            dir.join("temp"),
        ));
        TransportHandlerCtx {
            storage,
            modules: Arc::new(ModuleRegistry::new()),
            transfers,
            events,
            settings: Arc::new(RwLock::new(AppSettings::default_with(
                "Test".into(),
                dir.join("dl"),
            ))),
        }
    }

    #[tokio::test]
    async fn wildcard_quic_endpoints_fall_back_to_distinct_ports_and_remain_reachable() {
        let base = std::env::temp_dir().join(format!(
            "landa-transport-wildcard-test-{}",
            uuid::Uuid::new_v4()
        ));
        let dir_1 = base.join("ep1");
        let dir_2 = base.join("ep2");
        let dir_c = base.join("client");
        std::fs::create_dir_all(&dir_1).unwrap();
        std::fs::create_dir_all(&dir_2).unwrap();
        std::fs::create_dir_all(&dir_c).unwrap();

        let id_1 = Arc::new(DeviceIdentity::load_or_generate(&dir_1.join("id.key")).unwrap());
        let id_2 = Arc::new(DeviceIdentity::load_or_generate(&dir_2.join("id.key")).unwrap());
        let id_c = Arc::new(DeviceIdentity::load_or_generate(&dir_c.join("id.key")).unwrap());
        let peer_1 = id_1.peer_id();
        let peer_2 = id_2.peer_id();

        let ep1 = Arc::new(QuicTransport::bind_ip(id_1, Ipv4Addr::UNSPECIFIED).unwrap());
        let ep2 = Arc::new(QuicTransport::bind_ip(id_2, Ipv4Addr::UNSPECIFIED).unwrap());
        let client = Arc::new(QuicTransport::bind_ip(id_c, Ipv4Addr::LOCALHOST).unwrap());

        assert_ne!(
            ep1.local_port(),
            ep2.local_port(),
            "second wildcard endpoint must fall back to a distinct port when primary port is occupied"
        );
        assert_ne!(
            ep2.local_port(),
            DEFAULT_QUIC_PORT,
            "second wildcard endpoint must not share DEFAULT_QUIC_PORT"
        );

        let cancel_1 = CancellationToken::new();
        let cancel_2 = CancellationToken::new();
        let ep1_loop = Arc::clone(&ep1);
        let ep2_loop = Arc::clone(&ep2);
        let ctx_1 = make_test_ctx(&dir_1);
        let ctx_2 = make_test_ctx(&dir_2);
        let c1 = cancel_1.clone();
        let c2 = cancel_2.clone();
        tokio::spawn(async move {
            ep1_loop.run_accept_loop(ctx_1, c1).await;
        });
        tokio::spawn(async move {
            ep2_loop.run_accept_loop(ctx_2, c2).await;
        });

        let addr_1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), ep1.local_port());
        let addr_2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), ep2.local_port());

        let (mut s1, _r1) = client
            .open_stream(addr_1, peer_1, StreamKind::FriendHandshake)
            .await
            .unwrap();
        let _ = s1.finish();

        let (mut s2, _r2) = client
            .open_stream(addr_2, peer_2, StreamKind::FriendHandshake)
            .await
            .unwrap();
        let _ = s2.finish();

        cancel_1.cancel();
        cancel_2.cancel();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn open_stream_recovers_when_cached_connection_closes_or_peer_changes_port() {
        let base = std::env::temp_dir().join(format!("landa-transport-test-{}", uuid::Uuid::new_v4()));
        let dir_a = base.join("a");
        let dir_b1 = base.join("b1");
        let dir_b2 = base.join("b2");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b1).unwrap();
        std::fs::create_dir_all(&dir_b2).unwrap();

        let id_a = Arc::new(DeviceIdentity::load_or_generate(&dir_a.join("id.key")).unwrap());
        let id_b = Arc::new(DeviceIdentity::load_or_generate(&dir_b1.join("id.key")).unwrap());
        let id_a_peer_id = id_a.peer_id();
        let peer_id_b = id_b.peer_id();

        let transport_a = Arc::new(QuicTransport::bind_ip(id_a, Ipv4Addr::LOCALHOST).unwrap());
        let transport_b1 =
            Arc::new(QuicTransport::bind_ip(Arc::clone(&id_b), Ipv4Addr::LOCALHOST).unwrap());
        let cancel_b1 = CancellationToken::new();
        let b1_loop = Arc::clone(&transport_b1);
        let b1_ctx = make_test_ctx(&dir_b1);
        let b1_cancel = cancel_b1.clone();
        tokio::spawn(async move {
            b1_loop.run_accept_loop(b1_ctx, b1_cancel).await;
        });

        let addr_b1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), transport_b1.local_port());
        let (mut send1, _recv1) = transport_a
            .open_stream(addr_b1, peer_id_b, StreamKind::FriendHandshake)
            .await
            .unwrap();
        let _ = send1.finish();

        // Simulate remote peer closing the cached connection on the same address
        transport_b1.disconnect_peer(&id_a_peer_id);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (mut send2, _recv2) = transport_a
            .open_stream(addr_b1, peer_id_b, StreamKind::FriendHandshake)
            .await
            .unwrap();
        let _ = send2.finish();

        // Simulate peer restart on a new port while old connection is still cached
        cancel_b1.cancel();
        let transport_b2 = Arc::new(QuicTransport::bind_ip(id_b, Ipv4Addr::LOCALHOST).unwrap());
        let cancel_b2 = CancellationToken::new();
        let b2_loop = Arc::clone(&transport_b2);
        let b2_ctx = make_test_ctx(&dir_b2);
        let b2_cancel = cancel_b2.clone();
        tokio::spawn(async move {
            b2_loop.run_accept_loop(b2_ctx, b2_cancel).await;
        });

        let addr_b2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), transport_b2.local_port());
        assert_ne!(addr_b1.port(), addr_b2.port());
        let (mut send3, _recv3) = transport_a
            .open_stream(addr_b2, peer_id_b, StreamKind::FriendHandshake)
            .await
            .unwrap();
        let _ = send3.finish();

        cancel_b2.cancel();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn open_stream_evicts_fresh_connection_after_second_timeout_and_does_not_reuse_it() {
        let base = std::env::temp_dir().join(format!(
            "landa-transport-evict-test-{}",
            uuid::Uuid::new_v4()
        ));
        let dir_a = base.join("a");
        let dir_b = base.join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let id_a = Arc::new(DeviceIdentity::load_or_generate(&dir_a.join("id.key")).unwrap());
        let id_b = Arc::new(DeviceIdentity::load_or_generate(&dir_b.join("id.key")).unwrap());
        let peer_id_b = id_b.peer_id();

        let transport_a = Arc::new(QuicTransport::bind_ip(id_a, Ipv4Addr::LOCALHOST).unwrap());

        // Server endpoint that completes QUIC handshake but grants 0 bidi streams so open_bi times out
        let server_tls = build_server_config(&id_b).unwrap();
        let quic_server_crypto =
            quinn::crypto::rustls::QuicServerConfig::try_from(server_tls).unwrap();
        let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server_crypto));
        let mut tcfg = TransportConfig::default();
        tcfg.max_concurrent_bidi_streams(0u32.into());
        server_config.transport_config(Arc::new(tcfg));

        let std_sock =
            std::net::UdpSocket::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))
                .unwrap();
        let runtime = quinn::default_runtime().unwrap();
        let blocked_ep = Endpoint::new(
            EndpointConfig::default(),
            Some(server_config),
            std_sock,
            runtime,
        )
        .unwrap();
        let addr_b = blocked_ep.local_addr().unwrap();

        let accepted_count = Arc::new(AtomicUsize::new(0));
        let accepted_count_task = Arc::clone(&accepted_count);
        let blocked_ep_task = blocked_ep.clone();
        let accept_handle = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Some(incoming) = blocked_ep_task.accept().await {
                if let Ok(conn) = incoming.await {
                    accepted_count_task.fetch_add(1, Ordering::SeqCst);
                    held.push(conn);
                }
            }
        });

        let err = transport_a
            .open_stream_with_timeout(
                addr_b,
                peer_id_b,
                StreamKind::FriendHandshake,
                Duration::from_millis(120),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"));
        // Strictly 1 initial attempt + 1 retry
        assert_eq!(accepted_count.load(Ordering::SeqCst), 2);
        // Fresh connection must be evicted from cache after the second timeout
        assert!(transport_a
            .connections
            .lock()
            .expect("connections lock poisoned")
            .get(&peer_id_b)
            .is_none());

        // Next request to the same address must not reuse the evicted `fresh` connection
        let err2 = transport_a
            .open_stream_with_timeout(
                addr_b,
                peer_id_b,
                StreamKind::FriendHandshake,
                Duration::from_millis(120),
            )
            .await
            .unwrap_err();
        assert!(err2.to_string().contains("timed out"));
        assert_eq!(accepted_count.load(Ordering::SeqCst), 4);
        assert!(transport_a
            .connections
            .lock()
            .expect("connections lock poisoned")
            .get(&peer_id_b)
            .is_none());

        accept_handle.abort();
        let _ = accept_handle.await;
        blocked_ep.close(0u32.into(), b"done");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[tokio::test]
    async fn open_stream_reconnects_to_same_peer_id_on_new_port_while_old_connection_is_cached() {
        let base = std::env::temp_dir().join(format!(
            "landa-transport-port-change-test-{}",
            uuid::Uuid::new_v4()
        ));
        let dir_a = base.join("a");
        let dir_b1 = base.join("b1");
        let dir_b2 = base.join("b2");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b1).unwrap();
        std::fs::create_dir_all(&dir_b2).unwrap();

        let id_a = Arc::new(DeviceIdentity::load_or_generate(&dir_a.join("id.key")).unwrap());
        let id_b = Arc::new(DeviceIdentity::load_or_generate(&dir_b1.join("id.key")).unwrap());
        let id_a_peer_id = id_a.peer_id();
        let peer_id_b = id_b.peer_id();

        let transport_a = Arc::new(QuicTransport::bind_ip(id_a, Ipv4Addr::LOCALHOST).unwrap());
        let transport_b1 =
            Arc::new(QuicTransport::bind_ip(Arc::clone(&id_b), Ipv4Addr::LOCALHOST).unwrap());
        let cancel_b1 = CancellationToken::new();
        let b1_ctx = make_test_ctx(&dir_b1);
        let b1_loop = Arc::clone(&transport_b1);
        let b1_cancel = cancel_b1.clone();
        let b1_ctx_loop = b1_ctx.clone();
        tokio::spawn(async move {
            b1_loop.run_accept_loop(b1_ctx_loop, b1_cancel).await;
        });

        let addr_b1 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), transport_b1.local_port());
        let (mut send1, mut recv1) = transport_a
            .open_stream(addr_b1, peer_id_b, StreamKind::FriendHandshake)
            .await
            .unwrap();
        write_frame(
            &mut send1,
            &FriendMessage::Request {
                device_name: "Node A".into(),
                os: DeviceOs::Linux,
                category: DeviceCategory::Desktop,
            },
        )
        .await
        .unwrap();
        let _ = send1.finish();
        let ack1: FriendMessage = read_frame(&mut recv1, MAX_CONTROL_FRAME_BYTES)
            .await
            .unwrap();
        assert!(matches!(ack1, FriendMessage::Accept { .. }));

        // Verify old connection to addr_b1 is still alive and cached
        let old_cached = transport_a
            .connections
            .lock()
            .expect("connections lock poisoned")
            .get(&peer_id_b)
            .cloned()
            .expect("old connection must be cached");
        assert_eq!(old_cached.remote_address(), addr_b1);
        assert!(old_cached.close_reason().is_none());
        let old_stable_id = old_cached.stable_id();

        // Start a second endpoint with the same PeerId on a new port while old connection is still cached
        let transport_b2 = Arc::new(QuicTransport::bind_ip(id_b, Ipv4Addr::LOCALHOST).unwrap());
        let cancel_b2 = CancellationToken::new();
        let b2_ctx = make_test_ctx(&dir_b2);
        let b2_loop = Arc::clone(&transport_b2);
        let b2_cancel = cancel_b2.clone();
        let b2_ctx_loop = b2_ctx.clone();
        tokio::spawn(async move {
            b2_loop.run_accept_loop(b2_ctx_loop, b2_cancel).await;
        });

        let addr_b2 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), transport_b2.local_port());
        assert_ne!(addr_b1.port(), addr_b2.port());

        let (mut send2, mut recv2) = transport_a
            .open_stream(addr_b2, peer_id_b, StreamKind::FriendHandshake)
            .await
            .unwrap();
        write_frame(
            &mut send2,
            &FriendMessage::Request {
                device_name: "Node A".into(),
                os: DeviceOs::Linux,
                category: DeviceCategory::Desktop,
            },
        )
        .await
        .unwrap();
        let _ = send2.finish();
        let ack2: FriendMessage = read_frame(&mut recv2, MAX_CONTROL_FRAME_BYTES)
            .await
            .unwrap();
        assert!(matches!(ack2, FriendMessage::Accept { .. }));

        // Cache must now point to addr_b2 with a new Connection, and b2 must have received the request
        let new_cached = transport_a
            .connections
            .lock()
            .expect("connections lock poisoned")
            .get(&peer_id_b)
            .cloned()
            .expect("new connection must be cached");
        assert_eq!(new_cached.remote_address(), addr_b2);
        assert_ne!(new_cached.stable_id(), old_stable_id);
        let reqs_b2 = b2_ctx.storage.list_friend_requests().unwrap();
        assert_eq!(reqs_b2.len(), 1);
        assert_eq!(reqs_b2[0].peer_id, id_a_peer_id.to_hex());

        cancel_b1.cancel();
        cancel_b2.cancel();
        let _ = std::fs::remove_dir_all(&base);
    }
}
