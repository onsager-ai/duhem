#!/usr/bin/env bash
set -euo pipefail

# Runs the real `duhem validate` once per top-level directory under
# `verifications/` — structural + catalog-aware schema validation
# through the same `discover` + `load` pipeline `duhem run` uses, with
# no environment provisioning and no network/browser/DB access.
#
# Before this gate, 18 of Duhem's 21 in-tree suites were executed by no
# gate at all: only `duhem-cli`, `duhem-dashboard`, and the
# `defaults-example` fixture were ever run, leaving the other suites
# free to rot silently while doubling as authoring documentation people
# copy from. See onsager-ai/duhem#505 §2.9.
#
# Collects every failure instead of stopping at the first, so one run
# reports the full blast radius of a breaking change rather than the
# first casualty.
#
# Usage: scripts/validate-suites.sh [duhem-binary]
# Defaults to the release binary `just build` / CI already produced.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
duhem_bin="${1:-$repo_root/target/release/duhem}"
verifications_dir="$repo_root/verifications"

if [[ ! -x "$duhem_bin" ]]; then
    printf 'validate-suites: %s not found or not executable — build it first (cargo build --release -p duhem-cli)\n' "$duhem_bin" >&2
    exit 1
fi

total=0
failed=0
failed_suites=()

for suite in "$verifications_dir"/*/; do
    suite="${suite%/}"
    name="$(basename "$suite")"
    total=$((total + 1))
    printf '==> duhem validate %s\n' "$name"
    if ! "$duhem_bin" validate "$suite"; then
        failed=$((failed + 1))
        failed_suites+=("$name")
    fi
done

passed=$((total - failed))
printf '\nvalidate-suites: %d suite(s), %d passed, %d failed\n' "$total" "$passed" "$failed"

if (( failed > 0 )); then
    printf 'validate-suites: FAILED suites:\n' >&2
    for name in "${failed_suites[@]}"; do
        printf '  - %s\n' "$name" >&2
    done
    exit 1
fi
