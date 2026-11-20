//! Desktop shell: a thin command layer between the web UI and the core.
//!
//! Rules this file keeps:
//! - No business logic. Commands translate arguments, call the core, and
//!   translate results.
//! - The UI can hand a password in. No command hands one back out.
//! - Result rows cross to the UI as raw bytes, a window at a time.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sqltool_core::chart::{self, ChartData, ChartRequest, ColumnSummary};
use sqltool_core::edit::{self, EditTarget, RowEdit};
use sqltool_core::export::{self, CellRange, CopyFormat, ExportFormat, RangeStats};
use sqltool_core::guard::{self, Concern};
use sqltool_core::metadata::{Charset, ColumnEntry, ForeignKeyEntry, SchemaDefaults, SchemaObjects, TableDetails};
use sqltool_core::slices;
use sqltool_core::table::{self, IndexSpec, TableSpec};
use sqltool_core::{
    ddl, split, ApprovedFiles, Canceller, ColumnInfo, ConnectProfile, Error, History, HistoryEntry, ObjectKind, ProfileStore,
    ResultStore, RunEvent, SavedProfile, ServerInfo, Session, Settings, SettingsStore, TlsMode,
};
use tauri::ipc::{Channel, Response};
use tauri::{Manager, State};

/// Largest cell value sent to the UI in one piece by `get_cell`.
const MAX_CELL_BYTES: usize = 2 * 1024 * 1024;
/// Largest script file the editor will open.
const MAX_SCRIPT_BYTES: u64 = 20 * 1024 * 1024;
/// Largest picture of a chart that will be written to a file.
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
/// A connection that has been quiet this long is checked before it is used
/// again, and reopened if the server has dropped it meanwhile.
const IDLE_CHECK: Duration = Duration::from_secs(30);

// ---------- errors ----------

#[derive(Debug, Serialize)]
struct UiError {
    /// Stable category the UI can branch on.
    kind: String,
    message: String,
}

fn ui_error(kind: &str, message: impl Into<String>) -> UiError {
    UiError {
        kind: kind.into(),
        message: message.into(),
    }
}

impl From<Error> for UiError {
    fn from(e: Error) -> Self {
        ui_error(e.kind(), e.to_string())
    }
}

// ---------- state ----------

/// One editor tab's own connection, so tabs do not block each other and
/// each keeps its own session state (default database, variables, ...).
struct Tab {
    session: tokio::sync::Mutex<Session>,
    canceller: Canceller,
    cancel_requested: AtomicBool,
}

/// One open connection to a server, as the user sees it.
struct Workspace {
    profile: ConnectProfile,
    name: String,
    /// Refuse anything that is not a read.
    read_only: bool,
    /// The saved connection this is, to read its script folder from and to
    /// record its default schema in. `None` for the demo server.
    profile_id: Option<String>,
    /// The default schema of the connection as the user sees it: one for all
    /// its tabs, although each tab has a session of its own. A tab adopts it
    /// when its session is opened and again before each run.
    default_schema: Mutex<Option<String>>,
    /// Used for the schema browser, so it stays responsive while tabs run.
    utility: tokio::sync::Mutex<Session>,
    tabs: tokio::sync::Mutex<HashMap<String, Arc<Tab>>>,
}

impl Workspace {
    /// The schema browser's connection, checked for life first: servers
    /// close connections that sit idle, and the browser's often does.
    async fn utility(&self) -> Result<tokio::sync::MutexGuard<'_, Session>, Error> {
        let mut session = self.utility.lock().await;
        session.ensure_alive(IDLE_CHECK).await?;
        Ok(session)
    }

    fn default_schema(&self) -> Option<String> {
        self.default_schema.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    fn set_default_schema(&self, schema: Option<String>) {
        *self.default_schema.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = schema;
    }

    /// Selects the connection's default schema in a tab's session, if it is
    /// not already selected there. A schema that has since been dropped is
    /// left unselected; the statement that needs it will say so.
    async fn adopt_default_schema(&self, session: &mut Session) {
        if let Some(schema) = self.default_schema() {
            if session.database() != Some(schema.as_str()) {
                let _ = session.use_database(&schema).await;
            }
        }
    }

    async fn tab(&self, tab_id: &str) -> Result<Arc<Tab>, Error> {
        if let Some(tab) = self.tabs.lock().await.get(tab_id) {
            return Ok(tab.clone());
        }
        // Connecting can take seconds. The lock is not held meanwhile, so
        // other tabs can still be found, for one to cancel what they run.
        let mut session = Session::connect(&self.profile).await?;
        if self.read_only {
            session.set_read_only().await?;
        }
        self.adopt_default_schema(&mut session).await;
        let tab = Arc::new(Tab {
            canceller: session.canceller(),
            session: tokio::sync::Mutex::new(session),
            cancel_requested: AtomicBool::new(false),
        });
        Ok(self.tabs.lock().await.entry(tab_id.to_string()).or_insert(tab).clone())
    }
}

/// Runs work that can take a while (sorting, copying, writing a file) on a
/// thread meant for that, so the window and other commands stay responsive.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, UiError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| ui_error("other", e.to_string()))
}

#[cfg(feature = "devtools")]
struct Demo {
    server: sqltool_fakeserver::FakeServer,
    ca_pem: String,
}

struct AppState {
    profiles: ProfileStore,
    history: History,
    settings_store: SettingsStore,
    settings: RwLock<Settings>,
    workspaces: Mutex<HashMap<u64, Arc<Workspace>>>,
    results: Mutex<HashMap<u64, Arc<ResultStore>>>,
    /// Results that have a chart open, with the columns read out of them for
    /// it. Dropped with the result, or when the chart is closed.
    charts: Mutex<HashMap<u64, Arc<chart::Dataset>>>,
    /// Script files the user picked in a native dialog, in this run or an
    /// earlier one. The UI can only save to one of these, never to a path of
    /// its own choosing. Remembered between runs so that a query tab
    /// restored after a restart can be saved back to its file.
    approved_files: ApprovedFiles,
    next_id: AtomicU64,
    #[cfg(feature = "devtools")]
    demo: tokio::sync::OnceCell<Demo>,
}

impl AppState {
    fn new(data_dir: PathBuf) -> Self {
        let settings_store = SettingsStore::new(data_dir.join("settings.json"));
        Self {
            profiles: ProfileStore::new(data_dir.join("profiles.json")),
            history: History::new(data_dir.join("history.jsonl")),
            settings: RwLock::new(settings_store.load()),
            settings_store,
            workspaces: Mutex::new(HashMap::new()),
            results: Mutex::new(HashMap::new()),
            charts: Mutex::new(HashMap::new()),
            approved_files: ApprovedFiles::load(data_dir.join("scripts.json")),
            next_id: AtomicU64::new(1),
            #[cfg(feature = "devtools")]
            demo: tokio::sync::OnceCell::new(),
        }
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    fn workspace(&self, id: u64) -> Result<Arc<Workspace>, UiError> {
        self.workspaces
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| ui_error("not-connected", "This connection is no longer open."))
    }

    fn result(&self, id: u64) -> Result<Arc<ResultStore>, UiError> {
        self.results
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| ui_error("no-result", "This result is no longer available."))
    }

    /// The chart data kept for a result, started on first use.
    fn chart(&self, result_id: u64) -> Result<Arc<chart::Dataset>, UiError> {
        let store = self.result(result_id)?;
        let mut charts = self.charts.lock().unwrap();
        Ok(charts.entry(result_id).or_insert_with(|| Arc::new(chart::Dataset::new(store))).clone())
    }
}

// ---------- profiles ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileDto {
    #[serde(flatten)]
    profile: SavedProfile,
    /// Whether the keychain holds a password for this profile.
    has_password: bool,
}

fn profile_dto(profile: SavedProfile) -> ProfileDto {
    let has_password =
        profile.save_password && matches!(sqltool_secrets::load(&profile.id), Ok(Some(_)));
    ProfileDto { profile, has_password }
}

