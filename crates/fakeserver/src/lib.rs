//! A minimal fake MySQL server for testing client behaviour.
//!
//! It speaks just enough of the MySQL wire protocol to exercise the things a
//! real server would do during login, and records what the client sent so
//! tests can assert on it. It is hand-written on purpose: it shares no code
//! with the client driver, so it cannot share the driver's bugs.
//!
//! It is not a database. Queries get canned answers.

pub mod tls;

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub use tls::{generate_identity, TlsIdentity};

// Capability flags (subset).
const CLIENT_LONG_PASSWORD: u32 = 0x0000_0001;
const CLIENT_LONG_FLAG: u32 = 0x0000_0004;
const CLIENT_CONNECT_WITH_DB: u32 = 0x0000_0008;
const CLIENT_PROTOCOL_41: u32 = 0x0000_0200;
const CLIENT_SSL: u32 = 0x0000_0800;
const CLIENT_TRANSACTIONS: u32 = 0x0000_2000;
const CLIENT_SECURE_CONNECTION: u32 = 0x0000_8000;
const CLIENT_MULTI_RESULTS: u32 = 0x0002_0000;
const CLIENT_PLUGIN_AUTH: u32 = 0x0008_0000;
const CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA: u32 = 0x0020_0000;

const SERVER_STATUS_AUTOCOMMIT: u16 = 0x0002;

// Column types.
const TYPE_LONGLONG: u8 = 8;
const TYPE_DATETIME: u8 = 12;
const TYPE_NEWDECIMAL: u8 = 246;
const TYPE_VAR_STRING: u8 = 253;

/// Fixed 20-byte scramble. A test double has no need for randomness.
const SCRAMBLE: &[u8; 20] = b"abcdefghijklmnopqrst";

/// How the fake server answers a login.
#[derive(Clone, Debug)]
pub enum Auth {
    /// Accept whatever the client sends first.
    AcceptAny,
    /// Ask the client to switch to `mysql_clear_password`, the way a real
    /// server does for an LDAP or PAM account, then compare the password.
    Cleartext { password: String },
}

/// How the fake server answers queries.
#[derive(Clone, Debug)]
pub enum QueryBehaviour {
    /// Canned answers.
    Normal,
    /// Hostile: answer every query with a `LOCAL INFILE` request for this
    /// path, as a malicious server would to read files off the client.
    RequestLocalFile(String),
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Version string sent in the handshake.
    pub server_version: String,
    /// Value returned for `@@version_comment`.
    pub version_comment: String,
    /// Auth plugin named in the initial handshake.
    pub handshake_plugin: String,
    pub auth: Auth,
    /// `None` means the server does not offer TLS at all.
    pub tls: Option<TlsIdentity>,
    /// Offer TLS 1.2 only, as MySQL 5.7 does.
    pub tls12_only: bool,
    pub on_query: QueryBehaviour,
}

impl Config {
    /// Looks like MySQL Enterprise 8.4 with an LDAP-authenticated account.
    pub fn mysql_8_4_enterprise_ldap(password: &str, tls: Option<TlsIdentity>) -> Self {
        Self {
            server_version: "8.4.3-commercial".into(),
            version_comment: "MySQL Enterprise Server - Commercial".into(),
            handshake_plugin: "caching_sha2_password".into(),
            auth: Auth::Cleartext {
                password: password.into(),
            },
            tls,
            tls12_only: false,
            on_query: QueryBehaviour::Normal,
        }
    }

    /// Looks like MySQL Enterprise 5.7 with an LDAP-authenticated account.
    pub fn mysql_5_7_enterprise_ldap(password: &str, tls: Option<TlsIdentity>) -> Self {
        Self {
            server_version: "5.7.37-enterprise-commercial-advanced-log".into(),
            version_comment: "MySQL Enterprise Server - Advanced Edition (Commercial)".into(),
            handshake_plugin: "mysql_native_password".into(),
            auth: Auth::Cleartext {
                password: password.into(),
            },
            tls,
            tls12_only: true,
            on_query: QueryBehaviour::Normal,
        }
    }
}

