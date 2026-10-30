# Product regression migration contract

This is the Duhem-side producer/consumer contract and capability inventory for [#575](https://github.com/onsager-ai/duhem/issues/575), paired with Semon's [product migration issue #501](https://github.com/onsager-ai/semon/issues/501). It records the current foundation and a qualification order; it does not claim that a Semon assertion has been inventoried, migrated, or qualified.

## Baseline and ownership

The inventory is pinned to Duhem `8387333f874decd9d146fe042a23f461fc617d8d` (workspace and Verification Definition schema `0.5.1`). The [generated action reference](https://github.com/onsager-ai/duhem/blob/8387333f874decd9d146fe042a23f461fc617d8d/docs/action-reference.md) is the version-exact built-in action contract. Recheck the source, report contract, and pins at the candidate execution SHA before implementation or adoption. Keep the package/schema version separate from the run trace's `schema_version: v1` in the [run-bundle contract](run-bundle-contract.md).

Duhem owns reusable actions, lifecycle hooks, observations, deterministic judgment, evidence, and reports. Semon owns its `.duhem/` suite, intent-bearing criteria, real fixtures and inputs, thresholds, supported claim scope, and standalone CI. Product VDs stay with the product and must run without a hosted coordinator for collection/history claims. Native unit, parser/state-machine, build, formatting, and bundle-freshness checks keep their existing jobs.

Helpers may prepare environments, create controlled inputs, manage product resources, or return structured facts under a documented contract. Keep the acceptance rule visible in the VD's criteria, checks, and assertions; a broad test runner hidden behind `cli/invoke` with only `exit_code == 0` does not qualify the product behavior. A helper's ability to perform work does not establish a Duhem action contract, captured evidence, or a product claim.

## Consumer assertion map

The product-side map is pending the Semon owner under #501. Do not treat absent rows as completed inventory or infer active scope from file discovery. Map assertions and states, not test-file counts; distinguish active, deferred, disabled-boundary, internal, and historical cases before assigning a migration disposition.

Each row must record:

| Required field | What to record |
|---|---|
| Legacy owner | Old case/assertion/state identifier, source path, and responsible team or owner |
| Source status | Active, deferred/paused, disabled-boundary, internal, or historical; if an old case is retired, record the owner and reason |
| VD mapping | Stable criterion, check ID, parameter instance, input/fixture, and required starting state |
| Disposition | `VD` (migrated), `native` (retained at its current layer), or `deferred` (with the missing capability or scope decision) |
| Claim scope | What the result claims, including real versus synthetic inputs, services, authentication, browser, and any deliberate boundary |
| Evidence | Expected and actual values, trace/artifact needed, secret handling, and how a reviewer inspects the result |
| Version and pin | Duhem commit, package/schema version, action contract, and any browser/font/measurement pins relevant to the claim |
| CI owner | Product repository workflow/profile, owning team, and whether the row is required, diagnostic, or retained-native |

No product assertion rows are filled in by this Duhem document. Semon supplies the source assertions, classifications, real fixtures, thresholds, and CI owner; Duhem changes only the generic capability contracts that the reviewed map demonstrates are needed.

## Duhem capability inventory at the pinned baseline

The catalog is closed and generated in [`docs/action-reference.md`](action-reference.md). “No built-in contract” below means the baseline catalog or lifecycle schema does not define that primitive; a product helper may still produce explicit facts for a VD to judge. Qualify each mapped assertion against the exact helper/action and evidence path instead of treating a possible workaround as native coverage.

| Area | Current contract | Migration disposition / gap |
|---|---|---|
| Local command | `cli/invoke` runs one command with optional `cwd`, `env`, `stdin`, and `timeout`; outputs are `exit_code`, `stdout`, and `stderr`. | Native for narrow command results. There is no catalog action that owns a background process across steps and exposes its identity, logs, readiness, and stop result as structured outputs. Do not make a whole product suite's exit code the acceptance assertion. |
| Environment lifecycle | `provision.up` runs once before setup; optional `provision.down` runs after leaf teardown; `provision.ready.http` polls an HTTP status after `up` exits. See the [provision schema](../crates/duhem-schema/src/provision.rs). | Existing script hooks and HTTP readiness are usable. They are not a general owned-process/resource lease contract. At this baseline, `down` failure is recorded as evidence but does not alter the verdict; qualify required cleanup only after the gate behavior in [#567](https://github.com/onsager-ai/duhem/issues/567) is resolved. Other probe types and structured process observations remain deferred until a mapped assertion needs them. |
| HTTP and page network | `api/call` returns status/body; `api/poll` checks a response condition until its deadline; `api/stream` collects SSE events with `max_events` / `until_event` and returns `events`, `event_count`, `last_event`, `stopped_reason`, and `status`. `api/observe` matches a page request by method and URL pattern and declares no outputs. | Native for those exact contracts. A generic request/event stream with product-defined count, ordering, and provenance is not a separate catalog contract; map the assertion to existing returned facts or defer it until a reusable observation contract is specified. |
| Browser interaction and state | `ui/navigate`, `click`, `type`, `select`, `wait`, and `extract`; judging actions `assert-element` and `assert-url`; optional locator `frame:` addresses nested/cross-origin iframe contents; `capture-session` returns browser storage state. | Native for the listed locator, DOM/state, URL, and storage-state claims. Assert an application's state marker with `ui/assert-element`. Frame locators support the existing UI actions; general keyboard, pointer, scroll and focus actions are absent from the catalog. `capture-session` is not a screenshot or visual assertion. Helpers may expose facts, but the product must retain the action boundary and judgment in the VD. |
| Database | `db/query` reads rows; `db/observe` polls a query condition; `db/seed` writes seed data. | Native for these declared database operations. The product owns supported engine, fixture, credential, and recovery claims; a query or seed action alone does not establish a complete recovery journey. |
| Visual judgment | The action reference has no full-frame/region pixel comparator with reference binding, threshold, environment provenance, and inspectable diff artifacts. | Deferred until a reviewed visual capability exists. Capture used as diagnostic evidence does not qualify visual acceptance. |
| Performance | [`duhem-spec.md` §7.7](duhem-spec.md#77-evidence) records navigation/resource timing, web-vital observations, and long tasks in `capture/session-evidence` as diagnostic facts. Captures are not judge inputs; replay applies no score or threshold. | The gap is an explicit metric sampler, measurement-window, and comparator contract for judging a declared metric with samples, cold/warm state, and aggregation. Runner-step or whole-run duration cannot substitute for the product's metric or threshold. |
| Concurrent isolation | The baseline has no native per-run resource lease contract for isolated files, ports, databases, credentials, and cleanup ownership. | Existing CI sharding can continue. Qualify a mapped parallel/recovery claim when the lease work in [#410](https://github.com/onsager-ai/duhem/issues/410) or a scoped equivalent lands; all of #410 is not a prerequisite for the first domain. |
| Run evidence and report consumption | The [RunBundle v1 contract](run-bundle-contract.md) is a separate export envelope with ordered events and content-addressed artifacts. At this baseline, `--reporter json` emits one `RunSummary` object for a single run and one `RunSetSummary` object containing per-leaf summaries plus the aggregate verdict for a manifest (see [`reporter.rs`](../crates/duhem-cli/src/reporter.rs)). [#535](https://github.com/onsager-ai/duhem/issues/535) fixes the docs/help description; [#544](https://github.com/onsager-ai/duhem/issues/544) tracks leaf attribution. | Consume the existing shape for the selected report mode and pin its Duhem source/schema. #535 does not require a new report version. Do not infer a single-leaf result from a manifest summary or use a report without identity and executed claim scope. |

## Gate prerequisites and current owner work

The first domain can qualify incrementally, but a required consumer pass must identify what ran and what it covers. Status snapshot for this planning handoff (2026-10-28; refresh before implementation):

| Owner | Current state | Qualification consequence |
|---|---|---|
| [#531](https://github.com/onsager-ai/duhem/issues/531) | Open; claim-set reporting lacks count/provenance. | Distinguish authored, selected, executed, gated, and excluded claims; a filtered pass must not imply full coverage. |
| [#546](https://github.com/onsager-ai/duhem/issues/546) | Open; declared definitions are not yet inventoried against recorded runs. | Report absent records in the specified store; do not claim an unseen invocation never occurred. |
| [#544](https://github.com/onsager-ai/duhem/issues/544) | Open; manifest result attribution to its leaf is unresolved. | Attribute each result to its leaf. Check, parameter-instance, attempt, and executed-scope identity are additional qualification fields where #544 does not supply them. |
| [#535](https://github.com/onsager-ai/duhem/issues/535) | Open; help text names one of the two existing JSON reporter shapes. | Document both already-shipped single-leaf and manifest shapes; consume the selected existing shape without waiting for a new report version. |
| [#567](https://github.com/onsager-ai/duhem/issues/567) | Open; required teardown failure can leave the case passing. | Preserve the body verdict and cleanup finding; required cleanup failure must block the aggregate, while diagnostic-only cleanup is explicit. |
| [#574](https://github.com/onsager-ai/duhem/issues/574) | Open; assertion visibility in the steps rail and resolved operand values are incomplete. | Show expression, expected/actual values, and evidence with secret masking. |
| [#524](https://github.com/onsager-ai/duhem/issues/524) | Closed; lifecycle evidence landed. | Reuse landed setup/teardown evidence and ownership in the consumer report. |
| [#543](https://github.com/onsager-ai/duhem/issues/543) | Open; session cascade and teardown pairing landed; nested groups and fixture folding remain. | Reuse landed scope, then qualify only the remaining group/fixture behavior the mapped first domain requires. |

These are separate owner issues, not work completed by this inventory. Keep missing, skipped, deferred, diagnostic, timed-out, and failed work distinct. A known-bad consumer build or controlled fault must fail the intended check; the matching correct build must pass with the same criteria, fixture, and thresholds.

## Qualification sequence

1. **Complete the consumer map.** In Semon, inventory active assertions and states, assign each an owner and scope, and record its `native`, `VD`, or `deferred` disposition using the fields above. Keep disabled-boundary and historical cases explicit. No file-count equality substitutes for traceability.
2. **Make the gate trustworthy for the selected domain.** Close or qualify the relevant claim-set, missing-run, leaf/report, cleanup, and assertion-detail owners above. Zero selected required checks, a missing or malformed report, missing prerequisites, timeout, skipped required work, and required cleanup failure must block a complete pass. Report no-run as absence of a record in the specified store, without asserting that an invocation never happened. Preserve every retry attempt and the first-attempt requirements.
3. **Pilot the complete real-binary path.** Start with the Semon journey named in #501: native homes → Push → Receive → Catalog/selected reading → transcript paging → append/restart/recovery/revocation. Split it into independently runnable checks with explicit identity, inputs, fixture ownership, setup/readiness/cleanup, and evidence. Use disposable synthetic inputs and actual supported binaries/services for the claim; helpers expose facts, while the VD carries the acceptance rule.
4. **Migrate remaining active behavior by domain.** Cover CLI, HTTP/MCP, synchronization, and browser behavior with product-owned fixtures and thresholds. Keep fast native unit, parser/state-machine, and internal fault-injection tests where they provide the right layer. Keep cross-component journeys with the product that owns the integration.
5. **Add interaction and visual qualification.** Add only the keyboard/touch/focus/scroll/frame and transport observations demanded by mapped assertions. Visual checks need separate reviewed reference and candidate artifacts, source/fixture/browser/font/viewport/theme provenance, dimensions, explicit threshold and acceptance binding, and an inspectable diff; functional and visual verdicts remain distinct.
6. **Add measured performance and recovery claims.** Build on the existing diagnostic navigation/resource timing, web-vital, and long-task facts with an explicit metric sampler, measurement window, sample set, cold/warm state, aggregation, and comparator before a threshold can judge the metric. Keep long-session recovery evidence with its explicit identity and append/restart/revocation contract. Do not use runner duration as a substitute.
7. **Qualify parallel execution when needed.** Preserve existing isolated CI shards until Duhem resource leases prove file, port, database, credential, and cleanup ownership under concurrency. This can follow the first domain; #410 is not a blanket migration gate.
8. **Prove parity before each CI cutover.** Run old and VD checks against the same candidate and fixtures; compare known-good and representative known-bad cases; reconcile every disagreement; record stability, duration, and resource cost. Pin Duhem commit, VD/schema/action contract, fixture, and browser/measurement environment. Switch only a fully mapped domain with an explicit product CI owner; remove only duplicate execution after qualification.

For every capability-changing child, update the canonical spec, generated action reference/`describe`, authoring guide, worked product VD, and adoption template in that implementation change. This inventory is the planning baseline; it is not a schema change, a Semon assertion map, a CI gate change, or evidence that the full #575 plan is complete.
