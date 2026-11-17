#!/usr/bin/env bash
# Runs the desktop app's unattended self-test against one of the local test
# servers and prints the report. Run from the repository root in Git Bash
# (Windows) or any shell (Linux, macOS), after building the app:
#
#   (cd app && npm run build) && cargo build --release -p sqltool-app --features custom-protocol,devtools
#   bash tests/servers/selftest.sh mariadb
#   bash tests/servers/selftest.sh mysql84
#
# The app window opens briefly and closes itself. Profiles and history for
# the run go to a temporary folder, not your real settings.
set -uo pipefail

TARGET="${1:-mariadb}"
LOCAL="tests/servers/.local"
native() { command -v cygpath >/dev/null 2>&1 && cygpath -w "$1" || echo "$1"; }

OUT="${TMPDIR:-${TEMP:-/tmp}}/sqltool-selftest"
command -v cygpath >/dev/null 2>&1 && OUT="$(cygpath -u "$OUT")"
rm -rf "$OUT"
mkdir -p "$OUT/data"

export SQLTOOL_DATA_DIR="$(native "$OUT/data")"
export SQLTOOL_SELFTEST_HOST=127.0.0.1
export SQLTOOL_SELFTEST_DATABASE=sqltool_test
export SQLTOOL_SELFTEST_SCHEMA=sqltool_test
export SQLTOOL_SELFTEST_TABLE=people

case "$TARGET" in
  mariadb)
    # PAM account: the cleartext plugin over verified TLS.
    . "$LOCAL/mariadb.env"
    export SQLTOOL_SELFTEST_PORT=3307 SQLTOOL_SELFTEST_USER=sqltool_pam SQLTOOL_SELFTEST_PASSWORD="$PAM_PASSWORD"
    export SQLTOOL_SELFTEST_CLEARTEXT=1
    export SQLTOOL_SELFTEST_CA="$(native "$PWD/$LOCAL/mariadb-ca.pem")"
    export SQLTOOL_SELFTEST_BIG_QUERY="SELECT seq, CONCAT('row ', seq) FROM seq_1_to_20000000"
    export SQLTOOL_SELFTEST_EXPORT_QUERY="SELECT seq, CONCAT('row ', seq), REPEAT('x', 40) FROM seq_1_to_6000000"
    EXPORT_ROWS=6000000
    ;;
  mysql84)
    . "$LOCAL/mysql84.env"
    export SQLTOOL_SELFTEST_PORT=3308 SQLTOOL_SELFTEST_USER=sqltool_sha2 SQLTOOL_SELFTEST_PASSWORD="$SQL_PASSWORD"
    export SQLTOOL_SELFTEST_CA="$(native "$PWD/$LOCAL/mysql84-ca.pem")"
    export SQLTOOL_SELFTEST_BIG_QUERY="WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 3000000) SELECT /*+ SET_VAR(cte_max_recursion_depth = 3000000) */ n, CONCAT('row ', n) FROM s"
    export SQLTOOL_SELFTEST_EXPORT_QUERY="WITH RECURSIVE s(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM s WHERE n < 1500000) SELECT /*+ SET_VAR(cte_max_recursion_depth = 1500000) */ n, CONCAT('row ', n), REPEAT('x', 40) FROM s"
    EXPORT_ROWS=1500000
    ;;
  *)
    echo "usage: $0 mariadb|mysql84" >&2
    exit 2
    ;;
esac

# Two folders to stand for two connections' script folders.
mkdir -p "$OUT/scripts-a" "$OUT/scripts-b"
export SQLTOOL_SELFTEST_SCRIPTS_A="$(native "$OUT/scripts-a")"
export SQLTOOL_SELFTEST_SCRIPTS_B="$(native "$OUT/scripts-b")"

# Exports made by the self-test go here instead of through a file dialog.
mkdir -p "$OUT/export"
export SQLTOOL_SELFTEST_EXPORT_DIR="$(native "$OUT/export")"
# A small memory allowance, so that most of the large result is kept on disk.
export SQLTOOL_RESULT_MEMORY_MB=64

APP="target/release/sqltool-app"
[ -x "$APP.exe" ] && APP="$APP.exe"
timeout 300 "$APP" 2>/dev/null

if [ ! -f "$OUT/report.json" ]; then
  echo "The app wrote no report. Is the server running, and was the app built with --features custom-protocol,devtools?" >&2
  exit 1
fi

node -e '
const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
for (const s of r.steps) {
  const detail = s.ok ? (s.detail == null ? "" : "  " + JSON.stringify(s.detail).slice(0, 200)) : "  " + s.error;
  console.log((s.ok ? "  [ ok ] " : "  [FAIL] ") + s.name + " (" + s.ms + " ms)" + detail);
}
console.log(r.ok ? "All " + r.total + " steps passed." : r.passed + " of " + r.total + " steps passed.");
process.exit(r.ok ? 0 : 1);
' "$(native "$OUT/report.json")"
STATUS=$?

# Count what the app actually wrote, rather than trust the number it reported.
CSV_LINES=$(wc -l < "$OUT/export/result.csv" 2>/dev/null || echo 0)
SQL_LINES=$(wc -l < "$OUT/export/result.sql" 2>/dev/null || echo 0)
if [ "$CSV_LINES" -eq $((EXPORT_ROWS + 1)) ] && [ "$SQL_LINES" -eq "$EXPORT_ROWS" ]; then
  echo "  [ ok ] the exported files hold all $EXPORT_ROWS rows (CSV $CSV_LINES lines with its header, SQL $SQL_LINES statements)"
else
  echo "  [FAIL] exported files: CSV has $CSV_LINES lines and SQL $SQL_LINES, for $EXPORT_ROWS rows"
  STATUS=1
fi

# The password must never reach the settings or history files.
if grep -rqF "$SQLTOOL_SELFTEST_PASSWORD" "$OUT/data" 2>/dev/null; then
  echo "  [FAIL] the password was written to a file under $OUT/data"
  STATUS=1
else
  echo "  [ ok ] the password is in no settings or history file"
fi
exit $STATUS
