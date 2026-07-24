# Contributing to Duhem

Thanks for your interest in Duhem. This guide covers the contribution loop, how to build and test, and the schema-impact discipline. Duhem is Apache-2.0 licensed; see [`LICENSE`](LICENSE).

## The dev loop (spec-issue-driven)

Duhem follows a lightweight spec-driven loop. The short version:

1. **Spec issue.** Open a GitHub issue that states the intent — what "done" means — before writing code. Keep it small and focused on intent over implementation.
2. **Branch.** Cut a branch for the change.
3. **PR.** Open a pull request that references the spec issue (`Closes #N` or `Part of #N`).
4. **Merge.** Land it once the gate is green and review is satisfied.

**Hard rule: no spec, no PR** — with one exception. A genuinely **trivial** change (typo, doc-only, formatting, one-line obvious fix) may skip the spec issue; label the PR `trivial`. When in doubt, write the spec.

If a change introduces new product surface (a new action type, a new schema field, a new CLI command, new judge behavior), the spec must include or link a worked Verification Definition example that exercises it. A surface with no example can't be dogfooded.

## Build and test

Duhem uses [`just`](https://github.com/casey/just) as its task runner over a Cargo workspace.

```sh
just build    # cargo build --workspace
just check    # the pre-push gate: lint + tests
just test     # cargo test --workspace (skips #[ignore]'d tests)
just lint     # fmt --check + clippy -D warnings + file-budget
```

`just check` is what you should run before pushing — it mirrors CI:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- the file-budget check (`xtask check-file-budget`)
- `cargo test --workspace`

Browser-backed (`ui/*`) test lanes need Node ≥ 20 plus the Playwright sidecar's Chromium, installed once per host:

```sh
(cd crates/duhem-actions/sidecar && npm ci && npx playwright install chromium)
```

`just test browser-actions` exercises the generic `ui/*` and `api/observe` action lane. `just dashboard test` exercises the dashboard frontend, crate, and CLI lane. The core `just check` gate does not require either one.

## Schema-impact discipline

Duhem's schema is still **v0.x** — pre-stability, breaking changes are expected before v0.5 — so every schema-touching change is logged. If your change touches the Verification Definition or manifest schema, add an entry under `## Unreleased` in [`CHANGELOG.md`](CHANGELOG.md), tagged with one of:

- **`[breaking]`** — field renamed/removed, action-type removed, semantic change. Bumps the minor (v0.x → v0.x+1).
- **`[additive]`** — new optional field, new action type, new evidence variant. Bumps the patch.
- **`[clarifying]`** — doc-only, error-message wording, internal rename. Does not bump.

The live schema version is the `duhem_schema::SCHEMA_VERSION` constant (surfaced by `duhem --version` and `duhem validate`). `cargo run -p xtask -- schema-changelog-check` verifies the changelog is in order.

## Deeper guides

The repo's development process lives under [`.claude/skills/`](.claude/skills/):

- **`duhem-dev-process`** — the end-to-end SDD loop (spec → branch → implement → PR → merge), the check gate, and the CI-failure table.
- **`verification-authoring`** — how to author Verification Definitions (criteria + checks), enforcing the Holistic Verification Principle and the criteria-vs-checks separation.

## Sign-off / CLA

No CLA and no DCO sign-off are required to contribute. By submitting a contribution you agree it is licensed under the project's Apache-2.0 license.

## Code of Conduct

Be respectful and constructive. We follow the spirit of the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/). Report unacceptable behavior to the maintainers via a GitHub issue or direct contact.
