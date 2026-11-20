//! Login behaviour against a fake server that imitates MySQL Enterprise
//! with LDAP/PAM accounts (the `mysql_clear_password` plugin).
//!
//! These prove what the client sends and refuses to send. They do not prove
//! compatibility with a real server's TLS stack.

use std::time::{Duration, Instant};

use secrecy::SecretString;
use sqltool_core::{CaSource, ConnectProfile, Error, Flavor, PolicyError, RunEvent, Session, TlsMode};
use sqltool_fakeserver::{generate_identity, Auth, Config, FakeServer, QueryBehaviour, TlsIdentity};

const PASSWORD: &str = "correct horse battery staple";

fn identity() -> TlsIdentity {
    generate_identity(&["127.0.0.1", "localhost"])
}

/// A profile set up the way the app would for an LDAP account: cleartext
/// auth on, TLS verified against the given CA.
fn ldap_profile(server: &FakeServer, ca: &TlsIdentity) -> ConnectProfile {
    let mut p = ConnectProfile::new("127.0.0.1", server.port(), "ann", SecretString::from(PASSWORD));
    p.ca = Some(CaSource::Pem(ca.ca_pem.clone().into_bytes()));
    p.allow_cleartext_auth = true;
    p
}

async fn expect_err(profile: &ConnectProfile) -> Error {
    match Session::connect(profile).await {
        Ok(_) => panic!("connection unexpectedly succeeded"),
        Err(e) => e,
    }
}

async fn expect_tls_err(profile: &ConnectProfile) {
    match expect_err(profile).await {
        Error::Tls(_) => {}
        other => panic!("expected a TLS error, got {other:?}"),
    }
}

#[tokio::test]
async fn mysql_8_4_enterprise_cleartext_login_over_verified_tls() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();

    let session = Session::connect(&ldap_profile(&server, &id)).await.unwrap();
    let info = session.info();
    assert_eq!(info.version.flavor, Flavor::MySql);
    assert_eq!((info.version.major, info.version.minor, info.version.patch), (8, 4, 3));
    assert!(info.version.enterprise);
    assert_eq!(info.current_user, "ann@%");
    assert_eq!(info.tls_version.as_deref(), Some("TLSv1.3"));

    let seen = server.observed();
    assert_eq!(seen.first_plugin.as_deref(), Some("caching_sha2_password"));
    assert_eq!(seen.cleartext_password.as_deref(), Some(PASSWORD));
    assert_eq!(seen.cleartext_over_tls, Some(true));
}

#[tokio::test]
async fn mysql_5_7_enterprise_cleartext_login_over_tls_1_2() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_5_7_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();

    let session = Session::connect(&ldap_profile(&server, &id)).await.unwrap();
    let info = session.info();
    assert_eq!((info.version.major, info.version.minor, info.version.patch), (5, 7, 37));
    assert!(info.version.enterprise);
    assert_eq!(info.tls_version.as_deref(), Some("TLSv1.2"));

    let seen = server.observed();
    assert_eq!(seen.first_plugin.as_deref(), Some("mysql_native_password"));
    assert_eq!(seen.cleartext_password.as_deref(), Some(PASSWORD));
    assert_eq!(seen.cleartext_over_tls, Some(true));
}

#[tokio::test]
async fn password_is_not_sent_when_cleartext_is_not_enabled() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();

    let mut profile = ldap_profile(&server, &id);
    profile.allow_cleartext_auth = false;

    assert!(matches!(expect_err(&profile).await, Error::CleartextNotEnabled));
    assert_eq!(server.observed().cleartext_password, None);
    assert_eq!(server.observed().logins_ok, 0);
}

#[tokio::test]
async fn cleartext_without_tls_is_refused_before_connecting() {
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, None)).await.unwrap();

    let mut profile = ConnectProfile::new("127.0.0.1", server.port(), "ann", SecretString::from(PASSWORD));
    profile.allow_cleartext_auth = true;
    profile.tls = TlsMode::Disabled;

    assert!(matches!(expect_err(&profile).await, Error::Policy(PolicyError::CleartextWithoutTls)));
    assert_eq!(server.observed().connections, 0, "must not even open a socket");
}

#[tokio::test]
async fn cleartext_with_unverified_tls_is_refused_before_connecting() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();

    let mut profile = ldap_profile(&server, &id);
    profile.tls = TlsMode::Required;

    assert!(matches!(expect_err(&profile).await, Error::Policy(PolicyError::CleartextWithUnverifiedTls)));
    assert_eq!(server.observed().connections, 0);
}

