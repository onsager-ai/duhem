#!/bin/sh
# Tear the Arbor single-port server back down after the
# create-and-ship-app verification finishes. Best-effort: failures
# here are recorded as evidence but never alter the run verdict.
#
# Runs after the last criterion regardless of verdict, unless the
# operator passes `--keep-env` on `duhem run` to leave the SUT up for
# triage.

set -u

PID_FILE="${TMPDIR:-/tmp}/arbor-dev.pid"
if [ -f "$PID_FILE" ]; then
  PID=$(cat "$PID_FILE")
  if kill -0 "$PID" 2>/dev/null; then
    echo "down.sh: stopping arbor server (pid $PID)"
    # Signal the process group so pnpm/node children don't outlive the
    # recipe shell, then escalate to SIGKILL if it lingers.
    kill -- "-$PID" 2>/dev/null || kill "$PID" 2>/dev/null || true
    sleep 2
    if kill -0 "$PID" 2>/dev/null; then
      kill -9 -- "-$PID" 2>/dev/null || kill -9 "$PID" 2>/dev/null || true
    fi
  fi
  rm -f "$PID_FILE"
fi

# Arbor stores app/run data under ~/.arbor (ARBOR_HOME). It is left in
# place between runs — runs accumulate under unique ids and don't
# collide. Clear it (`rm -rf ~/.arbor`) when the fixtures pile up.

exit 0
