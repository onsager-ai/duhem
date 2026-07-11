# Agent failure envelope (`GET /api/runs/:run_id/failure`)

Spec: `onsager-ai/duhem#216` (Tier 4 of the richer-evidence epic #208).

The failure envelope is the **agent-facing** counterpart to the human
dashboard: one machine-readable document with everything a coding
agent needs to react to a Duhem `fail` in CI — the failing assertions
and their recorded cause, the delivery-web layer chain, artifact URLs
(screenshot / DOM / network / target-rect), and the first failing
network request — so the verify→repair loop never requires scraping
the SPA.

It is **evidence, never a judge input**: every field is *recorded*
trace data, mechanically assembled. No verdict is recomputed.

## Endpoints

- `GET /api/runs/:run_id/failure` — the whole run: every non-passing
  check.
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

### Semantics

- **`failing`** — every check whose recorded verdict is not `pass`
  (`fail` or `inconclusive:<cause>`). The scoped endpoint returns a
  single such object (not wrapped in `failing`).
- **`assertions`** — only the *non-passing* assertions, with the
  judge's recorded `detail` (e.g. `actual false, expected true`, or an
  inconclusive cause). `state` is a wire verdict token.
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