/// What the server saw. Aggregated over all connections.
#[derive(Clone, Debug, Default)]
pub struct Observed {
    pub connections: usize,
    pub logins_ok: usize,
    pub username: Option<String>,
    /// Plugin the client named in its first response.
    pub first_plugin: Option<String>,
    /// TLS protocol version of the most recent TLS connection.
    pub tls_version: Option<String>,
    /// The password, if the client sent it in clear.
    pub cleartext_password: Option<String>,
    /// Whether that password arrived inside TLS.
    pub cleartext_over_tls: Option<bool>,
    pub queries: Vec<String>,
    /// Bytes of local file content the client uploaded.
    pub local_file_bytes: usize,
}

struct Shared {
    cfg: Config,
    observed: Mutex<Observed>,
    kill: Mutex<HashMap<u32, Arc<Notify>>>,
    acceptor: Option<TlsAcceptor>,
}

/// A running fake server. Stops when dropped.
pub struct FakeServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl FakeServer {
    pub async fn start(cfg: Config) -> io::Result<Self> {
        let acceptor = match &cfg.tls {
            Some(id) => Some(build_acceptor(id, cfg.tls12_only)?),
            None => None,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            cfg,
            observed: Mutex::new(Observed::default()),
            kill: Mutex::new(HashMap::new()),
            acceptor,
        });
        let accept_shared = shared.clone();
        let task = tokio::spawn(async move {
            let mut next_id: u32 = 100;
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    break;
                };
                let _ = tcp.set_nodelay(true);
                next_id += 1;
                let shared = accept_shared.clone();
                let id = next_id;
                tokio::spawn(async move {
                    // Clients hanging up mid-handshake is expected in the
                    // refusal tests, so connection errors are not reported.
                    let _ = handle_connection(shared.clone(), tcp, id).await;
                    shared.kill.lock().unwrap().remove(&id);
                });
            }
        });
        Ok(Self { addr, shared, task })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    pub fn observed(&self) -> Observed {
        self.shared.observed.lock().unwrap().clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn build_acceptor(id: &TlsIdentity, tls12_only: bool) -> io::Result<TlsAcceptor> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let versions: &[&rustls::SupportedProtocolVersion] = if tls12_only {
        &[&rustls::version::TLS12]
    } else {
        &[&rustls::version::TLS13, &rustls::version::TLS12]
    };
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(id.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_der.clone())),
        )
        .map_err(io::Error::other)?;
    Ok(TlsAcceptor::from(Arc::new(cfg)))
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn read_packet<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 4];
    r.read_exact(&mut head).await?;
    let len = u32::from_le_bytes([head[0], head[1], head[2], 0]) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await?;
    Ok((head[3], payload))
}

async fn write_packet<W: AsyncWrite + Unpin>(w: &mut W, seq: u8, payload: &[u8]) -> io::Result<()> {
    debug_assert!(payload.len() < 0x00FF_FFFF, "fake server never splits packets");
    let len = (payload.len() as u32).to_le_bytes();
    w.write_all(&[len[0], len[1], len[2], seq]).await?;
    w.write_all(payload).await
}

