# Owned local processes

Available after v0.7.0 (Unreleased). Use `process/start` for a real local
server or watcher that must remain alive across steps. `cli/invoke` remains
the action for a command that finishes before the next step.

All four actions return observations. None emits `satisfied` or implicitly
judges product behavior. Assert readiness, response contents, publication,
identity and recovery through ordinary VD assertions.

## Launch and readiness

`process/start` accepts an executable `command` string, optional `args`
(string array), `cwd`, `env`, `ready`, `stop_timeout` and `kill_timeout`.
There is no implicit shell and stdin is closed. Relative paths use Duhem's
invocation directory, matching `cli/invoke`. The child receives the sanitized
inherited environment (`PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_*`, `DUHEM_*`),
then authored `env`; runtime store/lineage variables take precedence.
Declare credential inputs `secret: true` before passing them through `env`
or readiness headers.

`ready` requires a `timeout` and exactly one probe:

```yaml
ready:
  http:
    url: http://127.0.0.1:8769/health
    status: 200
    headers: { X-Ready: "yes" }
  timeout: 10s
```

Quote string header values where YAML could interpret them as booleans.
The probe performs GET without following redirects, retrying until that
exact status is observed. Alternatively:

```yaml
ready:
  log: { stream: stdout, pattern: '^READY' }
  timeout: 10s
```

Log readiness checks a continuously drained, generation-local window of at
most 64 KiB. It can match across reads; it never reuses a previous
generation's output. Supply the real product's endpoint or marker.

Readiness, stop and kill budgets must be greater than zero and at most
5 minutes. Stop/kill defaults are 5s/2s. These explicit lifecycle budgets
are independent of manifest `defaults.timeout`; process actions do not
accept a top-level `timeout` input. Omitted readiness returns `ready: false`
and means only that spawning succeeded. It does not establish that a
watcher published anything. Use response observations or `api/poll` for
eventual product conditions.

Outputs: opaque `handle`, integer `generation` (initially 1), diagnostic
`pid` and boolean `ready`. Handles belong to one run, declaring owner and
generation. A PID is never an authored stop target.

## Ownership and restart

A process belongs to the leaf, criterion, check attempt or fixture that
declared its start, independently of temporary browser contexts. A check
may inspect its enclosing leaf/criterion processes but may stop/restart
only processes owned by that check attempt or its fixtures. Sibling checks
and later retries cannot use each other's handles. Validation rejects
resolvable enclosing mutations; runtime ownership checks cover dynamically
supplied handles.

Process outputs from criterion/check setup are addressable as
`$setup.<step>.outputs.<name>`; a consuming check reads its fixture's process
outputs as `$fixture.<fixture>.<step>.outputs.<name>`. This is a narrow
addition for the new process actions; existing non-process setup/fixture
visibility is unchanged.

`process/stop` takes `handle`, sends SIGTERM to its owned group, waits for
the stop budget, then escalates to SIGKILL if needed. It waits for live
descendants, reaps the direct child, and drains log pipes. Outputs are
`stopped`, `already_exited`, nullable `exit_code`/`signal` and `escalated`.
Repeated stop is idempotent and retains the first stop/exit facts. An exited
leader does not mean its remaining descendants are stopped.

`process/restart` takes `handle`, stops/reaps the old generation before
spawning another with its original executable, arguments, working directory,
environment and readiness contract. A failed stop prevents replacement.
The new handle and incremented generation replace the old handle for future
restarts; the old handle remains inspectable. Each generation has separate
logs, readiness and exit evidence. Product-owned files/state are preserved.
Put the process in check-level `setup` or a fixture when a check must restart
it. Reusing a retired generation to fork another replacement is rejected.

## Bounded observations

`process/inspect` takes `handle` and optional integer `cursor` (default 0).
It returns `state` (`running`, `exited`, `stopped`), `running`, nullable
`exit_code`/`signal`, `generation`, `logs`, `next_cursor`, `truncated`,
`dropped_bytes` and `stream_errors`. A nonzero exit after readiness is data
for assertions, not an implicit product failure. `running` describes the
direct executable; only successful stop/finalization establishes group
cleanup.

Each log record has `stream`, monotonic `sequence`, monotonic `elapsed_us`,
`generation` and UTF-8 text (invalid UTF-8 is replaced). Sequences order
runtime reads across both streams, not the child's physical write times.
Pass a returned `next_cursor` to inspect only later retained records; use
the cursor with its original handle. A cursor beyond that generation's
current cursor is an error. Inspecting does not consume records.

Both streams drain continuously. Each retains at most 64 KiB of masked
text and 1024 records. Older records are evicted; `truncated` tells whether
the requested cursor crosses dropped records, and cumulative `dropped_bytes`
counts discarded masked text bytes. An empty delta is not proof that no
output was ever written. `stream_errors` records capture errors.

The live secret registry masks text before retention, including values
split across reads. Incomplete UTF-8 and a possible secret suffix may be
held until another read or EOF; inspect need not expose the latest partial
write immediately. Secret registration applies forward, like other
evidence; transformed or undeclared credentials are outside that guarantee.

`process_observed` events retain the declaring owner, handle, generation,
PID, operation, bounded facts and masked error detail. They cover failed
readiness, old/new restart generations and finalization, alongside ordinary
step observations. Replay retains these facts but never turns them into
extra acceptance judgments.

## Cleanup, failures and platforms

After authored cleanup, the runtime finalizes remaining resources in
reverse creation order at each owner scope exit. This includes setup/body
failure, action timeout, retries and observed cancellation. Pending authored
cleanup drains before process finalization when execution is interrupted.
A teardown already in progress is not restarted. Each retry owns fresh
check/fixture resources. `--keep-env` may skip ordinary leaf teardown but
does not disable owned-process finalization.

The process finalizer is **always required**, even inside a fixture with
`cleanup_policy: diagnostic_only`. There is no per-process policy override.
Its failure independently fails the effective run gate while preserving
the body verdict and cleanup evidence. Existing ordinary teardown/provision
diagnostics keep their semantics. Signal-driven aborts retain cleanup
findings and end as aborted; unobserved hard termination cannot claim
successful cleanup.

Spawn/readiness failure in setup or fixture up makes dependent checks
`inconclusive:environment_error`. In a check body it is a step error with
masked detail and an inconclusive missing observation. Neither is an
implicit acceptance pass/fail.

First-platform support is Linux and macOS. The runtime owns a Unix process
group, keeps its leader unreaped during inspection to pin its identity,
checks live membership, and reaps the leader after the group terminates.
The OS reaps non-child descendants. Windows and other platforms explicitly
reject start **before spawn**; Windows Job Objects are deferred. Programs
must not daemonize, call `setsid` or escape the owned group. Detached services,
remote managers, interactive stdin/JSON-RPC, port allocation and filesystem
comparison are outside this action contract. Fixed-port conflicts remain
the fixture's responsibility.

Run the [executable example](../verifications/owned-process-example/README.md)
to exercise all four actions against a real local HTTP process.
