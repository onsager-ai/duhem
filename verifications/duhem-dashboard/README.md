# `duhem-dashboard`

Duhem-on-Duhem regression coverage (epic #148): drives the **real**
`duhem dashboard` web surface — its embedded React/Vite SPA, its JSON
API, and its live SSE stream — over a **real** run's evidence, and
mechanically asserts the contract a dashboard operator depends on.
Black-box, whole-binary coverage that complements the white-box tests
in `crates/duhem-dashboard/tests/`.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Fixture run source: [`fixture/dashboard-fixture.yml`](fixture/dashboard-fixture.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh)

## ⚠️ Self-reference caveat — not a trust anchor

This VD is **regression coverage, not independent attestation**. Per
the Asymmetric-trust commitment (`docs/duhem-spec.md` §11.2), the
verifier of AI claims must be structurally independent of the AI making
them. Duhem verifying Duhem is correlated failure: a judge defect that
wrongly passes could equally wrongly pass its own self-test. A green run
means "no dashboard regression detected"; the Onsager seam holds the
trust role.

## What it verifies

| Criterion | Commitment | Surface |
| --------- | ---------- | ------- |
| **AC-1**  | `GET /api/runs` + `GET /api/runs/<id>` serve the recorded verdict and the criterion→check tree in shape. | JSON API |
| **AC-2**  | The browser renders the SPA shell, the run's `pass` verdict, and the criterion's check as a link. | SPA (real Chromium) |
| **AC-3**  | The live SSE stream replays the trace through the recorded `run_finished` verdict. | SSE |

No mocks at the boundary (`docs/duhem-spec.md` §8): `environment.up`
runs the offline `fixture/dashboard-fixture.yml` through the real
`duhem run` pipeline to record a genuine run in the production evidence
store (#189), then launches the real `duhem dashboard` over it.

## Holistic posture & the fixture

`scripts/up.sh`:

1. Runs `fixture/dashboard-fixture.yml` (one offline, page-free
   `cli/invoke` step → verdict `pass`) through `duhem run --db
   <scratch>/duhem.db --run-id dashboard-fixture-run`, recording a real
   run in the production store under a deterministic id.
2. Launches `duhem dashboard --db <scratch>/duhem.db --port <port>`
   (serve mode), backgrounded in its own process group.

`scripts/down.sh` signals that process group (and a targeted `pkill`
fallback) so the wrapper and the `duhem-dashboard` listener die
together — `duhem dashboard` is a thin wrapper that spawns the server as
a child, so the wrapper pid alone is not enough.

## Operator setup

A Rust toolchain, the **SPA bundle embedded into the dashboard binary**
(`just build dashboard`), and **Playwright Chromium** for AC-2's
browser:

```sh
just build dashboard                       # SPA bundle → duhem-dashboard
cargo build -p duhem-cli                   # the `duhem` CLI under test
cd crates/duhem-actions/sidecar && npm ci && npx playwright install chromium && cd -
```

### Config — script env, not VD inputs

The `up:`/`down:` child runs with a **sanitized environment** (only
`PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, `DUHEM_*` survive) and **does
not receive VD `inputs`** (see `crates/duhem-runtime/src/engine/env.rs`).
So unlike the `duhem-cli` VD — whose `duhem_bin` is consumed directly by
`cli/invoke` steps — here the binary and port are **script-side
`DUHEM_*` env vars**:

| Env var | Default | Purpose |
| ------- | ------- | ------- |
| `DUHEM_BIN` | `duhem` (PATH) | The `duhem` binary under test. `duhem dashboard` resolves the sibling `duhem-dashboard` next to it. |
| `DUHEM_DASHBOARD_PORT` | `7878` | Listen port. **MUST** match the `:7878` baked into the VD's URL inputs. |

The VD `inputs` (`runs_url`, `run_detail_url`, `live_url`,
`run_page_url`) are the whole-string URLs the steps and the readiness
probe hit. (Whole-string because template substitution does no string
concatenation yet — same constraint the chreode/onsager VDs work around.)

## Running

From the repo root, against a locally built binary:

```sh
DUHEM_BIN="$PWD/target/debug/duhem" \
  ./target/debug/duhem run verifications/duhem-dashboard
```

Against an installed `duhem` (default `DUHEM_BIN=duhem`):

```sh
duhem run verifications/duhem-dashboard
```

## CI

`.github/workflows/self-verify.yml` carries a `dashboard` job (separate
from the page-free `cli-regression` job): it builds the SPA bundle + the
binaries, installs Playwright Chromium, and runs this VD on PRs touching
`crates/duhem-dashboard/**` or `verifications/duhem-dashboard/**`.
Separate from the Onsager `dogfood` workflow by design (see the caveat
above).

## Action-catalog note (SSE)

AC-3 exercises the SSE endpoint with `api/call`: for a **finished** run
the replay-then-follow contract replays the whole trace and closes, so a
single read returns the full `text/event-stream` body, which the
assertions match. The catalog has no action that consumes an
*open-ended, incremental* SSE stream from an in-progress run (`api/poll`
is request/response polling, `api/observe` is Playwright network
interception of a navigated request), so true incremental live-follow is
not exercised here — only the replay-to-`run_finished` contract. That is
the correct mechanical slice for a deterministic fixture; incremental
streaming would need a new action type (its own `area:schema` spec).

## Status

Proven green end-to-end against a locally built binary: `verdict: pass`,
all three criteria pass (AC-2 via real Chromium). Provisions, serves,
asserts, and tears down in well under a minute.
