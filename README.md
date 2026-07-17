# Duhem

**Holistic verification for AI-delivered software.**

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![CI](https://github.com/onsager-ai/duhem/actions/workflows/ci.yml/badge.svg)](https://github.com/onsager-ai/duhem/actions/workflows/ci.yml)
[![Schema](https://img.shields.io/badge/schema-v0.1.0-blue.svg)](CHANGELOG.md)

Duhem sits between AI coding agents and production. It captures human intent as acceptance criteria, translates them into mechanically-judged checks that exercise the real delivery web — code + prompts + tool wiring + data + runtime — and gates merge/deploy on the verdict.

Two commitments shape everything:

- **Holistic.** A check exercises the whole delivery web at once. Duhem does not pretend the web decomposes into independently testable units, and it does not mock the web at verification time.
- **Mechanical judgment, not LLM judgment.** AI may help author criteria and checks, and humans review them — but the verdict is produced by deterministic evaluation of structured assertions. There is no LLM in the judging loop.

## Why "Duhem"

Pierre Duhem argued that no scientific hypothesis can be tested in isolation — only the whole web of theory, apparatus, and assumption gets tested. AI delivery is the engineering instance of that thesis: when an agent ships a feature, what gets delivered is code × prompt × tool config × data state × runtime × upstream contracts. Verification must be holistic, mechanical, and structurally independent of the AI making the claim. `docs/duhem-spec.md` Appendix C unpacks the philosophy.

## Install

The `duhem` CLI ships on npm and as prebuilt binaries on GitHub Releases.

```sh
# global install
npm i -g duhem

# or run without installing
npx duhem --version
```

Prebuilt binaries for each platform are attached to every [GitHub Release](https://github.com/onsager-ai/duhem/releases) — download, unpack, and put `duhem` on your `PATH`.

> The npm package and release binaries publish with the **v0.1.0** release. Until then, build from source with a Rust toolchain: `cargo build -p duhem-cli` produces `target/debug/duhem`. Substitute `cargo run -p duhem-cli --` for `duhem` in the commands below.

Running browser-backed `ui/*` checks additionally needs Node ≥ 20 and the Playwright sidecar's Chromium, installed once per host:

```sh
duhem browser install          # installs the sidecar deps + Chromium
```

The `duhem` binary embeds the sidecar source, so `browser install` works wherever `duhem` is on your `PATH` — it materializes the sidecar and runs `npm` + `npx playwright install` for you (add `--with-deps` in CI images to also install Chromium's OS libraries). From a source checkout the equivalent is `(cd crates/duhem-actions/sidecar && npm ci && npx playwright install chromium)`.

`init` and `validate`, and any VD that uses only `api/*` checks, do not need them.

## Quickstart

Scaffold a Verification Definition, validate it, and run one. The scaffold's default check is **browser-free** (an `api/call`), so `duhem run` goes green with only a network connection — no Chromium (add `--kind ui` for the browser-driven variant). New to Duhem? The [**getting-started guide**](docs/getting-started.md) walks this end-to-end and shows how to author your own check. Copy-paste the whole sequence:

```sh
# scaffold a new VD skeleton — a browser-free api/call check that runs
# green against https://example.com (network only, no browser)
duhem init ./verifications/sample --name sample

# structurally validate it, then run it end-to-end
duhem validate ./verifications/sample/duhem.yml
duhem run      ./verifications/sample

# (or run the repo's fully-offline worked example)
duhem run verifications/defaults-example
```

`duhem run` prints a per-leaf line and the run verdict:

```text
checks: pass
pass
```

Preview what a run would execute — without launching anything — with `--dry-run`:

```sh
duhem run verifications/defaults-example --dry-run
```

```text
WOULD RUN: checks::AC-1::AC-1.1
WOULD RUN: checks::AC-2::AC-2.1
```

`duhem run` auto-discovers the manifest: with no path it walks the current directory and its ancestors (capped at the enclosing `.git`) for a `duhem.yml` / `.duhem.yml`, so `cd anywhere-in-the-repo && duhem run` finds the repo-root manifest — same as `git`, `cargo`, `pnpm`. Pass an explicit path to override, or `-f path/to/manifest.yml` for an out-of-tree manifest.

For a real-world example — including the `up:` / `down:` environment hooks Duhem sequences around a check — see [`verifications/duhem-dashboard/`](verifications/duhem-dashboard/). Product suites live co-located in their own repos under `.duhem/` (e.g. `onsager-ai/chreode/.duhem/`); [`templates/product-repo/`](templates/product-repo/) is the drop-in skeleton.

## Core concepts

- **Criteria vs. checks.** *Criteria* are the human commitment about what "done" means; they are stable and survive implementation churn. *Checks* are how Duhem verifies that commitment; they are derivative and may change as the implementation does. Conflating the two is a defect.
- **Verification Definition (VD).** A YAML document (criteria + checks + inputs, optionally `environment` hooks) describing one workload to verify. `duhem init` scaffolds one; `verifications/` holds worked examples.
- **The manifest (`duhem.yml`).** Composes one or more VDs into a suite and carries shared configuration — `defaults:` (timeout / inconclusive policy / retry), `includes:`, `environments:`. A single-file VD *is* a manifest with one leaf.
- **The verdict.** Deterministic aggregation of structured assertions into `pass` / `fail` / `inconclusive`. No LLM in the loop.

The canonical reference is [`docs/duhem-spec.md`](docs/duhem-spec.md) — start with §1 (Why), §4 (Solution Overview), §7 (Core Concepts), §8 (Holistic Verification Principle), and §10 (VD shape).

## CLI surface

```text
duhem init       Scaffold a runnable Verification Definition skeleton
duhem actions    List the built-in action catalog
duhem describe   Print one action's contract (with-fields, outputs, example)
duhem validate   Parse and structurally validate a Verification Definition file
duhem run        Execute a Verification Definition end-to-end
duhem dashboard  Browse run evidence in a read-only web dashboard (serve + static export)
duhem mcp        Serve the action contract + validate over MCP (stdio) for AI authoring
duhem --version  Print the CLI and schema version
```

Run `duhem <command> --help` for the full flag surface (filters, inputs, environment selection, reporters, evidence directory, env-hook control).

## Status

**Phase 0 — Foundation.** The Cargo workspace ships the CLI plus the runtime, judge, schema, actions, evidence, summary, dashboard, and reporter crates. The `ui/*` and `api/*` action families are implemented, environment provisioning (`up:` / `down:` hooks) is wired into the runtime, and the first Onsager dogfood verification runs in-tree.

Schema is **v0.x** — breaking changes are expected before v0.5. The live schema version is the `duhem_schema::SCHEMA_VERSION` constant (surfaced by `duhem --version` and `duhem validate`); per-landing changes are recorded in [`CHANGELOG.md`](CHANGELOG.md). See `docs/duhem-spec.md` §14 for the roadmap.

## Contributing

Contributions are welcome. The dev loop is spec-issue-driven: **no spec, no PR** (with a `trivial` exception for typos and one-line obvious fixes). See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full loop, build/test commands, and schema-impact discipline.

## License

Apache-2.0. See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE).
