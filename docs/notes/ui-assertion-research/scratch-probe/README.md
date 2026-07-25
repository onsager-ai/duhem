# THROWAWAY PROBE CODE — NOT PRODUCTION, NOT A PACKAGE

Everything in this directory is disposable investigation code written to
answer the questions in `../design.md`. It is checked in **only** so the
numbers in the sibling documents are reproducible and auditable. It is
not wired into the Cargo workspace, not linted, not tested, and must not
be imported by anything.

If the design is ever built, none of this survives. Its value is the
method, not the code.

## Running it

Needs Node ≥ 22 with `playwright` resolvable and a Chromium at
`PLAYWRIGHT_BROWSERS_PATH`. Capture and the live sweep also need
outbound network; in a proxied environment run them with
`NODE_USE_ENV_PROXY=1`, because the browser's own network is routed
through Node's fetch via request interception.

```sh
node capture.mjs            # Phase 0: freeze the corpus to archives/*.mhtml
node phase1.mjs             # Phase 1: naive vs settle-v1        (SHARD/NSHARD to parallelise)
node phase1b.mjs            # Phase 1: settle-v3
node phase1c.mjs            # Phase 1: settle-v4  <- the protocol that reaches 100%
node agg1.mjs               # merge Phase 1 shards, print the table
node phase2a.mjs            # Phase 2: static checks, pure function over Facts (no browser)
node phase2c.mjs            # Phase 2: border-box adjudication crops + contact sheets
node phase2e.mjs            # Phase 2: text-run-rect overlap re-measurement
node phase2f.mjs            # Phase 2: text-run adjudication crops
node phase2d.mjs            # Phase 2: class-D cross-viewport sweep, LIVE
node validate.mjs           # archive-vs-live fidelity check
node verifyD.mjs            # scrollWidth heuristic vs actual scrollability
```

`phase2b.mjs` is the archive-based sweep. It is kept because it is the
artifact that *produced* the fidelity finding, but its output is not
valid — see `../phase2-failure-distribution.md` §"Result 4". Use
`phase2d.mjs` instead.

The corpus archives themselves (~50 MB of third-party page content) are
deliberately **not** committed: they are captured third-party material,
and `raw/corpus-provenance.json` records exactly what was fetched and
when so the corpus can be re-captured.

## raw/

| file | what |
|---|---|
| `corpus-provenance.json` | every capture attempt: URL, timestamp, viewport, UA, bytes, element count, SPA flag, usable flag, and the reason for each exclusion |
| `phase1.merged.json` | naive + settle-v1, per page, per field drift |
| `phase1b.merged.json` | settle-v3, incl. drift examples with per-run values |
| `phase1c.merged.json` | settle-v4 — the headline determinism result |
| `phase2a.summary.json` | static-check counts per page (fail / inconclusive / suppressed, by cause) + scale-regularity stats |
| `phase2e.textruns.json` | text-run-rect overlap counts + cover histogram |
| `phase2d.merged.json` | live cross-viewport sweep, per width, with offenders and change points |
| `adjudication-borderbox.json` | the 28 border-box findings I inspected |
| `adjudication-textruns.json` | the 43 text-run findings I inspected |

Adjudication was done by reading rendered crops of each finding. The
crops and contact sheets are not committed (they are screenshots of
third-party sites); regenerate with `phase2c.mjs` / `phase2f.mjs`.
