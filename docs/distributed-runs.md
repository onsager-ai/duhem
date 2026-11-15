# Distributed runs

Each CI machine runs the same `duhem run` with its own environment. Duhem
partitions selected criteria and native runs locally; CI still starts the
machines and gates their results. Each machine must have its own mutable test
environment. Completed run trees can be sent automatically to a Duhem server;
the server computes coverage and serves an aggregated view.

## Send shards to a server

Start an explicitly writable dashboard (the default remains read-only):

```sh
duhem dashboard --db collected.db --ingest --ingest-token "$DUHEM_HUB_TOKEN"
```

On every machine, point the existing hub setting at its ingest route:

```sh
export DUHEM_HUB_URL=https://verification.example/api/ingest
export DUHEM_HUB_TOKEN='<the server ingest token>'
duhem run verifications/sharding-example --shard 1/2 --run-id example-invocation
# The other machine uses --shard 2/2 with the same --run-id.
```

For a local server use `http://127.0.0.1:7878/api/ingest`. Listening on a remote
interface requires `--host` plus a nonempty `--ingest-token`; terminate TLS in
front of a remotely exposed server. No token is required for explicitly enabled
loopback ingest. Each machine uploads parents and descendants after execution;
ordinary unsharded runs upload too. No manual `ship` CI step is needed. Failed
uploads warn once and retain the verdict, exit code and local evidence. Use
`duhem ship <run-id> --db <local.db>` to resend a stored tree; identical content
is a no-op. Requests have a 10-second timeout and one attempt; the automatic tree
send is limited to 30 seconds.

The runs list, detail and failure endpoints expose replay-validated shard trees.
The logical parent's verdict is computed at read time, never stored. Until all
`n` distinct shard indexes arrive, the parent is `pending` with no verdict.
Logical parents have no heartbeat and are never classified as orphaned.

After 30 minutes since the last distinct shard's server receipt, missing work
becomes `inconclusive:missing_observation`. Configure this read-time grace with
`duhem dashboard --shard-timeout 1800` (seconds; zero expires immediately).
There is no background job or CI close signal. A late shard can complete the
coverage and resolve the verdict on the next read; identical resends never reset
the window. Old receipts without server timestamps use the recorded shard start.

With all shards present, the server aggregates recorded leaf verdicts and
required native gates just as for an ordinary run tree. Covered units are the
union of shard partitions. The run page lists each shard's index, verdict,
duration, detection source and unit count, with links to its evidence. Missing
shards list their planned units, reconstructed from `expected_partition` and the
deterministic round-robin assignment. Totals and layers include all shards.
The failure envelope traverses their checks and native runs and adds
`missing_shards: [{index, units}]` after expiry, omitted when empty or pending.

Ingest rejects a different manifest digest or `n` under one logical parent with
HTTP 409 and a specific message. A different content hash for an already stored
shard index is also 409, even if its run ID differs. An identical resend remains
a no-op. A GitHub Actions retry has a new `GITHUB_RUN_ATTEMPT`, so it gets a new
logical parent rather than conflicting with the previous attempt. See the
[bundle contract](run-bundle-contract.md) and the
[worked ingest definition](../verifications/bundle-ingest-example/duhem.yml).

## Detection and overrides

`--shard k/n` overrides `DUHEM_SHARD`, which overrides native CI detection.
Indexes in flags and `DUHEM_SHARD` are **one-based**, like Playwright.
`--run-id` overrides `DUHEM_RUN_ID`, which overrides the shared CI identity.
A flag or generic shard value can use the detected CI identity. Outside CI,
provide a shared identity explicitly. Invalid shard values fail before execution.
Without shard information, ordinary run behavior and JSON remain unchanged.

