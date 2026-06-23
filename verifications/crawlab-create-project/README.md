# `crawlab-create-project`

Duhem's third dogfood Verification Definition — and the first against a
genuinely **independent** vendor, Crawlab Pro
([`crawlab-team/crawlab-pro`](https://github.com/crawlab-team/crawlab)),
a Go REST + gRPC distributed crawler over MongoDB. It drives the real
Crawlab REST API and verifies the golden resource flow.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh)

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | Log in, create a project; the API persists it and returns a well-formed Mongo `_id` (24-hex, via #104 nested navigation) and the supplied name. |
| **AC-2**  | An authenticated `GET /projects` returns a well-formed, non-empty list from the real database. |

Real auth (login → JWT → `Authorization: <token>`), a real Mongo write,
and a real authenticated read — no mocks at the web boundary
(`docs/duhem-spec.md` §8). All REST shapes were verified from
`crawlab/core` source, not guessed (login/token/projects/health/
ObjectID/admin-seed).

## Provisioning — license-free by design

Crawlab Pro's `develop` image **panics on an empty license**
(`core/apps/license.go`). To stay license-free and reproducible,
`up.sh` runs Crawlab's **open-source core** (the `crawlab/` submodule)
from source with `go run` — its `main.go` has no license gate, and it
serves the exact REST contract this VD targets. An isolated `HOME` makes
the node register as **master** (Crawlab persists node identity in
`$HOME/.crawlab/config.json`; a developer's existing worker node would
otherwise be loaded and the master-only services panic). Go's build
cache is kept pointed at the real `HOME` so `go run` stays fast.

Default port **8090** (Crawlab's own 8080 is held by the dev-proxy
Traefik on the dev machine).

### Operator setup

1. A `crawlab-team/crawlab-pro` checkout (the `crawlab/core` OSS
   submodule must be present):
   ```sh
   export DUHEM_CRAWLAB_REPO_DIR=/path/to/crawlab-team/crawlab-pro
   ```
   `up.sh` defaults to `../../../../crawlab-team/crawlab-pro`.
2. Go (1.23+) and Docker on `PATH`. The first `go run` build is slow;
   subsequent runs reuse the cache.
3. A Playwright Chromium for Duhem's browser. The checks are REST-only,
   but `api/call` reports `requires_page = true` today, so the runtime
   opens a browser — `#105`'s auto-discovery handles this with no manual
   config on most hosts.

### Running against Crawlab **Pro** instead

Supply a license and use the maintainers' Pro stack
(`crawlab-pro/docker/dev/docker-compose.yml`, `CRAWLAB_LICENSE=…`),
remapping the master off 8080, and point the VD's URL inputs at it
(`--inputs login_url=… projects_url=… health_url=…`).

## Running

```sh
# Full run: provisions Crawlab, verifies, tears down.
duhem run verifications/crawlab-create-project/duhem.yml

# Against an already-running Crawlab master on :8090.
duhem run verifications/crawlab-create-project/duhem.yml --no-env-up
```

## Scope

REST-only for v1. Crawlab's primary store is **MongoDB**, which Duhem's
`db/*` actions (SQL-only, #101) can't read — so AC-1/AC-2 assert via the
REST API. Crawlab's distributed task lifecycle and worker gRPC are a
deeper, later VD.

## Status

Proven green end-to-end: full provisioning (`up.sh` → checks →
`down.sh`) **and** `--no-env-up`, `verdict: pass` on both criteria (7
assertions, including the nested-`_id` ObjectID match and the
name round-trip).