#[tauri::command(async)]
fn profiles_list(state: State<'_, AppState>) -> Result<Vec<ProfileDto>, UiError> {
    Ok(state.profiles.load()?.into_iter().map(profile_dto).collect())
}

/// Saves the profile. `password` is stored in the keychain when the profile
/// asks for that; `None` leaves an already stored password alone.
#[tauri::command(async)]
fn profile_save(
    state: State<'_, AppState>,
    profile: SavedProfile,
    password: Option<String>,
) -> Result<ProfileDto, UiError> {
    let saved = state.profiles.upsert(profile)?;
    if saved.save_password {
        if let Some(password) = password {
            sqltool_secrets::save(&saved.id, &SecretString::from(password))
                .map_err(|e| ui_error("keychain", format!("Could not store the password in the keychain: {e}")))?;
        }
    } else {
        sqltool_secrets::delete(&saved.id).map_err(|e| ui_error("keychain", e))?;
    }
    Ok(profile_dto(saved))
}

#[tauri::command(async)]
fn profile_delete(state: State<'_, AppState>, id: String) -> Result<(), UiError> {
    state.profiles.delete(&id)?;
    sqltool_secrets::delete(&id).map_err(|e| ui_error("keychain", e))
}

/// The password to use: the one just typed, else the one in the keychain.
fn resolve_password(profile: &SavedProfile, given: Option<String>) -> Result<SecretString, UiError> {
    if let Some(password) = given {
        return Ok(password.into());
    }
    if !profile.id.is_empty() && profile.save_password {
        if let Some(stored) = sqltool_secrets::load(&profile.id).map_err(|e| ui_error("keychain", e))? {
            return Ok(stored);
        }
    }
    Err(ui_error("password-required", "A password is needed for this connection."))
}

// ---------- connecting ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceDto {
    id: u64,
    /// The saved connection this came from; `None` for the demo server.
    /// The UI keys what it remembers per connection on this.
    profile_id: Option<String>,
    name: String,
    color: Option<String>,
    host: String,
    user: String,
    info: ServerInfo,
    /// `verified`, `pinned`, `unverified` or `none`.
    security: &'static str,
    database: Option<String>,
    read_only: bool,
    /// Ask before risky statements.
    production: bool,
}

async fn open_workspace(
    state: &AppState,
    profile: ConnectProfile,
    profile_id: Option<String>,
    name: String,
    color: Option<String>,
    read_only: bool,
    production: bool,
) -> Result<WorkspaceDto, UiError> {
    let mut profile = profile;
    let mut session = match Session::connect(&profile).await {
        // The connection's default schema has been dropped or is no longer
        // allowed. That must not keep the connection from opening: open it
        // with no default schema, and the user can pick another.
        Err(Error::Server { code: 1049, .. }) | Err(Error::AccessDenied { code: 1044, .. }) if profile.database.is_some() => {
            profile.database = None;
            Session::connect(&profile).await?
        }
        other => other?,
    };
    if read_only {
        session.set_read_only().await?;
    }
    let info = session.info().clone();
    let database = session.current_database().await.ok().flatten();
    let security = match profile.tls {
        TlsMode::Disabled => "none",
        TlsMode::Required => "unverified",
        TlsMode::VerifyCa | TlsMode::VerifyIdentity => "verified",
        TlsMode::Pinned => "pinned",
    };
    let dto = WorkspaceDto {
        id: state.id(),
        profile_id: profile_id.clone(),
        name: name.clone(),
        color,
        host: format!("{}:{}", profile.host, profile.port),
        user: profile.user.clone(),
        info,
        security,
        database,
        read_only,
        production,
    };
    state.workspaces.lock().unwrap().insert(
        dto.id,
        Arc::new(Workspace {
            profile,
            name,
            read_only,
            profile_id,
            default_schema: Mutex::new(dto.database.clone()),
            utility: tokio::sync::Mutex::new(session),
            tabs: tokio::sync::Mutex::new(HashMap::new()),
        }),
    );
    Ok(dto)
}

/// Fetches the certificate a server presents, so the user can decide to
/// trust it. Sends no credentials.
#[tauri::command]
async fn certificate_probe(host: String, port: u16) -> Result<sqltool_core::probe::CertificateInfo, UiError> {
    Ok(sqltool_core::probe::probe_certificate(host.trim(), port).await?)
}

/// Connects with the given settings and disconnects again.
#[tauri::command]
async fn connection_test(
    state: State<'_, AppState>,
    profile: SavedProfile,
    password: Option<String>,
) -> Result<ServerInfo, UiError> {
    let password = resolve_password(&profile, password)?;
    let mut connect = profile.to_connect_profile(password);
    connect.connect_timeout = state.settings().connect_timeout();
    let session = Session::connect(&connect).await?;
    let info = session.info().clone();
    let _ = session.close().await;
    Ok(info)
}