fn put_lenenc_int(buf: &mut Vec<u8>, n: u64) {
    match n {
        0..=250 => buf.push(n as u8),
        251..=0xFFFF => {
            buf.push(0xFC);
            buf.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x1_0000..=0xFF_FFFF => {
            buf.push(0xFD);
            buf.extend_from_slice(&(n as u32).to_le_bytes()[..3]);
        }
        _ => {
            buf.push(0xFE);
            buf.extend_from_slice(&n.to_le_bytes());
        }
    }
}

fn put_lenenc_str(buf: &mut Vec<u8>, s: &[u8]) {
    put_lenenc_int(buf, s.len() as u64);
    buf.extend_from_slice(s);
}

fn ok_packet() -> Vec<u8> {
    let mut p = vec![0x00, 0x00, 0x00];
    p.extend_from_slice(&SERVER_STATUS_AUTOCOMMIT.to_le_bytes());
    p.extend_from_slice(&0u16.to_le_bytes());
    p
}

fn eof_packet() -> Vec<u8> {
    let mut p = vec![0xFE];
    p.extend_from_slice(&0u16.to_le_bytes());
    p.extend_from_slice(&SERVER_STATUS_AUTOCOMMIT.to_le_bytes());
    p
}

fn err_packet(code: u16, state: &str, msg: &str) -> Vec<u8> {
    let mut p = vec![0xFF];
    p.extend_from_slice(&code.to_le_bytes());
    p.push(b'#');
    p.extend_from_slice(state.as_bytes());
    p.extend_from_slice(msg.as_bytes());
    p
}

fn handshake_packet(cfg: &Config, conn_id: u32, caps: u32) -> Vec<u8> {
    let mut p = vec![10u8];
    p.extend_from_slice(cfg.server_version.as_bytes());
    p.push(0);
    p.extend_from_slice(&conn_id.to_le_bytes());
    p.extend_from_slice(&SCRAMBLE[..8]);
    p.push(0);
    p.extend_from_slice(&((caps & 0xFFFF) as u16).to_le_bytes());
    p.push(45); // utf8mb4_general_ci
    p.extend_from_slice(&SERVER_STATUS_AUTOCOMMIT.to_le_bytes());
    p.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
    p.push(21);
    p.extend_from_slice(&[0u8; 10]);
    p.extend_from_slice(&SCRAMBLE[8..]);
    p.push(0);
    p.extend_from_slice(cfg.handshake_plugin.as_bytes());
    p.push(0);
    p
}

struct ClientHello {
    user: String,
    plugin: Option<String>,
}

fn take_cstr<'a>(p: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let rest = p.get(*pos..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    *pos += end + 1;
    Some(&rest[..end])
}

fn parse_client_hello(p: &[u8]) -> Option<ClientHello> {
    let caps = u32::from_le_bytes(p.get(0..4)?.try_into().ok()?);
    let mut pos = 32;
    let user = String::from_utf8_lossy(take_cstr(p, &mut pos)?).into_owned();
    // Skip the first auth response; the fake server never checks scrambles.
    if caps & CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA != 0 || caps & CLIENT_SECURE_CONNECTION != 0 {
        // Responses here are always shorter than 251 bytes, so the
        // length-encoded form is a single byte as well.
        let len = *p.get(pos)? as usize;
        pos += 1 + len;
    } else {
        take_cstr(p, &mut pos)?;
    }
    if caps & CLIENT_CONNECT_WITH_DB != 0 {
        take_cstr(p, &mut pos)?;
    }
    let plugin = if caps & CLIENT_PLUGIN_AUTH != 0 {
        take_cstr(p, &mut pos).map(|s| String::from_utf8_lossy(s).into_owned())
    } else {
        None
    };
    Some(ClientHello { user, plugin })
}

