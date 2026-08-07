# Changelog

All notable schema-impacting changes to Duhem are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the schema-versioning policy (categories, bump cadence, v0.5 readiness
criteria) lives in the spec issue that introduced
`duhem_schema::SCHEMA_VERSION`. Each entry is tagged with one of
`[breaking]`, `[additive]`, or `[clarifying]`:

- **breaking** — field renamed/removed, action-type removed, semantic
  change. Bumps the minor under v0.x (v0.x → v0.x+1).
- **additive** — new optional field, new action type, new evidence
  variant. Bumps the patch (v0.x.y → v0.x.y+1).
- **clarifying** — doc-only, error-message wording, internal rename.
  Does not bump.

`SCHEMA_VERSION` advances in a dedicated bump commit that inserts
`## v0.x.y — YYYY-MM-DD` below `## Unreleased`, leaving `## Unreleased`
empty. The release commit is tagged `v0.x.y`.

**Entry style.** One bullet per landing — the tag, the
consumer-facing change, and (if `[breaking]`) the migration action,
ending in the PR ref `(#N)`. Each entry must fit on one line of at
most 400 characters. Design rationale, metrics, internal file paths,
and "found via" belong in the linked PR, not here.

The pre-consolidation long-form history for v0.1.0–v0.1.8 is frozen in
[`docs/changelog-archive.md`](docs/changelog-archive.md).

## Unreleased

