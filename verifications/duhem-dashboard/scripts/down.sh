#!/bin/sh
# Tear the duhem-dashboard back down after the regression VD finishes
# (part of #148). Best-effort: failures here are recorded as evidence
# but never alter the run verdict (mechanism from issue #50).
#
# Runs after the last criterion regardless of verdict, unless the
# operator passes `--keep-env` on `duhem run`.

set -u

PORT="${DUHEM_DASHBOARD_PORT:-7878}"
WORK="${TMPDIR:-/tmp}/duhem-dashboard-vd-${PORT}"
PID_FILE="${WORK}/dashboard.pid"

if [ -f "$PID_FILE" ]; then
  PID=$(cat "$PID_FILE" 2>/dev/null || true)
  if [ -n "${PID:-}" ]; then
    echo "down.sh: stopping duhem dashboard process group (pid $PID)"
    # `up.sh` started the wrapper via `setsid`, so the wrapper and the
    # `duhem-dashboard` listener share a process group (pgid == pid).
    # Signal the whole group so the listener dies with the wrapper.
    kill -TERM -- -"$PID" 2>/dev/null || kill -TERM "$PID" 2>/dev/null || true
    sleep 1
    kill -KILL -- -"$PID" 2>/dev/null || kill -KILL "$PID" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
fi

# Belt-and-suspenders: kill any `duhem-dashboard` listener still serving
# THIS port's scratch evidence dir (the match is unique per port). Guards
# against a startup race where the group signal missed a just-spawned
# child.
if command -v pkill >/dev/null 2>&1; then
  pkill -KILL -f "duhem-dashboard --evidence-dir ${WORK}/runs" 2>/dev/null || true
fi

# Drop the scratch evidence dir so runs don't accumulate.
rm -rf "$WORK" 2>/dev/null || true
exit 0
