use std::sync::Arc;

use rcgen::{CertificateParams, KeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::ring::default_provider;
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{ClientConfig, DistinguishedName, Error as TlsError, ServerConfig, SignatureScheme};
use x509_parser::prelude::*;

use crate::error::ProtoError;
use crate::identity::{DeviceIdentity, PeerId};

/// RFC 8410 PKCS#8 v1 DER prefix for Ed25519 private key (32-byte seed follows).
const ED25519_PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

pub struct CertifiedIdentity {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
}

pub fn build_certified_identity(identity: &DeviceIdentity) -> Result<CertifiedIdentity, ProtoError> {
    let mut pkcs8 = Vec::with_capacity(48);
    pkcs8.extend_from_slice(&ED25519_PKCS8_PREFIX);
    pkcs8.extend_from_slice(&identity.secret_bytes());

    let key_pair = KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(pkcs8.clone()),
        &rcgen::PKCS_ED25519,
    )
    .map_err(|e| ProtoError::Tls(e.to_string()))?;

    let peer_hex = identity.peer_id().to_hex();
    let mut params = CertificateParams::new(vec![format!("{peer_hex}.landa.local")])
        .map_err(|e| ProtoError::Tls(e.to_string()))?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, peer_hex);

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| ProtoError::Tls(e.to_string()))?;

    Ok(CertifiedIdentity {
        cert_der: cert.der().clone(),
        key_der: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pkcs8)),
    })
}

/// Extracts and cryptographically verifies the Ed25519 public key from a self-signed Landa X.509 cert.
pub fn extract_verified_peer_pubkey(cert_der: &[u8]) -> Result<[u8; 32], ProtoError> {
    let (_, cert) =
        X509Certificate::from_der(cert_der).map_err(|e| ProtoError::Tls(e.to_string()))?;

    let spki = cert.public_key();
    // Ed25519 OID: 1.3.101.112
    if spki.algorithm.algorithm.to_id_string() != "1.3.101.112" {
        return Err(ProtoError::Tls("certificate key is not Ed25519".into()));
    }
    let raw_pk = spki.subject_public_key.data.as_ref();
    if raw_pk.len() != 32 {
        return Err(ProtoError::Tls("invalid Ed25519 public key length".into()));
    }
    let mut pk = [0u8; 32];
    pk.copy_from_slice(raw_pk);

    // Verify self-signature on the X.509 TBS certificate
    let sig_bytes = cert.signature_value.data.as_ref();
    if sig_bytes.len() != 64 {
        return Err(ProtoError::Tls("invalid Ed25519 cert signature length".into()));
    }
    let mut sig = [0u8; 64];
    sig.copy_from_slice(sig_bytes);
    DeviceIdentity::verify(&pk, cert.tbs_certificate.as_ref(), &sig)?;

    Ok(pk)
}

pub fn build_server_config(identity: &DeviceIdentity) -> Result<ServerConfig, ProtoError> {
    let certified = build_certified_identity(identity)?;
    let provider = Arc::new(default_provider());
    let client_verifier = Arc::new(LandaPeerCertVerifier::any_peer(provider.signature_verification_algorithms));

    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ProtoError::Tls(e.to_string()))?
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(vec![certified.cert_der], certified.key_der)
        .map_err(|e| ProtoError::Tls(e.to_string()))?;

    config.alpn_protocols = vec![b"landa/2".to_vec()];
    Ok(config)
}

pub fn build_client_config(
    identity: &DeviceIdentity,
    expected_peer: Option<PeerId>,
) -> Result<ClientConfig, ProtoError> {
    let certified = build_certified_identity(identity)?;
    let provider = Arc::new(default_provider());
    let server_verifier = Arc::new(LandaPeerCertVerifier::new(
        expected_peer,
        provider.signature_verification_algorithms,
    ));

    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| ProtoError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(server_verifier)
        .with_client_auth_cert(vec![certified.cert_der], certified.key_der)
        .map_err(|e| ProtoError::Tls(e.to_string()))?;

    config.alpn_protocols = vec![b"landa/2".to_vec()];
    Ok(config)
}

#[derive(Debug)]
struct LandaPeerCertVerifier {
    expected_peer: Option<PeerId>,
    supported_algs: WebPkiSupportedAlgorithms,
}

impl LandaPeerCertVerifier {
    fn any_peer(supported_algs: WebPkiSupportedAlgorithms) -> Self {
        Self {
            expected_peer: None,
            supported_algs,
        }
    }

    fn new(expected_peer: Option<PeerId>, supported_algs: WebPkiSupportedAlgorithms) -> Self {
        Self {
            expected_peer,
            supported_algs,
        }
    }

    fn verify_peer_cert(&self, end_entity: &CertificateDer<'_>) -> Result<(), TlsError> {
        let pk = extract_verified_peer_pubkey(end_entity.as_ref())
            .map_err(|e| TlsError::General(e.to_string()))?;
        if let Some(expected) = self.expected_peer {
            let actual = PeerId::from_public_key(&pk);
            if actual != expected {
                return Err(TlsError::General(format!(
                    "peer identity mismatch: expected {expected}, got {actual}"
                )));
            }
        }
        Ok(())
    }
}

impl ServerCertVerifier for LandaPeerCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        self.verify_peer_cert(end_entity)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

impl ClientCertVerifier for LandaPeerCertVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, TlsError> {
        self.verify_peer_cert(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported_algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported_algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.supported_algs.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_roundtrip_extracts_matching_peer_id() {
        let id = DeviceIdentity::generate();
        let certified = build_certified_identity(&id).unwrap();
        let pk = extract_verified_peer_pubkey(certified.cert_der.as_ref()).unwrap();
        assert_eq!(pk, id.public_key_bytes());
        assert_eq!(PeerId::from_public_key(&pk), id.peer_id());
    }
}