fn bad_data(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

async fn handle_connection(shared: Arc<Shared>, mut tcp: TcpStream, conn_id: u32) -> io::Result<()> {
    let cfg = &shared.cfg;
    shared.observed.lock().unwrap().connections += 1;

    let mut caps = CLIENT_LONG_PASSWORD
        | CLIENT_LONG_FLAG
        | CLIENT_CONNECT_WITH_DB
        | CLIENT_PROTOCOL_41
        | CLIENT_TRANSACTIONS
        | CLIENT_SECURE_CONNECTION
        | CLIENT_MULTI_RESULTS
        | CLIENT_PLUGIN_AUTH
        | CLIENT_PLUGIN_AUTH_LENENC_CLIENT_DATA;
    if shared.acceptor.is_some() {
        caps |= CLIENT_SSL;
    }

    write_packet(&mut tcp, 0, &handshake_packet(cfg, conn_id, caps)).await?;
    tcp.flush().await?;

    let (seq, first) = read_packet(&mut tcp).await?;
    let client_caps = u32::from_le_bytes(
        first
            .get(0..4)
            .ok_or_else(|| bad_data("short first packet"))?
            .try_into()
            .unwrap(),
    );

    // A 32-byte first packet with CLIENT_SSL set is an SSLRequest: the client
    // wants to upgrade before sending anything else.
    let wants_tls = first.len() == 32 && client_caps & CLIENT_SSL != 0;
    let (mut io, mut seq, hello_bytes, over_tls): (Box<dyn Io>, u8, Vec<u8>, bool) =
        match (&shared.acceptor, wants_tls) {
            (Some(acceptor), true) => {
                let tls = acceptor.accept(tcp).await?;
                let version = tls.get_ref().1.protocol_version().map(|v| format!("{v:?}"));
                shared.observed.lock().unwrap().tls_version = version;
                let mut io: Box<dyn Io> = Box::new(tls);
                let (seq, bytes) = read_packet(&mut io).await?;
                (io, seq, bytes, true)
            }
            _ => (Box::new(tcp), seq, first, false),
        };

    let hello = parse_client_hello(&hello_bytes).ok_or_else(|| bad_data("bad handshake response"))?;
    {
        let mut o = shared.observed.lock().unwrap();
        o.username = Some(hello.user.clone());
        o.first_plugin = hello.plugin.clone();
    }

    match &cfg.auth {
        Auth::AcceptAny => {}
        Auth::Cleartext { password } => {
            seq = seq.wrapping_add(1);
            let mut switch = vec![0xFE];
            switch.extend_from_slice(b"mysql_clear_password\0");
            write_packet(&mut io, seq, &switch).await?;
            io.flush().await?;

            let (resp_seq, mut resp) = read_packet(&mut io).await?;
            seq = resp_seq;
            if resp.last() == Some(&0) {
                resp.pop();
            }
            let got = String::from_utf8_lossy(&resp).into_owned();
            {
                let mut o = shared.observed.lock().unwrap();
                o.cleartext_password = Some(got.clone());
                o.cleartext_over_tls = Some(over_tls);
            }
            if &got != password {
                let msg = format!("Access denied for user '{}'@'localhost'", hello.user);
                write_packet(&mut io, seq.wrapping_add(1), &err_packet(1045, "28000", &msg)).await?;
                io.flush().await?;
                return Ok(());
            }
        }
    }
    write_packet(&mut io, seq.wrapping_add(1), &ok_packet()).await?;
    io.flush().await?;
    shared.observed.lock().unwrap().logins_ok += 1;

    let killed = Arc::new(Notify::new());
    shared.kill.lock().unwrap().insert(conn_id, killed.clone());

    command_loop(&shared, io, &hello.user, killed).await
}

enum Reply {
    Ok,
    Err(u16, &'static str, String),
    Rows {
        columns: Vec<String>,
        rows: Vec<Vec<Option<String>>>,
    },
    Generated {
        rows: u64,
        columns: usize,
    },
    Sleep(f64),
    LocalInfile(String),
}

async fn command_loop(
    shared: &Arc<Shared>,
    io: Box<dyn Io>,
    user: &str,
    killed: Arc<Notify>,
) -> io::Result<()> {
    let mut io = BufWriter::with_capacity(256 * 1024, io);
    loop {
        let (_, packet) = read_packet(&mut io).await?;
        let Some((&cmd, body)) = packet.split_first() else {
            return Err(bad_data("empty command"));
        };
        match cmd {
            0x01 => return Ok(()),                                         // COM_QUIT
            0x0E | 0x1F => write_packet(&mut io, 1, &ok_packet()).await?,  // PING, RESET_CONNECTION
            0x03 => {
                let sql = String::from_utf8_lossy(body).into_owned();
                shared.observed.lock().unwrap().queries.push(sql.clone());
                let mut reply = answer(shared, &sql, user);
                if let Reply::Sleep(secs) = reply {
                    // As on a real server, SLEEP() returns 0 when it runs to
                    // completion and 1 when KILL QUERY interrupts it.
                    let interrupted = tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs_f64(secs)) => "0",
                        _ = killed.notified() => "1",
                    };
                    reply = Reply::Rows {
                        columns: vec!["SLEEP".into()],
                        rows: vec![vec![Some(interrupted.into())]],
                    };
                }
                write_reply(shared, &mut io, reply).await?;
            }
            _ => {
                let p = err_packet(1047, "08S01", "Unknown command");
                write_packet(&mut io, 1, &p).await?;
            }
        }
        io.flush().await?;
    }
}