- [clarifying] Semantic validation diagnostics now emit one file-tagged line per error and include exact line and column spans for offending reference values when recoverable; the grouped `N validation error(s):` preamble and count are removed. (#393)
- [additive] Verification Definitions accept an optional opaque `metadata:` map that is recorded in run evidence for downstream consumers and never affects verdicts. (#386)

## v0.2.2 — 2026-08-04

- [additive] Reusable flows accept an optional `description:` that the dashboard prefers for flow group labels while preserving the existing label fallback for undescribed flows. (#387)
- [additive] A run's recorded definition snapshot is now the composed Verification Definition — the leaf plus the effective manifest-level `flows:` and `pages:` it resolved against — so a leaf run by path is self-describing away from its repo, as #302 intended. Snapshot bytes therefore no longer match the source file; parse the snapshot as the effective run definition. (#387)

## v0.2.1 — 2026-08-04

- [additive] `duhem run <leaf-path>` resolves `$pages.*` and `call:` flow references through the nearest ancestor root manifest, matching no-path resolution; the run stays scoped to the requested leaf, and a leaf with unresolved references and no discoverable manifest fails naming exactly what's missing. (#384)

## v0.2.0 — 2026-08-03

- [clarifying] Assertions and verdicts share one three-state vocabulary; `skipped` is an execution fact on steps only, and gating follows judgment rather than a raw `satisfied` observation. (#374)
- [additive] Manifests and VDs accept a `flows:` catalog of parameterized step sequences, invoked from a check with `call:` and expanded into recorded steps. (#376)
- [additive] Manifests and VDs accept a `pages:` catalog of named locators, referenced from steps as `$pages.<page>.<name>`. (#375)
- [breaking] A failing step now blocks the rest of its check by default; steps accept `if: success|always|failure` to opt out. Add `if: always` to steps that must still run after a failure. (#374)
- [additive] Steps accept an optional `description:` — a prose label for what the step is for, shown in the run report and failure detail. (#373)
- [clarifying] New `duhem resolve` prints the fully composed document — merged includes, resolved inputs, and per-value provenance — without executing anything. (#372)
- [breaking] Naming pass: step `within:`→`timeout:`, step `secret:`→`secret_outputs:`, lifecycle `environment:`→`provision:`, `environments:`→`profiles:` (`defaults.profile`, `--profile`), and `inherits: [n]` folded into `inputs: { n: { inherit: true } }`. (#371)

## v0.1.9 — 2026-07-29

- [additive] Checks accept `session:` to seed their browser context from the new `ui/capture-session` action, so login can run once per run instead of once per check. (#347)
- [additive] Runtime lifecycle is separate from judge verdicts, with `running`, `finished`, `aborted`, inferred `orphaned`, heartbeat, and terminal trace events so completed unjudged runs are representable. (#352)
- [additive] Steps may declare `secret:` output paths whose values are masked like secret inputs, keeping credentials acquired at run time out of evidence. (#355)
- [additive] Manifests may declare `inputs:`, allowing credentials shared through `inherits:` to carry `env:` and `secret: true`. (#354)
- [additive] Inputs accept `env:` for sourcing values from process environment variables and `secret: true` for masking recorded or displayed values. (#346)
- [additive] Step-level screenshot storyboards and synchronized browser replay evidence are available. (#337)

## v0.1.8 — 2026-07-24

- _No schema-impacting changes._

## v0.1.7 — 2026-07-24

- [additive] Explicit assertions now record their authored expression and link single-step references to that step, so reporters can show the failed step with its expression and observed-versus-expected reason. (#280)

## v0.1.6 — 2026-07-24

- _No schema-impacting changes._

## v0.1.5 — 2026-07-24

- [additive] In-process progress streaming enables live terminal narration for `duhem run`, automatic on a TTY and controllable with `--live` or `--no-live`. (#299)
- [additive] A serving dashboard advertises its address beside the store, allowing `duhem run` to print a live run link from that address or `DUHEM_DASHBOARD_URL`. (#298)
- [additive] Implicit assertion events link back to their judging step and carry semantic failure details, allowing reporters to mark and explain the failed step. (#280)
- [additive] Assertion events carry their human-readable expression, and run-start events may embed the Verification Definition source for later inspection. (#302)

## v0.1.4 — 2026-07-21

- [additive] `outputs:` bindings now support renames and dotted or indexed extraction paths into raw step results; missing paths remain unrecorded and become inconclusive when referenced. (#273)
- [additive] Declared action outputs can be referenced without identity bindings in `outputs:`; bindings remain available for renames, aliases, and manual judgment control. (#267)
- [additive] `api/stream` declares its existing `stopped_reason` output, making it available to validation, description, and authoring references. (#268)
- [clarifying] Validation warns about redundant identity output bindings and explicit `satisfied == true` plumbing, while authoring guidance teaches the terse forms. (#267)

## v0.1.3 — 2026-07-20

- [additive] `$runtime.contains` supports substring tests on strings while retaining array membership, and assertion type mismatches now gate as failures instead of retryable environment errors. (#259)
- [clarifying] `duhem init` uses the published schema URL outside a checkout, and early Playwright-sidecar exits now suggest running `duhem browser install`. (#260)
- [additive] `Check.assertions` is optional when a judging step supplies `satisfied`; bind it explicitly to opt out, and checks without assertions or judging steps remain invalid. (#253)

## v0.1.2 — 2026-07-17

- [additive] UI locators add `label`, `testid`, `placeholder`, `css`, and standalone `text` strategies, with exactly one primary strategy required and existing locators remaining compatible. (#240)

## v0.1.1 — 2026-07-11

- [clarifying] UI checks can capture WebM video with `--capture-video`; on-failure capture retains video only for failing checks, and the dashboard renders it inline. (#215)
- [clarifying] Failure-envelope APIs return every non-passing check, assertion cause, delivery-web layer chain, capture URL, and first failing network request in one machine-readable document. (#216)
- [clarifying] Failed element assertions record the target rectangle for screenshot highlighting, or explicitly report that the target was not found. (#214)
- [clarifying] The dashboard compares runs through verdict transitions, side-by-side screenshots, network deltas, and an optional non-gating pixel-diff overlay. (#212, #213)
- [clarifying] The run-diff API compares against the latest prior passing run or an explicitly selected baseline and returns honest null baselines when none exists. (#211)
- [clarifying] Dashboard artifacts are inspectable in place through expandable HAR details, sandboxed searchable DOM snapshots, folded step timelines, and image thumbnails. (#210)
- [clarifying] Check evidence renders as a plain-language verdict summary and readable timeline with failing assertion causes, while preserving access to raw JSON. (#206)
- [clarifying] UI checks capture a redacted, size-bounded HAR network log under the existing capture policy. (#204)
- [clarifying] Non-passing UI checks capture screenshots and DOM snapshots by default; migrate authored output aliases away from the reserved `capture/` prefix. (#202)
- [additive] `duhem export` and `duhem ship` share a versioned run-bundle format, and the GitHub action can optionally ship bundles without affecting its verdict gate. (#194)
- [clarifying] Cross-run dashboard views add verification history, failure-first run details, and delivery-web span chains, including static export. (#193)
- [additive] Step-start events carry optional delivery-web layer tags, and the store exposes each check's ordered layer chain while remaining compatible with untagged traces. (#192)
- [additive] Verification Definitions and root manifests may declare one optional `project:` coordinate by repository, URL, image, or ID. (#191)
- [clarifying] The evidence store scopes runs by workspace, project, and verification, records verifier and target provenance, and exposes target status. (#190)
- [breaking] Evidence moves from per-run trace directories to SQLite, and `RunSummary` v2 renames `evidence_dir` to `store`. Migrate run and dashboard `--evidence-dir` to `--db`, use `--run-id` and `duhem export`, replace the action's `evidence-dir` output with `store` plus `run-id`, and update reporter plugins to v2. (#189)
- [additive] `db/observe` repeatedly queries SQL or MongoDB until row-count or row-field conditions pass or its budget expires. (#181)
- [clarifying] The Crawlab regression suite expands API coverage and image selection, fixes PATCH request shape, and `$runtime.exists` now returns false for an absent nested field on a present base. (#160)
- [additive] Mongo `db/query` coerces 24-hex string filters on ID fields to BSON ObjectIds. (#171)
- [additive] `$runtime.contains` and `$runtime.any` provide deterministic array-membership assertions. (#173)
- [additive] `api/call` accepts an optional mapping encoded as a deterministic query string. (#172)
- [clarifying] Chreode verification definitions and supporting documentation replace the former Arbor product name and literals. (#163)
- [clarifying] Specification sections 10.2, 10.3, 10.5–10.7, and 11.1 now match the shipped implementation. (#90, #91, #92, #93, #94, #95)
- [additive] `api/stream` follows SSE or chunked streams until a terminal event or timeout and exposes events, count, and stop reason for assertions. (#153)
- [clarifying] `duhem validate` accepts manifests and directories through the same discovery and loading path as `duhem run`. (#150)
- [clarifying] `duhem run --dry-run` prints resolved input names and values after precedence is applied. (#155)
- [breaking] Run flags `--seed` and `--headed` are removed, and `--inputs-file` is folded into repeatable `--inputs`. Migrate headed runs to `DUHEM_HEADED=1`, input files to `--inputs @file.yml`, and stop using `--seed`. (#151)
- [clarifying] Duhem self-verification adds dashboard and CLI regression suites covering the black-box command and UI contracts. (#148)

## v0.1.0 — 2026-06-29

- [clarifying] The first public release adds open-source README and contribution guidance, adopts Apache-2.0, and distributes the `duhem` CLI through npm and GitHub Releases. (#143)
- [clarifying] CLI manifest discovery walks ancestors, accepts the `.duhem.yml` alias, and supports `-f` overrides. (#69)
- [additive] Root manifests add default environment, timeout, inconclusive, and retry policy, while run summaries expose non-fatal warnings such as accepted inconclusive criteria. (#66)
- [additive] Root manifests compose shared and local configuration through bounded, cycle-checked `includes:` with root-wins merging. (#67)
- [additive] Verification Definitions may `inherit:` shared inputs from a parent manifest environment; unresolved or redeclared inherited inputs fail with an actionable remedy. (#135)
- [additive] Root manifests add named `environments:` with CLI selection, automatic single-environment selection, input precedence, and an environment-variable whitelist. (#68)
- [clarifying] Duhem publishes a JSON Schema for Verification Definition and manifest YAML and emits its schema header from `duhem init`. (#133)
- [clarifying] Validation rejects unresolved references in step payloads while preserving `default()` as the missing-reference escape hatch. (#134)
- [additive] Root manifests may provision one shared environment for the whole suite, with leaf environments suppressed and existing environment-control flags applied at suite scope. (#131)
- [additive] `RunSummary` reports each non-passing check's failed assertions, authored expressions, verdicts, and observed-versus-expected details without changing its additive contract version. (#125, #129)
- [additive] MongoDB `find:` support in `db/query` maps BSON results to judge-comparable JSON while preserving the SQL output contract. (#121)
- [additive] Pure runtime helpers add concatenation, length, case conversion, trimming, replacement, and missing-value defaults. (#119)
- [additive] `$runtime.format` composes scalar values through deterministic ordered placeholders and reports format or type errors mechanically. (#117)
- [additive] `api/poll` retries HTTP requests until a closed response condition passes or a timeout expires and exposes the final status and body for judgment. (#115)
- [clarifying] Environment setup and teardown scripts resolve correctly when a Verification Definition is supplied by relative path. (#110)
- [additive] `cli/invoke` runs a real command with configurable arguments, directory, environment, input, and timeout, exposing exit code and output for assertions. (#102)
- [additive] `db/query` and `db/seed` read and seed real PostgreSQL, MySQL, or SQLite databases and expose rows, row count, or affected rows for assertions. (#101)
- [clarifying] The Playwright sidecar falls back to cached or system Chromium and provides actionable installation or executable-override guidance when no browser is found. (#105)
- [additive] References can navigate nested object fields and array indices, with missing-field and non-navigable causes represented in evidence. (#104)
- [clarifying] Specification sections 10–11 now describe the shipped filter grammar, action payload typing, inconclusive causes, extensibility phase, judge logic, and enterprise roadmap. (#63)
- [clarifying] Pre-push guidance now reflects the repository's actual checks and merge-collision surfaces. (#70)
- [clarifying] `duhem dashboard` serves, live-streams, and statically exports a read-only view of recorded evidence. (#53, #84, #85, #86, #87)
- [additive] Root manifests and multi-leaf runs add polymorphic loading, run-set aggregation, and verification-aware filters. (#49)
- [clarifying] Schema-impact callouts, the live schema-version constant, and schema drift and changelog checks establish v0.x schema discipline. (#51)
- [additive] Verification Definitions add environment setup, teardown, and HTTP readiness hooks with lifecycle evidence and CLI controls. (#50)
- [clarifying] Phase 0 status is synchronized across project documentation and developer guidance. (#62)
- [clarifying] The Onsager dashboard verification targets the workspace-scoped Create Plan flow after its prior target was removed. (#79)
- [clarifying] Browser actions can launch system browsers through executable, channel, and argument environment overrides. (#82)
- [additive] `api/observe` passively records HTTP traffic through Playwright network interception. (#38)
- [additive] Reporter contract v1 adds `RunSummary`, subprocess plugins, and pretty and JUnit reference reporters. (#34)
- [additive] UI actions add typing, selection, URL assertion, and state assertion. (#37)
- [additive] Setup steps add ordered lifecycle evidence and a dedicated output-reference namespace. (#20)
- [additive] `api/call` performs active HTTP requests as the first API action. (#21)
- [breaking] Input types become the closed `string`, `integer`, `number`, `boolean`, `array`, and `object` catalog. Migrate CLI values to declared-type coercion and `Engine::run` callers from string maps to JSON-value maps. (#22)
- [additive] Evidence trace v1 records JSONL events, content-addressed blobs, a manifest, and deterministic replay. (#10)
- [additive] Judge v1 aggregates assertions into pass, fail, or typed inconclusive verdicts at check, criterion, and run levels. (#9)
- [additive] UI navigation, click, and element assertion establish the browser action contract and browser lifetimes. (#12)
- [additive] The initial Verification Definition schema, expression grammar, structural validation, and `duhem validate` CLI are available. (#8)
