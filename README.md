# SQL Tool

[![CI](https://github.com/wjhdfnmf/sql-tool/actions/workflows/ci.yml/badge.svg)](https://github.com/wjhdfnmf/sql-tool/actions/workflows/ci.yml)

A free, open-source desktop SQL client for MySQL and MariaDB, built to be light and fast.

**Status: early alpha.** It connects, runs queries, shows results and browses schemas. It has been tested on Windows against MariaDB 11.8 and MySQL 8.4 only. Expect rough edges, and do not rely on it for production work yet.

## What it does

- **Connections**: saved profiles, test connection, colour tags. Passwords go in the operating system keychain or are asked for each time; they are never written to a settings file.
- **Authentication**: `mysql_native_password`, `caching_sha2_password`, MariaDB `ed25519`, and the cleartext plugin (`mysql_clear_password`) that LDAP and PAM accounts need.
- **Encryption**: verified TLS, a pinned certificate (trust one server certificate by its fingerprint, with no CA file), or "encrypted, not verified" as an explicit choice. Cleartext authentication on an unverified connection requires a visible acknowledgement.
- **Editor**: tabs, MySQL and MariaDB syntax highlighting, run the statement at the cursor (Ctrl+Enter) or the whole script (Ctrl+Shift+Enter), `DELIMITER` support, stop a running statement. Open query tabs come back the next time the connection is opened. Right-click a table to write its name or a full SELECT into the editor; double-click a column in the information pane to insert its name.
- **Results**: rows appear while they stream, in a grid that stays smooth with millions of rows. Columns size themselves to their contents, full-value viewer, one tab per result set.
- **Selecting and copying**: drag, Shift+click or Shift+arrows to select cells; click a row number for a row, Ctrl+click a header for a column, Ctrl+A for everything. Ctrl+C copies in a form spreadsheets paste as cells; the right-click menu also copies as CSV, as SQL INSERT statements, or as a list for an `IN (...)` clause. The count, sum, average, smallest and largest of the selection show under the grid.
- **Row limit**: 1,000 to 100,000 rows per result, or none. At the limit the query is interrupted on the server.
- **Working with results**: sort and filter without re-running, export as CSV, TSV, JSON or SQL INSERT, and edit rows in place with changes reviewed as SQL and applied in one transaction.
- **Table designer**: create and alter tables in a tab of their own, next to the query tabs. Columns, indexes and foreign keys each have a section; index and key columns are picked from the table's columns. The SQL that will run is always in view, and nothing runs until Apply is clicked.
- **Safety**: read-only connections, and confirmation of risky statements on connections marked as production.
- **Schema browser**: schemas, tables, views, columns, routines, triggers and events, loaded on demand, with right-click actions. Clicking a table shows its columns and indexes in the information pane below the tree.
- **Schema changes**: create and alter schemas in a tab that shows the SQL first; create and alter views and routines from generated scripts.
- **Menus and toolbars**: menu bar, toolbars, open and save script files, explain the statement under the cursor, reformat a query.
- **History**: searchable. Statements that contain a password are not stored in full.

## Security defaults

- A server that asks for the password in clear text is refused unless that connection has cleartext authentication turned on.
- A new connection verifies the server certificate by default.
- `LOAD DATA LOCAL INFILE` requests from the server are refused, so a hostile server cannot read local files.
- The UI runs under a strict content security policy, with no remote content and no telemetry.

## Known limits

- The automated tests run against MariaDB 11.8 and MySQL 8.4. MySQL Enterprise 8.4.3 and 5.7.37 have been connected to and used by hand. Hands-on use has been on Windows; the Linux and macOS builds pass the automated tests but have had no hands-on use.
- Query history and remembered query tabs are kept on this computer without encryption. Statements that contain a password are left out of both.
- `sha256_password` accounts are not supported.
- A result keeps about 512 MB in memory and spills the rest to a temporary file, which is private and removed when the result closes, but not encrypted.
- No automatic reconnect and no commit/rollback controls yet.
- Installers are not code-signed, so Windows and macOS show a warning.

[PLAN.md](PLAN.md) has the design, the roadmap and the test results in detail. See [SECURITY.md](SECURITY.md) to report a security problem and [CONTRIBUTING.md](CONTRIBUTING.md) to build and test.

## Download

Installers are on the [releases page](../../releases). On Windows the app needs the Microsoft Edge WebView2 runtime, which Windows 10 and 11 normally already have.

Installers for Windows, Linux and macOS are also built for every commit; they are attached to the run on the [Actions page](../../actions/workflows/ci.yml) for 14 days.

## Build from source

You need [Rust](https://rustup.rs) and [Node.js](https://nodejs.org). On Linux, Tauri also needs the WebKitGTK development packages; see the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

Development, with live reload:

```bash
cd app && npm install && npm run tauri:dev
```

That build includes a "Demo server" on the connection list: a fake server inside the app, for trying the UI without a database.

Installers, as CI builds them:

```bash
cd app && npm install && npm run tauri build
```

They land in `target/release/bundle`.

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
| `vendor/mysql_async` | A patched copy of the database driver; see its `PATCHES.md` |

## Licence

[Apache-2.0](LICENSE).
