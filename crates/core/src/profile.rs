//! What the user configures for one connection.

use std::path::PathBuf;

use secrecy::SecretString;

/// How the connection is encrypted and how the server is verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsMode {
    /// No encryption.
    Disabled,
    /// Encrypted, but the server certificate is not checked. Stops passive
    /// eavesdropping only; an impersonating server is not detected.
    Required,
    /// Encrypted, and the certificate must chain to a trusted CA.
    VerifyCa,
    /// As `VerifyCa`, and the certificate must also match the host name.
    VerifyIdentity,
    /// Encrypted, and the server must present exactly the certificate the
    /// user chose to trust (`ConnectProfile::pinned_sha256`).
    Pinned,
}

impl TlsMode {
    /// True when an impersonating server would be rejected.
    pub fn verifies_server(self) -> bool {
        matches!(self, TlsMode::VerifyCa | TlsMode::VerifyIdentity | TlsMode::Pinned)
    }
}

/// Where the trusted CA certificate comes from.
#[derive(Clone, Debug)]
pub enum CaSource {
    File(PathBuf),
    Pem(Vec<u8>),
}

#[derive(Clone, Debug)]
pub struct ConnectProfile {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: SecretString,
    pub database: Option<String>,
    pub tls: TlsMode,
    /// CA to trust. When set, the system trust store is not used.
    pub ca: Option<CaSource>,
    /// SHA-256 of the one certificate trusted in `TlsMode::Pinned`.
    pub pinned_sha256: Option<[u8; 32]>,
    /// Allow the `mysql_clear_password` plugin (LDAP and PAM accounts).
    pub allow_cleartext_auth: bool,
    /// Allow cleartext auth even though the transport does not verify the
    /// server. Only for links secured some other way, such as an SSH tunnel.
    pub accept_cleartext_on_unverified_transport: bool,
}

impl ConnectProfile {
    /// A profile with the safe defaults: verified TLS, cleartext auth off.
    pub fn new(host: impl Into<String>, port: u16, user: impl Into<String>, password: SecretString) -> Self {
        Self {
            host: host.into(),
            port,
            user: user.into(),
            password,
            database: None,
            tls: TlsMode::VerifyIdentity,
            ca: None,
            pinned_sha256: None,
            allow_cleartext_auth: false,
            accept_cleartext_on_unverified_transport: false,
        }
    }
}
