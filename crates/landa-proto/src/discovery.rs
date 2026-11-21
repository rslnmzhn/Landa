use serde::{Deserialize, Serialize};

use crate::error::ProtoError;
use crate::identity::{DeviceIdentity, PeerId};

pub const DISCOVERY_MAGIC: &[u8; 4] = b"LND2";
pub const DISCOVERY_VERSION: u8 = 2;
pub const DISCOVERY_PORT: u16 = 40405;
pub const MAX_BEACON_BYTES: usize = 1024;
pub const MAX_DEVICE_NAME_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceOs {
    Windows,
    Linux,
    MacOs,
    Android,
    Ios,
    Unknown,
}

impl DeviceOs {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "android") {
            Self::Android
        } else if cfg!(target_os = "ios") {
            Self::Ios
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceCategory {
    Desktop,
    Phone,
    Unknown,
}

impl DeviceCategory {
    pub fn current() -> Self {
        if cfg!(any(target_os = "android", target_os = "ios")) {
            Self::Phone
        } else if cfg!(any(
            target_os = "windows",
            target_os = "linux",
            target_os = "macos"
        )) {
            Self::Desktop
        } else {
            Self::Unknown
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BeaconPayload {
    version: u8,
    peer_id: PeerId,
    public_key: [u8; 32],
    quic_port: u16,
    device_name: String,
    os: DeviceOs,
    category: DeviceCategory,
    timestamp_ms: i64,
    nonce: u64,
    capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignedBeaconWire {
    payload: BeaconPayload,
    signature: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct VerifiedBeacon {
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub quic_port: u16,
    pub device_name: String,
    pub os: DeviceOs,
    pub category: DeviceCategory,
    pub timestamp_ms: i64,
    pub nonce: u64,
    pub capabilities: Vec<String>,
}

pub fn encode_signed_beacon(
    identity: &DeviceIdentity,
    quic_port: u16,
    device_name: &str,
    timestamp_ms: i64,
    nonce: u64,
    capabilities: Vec<String>,
) -> Result<Vec<u8>, ProtoError> {
    let truncated_name: String = device_name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_DEVICE_NAME_BYTES)
        .collect();
    let safe_name = if truncated_name.is_empty() {
        "Landa Device".to_string()
    } else {
        truncated_name
    };

    let payload = BeaconPayload {
        version: DISCOVERY_VERSION,
        peer_id: identity.peer_id(),
        public_key: identity.public_key_bytes(),
        quic_port,
        device_name: safe_name,
        os: DeviceOs::current(),
        category: DeviceCategory::current(),
        timestamp_ms,
        nonce,
        capabilities,
    };

    let payload_bytes =
        postcard::to_allocvec(&payload).map_err(|e| ProtoError::Codec(e.to_string()))?;
    let mut sign_input = Vec::with_capacity(DISCOVERY_MAGIC.len() + payload_bytes.len());
    sign_input.extend_from_slice(DISCOVERY_MAGIC);
    sign_input.extend_from_slice(&payload_bytes);
    let sig = identity.sign(&sign_input);

    let wire = SignedBeaconWire {
        payload,
        signature: sig.to_vec(),
    };
    let wire_bytes = postcard::to_allocvec(&wire).map_err(|e| ProtoError::Codec(e.to_string()))?;

    let mut out = Vec::with_capacity(DISCOVERY_MAGIC.len() + wire_bytes.len());
    out.extend_from_slice(DISCOVERY_MAGIC);
    out.extend_from_slice(&wire_bytes);
    if out.len() > MAX_BEACON_BYTES {
        return Err(ProtoError::FrameTooLarge {
            size: out.len(),
            max: MAX_BEACON_BYTES,
        });
    }
    Ok(out)
}

pub fn decode_and_verify_beacon(datagram: &[u8]) -> Result<VerifiedBeacon, ProtoError> {
    if datagram.len() > MAX_BEACON_BYTES {
        return Err(ProtoError::FrameTooLarge {
            size: datagram.len(),
            max: MAX_BEACON_BYTES,
        });
    }
    if datagram.len() < DISCOVERY_MAGIC.len() || &datagram[..4] != DISCOVERY_MAGIC {
        return Err(ProtoError::InvalidBeacon("bad magic"));
    }

    let wire: SignedBeaconWire =
        postcard::from_bytes(&datagram[4..]).map_err(|e| ProtoError::Codec(e.to_string()))?;

    if wire.payload.version != DISCOVERY_VERSION {
        return Err(ProtoError::InvalidBeacon("unsupported version"));
    }
    if wire.payload.quic_port == 0 {
        return Err(ProtoError::InvalidBeacon("invalid quic port"));
    }
    if wire.signature.len() != 64 {
        return Err(ProtoError::InvalidBeacon("invalid signature length"));
    }

    let expected_peer_id = PeerId::from_public_key(&wire.payload.public_key);
    if expected_peer_id != wire.payload.peer_id {
        return Err(ProtoError::InvalidBeacon("peer_id mismatch"));
    }

    let payload_bytes =
        postcard::to_allocvec(&wire.payload).map_err(|e| ProtoError::Codec(e.to_string()))?;
    let mut sign_input = Vec::with_capacity(DISCOVERY_MAGIC.len() + payload_bytes.len());
    sign_input.extend_from_slice(DISCOVERY_MAGIC);
    sign_input.extend_from_slice(&payload_bytes);

    let mut sig = [0u8; 64];
    sig.copy_from_slice(&wire.signature);
    DeviceIdentity::verify(&wire.payload.public_key, &sign_input, &sig)?;

    let clean_name: String = wire
        .payload
        .device_name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_DEVICE_NAME_BYTES)
        .collect();

    Ok(VerifiedBeacon {
        peer_id: wire.payload.peer_id,
        public_key: wire.payload.public_key,
        quic_port: wire.payload.quic_port,
        device_name: if clean_name.is_empty() {
            "Landa Device".to_string()
        } else {
            clean_name
        },
        os: wire.payload.os,
        category: wire.payload.category,
        timestamp_ms: wire.payload.timestamp_ms,
        nonce: wire.payload.nonce,
        capabilities: wire.payload.capabilities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_roundtrip_and_tamper_rejection() {
        let id = DeviceIdentity::generate();
        let mut bytes = encode_signed_beacon(
            &id,
            40406,
            "My Workstation",
            1_700_000_000_000,
            42,
            vec!["shares".into(), "clipboard".into()],
        )
        .unwrap();

        let decoded = decode_and_verify_beacon(&bytes).unwrap();
        assert_eq!(decoded.peer_id, id.peer_id());
        assert_eq!(decoded.quic_port, 40406);
        assert_eq!(decoded.device_name, "My Workstation");

        // Tamper last byte of signature or payload
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert!(decode_and_verify_beacon(&bytes).is_err());
    }
}
