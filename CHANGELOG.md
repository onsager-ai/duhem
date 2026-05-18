# Changelog

All notable schema-impacting changes to Duhem are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is described in `docs/duhem-spec.md` §11.3 (Phase 1 keeps
the schema closed and breaking; deprecation policy lands at v1.0).

The product is pre-publication. Everything below accumulates under
`v0.1.0` until first release; spec landings get sub-headings, not
their own minor bumps.

## v0.1.0 — unreleased

### Reporter contract — v1, plus subprocess plugin loader (#34)

Introduces `RunSummary` as the externally-frozen reporter-plugin
contract and a subprocess-based plugin loader that lets authors
register custom `--reporter <name>` formatters (`pretty`, `junit`,
etc.) without forking the CLI.

#### Reporter contract

- New crate `duhem-summary` carries the contract:

  ```jsonc
  {
    "schema_version": "1",
    "run_id": "01J...",
    "verdict": "pass" | "fail" | "inconclusive:<cause>",
    "criteria": [ { "id": "AC-1", "verdict": "pass" } ],
    "evidence_dir": ".duhem/runs/01J..."
  }
  ```

- `schema_version` is on the wire so a plugin written against today's
  contract can refuse a future shape rather than silently misrender.
  Changes to the contract are schema-impacting and require a new
  `### Reporter contract — vN ...` heading in the current unreleased
  v0.x section, plus a bump of `RunSummary::SCHEMA_VERSION`.

#### Plugin discovery

- Repo config: `.duhem.toml` at cwd or any ancestor.
- User config: `~/.duhem/config.toml`.
- Schema:

  ```toml
  [reporter.pretty]
  command = ["duhem-reporter-pretty"]

  [reporter.junit]
  command = ["python3", "-m", "duhem_reporter_junit"]
  ```

- Resolution order: built-in (`default` / `quiet` / `json`) → repo
  config → user config → error. Built-ins are not shadowable; a config
  entry naming one of them is ignored. Empty `command` is rejected
  at load time.

#### Invocation protocol

- The CLI invokes the plugin's `command` with `stdin` piped, writes
  one line of `RunSummary` JSON, closes `stdin`, captures `stdout` (the
  formatted output), and waits for the process to exit.
- Non-zero exit surfaces as a CLI reporter error (distinct from the
  verification verdict). Stderr is captured and inlined into the error
  message.
- Unknown `--reporter <name>` exits 2 with `unknown reporter: <name>`
  on stderr.

#### Reference plugins

Two new crates ship as separate (optional) binaries — *not* built into
the `duhem` binary; they prove the contract end-to-end:

- `duhem-reporter-pretty` — ANSI 2-column table (criterion id +
  verdict), with a run/evidence header/footer.
- `duhem-reporter-junit` — minimal JUnit XML: one `<testsuite>` per
  run, one `<testcase>` per criterion. `pass` → empty case; `fail` →
  `<failure type="fail"/>`; `inconclusive:<cause>` →
  `<skipped type="<cause>"/>`.

#### Wire format

- `RunSummary` is a new external surface, separate from the
  Verification Definition schema, but stable enough that external
  consumers depend on it. Treated as schema-impacting.
- The built-in `--reporter json` output already matched the
  `RunSummary` shape from #23; this spec adds the `schema_version`
  field (additive) and freezes the rest.
- Breaking change? **no** (additive — `schema_version` is the only new
  field; existing JSON consumers ignore unknown keys).

### ui/* action types — rest-of-slice (#37)