#[tauri::command]
async fn connect(
    state: State<'_, AppState>,
    profile_id: String,
    password: Option<String>,
) -> Result<WorkspaceDto, UiError> {
    let saved = state
        .profiles
        .get(&profile_id)?
        .ok_or_else(|| ui_error("no-profile", "This connection no longer exists."))?;
    let password = resolve_password(&saved, password)?;
    let name = if saved.name.trim().is_empty() {
        format!("{}@{}", saved.user, saved.host)
    } else {
        saved.name.clone()
    };
    let mut profile = saved.to_connect_profile(password);
    profile.connect_timeout = state.settings().connect_timeout();
    open_workspace(
        &state,
        profile,
        Some(saved.id.clone()),
        name,
        saved.color.clone(),
        saved.read_only,
        saved.production,
    )
    .await
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>, workspace_id: u64) -> Result<(), UiError> {
    let removed = state.workspaces.lock().unwrap().remove(&workspace_id);
    if let Some(workspace) = removed {
        // A statement still running would otherwise carry on at the server
        // with nothing left here to stop it. Interrupt it; the connections
        // themselves close when the last reference to the workspace goes.
        let tabs: Vec<Arc<Tab>> = workspace.tabs.lock().await.values().cloned().collect();
        for tab in tabs {
            if tab.session.try_lock().is_err() {
                tab.cancel_requested.store(true, Ordering::Relaxed);
                tauri::async_runtime::spawn(async move {
                    let _ = tab.canceller.cancel().await;
                });
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn close_tab(state: State<'_, AppState>, workspace_id: u64, tab_id: String) -> Result<(), UiError> {
    if let Ok(workspace) = state.workspace(workspace_id) {
        workspace.tabs.lock().await.remove(&tab_id);
    }
    Ok(())
}

// ---------- running statements ----------

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum RunMode {
    /// Every statement in the script.
    All,
    /// Only the statement at this cursor position (UTF-16 offset).
    At { offset: usize },
    /// The execution plan of the statement at this cursor position.
    ExplainAt { offset: usize },
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum RunMsg {
    Started {
        index: usize,
        total: usize,
        sql: String,
    },
    ResultSet {
        index: usize,
        result_id: u64,
        columns: Vec<ColumnInfo>,
        /// The whole statement, so the UI can run it again after edits.
        sql: String,
    },
    Progress {
        result_id: u64,
        rows: u64,
    },
    ResultDone {
        index: usize,
        result_id: u64,
        rows: u64,
        truncated: bool,
        elapsed_ms: u64,
    },
    Done {
        index: usize,
        affected_rows: u64,
        last_insert_id: Option<u64>,
        info: String,
        warnings: u16,
        elapsed_ms: u64,
    },
    Failed {
        index: usize,
        message: String,
        elapsed_ms: u64,
    },
    /// The session's default database changed (after `USE`).
    Database {
        name: Option<String>,
    },
    /// The server had closed the tab's connection; a new one was opened
    /// before anything was sent.
    Reconnected,
    /// How far a query run in time slices has got.
    #[serde(rename_all = "camelCase")]
    Slices {
        done: usize,
        total: usize,
        rows: u64,
        /// How long the rest should take at the pace so far. `None` before the first slice is in.
        eta_ms: Option<u64>,
        /// Connections running slices at once.
        threads: usize,
    },
    Finished {
        ran: usize,
        total: usize,
        failed: bool,
        elapsed_ms: u64,
    },
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn preview(sql: &str) -> String {
    let mut out: String = sql.chars().take(300).collect();
    if out.len() < sql.len() {
        out.push('\u{2026}');
    }
    out
}

/// The statements a run in this mode would execute.
fn select_statements(script: &str, mode: &RunMode) -> Vec<split::Statement> {
    let all = split::split(script);
    match mode {
        RunMode::All => all,
        RunMode::At { offset } => {
            let byte_offset = split::utf16_to_byte_offset(script, *offset);
            split::statement_at(&all, byte_offset).cloned().into_iter().collect()
        }
        RunMode::ExplainAt { offset } => {
            let byte_offset = split::utf16_to_byte_offset(script, *offset);
            split::statement_at(&all, byte_offset)
                .cloned()
                .map(|mut statement| {
                    if split::first_keyword(&statement.text) != "EXPLAIN" {
                        statement.text = format!("EXPLAIN {}", statement.text);
                    }
                    statement
                })
                .into_iter()
                .collect()
        }
    }
}

/// What a production connection should confirm before running this:
/// structure changes, and UPDATE or DELETE without a WHERE clause.
#[tauri::command(async)]
fn review_script(script: String, mode: RunMode) -> Vec<Concern> {
    let statements = select_statements(&script, &mode);
    guard::review(statements.iter().map(|s| s.text.as_str()))
}

/// Runs the script's statements one after another in the tab's session,
/// stopping at the first failure. Progress is reported through `on_event`;
/// the command itself returns when everything has finished.
#[tauri::command]
async fn run(
    state: State<'_, AppState>,
    workspace_id: u64,
    tab_id: String,
    script: String,
    mode: RunMode,
    max_rows: Option<u64>,
    on_event: Channel<RunMsg>,
) -> Result<(), UiError> {
    let workspace = state.workspace(workspace_id)?;
    let statements = select_statements(&script, &mode);

    let tab = workspace.tab(&tab_id).await?;
    let mut session = tab
        .session
        .try_lock()
        .map_err(|_| ui_error("busy", "A query is already running in this tab."))?;
    tab.cancel_requested.store(false, Ordering::Relaxed);

    // Nothing has been sent yet, so reopening a dropped connection here can
    // never run a statement twice.
    match session.ensure_alive(IDLE_CHECK).await {
        Ok(true) => {
            let _ = on_event.send(RunMsg::Reconnected);
        }
        Ok(false) => {}
        Err(e) => {
            return Err(ui_error(
                e.kind(),
                format!("The connection to the server was lost and could not be reopened: {e}"),
            ))
        }
    }
    workspace.adopt_default_schema(&mut session).await;
    let settings = state.settings();
    let query_timeout = settings.query_timeout();

    let run_started = Instant::now();
    let total = statements.len();
    let mut ran = 0;
    let mut failed = false;

    for (index, statement) in statements.iter().enumerate() {
        let _ = on_event.send(RunMsg::Started {
            index,
            total,
            sql: preview(&statement.text),
        });
        let started = Instant::now();
        let elapsed = || started.elapsed().as_millis() as u64;
        let mut summary = String::new();
        let mut current_result = 0u64;

        if workspace.read_only && !guard::allowed_when_read_only(&statement.text) {
            // Refused here, before anything is sent. The server would refuse
            // it too, since the session is in read-only mode.
            failed = true;
            let _ = on_event.send(RunMsg::Failed {
                index,
                message: "Not run: this connection is read-only, and this statement is not a plain read.".into(),
                elapsed_ms: 0,
            });
            break;
        }

        // The query timeout from the settings: interrupt the statement on
        // the server once it has run that long.
        let timed_out = Arc::new(AtomicBool::new(false));
        let timer = query_timeout.map(|limit| {
            let canceller = tab.canceller.clone();
            let timed_out = timed_out.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(limit).await;
                timed_out.store(true, Ordering::Relaxed);
                let _ = canceller.cancel().await;
            })
        });

        let mut outcome = session
            .run(&statement.text, max_rows, |event| {
                let message = match event {
                    RunEvent::ResultSet { store } => {
                        current_result = state.id();
                        let columns = store.status().columns;
                        state.results.lock().unwrap().insert(current_result, store);
                        RunMsg::ResultSet {
                            index,
                            result_id: current_result,
                            columns,
                            sql: statement.text.clone(),
                        }
                    }
                    RunEvent::Progress { rows } => RunMsg::Progress {
                        result_id: current_result,
                        rows,
                    },
                    RunEvent::ResultDone { rows, truncated } => {
                        summary = format!("{rows} rows{}", if truncated { " (row limit reached)" } else { "" });
                        RunMsg::ResultDone {
                            index,
                            result_id: current_result,
                            rows,
                            truncated,
                            elapsed_ms: elapsed(),
                        }
                    }
                    RunEvent::Done {
                        affected_rows,
                        last_insert_id,
                        info,
                        warnings,
                    } => {
                        summary = format!("{affected_rows} rows affected");
                        RunMsg::Done {
                            index,
                            affected_rows,
                            last_insert_id,
                            info,
                            warnings,
                            elapsed_ms: elapsed(),
                        }
                    }
                };
                let _ = on_event.send(message);
            })
            .await;

        if let Some(timer) = timer {
            if timed_out.load(Ordering::Relaxed) {
                // Let the interrupt finish going out, then make sure a late
                // one cannot hit whatever runs next.
                let _ = timer.await;
                session.absorb_stray_kill().await;
            } else {
                timer.abort();
                if timed_out.load(Ordering::Relaxed) {
                    session.absorb_stray_kill().await;
                }
            }
        }
        if timed_out.load(Ordering::Relaxed) && outcome.is_err() {
            let seconds = query_timeout.map_or(0, |limit| limit.as_secs());
            outcome = Err(Error::Other(format!(
                "Stopped after {seconds} second{}, the query timeout in Settings",
                if seconds == 1 { "" } else { "s" }
            )));
        }

        ran += 1;
        if let Err(e) = &outcome {
            summary = e.to_string();
            if e.is_connection_lost() {
                summary.push_str(
                    ". The connection to the server was lost. It will be reopened the next time \
                     something is run in this tab; session variables, temporary tables and any \
                     open transaction are gone",
                );
            }
            failed = true;
            let _ = on_event.send(RunMsg::Failed {
                index,
                message: summary.clone(),
                elapsed_ms: elapsed(),
            });
        }
        // History is a convenience; failing to write it must not fail the query.
        if settings.keep_history {
            let _ = state.history.append(HistoryEntry {
                at_ms: now_ms(),
                profile_name: workspace.name.clone(),
                sql: statement.text.clone(),
                duration_ms: elapsed(),
                ok: outcome.is_ok(),
                summary,
            });
        }
        if failed {
            break;
        }
        if split::first_keyword(&statement.text) == "USE" {
            if let Ok(name) = session.current_database().await {
                // USE in a script changes the default for the whole connection.
                workspace.set_default_schema(name.clone());
                record_default_schema(&state, &workspace, name.as_deref());
                let _ = on_event.send(RunMsg::Database { name });
            }
        }
    }

    if tab.cancel_requested.swap(false, Ordering::Relaxed) {
        session.absorb_stray_kill().await;
    }
    let _ = on_event.send(RunMsg::Finished {
        ran,
        total,
        failed,
        elapsed_ms: run_started.elapsed().as_millis() as u64,
    });
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SlicePlan {
    slices: usize,
    /// The first slice's statement, to show what will be sent.
    first: String,
    last: String,
    threads: usize,
}

/// What running a query in time slices would do: how many slices, and the
/// statement of the first and last. Rejects with the reason if it cannot be sliced.
#[tauri::command(async)]
fn slice_plan(state: State<'_, AppState>, sql: String, start: String, end: String, step_secs: u64) -> Result<SlicePlan, UiError> {
    let plan = slices::plan(&sql, &start, &end, step_secs)?;
    Ok(SlicePlan {
        slices: plan.len(),
        first: plan[0].sql.clone(),
        last: plan[plan.len() - 1].sql.clone(),
        threads: (state.settings().slice_threads as usize).min(plan.len()),
    })
}

/// Runs one query as many: the time range between `start` and `end` is cut
/// into slices of `step_secs`, the query (which marks its range with
/// `{start}` and `{end}`) is run for each, and the rows are put end to end
/// in the order of the slices, as one result. No single run is long, so none
/// meets a timeout.
///
/// The slices run on connections of their own, as many at once as the
/// settings allow, so the tab's session is not touched; the tab is still
/// held busy until the run ends, and `cancel` with its id stops it.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn run_sliced(
    state: State<'_, AppState>,
    workspace_id: u64,
    tab_id: String,
    sql: String,
    start: String,
    end: String,
    step_secs: u64,
    max_rows: Option<u64>,
    on_event: Channel<RunMsg>,
) -> Result<(), UiError> {
    let workspace = state.workspace(workspace_id)?;
    let plan = Arc::new(slices::plan(&sql, &start, &end, step_secs)?);
    let whole = slices::whole(&sql, &start, &end)?;
    let tab = workspace.tab(&tab_id).await?;
    let mut session = tab
        .session
        .try_lock()
        .map_err(|_| ui_error("busy", "A query is already running in this tab."))?;
    tab.cancel_requested.store(false, Ordering::Relaxed);
    let settings = state.settings();
    let total = plan.len();
    let run_started = Instant::now();
    let elapsed = || run_started.elapsed().as_millis() as u64;
    let _ = on_event.send(RunMsg::Started { index: 0, total: 1, sql: preview(&whole) });

    // A connection for each slice running at once. If the server will not
    // give as many as were asked for, the run goes ahead with what it gave.
    let wanted = (settings.slice_threads as usize).clamp(1, total);
    let mut workers: Vec<(String, Arc<Tab>)> = Vec::new();
    let mut refused: Option<UiError> = None;
    for n in 0..wanted {
        let id = format!("{tab_id}#slice{n}");
        match workspace.tab(&id).await {
            Ok(worker) => {
                worker.cancel_requested.store(false, Ordering::Relaxed);
                workers.push((id, worker));
            }
            Err(e) => {
                refused = Some(UiError::from(e));
                break;
            }
        }
    }
    let threads = workers.len();

    let next = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(usize, Result<Arc<ResultStore>, Error>)>();
    let query_timeout = settings.query_timeout();
    for (_, worker) in &workers {
        let (worker, plan, next, stop, tx) = (worker.clone(), plan.clone(), next.clone(), stop.clone(), tx.clone());
        tauri::async_runtime::spawn(async move {
            let mut session = worker.session.lock().await;
            while !stop.load(Ordering::Relaxed) {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(slice) = plan.get(index) else { break };
                // The query timeout applies to each slice, which is the point of slicing.
                let timed_out = Arc::new(AtomicBool::new(false));
                let timer = query_timeout.map(|limit| {
                    let (canceller, timed_out) = (worker.canceller.clone(), timed_out.clone());
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(limit).await;
                        timed_out.store(true, Ordering::Relaxed);
                        let _ = canceller.cancel().await;
                    })
                });
                let mut found: Option<Arc<ResultStore>> = None;
                let outcome = session
                    .run(&slice.sql, max_rows, |event| {
                        if let RunEvent::ResultSet { store } = event {
                            found.get_or_insert(store);
                        }
                    })
                    .await;
                if let Some(timer) = timer {
                    timer.abort();
                }
                if timed_out.load(Ordering::Relaxed) || worker.cancel_requested.load(Ordering::Relaxed) {
                    session.absorb_stray_kill().await;
                }
                let message = match outcome {
                    Ok(()) => found.ok_or_else(|| Error::Other("the statement returned no result set".into())),
                    Err(_) if timed_out.load(Ordering::Relaxed) => Err(Error::Other(
                        "it ran longer than the query timeout in Settings. Use shorter slices, or a longer timeout".into(),
                    )),
                    Err(e) => Err(e),
                };
                let failed = message.is_err();
                if tx.send((index, message)).is_err() || failed {
                    break;
                }
            }
        });
    }
    drop(tx);

    // Slices finish in any order; their rows are added in the order of the slices.
    let combined = Arc::new(ResultStore::new());
    let result_id = state.id();
    let mut waiting: HashMap<usize, Arc<ResultStore>> = HashMap::new();
    let (mut announced, mut added, mut done, mut rows, mut truncated) = (false, 0usize, 0usize, 0u64, false);
    let mut failure: Option<String> = refused.filter(|_| threads == 0).map(|e| e.message);
    let mut tick = tokio::time::interval(Duration::from_millis(200));
    while failure.is_none() && !truncated && added < total {
        tokio::select! {
            message = rx.recv() => match message {
                Some((index, Ok(store))) => {
                    done += 1;
                    waiting.insert(index, store);
                    while let Some(store) = waiting.remove(&added) {
                        let room = max_rows.map(|limit| limit.saturating_sub(rows));
                        let piece = store.status();
                        let (target, from) = (combined.clone(), store.clone());
                        rows += blocking(move || target.append_from(&from, room)).await?;
                        added += 1;
                        if !announced {
                            announced = true;
                            let columns = combined.status().columns;
                            state.results.lock().unwrap().insert(result_id, combined.clone());
                            let _ = on_event.send(RunMsg::ResultSet { index: 0, result_id, columns, sql: whole.clone() });
                        }
                        // The row limit counts the rows of all the slices together.
                        if piece.truncated || room.is_some_and(|room| piece.total_rows > room) || (max_rows.is_some_and(|limit| rows >= limit) && added < total) {
                            truncated = true;
                            break;
                        }
                    }
                    let eta_ms = (done < total).then(|| elapsed() * (total - done) as u64 / done as u64);
                    let _ = on_event.send(RunMsg::Progress { result_id, rows });
                    let _ = on_event.send(RunMsg::Slices { done, total, rows, eta_ms, threads });
                }
                Some((index, Err(e))) => {
                    let slice = &plan[index];
                    failure = Some(format!("The slice from {} to {} failed: {e}", slice.start, slice.end));
                }
                None => failure = Some("The slices stopped before all of them had run.".into()),
            },
            _ = tick.tick() => {
                if tab.cancel_requested.load(Ordering::Relaxed) {
                    failure = Some("Stopped.".into());
                }
            }
        }
    }

    // Whatever ended the run, nothing more is started and what is running is interrupted.
    stop.store(true, Ordering::Relaxed);
    if added < total {
        for (_, worker) in &workers {
            worker.cancel_requested.store(true, Ordering::Relaxed);
            let _ = worker.canceller.cancel().await;
        }
    }
    {
        let mut tabs = workspace.tabs.lock().await;
        for (id, _) in &workers {
            tabs.remove(id);
        }
    }
    if tab.cancel_requested.swap(false, Ordering::Relaxed) {
        session.absorb_stray_kill().await;
    }

    if let Some(message) = &mut failure {
        if announced && added > 0 {
            message.push_str(&format!(
                ". The result holds the {added} of {total} slices before it, up to {}",
                plan[added - 1].end
            ));
        }
    }
    if truncated {
        combined.set_truncated();
    }
    combined.finish(failure.clone());
    if announced {
        let _ = on_event.send(RunMsg::ResultDone { index: 0, result_id, rows, truncated: truncated || failure.is_some(), elapsed_ms: elapsed() });
    }
    if let Some(message) = &failure {
        let _ = on_event.send(RunMsg::Failed { index: 0, message: message.clone(), elapsed_ms: elapsed() });
    }
    if settings.keep_history {
        let _ = state.history.append(HistoryEntry {
            at_ms: now_ms(),
            profile_name: workspace.name.clone(),
            sql: whole,
            duration_ms: elapsed(),
            ok: failure.is_none(),
            summary: failure.clone().unwrap_or_else(|| format!("{rows} rows, in {total} time slices")),
        });
    }
    let _ = on_event.send(RunMsg::Finished { ran: 1, total: 1, failed: failure.is_some(), elapsed_ms: elapsed() });
    Ok(())
}

/// Makes `schema` the default for every tab of the connection. Checked
/// against the server first, so a schema that does not exist is refused here.
#[tauri::command]
async fn set_default_schema(state: State<'_, AppState>, workspace_id: u64, schema: String) -> Result<(), UiError> {
    let workspace = state.workspace(workspace_id)?;
    workspace.utility().await?.use_database(&schema).await?;
    record_default_schema(&state, &workspace, Some(&schema));
    workspace.set_default_schema(Some(schema));
    Ok(())
}

/// Writes the connection's default schema into its saved settings, so the
/// Connections page, the connection's settings and the next connect all
/// agree with what is selected now. Not being able to write it is not worth
/// failing a statement over; the schema is still selected for this run.
fn record_default_schema(state: &AppState, workspace: &Workspace, schema: Option<&str>) {
    let Some(id) = workspace.profile_id.as_deref() else {
        return;
    };
    if let Ok(Some(mut saved)) = state.profiles.get(id) {
        if saved.database.as_deref() != schema {
            saved.database = schema.map(str::to_string);
            let _ = state.profiles.upsert(saved);
        }
    }
}

/// Interrupts whatever the tab is running.
#[tauri::command]
async fn cancel(state: State<'_, AppState>, workspace_id: u64, tab_id: String) -> Result<(), UiError> {
    let workspace = state.workspace(workspace_id)?;
    let tab = workspace.tabs.lock().await.get(&tab_id).cloned();
    if let Some(tab) = tab {
        tab.cancel_requested.store(true, Ordering::Relaxed);
        tab.canceller.cancel().await?;
    }
    Ok(())
}

// ---------- reading results ----------

/// A window of rows as raw bytes (layout in `sqltool_core::store`).
/// Raw, not JSON: this call is on the scrolling hot path.
#[tauri::command(async)]
fn get_rows(state: State<'_, AppState>, result_id: u64, offset: u64, count: u32) -> Result<Response, UiError> {
    Ok(Response::new(state.result(result_id)?.window(offset, count.min(4096))))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CellDto {
    /// `None` for SQL NULL.
    text: Option<String>,
    /// Size of the full value in bytes.
    bytes: usize,
    /// True when `text` was cut because the value is very large.
    truncated: bool,
}

/// The complete value of one cell, for the value viewer and for copying.
#[tauri::command(async)]
fn get_cell(state: State<'_, AppState>, result_id: u64, row: u64, column: usize) -> Result<CellDto, UiError> {
    let value = state
        .result(result_id)?
        .cell(row, column)
        .ok_or_else(|| ui_error("no-cell", "That cell does not exist."))?;
    Ok(match value {
        None => CellDto {
            text: None,
            bytes: 0,
            truncated: false,
        },
        Some(bytes) => {
            let shown = &bytes[..bytes.len().min(MAX_CELL_BYTES)];
            CellDto {
                text: Some(String::from_utf8_lossy(shown).into_owned()),
                bytes: bytes.len(),
                truncated: bytes.len() > MAX_CELL_BYTES,
            }
        }
    })
}

#[tauri::command]
fn close_results(state: State<'_, AppState>, result_ids: Vec<u64>) {
    let mut results = state.results.lock().unwrap();
    let mut charts = state.charts.lock().unwrap();
    for id in result_ids {
        results.remove(&id);
        charts.remove(&id);
    }
}

// ---------- schema browser ----------

#[tauri::command]
async fn schema_list(state: State<'_, AppState>, workspace_id: u64) -> Result<Vec<String>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.schemas().await?)
}

#[tauri::command]
async fn schema_objects(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
) -> Result<SchemaObjects, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.schema_objects(&schema).await?)
}

#[tauri::command]
async fn table_columns(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
    table: String,
) -> Result<Vec<ColumnEntry>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.table_columns(&schema, &table).await?)
}

