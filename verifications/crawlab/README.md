# `crawlab` — verification suite

A Duhem verification **suite**: several Crawlab Verification Definitions
that share **one** stack. The root manifest ([`duhem.yml`](duhem.yml))
declares a manifest-level `environment:` (spec #131), so the runtime
provisions a single Crawlab master + MongoDB once, runs every leaf
against it, and tears it down once — instead of each leaf standing up
its own. This is the "1→N" infrastructure: adding a leaf costs an entry
in the manifest, not another stack bring-up.

```
duhem run verifications/crawlab               # up once, run all leaves, down once
duhem run verifications/crawlab --filter auth # one leaf, still shared stack
duhem run verifications/crawlab/auth          # one leaf, standalone (needs its own stack)
```

## Leaves

| Leaf | What it verifies |
| ---- | ---------------- |
| [`auth/`](auth/duhem.yml) | The auth boundary both ways: a valid login issues a token that authenticates a gated endpoint (`GET /projects` → 200), and the same endpoint rejects an unauthenticated request (→ 401). |
| [`projects/`](projects/duhem.yml) | A project created over REST is actually persisted in Mongo's `projects` collection (read back with `db/query`, same `_id`). |

Each leaf has **no** `environment:` of its own — it targets the shared
stack (`8090` REST, `27018` Mongo). Run a leaf standalone by pointing it
at an already-up Crawlab and passing `--no-env-up`.

The leaves' shared inputs (`login_url`, `projects_url`, `username`,
`password`) are **inherited** from the manifest's `environments:` block
(spec #135) — declared once, not redeclared per leaf. A leaf run under
the suite binds them automatically; a standalone leaf run must supply
them with `--inputs` (e.g. `--inputs login_url=http://127.0.0.1:8090/login --inputs username=admin ...`), or the run fails loudly naming the missing inherited input.

## Provisioning

`scripts/up.sh` / `scripts/down.sh` bring up Crawlab's **open-source
core** (the `crawlab/` submodule) from source plus a throwaway MongoDB —
the same license-free recipe the standalone create-project VD uses. The
manifest's `ready:` probe gates the leaves on Crawlab's `/health`.

The deeper distributed-execution lifecycle (a task driven to `finished`
on a worker) is **not** in this suite — it needs a licensed Pro
master+worker cluster, so it lives in its own VD
([`../crawlab-spider-task-lifecycle/`](../crawlab-spider-task-lifecycle/)).
A suite sharing a Pro cluster is a natural next step once this OSS suite
grows.

### Operator setup

```sh
export DUHEM_CRAWLAB_REPO_DIR=/path/to/crawlab-team/crawlab-pro
duhem run verifications/crawlab
```

`up.sh` defaults `DUHEM_CRAWLAB_REPO_DIR` to `../../../../crawlab-team/crawlab-pro`.
Go (1.23+) and Docker must be on `PATH`; the first `go run` build is
slow, then cached. To iterate against an already-running suite stack,
add `--no-env-up --keep-env`.
