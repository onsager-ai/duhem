# Crawlab — a git spider with git root path "~" syncs and runs to completion

Acceptance criteria for the client-reported regression where a git-backed
Crawlab spider configured with a **git root path of `~`** failed in
production: the worker could not scan/sync the spider's files
("线上环境扫不到我的文件") and the task ended at startup without running the
script ("直接 crawlab 启动阶段就结束了，不执行任务脚本"), surfacing as
`fork/exec /usr/bin/bash: no such file or directory`.

Root cause: `~` was joined verbatim into `<workspace>/<gitId>/~` — a path
that does not exist on disk — so the master file scan failed
(`lstat .../~: no such file or directory`), nothing synced to the worker,
the worker working directory was never created, and the task command could
not `chdir` into it. Fix (crawlab-pro `4e0de456`):
`utils.NormalizeGitRootPath` maps `~` (and `~/`, `/`, `.`, `..`, …) to the
repository root, applied at every git-root consumer (worker cwd, HTTP + gRPC
sync, `GetSpiderRootPath`, and the master scan handlers).

Verified against a real Crawlab **Pro** cluster (master + worker over gRPC,
backed by a real MongoDB, no mocks at the web boundary). `scripts/up.sh`
seeds the master workspace like a freshly *cloned* repo — the spider's files
live at the repository ROOT (`<workspace>/<gitId>/main.sh`), not under a
`~/` sub-folder. Seeding through Crawlab's own `files/save` would instead
write the file to the spider's (broken) root path, moving it *with* the bug
and masking the regression; writing at the repo root is what makes this a
true guard. The deterministic fail-without-fix unit/integration guards live
in crawlab-pro itself (`TestGitRootPathTildeScanE2E`,
`TestConfigureCwdNormalizesGitRootPath`, `TestNormalizeGitRootPath`); this VD
is the holistic, process-level proof across the real distributed lifecycle.

Demonstrated before/after on this VD: against the pre-fix binary AC-2 fails
with the persisted task `status == "error"` (the client's symptom); against
the fixed binary it reaches `finished`.

## AC-1

A spider can be configured exactly as the client's — git-backed (`git_id`
set) with `git_root_path: "~"` — and the configuration persists to Crawlab's
Mongo `spiders` collection verbatim (the `~` root path and the supplied git
id), read straight from the store rather than echoed by the handler.

## AC-2

Running the `~`-root git spider drives the full distributed task lifecycle to
the terminal `finished` state: the worker claims the run over gRPC, syncs the
repo-root files the clone left at `<workspace>/<gitId>/`, executes the
command, and the task reaches `finished` — confirmed in MongoDB, not inferred
from the API. Pre-fix the sync failed on `<gitId>/~` and the task reached
`error`; this criterion is the regression boundary.