#[tauri::command]
async fn table_details(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
    table: String,
) -> Result<Option<TableDetails>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.table_details(&schema, &table).await?)
}

#[tauri::command]
async fn schema_defaults(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
) -> Result<Option<SchemaDefaults>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.schema_defaults(&schema).await?)
}

#[tauri::command]
async fn charsets(state: State<'_, AppState>, workspace_id: u64) -> Result<Vec<Charset>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.charsets().await?)
}

#[tauri::command]
async fn collations(state: State<'_, AppState>, workspace_id: u64, charset: String) -> Result<Vec<String>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.collations(&charset).await?)
}

// ---------- building SQL for schema changes ----------

/// The SQL to create (`alter` false) or alter a schema, for review before running.
#[tauri::command]
fn schema_ddl(alter: bool, name: String, charset: Option<String>, collation: Option<String>) -> Result<String, UiError> {
    let (charset, collation) = (charset.as_deref(), collation.as_deref());
    Ok(if alter {
        ddl::alter_schema(&name, charset, collation)?
    } else {
        ddl::create_schema(&name, charset, collation)?
    })
}

/// A script that changes an existing object, built from its current definition.
#[tauri::command]
async fn alter_script(
    state: State<'_, AppState>,
    workspace_id: u64,
    kind: ObjectKind,
    schema: String,
    name: String,
) -> Result<String, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    let create = session.show_create(kind, &schema, &name).await?;
    // MariaDB can replace routines, triggers and events in one statement.
    let or_replace = session.info().version.flavor == sqltool_core::Flavor::MariaDb;
    Ok(ddl::alter_script(kind, &schema, &name, &create, or_replace))
}

