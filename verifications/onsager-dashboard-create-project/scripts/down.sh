#!/bin/sh
# Tear the Onsager dev server back down after the create-project
# verification finishes. Best-effort: failures here are recorded as
# evidence but never alter the run verdict (spec on issue #50).
#
# Runs after the last criterion regardless of verdict, unless the
# operator passes `--keep-env` on `duhem run` to leave the SUT up
# for triage.

set -u

PID_FILE="${TMPDIR:-/tmp}/onsager-dev.pid"
if [ -f "$PID_FILE" ]; then
  PID=$(cat "$PID_FILE")
  if kill -0 "$PID" 2>/dev/null; then
    echo "down.sh: stopping onsager dev server (pid $PID)"
    kill "$PID" 2>/dev/null || true
    # Give Next.js a moment to flush, then escalate if it hasn't
    # actually stopped.
    sleep 1
    if kill -0 "$PID" 2>/dev/null; then
      kill -9 "$PID" 2>/dev/null || true
    fi
  fi
  rm -f "$PID_FILE"
fi

# Drop the dogfood fixture rows so each run starts from a known
# state. Skipped silently when the Onsager repo isn't on disk —
# teardown should never wedge on a missing optional artifact.
ONSAGER_REPO="${DUHEM_ONSAGER_REPO_DIR:-../../../onsager}"
if [ -f "$ONSAGER_REPO/scripts/drop-duhem-fixtures.sh" ]; then
  echo "down.sh: dropping dogfood fixtures"
  ( cd "$ONSAGER_REPO" && ./scripts/drop-duhem-fixtures.sh ) || true
fi

exit 0
