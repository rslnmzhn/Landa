use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::discovery::{DeviceCategory, DeviceOs};
use crate::error::ProtoError;

pub const MAX_CONTROL_FRAME_BYTES: usize = 2 * 1024 * 1024; // 2 MiB hard cap

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum StreamKind {
    FriendHandshake = 0x01,
    FileTransfer = 0x02,
    Shares = 0x10,
    Clipboard = 0x11,
}

impl StreamKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::FriendHandshake),
            0x02 => Some(Self::FileTransfer),
            0x10 => Some(Self::Shares),
            0x11 => Some(Self::Clipboard),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FriendMessage {
    Request {
        device_name: String,
        os: DeviceOs,
        category: DeviceCategory,
    },
    Accept {
        device_name: String,
        os: DeviceOs,
        category: DeviceCategory,
    },
    Reject,
    Unfriend,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferFileMeta {
    pub relative_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferOffer {
    pub transfer_id: String,
    pub files: Vec<TransferFileMeta>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferDecision {
    pub accepted: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResultAck {
    pub ok: bool,
    pub message: Option<String>,
}

pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), ProtoError> {
    let bytes = postcard::to_allocvec(value).map_err(|e| ProtoError::Codec(e.to_string()))?;
    if bytes.len() > MAX_CONTROL_FRAME_BYTES {
        return Err(ProtoError::FrameTooLarge {
            size: bytes.len(),
            max: MAX_CONTROL_FRAME_BYTES,
        });
    }
    let len = bytes.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&bytes).await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin, T: for<'de> Deserialize<'de>>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<T, ProtoError> {
    let cap = max_bytes.min(MAX_CONTROL_FRAME_BYTES);
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtoError::UnexpectedEof
        } else {
            ProtoError::Io(e)
        }
    })?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > cap {
        return Err(ProtoError::FrameTooLarge {
            size: len,
            max: cap,
        });
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtoError::UnexpectedEof
        } else {
            ProtoError::Io(e)
        }
    })?;
    postcard::from_bytes(&buf).map_err(|e| ProtoError::Codec(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_oversized_frame_before_allocating() {
        let mut buf = Vec::new();
        let huge_len: u32 = (MAX_CONTROL_FRAME_BYTES as u32) + 1;
        buf.extend_from_slice(&huge_len.to_be_bytes());
        let mut cursor = std::io::Cursor::new(buf);
        let res: Result<TransferDecision, _> =
            read_frame(&mut cursor, MAX_CONTROL_FRAME_BYTES).await;
        assert!(matches!(res, Err(ProtoError::FrameTooLarge { .. })));
    }
}