fn answer(shared: &Arc<Shared>, sql: &str, user: &str) -> Reply {
    if let QueryBehaviour::RequestLocalFile(path) = &shared.cfg.on_query {
        return Reply::LocalInfile(path.clone());
    }
    let trimmed = sql.trim().trim_end_matches(';').trim();
    let lower = trimmed.to_ascii_lowercase();

    if let Some(rest) = lower.strip_prefix("kill query ") {
        return match rest.trim().parse::<u32>() {
            Ok(id) => match shared.kill.lock().unwrap().get(&id) {
                Some(n) => {
                    n.notify_one();
                    Reply::Ok
                }
                None => Reply::Err(1094, "HY000", format!("Unknown thread id: {id}")),
            },
            Err(_) => Reply::Err(1064, "42000", "bad KILL".into()),
        };
    }

    if lower.starts_with("show session status where variable_name in") {
        let version = shared
            .observed
            .lock()
            .unwrap()
            .tls_version
            .clone()
            .map(|v| v.replace("TLSv1_", "TLSv1."))
            .unwrap_or_default();
        return Reply::Rows {
            columns: vec!["Variable_name".into(), "Value".into()],
            rows: vec![
                vec![Some("Ssl_cipher".into()), Some(String::new())],
                vec![Some("Ssl_version".into()), Some(version)],
            ],
        };
    }

    if let Some(rest) = lower.strip_prefix("show session status like ") {
        let name = rest.trim().trim_matches('\'');
        let value = match name {
            "ssl_version" => shared
                .observed
                .lock()
                .unwrap()
                .tls_version
                .clone()
                .map(|v| v.replace("TLSv1_", "TLSv1."))
                .unwrap_or_default(),
            "ssl_cipher" => String::new(),
            _ => String::new(),
        };
        return Reply::Rows {
            columns: vec!["Variable_name".into(), "Value".into()],
            rows: vec![vec![Some(name.to_string()), Some(value)]],
        };
    }

    let Some(list) = lower.strip_prefix("select") else {
        return Reply::Ok;
    };
    let list = list.trim();

    // SELECT fake_rows(<rows>, <columns>) streams a synthetic result set.
    if let Some(args) = list.strip_prefix("fake_rows(").and_then(|s| s.strip_suffix(')')) {
        let mut parts = args.split(',').map(|s| s.trim().parse::<u64>());
        if let (Some(Ok(rows)), Some(Ok(columns))) = (parts.next(), parts.next()) {
            return Reply::Generated {
                rows,
                columns: columns.clamp(1, 200) as usize,
            };
        }
        return Reply::Err(1064, "42000", "usage: SELECT fake_rows(rows, columns)".into());
    }
    if let Some(arg) = list.strip_prefix("sleep(").and_then(|s| s.strip_suffix(')')) {
        if let Ok(secs) = arg.trim().parse::<f64>() {
            return Reply::Sleep(secs);
        }
    }

    let mut columns = Vec::new();
    let mut row = Vec::new();
    for item in list.split(',') {
        let item = item.trim();
        columns.push(item.to_string());
        row.push(match item {
            "@@max_allowed_packet" => Some("67108864".to_string()),
            "@@wait_timeout" => Some("28800".to_string()),
            "@@socket" => None,
            "@@version" | "version()" => Some(shared.cfg.server_version.clone()),
            "@@version_comment" => Some(shared.cfg.version_comment.clone()),
            "current_user()" => Some(format!("{user}@%")),
            "connection_id()" => None,
            other => other.parse::<i64>().ok().map(|n| n.to_string()),
        });
    }
    Reply::Rows {
        columns,
        rows: vec![row],
    }
}

fn column_def(name: &str, col_type: u8, charset: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(64);
    put_lenenc_str(&mut p, b"def");
    put_lenenc_str(&mut p, b"");
    put_lenenc_str(&mut p, b"");
    put_lenenc_str(&mut p, b"");
    put_lenenc_str(&mut p, name.as_bytes());
    put_lenenc_str(&mut p, name.as_bytes());
    p.push(0x0C);
    p.extend_from_slice(&charset.to_le_bytes());
    p.extend_from_slice(&1024u32.to_le_bytes());
    p.push(col_type);
    p.extend_from_slice(&0u16.to_le_bytes());
    p.push(0);
    p.extend_from_slice(&[0, 0]);
    p
}

