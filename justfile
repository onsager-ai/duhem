# Duhem dev task runner.

set positional-arguments

default:
    @printf 'Duhem development\n\n'
    @printf '  just dev [args]  Run the CLI locally\n'
    @printf '  just build                   Build the workspace\n'
    @printf '  just lint                    Run static checks\n'
    @printf '  just test [browser-actions]  Run tests\n'
    @printf '  just check                   Run lint + test (fast inner loop)\n'
    @printf '  just self-verify             Run Duhem against its own VDs\n'
    @printf '  just preflight               Full CI-equivalent gate before pushing\n\n'
    @printf '  just dashboard [dev|build|test]  Develop, build, or test the dashboard\n'
    @printf '  just worktree [add|list]     Manage task worktrees\n'

# Manage isolated task worktrees (`add <branch> [base]` or `list`).
worktree action="help" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    action="$1"
    shift
    usage() {
        printf 'usage: just worktree add <branch> [base]\n' >&2
        printf '       just worktree list\n' >&2
    }
    case "$action" in
        add)
            if (( $# < 1 || $# > 2 )); then
                usage
                exit 2
            fi
            branch="$1"
            base="${2:-main}"
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
            ;;
        list)
            if (( $# != 0 )); then
                usage
                exit 2
            fi
            exec git worktree list
            ;;
        help|-h|--help)
            usage
            ;;
        *)
            printf 'unknown worktree action: %s\n' "$action" >&2
            usage
            exit 2
            ;;
    esac

# Build the whole workspace.
build:
    cargo build --workspace

# Develop, build, or test the dashboard application.
dashboard action="help" *args:
    #!/usr/bin/env bash
    set -euo pipefail
    action=$1
    shift
    case "$action" in
        dev)
            (cd crates/duhem-dashboard/web && npm install)
            cargo run -p duhem-dashboard -- "$@" &
            api_pid=$!
            (cd crates/duhem-dashboard/web && npm run dev) &
            web_pid=$!
            cleanup() {
                kill "$api_pid" "$web_pid" 2>/dev/null || true
                wait "$api_pid" "$web_pid" 2>/dev/null || true
            }
            trap cleanup EXIT INT TERM
            wait -n "$api_pid" "$web_pid"
            ;;
        build)
            if (( $# != 0 )); then
                printf 'usage: just dashboard build\n' >&2
                exit 2
            fi
            (cd crates/duhem-dashboard/web && npm ci && npm run build)
            exec cargo build -p duhem-dashboard
            ;;
        test)
            if (( $# != 0 )); then
                printf 'usage: just dashboard test\n' >&2
                exit 2
            fi
            (cd crates/duhem-dashboard/web && npm ci && npm test && npm run build)
            cargo test -p duhem-dashboard
            cargo build -p duhem-dashboard -p duhem-cli
            exec cargo test -p duhem-cli --test dashboard_cmd -- --ignored
            ;;
        help|-h|--help)
            printf 'usage: just dashboard <dev|build|test> [dashboard options]\n'
            ;;
        *)
            printf 'unknown dashboard action: %s\n' "$action" >&2
            printf 'usage: just dashboard <dev|build|test> [dashboard options]\n' >&2
            exit 2
            ;;
    esac

# Run the CLI locally; arguments are forwarded (`just dev run ...`).
dev *args:
    #!/usr/bin/env bash
    set -euo pipefail
    if (( $# == 0 )); then
        set -- --help
    fi
    if [[ "$1" == "dashboard" ]]; then
        (cd crates/duhem-dashboard/web && npm ci && npm run build)
        cargo build -p duhem-dashboard
    fi
    exec cargo run -p duhem-cli -- "$@"

# Run workspace tests or the browser-action integration lane.
test target="workspace":
    #!/usr/bin/env bash
    set -euo pipefail
    case "$1" in
        workspace)
            exec cargo test --workspace
            ;;
        browser-actions)
            cargo test -p duhem-actions --test ui_smoke --test api_observe_smoke -- --ignored
            exec cargo test -p duhem-runtime --test capture_smoke --test session_injection_smoke -- --ignored
            ;;
        *)
            printf 'unknown test target: %s\n' "$1" >&2
            printf 'usage: just test [browser-actions]\n' >&2
            exit 2
            ;;
    esac

# Static checks, tuned for fast iteration (not CI-equivalent).
lint:
    # NOT the same as CI: `schema-changelog-check` runs here in `--lint`
    # (advisory) mode so an in-progress change is not blocked on a
    # CHANGELOG entry it does not have yet. CI runs it strict.
    # `just preflight` closes that gap.
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo run -p xtask --quiet -- check-file-budget --mode=fail
    cargo run -p xtask --quiet -- schema-changelog-check --lint
    cargo run -p xtask --quiet -- skill-scrub
    cargo run -p xtask --quiet -- dx-drift --mode=warn

# Fast inner-loop gate: lint + test. Green here does NOT imply green CI.
check: lint test

# Run Duhem's own self-verification suite against a freshly built CLI.
self-verify:
    #!/usr/bin/env bash
    set -euo pipefail
    # `just check` does not do this, and it is where real breakage hides:
    # a change can pass every static check and every unit test while
    # breaking the VDs Duhem uses to verify itself. That is exactly what
    # happened on #437 — catalog-aware validation rejected an in-tree
    # example — and only CI caught it.
    cargo build --release -p duhem-cli
    db="$(mktemp -d)/self-verify.db"
    ./target/release/duhem run verifications/duhem-cli \
        --db "$db" \
        --inputs duhem_bin="$PWD/target/release/duhem"

# Full CI-equivalent gate. Slower than `check` on purpose; run before pushing.
preflight: lint test self-verify
    # The strict form of the changelog check, as CI runs it.
    cargo run -p xtask --quiet -- schema-changelog-check
    # docs §10 yaml blocks parse and validate against the live schema.
    # CI runs this in the `rust` job; leaving it out of the gate meant a
    # spec example could break CI while `preflight` stayed green — which
    # is exactly what happened on #453.
    cargo run -p xtask --quiet -- schema-drift
