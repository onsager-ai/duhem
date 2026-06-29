#!/bin/sh
# Tear the Crawlab Pro cluster back down after the git-root-path verification
# finishes. Best-effort: failures here are recorded as evidence but never
# alter the run verdict.

set -u

TMP="${TMPDIR:-/tmp}"
# Defaults MUST match scripts/up.sh's offset ports. The port backstop below
# blindly kills whatever listens on these ports, so a mismatch here would
# reap an unrelated Crawlab dev stack on the conventional 8090/9666.
PORT="${CRAWLAB_HOST_PORT:-8190}"

# Stop the master and worker. up.sh `setsid`s each into its own process
# group, so killing the group (-PID) reaps the binary and any children.
for PID_FILE in "$TMP/duhem-grp-master.pid" "$TMP/duhem-grp-worker.pid"; do
  if [ -f "$PID_FILE" ]; then
    PID=$(cat "$PID_FILE")
    if kill -0 "$PID" 2>/dev/null; then
      echo "down.sh: stopping crawlab node (pgid $PID)"
      kill -- "-$PID" 2>/dev/null || kill "$PID" 2>/dev/null || true
      sleep 1
      kill -9 -- "-$PID" 2>/dev/null || kill -9 "$PID" 2>/dev/null || true
    fi
    rm -f "$PID_FILE"
  fi
done

# Backstop: kill whatever still listens on the master REST + gRPC ports.
for P in "$PORT" "${CRAWLAB_GRPC_PORT:-9766}"; do
  LPID=$(ss -ltnp 2>/dev/null | grep ":${P} " | grep -oE 'pid=[0-9]+' | head -1 | cut -d= -f2)
  if [ -n "$LPID" ]; then
    echo "down.sh: killing leftover listener on :$P (pid $LPID)"
    kill -9 "$LPID" 2>/dev/null || true
  fi
done

echo "down.sh: removing mongo container"
docker rm -f duhem-crawlab-mongo >/dev/null 2>&1 || true

rm -rf "$TMP/duhem-grp-master-home" "$TMP/duhem-grp-worker-home" 2>/dev/null || true
exit 0
