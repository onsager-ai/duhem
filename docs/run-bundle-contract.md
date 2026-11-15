# The run-bundle wire contract (#194)

One immutable run, self-contained, versioned — the seam between the
open-source engine and the hosted hub (#188 open-core boundary). The
closed-source hub builds against this document plus the contract test
in `crates/duhem-evidence/tests/bundle_contract.rs`; the test is the
enforcement, this file is the narrative.

## Envelope (`bundle_version: 1`)

The canonical wire form is the compact JSON serialization of
`duhem_evidence::RunBundle`:

```jsonc
{
  "bundle_version": 1,
  "run": {
    "run_id": "01…",                    // ULID
    "verification": "verifications/login/duhem.yml",
    "schema_version": "v1",             // trace wire version (#10)
    "inputs": { … },
    "started_at": "2026-07-06T00:00:00.000Z",
    // present once judged:
    "verdict": "pass",                  // pass | fail | inconclusive:<cause>
    "finished_at": "…", "duration_ms": 1000,
    // #190 scope + provenance (all optional):
    "project_id": "github.com/acme/app",
    "verifier_repo": "github.com/onsager-ai/duhem", "verifier_sha": "…",
    "target_repo": "github.com/acme/app", "target_sha": "…",
    // #650 lineage and distributed plan (all optional):
    "parent_run_id": "01…", "origin": "shard", // suite | invocation | shard
    "shard": {"index": 1, "total": 2, "source": "env"},
    "manifest_digest": "…64 hex…",
    "expected_partition": {"units": ["…"], "total_units": 8}
  },
  "events": [ { "seq": 0, "ts": "…", "kind": "…", … } ],   // full #10 stream, seq order
  "artifacts": [ { "sha256": "…64 hex…", "bytes_base64": "…" } ]  // sorted by sha
}
```

Everything the hub needs is inside: replay recomputes the verdict
from `events`; the normalized projections (criteria / checks /
assertions / spans) are derivable server-side exactly as the local
store derives them. Shipping is **replication, not dual-truth**: a
bundle is immutable and identified by content.

## Identity & idempotency

The bundle's **content hash** is the lowercase-hex SHA-256 of the
canonical wire bytes. `duhem ship` sends it as
`X-Duhem-Content-Hash`; a hub that already stores that hash answers
2xx without re-storing. Re-shipping the same run is therefore always
safe.

## Transport

`POST <DUHEM_HUB_URL>` with:

| Header | Value |
|--------|-------|
| `Content-Type` | `application/json` |
| `X-Duhem-Bundle-Version` | `1` |
| `X-Duhem-Content-Hash` | the content hash |
| `Authorization` | `Bearer <DUHEM_HUB_TOKEN>` (when configured) |

2xx = ingested (or already present). `duhem run` automatically sends each
completed invocation when the URL is set, including ordinary unsharded runs.
`duhem ship <run-id>` uses the same transport for manual delivery. Each request
has a 10-second timeout and one attempt (no retry); automatic delivery has a
30-second limit for the complete tree. A send failure prints one warning line
on stderr and cannot change the verdict, local evidence or exit code. Nested
invocations are delivered with their enclosing invocation after it finishes.
Dry runs and failures before execution produce no upload.

The OSS dashboard enables `POST /api/ingest` only with `--ingest`. Set
`DUHEM_HUB_URL=http://127.0.0.1:7878/api/ingest` to use it. `--ingest-token`
requires the matching `DUHEM_HUB_TOKEN` bearer token; a non-loopback listen host
requires a nonempty token. Default dashboard and static-export mode stay
read-only. Ingest limits the request body to 16 MiB, validates the version,
canonical content hash, event sequence, header/event agreement and artifact
hashes, then replays judged event streams before committing. Unjudged suite
containers and logical parents stay unjudged. The supplied header verdict,
finish time and duration are never used as judgment inputs. Receipts, events,
blobs and their normal projections commit atomically; an identical receipt is
a no-op. Serve remotely behind TLS when sending a bearer token.

The v1 transport is a single JSON envelope (base64 artifacts). Runs
with large artifact sets may outgrow it; a chunked/multipart
transport would be `bundle_version: 2` — tracked as the open
question on #194.

## Two destinations, one format

`duhem export <run-id>` writes the same `RunBundle` as a browsable
directory (`bundle.json` header + `events.jsonl` + `artifacts/…`),
and `RunBundle::from_dir` reads it back byte-equivalently — the
round-trip the contract test pins.

## Change discipline

Any field rename, removal, or semantic change bumps `BUNDLE_VERSION`,
updates the golden in `bundle_contract.rs`, and lands with a
coordinated hub-side change. Additive optional fields are allowed
within a version; the hub must ignore unknown fields.

The #650 extension adds optional `run.parent_run_id`, `run.origin`, `run.shard`,
`run.manifest_digest` and `run.expected_partition` within v1. Old envelopes keep
their exact canonical bytes; older readers ignore these fields. Each envelope
still contains **one run**. A tree is a parent-first sequence of envelopes,
linked by `parent_run_id`, with independent content hashes and receipts. This
keeps the original export directory and artifact layout unchanged and lets a
failed upload resume by safely re-sending the same envelopes. `ship` includes
descendants and the ancestors needed to resolve lineage. Shard arrivals can
also create their logical parent idempotently when it is absent. Independently
recorded logical-parent start times may differ; the first unjudged parent wins.

Distributed metadata comes from `claim_scope_declared`; lineage comes from the
store row and agrees with `run_started`. Leaf metadata can describe the enclosing
shard while its actual parent remains the suite container. Ingest stores these
facts for later coverage computation. Pending/inconclusive coverage, digest/count
conflict rejection across shards and the aggregated UI remain a later #650
slice. Until then, logical parents have no verdict, even when all shards arrive.
