use std::collections::HashMap;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use landa_proto::tls::{
    build_client_config, build_server_config, extract_verified_peer_pubkey,
};
use landa_proto::wire::{
    read_frame, write_frame, FriendMessage, StreamKind, MAX_CONTROL_FRAME_BYTES,
};
use landa_proto::{DeviceIdentity, PeerId};
use quinn::{Connection, Endpoint, RecvStream, SendStream};
use rustls::pki_types::CertificateDer;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::discovery::{current_category, current_os};
use crate::error::CoreError;
use crate::events::CoreEvent;
use crate::module::{ModuleRegistry, PeerContext};
use crate::storage::{AppSettings, Storage};
use crate::transfer::TransferManager;

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
    local_port: u16,
    connections: Mutex<HashMap<PeerId, Connection>>,
    handler_ctx: OnceLock<TransportHandlerCtx>,
}

impl QuicTransport {
    pub fn bind(identity: Arc<DeviceIdentity>) -> Result<Self, CoreError> {
        Self::bind_ip(identity, std::net::Ipv4Addr::UNSPECIFIED)
    }

    pub fn bind_ip(
        identity: Arc<DeviceIdentity>,
        ip: std::net::Ipv4Addr,
    ) -> Result<Self, CoreError> {
        let server_tls = build_server_config(&identity)?;
        let quic_server_crypto = quinn::crypto::rustls::QuicServerConfig::try_from(server_tls)
            .map_err(|e| CoreError::InvalidInput(format!("quic server config: {e}")))?;
        let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server_crypto));

        let bind_addr = SocketAddr::V4(SocketAddrV4::new(ip, 0));
        let endpoint = Endpoint::server(server_config, bind_addr)?;
        let local_port = endpoint.local_addr()?.port();

        Ok(Self {
            identity,
            endpoint,
            local_port,
            connections: Mutex::new(HashMap::new()),
            handler_ctx: OnceLock::new(),
        })
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    pub fn set_handler_ctx(&self, ctx: TransportHandlerCtx) {
        let _ = self.handler_ctx.set(ctx);
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
            if existing.close_reason().is_none() {
                return Ok(existing);
            }
        }

        let client_tls = build_client_config(&self.identity, Some(expected_peer_id))?;
        let quic_client_crypto = quinn::crypto::rustls::QuicClientConfig::try_from(client_tls)
            .map_err(|e| CoreError::InvalidInput(format!("quic client config: {e}")))?;
        let client_config = quinn::ClientConfig::new(Arc::new(quic_client_crypto));

        let conn = self
            .endpoint
            .connect_with(client_config, addr, "landa.peer")?
            .await?;

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
        let (mut send, recv) = conn.open_bi().await?;
        send.write_all(&[kind as u8]).await?;
        Ok((send, recv))
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
