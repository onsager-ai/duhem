#!/bin/sh
set -eu

PORT="${DUHEM_DASHBOARD_FIXTURE_PORT:-7979}"
WORK="${TMPDIR:-/tmp}/duhem-dashboard-fixture-${PORT}"
PID_FILE="${WORK}/server.pid"
mkdir -p "$WORK"

if [ -f "$PID_FILE" ]; then
  OLD="$(cat "$PID_FILE" 2>/dev/null || true)"
  [ -z "$OLD" ] || kill -- -"$OLD" 2>/dev/null || kill "$OLD" 2>/dev/null || true
fi

setsid python3 -m http.server "$PORT" --bind 127.0.0.1 --directory . \
  >"${WORK}/server.log" 2>&1 &
echo $! > "$PID_FILE"
