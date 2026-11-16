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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sqltool_core::metadata::{ColumnEntry, SchemaObjects};
use sqltool_core::{
    split, Canceller, ColumnInfo, ConnectProfile, Error, History, HistoryEntry, ProfileStore, ResultStore,
    RunEvent, SavedProfile, ServerInfo, Session, TlsMode,
};
use tauri::ipc::{Channel, Response};
use tauri::{Manager, State};

/// Largest cell value sent to the UI in one piece by `get_cell`.
const MAX_CELL_BYTES: usize = 2 * 1024 * 1024;

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
    /// Used for the schema browser, so it stays responsive while tabs run.
    utility: tokio::sync::Mutex<Session>,
    tabs: tokio::sync::Mutex<HashMap<String, Arc<Tab>>>,
}

impl Workspace {
    async fn tab(&self, tab_id: &str) -> Result<Arc<Tab>, Error> {
        let mut tabs = self.tabs.lock().await;
        if let Some(tab) = tabs.get(tab_id) {
            return Ok(tab.clone());
        }
        let session = Session::connect(&self.profile).await?;
        let tab = Arc::new(Tab {
            canceller: session.canceller(),
            session: tokio::sync::Mutex::new(session),
            cancel_requested: AtomicBool::new(false),
        });
        tabs.insert(tab_id.to_string(), tab.clone());
        Ok(tab)
    }
}

#[cfg(feature = "devtools")]
struct Demo {
    server: sqltool_fakeserver::FakeServer,
    ca_pem: String,
}

struct AppState {
    profiles: ProfileStore,
    history: History,
    workspaces: Mutex<HashMap<u64, Arc<Workspace>>>,
    results: Mutex<HashMap<u64, Arc<ResultStore>>>,
    next_id: AtomicU64,
    #[cfg(feature = "devtools")]
    demo: tokio::sync::OnceCell<Demo>,
}

