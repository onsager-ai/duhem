#!/bin/sh
# Bring up a Crawlab **Pro** master + worker cluster for the git-root-path
# "~" regression verification, and pre-seed the master workspace to look
# exactly like a freshly *cloned git repo* whose spider code lives at the
# repository root.
#
# Why the pre-seed (this is the whole point of the VD): the client hit a
# git-backed spider configured with a git root path of "~". Crawlab clones
# such a repo into `<workspace>/<gitId>/` (files at the repo root) but then
# joined the configured root path verbatim, scanning `<workspace>/<gitId>/~`
# — a path that does not exist — so the worker file sync failed with
# "lstat .../~: no such file or directory", no files synced, and the task
# aborted at startup with the misleading
# "fork/exec /usr/bin/bash: no such file or directory".
#
# To reproduce that faithfully the spider's file must live at the repo ROOT
# (`<workspace>/<gitId>/main.sh`), NOT under a `~/` sub-folder. Seeding the
# file through Crawlab's own `files/save` API would instead write it to the
# spider's (broken) root path and so move WITH the bug, masking it. So this
# hook writes the file straight into the master's workspace at the repo
# root, simulating the git clone, and the VD only sets `git_root_path: "~"`
# on the spider. Pre-fix: the scan of `<gitId>/~` misses the repo-root file
# and the task errors. Post-fix: "~" normalizes to the repo root, the file
# syncs, and the task drives to `finished`.
#
# Builds the **Pro** binary from the crawlab-pro checkout (the entrypoint is
# `<repo>/core`, which depends on `<repo>/crawlab/core` via the repo's
# go.work — where the fix lives). Same shape as the sibling
# crawlab-spider-task-lifecycle VD; see its header and ../README.md.
#
# Config points (env):
#   DUHEM_CRAWLAB_REPO_DIR  path to a crawlab-team/crawlab-pro checkout
#   DUHEM_CRAWLAB_LICENSE   REQUIRED. Crawlab Pro verifies a JWT license.
#   DUHEM_CRAWLAB_GIT_ID    24-hex git id to seed + put on the spider
#                           (default below; MUST match duhem.yml's git_id).
#   CRAWLAB_HOST_PORT       master REST port (default 8190)
#   CRAWLAB_MONGO_HOST_PORT host port for the throwaway Mongo (27019)
#   CRAWLAB_GRPC_PORT       master gRPC port (default 9766)
#
# NOTE: defaults are deliberately offset from the sibling lifecycle VD
# (8090/9666/27018) so this VD can run alongside a separate local Crawlab
# dev stack holding the conventional ports. Keep them in sync with duhem.yml.

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

# MUST match the git_id input default in duhem.yml.
GIT_ID="${DUHEM_CRAWLAB_GIT_ID:-6a3b50c0483d52477bebb41a}"

PORT="${CRAWLAB_HOST_PORT:-8190}"
GRPC_PORT="${CRAWLAB_GRPC_PORT:-9766}"
MONGO_PORT="${CRAWLAB_MONGO_HOST_PORT:-27019}"
HEALTH_URL="http://127.0.0.1:${PORT}/health"
API_ENDPOINT="http://127.0.0.1:${PORT}"
TMP="${TMPDIR:-/tmp}"
BIN="$TMP/duhem-crawlab-pro-core"
M_HOME="$TMP/duhem-grp-master-home"
W_HOME="$TMP/duhem-grp-worker-home"
M_WS="$M_HOME/crawlab_workspace"
M_LOGDIR="$M_HOME/task-logs"
W_LOGDIR="$W_HOME/task-logs"
M_LOG="$TMP/duhem-grp-master.log"
W_LOG="$TMP/duhem-grp-worker.log"
M_PID="$TMP/duhem-grp-master.pid"
W_PID="$TMP/duhem-grp-worker.pid"
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

# Isolated HOMEs so master and worker hold distinct node identities.
rm -rf "$M_HOME" "$W_HOME" && mkdir -p "$M_HOME" "$W_HOME" "$M_LOGDIR" "$W_LOGDIR"

# --- Pre-seed the master workspace like a cloned git repo ---------------
# Files live at the repo ROOT (<workspace>/<gitId>/), exactly where Crawlab
# clones a git spider. The spider will set git_root_path: "~" — the bug under
# test is whether the sync still finds these repo-root files.
echo "up.sh: seeding master workspace $M_WS/$GIT_ID (simulated git clone)"
mkdir -p "$M_WS/$GIT_ID"
printf 'echo hello-from-duhem-git-root-path\n' > "$M_WS/$GIT_ID/main.sh"
printf 'scrapy\n' > "$M_WS/$GIT_ID/requirements.txt"

echo "up.sh: starting Pro master on :$PORT (gRPC :$GRPC_PORT)"
HOME="$M_HOME" \
CRAWLAB_LICENSE="$LICENSE" CRAWLAB_NODE_MASTER=Y \
CRAWLAB_WORKSPACE="$M_WS" CRAWLAB_LOG_PATH="$M_LOGDIR" \
CRAWLAB_SERVER_PORT="$PORT" \
CRAWLAB_GRPC_SERVER_PORT="$GRPC_PORT" CRAWLAB_GRPC_PORT="$GRPC_PORT" \
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
CRAWLAB_LOG_PATH="$W_LOGDIR" \
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
