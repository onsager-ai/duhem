# `crawlab-git-root-path`

Holistic regression Verification Definition for the client-reported Crawlab
bug where a git spider configured with a **git root path of `~`** could not
sync its files and the task ended at startup without running the script
(surfacing as `fork/exec /usr/bin/bash: no such file or directory`). It drives
the real Pro cluster: create the spider exactly as the client's, run it, and
assert the distributed task lifecycle reaches `finished` — read from MongoDB.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Provisioning: [`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh)

Forked from the sibling [`../crawlab-spider-task-lifecycle/`](../crawlab-spider-task-lifecycle/);
read that VD's README for the Pro-cluster / license / operator-setup details,
which apply here unchanged except as noted below.

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | A git spider (`git_id` set) with `git_root_path: "~"` is persisted to Mongo's `spiders` collection verbatim — the `~` root path and the supplied git id, read straight from the store. |
| **AC-2**  | Running that `~`-root spider drives the lifecycle to the terminal `finished` state (worker claims over gRPC, syncs the repo-root files, executes) — awaited over REST and confirmed in Mongo's `tasks` collection. This is the regression boundary. |

## Why the workspace pre-seed (the crux)

The faithful reproduction needs the spider's files at the repository **root**
(`<workspace>/<gitId>/main.sh`) — exactly where Crawlab's git clone puts them
— while the spider's `git_root_path` is `~`. Pre-fix, the scan looked at the
non-existent `<workspace>/<gitId>/~` and the task errored.

So `up.sh` writes the file straight into the master workspace at the repo
root, **simulating the clone**, and the VD only sets `git_root_path: "~"` on
the spider. Seeding the file through Crawlab's own `files/save` API would
instead write it to the spider's (broken) root path, moving it *with* the bug
and masking the regression. The fabricated `git_id` is shared between `up.sh`
(`DUHEM_CRAWLAB_GIT_ID`) and `duhem.yml` (the `git_id` input) and must match.

No real Git resource / clone is needed: at task time the worker reads only
`spider.git_id` + `spider.git_root_path` from Mongo to decide the sync path —
it does not query the `gits` collection.

## Demonstrated before/after

Same VD, same harness, only the crawlab-pro binary differs:

| Binary | AC-1 | AC-2 | Persisted task status |
| ------ | ---- | ---- | --------------------- |
| pre-fix (`6e6586b8`) | pass | **fail** | `error` (the client's symptom) |
| fixed (`4e0de456`)   | pass | **pass** | `finished` |

The deterministic, fast fail-without-fix guards live in crawlab-pro itself
(`TestGitRootPathTildeScanE2E`, `TestConfigureCwdNormalizesGitRootPath`,
`TestNormalizeGitRootPath`); this VD is the process-level, whole-cluster proof.

## Ports

Defaults are offset from the lifecycle VD — REST **8190**, gRPC **9766**,
Mongo **27019** — so this can run beside a local Crawlab dev stack holding the
conventional `8090`/`9666`/`27018`. The master's gRPC **server** port is set
via `CRAWLAB_GRPC_SERVER_PORT` (note: `CRAWLAB_GRPC_PORT` only sets the
worker's *target*); a writable `CRAWLAB_LOG_PATH` is set per node so the task
log driver does not need `/var/log/crawlab`. Keep `up.sh` and `duhem.yml` in
sync.

## Run

```sh
export DUHEM_CRAWLAB_REPO_DIR=/path/to/crawlab-team/crawlab-pro
export DUHEM_CRAWLAB_LICENSE=<jwt>   # HS256 over key "test-secret", no expired_at
cargo run -p duhem-cli -- run verifications/crawlab-git-root-path/duhem.yml
```
