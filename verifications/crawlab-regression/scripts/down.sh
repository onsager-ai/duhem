#!/bin/sh
# Tear the Crawlab Pro regression cluster back down after the suite
# finishes. Best-effort: failures here are recorded as evidence but never
# alter the run verdict.
#
# Runs after the last leaf regardless of verdict, unless the operator
# passes `--keep-env` on `duhem run` to leave the cluster up for iteration.
# `down -v` also drops the Mongo volume so each suite run starts clean.

set -u

HERE=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
COMPOSE="docker compose -f $HERE/docker-compose.yml -p duhem-crawlab-regression"

echo "down.sh: tearing down crawlab-pro regression cluster"
# shellcheck disable=SC2086
$COMPOSE down -v --remove-orphans || true

exit 0
