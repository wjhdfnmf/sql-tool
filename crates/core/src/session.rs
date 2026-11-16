//! One authenticated connection to a server.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::Queryable;
use mysql_async::{Column, Conn, Opts, OptsBuilder, Params, Row, SslOpts, Value};
use secrecy::ExposeSecret;
use serde::Serialize;

use crate::error::Error;
use crate::export::{ExportFormat, RowWriter};
use crate::policy;
use crate::profile::{CaSource, ConnectProfile, TlsMode};
use crate::store::{ColumnInfo, ColumnKind, ColumnSource, ResultStore, RowBatch};
use crate::version::ServerVersion;

/// MySQL's error code for a statement stopped by `KILL QUERY`.
const ER_QUERY_INTERRUPTED: u16 = 1317;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub version: ServerVersion,
    pub version_comment: String,
    pub connection_id: u32,
    pub current_user: String,
    /// TLS protocol version as the server reports it; `None` when unencrypted.
    pub tls_version: Option<String>,
    pub tls_cipher: Option<String>,
}

/// A small result set held fully in memory, values as text.
#[derive(Clone, Debug, Default)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StreamStats {
    pub rows: u64,
    pub bytes: u64,
    pub time_to_first_row: Option<Duration>,
    pub total: Duration,
}

/// What happened while running one statement, reported as it happens.
pub enum RunEvent {
    /// A result set has started. Rows appear in `store` as they arrive.
    ResultSet { store: Arc<ResultStore> },
    /// More rows are available in the current result set.
    Progress { rows: u64 },
    /// The current result set is complete.
    ResultDone { rows: u64, truncated: bool },
    /// A statement without a result set finished (INSERT, UPDATE, DDL, ...).
    Done {
        affected_rows: u64,
        last_insert_id: Option<u64>,
        info: String,
        warnings: u16,
    },
}

/// How often the operating system probes a connection that has gone quiet.
/// Without this, a connection cut off by the network (no reply at all, as
/// opposed to a refusal) would leave a query waiting for ever.
const TCP_KEEPALIVE: Duration = Duration::from_secs(30);
/// How long a liveness check waits for the server's answer.
const PING_TIMEOUT: Duration = Duration::from_secs(5);

pub struct Session {
    conn: Conn,
    opts: Opts,
    info: ServerInfo,
    connect_timeout: Duration,
    /// The server's id for this connection, shared with the handles that
    /// cancel it. It changes when the session reconnects.
    connection_id: Arc<AtomicU32>,
    /// Whether the session was put in read-only mode, to do so again after a reconnect.
    read_only: bool,
    /// The default database as last seen, to select it again after a reconnect.
    database: Option<String>,
    last_used: Instant,
    /// The last statement ended with the connection lost.
    broken: bool,
    /// A second connection used only to interrupt this one. Opened the first
    /// time it is needed and then kept, so repeated cancels and row-limit
    /// stops do not each log in again.
    control: Arc<tokio::sync::Mutex<Option<Conn>>>,
}

fn build_opts(profile: &ConnectProfile) -> Opts {
    let ssl = match profile.tls {
        TlsMode::Disabled => None,
        mode => {
            let mut ssl = SslOpts::default();
            if let Some(ca) = &profile.ca {
                let root = match ca {
                    CaSource::File(path) => path.clone().into(),
                    CaSource::Pem(pem) => pem.clone().into(),
                };
                ssl = ssl.with_root_certs(vec![root]).with_disable_built_in_roots(true);
            }
            Some(match mode {
                TlsMode::Required => ssl.with_danger_accept_invalid_certs(true),
                TlsMode::VerifyCa => ssl.with_danger_skip_domain_validation(true),
                // The policy check has already refused a pinned mode without a pin.
                TlsMode::Pinned => ssl.with_pinned_cert_sha256(profile.pinned_sha256),
                _ => ssl,
            })
        }
    };

    OptsBuilder::default()
        .ip_or_hostname(profile.host.clone())
        .tcp_port(profile.port)
        .user(Some(profile.user.clone()))
        .pass(Some(profile.password.expose_secret().to_string()))
        .db_name(profile.database.clone())
        .ssl_opts(ssl)
        .enable_cleartext_plugin(profile.allow_cleartext_auth)
        // The driver would otherwise quietly reconnect over a local socket
        // when the host is loopback, which sidesteps the TLS settings above.
        .prefer_socket(false)
        .tcp_nodelay(true)
        .tcp_keepalive(Some(TCP_KEEPALIVE))
        .connect_attribute("program_name", "sqltool")
        .into()
}

