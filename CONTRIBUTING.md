# Contributing

Thanks for looking. This file covers how to build and test, and the few rules the code keeps.

## Building

You need [Rust](https://rustup.rs) and [Node.js](https://nodejs.org). On Linux, also the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

```bash
cd app && npm install && npm run tauri:dev
```

`tauri:dev` turns on the `devtools` feature, which adds a "Demo server" (a fake MySQL server inside the app) and the unattended self-test. Release builds leave it out.

The UI also runs in a plain browser against a mock backend, which is the quickest way to work on screens:

```bash
cd app && npm run dev
```

## Testing

```bash
cargo test --workspace
```

```bash
cd app && npm run check
```

The workspace tests need no database: login, TLS and query behaviour run against the fake server in `crates/fakeserver`.

CI also refuses Rust code that is not formatted or that the lints object to. Before pushing:

```bash
cargo fmt
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

The version of Rust, and so of its lints, is the one named in `rust-toolchain.toml`; rustup installs it the first time `cargo` runs here. `cargo fmt` formats this workspace's own crates and leaves the driver copy under `vendor/` alone.

Tests against real MariaDB and MySQL, and the app's end-to-end run, use local servers set up by the scripts in `tests/servers` (Linux or WSL, as root):

```bash
bash tests/servers/mariadb-pam.sh
```

```bash
bash tests/servers/mysql84.sh san
```

```bash
cargo test -p sqltool-core --test real_servers -- --ignored
```

```bash
cargo build --release -p sqltool-app --features custom-protocol,devtools && bash tests/servers/selftest.sh mariadb
```

The same build also runs a shorter check that needs no database, against the server built into it. CI runs it on Windows, Linux and macOS, so that a change which stops the app from starting on one of them is caught:

```bash
bash tests/smoke.sh
```

## Rules the code keeps

- **`crates/core` knows nothing about the UI.** Connections, policy, running statements, result storage and SQL generation live there and are tested there.
- **The desktop shell is thin.** Commands in `app/src-tauri` translate arguments and call the core.
- **The UI never receives a stored password**, and never chooses a file path to read or write. File dialogs run in the shell.
- **Trust is decided before credentials are sent.** Anything that weakens a check in `policy.rs` or the TLS setup needs a test showing what a hostile server receives.
- **SQL that changes something is shown before it runs**, and the text shown is what runs.
- **Server differences go through version checks in the core**, not through guesses in the UI.
- **New behaviour comes with a test.** For anything touching login or TLS, that means a test against the fake server.

## The driver copy

`vendor/mysql_async` is a patched copy of the upstream driver. Read `vendor/mysql_async/PATCHES.md` before changing it; every local change is marked `SQL Tool patch` and listed there.

## Pull requests

A change that someone using the app would notice gets a line under **Unreleased** in [CHANGELOG.md](CHANGELOG.md), in words a user would use. A release moves those lines under its version number.

CI builds, tests and packages on Windows, Linux and macOS. Keep pull requests to one topic, and say in the description what you checked and what you could not.
