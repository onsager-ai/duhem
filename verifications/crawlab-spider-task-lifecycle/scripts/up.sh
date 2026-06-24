#!/bin/sh
# Bring up a license-free Crawlab master for the create-project
# verification: a throwaway MongoDB container plus Crawlab's
# OPEN-SOURCE core (the `crawlab/` submodule of crawlab-pro) run from
# source with `go run`. The OSS core's `main.go` has no license gate, so
# this boots without a Pro license while serving the exact REST contract
# this VD targets (the Pro `develop` image panics on an empty license —
# see ../README.md for running against Pro instead).
#
# Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment".
#
# Inherits a sanitized env (PATH, HOME, TMPDIR, LANG, LC_*, DUHEM_*).
# Config points:
#   DUHEM_CRAWLAB_REPO_DIR  path to a crawlab-team/crawlab-pro checkout
#   CRAWLAB_HOST_PORT       master REST port (default 8090, off Crawlab's
#                           own 8080 which the dev-proxy Traefik holds)
#   CRAWLAB_MONGO_HOST_PORT host port for the throwaway Mongo (27018)
#
# Exits non-zero on a hard boot failure (Duhem maps `up:` non-zero to
# Inconclusive(EnvironmentError)).

set -eu

REPO="${DUHEM_CRAWLAB_REPO_DIR:-../../../../crawlab-team/crawlab-pro}"
if [ ! -d "$REPO/crawlab/core" ]; then
  echo "up.sh: cannot find the OSS core at $REPO/crawlab/core" >&2
  echo "up.sh: set DUHEM_CRAWLAB_REPO_DIR to a crawlab-team/crawlab-pro clone" >&2
  exit 2
fi
CORE=$(CDPATH= cd -- "$REPO/crawlab/core" && pwd)

PORT="${CRAWLAB_HOST_PORT:-8090}"
MONGO_PORT="${CRAWLAB_MONGO_HOST_PORT:-27018}"
HEALTH_URL="http://127.0.0.1:${PORT}/health"
TMP="${TMPDIR:-/tmp}"
ISO_HOME="$TMP/duhem-crawlab-home"
LOG="$TMP/duhem-crawlab.log"
PID_FILE="$TMP/duhem-crawlab.pid"

# Throwaway MongoDB (no auth → no credential wiring needed).
echo "up.sh: starting mongo on host port $MONGO_PORT"
docker rm -f duhem-crawlab-mongo >/dev/null 2>&1 || true
docker run -d --rm --name duhem-crawlab-mongo -p "${MONGO_PORT}:27017" mongo:5 >/dev/null

# Fresh, isolated Crawlab home so the node registers as MASTER. Crawlab
# persists node identity in `$HOME/.crawlab/config.json`; a host-wide one
# (e.g. a developer's worker node) would otherwise be loaded and the
# master-only spider-admin service would panic. We keep Go's build cache
# pointed at the real HOME so `go run` doesn't re-download / rebuild.
rm -rf "$ISO_HOME" && mkdir -p "$ISO_HOME"
REAL_HOME="$HOME"

echo "up.sh: starting crawlab OSS master on :$PORT (go run; first build may be slow)"
# `setsid` puts `go run` and its compiled `core` child in a fresh
# process group, so down.sh's `kill -- -PID` reaps the whole tree (a
# plain background `&` leaves `core` in the script's group, where it
# survives the group kill).
HOME="$ISO_HOME" \
GOCACHE="${REAL_HOME}/.cache/go-build" \
GOMODCACHE="${REAL_HOME}/go/pkg/mod" \
GOPATH="${REAL_HOME}/go" \
CRAWLAB_NODE_MASTER=Y \
CRAWLAB_SERVER_PORT="$PORT" \
CRAWLAB_MONGO_HOST=localhost \
CRAWLAB_MONGO_PORT="$MONGO_PORT" \
CRAWLAB_MONGO_DB=crawlab \
  setsid sh -c "cd '$CORE' && exec go run . server" >"$LOG" 2>&1 &
echo $! > "$PID_FILE"

echo "up.sh: waiting for crawlab health at $HEALTH_URL"
i=0
until [ "$(curl -s -o /dev/null -w '%{http_code}' "$HEALTH_URL" 2>/dev/null)" = "200" ]; do
  i=$((i + 1))
  if [ "$i" -gt 240 ]; then
    echo "up.sh: crawlab did not become healthy within ~240s; see $LOG" >&2
    tail -30 "$LOG" >&2 || true
    exit 1
  fi
  sleep 1
done
echo "up.sh: crawlab healthy"

exit 0
