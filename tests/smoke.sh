#!/usr/bin/env bash
# Starts the desktop app and lets it check itself: it connects to the server
# built into it, runs a query through the real interface, copies, exports,
# and exits with a report. No database is needed, so this runs anywhere the
# app builds; CI runs it on Windows, Linux and macOS. Run from the repository
# root in Git Bash (Windows) or any shell (Linux, macOS), after building:
#
#   (cd app && npm ci && npm run build) && cargo build --release -p sqltool-app --features custom-protocol,devtools
#   bash tests/smoke.sh
#
# The app's window opens for a few seconds and closes itself. Its settings for
# the run go to a folder under target/, not to your real ones. A path to the
# app can be given, for a build made for a named target:
#
#   bash tests/smoke.sh target/aarch64-apple-darwin/release/sqltool-app
set -uo pipefail

APP="${1:-target/release/sqltool-app}"
[ -x "$APP.exe" ] && APP="$APP.exe"
if [ ! -x "$APP" ]; then
  echo "No app at $APP. Build it first; see the top of this script." >&2
  exit 2
fi

native() { command -v cygpath >/dev/null 2>&1 && cygpath -w "$1" || echo "$1"; }

OUT="$PWD/target/smoke"
rm -rf "$OUT"
mkdir -p "$OUT/data" "$OUT/export"

export SQLTOOL_SMOKE=1
export SQLTOOL_DATA_DIR="$(native "$OUT/data")"
export SQLTOOL_SELFTEST_EXPORT_DIR="$(native "$OUT/export")"
export SQLTOOL_SELFTEST_REPORT="$(native "$OUT/report.json")"

# With no screen (a CI runner), Linux gets a virtual one, and the web view is
# told not to look for a graphics card.
RUN=()
if [ "$(uname -s)" = Linux ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
  if ! command -v xvfb-run >/dev/null 2>&1; then
    echo "There is no screen and xvfb-run is not installed." >&2
    exit 2
  fi
  RUN=(xvfb-run -a)
  export WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
fi

"${RUN[@]}" "$APP" > "$OUT/app.log" 2>&1 &
PID=$!
LIMIT=120
for _ in $(seq 1 "$LIMIT"); do
  kill -0 "$PID" 2>/dev/null || break
  sleep 1
done
STATUS=0
if kill -0 "$PID" 2>/dev/null; then
  echo "  [FAIL] the app was still running after $LIMIT seconds; it was stopped"
  kill "$PID" 2>/dev/null
  STATUS=1
fi
wait "$PID" 2>/dev/null

if [ -f "$OUT/report.json" ]; then
  node -e '
const r = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"));
for (const s of r.steps) {
  const detail = s.ok ? (s.detail == null ? "" : "  " + JSON.stringify(s.detail).slice(0, 260)) : "  " + s.error;
  console.log((s.ok ? "  [ ok ] " : "  [FAIL] ") + s.name + " (" + s.ms + " ms)" + detail);
}
console.log(r.ok ? "All " + r.total + " steps passed." : r.passed + " of " + r.total + " steps passed.");
process.exit(r.ok ? 0 : 1);
' "$(native "$OUT/report.json")" || STATUS=1
else
  echo "  [FAIL] the app wrote no report. Was it built with --features custom-protocol,devtools?"
  STATUS=1
fi

# Count what the app actually wrote, rather than trust the number it reported.
CSV_LINES=$(wc -l < "$OUT/export/result.csv" 2>/dev/null || echo 0)
if [ "$CSV_LINES" -eq 2001 ]; then
  echo "  [ ok ] the exported file holds 2000 rows and its header"
else
  echo "  [FAIL] the exported file has $CSV_LINES lines, not 2001"
  STATUS=1
fi

# A crash, or an error nothing handled, leaves a line here.
if [ -s "$OUT/data/crash.log" ]; then
  echo "  [FAIL] crash.log is not empty:"
  sed 's/^/         /' "$OUT/data/crash.log"
  STATUS=1
else
  echo "  [ ok ] nothing was written to crash.log"
fi

if [ "$STATUS" -ne 0 ] && [ -s "$OUT/app.log" ]; then
  echo "What the app printed:"
  tail -n 40 "$OUT/app.log" | sed 's/^/  /'
fi
exit $STATUS
