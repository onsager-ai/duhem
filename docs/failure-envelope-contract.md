# Agent failure envelope (`GET /api/runs/:run_id/failure`)

Spec: `onsager-ai/duhem#216` (Tier 4 of the richer-evidence epic #208).

The failure envelope is the **agent-facing** counterpart to the human
dashboard: one machine-readable document with everything a coding
agent needs to react to a Duhem `fail` in CI — the failing assertions
and their recorded cause, the delivery-web layer chain, artifact URLs
(screenshot / DOM / network / target-rect / video), and the first failing
network request — so the verify→repair loop never requires scraping
the SPA.

It is **evidence, never a judge input**: every field is *recorded*
trace data, mechanically assembled. No verdict is recomputed.

## Endpoints

- `GET /api/runs/:run_id/failure` — the whole run: every non-passing
  check and every non-passing native run (including diagnostic runs).
- `GET /api/runs/:run_id/failure/:crit::check` — one check's entry (an
  agent handling a specific failure).

`404` when the run (or, for the scoped form, the check) doesn't exist.
A passing run is `200` with `failing: []` — not an error.

## Response shape

```jsonc
{
  "run_id": "01…",
  "verification": "checkout",
  "verdict": "fail",
  "failing": [                          // one entry per non-passing check; [] on a pass
    {
      "run_id": "01…",                 // present only in a tree-wide parent envelope
      "verification": "checkout",      // verification name for this run
      "criterion_id": "AC-1",
      "check_id": "AC-1.1",
      "verdict": "fail",
      "layers": ["ui", "api", "data"],  // the delivery-web chain (#192), in order
      "assertions": [
        { "assertion_index": 0, "state": "fail", "detail": "actual false, expected true" }
      ],
      "artifacts": [                     // fetch these for the full evidence
        { "id": "<sha>", "kind": "capture/screenshot", "url": "/api/runs/01…/artifact/<sha>" },
        { "id": "<sha>", "kind": "capture/network",    "url": "/api/runs/01…/artifact/<sha>" }
      ],
      "first_failing_request": {         // omitted when there's no network capture / no error
        "method": "POST", "url": "http://…/api/charge", "status": 500
      }
    }
  ]
}
```

### Native failures (#649)

`failing_native` is additive and omitted when empty. Single-leaf envelopes
without native runs keep their existing JSON bytes. Each non-passing native
run contributes one entry, including `required: false` diagnostic runs even
when the overall gate passes. The summary's bounded `failing_tests` index
identifies individual failures; fetch `results` for the full normalized list.

```jsonc
"failing_native": [{
  "run_id": "01…", "native_id": "rust-unit", "layer": "unit",
  "outcome": "fail", "required": true,
  "detail": "command exited with 1",
  "failing_tests": [{"test_id": "stable identity", "name": "rejects invalid input", "file": "src/test.rs", "message": "masked message"}],
  "evidence": [
    {"id": "<sha>", "kind": "stdout", "url": "/api/runs/01…/artifact/<sha>"},
    {"id": "<sha>", "kind": "stderr", "url": "/api/runs/01…/artifact/<sha>"},
    {"id": "<sha>", "kind": "report", "url": "/api/runs/01…/artifact/<sha>"},
    {"id": "<sha>", "kind": "results", "url": "/api/runs/01…/artifact/<sha>"}
  ]
}]
```

`run_id` is always present, including on a leaf. `layer` and `detail` are
omitted when absent. Detail is masked at recording and limited to 2048
Unicode characters plus a truncation marker in this projection. Test failure
messages are the already masked, bounded summary messages. Evidence includes
only recorded blobs, in stdout/stderr/report/results order. URLs resolve
through the existing raw artifact route; no new endpoint or test verdict is
introduced. Static exports copy these blobs and rewrite URLs relative to the
export root. The per-check endpoint remains check-only.

### Missing shards (#650)

Logical-parent envelopes include every shard's suite descendants, checks and
native failures. While coverage is pending, `verdict` is null. After the
configured grace since the last distinct arrival (30 minutes by default), the
verdict is `inconclusive:missing_observation` and an additive field names missing
planned work:

```json
"missing_shards": [{"index": 2, "units": ["[\"criterion\",\"leaf.yml\",\"AC-2\"]"]}]
```

`missing_shards` is omitted when none are missing or the parent is still pending.
Each entry is an inconclusive coverage item, including an empty `units` list for
an expected shard assigned no units. Existing failing checks and native results
remain available alongside it. The status can resolve after a late arrival;
this is read-time policy, with no stored parent verdict or terminal marker.

### Semantics

- **`gated_checks`** (#509, optional) — entries with `criterion_id`,
  `check_id`, and non-zero `gated_judging_steps`, including passing
  checks. Omitted entirely when no judging steps were value-gated; static
  gates, upstream blocking, and lifecycle steps do not count. The scoped
  check response and each failing entry also carry `gated_judging_steps`
  when non-zero. Tree envelopes combine these entries using this existing
  summary shape, without per-run tags. The check timeline
  preserves the skipped step's condition and evaluated operands.
  None of these fields changes the recorded verdict.
- **Tree semantics (#651)** — when the requested run has manifest
  (`origin: suite`) children, `failure` includes checks from that run and
  every such descendant, with
  descendants traversed in execution order (`started_at` ascending, then
  `run_id`). The top-level `verdict` is the existing three-state
  run-set aggregate over tree leaves, folded through `native_gate` with
  required native outcomes recorded on suite parents. Leaf verdicts already
  include their native gate. Diagnostic native outcomes do not change the
  verdict; if any leaf has no verdict yet, the
  tree `verdict` is `null`, so an incomplete tree never reads as a pass.
  Nested `invocation` runs started from inside a check are that check's
  evidence, not tree members. Each tree item adds
  `run_id` and `verification` for its source run. A leaf envelope keeps
  its existing JSON shape; these provenance fields are omitted. Per-check
  routes and envelopes requested for a child remain scoped to that run.
- **`failing`** — every check whose recorded verdict is not `pass`
  (`fail` or `inconclusive:<cause>`). The scoped endpoint returns a
  single such object (not wrapped in `failing`).
- **`assertions`** — every assertion that is not `pass`, with the
  recorded `detail` (e.g. `actual false, expected true`, an
  inconclusive cause, or a gate reason). `state` is `fail`,
  `inconclusive:<cause>`, or `skipped`; skipped is absence from verdict
  aggregation, not a fourth check verdict.
- **`layers`** — the ordered delivery-web layers the check crossed
  (`ui` / `api` / `data` / `runtime`), from the #192 spans. Empty for
  pre-tag runs.
- **`artifacts`** — the check's `capture/*` (and other blob)
  observations as `{id, kind, url}`; the URLs serve raw bytes
  (`GET /api/runs/:id/artifact/:sha`). The agent fetches the
  screenshot, the DOM, or the HAR as needed.
- **`first_failing_request`** — the first entry in the check's
  `capture/network` HAR (#204) whose response status is ≥ 400 — usually
  the request that broke. `None`/omitted when there's no network
  capture or no error response.

## Provenance

Read-side only, assembled in `duhem-dashboard`'s reader from the
recorded trace (`run_events` projections + the `spans` table + the
`capture/network` HAR blob). No store-trait change, no migration, no
`SCHEMA_VERSION` change. The hosted hub mirrors this endpoint as a
sibling task in the closed repo.
