# SQL Tool

[![CI](https://github.com/wjhdfnmf/sql-tool/actions/workflows/ci.yml/badge.svg)](https://github.com/wjhdfnmf/sql-tool/actions/workflows/ci.yml)

A free, open-source desktop SQL client for MySQL and MariaDB, built to be light and fast.

**Status: early alpha.** It connects, runs queries, shows results and browses schemas. It has been tested on Windows against MariaDB 11.8 and MySQL 8.4 only. Expect rough edges, and do not rely on it for production work yet.

## What it does

- **Connections**: saved profiles shown as cards, two to a row, each with its name and user@host; test connection, colour tags. Drag a card to move it, or to the bottom of the page to start a named group; groups can be renamed, folded away, moved and dissolved, and everything dragging does is on each card's menu too. A setting turns organising off. Passwords go in the operating system keychain or are asked for each time; they are never written to a settings file.
- **Import from MySQL Workbench**: Settings has an Import button that reads the connections Workbench has saved on this computer (or a `connections.xml` you choose), lists them with what each would be here, and brings over the ones you tick. Standard TCP/IP and LDAP user/password connections come over, with host, port, user, default schema, the SSL choice and the CA file. Connections through an SSH tunnel, a socket or Kerberos are listed but cannot be brought over. Passwords are not imported: each connection asks for its password the first time it is used.
- **Authentication**: `mysql_native_password`, `caching_sha2_password`, MariaDB `ed25519`, and the cleartext plugin (`mysql_clear_password`) that LDAP and PAM accounts need.
- **Encryption**: verified TLS, a pinned certificate (trust one server certificate by its fingerprint, with no CA file), or "encrypted, not verified" as an explicit choice. Cleartext authentication on an unverified connection requires a visible acknowledgement.
- **Editor**: tabs, MySQL and MariaDB syntax highlighting, run the statement at the cursor (Ctrl+Enter) or the whole script (Ctrl+Shift+Enter), `DELIMITER` support, stop a running statement. Open query tabs come back the next time the connection is opened, each still attached to the file it was opened from, so Save writes back to that file. Each connection can have a script folder, where Open and Save As start. Right-click a table to write its name or a full SELECT into the editor; double-click a column in the information pane to insert its name.
- **Long queries in time slices**: a query over a long stretch of time can outlast a server's or a proxy's patience. Mark where its time range goes with `{start}` and `{end}` (`WHERE completed >= {start} AND completed < {end}`), press **In Slices**, and give the range and the length of a slice. The query is run once for each slice and the rows are put end to end, in order, as one result. A small bar in the bottom row shows how many slices are in and how long the rest should take. Only a plain read can be sliced.
- **Reformat**: reformats what is selected and leaves everything around it as it was, indent and line breaks included; with nothing selected, the whole text. Four layouts chosen in Settings (standard, compact, aligned, one line), with keywords in upper case, lower case or as typed.
- **Results**: rows appear while they stream, in a grid that stays smooth with millions of rows. Columns size themselves to their contents, full-value viewer, one tab per result set.
- **Selecting and copying**: drag, Shift+click or Shift+arrows to select cells; click a row number for a row, Ctrl+click a header for a column, Ctrl+A for everything. Ctrl+C copies in a form spreadsheets paste as cells; the right-click menu also copies as CSV, as SQL INSERT statements, or as a list for an `IN (...)` clause. The count, sum, average, smallest and largest of the selection show under the grid.
- **Row limit**: 1,000 to 100,000 rows per result, or none. At the limit the query is interrupted on the server.
- **Working with results**: sort and filter without re-running, export as CSV, TSV, JSON or SQL INSERT, and edit rows in place with changes reviewed as SQL and applied in one transaction.
- **Exporting everything**: a result that stopped at the row limit holds only the rows up to it. Export then asks whether to write every row the query returns: the query is run again and each row goes straight to the file, so millions of rows export without being held in memory, with a running count and a Stop button.
- **Graphs**: the Graph button beside the menu button switches from the query to a graph of the result in front, once it has finished. Columns go on the X axis, the Y axis or the colour, by dragging or from a list. The chart is worked out from every row of the result, however many.
  - *Kinds:* bar and line charts (a count, sum, average, smallest or largest per value), pie, Pareto (largest first, with the running total), histogram, box plot (median, quartiles, whiskers, outliers), scatter plot and control chart. Time on X is cut into seconds up to years; numbers are binned at round values. A scatter plot of more than 50,000 points is drawn as shading by count, or as a sample.
  - *Pareto:* bars from the largest down with the running total on a percent scale of its own, a mark (80% unless changed) with the bars it takes to reach it picked out as the vital few, values on the bars, and rows with no value left out if wanted.
  - *Control chart:* the rows in order (one, or a chosen number, to a point) or the averages by lot, batch or time, with a centre line and three-sigma limits; the eight run rules, any of which can mark a point, with what each found; the 1 and 2 sigma lines; a second chart underneath of the spread (moving range, or standard deviation within each point); and a centre and sigma given from before to judge new data against.
  - *For process work:* Lines drawn at a value can be marked as specification limits or a target; a histogram or control chart then shows Cp, Cpk, Pp, Ppk and the share of rows outside. A normal curve can be laid over a histogram.
  - *Lines and trend lines:* right-click an axis or the chart to draw a line at a value, with a name, a style and a colour. A line belongs to the column it was drawn for, so a limit set on a histogram is there on the control chart of the same column. Right-click the chart for a trend line: straight, polynomial, exponential, logarithmic, power, or a moving average, with its equation and R². The trend line of a scatter plot is fitted to every row.
  - *The rest:* filters by range or by ticked values, dragging across a chart to zoom to a range of X or of time, a title and axis names (right-click, or the options), a legend, and a picture of the chart copied or saved as a PNG.
- **Table designer**: create and alter tables in a tab of their own, next to the query tabs. Columns, indexes and foreign keys each have a section; index and key columns are picked from the table's columns. The SQL that will run is always in view, and nothing runs until Apply is clicked.
- **Safety**: read-only connections, and confirmation of risky statements on connections marked as production.
- **Schema browser**: schemas with their tables, views, stored procedures and functions, loaded on demand; every group has a + to add one. Filter by table, or by `schema.table`. Clicking a table shows its columns and indexes in the object info pane below, which can be dragged taller or shorter; the whole navigator can be dragged wider or narrower. Right-click for everything else.
- **Schema changes**: create and alter schemas in a tab that shows the SQL first; create and alter views and routines from generated scripts.
- **Layout**: no menu bar. Navigator, Output and one menu button (history, connection info, create, server information, settings, help) sit at the end of the row of connection tabs. Query tabs stay on one row, with first/previous/next/last arrows and a list of all open tabs; a tab with a query running is coloured. Tabs run independently, so several can run at once. The output appears when a tab first has something to show, and its filter, export and copy controls share the row of result tabs.
- **History**: searchable. Statements that contain a password are not stored in full.
- **Timeouts and dropped connections**: connecting gives up after 15 seconds (adjustable). A query runs until it finishes or is stopped, unless a query timeout is set, in which case it is interrupted on the server. A connection the server has closed for sitting idle is reopened before the next statement is sent, with a note in the output; an optional keep-alive prevents the closing.
- **Settings and themes**: one window with a tab for each area (Appearance, Editor, Queries, Connections, Privacy), a mark on a tab where something differs from the defaults, and a button that restores one tab's defaults; sixteen themes, soft to bold, light and dark, each checked for readable contrast in the editor and grid (`python tools/themes.py . --check`); size of text; editor indent and wrapping; default row limit; timeouts; whether history and open tabs are kept.
- **Default schema**: one per connection, used by every query tab. Choosing one writes it into the connection's settings, so the settings, the schema tree and the next connect all agree. If that schema has since been dropped, the connection still opens, with no default.

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
- A graph is of one result as it was loaded: it does not run the query again, and a result that stopped at the row limit is charted up to the limit. A graph's setup is kept while its tab is open, not between runs of the app. Graphs do not yet split into a grid of small charts by a column.
- A query run in time slices is not one statement: the slices do not see the database at one instant, and rows that change while it runs may be seen by one slice and not by its neighbour. Times are sent as typed, with no time zone.
- No commit/rollback controls yet. When a dropped connection is reopened, session variables, temporary tables and an open transaction are not carried over; the output says so.
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

How long charting six million rows takes (no database needed; it also checks the answers):

```bash
cargo test -p sqltool-core --release --test chart_speed -- --ignored --nocapture
```

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

[Apache-2.0](LICENSE). The libraries the app is built with, and their licences, are listed in [THIRD-PARTY.md](THIRD-PARTY.md).
