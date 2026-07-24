# Duhem dev task runner.

set positional-arguments

default:
    @just --list

# Create an isolated task worktree. Slashes in the branch name become `+`
# only in the directory name (for example, `fix/foo` -> `fix+foo`).
worktree-add branch base="main":
    #!/usr/bin/env bash
    set -euo pipefail
    branch="$1"
    base="$2"
    git check-ref-format --branch "$branch" >/dev/null
    git_common_dir="$(git rev-parse --path-format=absolute --git-common-dir)"
    primary_root="$(dirname "$git_common_dir")"
    repo_name="$(basename "$primary_root")"
    worktree_parent="$(dirname "$primary_root")/${repo_name}-wt"
    worktree_name="${branch//\//+}"
    worktree_path="$worktree_parent/$worktree_name"
    mkdir -p "$worktree_parent"
    git worktree add "$worktree_path" -b "$branch" "$base"
    printf 'Worktree ready: %s\n\n' "$worktree_path"
    printf 'Next:\n  cd %q\n' "$worktree_path"
    printf '  just dev     # run the CLI locally\n'
    printf '  just build\n  just lint\n  just test\n'

# List active worktrees and their branches.
worktree-list:
    git worktree list

# Build the whole workspace.
build:
    cargo build --workspace

# Run the CLI locally; arguments are forwarded (`just dev run ...`).
dev *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if (( $# == 0 )); then
        set -- --help
    fi
    exec cargo run -p duhem-cli -- "$@"

# Run all unit + integration tests (skips `#[ignore]`'d tests).
test:
    cargo test --workspace

# Run the Playwright-backed browser smoke suites (ui/* + api/observe).
# Requires Node >= 20 and, once on the host, the sidecar's deps +
# Chromium:
#   (cd crates/duhem-actions/sidecar && npm ci && npx playwright install chromium)
# CI runs the same suites in the `ui-smoke` lane (spec #77).
test-ui:
    cargo test -p duhem-actions --test ui_smoke --test api_observe_smoke -- --ignored

# Build the dashboard SPA bundle (requires Node >= 20) and embed it
# into the duhem-dashboard binary. Without this, the binary serves a
# placeholder index and the JSON API only.
dashboard-build:
    cd crates/duhem-dashboard/web && npm ci && npm run build
    cargo build -p duhem-dashboard

# Dashboard test lane: SPA component tests + crate tests with the
# real bundle + the `duhem dashboard` end-to-end. Mirrors CI's
# `dashboard` workflow (specs #85/#86/#87).
test-dashboard:
    cd crates/duhem-dashboard/web && npm ci && npm test && npm run build
    cargo test -p duhem-dashboard
    cargo build -p duhem-dashboard -p duhem-cli
    cargo test -p duhem-cli --test dashboard_cmd -- --ignored

# Static checks. Mirrors what CI runs.
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo run -p xtask --quiet -- check-file-budget --mode=fail
    cargo run -p xtask --quiet -- skill-scrub
    cargo run -p xtask --quiet -- dx-drift --mode=warn

# Cheap pre-push gate: lint + test.
check: lint test
