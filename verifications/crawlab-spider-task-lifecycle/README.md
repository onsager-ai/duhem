# `crawlab-spider-task-lifecycle`

Duhem's **deep** Crawlab Verification Definition — the companion to
[`../crawlab-create-project/`](../crawlab-create-project/). Where that VD
verifies a flat resource over REST + a single Mongo write, this one
drives Crawlab's distributed **task lifecycle**: it creates a spider,
runs it, and reads the scheduled task back from Crawlab's real MongoDB
with `db/query` (#121) — asserting the cross-collection linkage
(`tasks.spider_id == spiders._id`) that only a direct database read can
prove.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh)

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | Create a spider; it is persisted as a real document in Mongo's `spiders` collection (same `_id` the API returned, same name + command), read straight from the store. |
| **AC-2**  | Run the spider; the run persists a real task in Mongo's `tasks` collection in the initial `pending` state, carrying the spider's command and the id the run reported. |
| **AC-3**  | The persisted task references exactly the created spider — `tasks.spider_id == spiders._id`, verified by reading both collections from Mongo. |

Real auth (login → JWT), a real REST create + run, and real MongoDB
reads — no mocks at the web boundary (`docs/duhem-spec.md` §8). All REST
and storage shapes were verified from `crawlab/core` source, not guessed
(spider/task models, collection tags, the run endpoint, status enum).

## Scope boundary — scheduling, not execution

This VD verifies task **scheduling** (the `pending` task the master's
spider-admin Schedule service writes to Mongo), not task **execution**
(`pending → running → finished`).

Execution requires Crawlab's gRPC worker coordination. The gRPC server is
started in Crawlab's **Pro layer** (`crawlab-pro/core/apps/grpc.go`),
*outside* the open-source `crawlab/` submodule that the license-free
recipe below provisions — so the OSS-core stack never starts a gRPC
server, no worker can register, and a scheduled task stays `pending` with
an unassigned `node_id` (`000…000`). This was confirmed live while
authoring the VD: a master-only OSS node creates the task but nothing
claims it.

A deeper **execution** VD (asserting `running` → `finished` state
transitions in Mongo, optionally streaming task logs) waits on a stack
that starts the gRPC server — either licensed Crawlab Pro or an OSS
master + worker pair with the gRPC server wired up. The `db/query`
MongoDB read path this VD relies on is already sufficient for those
assertions; only the environment is missing.

## Provisioning — license-free by design

Identical to the create-project VD: `up.sh` runs Crawlab's open-source
core (the `crawlab/` submodule) from source with `go run` (no license
gate), plus a throwaway MongoDB container. An isolated `HOME` makes the
node register as **master**; Go's build cache stays pointed at the real
`HOME` so `go run` is fast after the first build. Backend on **8090**
(Crawlab's own 8080 is held by the dev-proxy Traefik); Mongo on host
**27018**, db `crawlab`.

### Operator setup

1. A `crawlab-team/crawlab-pro` checkout (the `crawlab/core` OSS
   submodule must be present):
   ```sh
   export DUHEM_CRAWLAB_REPO_DIR=/path/to/crawlab-team/crawlab-pro
   ```
   `up.sh` defaults to `../../../../crawlab-team/crawlab-pro`.
2. Go (1.23+) and Docker on `PATH`. The first `go run` build is slow;
   subsequent runs reuse the cache.
3. A Playwright Chromium for Duhem's browser. The checks are REST + DB
   only, but `api/call` reports `requires_page = true` today, so the
   runtime opens a browser — `#105`'s auto-discovery handles this with no
   manual config on most hosts.

### Iterating

`up.sh`/`down.sh` are wired into `environment:`, so `duhem run` brings
the stack up and tears it down. To iterate against an already-running
stack, pass `--no-env-up --keep-env`.
