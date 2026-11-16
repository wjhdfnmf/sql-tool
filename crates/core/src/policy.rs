//! Rules checked before any network connection is opened.

use crate::profile::{ConnectProfile, TlsMode};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error(
        "cleartext authentication is enabled but TLS is off, so the password would cross the \
         network unencrypted; turn TLS on, or explicitly accept the risk for this connection"
    )]
    CleartextWithoutTls,
    #[error(
        "cleartext authentication is enabled but TLS does not verify the server certificate, so \
         an impersonating server could capture the password; use Verify CA or Verify Identity, \
         or explicitly accept the risk for this connection"
    )]
    CleartextWithUnverifiedTls,
    #[error("no certificate has been trusted for this connection yet; fetch the server's certificate and trust it first")]
    NothingPinned,
}

/// Cleartext authentication hands the real password to whoever is on the
/// other end, so it is only allowed when that end has been verified.
pub fn check(profile: &ConnectProfile) -> Result<(), PolicyError> {
    if profile.tls == TlsMode::Pinned && profile.pinned_sha256.is_none() {
        return Err(PolicyError::NothingPinned);
    }
    if !profile.allow_cleartext_auth
        || profile.tls.verifies_server()
        || profile.accept_cleartext_on_unverified_transport
    {
        return Ok(());
    }
    match profile.tls {
        TlsMode::Disabled => Err(PolicyError::CleartextWithoutTls),
        _ => Err(PolicyError::CleartextWithUnverifiedTls),
    }
}