| Source | Index / total | Native index | Shared identity components |
| --- | --- | --- | --- |
| [GitLab](https://docs.gitlab.com/ci/variables/predefined_variables/) | `CI_NODE_INDEX` / `CI_NODE_TOTAL` | one-based | `CI_PIPELINE_ID`, `CI_JOB_NAME` without its trailing ` k/n` |
| [Buildkite](https://buildkite.com/docs/pipelines/configure/environment-variables) | `BUILDKITE_PARALLEL_JOB` / `BUILDKITE_PARALLEL_JOB_COUNT` | zero-based | `BUILDKITE_BUILD_ID`, `BUILDKITE_STEP_ID` |
| [CircleCI](https://circleci.com/docs/reference/variables/) | `CIRCLE_NODE_INDEX` / `CIRCLE_NODE_TOTAL` | zero-based | `CIRCLE_WORKFLOW_ID`, `CIRCLE_JOB` |
| [GitHub Actions](https://docs.github.com/en/actions/reference/workflows-and-actions/contexts#strategy-context) | Map `strategy.job-index + 1` and `strategy.job-total` to `DUHEM_SHARD` | context index is zero-based | `GITHUB_RUN_ID`, `GITHUB_RUN_ATTEMPT`, `GITHUB_JOB` |

GitHub Actions has no native shard-index environment variable. Its
[default variables](https://docs.github.com/en/actions/reference/variables-reference)
provide the shared identity only when sharding was selected. GitLab's total
alone does not enable sharding: it is also set for nonparallel jobs.
See [GitLab parallel jobs](https://docs.gitlab.com/ci/yaml/#parallel) for the
`job 1/N` naming convention and the [Playwright GitLab example](https://playwright.dev/docs/ci#gitlab-ci)
that passes `CI_NODE_INDEX` directly to its one-based `--shard` flag.
CI workflow recipes will follow separately.
Use a new shared identity for each logical invocation; if a provider's retry
keeps these components unchanged, include your attempt identifier explicitly.

## Partition and lifecycle

Selection (`--filter` and directory selection) happens before partitioning.
Each selected `(leaf, criterion)` is one indivisible unit, even if its filter
selects only some checks. Root-manifest and leaf-level native runs are units too.
Unit IDs are JSON-encoded triples `[kind, relative-leaf-path, authored-id]`;
`$manifest` identifies suite-native runs and `$leaf` a single-leaf invocation.
Duhem sorts these IDs lexically and assigns position `i` to shard
`(i mod n) + 1`. Round-robin keeps unit counts within one; it does not estimate
duration. The same definitions, selection and `n` produce disjoint partitions
covering the selection, without coordination or machine-dependent hashes.

A shard runs only the leaves it needs. Provisioning, leaf setup, teardown and
cleanup run on each shard that owns a unit in that leaf. Checks of one criterion
never cross machines. A shard with no assigned units succeeds when the overall
selection is nonempty; an empty user selection retains its inconclusive verdict.
`--dry-run` shows the narrowed plan without opening the evidence store.

## Evidence and identity

`--run-id` is allowed on manifests when sharding. Its value names the logical
parent, rather than pinning a shard's own record. Duhem hashes this shared
identity with a versioned namespace into a valid ULID; each shard gets a fresh
ULID. A local unjudged logical-parent row satisfies SQLite's parent foreign key.
It is intentionally left unfinished, because one machine cannot judge all
shards. Manifest leaf records remain children of their shard's suite record.

Shard JSON summaries add these optional fields, also persisted in the
`claim_scope_declared` evidence event:

```json
{
  "origin": "shard",
  "parent_run_id": "<logical-parent-ULID>",
  "shard": {"index": 1, "total": 2, "source": "flag"},
  "manifest_digest": "<SHA-256>",
  "expected_partition": {
    "units": ["[\"criterion\",\"feature/duhem.yml\",\"AC-1\"]"],
    "total_units": 8
  }
}
```

The digest covers the parsed manifest and loaded leaf definitions before
selection and input resolution, including composed definitions. It excludes
absolute source paths and resolved runtime inputs. The expected partition lists
this shard's unit IDs, the total selected unit count, and other selected units
in `excluded: [{unit, reason: "shard"}]` (including native units). Check exclusions carry
`reason: shard`; user-filter exclusions retain `reason: check_filter`. Native
exclusions remain available in `excluded_native_runs`. Ordinary summaries omit
all distributed fields. On manifest leaves the plan describes their enclosing
shard; the actual store lineage still points to the local suite record.

The [worked definition](../verifications/sharding-example/duhem.yml) can be run
on two machines with the same identity:

```sh
duhem run verifications/sharding-example --shard 1/2 --run-id example-invocation --reporter json
duhem run verifications/sharding-example --shard 2/2 --run-id example-invocation --reporter json
```
