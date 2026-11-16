# SQL Tool

[![CI](https://github.com/wjhdfnmf/sql-tool/actions/workflows/ci.yml/badge.svg)](https://github.com/wjhdfnmf/sql-tool/actions/workflows/ci.yml)

A free, open-source desktop SQL client for MySQL and MariaDB, built to be light and fast.

**Status: early alpha.** It connects, runs queries, shows results and browses schemas. It has been tested on Windows against MariaDB 11.8 and MySQL 8.4 only. Expect rough edges, and do not rely on it for production work yet.

## What it does

- **Connections**: saved profiles, test connection, colour tags. Passwords go in the operating system keychain or are asked for each time; they are never written to a settings file.
- **Authentication**: `mysql_native_password`, `caching_sha2_password`, MariaDB `ed25519`, and the cleartext plugin (`mysql_clear_password`) that LDAP and PAM accounts need.
- **Encryption**: verified TLS, or "encrypted, not verified" as an explicit choice. Cleartext authentication on an unverified connection requires a visible acknowledgement.
- **Editor**: tabs, MySQL and MariaDB syntax highlighting, run the statement at the cursor (Ctrl+Enter) or the whole script (Ctrl+Shift+Enter), `DELIMITER` support, stop a running statement.
- **Results**: rows appear while they stream, in a grid that stays smooth with millions of rows. Resizable columns, full-value viewer, one tab per result set.
- **Row limit**: 1,000 to 100,000 rows per result, or none. At the limit the query is interrupted on the server.
- **Working with results**: sort and filter without re-running, export as CSV, TSV, JSON or SQL INSERT, and edit rows in place with changes reviewed as SQL and applied in one transaction.
- **Table designer**: create and alter tables in a form that shows the SQL before it runs.
- **Safety**: read-only connections, and confirmation of risky statements on connections marked as production.
- **Schema browser**: schemas, tables, views, columns, routines, triggers and events, loaded on demand, with right-click actions and an information pane for the selected object.
- **Schema changes**: create and alter schemas in a dialog that shows the SQL first; create and alter tables, views and routines from generated scripts.
- **Menus and toolbars**: menu bar, toolbars, open and save script files, explain the statement under the cursor, reformat a query.
- **History**: searchable. Statements that contain a password are not stored in full.

## Security defaults

- A server that asks for the password in clear text is refused unless that connection has cleartext authentication turned on.
- A new connection verifies the server certificate by default.
- `LOAD DATA LOCAL INFILE` requests from the server are refused, so a hostile server cannot read local files.
- The UI runs under a strict content security policy, with no remote content and no telemetry.

## Known limits

- Tested only on Windows, and only against MariaDB 11.8 and MySQL 8.4. MySQL 5.7, MySQL Enterprise, Linux and macOS are untested.
- "Verify CA" mode currently behaves like full verification, because of a defect in the database driver. Servers using MySQL's auto-generated certificates can only be reached with "encrypted, not verified".
- `sha256_password` accounts are not supported.
- Results are held in memory. With the row limit set to "none", a very large table can exhaust it.
- No automatic reconnect and no commit/rollback controls yet.
- Release binaries are not code-signed, so Windows shows a warning when you run one.

[PLAN.md](PLAN.md) has the design, the roadmap and the test results in detail.

## Download

Pre-built Windows binaries are on the [releases page](../../releases). They need the Microsoft Edge WebView2 runtime, which Windows 10 and 11 normally already have.

Development builds include a "Demo server" on the connection list: a fake server inside the app, for trying the UI without a database.

## Build from source

You need [Rust](https://rustup.rs) and [Node.js](https://nodejs.org). On Linux, Tauri also needs the WebKitGTK development packages; see the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

Development, with live reload:

```bash
cd app && npm install && npm run tauri dev
```

A release build:

```bash
cd app && npm install && npm run build && cd .. && cargo build --release -p sqltool-app --features custom-protocol
```

The binary is `target/release/sqltool-app` (`.exe` on Windows).

## Tests

```bash
cargo test --workspace
```

This runs the unit tests and the login, TLS and query tests against a fake MySQL server that ships in this repository. No database is needed.

Tests against real servers, and the app's end-to-end run, use local test servers set up by the scripts in [tests/servers](tests/servers) (Linux or WSL):

```bash
cargo test -p sqltool-core --test real_servers -- --ignored
```

```bash
bash tests/servers/selftest.sh mariadb
```

## Layout

| Path | What it is |
|---|---|
| `crates/core` | Connections, security policy, running statements, result storage, metadata. No UI code |
| `crates/secrets` | Keychain storage for passwords |
| `crates/fakeserver` | A minimal fake MySQL server for tests |
| `crates/cli` | `sqltool-smoke`, a command-line connection check |
| `app/src-tauri` | The desktop shell: a thin command layer over the core |
| `app/src` | The user interface (Svelte and TypeScript) |
| `tests/servers` | Scripts that set up local MariaDB and MySQL test servers |

## Licence

[Apache-2.0](LICENSE).