/// A starting script for a new object.
#[tauri::command]
fn create_template(kind: ObjectKind, schema: Option<String>) -> String {
    ddl::create_template(kind, schema.as_deref())
}

// ---------- script files ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptFile {
    path: String,
    /// File name without the folder, for the tab title.
    name: String,
    text: String,
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Shows the native open or save dialog and returns the chosen path. The
/// dialog runs in the shell, so the UI never supplies a path of its own.
///
/// `start` is the folder the dialog opens in: the connection's script folder.
async fn pick_file(app: &tauri::AppHandle, save_as: Option<&str>, start: Option<PathBuf>) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .file()
        .add_filter("SQL scripts", &["sql"])
        .add_filter("All files", &["*"]);
    if let Some(folder) = start {
        dialog = dialog.set_directory(folder);
    }
    let done = move |path| {
        let _ = tx.send(path);
    };
    match save_as {
        Some(name) => dialog.set_file_name(name).save_file(done),
        None => dialog.pick_file(done),
    }
    rx.await.ok().flatten().and_then(|path| path.into_path().ok())
}

/// The folder a connection's file dialogs start in: its script folder, if it
/// has one and the folder is there.
///
/// Read from the saved connection each time, not kept from when the
/// connection was opened: a folder set or changed while the connection is
/// open applies at once. (A connection with no folder to give leaves the
/// dialog where it last was, which with two connections open is the other
/// connection's folder.)
fn script_folder(state: &AppState, workspace_id: Option<u64>) -> Option<PathBuf> {
    let workspace = state.workspace(workspace_id?).ok()?;
    let saved = state.profiles.get(workspace.profile_id.as_deref()?).ok()??;
    saved.script_folder.filter(|folder| folder.is_dir())
}

/// Where Open and Save As will start for this connection, if it has a script folder.
#[tauri::command(async)]
fn script_folder_of(state: State<'_, AppState>, workspace_id: u64) -> Option<String> {
    script_folder(&state, Some(workspace_id)).map(|folder| folder.to_string_lossy().into_owned())
}

/// Lets the user choose a folder, for a connection's script folder. `None` if cancelled.
#[tauri::command]
async fn folder_pick(app: tauri::AppHandle, start: Option<String>) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file();
    if let Some(start) = start.map(PathBuf::from).filter(|folder| folder.is_dir()) {
        dialog = dialog.set_directory(start);
    }
    dialog.pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let folder = rx.await.ok().flatten()?.into_path().ok()?;
    Some(folder.to_string_lossy().into_owned())
}

