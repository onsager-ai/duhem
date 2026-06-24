# `crawlab-ui-dashboard`

Duhem's UI dogfood against Crawlab Pro's **Vue dashboard** — the
companion to the REST VD (`../crawlab-create-project/`). It drives the
real Vite-served SPA against the real Crawlab REST backend over a real
MongoDB.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh)

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | Signing in from the dashboard authenticates the user — the sign-in form leaves the DOM and the route settles on the authenticated home view. |
| **AC-2**  | Once signed in, the projects page renders the authenticated list surface (its `New Project` create affordance) from the real backend. |

The login form really `POST`s to the backend `/login`, stores the
token, and the auth-gated routes really call the backend — no mocks at
the web boundary (`docs/duhem-spec.md` §8). The app uses **hash
routing** (`…/#/login`, `#/home`, `#/projects`). Locators were confirmed
against the live UI (`Username` / `Password` textboxes, `Sign In` and
`New Project` buttons).

## Provisioning — license-free

`up.sh` brings up three pieces, all license-free:

1. a throwaway MongoDB container;
2. Crawlab's **open-source core** (the `crawlab/` submodule) run from
   source as the master via `go run` — no Pro license gate, isolated
   `HOME` so it registers as master (backend on **:8090**);
3. the Vue frontend via `vite`, with `VITE_APP_API_BASE_URL` pointed at
   the backend (frontend on **:5188** — Crawlab's Vite default 5173
   collides with Onsager on the dev machine). The backend's CORS
   middleware allows the cross-origin calls.

`down.sh` reaps both servers (each `setsid`'d into its own process
group) plus a port-listener backstop, and removes the Mongo container.

### Operator setup

1. A `crawlab-team/crawlab-pro` checkout:
   ```sh
   export DUHEM_CRAWLAB_REPO_DIR=/path/to/crawlab-team/crawlab-pro
   ```
   `up.sh` defaults to `../../../../crawlab-team/crawlab-pro`.
2. Go (1.23+), Docker, and `pnpm` on `PATH`. First run builds the Go
   backend and `pnpm install`s the frontend (both slow once, then
   cached).
3. A Playwright Chromium — `#105`'s auto-discovery handles it with no
   manual config on most hosts.

## Running

```sh
# Full run: provisions backend + frontend, verifies, tears down.
duhem run verifications/crawlab-ui-dashboard/duhem.yml

# Against an already-running dashboard (backend :8090, frontend :5188).
duhem run verifications/crawlab-ui-dashboard/duhem.yml --no-env-up
```

## Status

Proven green end-to-end: full provisioning (`up.sh` → checks →
`down.sh`) **and** `--no-env-up`, `verdict: pass` on both criteria (4
assertions). The browser came up via #105's auto-discovery with no
manual config.
