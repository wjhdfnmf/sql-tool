//! Fetching a server's TLS certificate so the user can decide to trust it.
//!
//! This speaks just enough of the MySQL protocol to start TLS: read the
//! server's greeting, ask to switch to TLS, complete the TLS handshake, and
//! hang up. No user name, password or query is ever sent.
//!
//! The certificate is accepted whatever it is, because the point is to show
//! it to the user. Nothing is trusted as a result of a probe; trust comes
//! from the user pinning the fingerprint, which later connections enforce.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use x509_parser::prelude::{FromDer, X509Certificate};

use crate::error::Error;

const CLIENT_PROTOCOL_41: u32 = 0x0000_0200;
const CLIENT_SSL: u32 = 0x0000_0800;
const CLIENT_SECURE_CONNECTION: u32 = 0x0000_8000;
const CLIENT_PLUGIN_AUTH: u32 = 0x0008_0000;
const TIMEOUT: Duration = Duration::from_secs(15);

/// What a server presented, for the user to look at before trusting it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateInfo {
    /// SHA-256 of the certificate, as 64 hex digits.
    pub sha256: String,
    pub subject: String,
    pub issuer: String,
    pub not_after: String,
    /// The certificate names itself as its own issuer.
    pub self_signed: bool,
}

pub fn fingerprint(der: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(der);
    hasher.finalize().into()
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses 64 hex digits, ignoring colons and spaces.
pub fn parse_fingerprint(text: &str) -> Option<[u8; 32]> {
    let digits: Vec<u8> = text.bytes().filter(|b| !matches!(b, b':' | b' ')).collect();
    if digits.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in digits.chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// Accepts any certificate and keeps a copy. Used only by the probe.
#[derive(Debug)]
struct Capture {
    seen: Mutex<Option<Vec<u8>>>,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Capture {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.seen.lock().unwrap() = Some(end_entity.as_ref().to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn describe(der: &[u8]) -> CertificateInfo {
    let sha256 = to_hex(&fingerprint(der));
    match X509Certificate::from_der(der) {
        Ok((_, cert)) => CertificateInfo {
            sha256,
            subject: cert.subject().to_string(),
            issuer: cert.issuer().to_string(),
            not_after: cert.validity().not_after.to_string(),
            self_signed: cert.subject() == cert.issuer(),
        },
        Err(_) => CertificateInfo {
            sha256,
            subject: "(could not be read)".into(),
            issuer: "(could not be read)".into(),
            not_after: String::new(),
            self_signed: false,
        },
    }
}

fn io(e: std::io::Error) -> Error {
    Error::Io(e.to_string())
}

async fn probe(host: &str, port: u16) -> Result<CertificateInfo, Error> {
    let mut tcp = TcpStream::connect((host, port)).await.map_err(io)?;

    // The greeting: a 4-byte header, then the handshake packet.
    let mut header = [0u8; 4];
    tcp.read_exact(&mut header).await.map_err(io)?;
    let len = u32::from_le_bytes([header[0], header[1], header[2], 0]) as usize;
    let mut greeting = vec![0u8; len];
    tcp.read_exact(&mut greeting).await.map_err(io)?;
    if greeting.first() == Some(&0xFF) {
        // The server refused the connection outright (for example, host not allowed).
        let message = String::from_utf8_lossy(greeting.get(3..).unwrap_or(&[])).into_owned();
        return Err(Error::Other(format!("The server refused the connection: {message}")));
    }
    // Protocol byte, version string, connection id (4), scramble (8), filler
    // (1), then the lower 16 capability bits.
    let version_end = greeting.iter().skip(1).position(|b| *b == 0).map(|p| p + 1);
    let caps_at = version_end.map(|end| end + 1 + 4 + 8 + 1);
    let lower = caps_at
        .and_then(|at| greeting.get(at..at + 2))
        .map(|b| u16::from_le_bytes([b[0], b[1]]) as u32)
        .ok_or_else(|| Error::Other("This does not look like a MySQL or MariaDB server.".into()))?;
    if lower & CLIENT_SSL == 0 {
        return Err(Error::ServerHasNoTls);
    }

    // SSLRequest: capabilities, max packet size, character set, 23 zero bytes.
    let mut request = Vec::with_capacity(36);
    request.extend_from_slice(&[32, 0, 0, 1]);
    request.extend_from_slice(&(CLIENT_PROTOCOL_41 | CLIENT_SSL | CLIENT_SECURE_CONNECTION | CLIENT_PLUGIN_AUTH).to_le_bytes());
    request.extend_from_slice(&(16u32 * 1024 * 1024).to_le_bytes());
    request.push(45); // utf8mb4
    request.extend_from_slice(&[0u8; 23]);
    tcp.write_all(&request).await.map_err(io)?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let capture = Arc::new(Capture { seen: Mutex::new(None), provider: provider.clone() });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(capture.clone())
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string()).map_err(|e| Error::Tls(e.to_string()))?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp).await.map_err(|e| Error::Tls(e.to_string()))?;
    drop(tls);

    let der = capture.seen.lock().unwrap().take();
    der.map(|der| describe(&der)).ok_or_else(|| Error::Tls("The server did not present a certificate.".into()))
}

/// Connects to `host:port`, starts TLS, and returns the certificate the
/// server presented. Sends no credentials.
pub async fn probe_certificate(host: &str, port: u16) -> Result<CertificateInfo, Error> {
    tokio::time::timeout(TIMEOUT, probe(host, port)).await.unwrap_or_else(|_| Err(Error::Io("Timed out waiting for the server.".into())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_round_trip_through_hex() {
        let print = fingerprint(b"some certificate bytes");
        let hex = to_hex(&print);
        assert_eq!(hex.len(), 64);
        assert_eq!(parse_fingerprint(&hex), Some(print));
        let with_colons: String = hex.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(":");
        assert_eq!(parse_fingerprint(&with_colons.to_uppercase()), Some(print));
    }

    #[test]
    fn malformed_fingerprints_are_rejected() {
        assert_eq!(parse_fingerprint(""), None);
        assert_eq!(parse_fingerprint("abcd"), None);
        assert_eq!(parse_fingerprint(&"zz".repeat(32)), None);
        assert_eq!(parse_fingerprint(&"ab".repeat(33)), None);
    }
}
