# Changelog

All notable schema-impacting changes to Duhem are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is described in `docs/duhem-spec.md` §11.3 (Phase 1 keeps
the schema closed and breaking; deprecation policy lands at v1.0).

## v0.3.0 — evidence trace v1

First on-disk format for verification evidence. One run = one
append-only directory under `.duhem/runs/<run_id>/`; the directory
is the unit of replay. Schema version string `"v1"` is carried in
both `manifest.json` and every `run_started` event (redundantly on
purpose — the manifest can be lost).

### Added

- `trace.jsonl` — append-only structured event stream. One JSON
  object per line; `seq` is monotonic per run (gap = bug); `ts` is
  RFC 3339 with millisecond precision. Unknown `kind` on read is a
  hard error.
- `blobs/<sha256>` — content-addressed blob store. `step_observation`
  values larger than 4 KiB serialized are written here and the event
  carries `blob_sha256` instead of an inline `value`. Writes are
  write-then-rename so the directory is poll-safe.
- `manifest.json` — run-level header: `run_id`, `started_at`,
  `definition_path`, `schema_version`.
- Event kinds (closed set for v1; new kinds in future minor versions
  are additive, existing kinds are stable):
  `run_started`, `step_started`, `step_observation`,
  `step_finished`, `assertion_evaluated`, `check_finished`,
  `criterion_finished`, `run_finished`.
- `EvidenceWriter` — `O_APPEND` writer with the v1 fsync policy:
  fsync at every `*_finished` event, buffer step observations.
- `Trace::open` — reader that fully materializes events and enforces
  `seq` monotonicity on load.
- `replay(trace) -> Result<RunVerdict, ReplayError>` — empirical
  verifier of the §11.2 reproducibility commitment. Re-aggregates
  recorded `assertion_evaluated` outcomes and returns
  `ReplayDivergence` when the recomputed verdict disagrees with the
  recorded `check_finished` / `criterion_finished` / `run_finished`.
  The aggregation rule is a local minimal implementation (fail
  dominates → inconclusive → pass) until `spec(judge): three-state
  verdict aggregation rules` lands and replay delegates to
  `duhem-judge::aggregate_run`.
- `new_run_id()` — ULID generator for `.duhem/runs/<run_id>/`.

### Reserved (not yet emitted)

- `screen_recorded` — video recording, Phase 1+.

### Operator notes

- The run directory defaults to `.duhem/runs/<run_id>/`. Override
  via `--evidence-dir` on `duhem run` (CLI wiring lands with the
  CLI spec).
- Cross-run indexing / dashboard query / cloud upload / retention /
  compaction are explicitly out of scope for v1 (Phase 1+).

## v0.2.0 — ui/* action types v1 (minimal slice)

First entries in the action-type catalog. `Step.uses` is still an
opaque string at the schema layer (#8); these names are not yet
enforced as a closed set — that lands with `spec(schema):
catalog-aware validation`.

### Added

- `ui/navigate` — drive the browser to a URL.
  `with: { url: String, within: Duration? }`. No outputs.
- `ui/click` — click an element via `getByRole`-style locator
  fields. `with: { role, name?, text?, scope?, within? }`. No
  outputs.
- `ui/assert-element` — observe an element's existence/visibility.
  `with: { locator: Locator, expected: ExistenceState, within? }`.
  Native outputs: `satisfied: bool`, `count: u32`.
- `Locator` — `{ role, name?, text?, scope? }` with recursive
  `scope`. Stable shared shape across the `ui/*` catalog.
- `ExistenceState` — closed enum `{ exists, not_exists, visible,
  hidden }`.
- `Action` trait + `ActionCtx` + `ActionResult` + `Outcome` +
  `Observation` — the substrate every catalog entry implements.
- `RunBrowser` / `CheckBrowser` — Playwright lifecycle helpers
  (one Browser per `duhem run`, one Context+Page per check).
  Headless by default; `--headed` opt-out lands with the CLI spec.

### Reserved (not yet implemented)

- `ui/type`, `ui/select`, `ui/assert-url`, `ui/assert-state` —
  declared in `docs/duhem-spec.md` §10.5; same trait, follow-up
  spec.

### Operator notes

- The Playwright Node driver is bundled via the `playwright` crate.
  The browser binary is *not* — run `npx playwright install
  chromium` once before first `duhem run`. `RunBrowser::launch`
  emits the install command on missing-binary errors.
- The `ui_smoke` integration test (Playwright + axum) is
  `#[ignore]`'d by default. `just test-ui` runs it locally.

## v0.1.0 — schema introduced

First on-the-wire shape for a Verification Definition (`Pattern A`,
single-file). No prior version exists; this is a non-breaking initial
release.

### Added

- `VerificationDefinition` — top-level document. Fields: `verification`,
  `spec_ref?`, `inputs?`, `setup?`, `criteria`. Unknown top-level keys
  rejected (`deny_unknown_fields`).
- `InputDecl` — `{ type: String, default?: any }`.
- `Criterion` — `{ id, description, checks }`. `id` is authored and
  required.
- `Check` — `{ id, description?, steps?, assertions }`. `id` is
  authored and required.
- `Step` — `{ id?, uses, with?, outputs? }`. `uses:` is any non-empty
  string at v0.1; the typed action catalog lands in
  `spec(actions): ui/* action types v1`.
- `Assertion` — closed enum of six forms: bare boolean expression,
  `type_check`, `matches`, `in`, `exists`, `equal`. The closed-enum
  shape is the structural enforcer for the *mechanical judgment, not
  LLM judgment* identity commitment (`CLAUDE.md`).
- `TypeCheckKind` — closed enum: `uuid`, `string`, `integer`, `float`,
  `boolean`, `object`, `array`, `null`.
- `Expr` AST + `chumsky`-based parser. Boolean expressions are parsed
  at schema-load time (decision in #8) so syntax errors surface before
  the runtime is invoked. Grammar covers literals, `$steps.*` /
  `$inputs.*` / `$env.*` / `$runtime.*` paths (the four references
  defined in `docs/duhem-spec.md` §10.7), function calls (legal only
  under `$runtime`), comparisons (`== != < <= > >=`), boolean logic
  (`&& || !`), and parens.
- `validate()` — structural validator. Rules: non-empty `criteria`;
  unique `Criterion.id`, `Check.id` per criterion, and `Step.id` per
  check; every `$steps.<id>.outputs.<output>` and `$inputs.<name>`
  resolves against the same definition.
- `SchemaError` — wraps `serde_yml::Error`; preserves line/column on
  parse failures.
- `duhem validate <path>` — CLI subcommand (preview); the full
  `init` / `validate` / `run` surface lives in
  `spec(cli): duhem init / validate / run skeletons`.

### Deferred (named for traceability)

- Action-type catalog (`uses:` is a string today) —
  `spec(actions): ui/* action types v1`.
- Root manifest format (`duhem.yml`) —
  `spec(schema): root manifest v0.1`.
- Expression *evaluation* (paths/calls resolve to live values) —
  `spec(runtime): expression evaluator v1`.
- Assertion *evaluation* and verdict aggregation —
  `spec(judge): three-state verdict aggregation rules`.
