# `crawlab-spider-task-lifecycle`

Duhem's **deepest** Crawlab Verification Definition — it drives a spider
through Crawlab's distributed **task lifecycle** end to end: create, run,
and **execute to completion** on a real worker, then read the terminal
state back from MongoDB with `db/query` (#121). The companion
[`../crawlab-create-project/`](../crawlab-create-project/) covers the flat
REST + Mongo surface; this VD covers the gRPC master/worker execution
path that is Crawlab's distinctive value.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh)

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | Create a spider; it is persisted as a real document in Mongo's `spiders` collection (same `_id` the API returned, name + command). |
| **AC-2**  | Run the spider; a worker claims it over gRPC, syncs its files, executes the command, and the task reaches the terminal `finished` state — awaited over REST (`api/poll`) and confirmed in Mongo's `tasks` collection. |
| **AC-3**  | The executed task references exactly the created spider (`tasks.spider_id == spiders._id`) and is the task the run reported. |

Real auth, a real REST create + file-save + run, a real gRPC master/worker
execution, and real MongoDB reads — no mocks at the web boundary
(`docs/duhem-spec.md` §8). All REST and storage shapes were verified from
`crawlab/core` source, not guessed (spider/task models, collection tags,
the run + save-file + task-get endpoints, the status enum).

## Why a Pro cluster

Task **execution** (`pending → assigned → running → finished`) needs
Crawlab's gRPC worker coordination. The gRPC server is started only in
Crawlab's **Pro layer** (`crawlab-pro/core/apps/grpc.go`), *outside* the
open-source `crawlab/` submodule. The license-free OSS core never starts
it, so a task there stays `pending` with an unassigned node — which is
exactly the scope boundary the create-project VD documents. To exercise
the real lifecycle, `up.sh` runs the **Pro** binary (`crawlab-pro/core`)
as a master plus one worker over a throwaway MongoDB.

### License

Crawlab Pro verifies a JWT license against a built-in HMAC key — no
license *server* is contacted. For local dogfooding of one's own
product, the **permanent test license committed in crawlab-pro**
(`core/license/service_test.go`, issued to `tikazyq@163.com`, no
expiry) validates. Supply it to the VD via `DUHEM_CRAWLAB_LICENSE`
(Duhem's hook env is whitelisted to `PATH`/`HOME`/`TMPDIR`/`LANG`/
`LC_*`/`DUHEM_*`, so a bare `CRAWLAB_LICENSE` would not reach `up.sh`).
The token is **not** committed into this repo — the operator, who owns
the product, supplies it.

## Provisioning

`up.sh` builds the Pro binary once (`go build`, cached after the first
run), starts a throwaway MongoDB container, then launches a **master**
(REST `8090`, gRPC `9666`) and a **worker** (connecting to the master's
gRPC, syncing files from the master's REST endpoint). Each node gets an
isolated `HOME` so it holds a distinct identity; Go's caches stay on the
real `HOME`. `down.sh` reaps both nodes and removes the Mongo container.

A key worker setting: `CRAWLAB_API_ENDPOINT` must point at the master's
REST port (the worker pulls spider files over HTTP from there). `up.sh`
wires this to `http://127.0.0.1:8090`; without it the worker would try
the default `:8000`, the file sync would 404, and the task would fail
with `fork/exec … no such file or directory` (a missing workspace).

### Operator setup

1. A `crawlab-team/crawlab-pro` checkout (Pro `core` + the `crawlab/`
   OSS submodule present):
   ```sh
   export DUHEM_CRAWLAB_REPO_DIR=/path/to/crawlab-team/crawlab-pro
   ```
   `up.sh` defaults to `../../../../crawlab-team/crawlab-pro`.
2. The Pro license:
   ```sh
   export DUHEM_CRAWLAB_LICENSE=<jwt>   # the committed test license works
   ```
3. Go (1.23+) and Docker on `PATH`. The first `go build` is slow;
   subsequent runs reuse the cache.
4. A Playwright Chromium for Duhem's browser. The checks are REST + DB
   only, but `api/call` reports `requires_page = true` today, so the
   runtime opens a browser — `#105`'s auto-discovery handles this with no
   manual config on most hosts.

### Iterating

`up.sh`/`down.sh` are wired into `environment:`, so `duhem run` brings
the cluster up and tears it down. To iterate against an already-running
cluster, pass `--no-env-up --keep-env`.
