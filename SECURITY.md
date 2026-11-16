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
- **The user interface cannot read a stored password back**, cannot read or write files of its own choosing, and loads no remote content.
- **A server cannot make the client upload local files** (`LOAD DATA LOCAL INFILE` requests are refused).
- **Read-only connections do not change data.**
- **Statements shown for review are the statements that run.**

## Known limits

- Release builds are not code-signed.
- With "Encrypted, not verified" or "Not encrypted", the server is not authenticated. The tool says so in the connection dialog and the status bar.
- Large results spill to a temporary file that is private to the user and removed when the result is closed, but is not encrypted.
- The database driver keeps the password in memory for the life of a connection.
- The tool has not had an independent security review.

## Supported versions

Only the latest release receives fixes.
