//! One authenticated connection to a server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use mysql_async::consts::{ColumnFlags, ColumnType};
use mysql_async::prelude::Queryable;
use mysql_async::{Column, Conn, Opts, OptsBuilder, Params, Row, SslOpts, Value};
use secrecy::ExposeSecret;
use serde::Serialize;

use crate::error::Error;
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

pub struct Session {
    conn: Conn,
    opts: Opts,
    info: ServerInfo,
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
                // Known driver defect: with rustls this still checks the host
                // name, so Verify CA currently behaves like Verify Identity.
                TlsMode::VerifyCa => ssl.with_danger_skip_domain_validation(true),
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

async fn status_value(conn: &mut Conn, name: &str) -> Result<Option<String>, Error> {
    let row: Option<(String, Option<String>)> = conn
        .query_first(format!("SHOW SESSION STATUS LIKE '{name}'"))
        .await?;
    Ok(row.and_then(|(_, v)| v).filter(|v| !v.is_empty()))
}

impl Session {
    /// Checks the profile against the security policy, then connects and
    /// authenticates. Nothing touches the network if the policy check fails.
    pub async fn connect(profile: &ConnectProfile) -> Result<Self, Error> {
        policy::check(profile)?;
        let opts = build_opts(profile);
        let mut conn = Conn::new(opts.clone()).await?;

        let row: Option<(Option<String>, Option<String>, Option<String>)> = conn
            .query_first("SELECT @@version, @@version_comment, CURRENT_USER()")
            .await?;
        let (version, comment, user) = row.unwrap_or_default();
        let version_comment = comment.unwrap_or_default();
        let info = ServerInfo {
            version: ServerVersion::parse(&version.unwrap_or_default(), &version_comment),
            version_comment,
            connection_id: conn.id(),
            current_user: user.unwrap_or_default(),
            tls_version: status_value(&mut conn, "Ssl_version").await?,
            tls_cipher: status_value(&mut conn, "Ssl_cipher").await?,
        };
        Ok(Self { conn, opts, info })
    }

    pub fn info(&self) -> &ServerInfo {
        &self.info
    }

    /// Runs a statement and returns its first result set in memory.
    /// For small results only; large ones go through [`Session::run`].
    pub async fn query(&mut self, sql: &str) -> Result<ResultSet, Error> {
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
        let rows: Vec<Row> = self.conn.exec(sql, params).await?;
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
        self.conn.exec_drop(sql, params).await?;
        Ok(self.conn.affected_rows())
    }

    /// Makes the server refuse changes on this session. A second line of
    /// defence behind the client-side check for read-only connections.
    pub async fn set_read_only(&mut self) -> Result<(), Error> {
        self.conn.query_drop("SET SESSION TRANSACTION READ ONLY").await?;
        Ok(())
    }

    /// The default database of this session, if one is selected.
    pub async fn current_database(&mut self) -> Result<Option<String>, Error> {
        let row: Option<Option<String>> = self.conn.query_first("SELECT DATABASE()").await?;
        Ok(row.flatten())
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
    pub async fn run<F>(&mut self, sql: &str, max_rows: Option<u64>, mut on_event: F) -> Result<(), Error>
    where
        F: FnMut(RunEvent),
    {
        // A small first batch gets rows on screen quickly; larger batches
        // after that keep lock traffic on the store low.
        const FIRST_BATCH: usize = 256;
        const BATCH: usize = 4096;

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
                            if at_cap || batch.rows() >= limit {
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
            connection_id: self.info.connection_id,
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
    connection_id: u32,
}

impl Canceller {
    pub async fn cancel(&self) -> Result<(), Error> {
        let mut side = Conn::new(self.opts.clone()).await?;
        side.query_drop(format!("KILL QUERY {}", self.connection_id)).await?;
        side.disconnect().await?;
        Ok(())
    }
}
