# Project plan: open-source SQL client for MySQL and MariaDB

Status: Phases 0 to 2 done; Phase 3 partly done (see "Phase 3 status"); 0.1.0 released 2026-11-16, followed by a pass over everyday use and a code review (see "Everyday-use pass"). Project name is still to be chosen.

## Release-quality pass over all three systems (after 0.3.0)

A review of the whole app for completeness, error handling and release quality on Windows, Linux and macOS, then the fixes for what would stop it being used on one of them and for errors that were swallowed.

### What stood in the way on Linux and macOS

- **The Linux packages were built on the newest Ubuntu**, so they needed its C library and would not start on Ubuntu 22.04 or Debian 12. They are built on Ubuntu 22.04 now, the oldest there is a runner for.
- **macOS had an Apple silicon build only.** There is one universal app now, for Apple silicon and Intel.
- **macOS shortcuts.** Explain (Cmd+Option+X) could never fire: Option changes the letter a key gives, so the key is now told by its position. Cmd+W belonged to the system's "Close Window", which on a one-window app is quitting; the menu bar is now the app's own, without that item, so Cmd+W closes the query tab as Ctrl+W does elsewhere, and Quit asks before losing work. Shortcuts in tooltips, menus and Help are spelled Cmd and Option on a Mac.
- **A Linux desktop with no keyring** made saving or deleting any connection report a keychain error, even with "save password" off, and refused to connect to one with a saved password. Now the connection is saved and a notice says what became of the password; connecting asks for the password and says why.
- **The licences of what the app is built with were only in the repository.** They are collected when the interface is built (`tools/licences.mjs`: 29 npm packages, 530 crates, 278 distinct texts) and shown under Help.
- **Nothing ever started the app on Linux or macOS.** `tests/smoke.sh` starts it against the server built into development builds and lets it check itself: settings, connect over TLS, a query through the real interface with the grid painted, copy, export, the licences, no blocked content and no unhandled error. CI runs it on all three systems after building the installers.

### Errors that were swallowed

- **An error nothing handled showed nothing.** It is now shown along the bottom of the window and noted in `crash.log` (the kind and the place in the code, not what a server said).
- **Closing the window asked nothing.** It asks first while a statement is running, while changes in a result or a designer are not applied, or while a script has unsaved changes that would not be remembered. A second request while the question is up closes, so an interface that has stopped answering cannot hold the window open.
- **An export of loaded rows that failed part-way left the partial file.** It is removed, as the full-query export already did.
- **A script saved with a byte-order mark** kept the mark, and its first statement failed on the server. The mark is dropped; UTF-16 files with a mark are read too.
- **A failure to start was silent.** The reason goes to `crash.log` and into a message box.
- **Two copies of the app** could overwrite each other's settings. A second start now brings the first to the front.

### How it was verified

- `cargo test --workspace`: 169 core tests (one new, for reading script files) and 26 connection tests pass. `npm run check`: no errors.
- `tests/smoke.sh` on Windows: 10 of 10 steps, the exported file counted, `crash.log` empty.
- The desktop self-test against MariaDB 11.8 and MySQL 8.4 on Windows: 41 of 41 steps on each. Two are new: asking the window to close while `SELECT SLEEP(8)` runs brings up the question, and "Keep Open" keeps it open; and no error went unhandled during the whole run.
- Starting the app twice on Windows: the second start exits at once and the first stays.
- In the preview: Help with the licences and the About section, shortcut labels, the notice for an unhandled error.

### Not verified

- **Anything on Linux or macOS by hand.** The macOS menu bar, Cmd+W and Cmd+Option+X, the universal build, and the Linux packages on Ubuntu 22.04 are reasoned from the code and checked only as far as CI builds them and the smoke test runs. The macOS menu cannot even be compiled into a running app on Windows; it is type-checked there.
- The keyring-less Linux case: the code paths were read, not run on such a system.
- The message box for a failure to start, and the notice after a save without a keychain: not seen.
- "Close Anyway", which exits the app, by the self-test: it would end the test.

## Graph builder (after 0.1.6)

Asked for: a graphing tool in the manner of JMP's Graph Builder, reached from a button beside the menu button, that takes over the window from the query; columns placed on axes; bar charts, histograms and scatter plots; time, discrete and continuous X; filters, a legend and time range selection.

**How it works.** The interface holds none of a result's rows, so a chart cannot be drawn from rows. `crates/core/src/chart.rs` reads the columns in use out of the result store once (as `f64`, or as category codes with up to 5,000 names) and keeps them while the graph is open; every change of axis, filter or range is then a pass over those arrays. It returns only what is drawn: a value per bar and series, a count per bin, the points of a scatter plot up to 50,000, and past that a 160 by 100 grid of counts or a sample chosen by row number. Kept columns are counted against the shared result memory budget and released when the graph is closed.

