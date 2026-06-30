#!/bin/sh
# Provision the Crawlab Pro regression cluster ONCE for the suite: a
# crawlab-pro:develop master + worker plus a MongoDB, via `docker compose`
# (see ./docker-compose.yml). Stage 3 of `docs/duhem-spec.md` §9 —
# "Provision Environment".
#
# Why docker compose (not the go-build-from-source recipe used by
# ../../crawlab-spider-task-lifecycle): this suite verifies the REAL Pro
# product as it ships (the `develop` image), needs no crawlab-pro source
# checkout, and mirrors crawlab-team/crawlab-test's own CI blueprint — the
# cleanest reusable shape for a growing regression suite. The from-source
# recipe stays the right tool when you must verify uncommitted source.
#
# LICENSE: the develop image verifies an HS256 JWT against the hardcoded
# dev key `test-secret`. We mint a fresh dev/test license at provision
# time (scripts/mint-license.sh) and inject it as CRAWLAB_LICENSE — never
# hardcoded, never committed. Pass CRAWLAB_LICENSE in the environment to
# override (e.g. a real license in a product env).
#
# Inherits a sanitized env (PATH/HOME/TMPDIR/LANG/LC_*/DUHEM_*). docker +
# docker compose v2 must be on PATH.
#
# Exits non-zero on a hard boot failure (Duhem maps `up:` non-zero to
# Inconclusive(EnvironmentError)).

set -eu

HERE=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
COMPOSE="docker compose -f $HERE/docker-compose.yml -p duhem-crawlab-regression"
HEALTH_URL="http://127.0.0.1:8090/api/health"

# Mint a fresh dev/test license unless the operator supplied one.
if [ -z "${CRAWLAB_LICENSE:-}" ]; then
  CRAWLAB_LICENSE=$(sh "$HERE/mint-license.sh")
  echo "up.sh: minted fresh dev/test license (HS256 / dev key 'test-secret'); len ${#CRAWLAB_LICENSE}"
else
  echo "up.sh: using operator-supplied CRAWLAB_LICENSE; len ${#CRAWLAB_LICENSE}"
fi
export CRAWLAB_LICENSE

echo "up.sh: bringing up crawlab-pro cluster (mongo + master + worker)"
# shellcheck disable=SC2086
$COMPOSE up -d

echo "up.sh: waiting for master health at $HEALTH_URL"
i=0
until [ "$(curl -s -o /dev/null -w '%{http_code}' "$HEALTH_URL" 2>/dev/null)" = "200" ]; do
  i=$((i + 1))
  if [ "$i" -gt 180 ]; then
    echo "up.sh: master did not become healthy within ~180s" >&2
    # shellcheck disable=SC2086
    $COMPOSE logs --tail 40 master >&2 || true
    exit 1
  fi
  sleep 1
done
echo "up.sh: master healthy"

# Wait for the worker to register online in Mongo. The auth/token leaf
# does not exercise the worker, but P0 provisions the full cluster so the
# rest of the regression suite (task-lifecycle, etc.) has it ready. This
# query mirrors the proven recipe in ../../crawlab-spider-task-lifecycle.
echo "up.sh: waiting for worker to register online"
i=0
until [ "$($COMPOSE exec -T mongo mongosh --quiet -u admin -p admin \
  --authenticationDatabase admin crawlab --eval \
  'db.nodes.countDocuments({is_master:false,status:"online"})' 2>/dev/null | tr -dc '0-9')" -ge 1 ] 2>/dev/null; do
  i=$((i + 1))
  if [ "$i" -gt 180 ]; then
    echo "up.sh: worker did not come online within ~180s" >&2
    # shellcheck disable=SC2086
    $COMPOSE logs --tail 40 worker >&2 || true
    exit 1
  fi
  sleep 1
done
echo "up.sh: worker online — Pro regression cluster ready"

exit 0
