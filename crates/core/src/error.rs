//! Errors, sorted into the categories the UI needs to explain to a user.

use mysql_async::{DriverError, IoError};

use crate::policy::PolicyError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Policy(#[from] PolicyError),

    #[error(
        "the server asked for the password in clear text, but cleartext authentication is not \
         enabled for this connection; nothing was sent"
    )]
    CleartextNotEnabled,

    #[error("the server requires the authentication plugin `{0}`, which is not supported yet")]
    UnsupportedAuthPlugin(String),

    #[error("this connection requires TLS but the server does not offer it")]
    ServerHasNoTls,

    #[error("TLS failed: {0}")]
    Tls(String),

    #[error(
        "the server presented a different certificate from the one trusted for this connection; \
         nothing was sent. If the certificate was replaced on purpose, fetch and trust the new one"
    )]
    CertificateChanged,

    #[error("access denied: {message} (error {code})")]
    AccessDenied { code: u16, message: String },

    #[error("server error {code} ({state}): {message}")]
    Server {
        code: u16,
        state: String,
        message: String,
    },

    #[error("network error: {0}")]
    Io(String),

    #[error("could not read or write local settings: {0}")]
    Storage(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// A stable short name for the category, for the UI to branch on.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Policy(_) => "policy",
            Error::CleartextNotEnabled => "cleartext-not-enabled",
            Error::UnsupportedAuthPlugin(_) => "unsupported-auth",
            Error::ServerHasNoTls => "no-tls",
            Error::Tls(_) => "tls",
            Error::CertificateChanged => "certificate-changed",
            Error::AccessDenied { .. } => "access-denied",
            Error::Server { .. } => "server",
            Error::Io(_) => "network",
            Error::Storage(_) => "storage",
            Error::Other(_) => "other",
        }
    }
}

impl From<mysql_async::Error> for Error {
    fn from(e: mysql_async::Error) -> Self {
        match e {
            mysql_async::Error::Driver(DriverError::CleartextPluginDisabled) => Error::CleartextNotEnabled,
            mysql_async::Error::Driver(DriverError::UnknownAuthPlugin { name }) => {
                Error::UnsupportedAuthPlugin(name)
            }
            mysql_async::Error::Driver(DriverError::NoClientSslFlagFromServer) => Error::ServerHasNoTls,
            mysql_async::Error::Io(IoError::Tls(e)) => Error::Tls(e.to_string()),
            // A failed TLS handshake (bad certificate, wrong host name, no
            // shared protocol version) arrives as an I/O error wrapping the
            // rustls error.
            mysql_async::Error::Io(IoError::Io(e))
                if e.get_ref().is_some_and(|inner| inner.is::<rustls::Error>()) =>
            {
                // The pinned-certificate check reports a mismatch this way.
                let pin_mismatch = e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()).is_some_and(|tls| {
                    matches!(
                        tls,
                        rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure)
                    )
                });
                if pin_mismatch {
                    Error::CertificateChanged
                } else {
                    Error::Tls(e.to_string())
                }
            }
            mysql_async::Error::Io(IoError::Io(e)) => Error::Io(e.to_string()),
            mysql_async::Error::Server(e) => match e.code {
                // ER_DBACCESS_DENIED, ER_ACCESS_DENIED, ER_ACCESS_DENIED_NO_PASSWORD
                1044 | 1045 | 1698 => Error::AccessDenied {
                    code: e.code,
                    message: e.message,
                },
                _ => Error::Server {
                    code: e.code,
                    state: e.state,
                    message: e.message,
                },
            },
            other => Error::Other(other.to_string()),
        }
    }
}
