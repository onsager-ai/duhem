#!/bin/sh
# Bring up a license-free Crawlab dashboard for the UI verification: a
# throwaway MongoDB, Crawlab's OPEN-SOURCE core (the `crawlab/`
# submodule) run from source as the master (no license gate), and the
# Vue frontend served by Vite pointed at that backend.
#
# Inherits a sanitized env (PATH, HOME, TMPDIR, LANG, LC_*, DUHEM_*).
# Config points:
#   DUHEM_CRAWLAB_REPO_DIR  path to a crawlab-team/crawlab-pro checkout
#   CRAWLAB_HOST_PORT       backend REST port (default 8090)
#   CRAWLAB_UI_PORT         frontend Vite port (default 5188; Crawlab's
#                           Vite default 5173 collides with Onsager)
#   CRAWLAB_MONGO_HOST_PORT throwaway Mongo host port (default 27018)
#
# Exits non-zero on a hard boot failure (Duhem maps `up:` non-zero to
# Inconclusive(EnvironmentError)).

set -eu

REPO="${DUHEM_CRAWLAB_REPO_DIR:-../../../../crawlab-team/crawlab-pro}"
if [ ! -d "$REPO/crawlab/core" ] || [ ! -d "$REPO/crawlab/frontend/crawlab-ui" ]; then
  echo "up.sh: cannot find crawlab OSS core + frontend under $REPO" >&2
  echo "up.sh: set DUHEM_CRAWLAB_REPO_DIR to a crawlab-team/crawlab-pro clone" >&2
  exit 2
fi
CORE=$(CDPATH= cd -- "$REPO/crawlab/core" && pwd)
UI_DIR=$(CDPATH= cd -- "$REPO/crawlab/frontend/crawlab-ui" && pwd)

PORT="${CRAWLAB_HOST_PORT:-8090}"
UI_PORT="${CRAWLAB_UI_PORT:-5188}"
MONGO_PORT="${CRAWLAB_MONGO_HOST_PORT:-27018}"
HEALTH_URL="http://127.0.0.1:${PORT}/health"
UI_URL="http://127.0.0.1:${UI_PORT}/"
TMP="${TMPDIR:-/tmp}"
ISO_HOME="$TMP/duhem-crawlab-home"
REAL_HOME="$HOME"

# --- MongoDB ---------------------------------------------------------
echo "up.sh: starting mongo on host port $MONGO_PORT"
docker rm -f duhem-crawlab-mongo >/dev/null 2>&1 || true
docker run -d --rm --name duhem-crawlab-mongo -p "${MONGO_PORT}:27017" mongo:5 >/dev/null

# --- Backend (OSS core master) ---------------------------------------
# Isolated HOME so the node registers as MASTER (Crawlab persists node
# identity in $HOME/.crawlab); Go's build cache stays at the real HOME.
# setsid → own process group so down.sh's group kill reaps `go run` +
# its compiled `core` child.
rm -rf "$ISO_HOME" && mkdir -p "$ISO_HOME"
echo "up.sh: starting crawlab OSS master on :$PORT (go run; first build may be slow)"
HOME="$ISO_HOME" \
GOCACHE="${REAL_HOME}/.cache/go-build" GOMODCACHE="${REAL_HOME}/go/pkg/mod" GOPATH="${REAL_HOME}/go" \
CRAWLAB_NODE_MASTER=Y CRAWLAB_SERVER_PORT="$PORT" \
CRAWLAB_MONGO_HOST=localhost CRAWLAB_MONGO_PORT="$MONGO_PORT" CRAWLAB_MONGO_DB=crawlab \
  setsid sh -c "cd '$CORE' && exec go run . server" >"$TMP/duhem-crawlab-backend.log" 2>&1 &
echo $! > "$TMP/duhem-crawlab-backend.pid"

# --- Frontend (Vite) -------------------------------------------------
if [ ! -d "$UI_DIR/node_modules" ]; then
  echo "up.sh: installing frontend deps (pnpm install; slow, one-time)"
  ( cd "$UI_DIR" && pnpm install ) >"$TMP/duhem-crawlab-frontend-install.log" 2>&1 || {
    echo "up.sh: frontend pnpm install failed; see $TMP/duhem-crawlab-frontend-install.log" >&2
    exit 1
  }
fi
echo "up.sh: starting crawlab frontend (vite) on :$UI_PORT -> API :$PORT"
VITE_APP_API_BASE_URL="http://127.0.0.1:${PORT}" \
  setsid sh -c "cd '$UI_DIR' && exec pnpm exec vite --port '$UI_PORT' --strictPort --host 127.0.0.1" \
  >"$TMP/duhem-crawlab-frontend.log" 2>&1 &
echo $! > "$TMP/duhem-crawlab-frontend.pid"

# --- Readiness: backend health AND frontend up -----------------------
echo "up.sh: waiting for backend health at $HEALTH_URL"
i=0
until [ "$(curl -s -o /dev/null -w '%{http_code}' "$HEALTH_URL" 2>/dev/null)" = "200" ]; do
  i=$((i + 1))
  if [ "$i" -gt 240 ]; then
    echo "up.sh: backend did not become healthy within ~240s" >&2
    tail -30 "$TMP/duhem-crawlab-backend.log" >&2 || true
    exit 1
  fi
  sleep 1
done
echo "up.sh: backend healthy; waiting for frontend at $UI_URL"
i=0
until [ "$(curl -s -o /dev/null -w '%{http_code}' "$UI_URL" 2>/dev/null)" = "200" ]; do
  i=$((i + 1))
  if [ "$i" -gt 60 ]; then
    echo "up.sh: frontend did not come up within ~60s" >&2
    tail -20 "$TMP/duhem-crawlab-frontend.log" >&2 || true
    exit 1
  fi
  sleep 1
done
echo "up.sh: crawlab dashboard ready"

exit 0