Closes the §10.5 UI catalog by landing the four actions that the
#12 minimal slice reserved. The first Onsager Verification
Definition (#35) calls `ui/type` (typing a project name) and
`ui/assert-url` (matching the post-submit URL); landing these
unblocks that VD from `Inconclusive(MissingObservation)` per #15.

#### Added

- `ui/type` — type into an input addressed by a `Locator`.
  `with: { locator: Locator, text: String, clear?: bool,
  within?: Duration }`. No outputs. `clear: true` (the default)
  replaces existing text via Playwright's `Locator.fill`;
  `clear: false` appends via `Locator.type`. The clear-first default
  matches authoring intuition — "type 'Alice' into the name field"
  usually means *replace*.
- `ui/select` — choose an option in a `<select>` or
  `role=combobox` widget. `with: { locator: Locator, by: <By>,
  within?: Duration }` where `by:` is the tagged union
  `{ value: String } | { label: String } | { index: u32 }`.
  No outputs. Dispatches to Playwright's `selectOption`. The
  `by:` variants are mutually exclusive — setting two of
  `value`/`label`/`index` is a `with:` validation error at
  deserialize time.
- `ui/assert-url` — observe the current page URL. `with: {
  equals?: String, matches?: String, within?: Duration }`. Exactly
  one of `equals:` (literal match) or `matches:` (regex) is
  required; both-set and neither-set are rejected at invocation.
  Outputs: `satisfied: bool`, `actual: String` (the URL observed
  at decision time, included on both pass and timeout for triage).
  Timeout shape: a URL that never matches within `within:` yields
  `Outcome::Timeout` (the assertion didn't reach a conclusive
  positive observation), distinct from `ui/assert-element`'s
  `Outcome::Ok + satisfied: false`. The judge maps it to
  `Inconclusive(Timeout)`.
- `ui/assert-state` — observe a page-level state. `with: { state:
  PageState, marker?: Marker, within?: Duration }` where
  `PageState ∈ { loaded, network_idle, authenticated, signed_out }`.
  `loaded` waits for `document.readyState === 'complete'`;
  `network_idle` waits until the `performance.resource` entry
  count stays flat for 500 ms (heuristic — the Rust playwright
  binding does not expose `waitForLoadState('networkidle')`
  directly); `authenticated`/`signed_out` require
  `marker: { kind: cookie|local_storage, name: String }` and
  strictly observe presence/absence of the named cookie or
  local-storage key. No app-specific logic. Output:
  `satisfied: bool`. Same wait-with-deadline shape as
  `ui/assert-element` (`Outcome::Ok` + `satisfied: false` on miss,
  not `Outcome::Timeout`).
- `Locator` shape unchanged — reused as-is.
- `regex` workspace crate is now a `duhem-actions` runtime
  dependency (compiles `ui/assert-url`'s `matches:` patterns).

#### Registry

- All four actions land in the default registry alongside the
  `ui/*` trio from #12 and `api/call` from #21. The full v1
  catalog is now `api/call`, `ui/assert-element`,
  `ui/assert-state`, `ui/assert-url`, `ui/click`, `ui/navigate`,
  `ui/select`, `ui/type`.

#### Wire format

- No change to `VerificationDefinition`, `Step`, or `Assertion`
  shapes — additive only. Existing VDs and fixtures unchanged.
- Breaking change? **no** (additive).

#### Operator notes

- No new install step beyond what the existing `ui/*` already
  requires (`npx playwright install chromium`). The new
  `ui_smoke` cases are `#[ignore]`'d by default — `just test-ui`
  runs them locally.

### runtime: setup-step ordering (#20)

`VerificationDefinition.setup:` has lived in the schema since #8 but
the runtime added in #15 / #19 walked `def.criteria` only — setup
was silently dropped. This landing defines setup semantics and wires
execution into the `Engine`. Per `docs/duhem-spec.md` §10.3 setup
runs once before the criteria; the failure policy is
three-state-faithful (a setup failure yields `Inconclusive`, not
`Fail` — we couldn't observe the workload in the state the
Verification Definition claims to verify).

#### Added

- Evidence `Setup*` event variants (additive — no existing variant
  changes shape, byte-identical wire for setup-free definitions):
  `setup_started { step_count }`,
  `setup_step_started { step_index, uses, with? }`,
  `setup_step_observation { step_index, output_name, value | blob_sha256 }`,
  `setup_step_finished { step_index, outcome }`,
  `setup_finished { aborted }`. `setup_finished` and
  `setup_step_finished` fsync (same rule as the other `*_finished`
  events).
- `$setup.<step_id>.outputs.<name>` namespace in runtime expressions.
  Run-scoped, read-only across checks. Setup gets its own browser
  context; only named outputs cross the boundary, browser state
  does not.
- `aggregate_run` is defined on empty criterion verdicts as
  `Inconclusive(EmptyAggregation)`. Setup-abort takes that path —
  no criterion runs, and the run-level verdict's
  `InconclusiveCause` preserves the abort trigger: a setup-step
  `Outcome::Timeout` surfaces as `Inconclusive(Timeout)`; an
  `Outcome::Error`, unknown-action step, or missing browser
  surfaces as `Inconclusive(EnvironmentError)`.
- Schema-side support for `$setup.*` in assertion paths and the
  corresponding validator rules: `DuplicateSetupStepId`,
  `UnresolvedSetupStepRef`, `UnresolvedSetupStepOutput`,
  `MalformedSetupRef`.

#### Wire compatibility

- Traces written before this landing contain no `Setup*` events;
  readers (`replay()`) treat absence as "no setup ran" — identical
  to today's behavior.
- New traces from definitions without `setup:` are byte-identical
  to today's. `SetupStarted` is the boundary marker; it is only
  emitted when `def.setup` is non-empty.

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
    `headers: Map<String, String>` (rendered via UTF-8 lossy from
    raw header bytes so non-ASCII / opaque values are preserved).
  - Scalar outputs (`status`, `body_text`) are reachable from
    assertions as `$steps.<id>.outputs.<name>` against the v1
    evaluator. Object / map outputs (`body` when JSON, `headers`)
    land in the evidence trace but are not yet reachable from the
    expression evaluator — `$steps.<id>.outputs.body` surfaces as
    `Inconclusive(MissingObservation)` until the value-model
    extension lands in a follow-up spec. Plan assertions over the
    scalar outputs for now; nested navigation into `body` is the
    follow-up.
  - JSON-body parse failures (server advertised `application/json`
    but emitted unparseable bytes) are surfaced as an
    `api.json_parse_failure` observation on the action result;
    `body` stays `null` and `body_text` carries the raw bytes for
    triage.
  - Template substitution in `Step.with` resolves whole-string
    `$inputs.<name>` and `$runtime.<helper>()` references. String
    concatenation (`$inputs.base + "/path"`) is not supported in
    v1; authors pass the full URL as a single input.
  - Outgoing JSON bodies require string mapping keys. A YAML
    mapping with non-string keys (`{1: "x"}`) yields
    `Outcome::Error` with an explicit message rather than silently
    dropping the entry — the JSON sent on the wire matches what the
    author wrote.
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

### typed input catalog

`InputDecl.type` was an opaque string at v0.1 (#13); CLI `--inputs k=v`
carried unchanged into `RunContext` as a string regardless of the
declared type. This spec promotes `type` to a closed catalog and
teaches the CLI to coerce values per the declaration so authors get
real integers, booleans, arrays, and objects in expressions and
`type_check` assertions.

#### Changed (BREAKING)

- `InputDecl.type` is now a closed catalog
  (`string|integer|number|boolean|array|object`); unknown type names
  are parse errors at `from_yaml_str`. (`docs/duhem-spec.md` §7.5
  / §10.7.)
- CLI `--inputs k=v` coerces `v` per the declared type:
  - `string` — taken literally; no JSON parse.
  - `integer` — parsed as `i64`; fractional rejected.
  - `number` — parsed as integer or `f64`; fractional allowed.
  - `boolean` — only `true` / `false` accepted (strict; rejects
    `1`/`0`/`yes`/`no` per the Alignment decision).
  - `array` / `object` — JSON-parsed; shape-checked against the
    declared kind.
- `Engine::run` input signature: `BTreeMap<String, String>` →
  `BTreeMap<String, serde_json::Value>`. Callers pass typed values;
  the engine no longer collapses everything to `Value::Str`.
- Inputs declared without a `default:` and not supplied on the
  command line now fail before `Engine::run` is invoked
  (`missing required input: <name>`). Inputs not declared but
  passed on the command line fail similarly (`unknown input:
  <name>`).
- Runtime `Value` grows `Array(Vec<Value>)` and
  `Object(BTreeMap<String, Value>)` variants so declared `array` /
  `object` inputs flow through the evaluator. Equality / ordering
  over arrays / objects still surface as `TypeMismatch`; the
  comparison surface is unchanged. `type_check` is the supported
  interaction at v1.

#### Added

- `ValidationError::InputDefaultTypeMismatch { input, declared,
  actual }` — fires when a declared `default:` doesn't structurally
  match its `type:`. Integer defaults under `number` are
  promoted (no error); everything else must match exactly.
- `fixtures/typed-inputs.yml` — worked example exercising all six
  catalog types in declarations and assertions.

#### Migration

- Verification Definitions whose `type:` value is in the catalog
  (the only one used by fixtures today is `string`) work unchanged.
- Out-of-catalog `type:` names become a parse error — fix the
  Verification Definition.
- Callers passing `--inputs count=3` against `type: integer` no
  longer silently see `Value::Str("3")` — assertions like
  `$inputs.count == 3` now compare like-against-like and pass.
- Programmatic callers of `Engine::run` swap their string map for
  `serde_json::Value`.

#### Deferred (named for traceability)

- Optional inputs (`optional: true` / explicit `null`). Today an
  input without `default:` is required; opt-in absence is a follow-
  up spec.
- Refined-type catalog members (`uuid`, `email`, regex-bounded
  strings). `type_check` already covers post-hoc typing; declaring
  them upfront is Phase-2+.
- Schema-driven `--<input-name>` CLI flags. Authors still pass
  `--inputs key=value`.

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
  spec. (Landed in #37.)

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