pub(crate) fn text(v: Value) -> Option<String> {
    match v {
        Value::NULL => None,
        Value::Bytes(b) => Some(String::from_utf8_lossy(&b).into_owned()),
        other => Some(other.as_sql(true).trim_matches('\'').to_string()),
    }
}

fn column_info(column: &Column) -> ColumnInfo {
    use ColumnType::*;
    const BINARY_CHARSET: u16 = 63;
    let kind = match column.column_type() {
        MYSQL_TYPE_DECIMAL | MYSQL_TYPE_NEWDECIMAL | MYSQL_TYPE_TINY | MYSQL_TYPE_SHORT | MYSQL_TYPE_INT24
        | MYSQL_TYPE_LONG | MYSQL_TYPE_LONGLONG | MYSQL_TYPE_FLOAT | MYSQL_TYPE_DOUBLE | MYSQL_TYPE_YEAR => {
            ColumnKind::Number
        }
        MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 | MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE | MYSQL_TYPE_TIME
        | MYSQL_TYPE_TIME2 | MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2 => ColumnKind::Date,
        MYSQL_TYPE_JSON => ColumnKind::Json,
        MYSQL_TYPE_BIT | MYSQL_TYPE_GEOMETRY | MYSQL_TYPE_VECTOR => ColumnKind::Binary,
        _ if column.character_set() == BINARY_CHARSET => ColumnKind::Binary,
        _ => ColumnKind::Text,
    };
    // The server names the table column behind a result column, unless the
    // column is computed. That is what makes a result editable.
    let (schema, table, source_column) = (column.schema_str(), column.org_table_str(), column.org_name_str());
    let source = (!schema.is_empty() && !table.is_empty() && !source_column.is_empty()).then(|| ColumnSource {
        schema: schema.into_owned(),
        table: table.into_owned(),
        column: source_column.into_owned(),
    });
    ColumnInfo {
        name: column.name_str().into_owned(),
        kind,
        primary_key: source.is_some() && column.flags().contains(ColumnFlags::PRI_KEY_FLAG),
        source,
    }
}

