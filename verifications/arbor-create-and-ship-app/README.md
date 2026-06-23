# `arbor-create-and-ship-app`

Duhem's second dogfood Verification Definition: it drives the real
[Arbor](https://github.com/onsager-ai/arbor) dashboard end-to-end and
verifies the product's core promise — describe an app in plain
language, and Arbor builds and **ships** a working app.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) /
  [`scripts/down.sh`](scripts/down.sh)

It is the worked example that proves Duhem's `ui/*` + `api/*` surface
is enough to dogfood a second, independent web product (per the
re-prioritization toward Arbor + Crawlab).

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | Typing a description into the composer and submitting starts a build run and routes to that run's page; the create request is accepted (`POST /api/apps → 201`). |
| **AC-2**  | The build run executes the full factory pipeline and ships a live, reachable app — proven by the run page surfacing the live-preview link. |

## Why it stays deterministic without mocking the web

Arbor's **default** mode needs no opt-in env vars to be cheap and
repeatable:

- **FakeAgent** — with `ARBOR_AGENT` unset there is no live LLM call
  and no spend (`../arbor packages/factory/src/agent.ts`).
- **dry-run drivers** — with `ARBOR_DRIVERS` unset the deploy is
  simulated, but the dry-run promote step stands up a **real** local
  preview server and advertises a clickable `127.0.0.1` URL
  (`../arbor packages/factory/src/drivers/dry-run.ts`). So AC-2
  verifies a genuinely reachable deployment, not a stub — the
  Holistic Verification Principle (`docs/duhem-spec.md` §8) holds.
- **loopback no-auth** — binding to `127.0.0.1` (the default
  `ARBOR_HOST`) yields a local admin session, so there is no login
  prelude to script.

## Operator setup

1. Clone Arbor next to this repo, or point at it:
   ```sh
   export DUHEM_ARBOR_REPO_DIR=/path/to/onsager-ai/arbor
   ```
   `up.sh` defaults to `../../../arbor` relative to this directory.
2. Toolchain on `PATH`: `pnpm` + Node (Arbor targets Node 20+) and
   Playwright Chromium for Duhem's browser actions:
   ```sh
   npx playwright install chromium
   ```
3. Nothing else — `up.sh` runs `pnpm install && pnpm build` then
   boots the single-port server (UI + API on `:4100`).

## Running

```sh
# Full run: provisions Arbor, verifies, tears down.
duhem run verifications/arbor-create-and-ship-app/duhem.yml

# Arbor already running on :4100 — skip provisioning.
duhem run verifications/arbor-create-and-ship-app/duhem.yml --no-env-up

# Leave Arbor up afterward for triage.
duhem run verifications/arbor-create-and-ship-app/duhem.yml --keep-env

# Iterate on one criterion.
duhem run verifications/arbor-create-and-ship-app/duhem.yml --filter AC-1
```

Override any URL input for a non-default port or a staging host, e.g.
`--inputs new_app_url=http://127.0.0.1:5173/new` to target the Vite
dev server (`pnpm dev`) instead of single-port mode — the Vite proxy
forwards `/api` to `:4100`, so a single origin still works.

## Confirmed live; what's environment-coupled

This VD has run green end-to-end (`verdict: pass`, both criteria)
against a real single-port Arbor on `:4180` in default FakeAgent +
dry-run mode — the pipeline ships in ~4s. Two notes for other
environments:

1. **The terminal-success link (AC-2).** AC-2 keys "the app shipped"
   on a `role: link` whose visible text is `/preview/live/` — the
   proxied relative path the single-port registrar advertises once the
   deploy is promoted. This is **environment-coupled**: a non-
   registrar deploy or the cloud `cli` drivers (`ARBOR_DRIVERS=cli` +
   tokens) advertise an absolute host (e.g. `*.vercel.app`) instead,
   so the locator text changes with the environment. If AC-2 times
   out, first confirm a run actually reaches `outcome: shipped`
   (`GET /api/runs/<id>` → `status: "succeeded"`), then confirm the
   link's rendered text.
2. **Port 4180, not 4100.** Arbor's own default is `:4100`, but
   Onsager (Duhem's first dogfood) commonly runs on `:4100` + `:5173`
   on the dev machine — and Onsager's `/api/health` *also* returns
   200, so a 4100 target would silently drive Onsager and still pass
   the readiness probe. The VD and `up.sh` default to `:4180` to keep
   the two dogfoods side by side. Override the URL inputs / `ARBOR_PORT`
   for a different layout.

## Further reading

- Authoring discipline: the `verification-authoring` skill.
- Worked first-customer example:
  `verifications/onsager-dashboard-create-spec-plan/`.
- Product spec: `docs/duhem-spec.md` §7 / §8 / §10.
