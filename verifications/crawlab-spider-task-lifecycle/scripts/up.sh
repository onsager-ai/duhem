#!/bin/sh
# Bring up a Crawlab **Pro** master + worker cluster for the task
# lifecycle verification. Unlike the create-project VD (OSS core, no
# license, scheduling only), execution of a task to a terminal state
# needs Crawlab's gRPC worker coordination — and the gRPC server is
# started only in the Pro layer (`crawlab-pro/core/apps/grpc.go`). So
# this VD runs the **Pro** binary (`crawlab-pro/core`, not the OSS
# `crawlab/core` submodule), as a master plus one worker, both pointed
# at one throwaway MongoDB.
#
# Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment".
#
# Config points (env):
#   DUHEM_CRAWLAB_REPO_DIR  path to a crawlab-team/crawlab-pro checkout
#   DUHEM_CRAWLAB_LICENSE   REQUIRED. Crawlab Pro verifies a JWT license
#                           (HMAC key, no paid server). The repo's own
#                           permanent test license works for local
#                           dogfooding — see ../README.md. Passed via a
#                           DUHEM_* name because Duhem's hook env is
#                           whitelisted to PATH/HOME/TMPDIR/LANG/LC_*/
#                           DUHEM_*. Not committed here: the operator
#                           (who owns the product) supplies it. A bare
#                           CRAWLAB_LICENSE is also honoured for direct
#                           `sh scripts/up.sh` runs.
#   CRAWLAB_HOST_PORT       master REST port (default 8090, off 8080)
#   CRAWLAB_MONGO_HOST_PORT host port for the throwaway Mongo (27018)
#   CRAWLAB_GRPC_PORT       master gRPC port (default 9666)
#
# Exits non-zero on a hard boot failure (Duhem maps `up:` non-zero to
# Inconclusive(EnvironmentError)).

set -eu

REPO="${DUHEM_CRAWLAB_REPO_DIR:-../../../../crawlab-team/crawlab-pro}"
if [ ! -d "$REPO/core" ] || [ ! -f "$REPO/go.work" ]; then
  echo "up.sh: cannot find the Pro core at $REPO/core (need a crawlab-pro checkout)" >&2
  echo "up.sh: set DUHEM_CRAWLAB_REPO_DIR to a crawlab-team/crawlab-pro clone" >&2
  exit 2
fi
PRO_CORE=$(CDPATH= cd -- "$REPO/core" && pwd)

LICENSE="${DUHEM_CRAWLAB_LICENSE:-${CRAWLAB_LICENSE:-}}"
if [ -z "$LICENSE" ]; then
  echo "up.sh: DUHEM_CRAWLAB_LICENSE is required for the Pro stack (see ../README.md)" >&2
  exit 2
fi

PORT="${CRAWLAB_HOST_PORT:-8090}"
GRPC_PORT="${CRAWLAB_GRPC_PORT:-9666}"
MONGO_PORT="${CRAWLAB_MONGO_HOST_PORT:-27018}"
HEALTH_URL="http://127.0.0.1:${PORT}/health"
API_ENDPOINT="http://127.0.0.1:${PORT}"
TMP="${TMPDIR:-/tmp}"
BIN="$TMP/duhem-crawlab-pro-core"
M_HOME="$TMP/duhem-pro-master-home"
W_HOME="$TMP/duhem-pro-worker-home"
M_LOG="$TMP/duhem-pro-master.log"
W_LOG="$TMP/duhem-pro-worker.log"
M_PID="$TMP/duhem-pro-master.pid"
W_PID="$TMP/duhem-pro-worker.pid"
REAL_HOME="$HOME"

# Throwaway MongoDB (no auth → no credential wiring needed).
echo "up.sh: starting mongo on host port $MONGO_PORT"
docker rm -f duhem-crawlab-mongo >/dev/null 2>&1 || true
docker run -d --rm --name duhem-crawlab-mongo -p "${MONGO_PORT}:27017" mongo:5 >/dev/null

# Build the Pro binary once; master and worker share it.
echo "up.sh: building Crawlab Pro core (first build may be slow)"
( cd "$PRO_CORE" && GOCACHE="${REAL_HOME}/.cache/go-build" \
  GOMODCACHE="${REAL_HOME}/go/pkg/mod" GOPATH="${REAL_HOME}/go" \
  go build -o "$BIN" . )

# Isolated HOMEs so master and worker hold distinct node identities
# (Crawlab persists identity under $HOME/.crawlab); Go's caches stay on
# the real HOME so the build above is fast on repeat runs.
rm -rf "$M_HOME" "$W_HOME" && mkdir -p "$M_HOME" "$W_HOME"

echo "up.sh: starting Pro master on :$PORT (gRPC :$GRPC_PORT)"
HOME="$M_HOME" \
CRAWLAB_LICENSE="$LICENSE" CRAWLAB_NODE_MASTER=Y \
CRAWLAB_SERVER_PORT="$PORT" CRAWLAB_GRPC_PORT="$GRPC_PORT" \
CRAWLAB_MONGO_HOST=localhost CRAWLAB_MONGO_PORT="$MONGO_PORT" CRAWLAB_MONGO_DB=crawlab \
  setsid sh -c "exec '$BIN' server" >"$M_LOG" 2>&1 &
echo $! > "$M_PID"

echo "up.sh: waiting for master health at $HEALTH_URL"
i=0
until [ "$(curl -s -o /dev/null -w '%{http_code}' "$HEALTH_URL" 2>/dev/null)" = "200" ]; do
  i=$((i + 1))
  if [ "$i" -gt 240 ]; then
    echo "up.sh: master did not become healthy within ~240s; see $M_LOG" >&2
    tail -30 "$M_LOG" >&2 || true
    exit 1
  fi
  sleep 1
done
echo "up.sh: master healthy"

echo "up.sh: starting Pro worker (gRPC -> localhost:$GRPC_PORT, sync -> $API_ENDPOINT)"
HOME="$W_HOME" \
CRAWLAB_LICENSE="$LICENSE" CRAWLAB_NODE_MASTER=N \
CRAWLAB_GRPC_HOST=localhost CRAWLAB_GRPC_PORT="$GRPC_PORT" \
CRAWLAB_SERVER_PORT="$((PORT + 1))" \
CRAWLAB_API_ENDPOINT="$API_ENDPOINT" \
CRAWLAB_MONGO_HOST=localhost CRAWLAB_MONGO_PORT="$MONGO_PORT" CRAWLAB_MONGO_DB=crawlab \
CRAWLAB_NODE_NAME=duhem-worker01 \
  setsid sh -c "exec '$BIN' server" >"$W_LOG" 2>&1 &
echo $! > "$W_PID"

echo "up.sh: waiting for worker to register online"
i=0
until [ "$(docker exec duhem-crawlab-mongo mongosh --quiet crawlab --eval \
  'print(db.nodes.countDocuments({is_master:false,status:"online"}))' 2>/dev/null)" = "1" ]; do
  i=$((i + 1))
  if [ "$i" -gt 120 ]; then
    echo "up.sh: worker did not come online within ~120s; see $W_LOG" >&2
    tail -30 "$W_LOG" >&2 || true
    exit 1
  fi
  sleep 1
done
echo "up.sh: worker online — Pro cluster ready"

exit 0
