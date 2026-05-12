# Changelog

All notable schema-impacting changes to Duhem are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is described in `docs/duhem-spec.md` §11.3 (Phase 1 keeps
the schema closed and breaking; deprecation policy lands at v1.0).

The product is pre-publication. Everything below accumulates under
`v0.1.0` until first release; spec landings get sub-headings, not
their own minor bumps.

## v0.1.0 — unreleased

### api/* action types v1 (minimal slice — `api/call`)

First entry in the API half of the action-type catalog. The
companion `api/observe` (passive request sniffing) requires
Playwright `Route` plumbing and ships in its own spec; this slice
is `api/call` only.

#### Added

- `api/call` — active HTTP request against a real server. Backed
  by `reqwest` with `rustls-tls`. No mocks: real DNS, real TLS,
  real handler — same Holistic Verification Principle posture as
  the `ui/*` half.
  - `with:` schema (`deny_unknown_fields`):
    `{ method: String, url: String, headers?: Map<String, String>,
       body?: YAML, within?: Duration }`.
    `body:` as a YAML mapping/sequence is serialized as JSON; a
    YAML string is sent verbatim under the author-declared
    `Content-Type`.
  - Native outputs: `status: u16`, `body: JSON Value` (parsed when
    the response `Content-Type` starts with `application/json`,
    `null` otherwise), `body_text: String` (raw bytes UTF-8 lossy),
    `headers: Map<String, String>`. The author references them as
    `$steps.<id>.outputs.<name>`.
- `ActionError::Http(String)` — surfaced for transport-layer
  failures (DNS, TCP, TLS, malformed method, malformed URL). The
  engine maps it to `Outcome::Error`. Timeouts are *not* errors —
  they return `Outcome::Timeout` so the judge maps them to
  `Inconclusive(Timeout)`.

#### Outcome mapping

- HTTP completes within `within:` → `Outcome::Ok`. **Status is data,
  not a verdict** — a `500` response is still `Outcome::Ok` from
  the action's standpoint; assertions are where `200 vs. 500` gets
  judged. Same shape as `ui/click` against a button that triggers
  a 500 page.
- `within:` exceeded → `Outcome::Timeout`.
- HTTP transport error / malformed `with:` → `Outcome::Error`.

#### Registry

- `api/call` is added to the default action registry. Verification
  Definitions using `uses: api/call` move from
  "registry miss → `Inconclusive(MissingObservation)`" (the v1
  shape from `spec(runtime): minimal step executor`) to "runs
  against a real HTTP server."
- The per-check `CheckBrowser` is still opened even on API-only
  checks — every catalog entry registers through the production
  dispatcher whose `requires_page()` defaults to `true`. Stripping
  the browser for API-only Verification Definitions is an
  optimization deferred to a follow-up spec; the cost is one
  Playwright launch per check.

#### Reserved (not yet implemented)

- `api/observe` — passive sniffing of requests the `ui/*` actions
  trigger. Declared in `docs/duhem-spec.md` §10.5; same trait,
  follow-up spec (needs Playwright `Route` / network-interception
  plumbing).

#### Wire format

- No change to `VerificationDefinition`, `Step`, or `Assertion`
  shapes — additive only.
- Workspace `reqwest` dep tightened to
  `default-features = false, features = ["json", "rustls-tls"]`
  so the API client uses rustls end-to-end (no OS-level OpenSSL
  dependency).

#### Operator notes

- No new install step beyond what `ui/*` already requires. The
  `api_smoke` runtime integration test is `#[ignore]`'d because
  the engine still launches Chromium per check (browser-strip
  optimization is the follow-up).

### evidence trace v1

First on-disk format for verification evidence. One run = one
append-only directory under `.duhem/runs/<run_id>/`; the directory
is the unit of replay. Schema version string `"v1"` is carried in
both `manifest.json` and every `run_started` event (redundantly on
purpose — the manifest can be lost).

#### Added

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
  Directory entries (run dir + `blobs/`) fsynced after rename so
  the format survives crash / power loss.
- `Trace::open` — reader that fully materializes events and enforces
  `seq` monotonicity on load. `Trace::read_blob` validates the
  digest is exactly 64 lowercase hex characters before joining the
  path (rejects path-traversal in adversarial traces).