#[tokio::test]
async fn explicit_override_allows_cleartext_on_unverified_transport() {
    let server = FakeServer::start(Config::mysql_5_7_enterprise_ldap(PASSWORD, None)).await.unwrap();

    let mut profile = ConnectProfile::new("127.0.0.1", server.port(), "ann", SecretString::from(PASSWORD));
    profile.allow_cleartext_auth = true;
    profile.tls = TlsMode::Disabled;
    profile.accept_cleartext_on_unverified_transport = true;

    Session::connect(&profile).await.unwrap();
    assert_eq!(server.observed().cleartext_over_tls, Some(false));
}

#[tokio::test]
async fn server_signed_by_an_untrusted_ca_gets_no_password() {
    let real = identity();
    let someone_else = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(real))).await.unwrap();

    let profile = ldap_profile(&server, &someone_else);

    expect_tls_err(&profile).await;
    assert_eq!(server.observed().cleartext_password, None);
}

#[tokio::test]
async fn host_name_mismatch_fails_verify_identity() {
    let id = generate_identity(&["db.example.test"]);
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();

    expect_tls_err(&ldap_profile(&server, &id)).await;
    assert_eq!(server.observed().cleartext_password, None);
}

// This was a defect in the published driver (it failed closed). The patched
// copy in vendor/mysql_async fixes it.
#[tokio::test]
async fn verify_ca_accepts_a_host_name_mismatch() {
    let id = generate_identity(&["db.example.test"]);
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();

    let mut profile = ldap_profile(&server, &id);
    profile.tls = TlsMode::VerifyCa;
    Session::connect(&profile).await.unwrap();
}

#[tokio::test]
async fn tls_required_but_server_offers_none_is_refused() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, None)).await.unwrap();

    let profile = ldap_profile(&server, &id);

    assert!(matches!(expect_err(&profile).await, Error::ServerHasNoTls));
    assert_eq!(server.observed().cleartext_password, None);
}

#[tokio::test]
async fn wrong_password_is_reported_as_access_denied() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap("a different password", Some(id.clone()))).await.unwrap();

    match expect_err(&ldap_profile(&server, &id)).await {
        Error::AccessDenied { code, .. } => assert_eq!(code, 1045),
        other => panic!("expected access denied, got {other:?}"),
    }
}

#[tokio::test]
async fn hostile_local_infile_request_uploads_nothing() {
    let secret = std::env::temp_dir().join(format!("sqltool-infile-test-{}.txt", std::process::id()));
    std::fs::write(&secret, b"private file content").unwrap();

    let id = identity();
    let mut cfg = Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()));
    cfg.auth = Auth::AcceptAny;
    cfg.on_query = QueryBehaviour::RequestLocalFile(secret.to_string_lossy().into_owned());
    let server = FakeServer::start(cfg).await.unwrap();

    // The very first query after login triggers the hostile reply.
    let result = Session::connect(&ldap_profile(&server, &id)).await;
    let _ = std::fs::remove_file(&secret);

    assert!(result.is_err(), "a file request must not be treated as a normal reply");
    assert_eq!(server.observed().local_file_bytes, 0);
}

#[tokio::test]
async fn running_query_can_be_cancelled_from_a_second_connection() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();
    let mut session = Session::connect(&ldap_profile(&server, &id)).await.unwrap();

    let canceller = session.canceller();
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        canceller.cancel().await
    });

    let started = Instant::now();
    let result = session.query("SELECT SLEEP(30)").await;
    cancel.await.unwrap().unwrap();

    assert!(started.elapsed() < Duration::from_secs(10), "cancel did not interrupt the query");
    // SLEEP() reports an interruption by returning 1 rather than an error.
    assert_eq!(result.unwrap().rows, vec![vec![Some("1".to_string())]]);

    // The session must still be usable afterwards.
    let again = session.query("SELECT 1").await.unwrap();
    assert_eq!(again.rows, vec![vec![Some("1".to_string())]]);
}

#[tokio::test]
async fn large_result_streams_row_by_row() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();
    let mut session = Session::connect(&ldap_profile(&server, &id)).await.unwrap();

    let mut nulls = 0u64;
    let mut names = Vec::new();
    let stats = session
        .stream(
            "SELECT fake_rows(200000, 8)",
            |columns| names = columns,
            |row| {
                if row.as_ref(4) == Some(&mysql_null()) {
                    nulls += 1;
                }
            },
        )
        .await
        .unwrap();

    assert_eq!(stats.rows, 200_000);
    assert_eq!(names.len(), 8);
    assert_eq!(names[0], "id");
    assert_eq!(nulls, 20_000, "every tenth row has a NULL in column 4");
    assert!(stats.time_to_first_row.unwrap() < stats.total);
}

fn mysql_null() -> mysql_async::Value {
    mysql_async::Value::NULL
}

async fn connected(server: &FakeServer, id: &TlsIdentity) -> Session {
    Session::connect(&ldap_profile(server, id)).await.unwrap()
}

