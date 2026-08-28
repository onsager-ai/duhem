#!/usr/bin/env bash
set -euo pipefail

if (( $# == 0 )); then
    printf 'usage: %s <stage-command> [args...]\n' "$0" >&2
    exit 2
fi

repo_root="$(git rev-parse --show-toplevel)"
scratch_root="$(mktemp -d "${TMPDIR:-/tmp}/duhem-preflight.XXXXXX")"
scratch_tree="$scratch_root/tree"
worktree_added=false

cleanup() {
    if [[ "$worktree_added" == true ]]; then
        git -C "$repo_root" worktree remove --force "$scratch_tree" >/dev/null 2>&1 || true
    fi
    rm -rf "$scratch_root"
}
trap cleanup EXIT

printf 'preflight: fetching origin/main for merge preview\n'
if ! git -C "$repo_root" fetch --quiet origin main; then
    printf 'preflight: FETCH FAILED — refusing branch-only gate; origin/main is required.\n' >&2
    exit 1
fi
main_commit="$(git -C "$repo_root" rev-parse 'FETCH_HEAD^{commit}')"

git -C "$repo_root" worktree add --quiet --detach "$scratch_tree" HEAD
worktree_added=true

merge_output="$scratch_root/merge-output"
if ! git -C "$scratch_tree" merge --no-commit --no-ff "$main_commit" >"$merge_output" 2>&1; then
    printf 'preflight: MERGE PREVIEW CONFLICT — branch does not merge cleanly with fetched origin/main.\n' >&2
    cat "$merge_output" >&2
    exit 1
fi

# Reuse build artifacts without making the preview depend on the branch tree.
mkdir -p "$repo_root/target"
ln -s "$repo_root/target" "$scratch_tree/target"

printf 'preflight: running merged-tree gate against origin/main at %s\n' "$(git -C "$repo_root" rev-parse --short "$main_commit")"
(cd "$scratch_tree" && "$@")