/// Lets the user pick a script and returns its contents. `None` if cancelled.
/// The dialog starts in the connection's script folder.
#[tauri::command]
async fn script_open(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_id: Option<u64>,
) -> Result<Option<ScriptFile>, UiError> {
    let Some(path) = pick_file(&app, None, script_folder(&state, workspace_id)).await else {
        return Ok(None);
    };
    let storage = |e: std::io::Error| ui_error("storage", format!("Could not open {}: {e}", path.display()));
    if std::fs::metadata(&path).map_err(storage)?.len() > MAX_SCRIPT_BYTES {
        return Err(ui_error("storage", "That file is larger than 20 MB, which is more than the editor opens."));
    }
    let text = String::from_utf8(std::fs::read(&path).map_err(storage)?).map_err(|_| {
        ui_error("storage", "That file is not UTF-8 text. Convert it to UTF-8 to open it here.")
    })?;
    // The file is open either way; not being able to remember it only means
    // Save will ask where after a restart.
    let _ = state.approved_files.add(path.clone());
    Ok(Some(ScriptFile {
        name: file_name(&path),
        path: path.to_string_lossy().into_owned(),
        text,
    }))
}

/// Saves a script. With `path`, writes back to a file opened or saved earlier
/// in this session; without, asks where to save. `None` if cancelled.
#[tauri::command]
async fn script_save(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_id: Option<u64>,
    path: Option<String>,
    suggested_name: String,
    text: String,
) -> Result<Option<ScriptFile>, UiError> {
    let target = match path {
        Some(path) => {
            let path = PathBuf::from(path);
            if !state.approved_files.contains(&path) {
                return Err(ui_error(
                    "not-approved",
                    "That file has not been chosen in an Open or Save dialog. Use Save As to choose where to save.",
                ));
            }
            path
        }
        None => match pick_file(&app, Some(&suggested_name), script_folder(&state, workspace_id)).await {
            Some(path) => path,
            None => return Ok(None),
        },
    };
    std::fs::write(&target, &text)
        .map_err(|e| ui_error("storage", format!("Could not save {}: {e}", target.display())))?;
    let _ = state.approved_files.add(target.clone());
    Ok(Some(ScriptFile {
        name: file_name(&target),
        path: target.to_string_lossy().into_owned(),
        text: String::new(),
    }))
}

// ---------- sorting, filtering and exporting results ----------

/// Sorts and filters what the grid shows. Returns the number of visible rows.
#[tauri::command]
async fn result_view(
    state: State<'_, AppState>,
    result_id: u64,
    filter: Option<String>,
    sort_column: Option<usize>,
    descending: bool,
) -> Result<u64, UiError> {
    let store = state.result(result_id)?;
    blocking(move || store.set_view(filter.as_deref(), sort_column.map(|column| (column, descending)))).await
}

/// The cells of a selection as text for the clipboard. The text is built
/// here, from the stored rows, so a selection of any size copies exactly
/// what the server sent rather than what happens to be on screen.
#[tauri::command]
async fn copy_range(
    state: State<'_, AppState>,
    result_id: u64,
    range: CellRange,
    format: CopyFormat,
    headers: bool,
    table: String,
) -> Result<String, UiError> {
    let store = state.result(result_id)?;
    Ok(blocking(move || export::copy_range(&store, range, format, headers, &table)).await??)
}

/// Count, sum, average, smallest and largest of a selection.
#[tauri::command]
async fn range_stats(state: State<'_, AppState>, result_id: u64, range: CellRange) -> Result<RangeStats, UiError> {
    let store = state.result(result_id)?;
    blocking(move || export::range_stats(&store, range)).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Exported {
    path: String,
    rows: u64,
    /// The result stopped at the row limit, so the file does not hold
    /// everything the query would return.
    truncated: bool,
    /// A filter was on, so the file holds only the rows that matched.
    filtered: bool,
}

/// Asks where to save an export. `None` if the user cancels.
async fn pick_export_file(app: &tauri::AppHandle, format: ExportFormat) -> Option<PathBuf> {
    pick_save_file(app, "result", format.extension()).await
}

/// Asks where to save a file called `name.extension`. `None` if the user cancels.
async fn pick_save_file(app: &tauri::AppHandle, name: &str, extension: &str) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    // The unattended self-test cannot answer a file dialog. In a build that
    // includes it, and only while it is running, exports go to the folder it
    // names, so the real export commands can be exercised end to end.
    #[cfg(feature = "devtools")]
    if std::env::var_os("SQLTOOL_SELFTEST_HOST").is_some() {
        if let Some(folder) = std::env::var_os("SQLTOOL_SELFTEST_EXPORT_DIR") {
            return Some(PathBuf::from(folder).join(format!("{name}.{extension}")));
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter(extension.to_uppercase(), &[extension])
        .set_file_name(format!("{name}.{extension}"))
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    rx.await.ok().flatten().and_then(|p| p.into_path().ok())
}

/// Asks where to save, then writes every row a query returns in `format`,
/// by running the statement again and writing its rows straight to the file.
/// The row limit does not apply and nothing is kept in memory, so this is
/// the way to export a result of millions of rows. `None` if the user cancels.
///
/// It runs in a connection of its own under `tab_id`, so the tab that showed
/// the result stays usable, and `cancel` with that id stops it.
#[tauri::command]
async fn export_query(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    workspace_id: u64,
    tab_id: String,
    sql: String,
    format: ExportFormat,
    table: String,
    on_progress: Channel<u64>,
) -> Result<Option<Exported>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    // Only a plain read is run a second time. A procedure call or a change
    // could do something again that was meant to happen once.
    if guard::classify(&sql) != guard::StatementKind::Read {
        return Err(ui_error(
            "not-exportable",
            "Only a SELECT or another plain read can be run again to export all of its rows. \
             Raise the row limit, run the statement again, and export the rows shown.",
        ));
    }
    let Some(path) = pick_export_file(&app, format).await else {
        return Ok(None);
    };
    let storage = |e: std::io::Error| ui_error("storage", format!("Could not write {}: {e}", path.display()));

    let tab = workspace.tab(&tab_id).await?;
    let outcome = {
        let mut session = tab.session.lock().await;
        let mut writer = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(&path).map_err(storage)?);
        let rows = session
            .export(&sql, format, &table, &mut writer, |rows| {
                let _ = on_progress.send(rows);
            })
            .await;
        if tab.cancel_requested.load(Ordering::Relaxed) {
            session.absorb_stray_kill().await;
        }
        match rows {
            Ok(rows) => std::io::Write::flush(&mut writer).map(|()| rows).map_err(storage),
            Err(e) => Err(UiError::from(e)),
        }
    };
    let stopped = tab.cancel_requested.load(Ordering::Relaxed);
    // The export's own connection is no longer needed.
    workspace.tabs.lock().await.remove(&tab_id);

    match outcome {
        Ok(rows) => Ok(Some(Exported {
            path: path.to_string_lossy().into_owned(),
            rows,
            truncated: false,
            filtered: false,
        })),
        Err(e) => {
            // An incomplete file would pass for the whole result. Remove it.
            let _ = std::fs::remove_file(&path);
            Err(if stopped {
                ui_error("stopped", "Export stopped. The incomplete file was removed.")
            } else {
                ui_error(&e.kind, format!("{} The incomplete file was removed.", e.message))
            })
        }
    }
}

