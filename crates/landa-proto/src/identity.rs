use std::fmt;
use std::fs;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::error::ProtoError;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub [u8; 16]);

impl PeerId {
    pub fn from_public_key(public_key: &[u8; 32]) -> Self {
        let hash = blake3::hash(public_key);
        let mut id = [0u8; 16];
        id.copy_from_slice(&hash.as_bytes()[..16]);
        Self(id)
    }

    pub fn to_hex(&self) -> String {
        let mut out = String::with_capacity(32);
        for b in self.0 {
            use std::fmt::Write;
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    pub fn from_hex(s: &str) -> Result<Self, ProtoError> {
        let s = s.trim();
        if s.len() != 32 {
            return Err(ProtoError::InvalidCrypto);
        }
        let mut bytes = [0u8; 16];
        let (chunks, _) = s.as_bytes().as_chunks::<2>();
        for (i, chunk) in chunks.iter().enumerate() {
            let hex_str = std::str::from_utf8(chunk).map_err(|_| ProtoError::InvalidCrypto)?;
            bytes[i] = u8::from_str_radix(hex_str, 16).map_err(|_| ProtoError::InvalidCrypto)?;
        }
        Ok(Self(bytes))
    }

    pub fn short_fingerprint(&self) -> String {
        let hex = self.to_hex();
        format!("{}-{}", &hex[0..4], &hex[4..8]).to_uppercase()
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerId({})", self.to_hex())
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Clone)]
pub struct DeviceIdentity {
    signing_key: SigningKey,
    peer_id: PeerId,
}

impl DeviceIdentity {
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let peer_id = PeerId::from_public_key(signing_key.verifying_key().as_bytes());
        Self {
            signing_key,
            peer_id,
        }
    }

    pub fn from_secret_bytes(secret: [u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(&secret);
        let peer_id = PeerId::from_public_key(signing_key.verifying_key().as_bytes());
        Self {
            signing_key,
            peer_id,
        }
    }

    pub fn load_or_generate(path: &Path) -> Result<Self, ProtoError> {
        if path.exists() {
            let bytes = fs::read(path)?;
            if bytes.len() != 32 {
                return Err(ProtoError::InvalidCrypto);
            }
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            return Ok(Self::from_secret_bytes(secret));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let identity = Self::generate();
        fs::write(path, identity.secret_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o600);
            let _ = fs::set_permissions(path, perms);
        }
        Ok(identity)
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }

    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing_key.sign(message).to_bytes()
    }

    pub fn verify(
        public_key: &[u8; 32],
        message: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), ProtoError> {
        let vk = VerifyingKey::from_bytes(public_key).map_err(|_| ProtoError::InvalidCrypto)?;
        let sig = Signature::from_bytes(signature);
        vk.verify_strict(message, &sig)
            .map_err(|_| ProtoError::InvalidCrypto)
    }

    /// Deterministic 6-digit Short Authentication String (SAS) shared by both peers.
    pub fn sas_code(pk_a: &[u8; 32], pk_b: &[u8; 32]) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"LANDA2_SAS_V1");
        if pk_a <= pk_b {
            hasher.update(pk_a);
            hasher.update(pk_b);
        } else {
            hasher.update(pk_b);
            hasher.update(pk_a);
        }
        let digest = hasher.finalize();
        let bytes = digest.as_bytes();
        let num = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
        format!("{num:06}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sas_code_is_symmetric() {
        let a = DeviceIdentity::generate();
        let b = DeviceIdentity::generate();
        let sas_ab = DeviceIdentity::sas_code(&a.public_key_bytes(), &b.public_key_bytes());
        let sas_ba = DeviceIdentity::sas_code(&b.public_key_bytes(), &a.public_key_bytes());
        assert_eq!(sas_ab, sas_ba);
        assert_eq!(sas_ab.len(), 6);
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let id = DeviceIdentity::generate();
        let msg = b"hello landa v2";
        let sig = id.sign(msg);
        assert!(DeviceIdentity::verify(&id.public_key_bytes(), msg, &sig).is_ok());
        assert!(DeviceIdentity::verify(&id.public_key_bytes(), b"tampered", &sig).is_err());
    }
}
