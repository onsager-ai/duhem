#!/bin/sh
set -eu

PORT="${DUHEM_DASHBOARD_FIXTURE_PORT:-7979}"
WORK="${TMPDIR:-/tmp}/duhem-dashboard-fixture-${PORT}"
PID_FILE="${WORK}/server.pid"
if [ -f "$PID_FILE" ]; then
  PID="$(cat "$PID_FILE" 2>/dev/null || true)"
  [ -z "$PID" ] || kill -- -"$PID" 2>/dev/null || kill "$PID" 2>/dev/null || true
  rm -f "$PID_FILE"
fi