/// Asks where to save, then writes the result's visible rows in `format`.
/// `None` if the user cancels.
///
/// These are the rows the app holds. For a result that stopped at the row
/// limit that is not every row the query returns; `export_query` is.
#[tauri::command]
async fn export_result(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    result_id: u64,
    format: ExportFormat,
    table: String,
) -> Result<Option<Exported>, UiError> {
    let store = state.result(result_id)?;
    let Some(path) = pick_export_file(&app, format).await else {
        return Ok(None);
    };
    let storage = |e: std::io::Error| ui_error("storage", format!("Could not write {}: {e}", path.display()));
    let target = path.clone();
    let (rows, status) = blocking(move || -> std::io::Result<_> {
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&target)?);
        let rows = export::write(&store, format, &table, &mut writer)?;
        std::io::Write::flush(&mut writer)?;
        Ok((rows, store.status()))
    })
    .await?
    .map_err(storage)?;
    Ok(Some(Exported {
        path: path.to_string_lossy().into_owned(),
        rows,
        truncated: status.truncated,
        filtered: status.rows < status.total_rows,
    }))
}

// ---------- charts ----------

/// What a chart of a result shows: the bars, bins or points to draw, worked
/// out here from every stored row. The rows themselves never go to the UI.
#[tauri::command]
async fn chart_data(state: State<'_, AppState>, result_id: u64, request: ChartRequest) -> Result<ChartData, UiError> {
    let data = state.chart(result_id)?;
    blocking(move || data.chart(&request)).await?.map_err(|e| ui_error("chart", e.to_string()))
}

/// What one column holds when read as numbers, times or categories, for a filter control.
#[tauri::command]
async fn chart_column(state: State<'_, AppState>, result_id: u64, column: usize, role: chart::Role) -> Result<ColumnSummary, UiError> {
    let data = state.chart(result_id)?;
    blocking(move || data.summary(column, role)).await?.map_err(|e| ui_error("chart", e.to_string()))
}

/// Lets go of what was kept for a result's chart. The result itself stays.
#[tauri::command]
fn chart_close(state: State<'_, AppState>, result_id: u64) {
    state.charts.lock().unwrap().remove(&result_id);
}

/// Asks where to save, then writes a picture of a chart there. The request
/// body is the PNG itself. Returns the path, or `None` if the user cancels.
#[tauri::command]
async fn chart_save_image(app: tauri::AppHandle, request: tauri::ipc::Request<'_>) -> Result<Option<String>, UiError> {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let tauri::ipc::InvokeBody::Raw(image) = request.body() else {
        return Err(ui_error("other", "The picture did not arrive as image data."));
    };
    if !image.starts_with(PNG) || image.len() > MAX_IMAGE_BYTES {
        return Err(ui_error("other", "That is not a PNG picture this app can save."));
    }
    let image = image.clone();
    let Some(path) = pick_save_file(&app, "chart", "png").await else {
        return Ok(None);
    };
    let target = path.clone();
    blocking(move || std::fs::write(&target, image))
        .await?
        .map_err(|e| ui_error("storage", format!("Could not write {}: {e}", path.display())))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

// ---------- editing rows ----------

/// Whether a result can be edited, and which table the edits would go to.
#[tauri::command]
async fn edit_target(state: State<'_, AppState>, workspace_id: u64, result_id: u64) -> Result<EditTarget, UiError> {
    let workspace = state.workspace(workspace_id)?;
    if workspace.read_only {
        return Err(ui_error("not-editable", "This connection is read-only."));
    }
    let store = state.result(result_id)?;
    let mut session = workspace.utility().await?;
    session
        .edit_target(&store)
        .await
        .map_err(|e| ui_error("not-editable", e.to_string()))
}

/// The statements the edits would run, with values written in, for review.
#[tauri::command(async)]
fn preview_edits(state: State<'_, AppState>, result_id: u64, edits: Vec<RowEdit>) -> Result<Vec<String>, UiError> {
    let store = state.result(result_id)?;
    let target = edit::edit_target(&store.status().columns).map_err(|e| ui_error("not-editable", e))?;
    Ok(edit::plan(&target, &store, &edits)?.into_iter().map(|s| s.preview).collect())
}

/// Applies the edits in one transaction in the tab's own session.
#[tauri::command]
async fn apply_edits(
    state: State<'_, AppState>,
    workspace_id: u64,
    tab_id: String,
    result_id: u64,
    edits: Vec<RowEdit>,
) -> Result<usize, UiError> {
    let workspace = state.workspace(workspace_id)?;
    if workspace.read_only {
        return Err(ui_error("not-editable", "This connection is read-only."));
    }
    let store = state.result(result_id)?;
    let tab = workspace.tab(&tab_id).await?;
    let mut session = tab
        .session
        .try_lock()
        .map_err(|_| ui_error("busy", "A query is running in this tab. Wait for it to finish."))?;
    session.ensure_alive(IDLE_CHECK).await?;
    let keep_history = state.settings().keep_history;
    let started = Instant::now();
    let target = session
        .edit_target(&store)
        .await
        .map_err(|e| ui_error("not-editable", e.to_string()))?;
    let planned = edit::plan(&target, &store, &edits)?;
    let outcome = session.apply(&planned).await;
    for statement in planned.iter().filter(|_| keep_history) {
        let _ = state.history.append(HistoryEntry {
            at_ms: now_ms(),
            profile_name: workspace.name.clone(),
            sql: statement.preview.clone(),
            duration_ms: started.elapsed().as_millis() as u64,
            ok: outcome.is_ok(),
            summary: match &outcome {
                Ok(()) => "applied from the grid".into(),
                Err(e) => format!("not applied: {e}"),
            },
        });
    }
    outcome?;
    Ok(planned.len())
}

// ---------- table designer and destructive actions ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TableKeys {
    primary_key: Vec<String>,
    indexes: Vec<IndexSpec>,
    foreign_keys: Vec<ForeignKeyEntry>,
}

/// Keys and indexes of a table, for the information pane.
#[tauri::command]
async fn table_keys(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
    table: String,
) -> Result<Option<TableKeys>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    let Some(spec) = session.table_spec(&schema, &table).await? else {
        return Ok(None);
    };
    Ok(Some(TableKeys {
        primary_key: spec.primary_key,
        indexes: spec.indexes,
        foreign_keys: session.foreign_keys(&schema, &table).await?,
    }))
}

/// A table as the designer edits it. `None` if it does not exist.
#[tauri::command]
async fn table_spec(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
    table: String,
) -> Result<Option<TableSpec>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.table_spec(&schema, &table).await?)
}

