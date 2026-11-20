# Changelog

What changed in each release, newest first. The [releases page](https://github.com/wjhdfnmf/sql-tool/releases) has the installers and the longer notes for each version, with what was and was not tried by hand.

## Unreleased

### Changed

- **A new icon**: a query prompt, in place of the three bars that read as a menu button.

## 0.3.1 - 2026-11-20

### Added

- **Connections can be organised**: drag one to move it, and gather them into named groups that fold away. A setting, on to begin with.
- **Settings has a tab for each area** (Appearance, Editor, Queries, Connections, Privacy), a mark on a tab where something differs from the defaults, and a button that restores one tab's defaults.
- **Hide the output while editing** (a setting, off to begin with): a click in the editor puts the output away, the pointer at the bottom edge brings it back. Double-clicking Output, or Ctrl+Shift+O (Cmd+Shift+O on a Mac), switches it on and off, and the Output button takes the theme's colour while it is on.
- **Closing the window asks first** while a statement is running, while changes in a result or a designer are not applied, or while a script has changes that would be lost.
- **An error that nothing handled is shown**, and noted in `crash.log` in the data folder.
- **Help** has the licences of everything the app is built with, the project's address, and where the data folder is.
- **macOS**: one app for Apple silicon and Intel. **Linux**: the packages run on Ubuntu 22.04 and Debian 12 and later, not only on the newest systems.
- **The window opens where it was**, at the size it had.
- **An exported file is named** after the table its rows came from, or the script, instead of always `result`.
- **Starting the app a second time** brings the running one to the front.

### Changed

- **The editor's toolbar**: Execute Statement is labelled **ES**, to leave room, and the arrows for moving between query tabs are larger and darker.

### Fixed

- **macOS**: Cmd+W closed the window, which quit the app; it closes the query tab, as Ctrl+W does elsewhere. Explain (Cmd+Option+X) did nothing. Shortcuts are named Cmd and Option in tooltips, menus and Help.
- **Linux with no keyring**: saving or deleting a connection reported an error, and a connection with a saved password would not connect. The connection is saved and a notice says what became of the password; connecting asks for it.
- **Linux and macOS**: Copy could be refused by the system ("The clipboard could not be written to"). Copying cells, a name or a picture of a graph now goes through the system's own clipboard on every system.
- **An export of the rows shown that failed part-way** left the partial file. It is removed.
- **A script file that begins with a byte-order mark** failed at its first statement. The mark is dropped; UTF-16 files are opened too.
- **If the app could not start** it closed without a word. It says why, and notes it in `crash.log`.

## 0.3.0 - 2026-11-20

### Added

- **Long queries in time slices** (In Slices): a query over a long stretch of time is run as many short ones, marked with `{start}` and `{end}`, and the rows are put end to end as one result. A small bar shows how many slices are in and how long the rest should take.
- **Import from MySQL Workbench**, in Settings: reads the connections Workbench has saved and brings over the ones you tick. Passwords are not imported.
- **Pareto chart**: bars and running total on axes of their own, a mark for the vital few, values on the bars.
- **Control chart**: the eight run rules, a second chart of the spread, 1 and 2 sigma lines, a centre and sigma given from before, a chosen number of rows to a point.
- **New** beside Open, Save and Save As; **Reset** on a graph; four layouts and a choice of keyword case for **Reformat**.

### Changed

- **Connections page**: each connection is a card showing its name and `user@host`, two to a row.

### Fixed

- **Reformat on a selection disturbed what was around it.** It now changes the selection and nothing else; with nothing selected, the whole text.

## 0.2.0 - 2026-11-17

### Added

- **The graph builder**: a result charted without leaving the app, from every row however many there are. Bar, line, pie, Pareto, histogram, box plot, scatter plot and control chart; columns dragged onto axes; filters and ranges; lines at a value; specification limits with Cp, Cpk, Pp and Ppk; trend lines; titles and axis names; copy or save as a picture.

## 0.1.6 - 2026-11-17

### Changed

- **The default schema is kept in the connection's settings**, so the settings, the schema tree and the next connect agree. A connection whose default schema has been dropped still opens.
- **Designer tabs have no toolbar row**, and Navigator and Output sit directly after Reformat.

### Fixed

- **With two connections open, Open showed the other connection's script folder.**

## 0.1.5 - 2026-11-16

### Added

- **A script folder per connection**: Open and Save As start there.

### Changed

- **Exporting says how many rows were written**, and a result that stopped at the row limit asks whether to export every row the query returns or only the rows shown.
- **The result's footer row is gone**: notices, selection totals and the row count are in the bottom row of the window.

### Fixed

- **Save after a restart writes back to the file** the tab was opened from, instead of asking where to save.

## 0.1.4 - 2026-11-16

### Added

- **Ten more themes**, sixteen in all, each checked for contrast.

### Changed

- **More room for the editor and results**: the output appears when a tab first has something to show, query tabs stay on one row, and the result's controls share the row of result tabs.
- **A tab with a query running is coloured**, and the row count shows whether rows are still arriving, complete, or stopped at the limit.

### Fixed

- **A new tab could say "no database selected"** while the tree showed a default schema. The default schema now belongs to the connection, and every tab uses it.

## 0.1.3 - 2026-11-16

### Added

- **Timeouts**: connecting gives up after 15 seconds; a query timeout and a keep-alive for idle connections are in Settings, off to begin with.
- **Themes and a Settings window**: theme, size of text, editor indent and wrapping, row limit, memory for results, and whether history and open tabs are kept.

### Changed

- **The table designer and the schema editor open as tabs.**

### Fixed

- **A connection the server had closed failed on every run.** It is reopened before the next statement; a statement never runs twice.

## 0.1.2 - 2026-11-16

### Changed

- **A tidier navigator**: clicking a table selects it, and its columns and indexes are shown in Object info, not inside the tree.
- **No scroll bars on the rows of tabs and buttons**; what does not fit wraps.

## 0.1.1 - 2026-11-16

### Added

- **Selecting and copying cells** from a result, with totals for the selection.
- **Foreign keys** in the table designer.
- **Query tabs come back** when a connection is opened again.

### Fixed

- **Linux and macOS**: the history and saved-connection files were readable by other accounts on the same computer. They are readable by their owner only.

## 0.1.0 - 2026-11-16

The first release with installers for Windows, Linux and macOS: connections with passwords in the system keychain, verified or pinned TLS, the SQL editor, results that stay smooth with millions of rows, export, editing rows in place, the schema browser and the table designer, read-only connections, and confirmation of risky statements on production connections.
