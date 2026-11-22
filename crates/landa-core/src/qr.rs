use std::net::Ipv4Addr;

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use landa_proto::{DeviceIdentity, PeerId};

use crate::error::CoreError;
use crate::network::enumerate_lan_interfaces;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrConnectPayload {
    pub ip: Ipv4Addr,
    pub port: u16,
    pub peer_id: PeerId,
    pub public_key: [u8; 32],
    pub device_name: String,
}

pub fn pick_primary_lan_ip() -> Option<Ipv4Addr> {
    let ifaces = enumerate_lan_interfaces();
    ifaces
        .iter()
        .find(|i| i.ip.is_private())
        .or_else(|| ifaces.first())
        .map(|i| i.ip)
}

pub fn build_qr_connect_uri(
    ip: Ipv4Addr,
    port: u16,
    peer_id: PeerId,
    public_key: &[u8; 32],
    device_name: &str,
) -> String {
    let pk_b64 = URL_SAFE_NO_PAD.encode(public_key);
    let encoded_name = percent_encode(device_name.trim());
    format!(
        "landa2://connect?ip={ip}&port={port}&peer={}&pk={pk_b64}&name={encoded_name}",
        peer_id.to_hex()
    )
}

pub fn parse_qr_connect_uri(raw: &str) -> Result<QrConnectPayload, CoreError> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("landa2://connect?")
        .ok_or_else(|| CoreError::InvalidInput("invalid QR link scheme (expected landa2://connect)".into()))?;

    let mut ip_str: Option<String> = None;
    let mut port_str: Option<String> = None;
    let mut peer_str: Option<String> = None;
    let mut pk_str: Option<String> = None;
    let mut name_str: Option<String> = None;

    for pair in rest.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair
            .split_once('=')
            .ok_or_else(|| CoreError::InvalidInput("malformed QR query parameter".into()))?;
        let decoded_val = percent_decode(v)?;
        match k {
            "ip" => ip_str = Some(decoded_val),
            "port" => port_str = Some(decoded_val),
            "peer" => peer_str = Some(decoded_val),
            "pk" => pk_str = Some(decoded_val),
            "name" => name_str = Some(decoded_val),
            _ => {}
        }
    }

    let ip: Ipv4Addr = ip_str
        .ok_or_else(|| CoreError::InvalidInput("QR link missing ip".into()))?
        .trim()
        .parse()
        .map_err(|_| CoreError::InvalidInput("QR link has invalid IPv4 address".into()))?;
    if ip.is_unspecified() || ip.is_multicast() {
        return Err(CoreError::InvalidInput("QR link IP address is not routable".into()));
    }

    let port: u16 = port_str
        .ok_or_else(|| CoreError::InvalidInput("QR link missing port".into()))?
        .trim()
        .parse()
        .map_err(|_| CoreError::InvalidInput("QR link has invalid port".into()))?;
    if port == 0 {
        return Err(CoreError::InvalidInput("QR link port must be non-zero".into()));
    }

    let peer_hex = peer_str.ok_or_else(|| CoreError::InvalidInput("QR link missing peer".into()))?;
    let peer_id = PeerId::from_hex(&peer_hex)
        .map_err(|_| CoreError::InvalidInput("QR link has invalid peer id".into()))?;

    let pk_raw = pk_str.ok_or_else(|| CoreError::InvalidInput("QR link missing public key".into()))?;
    let pk_bytes = decode_base64_32(pk_raw.trim())?;
    DeviceIdentity::validate_public_key(&pk_bytes)
        .map_err(|_| CoreError::InvalidInput("QR link has invalid Ed25519 public key".into()))?;

    if PeerId::from_public_key(&pk_bytes) != peer_id {
        return Err(CoreError::InvalidInput(
            "QR link peer id does not match public key".into(),
        ));
    }

    let device_name = name_str
        .map(|s| s.trim().chars().take(64).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Peer {}", &peer_id.to_hex()[..8]));

    Ok(QrConnectPayload {
        ip,
        port,
        peer_id,
        public_key: pk_bytes,
        device_name,
    })
}

fn decode_base64_32(input: &str) -> Result<[u8; 32], CoreError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(input)
        .or_else(|_| URL_SAFE.decode(input))
        .or_else(|_| STANDARD_NO_PAD.decode(input))
        .or_else(|_| STANDARD.decode(input))
        .map_err(|_| CoreError::InvalidInput("QR link has invalid base64 public key".into()))?;
    if decoded.len() != 32 {
        return Err(CoreError::InvalidInput(
            "QR link public key must be 32 bytes".into(),
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&decoded);
    Ok(out)
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

fn percent_decode(input: &str) -> Result<String, CoreError> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err(CoreError::InvalidInput("invalid percent-encoding in QR link".into()));
                }
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3])
                    .map_err(|_| CoreError::InvalidInput("invalid percent-encoding in QR link".into()))?;
                let val = u8::from_str_radix(hex, 16)
                    .map_err(|_| CoreError::InvalidInput("invalid percent-encoding in QR link".into()))?;
                out.push(val);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out)
        .map_err(|_| CoreError::InvalidInput("QR link contains invalid UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_uri_roundtrip_with_utf8_name() {
        let id = DeviceIdentity::generate();
        let ip = Ipv4Addr::new(192, 168, 1, 42);
        let uri = build_qr_connect_uri(
            ip,
            40406,
            id.peer_id(),
            &id.public_key_bytes(),
            "Телефон Руслана (Pixel)",
        );
        assert!(uri.starts_with("landa2://connect?"));

        let parsed = parse_qr_connect_uri(&uri).unwrap();
        assert_eq!(parsed.ip, ip);
        assert_eq!(parsed.port, 40406);
        assert_eq!(parsed.peer_id, id.peer_id());
        assert_eq!(parsed.public_key, id.public_key_bytes());
        assert_eq!(parsed.device_name, "Телефон Руслана (Pixel)");
    }

    #[test]
    fn qr_uri_rejects_mismatched_peer_id_and_public_key() {
        let id_a = DeviceIdentity::generate();
        let id_b = DeviceIdentity::generate();
        let uri = build_qr_connect_uri(
            Ipv4Addr::new(10, 0, 0, 5),
            40406,
            id_a.peer_id(),
            &id_b.public_key_bytes(),
            "Spoofed",
        );
        assert!(parse_qr_connect_uri(&uri).is_err());
    }

    #[test]
    fn qr_uri_rejects_invalid_scheme_and_unroutable_ip() {
        let id = DeviceIdentity::generate();
        assert!(parse_qr_connect_uri("https://example.com").is_err());
        let bad_ip = build_qr_connect_uri(
            Ipv4Addr::UNSPECIFIED,
            40406,
            id.peer_id(),
            &id.public_key_bytes(),
            "Bad",
        );
        assert!(parse_qr_connect_uri(&bad_ip).is_err());
    }
}
