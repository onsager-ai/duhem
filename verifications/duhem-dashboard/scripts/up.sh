#!/bin/sh
# Bring the duhem-dashboard up for the regression VD (part of #148).
#
# Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment" — under
# Duhem control via the v1 environment-provisioning mechanism
# (onsager-ai/duhem#50). Two jobs:
#
#   1. Produce a REAL run for the dashboard to serve. We run the tiny
#      offline, page-free `fixture/dashboard-fixture.yml` through the
#      real `duhem run` pipeline, which writes a genuine `trace.jsonl`
#      in the production evidence format. Then we rename the produced
#      run directory to the fixed id `dashboard-fixture-run` so the
#      VD's API/SPA/SSE URLs are deterministic.
#   2. Launch `duhem dashboard` (serve mode) over that evidence dir on
#      a fixed port, backgrounded so it survives this script.
#
# Holistic posture (`docs/duhem-spec.md` §8): the served data is a real
# run's real evidence, the dashboard is the real binary serving its
# real embedded SPA + JSON API + SSE. Nothing is mocked.
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
EVIDENCE_DIR="${WORK}/runs"
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
mkdir -p "$EVIDENCE_DIR"

# --- 1. Produce a real run --------------------------------------------
echo "up.sh: producing fixture run with '$DUHEM_BIN run fixture/dashboard-fixture.yml'"
"$DUHEM_BIN" run fixture/dashboard-fixture.yml --evidence-dir "$EVIDENCE_DIR"

# The single-leaf run landed at <EVIDENCE_DIR>/<ulid>/. Find it (the one
# subdir holding a trace.jsonl) and rename to the fixed id.
produced=""
for d in "$EVIDENCE_DIR"/*/; do
  if [ -f "${d}trace.jsonl" ]; then
    produced="$d"
    break
  fi
done
if [ -z "$produced" ]; then
  echo "up.sh: no run directory was produced under $EVIDENCE_DIR" >&2
  exit 1
fi
mv "$produced" "$EVIDENCE_DIR/$RUN_ID"
echo "up.sh: fixture run available as run id '$RUN_ID'"

# --- 2. Launch the dashboard (serve mode) -----------------------------
# `duhem dashboard` is a thin wrapper that spawns the `duhem-dashboard`
# server as a child, so killing the wrapper alone would orphan the
# listener. Start it in its own process group (`setsid`) and record the
# leader pid; `down.sh` signals the whole group so both die together.
echo "up.sh: starting duhem dashboard on port $PORT (evidence: $EVIDENCE_DIR)"
setsid "$DUHEM_BIN" dashboard --evidence-dir "$EVIDENCE_DIR" --port "$PORT" --host 127.0.0.1 \
  >"$LOG" 2>&1 &
echo $! > "$PID_FILE"

# Exit zero — Duhem polls dashboard readiness asynchronously via the
# `environment.ready.http` probe in duhem.yml. The server survives this
# script because it was backgrounded.
exit 0
