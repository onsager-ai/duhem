# Changelog

All notable schema-impacting changes to Duhem are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is described in `docs/duhem-spec.md` §11.3 (Phase 1 keeps
the schema closed and breaking; deprecation policy lands at v1.0).

## v0.3.0 — judge: three-state verdict aggregation

First on-the-wire shape for verdicts. The judge is the architectural
enforcement of the *mechanical judgment, not LLM judgment* identity
commitment (`CLAUDE.md`, `docs/duhem-spec.md` §11.2): pure
deterministic aggregation over structured runtime outcomes, no model
in the loop. Wire shape lands now (ahead of the runtime that
produces its inputs) so the surface is stable before evidence and
PR-check rendering hang off it.

### Added

- `VerdictState` — closed enum `{ pass, fail, inconclusive(cause) }`
  per §7.6. Doctrinally three-state; not `#[non_exhaustive]`.
- `InconclusiveCause` — `#[non_exhaustive]` closed-at-v1 enum:
  `timeout`, `missing_observation`, `environment_error`,
  `empty_aggregation`. Wire tokens are snake_case.
- `AssertionOutcome` — `{ assertion_index, state, detail? }`. The
  runtime produces these by evaluating each `Assertion`
  (`duhem-schema`) against observed state; the judge consumes them.
- `CheckOutcome` — `{ check_id, assertions: Vec<AssertionOutcome> }`.
  Input to `aggregate_check`.
- `CheckVerdict` — `{ check_id, state }`. Output of
  `aggregate_check`.
- `CriterionVerdict` — `{ criterion_id, state, checks }`.
- `RunVerdict` — `{ state, criteria }`. Top-level output of one
  `duhem run`.
- `aggregate_check` / `aggregate_criterion` / `aggregate_run` —
  identical fold at every level: *any `fail` → fail; any
  `inconclusive` and no `fail` → inconclusive (first cause wins);
  all `pass` → pass*. Empty inputs are defensively
  `inconclusive:empty_aggregation` (the schema validator forbids
  empty `assertions`/`checks`/`criteria`, so this is unreachable in
  a well-formed run).
- Wire format for `VerdictState`: `"pass"`, `"fail"`,
  `"inconclusive:<cause>"`. `Display` and `serde::{Serialize,
  Deserialize}` are symmetric; unknown strings reject.

### Identity-commitment notes

- The `duhem-judge` `Cargo.toml` has a single runtime dependency:
  `serde`. (`serde_json` is a dev-dependency for wire round-trip
  tests.) No HTTP client, no async runtime, no AI SDK — the
  runtime dep tree is auditable as the structural firewall behind
  §11.2. A `cargo-deny` rule formalising this lands in a follow-up.
- Aggregation rules are identical at every level (§7.6) and do not
  try to localise blame within a check; the holistic-verification
  principle (§8) lives in the *absence* of structured-causal
  fields on `AssertionOutcome.detail`.

### Deferred (named for traceability)

- Producing `AssertionOutcome` from observed state —
  `spec(runtime): expression evaluator v1`.
- Persisting `RunVerdict` to evidence —
  `spec(evidence): append-only run trace v1`.
- Override / escalation policy (§9 Stage 5) — CLI / dashboard
  concern, not the judge's.

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
