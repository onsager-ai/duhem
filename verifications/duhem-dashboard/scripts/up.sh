#!/bin/sh
# Bring the duhem-dashboard up for the regression VD (part of #148).
#
# Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment" — under
# Duhem control via the v1 environment-provisioning mechanism
# (onsager-ai/duhem#50). Two jobs:
#
#   1. Produce a REAL run for the dashboard to serve. We run the tiny
#      offline, page-free `fixture/dashboard-fixture.yml` through the
#      real `duhem run` pipeline, which records a genuine run in the
#      production evidence store (#189), pinned to the fixed id
#      `dashboard-fixture-run` via `--run-id` so the VD's API/SPA/SSE
#      URLs are deterministic.
#   2. Launch `duhem dashboard` (serve mode) over that store on a
#      fixed port, backgrounded so it survives this script.
#
# Holistic posture (`docs/duhem-spec.md` §8): the served data is a real
# run's real evidence in a real store, the dashboard is the real binary
# serving its real embedded SPA + JSON API + SSE. Nothing is mocked.
#
# Config (the `up:`/`down:` child runs with a sanitized env — only
# PATH, HOME, TMPDIR, LANG, LC_*, DUHEM_* survive, and VD `inputs` are
# NOT passed to scripts; see `crates/duhem-runtime/src/engine/env.rs`).
# So the binary path and port are taken from DUHEM_* env vars, not VD
# inputs:
#   - DUHEM_BIN             : `duhem` binary under test (default: `duhem`
#                             on PATH). CI points it at the built artifact.
#                             `duhem dashboard` resolves the sibling
#                             `duhem-dashboard` next to this binary.
#   - DUHEM_DASHBOARD_PORT  : listen port (default 7878). MUST match the
#                             port baked into the VD's URL inputs.
#
# cwd is this VD's directory (the runtime anchors `up:`/`down:` there),
# so `fixture/` and the pid/log paths resolve from here.

set -eu

DUHEM_BIN="${DUHEM_BIN:-duhem}"
PORT="${DUHEM_DASHBOARD_PORT:-7878}"
RUN_ID="dashboard-fixture-run"

# Per-port scratch so concurrent ports don't collide; stable across
# re-runs so `down.sh` (a separate process) finds the pid/log.
WORK="${TMPDIR:-/tmp}/duhem-dashboard-vd-${PORT}"
DB="${WORK}/duhem.db"
PID_FILE="${WORK}/dashboard.pid"
LOG="${WORK}/dashboard.log"

# Clean any prior state for this port (idempotent re-runs).
if [ -f "$PID_FILE" ]; then
  OLD=$(cat "$PID_FILE" 2>/dev/null || true)
  if [ -n "${OLD:-}" ]; then
    kill -- -"$OLD" 2>/dev/null || kill "$OLD" 2>/dev/null || true
  fi
  rm -f "$PID_FILE"
fi
rm -rf "$WORK"
mkdir -p "$WORK"

# --- 1a. Start a real IN-FLIGHT run (#298, AC-4) -----------------------
# Backgrounded FIRST so its `started_at` predates the finished fixture
# run below — the list orders newest-first, keeping the finished run at
# body[0] (AC-1's anchor) and this live one at body[1]. It holds a
# `sleep` step open far longer than the suite's wall clock; `down.sh`
# kills the group. Same store, safe concurrently (WAL + busy timeout).
LIVE_RUN_ID="dashboard-live-run"
LIVE_PID_FILE="${WORK}/live-run.pid"
echo "up.sh: starting in-flight run '$LIVE_RUN_ID' (backgrounded sleep VD)"
setsid "$DUHEM_BIN" run fixture/live-fixture.yml --db "$DB" --run-id "$LIVE_RUN_ID" \
  >"${WORK}/live-run.log" 2>&1 &
echo $! > "$LIVE_PID_FILE"
# Let its run header commit before the fixture run starts, so the
# newest-first ordering between the two is deterministic.
sleep 2

# --- 1b. Produce a real FINISHED run -----------------------------------
echo "up.sh: producing fixture run with '$DUHEM_BIN run fixture/dashboard-fixture.yml'"
"$DUHEM_BIN" run fixture/dashboard-fixture.yml --db "$DB" --run-id "$RUN_ID"
echo "up.sh: fixture run recorded as run id '$RUN_ID' in $DB"

# --- 2. Launch the dashboard (serve mode) -----------------------------
# `duhem dashboard` is a thin wrapper that spawns the `duhem-dashboard`
# server as a child, so killing the wrapper alone would orphan the
# listener. Start it in its own process group (`setsid`) and record the
# leader pid; `down.sh` signals the whole group so both die together.
echo "up.sh: starting duhem dashboard on port $PORT (store: $DB)"
setsid "$DUHEM_BIN" dashboard --db "$DB" --port "$PORT" --host 127.0.0.1 \
  >"$LOG" 2>&1 &
echo $! > "$PID_FILE"

# Exit zero — Duhem polls dashboard readiness asynchronously via the
# `environment.ready.http` probe in duhem.yml. The server survives this
# script because it was backgrounded.
exit 0
