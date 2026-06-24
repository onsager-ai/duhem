#!/bin/sh
# Tear the Crawlab dashboard stack back down after the UI verification
# finishes. Best-effort: failures here are recorded as evidence but
# never alter the run verdict.
#
# Runs after the last criterion regardless of verdict, unless the
# operator passes `--keep-env` on `duhem run`.

set -u

TMP="${TMPDIR:-/tmp}"
ISO_HOME="$TMP/duhem-crawlab-home"
BACKEND_PORT="${CRAWLAB_HOST_PORT:-8090}"
UI_PORT="${CRAWLAB_UI_PORT:-5188}"

# Kill the backend and frontend by process group (up.sh setsid's both
# into their own groups so the compiled `core` child / esbuild children
# are reaped with them).
for name in backend frontend; do
  PF="$TMP/duhem-crawlab-${name}.pid"
  if [ -f "$PF" ]; then
    PID=$(cat "$PF")
    if kill -0 "$PID" 2>/dev/null; then
      echo "down.sh: stopping crawlab $name (pgid $PID)"
      kill -- "-$PID" 2>/dev/null || kill "$PID" 2>/dev/null || true
      sleep 1
      kill -9 -- "-$PID" 2>/dev/null || kill -9 "$PID" 2>/dev/null || true
    fi
    rm -f "$PF"
  fi
done

# Backstop: kill leftover listeners on either port (e.g. a `core` binary
# orphaned by an interrupted run). `ss` is best-effort.
for port in "$BACKEND_PORT" "$UI_PORT"; do
  LPID=$(ss -ltnp 2>/dev/null | grep ":${port} " | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2)
  [ -n "$LPID" ] && { echo "down.sh: killing leftover listener on :$port (pid $LPID)"; kill -9 "$LPID" 2>/dev/null || true; }
done

echo "down.sh: removing mongo container"
docker rm -f duhem-crawlab-mongo >/dev/null 2>&1 || true
rm -rf "$ISO_HOME" 2>/dev/null || true
exit 0
