#!/bin/sh
# Bring the Onsager dev server up for the create-project verification.
#
# Stage 3 of `docs/duhem-spec.md` §9 — "Provision Environment" — under
# Duhem control via the v1 environment-provisioning spec on
# onsager-ai/duhem#50. This script is the same boot sequence the
# README documented as a manual prerequisite, now wired into the
# Verification Definition so a fresh contributor can run the VD with
# one command and Duhem sequences the lifecycle.
#
# Inherits a sanitized env (PATH, HOME, TMPDIR, LANG, LC_*, DUHEM_*).
# DUHEM_ONSAGER_REPO_DIR (path to an `onsager-ai/onsager` checkout) and
# DUHEM_FIXTURE_DB_URL (Postgres DSN for the dogfood database) are the
# operator-side configuration points; see ../README.md for the
# bootstrap details.
#
# Best-effort: prints what it's doing on stdout and surfaces failures
# via a non-zero exit. Duhem maps `up:` non-zero exit to
# Inconclusive(EnvironmentError) on the run verdict (issue #50 §
# Failure-attribution boundary).

set -eu

ONSAGER_REPO="${DUHEM_ONSAGER_REPO_DIR:-../../../onsager}"
if [ ! -d "$ONSAGER_REPO" ]; then
  echo "up.sh: cannot find Onsager checkout at $ONSAGER_REPO" >&2
  echo "up.sh: set DUHEM_ONSAGER_REPO_DIR to an onsager-ai/onsager clone" >&2
  exit 2
fi

# Seed the dogfood Postgres with the fixture user / project rows the
# VD references. The script is idempotent — re-running it on an
# already-seeded database produces a no-op.
if [ -f "$ONSAGER_REPO/scripts/seed-duhem-fixtures.sh" ]; then
  echo "up.sh: seeding dogfood fixtures"
  ( cd "$ONSAGER_REPO" && ./scripts/seed-duhem-fixtures.sh )
fi

# Boot the Next.js dev server in the background. The readiness probe
# (`environment.ready.http.url`) gates `setup:` on `/healthz`, so the
# VD will wait for the server to come up before any check runs.
#
# `exec` inside the subshell replaces the subshell with the npm
# process, so `$!` captures the dev-server PID (not an intermediate
# subshell that would exit immediately and leave the server
# orphaned beyond `down.sh`'s reach).
echo "up.sh: starting onsager dev server"
( cd "$ONSAGER_REPO" && exec npm run dev >"${TMPDIR:-/tmp}/onsager-dev.log" 2>&1 ) &
echo $! > "${TMPDIR:-/tmp}/onsager-dev.pid"

# Exit zero immediately — Duhem polls readiness asynchronously. The
# server lifetime survives this script because we backgrounded it.
exit 0