#[tokio::test]
async fn run_stores_a_large_result_readable_by_window() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();
    let mut session = connected(&server, &id).await;

    let mut stores = Vec::new();
    let mut progress = Vec::new();
    let mut done = None;
    session
        .run("SELECT fake_rows(50000, 5)", None, |event| match event {
            RunEvent::ResultSet { store } => stores.push(store),
            RunEvent::Progress { rows } => progress.push(rows),
            RunEvent::ResultDone { rows, truncated } => done = Some((rows, truncated)),
            RunEvent::Done { .. } => panic!("a SELECT must not report a row-less result"),
        })
        .await
        .unwrap();

    assert_eq!(stores.len(), 1);
    assert_eq!(done, Some((50_000, false)));
    assert_eq!(progress.first(), Some(&256), "first batch is small so rows show up quickly");
    assert!(progress.windows(2).all(|w| w[0] < w[1]));

    let status = stores[0].status();
    assert_eq!(status.rows, 50_000);
    assert!(status.done && !status.truncated);
    assert_eq!(status.columns.len(), 5);
    assert_eq!(status.columns[0].name, "id");
    assert_eq!(status.columns[0].kind, sqltool_core::ColumnKind::Number);
    assert_eq!(status.columns[1].kind, sqltool_core::ColumnKind::Text);
    assert_eq!(status.columns[3].kind, sqltool_core::ColumnKind::Date);

    // Row 50,000 (index 49,999): id column holds "50000" as a 5-byte string.
    let window = stores[0].window(49_999, 10);
    assert_eq!(&window[..4], &1u32.to_le_bytes());
    assert_eq!(&window[4..10], b"\x0550000");
    assert_eq!(stores[0].cell(49_999, 0), Some(Some(b"50000".to_vec())));
}

#[tokio::test]
async fn run_stops_at_the_row_limit_and_leaves_the_session_usable() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();
    let mut session = connected(&server, &id).await;

    let mut store = None;
    let mut done = None;
    session
        .run("SELECT fake_rows(2000000, 4)", Some(1000), |event| match event {
            RunEvent::ResultSet { store: s } => store = Some(s),
            RunEvent::ResultDone { rows, truncated } => done = Some((rows, truncated)),
            _ => {}
        })
        .await
        .unwrap();

    assert_eq!(done, Some((1000, true)));
    let status = store.unwrap().status();
    assert_eq!(status.rows, 1000);
    assert!(status.truncated && status.done);
    assert_eq!(status.error, None);

    let again = session.query("SELECT 1").await.unwrap();
    assert_eq!(again.rows, vec![vec![Some("1".to_string())]]);
}

#[tokio::test]
async fn run_reports_statements_without_a_result_set() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();
    let mut session = connected(&server, &id).await;

    let mut events = Vec::new();
    session
        .run("SET @x = 1", None, |event| {
            events.push(match event {
                RunEvent::Done { affected_rows, .. } => format!("done:{affected_rows}"),
                RunEvent::ResultSet { .. } => "set".to_string(),
                RunEvent::Progress { .. } => "progress".to_string(),
                RunEvent::ResultDone { .. } => "result-done".to_string(),
            })
        })
        .await
        .unwrap();
    assert_eq!(events, ["done:0"]);
}

#[tokio::test]
async fn run_surfaces_server_errors_with_their_code() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();
    let mut session = connected(&server, &id).await;

    match session.run("SELECT fake_rows(oops)", None, |_| {}).await {
        Err(Error::Server { code, .. }) => assert_eq!(code, 1064),
        other => panic!("expected a syntax error, got {other:?}"),
    }
    assert!(session.query("SELECT 1").await.is_ok(), "session survives a failed statement");
}

#[tokio::test]
async fn probe_returns_the_certificate_without_sending_credentials() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();

    let info = sqltool_core::probe::probe_certificate("127.0.0.1", server.port()).await.unwrap();
    assert_eq!(info.sha256, sqltool_core::probe::to_hex(&sqltool_core::probe::fingerprint(&id.cert_der)));
    assert!(info.subject.contains("sqltool fake server"), "{info:?}");
    assert!(info.issuer.contains("test CA") && !info.self_signed, "{info:?}");

    let seen = server.observed();
    assert_eq!(seen.username, None, "the probe must not get as far as a login");
    assert_eq!(seen.cleartext_password, None);
    assert_eq!(seen.logins_ok, 0);
}

#[tokio::test]
async fn probe_reports_a_server_without_tls() {
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, None)).await.unwrap();
    assert!(matches!(sqltool_core::probe::probe_certificate("127.0.0.1", server.port()).await, Err(Error::ServerHasNoTls)));
}

