use landa_proto::ProtoError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("protocol error: {0}")]
    Proto(#[from] ProtoError),

    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("quic connect error: {0}")]
    QuinnConnect(#[from] quinn::ConnectError),

    #[error("quic connection error: {0}")]
    QuinnConnection(#[from] quinn::ConnectionError),

    #[error("quic write error: {0}")]
    QuinnWrite(#[from] quinn::WriteError),

    #[error("quic read error: {0}")]
    QuinnRead(#[from] quinn::ReadError),

    #[error("peer is not trusted")]
    UntrustedPeer,

    #[error("module '{0}' is disabled or unavailable")]
    ModuleDisabled(String),

    #[error("request rejected by peer")]
    Rejected,

    #[error("transfer cancelled")]
    Cancelled,

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("integrity check failed for {0}")]
    IntegrityMismatch(String),
}
