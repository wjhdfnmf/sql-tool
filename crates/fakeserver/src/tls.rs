//! Throwaway certificate authority and server certificate for tests.

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};

/// A server certificate, its private key, and the CA that signed it.
#[derive(Clone, Debug)]
pub struct TlsIdentity {
    /// PEM of the CA certificate. Give this to the client as its trust root.
    pub ca_pem: String,
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
}

/// Generates a fresh CA and a server certificate valid for `names`
/// (DNS names or IP addresses).
pub fn generate_identity(names: &[&str]) -> TlsIdentity {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "sqltool fake server test CA");
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_key = KeyPair::generate().expect("ca key");
    let ca_cert = ca_params.self_signed(&ca_key).expect("ca cert");

    let names: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let mut params = CertificateParams::new(names).expect("server params");
    params
        .distinguished_name
        .push(DnType::CommonName, "sqltool fake server");
    let key = KeyPair::generate().expect("server key");
    let cert = params
        .signed_by(&key, &ca_cert, &ca_key)
        .expect("server cert");

    TlsIdentity {
        ca_pem: ca_cert.pem(),
        cert_der: cert.der().to_vec(),
        key_der: key.serialize_der(),
    }
}