- `replay(trace) -> Result<ReplayedRun, ReplayError>` — empirical
  verifier of the §11.2 reproducibility commitment. Re-aggregates
  recorded `assertion_evaluated` outcomes via
  `duhem-judge::aggregate_run` and returns `ReplayDivergence` when
  the recomputed verdict disagrees with the recorded
  `check_finished` / `criterion_finished` / `run_finished`. Trace
  completeness is enforced: orphan assertions or unfinished
  checks/criteria fail replay rather than silently dropping.
- `new_run_id()` — ULID generator for `.duhem/runs/<run_id>/`.

#### Wire format

- Verdict-bearing fields (`assertion_evaluated.state`,
  `check_finished.verdict`, `criterion_finished.verdict`,
  `run_finished.verdict`) carry `duhem-judge::VerdictState` —
  `"pass"` / `"fail"` / `"inconclusive:<cause>"`. The same wire
  shape as the judge's output, so replay round-trips through the
  canonical aggregator without translation.
- All `ts` values are RFC 3339 with exactly millisecond precision
  (`...:SS.sssZ`), regardless of the wall-clock resolution at
  capture time.

#### Reserved (not yet emitted)

- `screen_recorded` — video recording, Phase 1+.

#### Operator notes

- The run directory defaults to `.duhem/runs/<run_id>/`. Override
  via `--evidence-dir` on `duhem run` (CLI wiring lands with the
  CLI spec).
- Cross-run indexing / dashboard query / cloud upload / retention /
  compaction are explicitly out of scope for v1 (Phase 1+).

### judge: three-state verdict aggregation

First on-the-wire shape for verdicts. The judge is the architectural
enforcement of the *mechanical judgment, not LLM judgment* identity
commitment (`CLAUDE.md`, `docs/duhem-spec.md` §11.2): pure
deterministic aggregation over structured runtime outcomes, no model
in the loop. Wire shape lands now (ahead of the runtime that
produces its inputs) so the surface is stable before evidence and
PR-check rendering hang off it.

#### Added

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

#### Identity-commitment notes

- The `duhem-judge` `Cargo.toml` has a single runtime dependency:
  `serde`. (`serde_json` is a dev-dependency for wire round-trip
  tests.) No HTTP client, no async runtime, no AI SDK — the
  runtime dep tree is auditable as the structural firewall behind
  §11.2. A `cargo-deny` rule formalising this lands in a follow-up.
- Aggregation rules are identical at every level (§7.6) and do not
  try to localise blame within a check; the holistic-verification
  principle (§8) lives in the *absence* of structured-causal
  fields on `AssertionOutcome.detail`.

#### Deferred (named for traceability)

- Producing `AssertionOutcome` from observed state —
  `spec(runtime): expression evaluator v1`.
- Persisting `RunVerdict` to evidence —
  `spec(evidence): append-only run trace v1`.
- Override / escalation policy (§9 Stage 5) — CLI / dashboard
  concern, not the judge's.

### ui/* action types v1 (minimal slice)

First entries in the action-type catalog. `Step.uses` is still an
opaque string at the schema layer (#8); these names are not yet
enforced as a closed set — that lands with `spec(schema):
catalog-aware validation`.

#### Added

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

#### Reserved (not yet implemented)

- `ui/type`, `ui/select`, `ui/assert-url`, `ui/assert-state` —
  declared in `docs/duhem-spec.md` §10.5; same trait, follow-up
  spec.

#### Operator notes

- The Playwright Node driver is bundled via the `playwright` crate.
  The browser binary is *not* — run `npx playwright install
  chromium` once before first `duhem run`. `RunBrowser::launch`
  emits the install command on missing-binary errors.
- The `ui_smoke` integration test (Playwright + axum) is
  `#[ignore]`'d by default. `just test-ui` runs it locally.

### schema introduced

First on-the-wire shape for a Verification Definition (`Pattern A`,
single-file).

#### Added

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

#### Deferred (named for traceability)

- Action-type catalog (`uses:` is a string today) —
  `spec(actions): ui/* action types v1`.
- Root manifest format (`duhem.yml`) —
  `spec(schema): root manifest v0.1`.
- Expression *evaluation* (paths/calls resolve to live values) —
  `spec(runtime): expression evaluator v1`.
- Assertion *evaluation* and verdict aggregation —
  `spec(judge): three-state verdict aggregation rules`.
