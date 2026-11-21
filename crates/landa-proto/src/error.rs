use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("invalid cryptographic key or signature")]
    InvalidCrypto,

    #[error("invalid discovery beacon: {0}")]
    InvalidBeacon(&'static str),

    #[error("frame exceeds maximum allowed size ({size} > {max})")]
    FrameTooLarge { size: usize, max: usize },

    #[error("unexpected end of stream")]
    UnexpectedEof,

    #[error("serialization error: {0}")]
    Codec(String),

    #[error("unsafe or invalid path: {0}")]
    UnsafePath(String),

    #[error("tls verification failed: {0}")]
    Tls(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