impl AppState {
    fn new(data_dir: PathBuf) -> Self {
        Self {
            profiles: ProfileStore::new(data_dir.join("profiles.json")),
            history: History::new(data_dir.join("history.jsonl")),
            workspaces: Mutex::new(HashMap::new()),
            results: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            #[cfg(feature = "devtools")]
            demo: tokio::sync::OnceCell::new(),
        }
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
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

#[tauri::command]
fn profiles_list(state: State<'_, AppState>) -> Result<Vec<ProfileDto>, UiError> {
    Ok(state.profiles.load()?.into_iter().map(profile_dto).collect())
}

/// Saves the profile. `password` is stored in the keychain when the profile
/// asks for that; `None` leaves an already stored password alone.
#[tauri::command]
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

#[tauri::command]
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
    name: String,
    color: Option<String>,
    host: String,
    user: String,
    info: ServerInfo,
    /// `verified`, `unverified` or `none`.
    security: &'static str,
    database: Option<String>,
}

async fn open_workspace(
    state: &AppState,
    profile: ConnectProfile,
    name: String,
    color: Option<String>,
) -> Result<WorkspaceDto, UiError> {
    let mut session = Session::connect(&profile).await?;
    let info = session.info().clone();
    let database = session.current_database().await.ok().flatten();
    let security = match profile.tls {
        TlsMode::Disabled => "none",
        TlsMode::Required => "unverified",
        TlsMode::VerifyCa | TlsMode::VerifyIdentity => "verified",
    };
    let dto = WorkspaceDto {
        id: state.id(),
        name: name.clone(),
        color,
        host: format!("{}:{}", profile.host, profile.port),
        user: profile.user.clone(),
        info,
        security,
        database,
    };
    state.workspaces.lock().unwrap().insert(
        dto.id,
        Arc::new(Workspace {
            profile,
            name,
            utility: tokio::sync::Mutex::new(session),
            tabs: tokio::sync::Mutex::new(HashMap::new()),
        }),
    );
    Ok(dto)
}

/// Connects with the given settings and disconnects again.
#[tauri::command]
async fn connection_test(profile: SavedProfile, password: Option<String>) -> Result<ServerInfo, UiError> {
    let password = resolve_password(&profile, password)?;
    let session = Session::connect(&profile.to_connect_profile(password)).await?;
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
    open_workspace(&state, saved.to_connect_profile(password), name, saved.color.clone()).await
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>, workspace_id: u64) -> Result<(), UiError> {
    // Dropping the workspace closes its connections.
    state.workspaces.lock().unwrap().remove(&workspace_id);
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
    let all = split::split(&script);
    let statements: Vec<split::Statement> = match mode {
        RunMode::All => all,
        RunMode::At { offset } => {
            let byte_offset = split::utf16_to_byte_offset(&script, offset);
            split::statement_at(&all, byte_offset).cloned().into_iter().collect()
        }
    };

    let tab = workspace.tab(&tab_id).await?;
    let mut session = tab
        .session
        .try_lock()
        .map_err(|_| ui_error("busy", "A query is already running in this tab."))?;
    tab.cancel_requested.store(false, Ordering::Relaxed);

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

        let outcome = session
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

        ran += 1;
        if let Err(e) = &outcome {
            summary = e.to_string();
            failed = true;
            let _ = on_event.send(RunMsg::Failed {
                index,
                message: summary.clone(),
                elapsed_ms: elapsed(),
            });
        }
        // History is a convenience; failing to write it must not fail the query.
        let _ = state.history.append(HistoryEntry {
            at_ms: now_ms(),
            profile_name: workspace.name.clone(),
            sql: statement.text.clone(),
            duration_ms: elapsed(),
            ok: outcome.is_ok(),
            summary,
        });
        if failed {
            break;
        }
        if split::first_keyword(&statement.text) == "USE" {
            if let Ok(name) = session.current_database().await {
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
#[tauri::command]
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
#[tauri::command]
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
    for id in result_ids {
        results.remove(&id);
    }
}

// ---------- schema browser ----------

#[tauri::command]
async fn schema_list(state: State<'_, AppState>, workspace_id: u64) -> Result<Vec<String>, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility.lock().await;
    Ok(session.schemas().await?)
}

#[tauri::command]
async fn schema_objects(
    state: State<'_, AppState>,
    workspace_id: u64,
    schema: String,
) -> Result<SchemaObjects, UiError> {
    let workspace = state.workspace(workspace_id)?;
    let mut session = workspace.utility.lock().await;
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
    let mut session = workspace.utility.lock().await;
    Ok(session.table_columns(&schema, &table).await?)
}

// ---------- history ----------

#[tauri::command]
fn history_list(
    state: State<'_, AppState>,
    search: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<HistoryEntry>, UiError> {
    Ok(state.history.list(search.as_deref(), limit.unwrap_or(200).min(2000))?)
}

#[tauri::command]
fn history_clear(state: State<'_, AppState>) -> Result<(), UiError> {
    Ok(state.history.clear()?)
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
        profile.ca = Some(CaSource::Pem(demo.ca_pem.clone().into_bytes()));
        profile.allow_cleartext_auth = true;
        open_workspace(&state, profile, "Demo server".into(), None).await
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
        .setup(|app| {
            // SQLTOOL_DATA_DIR overrides where profiles and history are kept,
            // which also keeps test runs away from real settings.
            let data_dir = match std::env::var_os("SQLTOOL_DATA_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            app.manage(AppState::new(data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            profiles_list,
            profile_save,
            profile_delete,
            connection_test,
            connect,
            connect_demo,
            disconnect,
            close_tab,
            run,
            cancel,
            get_rows,
            get_cell,
            close_results,
            schema_list,
            schema_objects,
            table_columns,
            history_list,
            history_clear,
            selftest_config,
            selftest_report
        ])
        .run(tauri::generate_context!())
        .expect("error while running the application");
}