/// SQL for the designer: CREATE TABLE without `original`, otherwise the
/// ALTER TABLE that turns `original` into `updated`.
#[tauri::command]
fn table_ddl(original: Option<TableSpec>, updated: TableSpec) -> Result<String, UiError> {
    Ok(match original {
        Some(original) => table::alter_table(&original, &updated)?,
        None => table::create_table(&updated)?,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum ObjectAction {
    Drop,
    DropSchema,
    Truncate,
    Rename,
}

/// SQL for dropping, truncating or renaming, for the confirmation dialog.
#[tauri::command]
fn object_ddl(
    action: ObjectAction,
    kind: ObjectKind,
    schema: String,
    name: String,
    new_name: Option<String>,
) -> Result<String, UiError> {
    Ok(match action {
        ObjectAction::Drop => table::drop_object(kind, &schema, &name),
        ObjectAction::DropSchema => table::drop_schema(&schema),
        ObjectAction::Truncate => table::truncate_table(&schema, &name),
        ObjectAction::Rename => table::rename_table(&schema, &name, new_name.as_deref().unwrap_or(""))?,
    })
}

/// Table and column names of a schema, for editor completion.
#[tauri::command]
async fn completion_columns(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
) -> Result<Vec<(String, String)>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility().await?;
    Ok(session.completion_columns(&schema).await?)
}

// ---------- history ----------

#[tauri::command(async)]
fn history_list(
    state: State<'_, AppState>,
    search: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<HistoryEntry>, UiError> {
    Ok(state.history.list(search.as_deref(), limit.unwrap_or(200).min(2000))?)
}

#[tauri::command(async)]
fn history_clear(state: State<'_, AppState>) -> Result<(), UiError> {
    Ok(state.history.clear()?)
}

// ---------- settings ----------

/// Puts settings into effect that the shell or the core acts on directly.
fn apply_settings(app: &tauri::AppHandle, settings: &Settings) {
    sqltool_core::store::set_memory_budget(u64::from(settings.result_memory_mb) * 1024 * 1024);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_zoom(f64::from(settings.zoom) / 100.0);
    }
}

#[tauri::command(async)]
fn settings_get(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Saves the settings and returns them as stored: values outside their
/// allowed range come back corrected.
#[tauri::command(async)]
fn settings_set(app: tauri::AppHandle, state: State<'_, AppState>, settings: Settings) -> Result<Settings, UiError> {
    let saved = state.settings_store.save(settings)?;
    apply_settings(&app, &saved);
    *state.settings.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = saved.clone();
    Ok(saved)
}

/// Pings idle connections as often as the settings ask, so the server does
/// not close them. Off unless a keep-alive interval is set.
async fn keep_alive_loop(app: tauri::AppHandle) {
    let mut tick = tokio::time::interval(Duration::from_secs(15));
    loop {
        tick.tick().await;
        let state = app.state::<AppState>();
        let Some(every) = state.settings().keep_alive() else {
            continue;
        };
        let workspaces: Vec<Arc<Workspace>> = state.workspaces.lock().unwrap().values().cloned().collect();
        for workspace in workspaces {
            // A connection that is busy is not idle; skip it.
            if let Ok(mut session) = workspace.utility.try_lock() {
                if session.idle_for() >= every {
                    let _ = session.ping().await;
                }
            }
            let tabs: Vec<Arc<Tab>> = workspace.tabs.lock().await.values().cloned().collect();
            for tab in tabs {
                if let Ok(mut session) = tab.session.try_lock() {
                    if session.idle_for() >= every {
                        let _ = session.ping().await;
                    }
                }
            }
        }
    }
}

/// Records a crash in `crash.log` in the data folder, so that "it just
/// closed" can be looked into afterwards. Only the panic message and where
/// it happened are written, never queries or data.
fn log_panics(dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
            let _ = writeln!(file, "{} sqltool {}: {info}", now_ms(), env!("CARGO_PKG_VERSION"));
        }
        previous(info);
    }));
}

// ---------- app info and development tools ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    version: &'static str,
    /// Whether the demo server and self-test are compiled in.
    devtools: bool,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        devtools: cfg!(feature = "devtools"),
    }
}

/// Opens a connection to a fake MySQL Enterprise server running inside the
/// app. For trying the UI without a database; queries get canned answers and
/// `SELECT fake_rows(rows, columns)` produces a synthetic result.
#[tauri::command]
async fn connect_demo(state: State<'_, AppState>) -> Result<WorkspaceDto, UiError> {
    #[cfg(feature = "devtools")]
    {
        use sqltool_core::CaSource;
        use sqltool_fakeserver::{generate_identity, Config, FakeServer};
        const PASSWORD: &str = "demo-password-not-a-real-secret";

        let demo = state
            .demo
            .get_or_try_init(|| async {
                let identity = generate_identity(&["127.0.0.1"]);
                let server =
                    FakeServer::start(Config::mysql_8_4_enterprise_ldap(PASSWORD, Some(identity.clone()))).await?;
                Ok::<_, std::io::Error>(Demo {
                    server,
                    ca_pem: identity.ca_pem,
                })
            })
            .await
            .map_err(|e| ui_error("other", e.to_string()))?;

        let mut profile = ConnectProfile::new("127.0.0.1", demo.server.port(), "demo", PASSWORD.to_string().into());
        profile.connect_timeout = state.settings().connect_timeout();
        profile.ca = Some(CaSource::Pem(demo.ca_pem.clone().into_bytes()));
        profile.allow_cleartext_auth = true;
        open_workspace(&state, profile, None, "Demo server".into(), None, false, false).await
    }
    #[cfg(not(feature = "devtools"))]
    {
        let _ = state;
        Err(ui_error("unavailable", "The demo server is not included in this build."))
    }
}

/// Settings for the unattended self-test, taken from environment variables.
/// `None` in normal use.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SelfTest {
    profile: SavedProfile,
    password: String,
    schema: String,
    table: String,
    big_query: String,
    /// A query returning a few million rows, to load in full and export.
    export_query: String,
    /// Two folders that exist, to use as script folders.
    script_folders: Vec<String>,
}

#[tauri::command]
fn selftest_config() -> Option<SelfTest> {
    if !cfg!(feature = "devtools") {
        return None;
    }
    let var = |name: &str| std::env::var(format!("SQLTOOL_SELFTEST_{name}")).ok();
    let host = var("HOST")?;
    Some(SelfTest {
        profile: SavedProfile {
            name: "Self-test".into(),
            host,
            port: var("PORT").and_then(|p| p.parse().ok()).unwrap_or(3306),
            user: var("USER").unwrap_or_default(),
            database: var("DATABASE"),
            tls: match var("TLS").as_deref() {
                Some("disabled") => sqltool_core::TlsSetting::Disabled,
                Some("unverified") => sqltool_core::TlsSetting::Unverified,
                Some("verify-ca") => sqltool_core::TlsSetting::VerifyCa,
                Some("pinned") => sqltool_core::TlsSetting::Pinned,
                _ => sqltool_core::TlsSetting::VerifyIdentity,
            },
            ca_file: var("CA").map(PathBuf::from),
            cleartext_auth: var("CLEARTEXT").is_some(),
            cleartext_risk_accepted: var("ACCEPT_RISK").is_some(),
            save_password: true,
            ..SavedProfile::default()
        },
        password: var("PASSWORD").unwrap_or_default(),
        schema: var("SCHEMA").unwrap_or_else(|| "mysql".into()),
        table: var("TABLE").unwrap_or_else(|| "db".into()),
        big_query: var("BIG_QUERY").unwrap_or_else(|| "SELECT 1".into()),
        export_query: var("EXPORT_QUERY").unwrap_or_else(|| "SELECT 1".into()),
        script_folders: ["SCRIPTS_A", "SCRIPTS_B"].iter().filter_map(|name| var(name)).collect(),
    })
}

/// Writes the self-test report next to the data directory and quits.
#[tauri::command]
fn selftest_report(app: tauri::AppHandle, report: String) -> Result<(), UiError> {
    if !cfg!(feature = "devtools") || std::env::var("SQLTOOL_SELFTEST_HOST").is_err() {
        return Err(ui_error("unavailable", "Not in self-test mode."));
    }
    let dir = std::env::temp_dir().join("sqltool-selftest");
    std::fs::create_dir_all(&dir).map_err(|e| ui_error("storage", e.to_string()))?;
    std::fs::write(dir.join("report.json"), report).map_err(|e| ui_error("storage", e.to_string()))?;
    app.exit(0);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // SQLTOOL_DATA_DIR overrides where profiles and history are kept,
            // which also keeps test runs away from real settings.
            let data_dir = match std::env::var_os("SQLTOOL_DATA_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            log_panics(data_dir.clone());
            let state = AppState::new(data_dir);
            apply_settings(app.handle(), &state.settings());
            app.manage(state);
            tauri::async_runtime::spawn(keep_alive_loop(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            profiles_list,
            profile_save,
            profile_delete,
            connection_test,
            certificate_probe,
            connect,
            connect_demo,
            disconnect,
            close_tab,
            run,
            set_default_schema,
            slice_plan,
            run_sliced,
            cancel,
            get_rows,
            get_cell,
            close_results,
            schema_list,
            schema_objects,
            table_columns,
            table_details,
            schema_defaults,
            charsets,
            collations,
            schema_ddl,
            alter_script,
            create_template,
            folder_pick,
            script_folder_of,
            script_open,
            script_save,
            review_script,
            result_view,
            copy_range,
            range_stats,
            export_result,
            export_query,
            chart_data,
            chart_column,
            chart_close,
            chart_save_image,
            edit_target,
            preview_edits,
            apply_edits,
            table_keys,
            table_spec,
            table_ddl,
            object_ddl,
            completion_columns,
            history_list,
            history_clear,
            settings_get,
            settings_set,
            selftest_config,
            selftest_report
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
