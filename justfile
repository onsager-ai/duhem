# Duhem dev task runner. Rust-only for now; UI targets land with the
# dashboard spec.

default:
    @just --list

# Build the whole workspace.
build:
    cargo build --workspace

# Show the CLI's top-level help (`init` / `run` / `validate` / etc.).
dev:
    cargo run -p duhem-cli -- --help

# Run all unit + integration tests (skips `#[ignore]`'d tests).
test:
    cargo test --workspace

# Run the Playwright-backed UI smoke tests. Requires
# `npx playwright install chromium` once on the host first.
test-ui:
    cargo test -p duhem-actions --test ui_smoke -- --ignored

# Static checks. Mirrors what CI runs.
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo run -p xtask --quiet -- check-file-budget --mode=fail

# Cheap pre-push gate: lint + test.
check: lint test
