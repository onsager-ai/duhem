# Duhem dev task runner. Rust-only for now; UI targets land with the
# dashboard spec.

default:
    @just --list

# Build the whole workspace.
build:
    cargo build --workspace

# Start the binary in dev mode (placeholder until the CLI grows
# subcommands — `spec(cli): duhem init / validate / run skeletons`).
dev:
    cargo run -p duhem-cli -- --help

# Run all unit + integration tests.
test:
    cargo test --workspace

# Static checks. Mirrors what CI runs.
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo run -p xtask --quiet -- check-file-budget --mode=fail

# Cheap pre-push gate: lint + test.
check: lint test
