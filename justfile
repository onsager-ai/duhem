# Duhem dev task runner.

set positional-arguments

default:
    @printf 'Duhem development\n\n'
    @printf '  just dev [args]  Run the CLI locally\n'
    @printf '  just build                   Build the workspace\n'
    @printf '  just lint                    Run static checks\n'
    @printf '  just test [browser-actions]  Run tests\n'
    @printf '  just check                   Run lint + test before pushing\n\n'
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
            exec cargo test -p duhem-actions --test ui_smoke --test api_observe_smoke -- --ignored
            ;;
        *)
            printf 'unknown test target: %s\n' "$1" >&2
            printf 'usage: just test [browser-actions]\n' >&2
            exit 2
            ;;
    esac

# Static checks. Mirrors what CI runs.
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo run -p xtask --quiet -- check-file-budget --mode=fail
    cargo run -p xtask --quiet -- skill-scrub
    cargo run -p xtask --quiet -- dx-drift --mode=warn

# Cheap pre-push gate: lint + test.
check: lint test
