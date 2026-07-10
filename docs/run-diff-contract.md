# Run-to-run diff contract (`GET /api/runs/:run_id/diff`)

Spec: `onsager-ai/duhem#211` (Tier 2 of the richer-evidence epic #208).

The diff compares a run against a **baseline** — the most recent prior
run of the same verification + target whose recorded verdict is
`pass` (**last pass**). It reaches back over a failing streak to the
last-known-good run, so the diff answers the regression question
("what changed since this last worked"), not "what changed since the
previous attempt".

The diff is **evidence, never a judge input**: it only surfaces
*recorded* verdict/assertion transitions and artifact references. It
never recomputes a verdict, and no field here gates anything.

## Query

- `GET /api/runs/:run_id/diff` — auto-resolve the last-pass baseline.
- `?baseline=<run-id>` — pin a specific baseline run instead.

`404` when `:run_id` doesn't exist. A missing *baseline* is not an
error — it's `baseline: null` (see below).

## Response shape

```jsonc
{
  "current":  { "run_id": "01…", "started_at": "…", "verdict": "fail" },
  "baseline": { "run_id": "01…", "started_at": "…", "verdict": "pass" } | null,
  "criteria": [
    {
      "id": "AC-1",
      "baseline_verdict": "pass",
      "current_verdict": "fail",
      "changed": true,
      "checks": [
        {
          "id": "AC-1.1",
          "baseline_verdict": "pass",
          "current_verdict": "fail",
          "changed": true,
          "assertions": [
            {
              "assertion_index": 0,
              "baseline_state": "pass",
              "current_state": "fail",
              "current_detail": "actual false, expected true",
              "changed": true
            }
          ],
          "baseline_artifacts": [ /* ArtifactRef[] on the baseline run */ ],
          "current_artifacts":  [ { "id": "<sha>", "kind": "capture/screenshot", "url": "/api/runs/01…/artifact/<sha>" } ]
        }
      ]
    }
  ]
}
```

### Semantics

- **`baseline: null`** — the verification has never passed against
  this target. Every `changed` is then `false`; the view renders a
  "no passing baseline to compare against" state rather than diffing
  two failures.
- **`changed`** (criterion / check) — `true` iff a baseline exists and
  the recorded verdict differs. **`changed`** (assertion) — `true` iff
  the recorded `(state, detail)` differs.
- **verdict / state values** — the judge's wire tokens: `"pass"`,
  `"fail"`, `"inconclusive:<cause>"`, or `null` (no recorded verdict).
- **added / removed** — a criterion/check/assertion present on only
  one side (the VD was edited between runs) appears with the missing
  side `null` and `changed: true`.
- **`*_artifacts`** — each check's `capture/*` (and other blob)
  observations, as `ArtifactRef {id, kind, url}`, on each side. URLs
  point at that side's run, so the view can render baseline↔current
  evidence side by side and diff the HAR / screenshot itself
  (consumed by #212 / #213).

## Provenance

Read-side only, assembled in `duhem-dashboard`'s reader from existing
store queries (`verification_history` for baseline resolution,
`run_events` for the projections). No store-trait change, no
migration, no `SCHEMA_VERSION` change.
