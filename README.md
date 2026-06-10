# Duhem

A holistic verification platform for AI-delivered software. Sits between
AI coding agents and production: captures human intent as acceptance
criteria, translates them into mechanically-judged checks that exercise
the real delivery web (code + prompts + tools + data + runtime), and
gates merge/deploy on the verdict.

> **Status:** Phase 0 — Foundation. The Cargo workspace ships nine
> product crates (`duhem-cli`, `duhem-runtime`, `duhem-judge`,
> `duhem-schema`, `duhem-actions`, `duhem-evidence`, `duhem-summary`,
> `duhem-reporter-pretty`, `duhem-reporter-junit`) plus an internal
> `xtask` build helper; the CLI exposes `init` / `run` / `validate` /
> `--version`; the `ui/*` and `api/*` action families are implemented
> (`ui/navigate`, `ui/click`, `ui/type`, `ui/select`, `ui/assert-*`,
> `api/call`, `api/observe`); environment provisioning (`up:` /
> `down:` hooks) is wired into the runtime; and the first Onsager
> dogfood verification ships in-tree at
> [`verifications/onsager-dashboard-create-spec-plan/`](verifications/onsager-dashboard-create-spec-plan/)
> with a working `duhem/run` composite GitHub Action. Schema is still
> v0.x — breaking changes are expected. See `docs/duhem-spec.md` §14
> for the roadmap and `CHANGELOG.md` for the per-landing ledger.

## Why "Duhem"

Pierre Duhem argued that no scientific hypothesis can be tested in
isolation — only the whole web of theory, apparatus, and assumption
gets tested. AI delivery is the engineering instance of that thesis:
when an agent ships a feature, what gets delivered is code × prompt ×
tool config × data state × runtime × upstream contracts. Verification
must be holistic, mechanical, and structurally independent of the AI
making the claim. `docs/duhem-spec.md` Appendix C unpacks the
philosophy; `docs/duhem-brand.md` shows how the mark visualizes it.

## Quickstart

Prerequisites for `duhem run` (browser-backed `ui/*` checks): Node
≥ 20, plus the Playwright sidecar's deps and Chromium — once per host:

```sh
(cd crates/duhem-actions/sidecar && npm ci && npx playwright install chromium)
```

`init` and `validate` don't need them.

```sh
cargo build --workspace
cargo run -p duhem-cli -- init /tmp/sample --name sample
cargo run -p duhem-cli -- validate /tmp/sample/duhem.yml
cargo run -p duhem-cli -- run /tmp/sample/duhem.yml
```

`duhem init` scaffolds a minimal Verification Definition that passes
on first run against `https://example.com`. Mutate from that
known-good baseline. For a real-world example — including the
`up:` / `down:` environment hooks Duhem sequences around a check —
see [`verifications/onsager-dashboard-create-spec-plan/`](verifications/onsager-dashboard-create-spec-plan/).

## Reading order

1. **`docs/duhem-spec.md`** — canonical product specification. Start
   with §1 (Why), §4 (Solution Overview), §7 (Core Concepts).
2. **`docs/duhem-brand.md`** — the mark, design rationale, and
   relationship to Onsager.
3. **`CLAUDE.md`** (also exposed as `AGENTS.md`) — orientation for
   AI assistants working in this repo: identity commitments, the
   Onsager dogfood seam, and pointers to the dev-process skills.
4. **`.claude/skills/`** — the dev process for working on this repo
   under Claude Code. Start with `duhem-dev-process`.

## Onsager

Duhem's first dogfood customer is [`onsager-ai/onsager`](https://github.com/onsager-ai/onsager).
The relationship and the cross-repo seam are documented in the
`onsager-dogfood` skill and `docs/duhem-spec.md` Appendix D.

## Contributing

Non-trivial changes start as a GitHub spec issue on this repo. See
`.claude/skills/duhem-dev-process/SKILL.md` for the SDD loop and
`.claude/skills/issue-spec/SKILL.md` for spec-issue authoring.
