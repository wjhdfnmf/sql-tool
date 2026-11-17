# Security

SQL Tool handles database credentials, so security reports are taken seriously.

## Reporting a problem

Please do not open a public issue for a security problem.

Use GitHub's private reporting instead: on the repository page, open the **Security** tab and choose **Report a vulnerability**. If that option is not shown, open an issue that says only that you have a security report, with no details, and a maintainer will arrange a private channel.

Include what you did, what happened, and what you expected. A way to reproduce it helps most.

This is a volunteer project. There is no guaranteed response time.

## What the tool is meant to guarantee

A report is most useful when it shows one of these being broken:

- **A password is never sent to a server that failed verification.** With verified or pinned TLS, a server presenting the wrong certificate gets nothing.
- **Cleartext authentication is refused unless it is turned on for that connection**, and on an unverified connection only after an explicit acknowledgement.
- **Passwords are stored only in the operating system keychain.** They are not written to settings, history or log files.
- **The user interface cannot read a stored password back**, cannot read or write files of its own choosing, and loads no remote content. A script can be saved only to a file the user picked in an Open or Save dialog. The backend keeps that list itself and remembers it between runs, so a restored tab can be saved back to its file; nothing the interface stores can add to it. A connection's script folder only sets where those dialogs start.
- **A server cannot make the client upload local files** (`LOAD DATA LOCAL INFILE` requests are refused).
- **Read-only connections do not change data.**
- **Statements shown for review are the statements that run.**

## Known limits

- Release builds are not code-signed.
- With "Encrypted, not verified" or "Not encrypted", the server is not authenticated. Something on the network path that impersonates the server can then read the password of any account, not only of accounts that use cleartext authentication, along with everything else sent. The tool says so in the connection dialog and the status bar. Pinning the server's certificate closes this without needing a CA file.
- Query history and the text of open query tabs are kept on the computer without encryption, so that they survive a restart. Statements that contain a password are left out of both. On Linux and macOS the history and saved-connection files are readable by their owner only.
- SQL INSERT statements produced by export and copy are escaped for the server's default SQL mode. They are not safe to run on a server set to `NO_BACKSLASH_ESCAPES` if the data contains backslashes.
- CSV and tab-separated exports and copies write values as they are. A value that begins with `=`, `+`, `-` or `@` may be run as a formula by a spreadsheet that opens the file. Treat exports of data you do not trust accordingly.
- When the server drops a connection, the tool opens a new one with the same login before the next statement, never in the middle of one. The new connection goes through the same certificate checks as the first. A statement that was running when the connection was lost is not sent again.
- A graph of a result is drawn by a third-party library (Apache ECharts) inside the app's window, under the same content security policy as everything else and loaded from the app itself. It is given numbers and names worked out by the backend, never rows; names from the data are drawn as text on the chart or set as text in a tooltip, never interpreted as markup. The columns a graph uses are kept in memory as numbers while it is open (about 8 bytes a row for each), counted against the result memory setting.
- Large results spill to a temporary file that is private to the user and removed when the result is closed, but is not encrypted.
- The database driver keeps the password in memory for the life of a connection.
- The database driver is a patched copy of `mysql_async` taken from its development branch (see `vendor/mysql_async/PATCHES.md`), not a published release.
- The tool has not had an independent security review.

## Supported versions

Only the latest release receives fixes.
