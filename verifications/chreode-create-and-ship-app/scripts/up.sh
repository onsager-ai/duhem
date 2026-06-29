#!/bin/sh
# Bring Chreode up for the create-and-ship-app verification, in its
# default deterministic mode: FakeAgent (no live LLM, no spend) +
# dry-run deploy drivers (a real local preview server, no cloud).
#
# Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment" — under
# Duhem control via the v1 environment-provisioning mechanism.
#
# Inherits a sanitized env (PATH, HOME, TMPDIR, LANG, LC_*, DUHEM_*).
# DUHEM_CHREODE_REPO_DIR (path to an `onsager-ai/chreode` checkout) is the
# operator-side configuration point; see ../README.md.
#
# Single-port mode: `pnpm build` then `pnpm start` serves the built
# dashboard SPA (with deep-link fallback) AND the API on :4100, so the
# whole VD runs against one origin. Duhem polls `/api/health`
# asynchronously via `environment.ready.http`, so this script exits 0
# once the server is backgrounded; a hard boot failure exits non-zero
# (Duhem maps `up:` non-zero to Inconclusive(EnvironmentError)).

set -eu

CHREODE_REPO="${DUHEM_CHREODE_REPO_DIR:-../../../chreode}"
if [ ! -d "$CHREODE_REPO" ]; then
  echo "up.sh: cannot find Chreode checkout at $CHREODE_REPO" >&2
  echo "up.sh: set DUHEM_CHREODE_REPO_DIR to an onsager-ai/chreode clone" >&2
  exit 2
fi

# Default 4180, not Chreode's own 4100: Onsager (Duhem's first dogfood)
# commonly runs on 4100 + 5173 on the dev machine, and its
# `/api/health` also returns 200 — so 4100 risks driving the wrong app.
CHREODE_PORT="${CHREODE_PORT:-4180}"
HEALTH_URL="http://127.0.0.1:${CHREODE_PORT}/api/health"
LOG="${TMPDIR:-/tmp}/chreode-dev.log"
PID_FILE="${TMPDIR:-/tmp}/chreode-dev.pid"

# Build the web bundle (slow on a cold install) then start the
# single-port server. CHREODE_WEB_DIST flips the server into single-port
# mode (UI + API on one port). We explicitly DO NOT set CHREODE_AGENT or
# CHREODE_DRIVERS: unset = FakeAgent + dry-run, which is what keeps this
# deterministic and free (../chreode packages/chreode/src/drivers/
# resolve.ts, packages/chreode/src/agent.ts). Loopback bind (the
# default CHREODE_HOST=127.0.0.1) means no auth gate.
echo "up.sh: installing + building chreode (this is slow on a cold cache)"
( cd "$CHREODE_REPO" && pnpm install --frozen-lockfile && pnpm build ) >"$LOG" 2>&1 || {
  echo "up.sh: chreode install/build failed; see $LOG" >&2
  exit 1
}

echo "up.sh: starting chreode single-port server on :${CHREODE_PORT}"
(
  cd "$CHREODE_REPO" \
    && CHREODE_WEB_DIST="$PWD/packages/web/dist" CHREODE_PORT="$CHREODE_PORT" \
       exec pnpm start >>"$LOG" 2>&1
) &
echo $! > "$PID_FILE"

# Best-effort readiness wait here too, so a server that dies on boot
# surfaces as a non-zero `up:` rather than a downstream probe timeout.
echo "up.sh: waiting for chreode health at $HEALTH_URL"
i=0
until curl -fsS "$HEALTH_URL" >/dev/null 2>&1; do
  i=$((i + 1))
  if [ "$i" -gt 120 ]; then
    echo "up.sh: chreode did not become healthy within ~120s; see $LOG" >&2
    exit 1
  fi
  sleep 1
done
echo "up.sh: chreode healthy"

exit 0
