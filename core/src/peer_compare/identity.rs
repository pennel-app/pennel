use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use rustls::{
    DigitallySignedStruct, DistinguishedName, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::PeerCompareError;

/// Newly generated QA encryption identity, never an Apple code-signing identity.
#[derive(Clone, Serialize, Deserialize, uniffi::Record)]
pub struct PeerCompareIdentity {
    pub certificate_der_b64: String,
    pub private_key_der_b64: String,
    pub identity_id: String,
}

#[uniffi::export]
pub fn generate_peer_compare_identity() -> Result<PeerCompareIdentity, PeerCompareError> {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|_| PeerCompareError::Storage)?;
    let mut params = rcgen::CertificateParams::new(vec!["pennel-peer.qa".into()])
        .map_err(|_| PeerCompareError::Storage)?;
    params.extended_key_usages = vec![
        rcgen::ExtendedKeyUsagePurpose::ServerAuth,
        rcgen::ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let cert = params
        .self_signed(&key)
        .map_err(|_| PeerCompareError::Storage)?;
    Ok(PeerCompareIdentity {
        identity_id: fingerprint(cert.der().as_ref()),
        certificate_der_b64: B64.encode(cert.der().as_ref()),
        private_key_der_b64: B64.encode(key.serialize_der()),
    })
}

pub(crate) fn fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(crate) fn valid_peer(peer: &str) -> bool {
    peer.len() == 64
        && peer
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl PeerCompareIdentity {
    pub(crate) fn parts(
        &self,
    ) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), PeerCompareError> {
        if self.certificate_der_b64.len() > 8192 || self.private_key_der_b64.len() > 8192 {
            return Err(PeerCompareError::Storage);
        }
        let cert = B64
            .decode(&self.certificate_der_b64)
            .map_err(|_| PeerCompareError::Storage)?;
        let key = B64
            .decode(&self.private_key_der_b64)
            .map_err(|_| PeerCompareError::Storage)?;
        if fingerprint(&cert) != self.identity_id || !valid_peer(&self.identity_id) {
            return Err(PeerCompareError::Storage);
        }
        Ok((
            CertificateDer::from(cert),
            PrivatePkcs8KeyDer::from(key).into(),
        ))
    }
}

pub(crate) fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

#[derive(Debug)]
pub(crate) struct PeerVerifier {
    pub(crate) pins: Arc<Mutex<HashSet<String>>>,
}

impl PeerVerifier {
    fn roots(
        &self,
        cert: &CertificateDer<'_>,
    ) -> Result<Arc<rustls::RootCertStore>, rustls::Error> {
        if !self
            .pins
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .contains(&fingerprint(cert.as_ref()))
        {
            return Err(rustls::Error::General("unapproved QA peer".into()));
        }
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert.clone().into_owned())?;
        Ok(Arc::new(roots))
    }
}

impl ServerCertVerifier for PeerVerifier {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            self.roots(end)?,
            provider(),
        )
        .build()
        .map_err(|_| rustls::Error::General("invalid QA trust anchor".into()))?;
        verifier.verify_server_cert(end, intermediates, name, ocsp, now)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

impl ClientCertVerifier for PeerVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        end: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
            self.roots(end)?,
            provider(),
        )
        .build()
        .map_err(|_| rustls::Error::General("invalid QA trust anchor".into()))?;
        verifier.verify_client_cert(end, intermediates, now)
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        <Self as ServerCertVerifier>::verify_tls12_signature(self, message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        <Self as ServerCertVerifier>::verify_tls13_signature(self, message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        <Self as ServerCertVerifier>::supported_verify_schemes(self)
    }
}
