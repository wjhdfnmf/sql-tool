# Patched copy of mysql_async

This is the `mysql_async` driver from https://github.com/blackbeam/mysql_async
(MIT OR Apache-2.0; see `LICENSE-MIT` and `LICENSE-APACHE`), copied at commit
`f5fe460cc6cc29538077ca94261935ee077d069a` (8 November 2026), with the changes
below. The workspace's `[patch.crates-io]` section points Cargo at it.

The upstream `test/`, `tests/` and CI files are not included.

## Why carry a copy

1. The published 0.37.1 release cannot do "verify the CA but not the host
   name" with rustls: it looks for the mismatch by error text that rustls no
   longer prints. Upstream has fixed this after 0.37.1 (their issue 404), but
   has not published the fix. This copy includes it.
2. SQL Tool needs certificate pinning, which upstream does not offer.

When upstream publishes a release with the fix, change 1 goes away. Changes
2 and 3 stay until upstream offers a way to supply a custom certificate
verifier.

## Changes made here

Every change is marked in the source with `SQL Tool patch`.

1. `src/opts/mod.rs`: `SslOpts` gains `with_pinned_cert_sha256` and
   `pinned_cert_sha256`.
2. `src/io/tls/rustls_io.rs`: when a fingerprint is pinned, the server
   certificate is accepted only if the SHA-256 of its DER encoding equals the
   pin. Chain, dates and names are not consulted. The handshake signature is
   still verified against the certificate's key.
3. `src/conn/mod.rs`: MariaDB "zero-configuration TLS" is switched off. It
   accepts an unverified self-signed certificate and validates it after login.
   SQL Tool's rule is that trust is decided before any credential is sent.
