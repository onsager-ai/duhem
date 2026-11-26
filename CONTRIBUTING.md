# Contributing to Duhem

Thanks for your interest in Duhem. This guide covers the contribution loop, how to build and test, and the schema-impact discipline. Duhem is Apache-2.0 licensed; see [`LICENSE`](LICENSE).

## The dev loop (spec-issue-driven)

Duhem follows a lightweight spec-driven loop. The short version:

1. **Spec issue.** Open a GitHub issue that states the intent — what "done" means — before writing code. Keep it small and focused on intent over implementation.
2. **Branch.** Cut a branch for the change.
3. **PR.** Open a pull request that references the spec issue (`Closes #N` or `Part of #N`).
4. **Merge.** Land it once the gate is green and review is satisfied.

**Hard rule: no spec, no PR** — with one exception. A genuinely **trivial** change (typo, doc-only, formatting, one-line obvious fix) may skip the spec issue; label the PR `trivial`. When in doubt, write the spec.

If a change adds an action type, schema field, CLI command, or verdict behavior, the spec must include or link a worked Verification Definition that exercises it.

## Build and test

Start with Git, [rustup](https://rustup.rs/), and the [`just` task runner](https://github.com/casey/just). Rustup selects the Rust toolchain and components in [`rust-toolchain.toml`](rust-toolchain.toml) when you run Cargo from the checkout. Install Node.js 24 with npm to reproduce the core CI runs and work on the dashboard.

```sh
just build       # cargo build --workspace
just check       # fast inner loop: lint + tests
just preflight   # check the merge with origin/main after committing
just test        # cargo test --workspace (skips #[ignore]'d tests)
just lint        # static checks (advisory changelog mode)
just self-verify # run Duhem against its own Verification Definitions
```

Run `just preflight` before pushing, with your changes committed and the worktree clean. It fetches `origin/main` and runs the checks on a temporary merge of your branch and that ref. If `origin` points to a fork, sync the fork's `main` with Duhem upstream first.

Use `just check` for quick local feedback. The preflight and CI checks cover more:

- `schema-changelog-check` runs advisory (`--lint`) inside `just lint`
  so an in-progress change is not blocked on a CHANGELOG entry it does
  not have yet. CI runs it strict.
- Preflight runs Duhem's CLI checks against a freshly built binary and validates the Verification Definitions under `verifications/`.

`just preflight` runs:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- the file-budget check (`xtask check-file-budget`)
- `xtask skill-scrub` and `xtask dx-drift`
- `cargo test --workspace`
- `just self-verify` and `just validate-suites`
- `xtask schema-changelog-check` in its strict form
- `xtask schema-drift` — spec examples against the live schema
- `xtask constraint-catalog --mode=fail` — spec catalogs against their code counterparts
- `xtask action-reference --check` — generated reference against action contracts

CI runs its core checks in two jobs using the repository's `duhem.yml`. It also runs separate browser, dashboard, and sharding-example checks. See the [development reference](docs/agent-development-reference.md#contributing) for the CI commands.

Browser-backed (`ui/*`) test lanes need Node ≥ 20 plus the Playwright sidecar's Chromium, installed once per host:

```sh
(cd crates/duhem-actions/sidecar && npm ci && npx playwright install chromium)
```

`just test browser-actions` exercises the generic `ui/*` and `api/observe` action lane. `just dashboard test` exercises the dashboard frontend, crate, and CLI lane. Neither `just check` nor `just preflight` requires either one.

For dashboard development, `just dashboard dev` starts the Rust API on
port 7878 and the Vite frontend with hot reload on port 5173. Dashboard
server options are forwarded, for example `just dashboard dev --db
path/to/duhem.db`.

## Schema-impact discipline

Duhem's schema is still **v0.x** — pre-stability, breaking changes are expected before v0.5 — so every schema-touching change is logged. If your change touches the Verification Definition or manifest schema, add an entry under `## Unreleased` in [`CHANGELOG.md`](CHANGELOG.md), tagged with one of:

- **`[breaking]`** — field renamed/removed, action-type removed, semantic change. Bumps the minor (v0.x → v0.x+1).
- **`[additive]`** — new optional field, new action type, new evidence variant. Bumps the patch.
- **`[clarifying]`** — doc-only, error-message wording, internal rename. Does not bump.

Keep each entry to one landing and at most 400 characters; the frozen pre-consolidation history is in [`docs/changelog-archive.md`](docs/changelog-archive.md).

The live schema version is the `duhem_schema::SCHEMA_VERSION` constant (surfaced by `duhem --version` and `duhem validate`). `cargo run -p xtask -- schema-changelog-check` verifies the changelog is in order.

## Deeper guides

The repo's development process lives under [`.claude/skills/`](.claude/skills/):

- **`duhem-dev-process`** — the end-to-end SDD loop (spec → branch → implement → PR → merge), the check gate, and the CI-failure table.
- **`verification-authoring`** — how to author Verification Definitions (criteria + checks), enforcing the Holistic Verification Principle and the criteria-vs-checks separation.
- **[Getting started](docs/getting-started.md)** — install Duhem, run a check, and write a Verification Definition.

## Sign-off / CLA

No CLA and no DCO sign-off are required to contribute. By submitting a contribution you agree it is licensed under the project's Apache-2.0 license.

## Code of Conduct

Be respectful and constructive. We follow the spirit of the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). Report unacceptable behavior to the maintainers via a GitHub issue or direct contact.
