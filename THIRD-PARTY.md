# Third-party software

SQL Tool is licensed under Apache-2.0 (see [LICENSE](LICENSE)). It is built with the open-source software below, which stays under its own licences.

## In the user interface

These are compiled into the app's window. Exact versions are in [app/package-lock.json](app/package-lock.json).

| Software | Licence | Used for |
|---|---|---|
| [Apache ECharts](https://echarts.apache.org) and its renderer, zrender | Apache-2.0 (zrender: BSD-3-Clause) | Drawing graphs |
| [CodeMirror 6](https://codemirror.net) and Lezer | MIT | The SQL editor |
| [Svelte](https://svelte.dev) | MIT | The interface |
| [sql-formatter](https://github.com/sql-formatter-org/sql-formatter) | MIT | Reformat |
| [Tauri API](https://tauri.app) | Apache-2.0 or MIT | Talking to the desktop shell |

The smaller packages these depend on are under MIT, BSD-3-Clause, 0BSD, CC0-1.0 or Python-2.0. `npm ls --omit=dev --all` in `app` lists them.

Apache ECharts carries this notice:

> Apache ECharts
> Copyright 2017-2026 The Apache Software Foundation
>
> This product includes software developed at
> The Apache Software Foundation (https://www.apache.org/).

## In the desktop shell and core

Rust crates, listed with their versions in [Cargo.lock](Cargo.lock): most under MIT or Apache-2.0. The database driver is a patched copy of `mysql_async` (MIT or Apache-2.0) in [vendor/mysql_async](vendor/mysql_async); its changes are described in `PATCHES.md` there.

The full text of each licence is in the software's own repository, linked above, and in the packages themselves under `app/node_modules` and Cargo's registry after a build.
