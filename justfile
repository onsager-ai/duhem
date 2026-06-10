# Duhem dev task runner.

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

# Cheap pre-push gate: lint + test.
check: lint test