/// The TLS version and cipher of this connection, in one round trip.
async fn tls_status(conn: &mut Conn) -> Result<(Option<String>, Option<String>), Error> {
    let rows: Vec<(String, Option<String>)> = conn
        .query("SHOW SESSION STATUS WHERE Variable_name IN ('Ssl_version', 'Ssl_cipher')")
        .await?;
    let find = |name: &str| {
        rows.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    Ok((find("Ssl_version"), find("Ssl_cipher")))
}

/// Connects, giving up after `timeout`. The time covers reaching the server,
/// TLS and logging in.
async fn open(opts: &Opts, timeout: Duration) -> Result<Conn, Error> {
    match tokio::time::timeout(timeout, Conn::new(opts.clone())).await {
        Ok(conn) => Ok(conn?),
        Err(_) => Err(Error::Io(format!(
            "no answer from the server after {} seconds",
            timeout.as_secs().max(1)
        ))),
    }
}

/// Connects and asks the server what it is.
async fn open_and_identify(opts: &Opts, timeout: Duration) -> Result<(Conn, ServerInfo), Error> {
    let mut conn = open(opts, timeout).await?;
    let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
        .query_first("SELECT @@version, @@version_comment, CURRENT_USER()")
        .await?;
    let (version, comment, user) = row.unwrap_or_default();
    let version_comment = comment.unwrap_or_default();
    let (tls_version, tls_cipher) = tls_status(&mut conn).await?;
    let info = ServerInfo {
        version: ServerVersion::parse(&version.unwrap_or_default(), &version_comment),
        version_comment,
        connection_id: conn.id(),
        current_user: user.unwrap_or_default(),
        tls_version,
        tls_cipher,
    };
    Ok((conn, info))
}

impl Session {
    /// Checks the profile against the security policy, then connects and
    /// authenticates. Nothing touches the network if the policy check fails.
    pub async fn connect(profile: &ConnectProfile) -> Result<Self, Error> {
        policy::check(profile)?;
        let opts = build_opts(profile);
        let (conn, info) = open_and_identify(&opts, profile.connect_timeout).await?;
        Ok(Self {
            conn,
            opts,
            connect_timeout: profile.connect_timeout,
            connection_id: Arc::new(AtomicU32::new(info.connection_id)),
            read_only: false,
            database: profile.database.clone(),
            last_used: Instant::now(),
            broken: false,
            info,
            control: Arc::new(tokio::sync::Mutex::new(None)),
        })
    }

    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    /// Notes that the session was just used and, if that ended with the
    /// connection lost, that it must be reopened before its next use.
    fn observe<T>(&mut self, outcome: Result<T, Error>) -> Result<T, Error> {
        self.last_used = Instant::now();
        if outcome.as_ref().is_err_and(Error::is_connection_lost) {
            self.broken = true;
        }
        outcome
    }

    /// How long since the session last talked to the server.
    pub fn idle_for(&self) -> Duration {
        self.last_used.elapsed()
    }

    /// Asks the server for a sign of life. Also counts as activity, which is
    /// what stops the server from closing an idle connection.
    pub async fn ping(&mut self) -> Result<(), Error> {
        let outcome = match tokio::time::timeout(PING_TIMEOUT, self.conn.ping()).await {
            Ok(result) => result.map_err(Error::from),
            Err(_) => Err(Error::Io("the server did not answer".into())),
        };
        self.observe(outcome)
    }

    /// Opens a new connection in place of this one, with the same login,
    /// TLS settings and policy. Read-only mode and the default database are
    /// restored. Everything else the old connection held is gone: session
    /// variables, temporary tables, an open transaction.
    pub async fn reconnect(&mut self) -> Result<(), Error> {
        let (conn, info) = open_and_identify(&self.opts, self.connect_timeout).await?;
        self.conn = conn;
        self.connection_id.store(info.connection_id, Ordering::Relaxed);
        self.info = info;
        self.broken = false;
        self.last_used = Instant::now();
        // The connection kept for cancelling may have gone the same way.
        *self.control.lock().await = None;
        if self.read_only {
            self.conn.query_drop("SET SESSION TRANSACTION READ ONLY").await?;
        }
        if let Some(database) = self.database.clone() {
            // Best effort: the database may have been dropped meanwhile.
            let _ = self
                .conn
                .query_drop(format!("USE {}", crate::split::quote_identifier(&database)))
                .await;
        }
        Ok(())
    }

    /// Makes sure the connection is usable before something is sent on it.
    ///
    /// A session that has been quiet for `idle` or longer is checked with a
    /// ping, and reopened if the server has dropped it (servers close idle
    /// connections after their `wait_timeout`). Returns true if it had to
    /// reconnect. Nothing has been sent at this point, so reconnecting here
    /// can never run a statement twice.
    pub async fn ensure_alive(&mut self, idle: Duration) -> Result<bool, Error> {
        if !self.broken {
            if self.last_used.elapsed() < idle || self.ping().await.is_ok() {
                return Ok(false);
            }
        }
        self.reconnect().await?;
        Ok(true)
    }

    /// Runs a statement and returns its first result set in memory.
    /// For small results only; large ones go through [`Session::run`].
    pub async fn query(&mut self, sql: &str) -> Result<ResultSet, Error> {
        let outcome = self.query_inner(sql).await;
        self.observe(outcome)
    }

    async fn query_inner(&mut self, sql: &str) -> Result<ResultSet, Error> {
        let mut result = self.conn.query_iter(sql).await?;
        let columns = result
            .columns_ref()
            .iter()
            .map(|c| c.name_str().into_owned())
            .collect();
        let mut rows = Vec::new();
        while let Some(row) = result.next().await? {
            rows.push(row.unwrap().into_iter().map(text).collect());
        }
        result.drop_result().await?;
        Ok(ResultSet { columns, rows })
    }

    /// Runs a parameterised statement (values are sent separately from the
    /// SQL, never spliced into it) and returns all rows as text.
    pub(crate) async fn query_params<P>(&mut self, sql: &str, params: P) -> Result<Vec<Vec<Option<String>>>, Error>
    where
        P: Into<Params> + Send,
    {
        let outcome = self.conn.exec(sql, params).await.map_err(Error::from);
        let rows: Vec<Row> = self.observe(outcome)?;
        Ok(rows
            .into_iter()
            .map(|row| row.unwrap().into_iter().map(text).collect())
            .collect())
    }

    /// Runs a parameterised statement that returns no rows; gives back the
    /// number of rows it affected.
    pub(crate) async fn exec_params(&mut self, sql: &str, params: Vec<Option<Vec<u8>>>) -> Result<u64, Error> {
        let values: Vec<Value> = params
            .into_iter()
            .map(|p| p.map_or(Value::NULL, Value::Bytes))
            .collect();
        let params = if values.is_empty() { Params::Empty } else { Params::Positional(values) };
        let outcome = self.conn.exec_drop(sql, params).await.map_err(Error::from);
        self.observe(outcome)?;
        Ok(self.conn.affected_rows())
    }

    /// Makes the server refuse changes on this session. A second line of
    /// defence behind the client-side check for read-only connections.
    pub async fn set_read_only(&mut self) -> Result<(), Error> {
        self.conn.query_drop("SET SESSION TRANSACTION READ ONLY").await?;
        self.read_only = true;
        Ok(())
    }

    /// The default database as this session last knew it, without asking the server.
    pub fn database(&self) -> Option<&str> {
        self.database.as_deref()
    }

    /// Makes `name` the default database of this session.
    pub async fn use_database(&mut self, name: &str) -> Result<(), Error> {
        let outcome = self
            .conn
            .query_drop(format!("USE {}", crate::split::quote_identifier(name)))
            .await
            .map_err(Error::from);
        self.observe(outcome)?;
        self.database = Some(name.to_string());
        Ok(())
    }

    /// The default database of this session, if one is selected.
    pub async fn current_database(&mut self) -> Result<Option<String>, Error> {
        let outcome = self.conn.query_first("SELECT DATABASE()").await.map_err(Error::from);
        let row: Option<Option<String>> = self.observe(outcome)?;
        self.database = row.flatten();
        Ok(self.database.clone())
    }

    /// Runs a statement and hands each row to `on_row` as it arrives,
    /// without holding the result in memory. `on_columns` is called once
    /// with the column names, before the first row.
    pub async fn stream<C, F>(&mut self, sql: &str, on_columns: C, mut on_row: F) -> Result<StreamStats, Error>
    where
        C: FnOnce(Vec<String>),
        F: FnMut(Row),
    {
        let started = Instant::now();
        let mut stats = StreamStats::default();
        let mut result = self.conn.query_iter(sql).await?;
        on_columns(
            result
                .columns_ref()
                .iter()
                .map(|c| c.name_str().into_owned())
                .collect(),
        );
        while let Some(row) = result.next().await? {
            if stats.rows == 0 {
                stats.time_to_first_row = Some(started.elapsed());
            }
            stats.rows += 1;
            for i in 0..row.len() {
                if let Some(Value::Bytes(b)) = row.as_ref(i) {
                    stats.bytes += b.len() as u64;
                }
            }
            on_row(row);
        }
        result.drop_result().await?;
        stats.total = started.elapsed();
        Ok(stats)
    }

    /// Runs one statement and reports what it produces through `on_event`.
    ///
    /// Result sets are written into [`ResultStore`]s as rows arrive, so a UI
    /// can show the first rows while the rest are still coming in. A
    /// statement can produce several result sets (a stored procedure call).
    ///
    /// `max_rows` caps each result set. When the cap is reached the statement
    /// is interrupted on the server and the result is marked truncated.
    pub async fn run<F>(&mut self, sql: &str, max_rows: Option<u64>, on_event: F) -> Result<(), Error>
    where
        F: FnMut(RunEvent),
    {
        let outcome = self.run_inner(sql, max_rows, on_event).await;
        self.observe(outcome)
    }

    async fn run_inner<F>(&mut self, sql: &str, max_rows: Option<u64>, mut on_event: F) -> Result<(), Error>
    where
        F: FnMut(RunEvent),
    {
        // A small first batch gets rows on screen quickly; larger batches
        // after that keep lock traffic on the store low. A batch is also
        // handed over once it holds this many bytes, so rows with very large
        // values cannot pile up in memory before reaching the store.
        const FIRST_BATCH: usize = 256;
        const BATCH: usize = 4096;
        const BATCH_BYTES: usize = 8 * 1024 * 1024;

        let canceller = self.canceller();
        let mut kill: Option<tokio::task::JoinHandle<Result<(), Error>>> = None;
        let mut outcome: Result<(), Error> = Ok(());

        let mut result = self.conn.query_iter(sql).await?;

        'sets: loop {
            let columns: Vec<ColumnInfo> = result.columns_ref().iter().map(column_info).collect();
            if columns.is_empty() {
                on_event(RunEvent::Done {
                    affected_rows: result.affected_rows(),
                    last_insert_id: result.last_insert_id(),
                    info: result.info().into_owned(),
                    warnings: result.warnings(),
                });
                // Consumes this (row-less) result and moves to the next one.
                if let Err(e) = result.next().await {
                    outcome = Err(e.into());
                    break 'sets;
                }
            } else {
                let store = Arc::new(ResultStore::new());
                store.set_columns(columns);
                on_event(RunEvent::ResultSet { store: store.clone() });

                let mut batch = RowBatch::default();
                let mut stored = 0u64;
                let mut truncated = false;
                let set_outcome = loop {
                    match result.next().await {
                        Ok(Some(_)) if truncated => {} // discard until the server stops
                        Ok(Some(row)) => {
                            for value in row.unwrap() {
                                match value {
                                    Value::NULL => batch.push_cell(None),
                                    Value::Bytes(bytes) => batch.push_cell(Some(&bytes)),
                                    other => batch.push_cell(Some(other.as_sql(true).as_bytes())),
                                }
                            }
                            batch.end_row();
                            let pending = batch.rows() as u64;
                            let at_cap = max_rows.is_some_and(|max| stored + pending >= max);
                            let limit = if stored == 0 { FIRST_BATCH } else { BATCH };
                            if at_cap || batch.rows() >= limit || batch.bytes() >= BATCH_BYTES {
                                stored += pending;
                                store.append(&mut batch);
                                on_event(RunEvent::Progress { rows: stored });
                            }
                            if at_cap {
                                truncated = true;
                                store.set_truncated();
                                let canceller = canceller.clone();
                                kill = Some(tokio::spawn(async move { canceller.cancel().await }));
                            }
                        }
                        Ok(None) => break Ok(()),
                        // The interruption we asked for is not a failure.
                        Err(mysql_async::Error::Server(e)) if truncated && e.code == ER_QUERY_INTERRUPTED => {
                            break Ok(())
                        }
                        Err(e) => break Err(Error::from(e)),
                    }
                };
                stored += batch.rows() as u64;
                store.append(&mut batch);
                store.finish(set_outcome.as_ref().err().map(|e| e.to_string()));
                on_event(RunEvent::ResultDone { rows: stored, truncated });
                if let Err(e) = set_outcome {
                    outcome = Err(e);
                    break 'sets;
                }
            }
            if result.is_empty() {
                break;
            }
        }
        // Finish reading whatever is left so the connection is ready for
        // the next statement and late errors surface here.
        if let Err(e) = result.drop_result().await {
            if outcome.is_ok() && kill.is_none() {
                outcome = Err(e.into());
            }
        }

        if let Some(kill) = kill {
            // Wait for the KILL to be delivered, then make sure a late one
            // cannot interrupt whatever the user runs next.
            let _ = kill.await;
            self.absorb_stray_kill().await;
        }
        outcome
    }

    /// Runs a statement and writes the rows of its result straight to `out`
    /// in `format`, keeping none of them. This is how a result of any size,
    /// and regardless of the row limit, is exported in full.
    ///
    /// `on_progress` is told the number of rows written, now and then.
    /// Returns the number of rows written. If the statement or the writing
    /// fails part-way, `out` holds an incomplete file.
    pub async fn export<W, P>(
        &mut self,
        sql: &str,
        format: ExportFormat,
        table: &str,
        out: &mut W,
        on_progress: P,
    ) -> Result<u64, Error>
    where
        W: std::io::Write + Send,
        P: FnMut(u64) + Send,
    {
        let outcome = self.export_inner(sql, format, table, out, on_progress).await;
        self.observe(outcome)
    }

    async fn export_inner<W, P>(
        &mut self,
        sql: &str,
        format: ExportFormat,
        table: &str,
        out: &mut W,
        mut on_progress: P,
    ) -> Result<u64, Error>
    where
        W: std::io::Write + Send,
        P: FnMut(u64) + Send,
    {
        const PROGRESS_EVERY: u64 = 20_000;
        let storage = |e: std::io::Error| Error::Storage(format!("could not write the file: {e}"));

        let mut result = self.conn.query_iter(sql).await?;
        let columns: Vec<ColumnInfo> = result.columns_ref().iter().map(column_info).collect();
        if columns.is_empty() {
            result.drop_result().await?;
            return Err(Error::Other("that statement returns no rows to export".into()));
        }
        let mut writer = RowWriter::new(format, columns, table);
        let mut outcome = writer.begin(out).map_err(storage);
        let mut rows = 0u64;
        while outcome.is_ok() {
            match result.next().await {
                Ok(Some(row)) => {
                    let cells: Vec<Option<Vec<u8>>> = row
                        .unwrap()
                        .into_iter()
                        .map(|value| match value {
                            Value::NULL => None,
                            Value::Bytes(bytes) => Some(bytes),
                            other => Some(other.as_sql(true).into_bytes()),
                        })
                        .collect();
                    outcome = writer.row(out, cells.iter().map(|cell| cell.as_deref())).map_err(storage);
                    rows += 1;
                    if rows % PROGRESS_EVERY == 0 {
                        on_progress(rows);
                    }
                }
                Ok(None) => break,
                Err(e) => outcome = Err(Error::from(e)),
            }
        }
        // Read off whatever is left, so the connection is usable afterwards.
        let drained = result.drop_result().await;
        outcome?;
        drained?;
        let rows = writer.end(out).map_err(storage)?;
        on_progress(rows);
        Ok(rows)
    }

    /// `KILL QUERY` can arrive just after the statement it was aimed at has
    /// finished, in which case some servers apply it to the next statement.
    /// Running a no-op here takes that hit instead of the user's next query.
    pub async fn absorb_stray_kill(&mut self) {
        let _ = self.conn.query_drop("DO 0").await;
    }

    /// A handle that can interrupt whatever this session is running.
    pub fn canceller(&self) -> Canceller {
        Canceller {
            opts: self.opts.clone(),
            connect_timeout: self.connect_timeout,
            connection_id: self.connection_id.clone(),
            control: self.control.clone(),
        }
    }

    pub async fn close(self) -> Result<(), Error> {
        self.conn.disconnect().await?;
        Ok(())
    }
}

/// Interrupts a running query from a second connection, which is the only
/// way the MySQL protocol allows.
#[derive(Clone)]
pub struct Canceller {
    opts: Opts,
    connect_timeout: Duration,
    /// Read when cancelling, so the handle stays right after a reconnect.
    connection_id: Arc<AtomicU32>,
    control: Arc<tokio::sync::Mutex<Option<Conn>>>,
}

impl Canceller {
    pub async fn cancel(&self) -> Result<(), Error> {
        let kill = format!("KILL QUERY {}", self.connection_id.load(Ordering::Relaxed));
        let mut control = self.control.lock().await;
        if let Some(conn) = control.as_mut() {
            if conn.query_drop(&kill).await.is_ok() {
                return Ok(());
            }
            // The kept connection has gone stale (timed out, most likely).
            *control = None;
        }
        let mut conn = open(&self.opts, self.connect_timeout).await?;
        conn.query_drop(&kill).await?;
        *control = Some(conn);
        Ok(())
    }
}