// The last `send!` in each branch bumps `seq` without a later read.
#[allow(unused_assignments)]
async fn write_reply<W: AsyncWrite + AsyncRead + Unpin>(
    shared: &Arc<Shared>,
    io: &mut W,
    reply: Reply,
) -> io::Result<()> {
    let mut seq: u8 = 1;
    macro_rules! send {
        ($payload:expr) => {{
            write_packet(io, seq, $payload).await?;
            seq = seq.wrapping_add(1);
        }};
    }
    match reply {
        Reply::Ok => write_packet(io, 1, &ok_packet()).await,
        Reply::Err(code, state, msg) => write_packet(io, 1, &err_packet(code, state, &msg)).await,
        Reply::Sleep(_) => unreachable!("resolved by the caller"),
        Reply::Rows { columns, rows } => {
            let mut count = Vec::new();
            put_lenenc_int(&mut count, columns.len() as u64);
            send!(&count);
            for c in &columns {
                send!(&column_def(c, TYPE_VAR_STRING, 45));
            }
            send!(&eof_packet());
            for row in rows {
                let mut buf = Vec::new();
                for v in row {
                    match v {
                        Some(v) => put_lenenc_str(&mut buf, v.as_bytes()),
                        None => buf.push(0xFB),
                    }
                }
                send!(&buf);
            }
            send!(&eof_packet());
            Ok(())
        }
        Reply::Generated { rows, columns } => {
            let mut count = Vec::new();
            put_lenenc_int(&mut count, columns as u64);
            send!(&count);
            for c in 0..columns {
                let (name, ty, charset) = match c {
                    0 => ("id".to_string(), TYPE_LONGLONG, 63),
                    c if c % 4 == 1 => (format!("name_{c}"), TYPE_VAR_STRING, 45),
                    c if c % 4 == 2 => (format!("amount_{c}"), TYPE_NEWDECIMAL, 63),
                    c if c % 4 == 3 => (format!("created_{c}"), TYPE_DATETIME, 63),
                    c => (format!("note_{c}"), TYPE_VAR_STRING, 45),
                };
                send!(&column_def(&name, ty, charset));
            }
            send!(&eof_packet());

            let mut buf = Vec::with_capacity(columns * 24);
            for r in 0..rows {
                use std::io::Write;
                buf.clear();
                let mut cell = [0u8; 48];
                for c in 0..columns {
                    let mut cur = io::Cursor::new(&mut cell[..]);
                    match c {
                        0 => write!(cur, "{}", r + 1)?,
                        c if c % 4 == 1 => write!(cur, "customer {} / {}", r % 9973, c)?,
                        c if c % 4 == 2 => write!(cur, "{}.{:02}", (r * 37 + c as u64) % 100_000, r % 100)?,
                        c if c % 4 == 3 => write!(
                            cur,
                            "2026-{:02}-{:02} {:02}:{:02}:{:02}",
                            r % 12 + 1,
                            r % 28 + 1,
                            r % 24,
                            r % 60,
                            (r / 7) % 60
                        )?,
                        // Every tenth note is NULL so NULL handling is exercised.
                        _ if r % 10 == 0 => {
                            buf.push(0xFB);
                            continue;
                        }
                        _ => write!(cur, "note for row {}", r)?,
                    }
                    let n = cur.position() as usize;
                    put_lenenc_str(&mut buf, &cell[..n]);
                }
                send!(&buf);
            }
            write_packet(io, seq, &eof_packet()).await
        }
        Reply::LocalInfile(path) => {
            let mut p = vec![0xFB];
            p.extend_from_slice(path.as_bytes());
            write_packet(io, 1, &p).await?;
            io.flush().await?;
            // The client answers with file content, then an empty packet.
            let mut last_seq;
            loop {
                let (s, chunk) = read_packet(io).await?;
                last_seq = s;
                if chunk.is_empty() {
                    break;
                }
                shared.observed.lock().unwrap().local_file_bytes += chunk.len();
            }
            write_packet(io, last_seq.wrapping_add(1), &ok_packet()).await
        }
    }
}
