#!/usr/bin/env bash
set -euo pipefail

# The outer manifest owns the CI shard. Regression fixtures invoke a complete
# inner run and must not reuse its shard partition or hosted logical run ID.
unset DUHEM_SHARD DUHEM_RUN_ID
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$repo_root/target/debug/duhem" "$@"