**Measured** on six million rows, 490 MB, most of it spilled to disk (`cargo test -p sqltool-core --release --test chart_speed -- --ignored --nocapture`, and the same through the desktop app's self-test against MariaDB): first use of a column 0.13 to 0.6 s depending on its width and kind; each chart after that 20 to 130 ms.

**Choices.**
- *Chart library:* Apache ECharts 6.1, Apache-2.0 like this project. Only the series and parts in use (bar, line, pie, scatter, box plot and custom series; grid, legend, title, tooltip, mark lines; the canvas renderer): 608 KB (205 KB compressed) in a file of its own, fetched when a graph is first opened. It runs under the existing content security policy unchanged: the self-test in the desktop webview draws a chart and reports no violation.
- *Tooltips* are elements built with `textContent`, not the library's HTML strings, so a value from the database is never read as markup.
- *Binned bars* (histograms, time on X) are drawn by a custom series on a real number or time axis, so bins sit where their edges are; category bars use the library's bar series.
- *Time* is milliseconds since 1970 with no time zone, parsed from MySQL's text by hand (zero dates are missing values; `TIME` values are durations) and shown through the UTC accessors, so what is drawn is what the server sent.
- *Colours:* eight series colours in a fixed order, one set for light backgrounds and one for dark, chosen for separation under colour-vision deficiency; a group keeps its colour as filters change; a ninth group goes to "Other" in grey. `tools/themes.py --check` checks them against every theme.
- *Dragging columns* uses pointer events, not HTML drag and drop, which Tauri's window intercepts on Windows. Every zone is also a list, so nothing needs dragging.
- *The graph's filters are its own.* The grid's sort and filter do not affect it.

**Second pass: charts for process work.** Asked for: a pie chart of counts, lines on an axis from a right-click popup (value, style, optional name), a box plot "and other common statistical visualizations", a trend line from a right-click popup, renaming axes and a title, and the tools a process engineer in manufacturing expects.

- *Kinds added:* line, pie, Pareto, box plot and control chart, beside bar, histogram and scatter.
- *Box plot:* quartiles by the in-between rule (R's type 7, Excel's `QUARTILE.INC`), whiskers to the furthest value within 1.5 box-heights, outliers thinned to 200 a box with the extremes kept and the true count reported. Every value is sorted in the core; six million take under 0.1 s.
- *Control chart:* with nothing on X the rows are charted in order as individuals, sigma from the average moving range over 1.128; with a lot, batch or time on X the points are subgroup averages, sigma pooled from the spread within subgroups, limits at three sigma over the square root of each subgroup's size. More than 2,000 rows in order are averaged in consecutive runs. Points outside their limits are marked and counted. The run rules (Western Electric, Nelson) are not applied.
- *Capability:* the core reports the measured column's count, mean, overall and short-term standard deviation, and the rows below and above the limits it is given. Cp and Cpk use the short-term figure, Pp and Ppk the overall one; with one limit only the one-sided index is given.
- *Lines:* a line is filed under what it measures (a column's values, its sum, or a count of rows), not under an axis, so a specification limit drawn on a histogram of a column appears on that column's control chart and on a scatter plot with it on either axis, and not on a chart of something else. Axes widen to keep a line in view.
- *Trend lines:* least squares on x and y, or their logarithms, with x brought into -1..1 and y centred first, so times in milliseconds and millions of rows keep their precision; checked against curves with known answers. A scatter plot's is fitted in the core from every row, including when the points are shaded or sampled; charts with few points are fitted in the interface by the same method.
- *Pareto:* the left axis ends at the total and the right axis at 100%, so the running-total line reads correctly on both: one scale with two sets of labels, not two scales.
- *Right-click:* the chart reports where a click landed (plot, X axis, Y axis, or on a drawn line) and the values under it; the menu and the popups are the app's own. Everything a right-click offers is also on a button beside the chart, for the keyboard.

**Measured** on six million rows through the desktop app against MariaDB: box plot 53 ms, control chart 56 ms, trend line 80 ms, once the column has been read.

**Not done:** small multiples (JMP's Group X, Group Y, Wrap); remembering a graph between runs of the app; clicking a bar to filter the grid; exporting the numbers behind a chart; control chart run rules, range and standard-deviation charts, attribute charts (p, np, c, u); normal quantile plots; confidence bands on trend lines.

**Not verified by hand:** copying a picture to the clipboard (Save image is exercised by the self-test); the graph builder on Linux and macOS.

## Import from MySQL Workbench (after 0.2.0)

Workbench keeps its connections in `connections.xml` in its data folder (`%APPDATA%\MySQL\Workbench` on Windows, `~/Library/Application Support/MySQL/Workbench` on macOS, `~/.mysql/workbench` on Linux): a list of `db.mgmt.Connection` objects, each with a driver, a name and a dictionary of settings. The names of the drivers and settings, and the numbering of the SSL choices, were taken from Workbench's own definition file (`mysql_rdbms_info.xml` in its source), not guessed.

`crates/core/src/workbench.rs` reads the file and turns each connection into a profile where it can:

- *Standard (TCP/IP)* and *LDAP User/Password* come over. The other four ways Workbench connects (socket or pipe, SSH tunnel, SASL/Kerberos, native Kerberos) are listed with the reason they cannot.
- *SSL:* No becomes not encrypted; If available and Require become encrypted, not verified (with a note for If available, which here never falls back to no encryption); Require and Verify CA, and Verify Identity, map across with the CA file. The LDAP connection numbers the same choices from nought, which the reader allows for. A number it does not know becomes verify identity, the safe side.
- *Clear-text sign-in* is carried over, and is on for the LDAP connection, which is clear text by nature. The acknowledgement of its risk is never set by the import.
- *Not carried over:* passwords (they are in the system vault, not the file, and the vault is not read), client certificates, SQL mode and other extra options. Each is said in the list.

The interface lists what was found, ticks everything that can come over except what is already here (same host, port and user), and saves the ticked ones through the same command as a connection typed by hand. A name already in use gets "(Workbench)" after it. The backend never saves anything on its own account, and the interface never names the file.

**Not verified:** against a `connections.xml` written by a real Workbench; this computer has none. The format is from Workbench's source and the tests use files built to it.

## Time slices, deeper Pareto and control charts (after 0.2.0)

Asked for, with a script of the user's own as the model: break a long query into time slices that are run one by one and put back together, so that none times out; run several at once as a hidden option; and show progress without taking room. Also a deeper Pareto chart and control chart, layouts for Reformat, a New button and a way to reset a graph.

**Time slices** (`crates/core/src/slices.rs`, `run_sliced` in the shell). The query marks its range with `{start}` and `{end}` (`{0}` and `{1}` are read too, as a script filling a query with `format` writes them). The range is cut into half-open slices; each slice's statement gets the times as quoted literals, written from parsed dates. Slices run on connections of their own, taken from a shared counter, so the tab's session is untouched. They finish in any order; a slice's rows are added to the one result only when every slice before it is in, so the result is in the order of the slices. The row limit counts all slices together. A failing slice stops the run and names itself; what came before it is kept. The query timeout applies to each slice, which is the point. Stop interrupts every connection. Only a plain read is accepted.

What the script did that this does differently: it sent the times unquoted; it used `BETWEEN`, which returns a row on a boundary twice (the dialog warns); it opened a new connection pool for every slice; a failed slice left a hole that broke the join at the end; and it kept logins in a text file.

**Threads** are a setting with no control until it is found (the Queries heading clicked five times, or the word typed in Settings): 1 to 16 connections. Verified in the desktop self-test on both servers: the same rows in the same order with one connection and with four.

**Control chart.** The eight run rules (Nelson's numbering; 1, 2, 5 and 6 are the Western Electric set) are worked out in the core for every point and returned as a bit mask; the interface chooses which ones mark a point. Limits can come from a centre and sigma given from before. A companion chart shows the spread: the moving range against 3.267 times its average for single rows, the standard deviation within each point against limits from c4 (taken as 4(n-1)/(4n-3)) for subgroups. Rows in order can be charted a chosen number to a point.

**Pareto chart.** The first version scaled the left axis to the total so that the running-total line could share it, which squashed the bars. Now the bars have the left axis to themselves and the line is read on a 0 to 100% axis on the right: two scales, but the second is a share of the first, as every Pareto chart is drawn.

**Not done:** attribute charts (p, np, c, u), range charts for small subgroups, phases with their own limits, capability from a non-normal fit; slicing by anything but time; resuming a sliced run that failed part-way.

## Reliability pass, settings and themes (after 0.1.2)

### How a query "timing out" is handled

Before this pass there were three gaps. They are closed as follows.

- **Connecting.** There was no limit of the tool's own; an address that swallowed packets held the dialog for as long as the operating system kept trying. Connecting (network, TLS and login together) now gives up after 15 seconds, a setting.
- **A query that runs long.** There is no time limit unless one is set, on purpose: a report may need an hour. Stop interrupts it on the server. A new setting, off by default, interrupts any statement that runs longer than a chosen number of seconds and does not run the rest of the script. A limit set on the server (`max_execution_time`, `max_statement_time`) shows as that statement's error and the connection stays usable.
- **A connection that has gone.** Servers close connections that sit idle (`wait_timeout`), and firewalls drop quiet ones. A tab whose connection had been closed failed on every run until the tab was closed, and so did the schema browser. Now a connection that has been quiet for 30 seconds is checked with a ping before use and reopened if it is gone. Nothing has been sent at that point, so a statement can never run twice. A statement that loses its connection midway is reported and not retried, because it may have taken effect; the next run reopens the connection. Read-only mode and the default schema are restored; session variables, temporary tables and an open transaction are not, and the output says so. The connection used for cancelling follows the new connection.
- **A connection cut without a word.** With no reply at all a query could wait for ever. TCP keep-alive probes (30 seconds) now turn that into an error.

### Other reliability findings fixed

- Closing a connection in the app left its running statements running on the server. They are now interrupted.
- Each result had its own memory allowance, so ten tabs of large results could use ten times it. The allowance is now shared by all open results.
- A damaged connections file stopped the home screen from loading. It is now set aside under another name, the list starts empty, and the app says what happened.
- A crash left no trace. The panic message and location are now written to `crash.log` in the data folder (never queries or data).

### Settings

Chosen by looking at what MySQL Workbench and DBeaver offer and keeping what bears on everyday querying:

| Setting | Default | Why |
|---|---|---|
| Theme | Follow the system | Light, dark, midnight, sepia, high contrast |
| Size of text and controls | 100% | Accessibility |
| Editor indent, line wrap, upper-case keywords | 2 spaces, off, on | Workbench and DBeaver both have these |
| Bring back open query tabs | On | Workbench saves a snapshot of open editors |
| Row limit for a new connection | 10,000 | Workbench "Limit Rows" |
| Stop a query after | Never | Workbench "read timeout", DBeaver query timeout |
| Give up connecting after | 15 s | Workbench "connection timeout" |
| Keep idle connections alive | Off | Workbench and DBeaver keep-alive interval |
| Memory for results | 512 MB | Was an environment variable |
| Keep a history | On | Privacy |

Considered and left out for now: CSV delimiter and quoting options, a "safe updates" mode for every connection (production connections already confirm risky statements), per-connection overrides of these settings, custom key bindings, and font choice.

### Security findings from this pass

- Settings read from disk are not trusted: every value is pulled into its allowed range, and the theme must be one of the known names.
- A reconnect uses the same TLS verification and policy as the first connection, and the password it uses is the one already held for the open connection; nothing new is stored.
- Recorded as a known limit, not changed: CSV exports write values as they are, so a spreadsheet may run a value beginning with `=` as a formula.
- Not changed: internal locks are not poison-tolerant, so a panic while one is held would make later calls fail until restart. No path to such a panic was found.

### How it was verified

- 106 unit tests, 25 fake-server tests (new: the connect timeout against a listener that never answers; the liveness check leaving a live session alone) and a test of the shared memory allowance.
- 7 real-server tests on MariaDB 11.8.6 and MySQL 8.4.13. New: a connection killed from another session is noticed and reopened, the default schema and read-only mode come back, and cancelling reaches the new connection.
- The desktop self-test, 31 steps on both servers. New: settings are stored and corrected; a 1-second query timeout stops a long query in about a second and the session stays usable; a killed connection is reported, then reopened on the next run.
- Themes and the settings screen were checked in the browser preview.

### Not verified

- An idle timeout as it really happens (hours of quiet, or a firewall dropping the connection); the tests kill the connection from another session instead.
- The keep-alive setting over a long idle period.
- The size setting (zoom) and the themes in the packaged app by eye; the self-test stores and reads the theme but does not look at the window.
- LDAP accounts on reconnect; the tests reconnect with the PAM cleartext account on MariaDB.

## Everyday-use pass and code review (after 0.1.0)

The remaining Phase 3 and 4 items (SSH tunnels, client certificates, user management, dump and restore, the wider version matrix) are parked: the project owner does not use them. This pass went instead into what is done all day: writing queries, reading results, copying and exporting them, and creating tables.

### Added

- **Selecting cells.** A rectangle of cells by dragging, Shift+click or Shift+arrows; whole rows from the row numbers; whole columns with Ctrl+click on a header; everything with Ctrl+A or the corner. Dragging past an edge scrolls.
- **Copying.** Ctrl+C copies the selection as tab-separated text, which spreadsheets paste as cells; Ctrl+Shift+C adds the column names. The right-click menu also copies as CSV, as SQL INSERT statements, and as a quoted list for an `IN (...)` clause. One cell copies as exactly its text. The text is built in the core from the stored rows, so a selection of a million rows copies what the server sent, not what is on screen.
- **Totals.** The count, sum, average, smallest and largest of the selected cells show under the grid.
- **Columns fit their contents** when the first rows arrive, and again on a double-click of a header's edge or from the header's right-click menu, which also sorts and selects.
- **Filter by a cell's value** from the right-click menu; **delete several rows** at once.
- **Exports say when they are partial**: when the result stopped at the row limit, or a filter was on.
- **Query tabs are remembered** per saved connection and come back when it is opened again.
- **From the schema tree into the editor**: a table's name, a SELECT naming every column, or (double-click) a column's name.
- **Table designer**: foreign keys (create, change, remove); move columns up and down; Enter goes to the next column and adds one after the last. Enter no longer applies the change: only the Apply button runs SQL.

### Review findings fixed

Efficiency:

- Sorting read each row again on every comparison. It now reads each row's key once.
- A batch of very large rows could grow without limit before reaching the result store. It is now handed over at 8 MB.
- Every cancel and every row-limit stop logged in to the server again. One control connection is now kept and reused, which matters most for LDAP accounts.
- Connecting asked for the TLS version and cipher in two round trips; now one.
- Sorting a large result, building the edit preview, listing the history and looking up keychain entries ran on the thread that keeps the window responsive. They no longer do.
- Opening a new tab's connection held a lock that cancelling in another tab needed.
- The grid built a lookup key for every visible cell on every frame, even with no edits pending.
- The history file was opened and closed for every statement. It is now kept open.
- A script of many thousands of statements grew the message list without limit. It now keeps the latest thousand.
- The editor copied the whole script out on every keystroke. It now does so shortly after typing stops.
- Completion read every column of a schema with no upper bound; now at most 50,000.

Security:

- On Linux and macOS the history and saved-connection files were created readable by other accounts on the computer. They are now readable by their owner only, including files written by 0.1.0.
- The connection dialog now says plainly that "Encrypted, not verified" lets an impersonating server read the password, and points to pinning.

### Review findings recorded, not changed

These are listed in [SECURITY.md](SECURITY.md) under "Known limits": the password of any account is exposed to an impersonating server on an unverified connection; history, remembered tabs and spilled results are not encrypted on disk; exported INSERT statements assume the default SQL mode; the driver keeps the password in memory and is an unreleased build; installers are unsigned. The workflow files pin GitHub's own actions by major version rather than by commit.

Nothing was found that lets database content run as code in the UI: values are drawn on a canvas or inserted as text, never as HTML, and the content security policy allows no inline or remote script.

### How it was verified

- 102 unit tests and 23 fake-server tests in the core.
- The five real-server tests, on MariaDB 11.8.6 and MySQL 8.4.13, including foreign keys through the designer (read, change a rule, add a key on a new column, remove a column with its key) and the reused control connection.
- The desktop self-test on both servers, with new steps for copying, totals, foreign-key SQL and selecting in the grid.

### Not verified

- Copying to the clipboard from the packaged app by hand (the self-test checks the text the core produces, not the clipboard).
- The owner-only file permissions on a real Linux or macOS machine; the test for them runs in CI on those systems.

## Phase 3 status

Hardening and release work. Part of it is done; the rest needs decisions or accounts that only the project owner has.

### Done

| Area | State |
|---|---|
| Certificate pinning | A new encryption mode. The app fetches the server's certificate (sending no credentials), shows its subject, issuer, expiry and fingerprint, and connects only to that exact certificate from then on. No CA file is needed, and cleartext authentication is allowed without the risk override |
| Verify CA | Works. It was broken in the published driver |
| Patched driver | `vendor/mysql_async` is the upstream driver with three marked changes, listed in its `PATCHES.md` |
| Memory limit | A result keeps about 512 MB in memory (`SQLTOOL_RESULT_MEMORY_MB` changes it); the rest goes to an anonymous temporary file and is read back through a memory map. Sorting, filtering, export and editing work the same either way |
| Installers | CI builds an NSIS installer on Windows, `.deb`, `.rpm` and AppImage on Linux, and a `.dmg` on macOS, on every push and pull request |
| Release pipeline | Pushing a `v*` tag builds the installers on clean CI machines and attaches them with checksums to a draft release. Nothing is public until the draft is published by hand |
| Release contents | The demo server and self-test are now a development-only feature and are not in release builds |
| Dependency audit | Weekly, and on demand: `cargo audit` and `npm audit` |
| Project documents | `SECURITY.md` and `CONTRIBUTING.md` |

### How it was verified

| Check | Result |
|---|---|
| Core unit tests | 87 pass, including spill to disk with a tiny memory budget |
| Login and TLS tests against the fake server | 23 pass, none switched off. New: the probe sends no credentials; a pinned certificate allows cleartext login with no CA; a different certificate receives no password |
| Core against real MariaDB 11.8.6 and MySQL 8.4.13, on the patched driver | 5 of 5 pass |
| Desktop app end to end, on each real server | 28 of 28 steps pass, including fetching the certificate, connecting with it pinned, and being refused with a wrong pin |

### Not verified

- **The installers themselves.** They are built by CI; none has been installed and run by a person.
- **The release workflow.** It runs only when a tag is pushed, and no tag has been pushed since it was written.
- **The pinning screen** has not been clicked through by hand in the built app.

### Not done

| Item | What it needs |
|---|---|
| Code signing | A Windows certificate and an Apple Developer account. Until then installers show operating system warnings |
| Auto-update | A signing key for update manifests, stored as a repository secret |
| SSH tunnels | Not started |
| Client certificates, socket and named-pipe connections | Not started |
| Automatic reconnect; commit and rollback controls | Not started |
| Accessibility pass, user documentation | Not started |
| Performance benchmarks in CI | Not started; the Phase 0 benchmark was a one-off |
| Lint and formatting gates in CI | Not started |
| External security review | Needs a reviewer |

## Phase 2 status

Schema and data tools are built. MySQL Enterprise 8.4.3 and 5.7.37 have both been connected to by the project owner with the released build (cleartext authentication over unverified TLS).

### What works

| Area | State |
|---|---|
| Completion | Schema, table and column names in the editor, for the default schema and any schema expanded in the tree |
| Table designer | Create and alter tables in a form: columns, types, primary key, not null, auto increment, defaults, comments, indexes, engine, collation. Shows the SQL before it runs. Altering writes only the differences |
| Editing data | In results that come from one table and include its primary key: edit a cell, set NULL, add a row, delete a row. Changes are staged, shown as SQL, and applied in one transaction |
| Export | The rows shown, as CSV, TSV, JSON or SQL INSERT, written straight from the result store |
| Sort and filter | Click a column header to sort; type to filter. Done on the stored result without running the query again |
| Copy | Cell or whole row from the right-click menu |
| Read-only connections | Changes are refused in the tool before they are sent, and the session is also put in read-only mode on the server |
| Production connections | Structure changes, and UPDATE or DELETE without WHERE, ask for confirmation first |
| Drop, truncate, rename | From the right-click menu, each with a confirmation that shows the SQL. Dropping a schema asks for its name to be typed |
| Table details | Indexes and foreign keys in the information pane |

### How it was verified

| Check | Result |
|---|---|
| Core unit tests | 81 pass |
| Login, TLS and run tests against the fake server | 17 pass, 1 known driver defect switched off |
| Core against real MariaDB 11.8.6 and MySQL 8.4.13 | 5 of 5 pass, including edits that roll back on failure and a table that is created, altered, read back and rebuilt identically |
| Desktop app end to end, on each real server | 27 of 27 steps pass |
| UI type check | 0 errors, 0 warnings |

### Not done or not verified

- **The new screens have not been clicked through by hand in the built app.** The table designer, the edit dialogs and the confirmation dialogs were type-checked and their backend calls pass the end-to-end run; they were only partly looked at in the browser preview.
- **Export to a file** uses the system save dialog, which the automated run cannot drive. The export formats themselves are unit tested.
- **Table designer limits**: foreign keys, checks, generated columns and partitioning are not editable and are left untouched. Columns cannot be reordered. Per-column character sets are preserved but not shown.
- **Editing limits**: binary columns and values too large to load in full cannot be edited. Tables without a primary key are read-only in the grid.
- **Sorting and filtering are disabled while edits are pending**, because staged edits refer to rows by position.
- **Completion** covers names only; it does not understand aliases.

## Phase 1 status

The desktop app now connects, runs queries and browses schemas. It replaces the Phase 0 prototype; the prototype's grid was carried over, its benchmark mode was not.

### What works

| Area | State |
|---|---|
| Connection manager | Saved profiles (no secrets in the file), create, edit, delete, test connection, colour tags |
| Passwords | Stored in the OS keychain, or asked for at connect time |
| Encryption choices | Verified, CA verified, "Encrypted, not verified", "Not encrypted". Cleartext authentication on an unverified connection needs a visible acknowledgement in the dialog |
| Editor | Tabs, MySQL and MariaDB syntax highlighting, keyword completion, search, undo history |
| Running | Statement at the cursor or the selection (Ctrl+Enter), whole script (Ctrl+Shift+Enter), `DELIMITER` support, stops at the first error, stop button |
| Sessions | Each editor tab has its own connection; the schema browser uses another, so it stays responsive during long queries |
| Results | One tab per result set, rows shown while they stream, resizable columns, keyboard navigation, copy a cell, full-value viewer with JSON formatting, messages with timings |
| Row limit | 1,000 / 10,000 / 100,000 / none per result. At the limit the query is interrupted on the server and the result is marked as cut short |
| Schema browser | Schemas, tables, views, columns, routines, triggers, events; loaded on demand; filter box; double-click a table to see its rows |
| History | Every statement with time, duration and outcome; searchable; statements containing a password are not stored in full |
| Multiple connections | Several servers open at once, one tab each |
| Menus and toolbars | Menu bar (File, Edit, View, Query, Database, Server, Help), a main toolbar, an editor toolbar, and buttons to show or hide the navigator and the output |
| Navigator | Schemas, Administration and History tabs, with an information pane below (Object info, Session) |
| Right-click menus | On schemas, tables, views, routines, triggers and events: set default schema, select rows, create, alter, copy name, refresh |
| Schema changes | Create and alter schema in a dialog that shows the SQL before it runs. Create table, view, procedure and function start from a template script. Alter opens a script built from the object's current definition |
| Script files | Open and save `.sql` files through the system file dialogs; the UI can only write to a file the user picked |
| Explain and reformat | Execution plan of the statement under the cursor; reformat the selection or the whole script |
| Connection errors | Test connection explains certificate and cleartext errors and offers the fix in one click |

### How it was verified

| Check | Result |
|---|---|
| Core unit tests (splitter, store, profiles, history, version detection) | 40 pass |
| Login, TLS and run tests against the fake server | 17 pass, 1 known driver defect switched off |
| Core against real MariaDB 11.8.6 and MySQL 8.4.13: metadata, row changes, binary and non-ASCII data, a procedure returning two result sets, the row limit interrupting a 20-million-row query | 3 of 3 pass |
| Desktop app end to end, driven through the same calls the screens make, against MariaDB (PAM cleartext over verified TLS) and MySQL 8.4 | 19 of 19 steps pass on each |
| Screens, in a browser against a mock backend, light and dark | Looked at: connection list, connection dialog, editor, results, messages, schema tree, cell viewer |
| UI type check | 0 errors, 0 warnings |

The end-to-end run covers the keychain round trip, wrong-password handling, the binary row transport, cancel, `USE` tracking, history, that the editor and grid paint inside the real webview, and that nothing is blocked by the content security policy. It also confirms the password is in no settings or history file.

### Not done or not verified

- **Nobody has clicked through the packaged app by hand.** The screens were checked in a browser with a mock backend, and the real app was checked by the automated run. Column resizing, keyboard navigation in the grid and the history panel were not exercised in the real app.
- **Open and save script files**: built, but not exercised. The system file dialogs cannot be driven by the automated run.
- **Alter table, view and routine** open a script to edit; there is no visual designer yet. On MySQL, altering a routine, trigger or event has to drop it and create it again, and the script says so at the top. On MariaDB it is replaced in one statement.
- **Tools and Scripting menus** are left out until there is something to put in them. The Administration items are shortcuts that run a statement in a query tab, not dashboards.
- **Your Enterprise servers, MySQL 5.7, Linux and macOS**: not tested by me. The CI workflow builds and runs the fake-server tests on Linux and macOS.
- **Continuous integration** and the container matrix: not set up. There is no remote repository yet and no container runtime on this machine.
- **Verify CA and certificate pinning**: done in Phase 3.
- **Memory**: done in Phase 3; large results spill to disk.
- **Lost connections**: a dropped connection surfaces as an error on the next statement. There is no automatic reconnect.
- **Transactions**: sessions run in the server's default autocommit mode. There is no commit/rollback control or open-transaction indicator yet.
- Installer packaging, signing and auto-update are Phase 3.

Left for Phase 2 as planned: completion of table and column names, the table designer, data editing, export.

### How to run it

Development, with live reload (needs Node and Rust):

```bash
cd app && npm install && npm run tauri dev
```

A release build without the Tauri CLI:

```bash
cd app && npm run build && cd .. && cargo build --release -p sqltool-app --features custom-protocol
```

The UI alone in a browser, against a mock backend:

```bash
cd app && npm run dev
```

The end-to-end run against the local test servers (see `tests/servers/`):

```bash
bash tests/servers/selftest.sh mariadb
```

```bash
cargo test -p sqltool-core --test real_servers -- --ignored
```

Development builds include a "Demo server" entry on the connection list: a fake MySQL Enterprise server inside the app, for trying the UI without a database.

## Phase 0 results

Run on Windows 10, WebView2 157, 160 Hz display, release builds. Your servers were not reachable, so a fake MySQL server (`crates/fakeserver`) stood in for them. It is hand-written, shares no code with the client driver, and imitates the login sequence of MySQL Enterprise 8.4.3 and 5.7.37 for an LDAP or PAM account.

### What is now proven

| Check | Result |
|---|---|
| Cleartext login, 8.4 shape (server offers `caching_sha2_password`, then switches to `mysql_clear_password`), verified TLS 1.3 | Pass |
| Cleartext login, 5.7 shape (server offers `mysql_native_password`, then switches), TLS 1.2 only | Pass |
| Server asks for cleartext but the profile has not enabled it | Refused; server received nothing |
| Cleartext enabled with TLS off, or TLS on but unverified | Refused before a socket is opened |
| Server certificate signed by an untrusted CA | Refused; no password sent |
| Certificate does not match the host name (Verify Identity) | Refused; no password sent |
| Profile requires TLS, server offers none | Refused; no password sent |
| Hostile server answers a query with a `LOCAL INFILE` file request | Refused; zero bytes uploaded |
| Cancel a running query from a second connection (`KILL QUERY`) | Interrupted in about 300 ms; session still usable |
| Driver throughput, 1,000,000 rows x 10 columns (138 MB) over loopback TLS | 0.8 s, about 1.2 million rows/s |

15 automated tests cover these (14 pass, 1 is a known driver defect, below).

### Grid prototype against the performance targets

1,000,000 rows x 50 columns (812 MB of cell data), full pipeline: connect, cleartext auth over TLS, stream into the Rust result store, UI fetches 512-row pages as binary.

| Target (section 7) | Measured | Verdict |
|---|---|---|
| First rows on screen under 100 ms | 39 ms after clicking Run | Met |
| Scrolling 1,000,000 x 50 with no dropped frames | 160 fps, worst frame 7.1 ms, at both 100 and 2,000 rows/s; no unloaded cells on screen | Met |
| Start to usable window under 1 s | 743 ms, but a warm start; cold start not measured | Likely met |
| Idle memory under 200 MB | 169 MB private across all 7 processes (296 MB working set) | Met, narrowly |
| Installer under 20 MB | Executable is 12.3 MB before packaging | On track |

Other measurements, not tied to a target:

| Measure | Result |
|---|---|
| Load the full million rows | 3.6 s, about 280,000 rows/s |
| Jump to a random position until fully painted | 7 ms typical, 9.9 ms worst of 40 |
| Page fetch over binary IPC (405 KB) | 4.9 ms typical, 8.4 ms worst |

Memory under load: the app process held 851 MB for the 812 MB result, because the store keeps everything in RAM. This is expected at this stage and confirms the planned memory budget with spill to disk is needed, not optional. The webview used about 305 MB with its page cache full.

### Real servers

Installed inside WSL (Ubuntu 26.04) with the scripts in `tests/servers/`, and tested from the Windows build of the smoke tool. Both servers use OpenSSL, as your Enterprise servers do.

| Server | Check | Result |
|---|---|---|
| MariaDB 11.8.6 | PAM account through `mysql_clear_password`, Verify Identity, TLS 1.3 | Pass |
| MariaDB 11.8.6 | Same login with the server limited to TLS 1.2 (`ECDHE-RSA-AES256-GCM-SHA384`) | Pass |
| MariaDB 11.8.6 | Same account, cleartext not enabled in the profile | Refused; nothing sent |
| MariaDB 11.8.6 | Wrong password | Reported as access denied |
| MariaDB 11.8.6 | `mysql_native_password` and `ed25519` accounts | Pass |
| MariaDB 11.8.6 | Server CA not trusted | Refused |
| MariaDB 11.8.6 | Stream 1,000,000 rows x 10 columns | 0.7 s |
| MySQL 8.4.13 | `caching_sha2_password` over verified TLS 1.3, first login and cached login | Pass |
| MySQL 8.4.13 | `caching_sha2_password` without TLS (RSA key exchange) | Pass |
| MySQL 8.4.13 | `mysql_native_password` | Pass |
| MySQL 8.4.13 | `sha256_password` | Not supported; reported clearly |
| MySQL 8.4.13 | Stream 1,000,000 rows x 10 columns | 1.1 s |
| MySQL 8.4.13 | Server's auto-generated certificate, Verify Identity | Refused, as it should be: the certificate names no host |
| MySQL 8.4.13 | Server's auto-generated certificate, Verify CA | Refused: the driver defect below, confirmed on a real server |

Version detection, metadata queries and query cancel passed on both.

**What this means for your servers.** A MySQL server running on its default auto-generated certificate can currently be reached only with unverified TLS, and the cleartext policy rightly refuses unverified TLS. Until the driver is fixed, cleartext login needs either a server certificate that includes the host name, or the explicit risk override. Fixing Verify CA is therefore the first item of Phase 1.

### What is not proven

- **MySQL Enterprise itself.** The Enterprise LDAP and PAM plugins use the same client-side `mysql_clear_password` exchange that passed against MariaDB's PAM plugin, and TLS 1.2 against OpenSSL passed, so the remaining risk is lower than before. It is still not a test of your servers. The smoke tool is ready for when one is reachable (command below).
- **MySQL 5.7.** Not in Ubuntu's packages. Testing it needs a large download from Oracle's archive, which has not been done.
- **Other versions.** MySQL 8.0, MariaDB 10.x and Percona Server have not run.
- **Linux and macOS.** Only the Windows webview was measured. The Linux webview remains the main open risk for the stack.

### Defects and limits found in the driver (`mysql_async` 0.37.1)

1. **Verify CA mode does not work with `rustls`.** The driver detects a host name mismatch by searching the error text for a string that current `rustls` no longer prints. The connection is refused, so it fails safe, but the mode is unusable. Confirmed against real MySQL 8.4.13. This matters for you: MySQL 5.7's auto-generated certificates carry no usable host name, so Verify CA is the mode such servers need.
2. **No hook for a custom TLS configuration.** Certificate fingerprint pinning (section 4) cannot be built on the driver as published.
3. **`sha256_password` is not supported** in this version; it is rejected as an unknown plugin. A newer `mysql_common` lists it, so this may resolve on upgrade.
4. TLS failures arrive as generic I/O errors. The core now recognises them and reports them as TLS errors.
5. By default the driver reconnects over a local socket when the host is loopback, bypassing TLS settings. The core turns this off.
6. The driver keeps the password in an ordinary string for the life of the connection, so it cannot be wiped from memory after login.

Items 1 and 2 need a change to the driver. Recommended: send both fixes upstream, and carry a patched copy until they are released.

### Decision

The stack holds on Windows with a wide margin. Recommended: proceed to Phase 1 on Tauri and Rust, and treat the Linux webview check and the real-server run as gates that must close before Phase 1 ends.

### How to run it

```bash
cargo test --workspace
```

```bash
cargo run --release -p sqltool-cli -- demo
```

Local test servers inside WSL or Linux, as root from the repository root. MariaDB listens on 127.0.0.1:3307 and MySQL 8.4 on 127.0.0.1:3308; generated passwords and CA certificates go to `tests/servers/.local/`, which git ignores:

```bash
bash tests/servers/mariadb-pam.sh
```

```bash
bash tests/servers/mysql84.sh san
```

Against a real server, once one is reachable. It prompts for the password and prints no secrets or table data:

```bash
cargo run --release -p sqltool-cli -- check --host HOST --user USER --cleartext --ca C:\path\to\ca.pem
```

## 1. Goals

A free, open-source desktop SQL client in the spirit of MySQL Workbench, built to be much lighter and faster.

- Runs on Windows, macOS and Linux from one codebase.
- Connects to MySQL and MariaDB, including these two required servers:
  - MySQL Enterprise Server 8.4.3-commercial
  - MySQL Enterprise Server Advanced 5.7.37-enterprise-commercial-advanced-log
- Supports the cleartext authentication plugin (`mysql_clear_password`), which is what LDAP and PAM logins on MySQL Enterprise use, plus the other common auth methods.
- Clean interface covering the everyday jobs: writing and running queries, browsing schemas, creating and altering schemas and tables, editing data.
- Secure by default, and fast with large result sets.

Out of scope for 1.0: other database engines, server administration dashboards, ER diagram modelling, migration tooling, cloud sync of settings.

## 2. Recommended stack

**Tauri 2 shell, Rust core, Svelte + TypeScript UI.**

| Layer | Choice | Why |
|---|---|---|
| Core (connections, queries, result storage, metadata, DDL generation) | Rust, async on Tokio | Memory safe, no garbage collector pauses, small binaries |
| MySQL protocol | `mysql_async` crate (pure Rust, MIT/Apache) | Has cleartext auth built in; no dependency on Oracle's GPL client library |
| TLS | `rustls` (TLS 1.2 and 1.3) | No OpenSSL to ship or patch; `native-tls` kept as a build option if a server needs it |
| Desktop shell | Tauri 2 | Uses the operating system's webview, so installers are roughly 10 to 20 MB instead of 100+ MB for Electron |
| UI | Svelte 5 + TypeScript | Compiles away the framework; small and fast |
| SQL editor | CodeMirror 6 | Has MySQL and MariaDB dialects, light, extensible autocomplete |
| Result grid | Virtualized grid fed by row windows from Rust | Only visible rows ever reach the UI |
| Secrets | OS keychain through the `keyring` crate | Windows Credential Manager, macOS Keychain, Linux Secret Service |

### What was checked

The driver is the part most likely to sink the project, so I checked its current documentation (`mysql_async` 0.37.1, `mysql_common` 0.38.2):

- `OptsBuilder::enable_cleartext_plugin` exists and defaults to off.
- The auth plugin list includes `mysql_native_password`, `caching_sha2_password`, `mysql_clear_password`, `mysql_old_password`, MariaDB `ed25519` (feature flag `client_ed25519`) and MariaDB `parsec` (feature flag `client_parsec`). `sha256_password` is listed in the newer `mysql_common` 0.38 but is not handled by `mysql_async` 0.37.1.
- TLS backends are selectable by feature flag: `rustls` or `native-tls`.

Checked in Phase 0: binary IPC throughput on Windows. Still open: that `rustls` negotiates with the 5.7.37 server's TLS configuration, and IPC and rendering speed on Linux and macOS.

### Alternatives considered

| Option | Verdict |
|---|---|
| Electron | Rejected. Ships a full Chromium, uses several hundred MB of RAM at idle. Conflicts with "fast and efficient". |
| Qt / C++ | Fastest fully native option, and MariaDB Connector/C supports every auth plugin. Rejected for memory-safety risk in a tool that handles credentials, and a slower path for outside contributors. This is the fallback if the webview approach fails the Phase 0 benchmarks. |
| Pure Rust GUI (egui, iced, Slint, gpui) | Best raw performance, but none has a mature SQL editor with autocomplete or a production data grid. We would be building both from scratch. |
| .NET Avalonia + MySqlConnector | Excellent driver coverage. Larger runtime and slower startup. |
| Java (the SQL Workbench/J and DBeaver route) | Heavy startup and memory. |

The core is a separate Rust library with no UI dependencies, so the shell can be replaced later without rewriting the database logic.

## 3. Architecture

```
┌──────────────────────── UI (webview) ────────────────────────┐
│ Connection manager │ Editor tabs │ Result grid │ Schema tree  │
│ Table designer     │ History     │ Settings                   │
└───────────────▲──────────────────────────────┬───────────────┘
                │ row windows (binary)         │ typed commands
┌───────────────┴──────────────────────────────▼───────────────┐
│ app/src-tauri    thin command layer, no business logic        │
├───────────────────────────────────────────────────────────────┤
│ crates/core      sessions, query execution, cancel,           │
│                  result store, metadata cache, history        │
│ crates/dialect   server detection, capability flags,          │
│                  statement splitter, DDL builders             │
│ crates/secrets   keychain access, in-memory secret handling   │
│ crates/cli       headless smoke-test tool (also used in CI)   │
└───────────────────────────────────────────────────────────────┘
                │ mysql_async + rustls
                ▼
        MySQL / MariaDB servers
```

Key rules:

- **The UI never holds a full result set.** Rust decodes rows into a compact store and the grid asks for `rows(result_id, offset, count)`.
- **The UI never sees a stored password.** It can send one in when the user types it. It cannot read one back.
- **The driver sits behind our own trait.** If `mysql_async` cannot do something we need (an unsupported auth plugin, say), we can patch, fork or replace it in one place.
- **Server differences live in `crates/dialect` only.** The rest of the code asks "does this server support CHECK constraints?" and never compares version numbers itself.

## 4. Authentication and transport

### Auth plugins

| Plugin | Where it appears | Plan |
|---|---|---|
| `mysql_native_password` | Default on 5.7 and MariaDB. Disabled by default on 8.4, removed in 9.0 | 1.0 |
| `caching_sha2_password` | Default on 8.0 and 8.4 | 1.0, including full authentication over TLS and RSA key exchange |
| `mysql_clear_password` | LDAP simple and PAM on MySQL Enterprise; MariaDB PAM with `pam_use_cleartext_plugin` | 1.0, **required for your servers** |
| `sha256_password` | Older 5.6 to 8.0 setups, deprecated | Not in the current driver; revisit on driver upgrade |
| `client_ed25519` | MariaDB | 1.0 |
| `parsec` | MariaDB 11.6+ | 1.0 |
| MariaDB `dialog` (interactive PAM) | MariaDB PAM default | After 1.0, needs our own implementation |
| Kerberos / GSSAPI, LDAP SASL, Windows native auth | Enterprise single sign-on | After 1.0 |
| WebAuthn / FIDO, OCI, OpenID Connect | Newer Enterprise options | Not planned until requested |

### Cleartext policy

Cleartext auth sends the real password to the server, so the client has to be strict about when it will do that. A hostile or impersonated server can ask any client to switch to cleartext mid-handshake in order to harvest the password.

1. Off by default. Enabled per connection with a clearly labelled toggle.
2. When enabled, the connection must use TLS with the server certificate verified (against a CA file or a pinned fingerprint). Unverified TLS does not count, because it does not stop impersonation.
3. Without verified TLS (unverified TLS, or none), cleartext is allowed only after an explicit acknowledgement on that connection, shown as a warning in the connection dialog.
4. A server that requests cleartext on a connection where it is not enabled gets a refused login and a clear error that says why. This is tested against a fake hostile server in CI.

### TLS

- Four modes: Verify Identity (the default for a new connection), Verify CA, Encrypted but not verified, and Not encrypted. "Encrypted, not verified" is what most MySQL clients do by default and is offered openly, with its limits explained in the dialog. There is no "use TLS if available" mode: silently falling back to no encryption is the downgrade this tool refuses.
- TLS 1.2 and 1.3 only. 5.7.37 Enterprise is built with OpenSSL and supports TLS 1.2, so this should work; the spike confirms it against your actual server.
- Self-signed and auto-generated server certificates are common on 5.7. The connection dialog shows the certificate fingerprint and offers to pin it, so users are not pushed into turning verification off.
- Client certificates (mutual TLS) supported.
- Servers that only speak TLS 1.0 or 1.1 (old community builds linked against yaSSL) are not supported by default. See open decisions.

### Other transport

- TCP, Unix socket, Windows named pipe.
- SSH tunnel with key or agent authentication, using the pure Rust `russh` crate (Phase 3).
- Protocol compression optional.

## 5. Server compatibility

### Target matrix

| Family | Versions tested |
|---|---|
| MySQL Community | 5.7, 8.0, 8.4 LTS, current 9.x |
| MySQL Enterprise | 5.7.37 and 8.4.3 (your servers, manual acceptance run) |
| MariaDB | 10.6, 10.11, 11.4, 11.8 and the current release |
| Percona Server | 8.0, 8.4 |

MySQL 5.6 and MariaDB 10.4/10.5 get best-effort support: expected to work, not in the release gate.

### Differences the dialect layer handles

- MariaDB before 11 prefixes its version string with `5.5.5-`; detection must strip it.
- `information_schema` is slow on large 5.7 instances. Metadata queries are always filtered by schema, loaded lazily, and cached.
- 8.4 removed `SHOW MASTER STATUS` and turned `mysql_native_password` off by default.
- Collations differ (`utf8mb4_0900_ai_ci` on MySQL 8+, `uca1400` on MariaDB 10.10+). The table designer lists only what the connected server reports.
- Feature availability varies: CHECK constraints, invisible columns, generated columns, JSON as a real type (MySQL) versus an alias (MariaDB), sequences and system-versioned tables (MariaDB only), roles.
- Query timeout uses `max_execution_time` on MySQL (SELECT only) and `max_statement_time` on MariaDB.
- Query cancel uses `KILL QUERY` from a second connection.

## 6. Features

### 1.0

**Connections**
- Saved profiles with folders, colour tags, and an environment label (for example "production").
- Test connection button with specific error messages for auth and TLS failures.
- Passwords in the OS keychain, or "ask every time".

**Query editor**
- Tabs, syntax highlighting for the connected dialect, formatting.
- Run current statement, run selection, run all. Statement splitting understands `DELIMITER`, comments, strings and `/*! */` version comments.
- Autocomplete for schemas, tables, columns, functions and keywords.
- Cancel a running query.
- Query history and saved snippets, searchable.
- Autocommit toggle with a visible open-transaction indicator, commit and rollback buttons.

**Results**
- Rows appear as they arrive. Smooth scrolling through millions of rows.
- Multiple result sets per run, with timing and affected-row counts.
- Sort and filter without re-running the query.
- Viewers for long text, JSON and binary values.
- Copy and export as CSV, TSV, JSON and SQL INSERT statements.
- Configurable automatic row limit (default 1000) with a one-click "fetch all".

**Schema browser**
- Tree of schemas, tables, views, routines, triggers and events, loaded on demand, with search.
- Table detail view: columns, indexes, foreign keys, DDL, row count estimate.

**Schema and table tools**
- Create and alter schema (character set, collation).
- Table designer: columns, types, defaults, nullability, auto-increment, primary key, indexes, foreign keys, engine, charset, comment.
- Alter table through the same designer. It compares before and after and generates the `ALTER`.
- Every change shows the exact SQL for review before it runs.
- Drop, truncate and rename with confirmation.

**Data editing**
- Edit, insert and delete rows in the grid for tables with a primary or unique key.
- Changes are staged, shown as SQL, and applied in one transaction.

**Guardrails**
- Read-only mode per connection.
- On connections labelled production: confirmation for DDL, and for `UPDATE` or `DELETE` without a `WHERE`.

### After 1.0

Visual EXPLAIN, process list and kill, server variables and status, user and privilege management, dump and restore, CSV import wizard, schema comparison, ER diagrams, the remaining auth plugins.

## 7. Performance design

Targets, to be validated in Phase 0 and then enforced by CI benchmarks:

| Measure | Target |
|---|---|
| Installer size | Under 20 MB |
| Cold start to usable window | Under 1 second |
| Idle memory, webview included | Under 200 MB |
| First rows on screen after the server starts replying | Under 100 ms |
| Scrolling a 1,000,000 x 50 result | No dropped frames |
| Schema tree on a server with 10,000 tables | Opens instantly, loads per schema |

How:

- Results stream from the driver into a compact store in Rust that keeps raw bytes plus column types. Values are formatted only for cells that are actually displayed.
- The store has a memory budget. Past it, rows spill to a temporary file that is private to the user and deleted when the result is closed.
- Row windows cross to the UI as binary, not JSON.
- Sorting and filtering of large results run in Rust.
- Metadata loads lazily and is cached per connection; the autocomplete index is built in the background.
- A small connection pool per profile: one session per editor tab (so session state and transactions behave as users expect), plus one shared utility connection for metadata and cancel.

## 8. Security design

**Credentials**
- Stored only in the OS keychain. Configuration files contain no secrets.
- Held in memory in zeroizing containers, never logged, never returned to the UI.
- On Linux without a keyring service: no storage, prompt each time.

**Server trust**
- TLS and cleartext rules as in section 4.
- `LOAD DATA LOCAL INFILE` is refused unless the user starts an import, and then only the chosen file is served. Without this a malicious server can request arbitrary local files.

**Application hardening**
- Strict content security policy: no remote content, no `eval`, no inline scripts.
- Tauri capabilities at the minimum: the UI has no shell or filesystem access, only our typed commands.
- All database content is rendered as text. A cell value containing HTML or script must never be interpreted by the webview.
- No telemetry. The only outbound connections are to the user's databases and an optional update check.

**History and logs**
- Query history is local and can be disabled.
- Statements containing passwords (`IDENTIFIED BY`, `SET PASSWORD`, and similar) are redacted before being saved.

**Supply chain and releases**
- Dependencies pinned by lockfile, checked with `cargo-deny`, `cargo-audit` and `npm audit` in CI.
- Signed installers on Windows and macOS, checksums and signatures on Linux, signed update manifests.
- `SECURITY.md` with a private reporting channel.
- An external security review before 1.0 if a volunteer or sponsor can be found.

## 9. Testing

- **Unit tests**: statement splitter, DDL generation (snapshot tests per dialect), version detection, redaction.
- **Compatibility matrix**: the core test suite runs against every server in section 5 in containers on each pull request.
- **Cleartext auth in CI without an Enterprise licence**: MariaDB with PAM and `pam_use_cleartext_plugin`, and Percona Server's PAM plugin, both exercise the same client-side `mysql_clear_password` path. (Percona's availability to be confirmed in the spike.)
- **Hostile server tests**: a small fake server that attempts an unsolicited cleartext switch, a TLS downgrade and a `LOCAL INFILE` file request. The client must refuse all three.
- **Enterprise acceptance**: Oracle's Enterprise images cannot be used in public CI. Before each release, the `cli` smoke tool is run against your 8.4.3 and 5.7.37 servers. It checks connect, auth, TLS, metadata, a query round trip and cancel, and prints a report with no secrets or data in it.
- **UI tests**: component tests in a browser with a mocked backend; a small end-to-end suite on Windows and Linux.
- **Benchmarks**: the section 7 targets, tracked per commit.

## 10. Open-source setup and distribution

- Licence: Apache-2.0 recommended (see open decisions).
- GitHub repository with CI on Windows, macOS (Intel and Apple Silicon) and Linux.
- `README`, `CONTRIBUTING`, `SECURITY`, code of conduct, issue templates, an architecture note.
- Packages: Windows installer and `winget`; macOS universal `.dmg` and Homebrew cask; Linux AppImage, `.deb`, `.rpm`, Flatpak later.
- Semantic versioning, changelog, signed auto-update that the user can turn off.

## 11. Roadmap

Each phase ends at a checkpoint that can be demonstrated.

### Phase 0: feasibility spike

Purpose: prove the two risky assumptions before building anything else.

- Rust command-line prototype that connects, authenticates and runs a query.
- Run it against your 8.4.3 and 5.7.37 servers with cleartext auth over verified TLS.
- Run it across the container matrix for the other auth plugins.
- Tauri prototype: stream a 1,000,000 row result into a grid on Windows, macOS and Linux, and measure against section 7.

**Checkpoint:** both of your servers connect; the grid meets the targets on all three systems. If the Linux webview misses the targets, decide between optimising and switching to the Qt fallback now, while it is cheap.

### Phase 1: connect and query (0.1 alpha)

Connection manager, keychain storage, TLS options, editor with tabs and highlighting, run and cancel, result grid, history, schema tree, compatibility matrix in CI.

**Checkpoint:** usable as a daily query tool against both of your servers.

### Phase 2: schema and data tools (0.2)

Autocomplete, create and alter schema, table designer with SQL preview, data editing, export, read-only mode and production guardrails.

**Checkpoint:** a table can be designed, created, altered and populated without typing SQL.

### Phase 3: hardening and release (beta, then 1.0)

SSH tunnels, signed installers for all three systems, auto-update, performance benchmarks in CI, hostile-server tests, accessibility and keyboard navigation pass, documentation, security review.

**Checkpoint:** public 1.0 with installers.

### Phase 4: after 1.0

The "After 1.0" list in section 6, ordered by user demand.

## 12. Risks

| Risk | Mitigation |
|---|---|
| `rustls` cannot negotiate with a server's TLS setup (old ciphers or certificates) | Tested in Phase 0 against the real 5.7.37 server; `native-tls` build option as fallback |
| The Linux webview (WebKitGTK) is slow for the grid | Benchmarked in Phase 0; core is UI-independent so the shell can change |
| `mysql_async` lacks a needed auth plugin or protocol feature | Driver is wrapped behind our own trait; contribute upstream or carry a patch. Phase 0 found two cases already (Verify CA, custom TLS configuration) |
| Enterprise-only behaviour cannot be tested in public CI | Manual acceptance run with the smoke tool before each release |
| Three different webview engines behave differently | UI tests run on all three; avoid cutting-edge web features |
| Code signing costs money for a free project | See open decisions |

## 13. Open decisions

1. **Stack.** Recommended: Tauri 2 + Rust + Svelte, confirmed or replaced at the end of Phase 0.
2. **Licence.** Recommended: Apache-2.0 (permissive, includes a patent grant). Choose GPL-3.0 instead if you want to prevent closed-source forks.
3. **Your servers' auth setup.** Is it LDAP or PAM behind the cleartext plugin, is TLS enabled on both servers, and which CA signs the server certificates? This decides what the Phase 0 test looks like.
4. **Legacy TLS.** Recommended: do not support TLS 1.0 and 1.1. Say so if you have servers that need it.
5. **Code signing.** macOS notarisation needs an Apple Developer account (paid yearly). Windows signing needs a certificate; free programmes for open-source projects exist and need an application. Unsigned builds work but show operating system warnings.
6. **Name.**