fn pinned_profile(server: &FakeServer, pin: Option<[u8; 32]>) -> ConnectProfile {
    let mut p = ConnectProfile::new("127.0.0.1", server.port(), "ann", SecretString::from(PASSWORD));
    p.tls = TlsMode::Pinned;
    p.pinned_sha256 = pin;
    p.allow_cleartext_auth = true;
    p
}

#[tokio::test]
async fn pinned_certificate_allows_cleartext_login_without_any_ca() {
    // A certificate no CA here vouches for, and whose name does not match:
    // exactly the kind of server certificate pinning is for.
    let id = generate_identity(&["db.example.test"]);
    let server = FakeServer::start(Config::mysql_5_7_enterprise_ldap(PASSWORD, Some(id.clone()))).await.unwrap();
    let pin = sqltool_core::probe::fingerprint(&id.cert_der);

    let session = Session::connect(&pinned_profile(&server, Some(pin))).await.unwrap();
    assert_eq!(session.info().tls_version.as_deref(), Some("TLSv1.2"));
    let seen = server.observed();
    assert_eq!(seen.cleartext_password.as_deref(), Some(PASSWORD));
    assert_eq!(seen.cleartext_over_tls, Some(true));
}

#[tokio::test]
async fn a_different_certificate_than_the_pinned_one_gets_no_password() {
    let trusted = generate_identity(&["127.0.0.1"]);
    let impostor = generate_identity(&["127.0.0.1"]);
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(impostor))).await.unwrap();
    let pin = sqltool_core::probe::fingerprint(&trusted.cert_der);

    assert!(matches!(expect_err(&pinned_profile(&server, Some(pin))).await, Error::CertificateChanged));
    assert_eq!(server.observed().cleartext_password, None);
    assert_eq!(server.observed().logins_ok, 0);
}

#[tokio::test]
async fn pinned_mode_without_a_pin_is_refused_before_connecting() {
    let id = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(id))).await.unwrap();
    assert!(matches!(expect_err(&pinned_profile(&server, None)).await, Error::Policy(PolicyError::NothingPinned)));
    assert_eq!(server.observed().connections, 0);
}

#[tokio::test]
async fn connecting_gives_up_when_the_server_never_answers() {
    // Takes the connection and then says nothing, like a host behind a
    // firewall that swallows packets instead of refusing them.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });

    let mut profile = ConnectProfile::new("127.0.0.1", port, "ann", SecretString::from(PASSWORD));
    profile.tls = TlsMode::Disabled;
    profile.connect_timeout = Duration::from_secs(1);
    let started = Instant::now();
    let error = expect_err(&profile).await;
    assert_eq!(error.kind(), "network", "{error}");
    assert!(error.to_string().contains("no answer from the server"), "{error}");
    assert!(error.is_connection_lost());
    assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    silent.abort();
}

#[tokio::test]
async fn a_live_session_is_left_alone_by_the_liveness_check() {
    let ca = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(ca.clone()))).await.unwrap();
    let mut session = Session::connect(&ldap_profile(&server, &ca)).await.unwrap();
    let id = session.info().connection_id;
    // Not idle long enough to be worth a ping.
    assert!(!session.ensure_alive(Duration::from_secs(3600)).await.unwrap());
    // Idle "long enough": pinged, found alive, not reconnected.
    assert!(!session.ensure_alive(Duration::ZERO).await.unwrap());
    assert_eq!(session.info().connection_id, id);
    assert!(session.idle_for() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_query_is_exported_in_full_without_keeping_its_rows() {
    use sqltool_core::export::ExportFormat;

    let ca = identity();
    let server = FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(ca.clone()))).await.unwrap();
    let mut session = Session::connect(&ldap_profile(&server, &ca)).await.unwrap();

    // Far more rows than any row limit would keep.
    let mut out: Vec<u8> = Vec::new();
    let mut reported = Vec::new();
    let rows = session.export("SELECT fake_rows(50000, 3)", ExportFormat::Csv, "", &mut out, |n| reported.push(n)).await.unwrap();
    assert_eq!(rows, 50_000);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.lines().count(), 50_001, "a header line and every row");
    assert_eq!(text.lines().next().unwrap().split(',').count(), 3);
    assert_eq!(reported.last(), Some(&50_000), "the final count is reported");
    assert!(reported.len() >= 2, "progress is reported along the way: {reported:?}");

    // The same rows as INSERT statements, and the session is usable in between.
    let mut sql: Vec<u8> = Vec::new();
    let rows = session.export("SELECT fake_rows(7, 2)", ExportFormat::Sql, "copy", &mut sql, |_| {}).await.unwrap();
    assert_eq!(rows, 7);
    let sql = String::from_utf8(sql).unwrap();
    assert_eq!(sql.lines().count(), 7);
    assert!(sql.lines().all(|line| line.starts_with("INSERT INTO `copy` (") && line.ends_with(");")), "{sql}");
}
