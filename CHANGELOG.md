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

`SCHEMA_VERSION` advances in a dedicated bump commit that converts
`## Unreleased` to `## v0.x.y — YYYY-MM-DD` and tags the git commit
`schema-v0.x.y`.

**Entry style.** One bullet per landing — the tag, the
consumer-facing change, and (if `[breaking]`) the migration action,
ending in the PR ref `(#N)`. Design rationale, metrics, internal
file paths, and "found via" belong in the linked PR, not here.
Reserve a `### detail` section for the rare landing that genuinely
needs more than a bullet.

## Unreleased

## v0.1.3 — 2026-07-20

- [additive] `$runtime.contains` accepts a string haystack — a literal substring test (`contains($steps.home.outputs.body_text, "Example Domain")`, complementing the regex `$runtime.matches`); an array haystack keeps its element-membership behavior. Separately, an assertion `TypeMismatch` (e.g. `contains(str, int)`, `len(int)`) now gates as `fail` with a `type_mismatch(...)` evidence detail, instead of the retry-eligible `inconclusive:environment_error` it produced before. No schema-shape change. (#259)
- [clarifying] Cleaner cold-start DX (#260): a `duhem init` scaffold created outside a Duhem checkout now writes the published `$schema` URL in its `# yaml-language-server:` header instead of a repo-internal path, so the header resolves for an author who never cloned the repo; and a Playwright sidecar that exits before responding surfaces an actionable `Run \`duhem browser install\` once and retry.` hint instead of a bare `sidecar closed the connection`. No schema impact. (#260)
- [additive] `Check.assertions` is now optional — a *judging* step (its action emits a boolean `satisfied`: `ui/assert-*`, `api/poll`) implicitly asserts `satisfied == true`, dropping the `id` / `outputs` / `== true` plumbing triplet every assert step used to carry. Bind `satisfied` yourself to opt out; a non-passing judging step contributes `inconclusive`, never a silent `pass`. A check with neither `assertions:` nor a judging step is rejected at validate time. Backward compatible; `Check` drops `assertions` from its required set. (#253)

## v0.1.2 — 2026-07-17

- [additive] Locator strategy union (#240): `ui/*` locators gain `label`, `testid`, `placeholder`, `css`, and a standalone `text`, alongside the existing `role` (+`name`) / `text`-filter / `scope` — exactly one primary strategy per locator, enforced at deserialize (two or none is rejected). Backward compatible: every `{role, name, text, scope}` locator maps byte-identically. `schema/duhem.schema.json` is unchanged (the 0.1.1 → 0.1.2 bump is the release version-literal, not a schema-shape event). (#240)

## v0.1.1 — 2026-07-11

- [clarifying] Video recording capture (#215): a `ui/*` check can record a WebM screencast as a `capture/video` blob, opt-in via `duhem run --capture-video` (large blobs; a 25 MiB per-check cap warns-and-skips). Under `--capture on-failure` every ui check records but only failing checks keep the file; the dashboard renders it inline. Evidence only, never a judge input; no wire/`SCHEMA_VERSION` change. (#215)

- [clarifying] Agent failure envelope (#216): new `GET /api/runs/:id/failure` (+ a `/:crit::check` scoped variant) returns one machine-readable document — every non-passing check with its failing assertions + cause, the #192 delivery-web layer chain, the `capture/*` URLs, and the first failing network request from the HAR — so an agent can react to a CI `fail` without scraping the SPA. A passing run is `failing: []`. Evidence only; read-side, no store/migration/`SCHEMA_VERSION` change. Contract at `docs/failure-envelope-contract.md`. (#216)

- [clarifying] Element-highlight on the failure screenshot (#214): a failing `ui/assert-element` records the target locator's bounding box as a `capture/target-rect` blob, and the dashboard overlays it on the screenshot; an absent/invisible target records `found: false` → a "target not found" note, never a guessed box. Evidence only; no wire/`SCHEMA_VERSION` change. (#214)

- [clarifying] Regression diff view + screenshot visual diff (#212, #213): the dashboard renders the #211 run-to-run diff at `/#/run/:id/diff` — verdict transitions (criterion → check → assertion, changed-first) as `baseline → current` badges, screenshots side by side, and a network delta table. #213 adds an on-demand pixel-diff overlay (% of pixels, anti-aliasing tolerance) — evidence only, gates nothing. Honest empty state with no passing baseline. Read-side only, no `SCHEMA_VERSION`/wire change. (#212, #213)

- [clarifying] Run-to-run diff API (#211): new `GET /api/runs/:id/diff` compares a run against its **baseline** — the most recent prior run of the same verification+target that passed (last-pass, reaching back over a failing streak) — surfacing verdict/assertion transitions per criterion/check and each side's `capture/*` refs. Honest `baseline: null` when the verification has never passed; `?baseline=<run-id>` pins a specific run. Evidence only; read-side, no store/migration/`SCHEMA_VERSION` change. Contract at `docs/run-diff-contract.md`. (#211)

- [clarifying] In-page artifact inspection (#210): the dashboard check page makes artifacts inspectable in place — HAR rows expand to redacted request/response headers and bodies, the `capture/dom` snapshot renders in a fully sandboxed `<iframe>` with text search over the source, the timeline folds each step's lifecycle into one collapsible node, and images render as expandable thumbnails. Read-side only, no `SCHEMA_VERSION`/wire change. (#210)

- [clarifying] Human-readable check evidence (#206): the dashboard check page renders a plain-language verdict summary (with each failing assertion's cause) and an `icon · label · detail · Δ` timeline, the raw JSON one click away, plus friendly artifact labels and an inline HAR request table — all derived mechanically over the recorded trace, never LLM-authored. Read-side only, no `SCHEMA_VERSION`/wire change. (#206)

- [clarifying] Network HAR capture (#204): a `ui/*` check records the page's network traffic (last 50 events) as a `capture/network` blob (HAR 1.2) alongside `capture/screenshot`/`capture/dom` under the same `--capture` policy. Secrets are redacted before storage — sensitive headers always, and auth-flow request bodies; bodies over 32 KiB are truncated. No sidecar, wire, or `SCHEMA_VERSION` change. (#204)

- [clarifying] Failure-evidence capture (#202): a `ui/*` check records a full-page screenshot (`capture/screenshot`) and DOM snapshot (`capture/dom`) when it ends non-pass — the first delivery of the §7.7 visual evidence, closing the verify→repair loop. Policy is `duhem run --capture <on-failure|always|off>` (default `on-failure`). The reserved `capture/` output prefix is enforced — `duhem validate` rejects an authored alias under it (new `ValidationError::ReservedOutputPrefix`), so the runtime is the sole source. Evidence only, never judge input; no wire/`SCHEMA_VERSION` change. (#202)

- [additive] Run-bundle wire contract + ingest client (#194): `duhem export` and the new `duhem ship <run-id>` share one versioned format (`bundle_version` 1: run header + #10 event stream + base64 artifacts, content-hash idempotency key). `duhem ship` POSTs the envelope to `$DUHEM_HUB_URL` (`--if-configured` = clean no-op), and the `duhem/run` action gains an opt-in never-gating ship step. Contract at `docs/run-bundle-contract.md`; decoupled from `SCHEMA_VERSION`. (#194)

- [clarifying] Cross-run dashboard views (#193): VD-over-time (`/#/verification/<name>` — per-criterion verdict sparklines across runs; new `GET /api/verifications/:name/history`), a failure-first run view (non-passing checks auto-expand their failing assertions), and the #192 delivery-web span chain on the check page. Read-side only; static export renders all three. No `SCHEMA_VERSION` change. (#193)

- [additive] `spans.layer` trace tags (#192): `step_started`/`setup_step_started` events gain an optional `layer` field (`ui`/`api`/`data`/`runtime`) stamped from the executed action's catalog family (out-of-catalog `uses` stays untagged). The store folds tagged steps into a `spans` table (migration 0003); `Store::check_spans` returns a check's ordered layer chain. Additive to the #10 wire — pre-tag traces load (absent tag = "layer unknown"). No VD-schema change. (#192)

- [additive] `project:` manifest field (#191): a VD or root manifest may declare its target coordinate (`repo:`/`url:`/`image:`/`id:` — exactly one), the top rung of the identity-resolution ladder (declared → CI context → `origin` remote → path fallback) feeding the store's provenance columns (#190). Optional and backward compatible — a VD without it behaves as before. `duhem.schema.json` regenerated. (#191)

- [clarifying] Store scoping + provenance (#190): an additive internal store migration (not the `SCHEMA_VERSION` wire contract) — `workspaces`/`projects`/`verifications` dimension tables, `(workspace, project, verification)` scoping and `(verifier, target)` repo/sha provenance columns on `runs`, and new read queries including `target_status()` (the asymmetric-trust SELECT: a target sha with no recorded pass is blocked). (#190)

- [breaking] Evidence store (#189): the DB is the single source of truth — runs record to a per-working-copy SQLite store (`$XDG_STATE_HOME/duhem/projects/<path-slug>/duhem.db`) instead of per-run `.duhem/runs/<id>/trace.jsonl` files; the #10 trace wire format is unchanged, so `SCHEMA_VERSION` is too. **Migrate:** `duhem run --evidence-dir` → `--db` (+ new `--run-id` and `duhem export <run-id>`); `duhem dashboard --evidence-dir` → `--db`; the `duhem/run` action output `evidence-dir` → `store` + `run-id`. (#189)

### Reporter contract

- [breaking] `RunSummary` v1 → **v2** (#189): `evidence_dir` (per-run trace directory) → `store` (evidence-store DB path). The field a plugin used to locate evidence changed name and meaning; plugins pinned to v1 refuse the new shape loudly. `RunSetSummary` stays v1 (wrapper unchanged).

- [additive] db/observe action — the DB analogue of api/poll: re-runs a db/query (Mongo find: or SQL sql:) on an interval until the rows satisfy an until: condition ({row_count: N} or a {path, equals|matches|exists|gte} predicate over rows[i].field/row_count) or a budget elapses; outputs satisfied/rows/row_count. Lets VDs read-after-settle against eventually-consistent backends (e.g. crawlab's async spider sync, #179) instead of one-shot reads that catch a row mid-write. (#181)

- [clarifying] Crawlab regression suite: nodes & schedules PATCH leaves now send the body under the {data:{...}} wrapper (crawlab's generic create/update contract, same as POST/PUT and the spiders leaf) instead of a top-level body that binds nothing; both leaves now assert a partial PATCH preserves unspecified fields (node is_master, schedule name) — the #128 partial-update claim. (#160)

- [clarifying] $runtime.exists() now returns false (not inconclusive) when a present base has an absent nested field/index (MissingField) — aligns with its documented contract ("false if any underlying lookup reports missing") and the sibling default() helper, so `exists(x) == false` is a usable absent-field assertion (e.g. a password the API must never echo). (#160)

- [additive] db/query Mongo find: coerces 24-hex string filter values on _id/*_id fields to ObjectId, so _id equality matches BSON-typed ids (fixes post-update state reads in Mongo VDs). (#171)

- [additive] $runtime.contains(array, value) + $runtime.any(array, field, value) — array-membership helpers for direct list-contains assertions (pure, deterministic). (#173)

- [additive] api/call: optional query: mapping appended as a deterministic urlencoded query string (no more $runtime.format for pagination/filters). (#172)

- [clarifying] Crawlab regression suite: API-003/008/009/018/019 leaves (users, schedules, nodes, roles, projects+environments). (#160)

- [clarifying] Crawlab regression cluster: select the image under test via DUHEM_CRAWLAB_IMAGE (e.g. crawlabteam/crawlab-pro:test, or a local before/after tag); defaults to :develop. (#160)

- [clarifying] Crawlab regression suite: API-004..007 leaves (spider CRUD, spider files, task CRUD+execution, task logs/results). (#160)

- [clarifying] Crawlab regression suite P0: licensed master+worker cluster shared-environment + API-002 auth/token leaf (verifications/crawlab-regression/). Authored; green run in product env. (#160)

- [clarifying] Chreode VDs: rebrand internal literals to chreode (repo onsager-ai/chreode, CHREODE_* env, ~/.chreode, pnpm chreode) per the full Arbor→Chreode product rename. (#163)

- [clarifying] Renamed product Arbor → Chreode across verifications + docs (verifications/chreode-*). (#163)

- [clarifying] Spec §10.2/§10.3/§10.5/§10.6/§10.7/§11.1 trued to shipped code (audit #63 drift D-2..D-22). (#90,#91,#92,#93,#94,#95)

- [additive] api/stream action: follow an open-ended SSE/chunked stream until a terminal event or within: timeout; outputs events/event_count/last_event for mechanical assertion. Worked example: verifications/duhem-dashboard /live. (#153)

- [clarifying] duhem validate now understands manifests — a manifest file or directory is loaded via the same leaf/manifest/discovery path as run and validated (manifest + its leaves), instead of being mis-parsed as a leaf. (#150)

- [clarifying] duhem run --dry-run now prints a RESOLVED INPUTS block (name = value, post-precedence), enabling value-level input assertions in black-box VDs. (#155)

- [breaking] CLI: removed run flags --seed and --headed (use DUHEM_HEADED=1), and folded --inputs-file into --inputs @file.yml (repeatable, mixable with k=v, last-wins). No VD-schema change; SCHEMA_VERSION unchanged. Breaking CLI surface → next release is a product minor (0.2.0). (#151)

- [clarifying] Duhem-on-Duhem dashboard regression VD (verifications/duhem-dashboard/). (#148)

- [clarifying] Duhem-on-Duhem CLI regression VD (verifications/duhem-cli/) — black-box coverage of the `--version` / `validate` / `init` / `run` contract via cli/invoke; self-verify CI lane. (#148)

## v0.1.0 — 2026-06-29

The first public release: Duhem open-sourced under Apache-2.0 and
distributed as the `duhem` CLI (npm + GitHub Releases). Entries are
one bullet per landing; per-feature detail sections follow the summary.

- [clarifying] Public README + CONTRIBUTING for the open-source release. (#143)

- [clarifying] Project relicensed to Apache-2.0; open-source posture (spec §12/§13/§14). (#143)

- [clarifying] npm + GitHub Releases distribution pipeline for the duhem CLI (release.yml, platform packages). (#143)

- [clarifying] CLI manifest discovery: ancestor walk, .duhem.yml alias, -f override. (#69)

- [additive] root manifest: defaults: block (environment, timeout, inconclusive_policy, retry) — sub-keys fall back to today's behavior (timeout→5s, inconclusive_policy→block, retry.max→0); retry is per-check, retrying only Inconclusive(Timeout|EnvironmentError). (#66)

- [additive] root manifest: includes: block for shared + local config composition — root-wins merge (includes fill only absent keys), verifications concatenated, depth ≤ 3, cycle-detected; PartialRootManifest type added. (#67)

- [additive] VD leaves may declare inherits: [name, ...] to pull shared inputs from the parent manifest's environment chain instead of redeclaring them; $inputs.<name> resolves against inputs ∪ inherits, an inherited name also declared under inputs: is an error, and an unresolved inherited input fails loudly with the suite/--inputs remedy. (#135)

- [additive] Root manifest gains an environments: block (named env configs) injected into leaf input resolution (precedence: --inputs > --inputs-file > selected env > VD default) and the $env whitelist; CLI --environment selects, single env auto-selects. (#68)

- [clarifying] Publish a JSON Schema for VD + manifest YAML (schemars);
  committed at `schema/duhem.schema.json`, `$schema` header emitted by
  `duhem init` and added to one worked-example VD. Purely additive
  tooling: `JsonSchema` derives do not change serialization, so
  `SCHEMA_VERSION` is unchanged. (#133)
- [clarifying] validate now scans step with: payloads for unresolved $refs; a bare missing reference is a hard error at validate time and at runtime (default() escape hatch preserved). (#134)

- [additive] Root manifests may declare a shared `environment:`
  provisioned once for the whole suite (#131): the runtime forks the
  manifest's `up:` (and polls `ready:`) before any leaf runs and forks
  `down:` once after the last leaf, instead of each leaf standing up its
  own stack. Leaf `environment:` blocks are suppressed under a manifest
  environment (the suite owns the stack); `--no-env-up` / `--keep-env`
  apply at the manifest level. Additive: `RootManifest.environment` is
  optional, so manifests without it behave exactly as before. Worked
  example: `verifications/crawlab/` runs N Crawlab VDs against one
  shared stack.

### Reporter contract: defaults warnings (#66)

- [additive] `RunSummary` gains a `warnings` list of non-fatal run notices — currently the `inconclusive_policy: warn` messages (a criterion that aggregated to `inconclusive` but was treated as a pass by the manifest default). Empty by default and `skip_serializing_if` empty, so a warning-free summary serializes byte-for-byte as before; the change is **additive**, `schema_version` stays `"1"`, and an older plugin ignores it. (#66)

### Reporter contract: failure detail (#125, #129)

- [additive] `RunSummary` gains a `failures` list: each non-passing
  check with its failing assertions (the authored `expr`, the
  `verdict`, and a cause `detail` — for a failed comparison the
  observed operands, `actual <lhs>, expected <rhs>`). Lets the
  `default` / `json` / `pretty` reporters show *which* assertion
  failed (and the values) instead of a bare verdict, without the
  author hand-reading `trace.jsonl`. The field is `#[serde(default)]`
  and the change is **additive**, so `schema_version` stays `"1"` and
  an older plugin simply ignores it — only breaking changes bump the
  version. (Policy clarified in #129; the field briefly shipped as a
  `"2"` bump before the additive-no-bump rule was settled.)

- [additive] `db/query` reads MongoDB via a `find:` block on
  `mongodb://` / `mongodb+srv://` connections (#121). The connection-URL
  scheme selects the path: SQL URLs keep `sql:` + `params:`; a Mongo URL
  takes `find:` (`collection` plus optional `filter` / `projection` /
  `sort` / `limit`, written as YAML mappings). The `rows` / `row_count`
  output contract is unchanged, so assertions and #104 nested navigation
  are identical across backends; BSON maps to judge-comparable JSON (an
  `ObjectId` → its 24-hex string, a `DateTime` → RFC3339). `sql`/`find`
  are mutually exclusive per scheme; the wrong pairing is an
  `Outcome::Error`. `db/seed` stays SQL-only. Worked example:
  `verifications/crawlab-create-project/` AC-5 reads Crawlab's real Mongo
  `projects` collection and asserts the REST-created project actually
  persisted (same `_id` the API returned) — the deep DB-state slice the
  REST-only criteria can't reach.
- [additive] More pure `$runtime` helpers (#119): `concat(args...)`
  (join string forms), `len(x)` (array/object element count or string
  char count), `lower`/`upper`/`trim(s)` (case + whitespace
  normalization), `replace(s, from, to)` (literal substring replace),
  `default(value, fallback)` (fallback when `value` is a missing
  reference — absent output/input/env/nested field). All pure and
  deterministic (no I/O / clock / randomness), so §11.2's
  mechanical-judgment and reproducible-run commitments hold; helpers
  compute values, the closed assertion set still decides. `len(x)` over
  a scalar (and `default`'s non-missing errors) surface as
  `type_mismatch`. Spec §10.7 updated. Worked example:
  `verifications/crawlab-create-project/` AC-2 asserts
  `$runtime.len(body.data) >= 1` over the real project array. (#119)
- [additive] `$runtime.format(fmt, args...)` pure helper (#117): `{}`
  placeholders in `fmt` are filled, in order, by the remaining scalar
  args (coerced to string). The sanctioned, identity-preserving way to
  compose a value — notably a dynamic URL from a prior step's output,
  `$runtime.format("{}/{}", $inputs.base, $steps.create.outputs.body.data._id)`
  — without scripting. Pure and deterministic (no I/O / clock /
  randomness), so the mechanical-judgment and reproducible-run
  commitments hold (§11.2); helpers compute values, the closed assertion
  set still decides. The grammar already parsed `$runtime.fn(args)`;
  this implements `format`. New evaluator cause `bad_format` (placeholder
  vs arg-count mismatch; non-scalar args are `type_mismatch`). Worked
  example: `verifications/crawlab-create-project/` AC-4 fetches a created
  project by id at a composed `/projects/<id>` URL. (#117)
- [additive] `api/poll` action (#115): hit an endpoint repeatedly until
  a response condition holds or a timeout elapses — the HTTP analogue of
  `ui/assert-element`, for verifying asynchronous outcomes without a
  flaky fixed `sleep`. `with: { method, url, headers, body, within,
  interval, until }`; `until` is a closed predicate — `{ status: <int> }`
  or `{ path: <json-path>, equals|matches|exists|gte: … }` over the JSON
  body (dotted/bracket path, mirroring #104). Outputs `satisfied` (did
  the condition hold in time), `status`, `body`, `body_text`. Like
  `ui/assert-*`, a completed poll is `Outcome::Ok` with `satisfied`
  true/false (the verdict stays in the judge); a transient request error
  counts as "not yet" so a still-starting service is tolerated. Worked
  example: `verifications/crawlab-create-project/` AC-3 polls the real
  Crawlab project list until the created project appears. (#115)
- [clarifying] Fixed `environment.up:` / `down:` script spawn on a
  relative VD path (#110): `run_script` set `current_dir(vd_dir)` and
  passed the program as `vd_dir.join("./scripts/up.sh")` — a relative
  path that Unix re-resolves against the child's new cwd, doubling to
  `<vd_dir>/<vd_dir>/…` → ENOENT (`env_up exit_code: -2`). Now the
  program is anchored absolutely before spawn while the script still
  runs with cwd = the VD directory. Unblocks all script-based
  provisioning (e.g. `duhem run verifications/<vd>/duhem.yml`). No
  schema or action-contract change. (#110)
- [additive] `cli/invoke` action (#102): run a command-line program in
  the SUT environment and capture `exit_code` / `stdout` / `stderr` for
  assertions. `command` accepts a shell string (run via `sh -c`) or an
  argv array (exec'd directly); optional `cwd`, `env`, `stdin`,
  `within`. Runs the real binary in a sanitized environment (the
  provisioning-script whitelist) — no shimmed shell. A completed
  process is `Outcome::Ok` regardless of exit code (the code is data,
  judged by an assertion); `within:` exceeded → `Outcome::Timeout`; a
  spawn / I/O failure → the new `ActionError::Process`. Worked example:
  `verifications/arbor-factory-cli/` drives Arbor's `pnpm factory` CLI,
  green end-to-end. (#102)
- [additive] `db/query` + `db/seed` actions (#101): read and seed a
  **real** SQL database over `sqlx`'s multi-backend `Any` driver
  (Postgres / MySQL / SQLite, by URL scheme). `db/query` runs a query
  (`?` placeholders bind from `params`) and outputs `rows` (array of
  column→value objects) + `row_count`; `db/seed` runs a multi-statement
  script (DDL + inserts) and outputs `rows_affected`. `connection:` is a
  whole-string URL (`$inputs.db_url` / `$env.DATABASE_URL`); a named-
  `environments:` registry is deferred (#68). New `ActionError::Db`. No
  mock of the store (§8) — SQLite is a real engine, not a double. Pairs
  with #104 so an assertion reaches a column as
  `$steps.q.outputs.rows[0].status`. Worked example:
  `verifications/db-task-state/`, green end-to-end. (#101)
- [clarifying] Playwright sidecar auto-discovers an installed Chromium
  (#105): when `DUHEM_BROWSER_EXECUTABLE` / `DUHEM_BROWSER_CHANNEL` are
  unset and the bundled-browser launch fails, the sidecar finds an
  existing browser — a `chromium-<rev>` build in a Playwright cache, or
  a system Chrome/Chromium on `PATH` — and falls back to it, so a fresh
  `duhem run` drives the UI with no manual config on a host where
  `playwright install` can't fetch a browser (unsupported OS, or a
  cached revision mismatched to this Playwright). If nothing is found,
  the launch error now names both `npx playwright install chromium` and
  the `DUHEM_BROWSER_EXECUTABLE` override. Off the critical path when a
  browser is available; no schema or action-contract change. (#105)
- [additive] Nested navigation into structured values (#104): path
  references may now reach past the bound output into a JSON `object` /
  `array` — `$steps.api.outputs.body.app.id`, `…body.items[0].id` —
  via dotted keys and `[N]` array indices. The grammar gains the
  bracket-index segment; the validator accepts segments past the
  `$steps/$setup.<id>.outputs.<output>` and `$inputs.<name>` address;
  the evaluator walks them, disambiguating key-vs-index by the value's
  shape. Two new evaluator inconclusive causes — `missing_field(path)`
  (absent key / out-of-range index) and `not_navigable(shape, segment)`
  (descending into a scalar) — surfaced in evidence `detail` and folded
  into the judge's coarse `missing_observation` / `environment_error`
  buckets. Backward compatible: every previously-valid path resolves
  identically; previously these deeper paths were a parse/validation
  error. (#104)
- [clarifying] §10–§11 audit (#63) clarifying bundle: trued six spec
  prose claims to the shipped code — §10.1 `--filter` example
  (`login::*` + three-axis grammar note, D-1), §10.5 `with:` typing
  (per-action `With` struct, untyped dispatch boundary, D-14), §10.6
  closed inconclusive-cause catalog (D-16), §10.8 extensibility as a
  Phase-2+ goal with a closed v0.1 catalog (D-20), §11.2 judge-logic
  documentation pointer (§10.6/§10.7 + crate comments, D-23) and the
  enterprise self-hosted judge as Phase-2+ (D-24). Docs only; no
  schema change. Breaking/additive drift tracked in #90–#95. (#63)
- [clarifying] Pre-push skill sync: replaced the stale "Phase 0 is
  near-empty / toolchain intentionally thinner" framing in
  `.claude/skills/duhem-pre-push/SKILL.md` with the recipes Duhem
  actually runs — `just check` as the default gate, plus
  `cargo xtask schema-drift` / `schema-changelog-check` (schema PRs),
  `duhem validate` (VD PRs), and `just test-ui` (UI PRs); refreshed
  the merge-collision patterns that referenced not-yet-existing
  surfaces. Docs only; no schema change. (#70)
- [clarifying] Added `duhem dashboard` (serve + `export`) — the
  `duhem-dashboard` crate: a read-only viewer over `.duhem/runs/`
  evidence (JSON API + Vite/React SPA + static export + SSE live
  streaming of in-progress runs). Consumes the existing evidence (#10)
  and `RunSummary` (#34) shapes as-is; no VD / trace / judge schema
  change. (#53, #84, #85, #86, #87)
- [additive] Root manifest schema (`manifest_version: 1`) and multi-leaf
  `duhem run`; `duhem_schema::load` polymorphic on leaf vs manifest vs
  directory; `aggregate_run_set` / `RunSetVerdict` / `RunSetSummary`;
  `--filter` gains optional `<verification>::` axis. Patterns B and C
  from §10.4 are now executable. (#49)
- [clarifying] formalized Schema impact callout shape; introduced
  `duhem_schema::SCHEMA_VERSION` constant (surfaced via
  `duhem --version` and `duhem validate`'s error preamble) and
  `cargo xtask schema-drift` / `cargo xtask schema-changelog-check`
  CI gates. (#51)
- [additive] Environment provisioning v1 — optional `environment:`
  block on `VerificationDefinition` with operator-supplied `up:` /
  `down:` scripts + HTTP readiness probe; new `EventPayload::Env*`
  evidence variants; `--no-env-up` / `--keep-env` CLI flags;
  sanitized child env. (#50)
- [clarifying] Phase 0 status sync across README, CLAUDE.md,
  dev-process skill, justfile, spec §14, Appendix D. (#62)
- [clarifying] Refreshed the Onsager dogfood VD: retired
  `onsager-dashboard-create-project` (its target feature was removed
  from Onsager) and replaced it with
  `onsager-dashboard-create-spec-plan`, targeting the workspace-scoped
  Create Plan flow. No schema surface touched — existing `ui/*`
  actions only. (#79)
- [clarifying] Playwright sidecar honors `DUHEM_BROWSER_EXECUTABLE` /
  `DUHEM_BROWSER_CHANNEL` / `DUHEM_BROWSER_ARGS` to launch a system
  browser where Playwright ships no bundled Chromium. Additive and
  off by default; no schema or action-contract change. Unblocked the
  first live green run of the dogfood VD. (#82)

- [additive] Root manifest schema (`manifest_version: 1`) and multi-leaf
  `duhem run`; `duhem_schema::load` polymorphic on leaf vs manifest vs
  directory; `aggregate_run_set` / `RunSetVerdict` / `RunSetSummary`;
  `--filter` gains optional `<verification>::` axis. Patterns B and C
  from §10.4 are now executable. (#49)
- [additive] `api/observe` action — passive HTTP observation via
  Playwright network interception. (#38)
- [additive] Reporter contract v1 (`RunSummary`) + subprocess plugin
  loader for `--reporter <name>`; reference plugins
  `duhem-reporter-pretty` and `duhem-reporter-junit`. (#34)
- [additive] `ui/type`, `ui/select`, `ui/assert-url`, `ui/assert-state`
  — completes the §10.5 UI action catalog. (#37)
- [additive] Setup-step ordering: `Setup*` evidence variants,
  `$setup.<id>.outputs.<name>` namespace, runtime wiring. (#20)
- [additive] `api/call` action — active HTTP request, first entry in
  the API half of the action-type catalog. (#21)
- [breaking] Typed input catalog: `InputDecl.type` promoted to a closed
  catalog (`string|integer|number|boolean|array|object`); CLI
  `--inputs k=v` coerces per declared type; `Engine::run` input
  signature changes from `BTreeMap<String, String>` to
  `BTreeMap<String, serde_json::Value>`. (#22)
- [additive] Evidence trace v1: `trace.jsonl` + content-addressed
  `blobs/<sha256>` + `manifest.json`; `EvidenceWriter` / `Trace::open`
  / `replay()`. (#10)
- [additive] Judge v1: three-state verdict aggregation (`pass` / `fail`
  / `inconclusive:<cause>`), `VerdictState` / `InconclusiveCause`,
  `aggregate_check` / `aggregate_criterion` / `aggregate_run`. (#9)
- [additive] `ui/navigate`, `ui/click`, `ui/assert-element` —
  first entries in the UI action-type catalog; `Action` trait,
  `RunBrowser` / `CheckBrowser`. (#12)
- [additive] Initial schema: `VerificationDefinition`, `Criterion`,
  `Check`, `Step`, `Assertion` (closed enum of six forms),
  `TypeCheckKind`, `Expr` AST + `chumsky` parser, structural
  `validate()`, `duhem validate <path>` CLI preview. (#8)

### Root manifest schema and multi-leaf `duhem run` (#49)

Patterns B and C from `docs/duhem-spec.md` §10.4 are now executable.
`duhem run` accepts a leaf, a root manifest, or a directory; the loader
dispatches by inspecting which discriminator key is present.

#### Added

- **`RootManifest` schema type** (`crates/duhem-schema/src/manifest.rs`).
  Top-level wire shape:
  - `manifest_version: u32` (1 today; future shape changes bump this).
  - `verifications: Vec<ManifestEntry>` — each entry is either
    `{ path: PathBuf }` (Pattern B) or `{ glob: String }` (Pattern C).
  - `deny_unknown_fields` at both levels.
- **`duhem_schema::load(path) -> Result<Loaded, LoadError>`** — the
  single entry point that distinguishes a manifest from a leaf by key
  presence:
  - File with `verifications:` → `Loaded::Manifest` (entries pre-resolved).
  - File with `criteria:` → `Loaded::Leaf` (today's behavior).
  - Directory → resolved to `<dir>/duhem.yml`.
  - Both keys present, or neither → load-time error.
  - Self-referential `path:` or self-only glob → `LoadError::SelfReference`.
  - Absolute paths and `..` escapes → load-time errors.
  - Zero-match glob → non-fatal `warnings` entry on the loaded value.
- **`aggregate_run_set`** + `RunSetVerdict` in `duhem-judge`. Same
  three-state rule as `aggregate_run`: any `Fail` → `Fail`; else any
  `Inconclusive` → `Inconclusive`; else `Pass`. First-inconclusive-cause
  ordering matches every other level.
- **`RunSetSummary`** in `duhem-summary` — wraps a `Vec<RunSummary>`
  plus the aggregated verdict. `schema_version: "1"`.
- **`--filter` grammar extension** — optional verification axis. The
  three-part form `<verification>::<criterion>::<check>` selects in a
  specific leaf; the two-part `<criterion>::<check>` form continues to
  mean "all verifications, this criterion-check"; the one-part form is
  unchanged.

#### Changed

- **`duhem run <path>` is polymorphic.** Directory → manifest; manifest
  → expand and run each leaf serially; leaf → today's single-run
  behavior. Per-leaf evidence lands under
  `<evidence-root>/<leaf>/<run_id>/` on manifest runs (leaf-namespaced);
  single-leaf runs keep the pre-#49 layout (`<evidence-root>/<run_id>/`).
- **CLI default reporter on a manifest** prints one `<leaf>: <verdict>`
  line per leaf, followed by the aggregated verdict on its own line as
  the last line of stdout.
- **JSON reporter on a manifest** emits a single-line `RunSetSummary`
  rather than a `RunSummary`. Single-leaf JSON output is unchanged.
- **`--dry-run`** prints `WOULD RUN: <leaf>::<criterion>::<check>` on a
  manifest run (qualified with the leaf name) and continues to print
  the two-part form on a single-leaf run.

#### Schema impact

Additive. `RunSummary`, `VerificationDefinition`, and the existing
`--filter` grammar are unchanged. Manifest is opt-in by introducing a
`verifications:` key. Existing `duhem run path/to/leaf.yml`
invocations continue to work byte-identically on stdout. Plugin
reporters that don't yet understand `RunSetSummary` continue to work:
on a manifest run, each leaf still produces a `RunSummary` they
receive; the set-level aggregated verdict is written by the CLI as a
final stdout line.

### runtime/schema: environment provisioning v1 — operator-supplied setup + teardown hooks (#50)

Stage 3 of `docs/duhem-spec.md` §9 ("Provision Environment") was
implicit before this landed: `duhem run` assumed the SUT was already
up. v1 closes that gap with operator-supplied scripts and a readiness
probe the runtime sequences around `setup:` and the criteria loop.
Phase 0's pragmatic answer is "Duhem invokes the operator's script,"
deliberately under the future Phase 1+ "AI provisions" extension —
no containerization, no ephemeral envs.

#### Added

- New optional top-level field `environment:` on
  `VerificationDefinition`:

  ```yaml
  environment:
    up:   ./scripts/up.sh        # required when environment: present
    down: ./scripts/down.sh      # optional
    ready:
      http:
        url: $inputs.health_url  # whole-string $inputs.<name> resolved
        expect_status: 200       # default 200
        timeout: 60s
  ```

  Absent `environment:` → no behavior change vs setup-only definitions;
  the wire shape for `environment:`-less VDs is byte-identical to today.
  Relative `up:` / `down:` paths resolve against the directory
  containing the Verification Definition.
- Five additive `EventPayload::Env*` variants on the evidence trace:
  `env_up_started { command }`,
  `env_up_finished { exit_code, duration_ms, stdout_blob_sha256?, stderr_blob_sha256? }`,
  `env_ready { probe_kind, ok, elapsed_ms }`,
  `env_down_started { command }`,
  `env_down_finished { exit_code, duration_ms, stdout_blob_sha256?, stderr_blob_sha256? }`.
  `*_finished` variants fsync (same rule as other `*_finished`
  events). Stdout/stderr of `up:` / `down:` are captured to
  content-addressed blobs and referenced by sha256 from the
  `*_finished` events — never inlined into the JSONL.
- Lifecycle:
  `input resolution → environment.up → environment.ready → setup → criteria → environment.down → run verdict`.
  `down:` runs after the last criterion regardless of verdict.
- Sanitized child env per the Alignment whitelist: `PATH`, `HOME`,
  `TMPDIR`, `LANG`, `LC_*`, `DUHEM_*`. Attacker-shaped vars like
  `LD_PRELOAD` are dropped before `up:` / `down:` are forked.
- `duhem run` flags:
  - `--no-env-up` — skip `up:` + readiness probing; trust that the
    operator brought the SUT up out-of-band. Teardown still runs
    unless `--keep-env` is also set.
  - `--keep-env` — skip `down:` so the SUT outlives the run for
    triage.
  Both default off.

#### Failure policy

- `up:` non-zero exit → run verdict `Inconclusive(EnvironmentError)`,
  no setup or criterion runs, `down:` is NOT invoked (nothing came
  up).
- `up:` exits 0 but `ready:` times out → run verdict
  `Inconclusive(Timeout)`, no setup or criterion runs, `down:` STILL
  runs (so a half-booted SUT can clean up after itself).
- `down:` non-zero exit → recorded as evidence (the
  `env_down_finished.exit_code` field), verdict unchanged. Teardown
  is best-effort.

Same three-state-faithful reasoning as the `setup:` failure policy
on issue #20: a boot failure means Duhem could not observe the
workload in the verified state — definitionally "we don't know"
(Inconclusive), not "we saw the workload misbehave" (Fail).

#### Wire format

- Schema: new `Environment` / `ReadyProbe` / `HttpReadyProbe` types
  in `duhem-schema`; optional on `VerificationDefinition`.
- Evidence: five new `EventPayload::Env*` variants. No existing
  variant renamed or restructured.
- Breaking change? **no** (additive throughout). VDs without
  `environment:` see no new events on the wire.
- New workspace dependency on `reqwest` from `duhem-runtime` for
  HTTP readiness polling. The runtime previously consumed `reqwest`
  transitively via `duhem-actions`.

#### Worked example

The dogfood Verification Definition at
`verifications/onsager-dashboard-create-project/duhem.yml` gains
an `environment:` block plus `scripts/up.sh` / `scripts/down.sh`.
The boot sequence the README previously described as a manual
prerequisite is now first-class: a fresh contributor can run
`duhem run` and Duhem boots Onsager, waits for `/healthz`, runs
the verification, then shuts Onsager down.

#### Reserved (Phase 1+ follow-ups)

- Duhem-managed provisioning (containers, ephemeral envs, DB-seed
  primitives, flag-store integration).
- Probe kinds beyond `http:` (`tcp:`, gRPC health, Kafka topic
  existence).
- Manifest-level `environment:` for multi-leaf runs. v1 only
  declares it at the leaf level.
- `down:` invocation on Ctrl-C via signal handlers. Today
  `down:` runs only after the criteria loop returns normally.

### api/observe action — passive HTTP observation via Playwright network interception (#38)

Lands the second `api/*` action — passive observation of requests the
browser issues (typically as a side effect of a `ui/click`) — and
completes the Phase 0 §14 action minimum of "UI click/type/assert,
API call/observe, basic assertions."

#### Added

- `api/observe` — passive HTTP recorder. `with:` schema
  (`deny_unknown_fields`):
  - `method: String` *(optional)* — exact-string-uppercased comparison.
    Omitted matches any method.
  - `url_pattern: String` *(required)* — exact full-URL match by
    default; regex when prefixed `re:` (substring `Regex::is_match`,
    not anchored — authors who want anchoring write `re:^...$`).
  - `after: String` *(optional, reserved)* — accepted today but not
    enforced at runtime; reserved for the future concurrent-listener
    engine extension.
  - `within: Duration` *(optional)* — max wait for a matching event;
    defaults to `DEFAULT_WITHIN` (5 s).
- Outputs (response-side names match `api/call`'s so authors can
  write assertions like `$steps.x.outputs.status == 201` regardless
  of which `api/*` action produced the traffic):
  - **Request side** (new — `api/call` doesn't surface these since
    its caller specifies them in `with:`):
    `method` (uppercased), `url` (full URL), `request_body` (parsed
    JSON when the request `Content-Type` starts with
    `application/json`; `null` otherwise), `request_headers` (JSON
    object of strings, names lowercased for case-insensitive lookup).
  - **Response side** (shape matches `api/call`):
    `status` (u16 widened to integer), `body` (parsed JSON when the
    response `Content-Type` starts with `application/json`; `null`
    otherwise), `body_text` (raw response bytes as UTF-8 lossy),
    `headers` (JSON object of strings).
- When the request or response declares `application/json` but the
  body fails to parse, the corresponding output stays `null` and an
  `api.json_parse_failure` observation is recorded — same shape as
  `api/call`'s parse-failure signal.
- Implementation listens to `page.subscribe_event()` and matches on
  the first `Response` event whose URL + method satisfy the filters.
  The originating `Request` is reached via `response.request()`. No
  routing / interception is installed — observation is read-only,
  preserving the Holistic Verification Principle.

#### Outcome mapping

- Matching event arrives within `within:` → `Outcome::Ok`.
- No matching event within `within:` → `Outcome::Timeout`
  (judge maps to `Inconclusive(Timeout)`).
- Subscription failure / malformed regex in `url_pattern` →
  `ActionError`.

#### v1 ordering caveat

The spec's worked example places `api/observe` *before* the
`ui/click` it conceptually observes. That ordering needs the engine
to run the observe listener concurrently with subsequent steps — a
Phase-1 follow-up. **v1 is synchronous**: the listener subscribes at
this step's runtime and waits up to `within:` for a matching event,
so authors who want to capture a click-triggered request either
(a) place `api/observe` AFTER the trigger and rely on Playwright's
event stream still carrying the in-flight or just-finished event, or
(b) wait for the concurrent-listener engine extension. The
`api-observe.yml` fixture uses pattern (a) with a 200 ms delayed
fetch so the listener has time to attach.

#### Registry

- `api/observe` joins the default registry alongside `api/call`
  and the seven `ui/*` actions. Full v1 catalog is now `api/call`,
  `api/observe`, `ui/assert-element`, `ui/assert-state`,
  `ui/assert-url`, `ui/click`, `ui/navigate`, `ui/select`, `ui/type`.

#### Wire format

- Action type `api/observe` added to the closed action catalog. Typed
  `with:` schema; new outputs documented above.
- No change to `VerificationDefinition`, `Step`, or `Assertion`
  shapes — additive only. Existing VDs and fixtures unchanged.
- Breaking change? **no** (additive).
- New workspace dependency on `futures = "0.3"` for stream
  combinators against Playwright's broadcast subscription.

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
