pub mod discovery;
pub mod engine;
pub mod error;
pub mod events;
pub mod module;
pub mod network;
pub mod qr;
pub mod storage;
pub mod transfer;
pub mod transport;

pub use engine::{LandaConfig, LandaEngine, LocalIdentityInfo};
pub use error::CoreError;
pub use events::{
    CoreEvent, FriendRequestInfo, IncomingTransferOfferInfo, ManualTarget, PeerInfo,
    TransferHistoryRecord, TransferProgressInfo,
};
pub use module::{LandaModule, ModuleRegistry, ModuleStatus, PeerContext};
pub use network::{
    set_lan_interface_hook, set_socket_bind_hook, LanInterface, LanInterfaceHook,
    LanInterfaceStatus, SocketBindHook,
};
pub use qr::{build_qr_connect_uri, parse_qr_connect_uri, pick_primary_lan_ip, QrConnectPayload};
pub use storage::{AppSettings, Storage};
pub use transfer::{OpenSourceHook, OutgoingFileItem, ReceivePublishHook};
