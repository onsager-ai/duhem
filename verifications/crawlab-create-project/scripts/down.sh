#!/bin/sh
# Tear the Crawlab stack back down after the create-project verification
# finishes. Best-effort: failures here are recorded as evidence but never
# alter the run verdict.
#
# Runs after the last criterion regardless of verdict, unless the
# operator passes `--keep-env` on `duhem run` to leave the SUT up.

set -u

TMP="${TMPDIR:-/tmp}"
PID_FILE="$TMP/duhem-crawlab.pid"
ISO_HOME="$TMP/duhem-crawlab-home"
PORT="${CRAWLAB_HOST_PORT:-8090}"

# Stop the `go run` master. up.sh `setsid`s it into its own process
# group, so killing the group (-PID) reaps `go run` AND its compiled
# `core` child.
if [ -f "$PID_FILE" ]; then
  PID=$(cat "$PID_FILE")
  if kill -0 "$PID" 2>/dev/null; then
    echo "down.sh: stopping crawlab master (pgid $PID)"
    kill -- "-$PID" 2>/dev/null || kill "$PID" 2>/dev/null || true
    sleep 2
    kill -9 -- "-$PID" 2>/dev/null || kill -9 "$PID" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
fi

# Backstop: kill whatever still listens on the REST port (e.g. a `core`
# binary orphaned by an interrupted run). `ss` is best-effort.
LPID=$(ss -ltnp 2>/dev/null | grep ":${PORT} " | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2)
if [ -n "$LPID" ]; then
  echo "down.sh: killing leftover listener on :$PORT (pid $LPID)"
  kill -9 "$LPID" 2>/dev/null || true
fi

echo "down.sh: removing mongo container"
docker rm -f duhem-crawlab-mongo >/dev/null 2>&1 || true

rm -rf "$ISO_HOME" 2>/dev/null || true
exit 0
