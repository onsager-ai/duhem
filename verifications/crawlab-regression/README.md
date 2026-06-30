# `crawlab-regression` — verification suite

The foundation of the Crawlab 0.2 regression suite (Duhem issues
#160 / #163): a faithful Duhem port of crawlab-team/crawlab-test's spec
corpus, run against the **real** crawlab-pro product. This first slice
ports **API-002 — Authentication & Token Management**; the rest of the
API / INT / UI / REL specs fan out behind it once this pattern is
validated.

A Duhem verification **suite**: several Crawlab regression Verification
Definitions that share **one** licensed Pro cluster. The root manifest
([`duhem.yml`](duhem.yml)) declares a manifest-level `environment:`
(spec #131), so the runtime provisions a single crawlab-pro
**master + worker + MongoDB** once, runs every leaf against it, and tears
it down once — instead of each leaf standing up its own. Adding a leaf
costs an entry in the manifest, not another cluster bring-up.

> **Status: authored + structurally validated; pending product-env green.**
> This VD verifies the real crawlab-pro product and CANNOT run green in a
> generic sandbox (it needs the `crawlabteam/crawlab-pro:develop` image and
> a running cluster). It is validated structurally here (`duhem validate`,
> `duhem run --dry-run --no-env-up`); the **green run happens in the product
> env / Crawlab's CI**. It is intentionally **NOT** wired into Duhem's CI
> (no self-verify lane for it).

```
duhem run verifications/crawlab-regression                  # up once, run all leaves, down once
duhem run verifications/crawlab-regression --no-env-up      # against an already-up cluster
duhem run verifications/crawlab-regression --no-env-up --keep-env  # iterate live
```

## Leaves

| Leaf | What it verifies |
| ---- | ---------------- |
| [`auth-tokens/`](auth-tokens/duhem.yml) | API-002: login issues a usable session token (→ `/api/users/me`); the API-token CRUD lifecycle is backed by Mongo's `tokens` collection; logout invalidates the session; the gate rejects missing/malformed tokens (401); login refuses bad credentials. |
| [`spiders/`](spiders/duhem.yml) | API-004: spider CRUD — create persists every field to Mongo's `spiders` collection; get/list (with `?page=&size=` + total); PATCH partial / PUT full update verified in Mongo; delete is gone from API (404) and Mongo; invalid id → 404; duplicate name → 4xx (contract claim — likely RED, see leaf header). |
| [`spider-files/`](spider-files/duhem.yml) | API-005: spider file management — save a file and read it back byte-for-byte (real workspace round-trip); file info (is_dir false); directory create + nested-file listing; copy/rename/delete lifecycle; missing path → 4xx. |
| [`tasks/`](tasks/duhem.yml) | API-006: task CRUD & execution — the deepest leaf: run a spider to `finished` over the gRPC worker and assert the persisted `tasks` doc (status, `cmd`, `spider_id` link, reported id); `/api/tasks/run` create+link; list/pagination/get-by-id; delete a finished task → 404. |
| [`task-logs/`](task-logs/duhem.yml) | API-007: task logs & results — after a `finished` task, logs endpoint returns a non-empty body (+ paginated form); results endpoint is a well-formed 200; logs/results for an unknown task id → 4xx. |

Each leaf has **no** `environment:` of its own — it targets the shared
cluster (`8090` REST under `/api`, `27018` Mongo). The leaves' shared
inputs (endpoint URLs, `username`, `password`, `mongo_url`) are
**inherited** from the manifest's `environments:` block (spec #135) —
declared once, not redeclared per leaf. A leaf run under the suite binds
them automatically; a standalone leaf run must supply them with `--inputs`
or the run fails loudly naming the missing inherited input.

## Provisioning

`scripts/up.sh` / `scripts/down.sh` bring up
[`scripts/docker-compose.yml`](scripts/docker-compose.yml) — a
mongo + master + worker cluster on the official
`crawlabteam/crawlab-pro:develop` image, modeled on
crawlab-team/crawlab-test's `docker-compose.test.yml`. The manifest's
`ready:` probe gates the leaves on the Pro image's `/api/health`.

**Why docker compose** (and not the go-build-from-source recipe in
[`../crawlab-spider-task-lifecycle/scripts/up.sh`](../crawlab-spider-task-lifecycle/scripts/up.sh)):
this suite verifies the Pro product **as it ships**, needs no crawlab-pro
source checkout, and mirrors Crawlab's own CI blueprint — the cleanest
reusable shape for a growing regression suite. The from-source recipe
remains the right tool when verifying uncommitted source.

### License

The `develop` image verifies an **HS256 JWT** license against the
hardcoded dev key `test-secret` (claims: `created_at` unix-int + any
`username`; header `{"alg":"HS256"}`, no `typ`).
[`scripts/mint-license.sh`](scripts/mint-license.sh) mints a **fresh
dev/test license at provision time** and `up.sh` injects it as
`CRAWLAB_LICENSE` — it is never hardcoded or committed. This is the
maintainer-sanctioned **dev/test** license for the regression cluster
(the maintainer owns crawlab-pro); it is **not** a production license. To
use a real license instead, export `CRAWLAB_LICENSE` before `duhem run`
and `up.sh` will use it as-is.

### Image under test

By default the suite runs `crawlabteam/crawlab-pro:develop`. Select a
different image with **`DUHEM_CRAWLAB_IMAGE`** (the Duhem env whitelist only
passes `DUHEM_*` through, so a bare `CRAWLAB_IMAGE` won't reach `up.sh`):

```sh
# the published test tag
DUHEM_CRAWLAB_IMAGE=crawlabteam/crawlab-pro:test duhem run verifications/crawlab-regression
# or a locally-built tag, e.g. before/after a fix for regression diffing
DUHEM_CRAWLAB_IMAGE=crawlab-pro:after duhem run verifications/crawlab-regression
```

(Both `:develop` and `:test` currently exhibit the bugs tracked in #167.)

### Operator setup (to run it green in a product env)

```sh
# Prereqs: docker + docker compose v2 on PATH; python3 OR openssl (for the
# license mint); host ports 8090 (REST/UI), 9666 (gRPC), 27018 (Mongo) free.

# Pull the Pro develop image (must be reachable from your registry/login):
docker pull crawlabteam/crawlab-pro:develop

# From the Duhem repo root:
duhem run verifications/crawlab-regression
```

Ports / gotchas baked into the compose (from the proven recipe):

- Master REST/UI on host **8090** (→ container 8080; `/api` proxied), off
  the dev machine's Traefik on 8080/8000.
- Mongo on **27018** (admin/admin, `authSource=admin`); gRPC on **9666**.
- The worker reaches the master by service name on the **internal**
  container port (`http://master:8080`), not the host 8090 — it is on the
  compose network.
- `down -v` drops the Mongo volume, so each suite run starts clean.

To iterate against an already-running cluster, bring it up once
(`sh scripts/up.sh`) and run leaves with `--no-env-up --keep-env`.
