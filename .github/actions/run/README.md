# `duhem/run` — composite GitHub Action

Runs a Duhem Verification Definition against the calling workflow's
environment and surfaces a verdict suitable for a required-check
status gate.

Spec: [`onsager-ai/duhem#36`](https://github.com/onsager-ai/duhem/issues/36).
Product context: `docs/duhem-spec.md` §12.1 (GitHub integration)
and §11.2 (trust boundary).

> **Phase 0 status.** This action is the wiring for the first
> deployment — the Onsager dashboard create-project Verification
> Definition. The action surface generalizes to additional VDs and
> additional consumers as Phase 1 alpha customers adopt it
> (`docs/duhem-spec.md` §14). Pre-`v1`, the action lives at
> `onsager-ai/duhem/.github/actions/run@v0.<n>` tags; consumers
> should pin to a tag, never `@main`.

## Inputs

| Name                | Required | Default | Description |
|---------------------|----------|---------|-------------|
| `verification-path` | yes      | —       | Path to a `.yml` Verification Definition, **resolved relative to the `onsager-ai/duhem` repo at the pinned action tag** — not relative to the caller's checkout. Absolute paths and paths that normalize to anything outside the Duhem checkout are rejected before invoking `duhem run`; see "Trust contract" below. |
| `inputs`            | no       | `""`    | Newline-separated `key=value` pairs forwarded as repeated `--inputs key=value` flags to `duhem run`. Coerced per the Verification Definition's typed input catalog. |
| `reporter`          | no       | `json`  | Stdout reporter (`default` / `quiet` / `json`, plus plugin reporters from `.duhem.toml`). The action's `verdict` + `evidence-dir` outputs depend on parsing the json summary, so leave at the default unless you have a plugin that emits the same single-line JSON contract. |

## Outputs

| Name           | Description |
|----------------|-------------|
| `verdict`      | `pass` / `fail` / `inconclusive:<cause>` as emitted by `duhem run --reporter json`. Empty when the CLI failed before producing a summary (e.g. browser launch failure). |
| `evidence-dir` | Path on the runner to the per-run evidence directory (`.duhem/runs/<run-id>`). Empty when no summary was produced. |

## Exit code contract

The action propagates `duhem run`'s exit code: `0` on `Pass`,
non-zero on `Fail` or `Inconclusive`. That makes the step turn red on
a failing verdict, which is what required-check gating relies on.
Outputs are set in both cases, so downstream `if: failure()` steps
can still read `verdict` / `evidence-dir` to post comments or upload
evidence.

## Usage (from `onsager-ai/onsager`)

The intended consumer pattern. The Onsager workflow checks out
the PR, builds and serves Onsager locally, then calls the action
pinned to a Duhem release tag:

```yaml
# onsager-ai/onsager/.github/workflows/duhem-create-project.yml
name: duhem / create-project

on:
  pull_request:

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Build + serve Onsager
        run: |
          # Onsager-side build steps that bring up the dashboard on
          # http://localhost:3000 and provision a fixture user. The
          # specifics live in the Onsager-side companion spec.
          ./scripts/serve-for-duhem.sh &

      - name: Verify create-project
        id: duhem
        uses: onsager-ai/duhem/.github/actions/run@v0.1
        with:
          verification-path: verifications/onsager-dashboard-create-project/duhem.yml
          inputs: |
            login_url=http://localhost:3000/login
            new_project_url=http://localhost:3000/projects/new
            projects_url=http://localhost:3000/projects
            test_email=${{ secrets.DUHEM_FIXTURE_EMAIL }}
            test_password=${{ secrets.DUHEM_FIXTURE_PASSWORD }}
            project_name=duhem-fixture-${{ github.run_id }}

      - name: Surface evidence on red
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: duhem-evidence
          path: ${{ steps.duhem.outputs.evidence-dir }}
```

Onsager-side branch protection on `main` then adds the
`duhem / create-project` status check as required. An Onsager PR
cannot merge past a Duhem `fail`.

## Trust contract (§11.2)

Duhem's identity rests on the verifier being structurally
independent of the AI being verified. This action is the
deployment of that boundary at the CI seam.

What the action does:

- Resolves the Verification Definition from the `onsager-ai/duhem`
  checkout that GitHub fetches when the action is referenced as
  `onsager-ai/duhem/.github/actions/run@<tag>`. The caller's
  workflow steps never touch that checkout.
- Rejects `verification-path` values that are absolute or normalize
  (via `realpath -m`) to anything outside the Duhem checkout root.
  An attempt to set `verification-path: ../../my-attacker-vd.yml`
  fails before `duhem run` is invoked.
- Builds the `duhem` CLI from that same Duhem clone — pinned by tag.
- Runs `duhem run` with the caller-supplied `inputs:` block. Inputs
  flow through the typed input catalog (see
  `crates/duhem-schema/src/inputs.rs`), so a caller cannot smuggle
  options that change action behavior — only data values.

What the action does **not** do:

- It does not read the Verification Definition from the caller's
  workspace. A caller cannot land a PR that "fixes" a failing
  verdict by editing the VD inline; that edit lives in the caller's
  PR and is not visible to this action's CLI invocation.
- It does not accept arbitrary `duhem run` flags. The surface is
  the three inputs above plus the verdict-line contract. Adding a
  pass-through `extra-args` input here would let a caller pass
  `--filter` to skip a check that would otherwise fail — exactly
  the self-attestation we are designing out.
- It does not run the caller's `duhem` binary. The CLI built here
  is the one shipped with the Duhem tag the action is pinned at.

Two checks keep the boundary honest:

1. **Pinned ref.** Consumers reference the action at a tag
   (`@v0.1`), not at `@main`. A Duhem-side change cannot ship into
   the consumer's verdict pipeline without a coordinated tag bump.
2. **CODEOWNERS on `verifications/**`.** Edits to a VD on this
   repo require a Duhem maintainer's approval. A consumer PR that
   tries to open a Duhem PR re-authoring its own check still
   passes through review.

The combination — caller cannot pick the VD, caller cannot pick
the version, and editing the VD is gated by review on Duhem's
side — is the asymmetric-trust seam.

### Third-party dependencies (none, by policy)

The action references only `actions/checkout` (GitHub-owned) and
the runner's preinstalled `rustup`, `npm`, and `jq`. No
third-party actions (no `Swatinem/rust-cache`, no
`dtolnay/rust-toolchain`) — those would re-enter the trust
boundary at a floating tag every run, which is precisely the
supply-chain surface the asymmetric-trust seam is designed to
narrow. The cost is cold-start (full Cargo build every run); the
benefit is that every line of action runtime is auditable from
this directory plus the Duhem source it builds.

Playwright's npm CLI is invoked floating (`npx --yes playwright
install --with-deps chromium`). This is a deliberate seam, not an
oversight: the runtime driver is bundled into the Rust
`playwright = "0.0.20"` crate (driver `1.11.0-1620331022000`,
locked by `Cargo.lock`), so Cargo.lock already pins the bits that
touch the browser at runtime. The npm package is the *installer*
for the Chromium binary only. Pinning a specific
`playwright@x.y.z` here would risk downloading a Chromium
revision the bundled driver can't drive. The real supply-chain
hardening to do is bumping the Rust `playwright` crate to a
maintained version with a contemporary driver; that's tracked as
a follow-on and is out of scope for this action. Note also that
the same crate-version pairing constraint is the reason the
`dogfood.yml` `wiring` job only exercises the action through
negative path-validation tests today — a happy-path active smoke
test waits on the crate upgrade.

If we ever bring caching back, the rule is: pin every third-party
action by full commit SHA in `action.yml`, list it here, and
re-review on every bump.

## Local invocation (dogfood)

The Duhem repo's own [`.github/workflows/dogfood.yml`](../../workflows/dogfood.yml)
exercises this action via `uses: ./.github/actions/run` so changes
to the action and the VD are caught on every Duhem PR. That same
workflow doubles as the worked-example consumer for this surface.

## Why composite (vs. JS or Docker)

| dimension              | composite (chosen) | JS                  | Docker                |
|------------------------|--------------------|---------------------|-----------------------|
| cold-start cost        | building CLI from source dominates; ~minutes | fast (no Rust build needed) | image pull + run |
| reproducibility        | pinned by Cargo.lock + rust-toolchain.toml + action tag | requires bundling node_modules | strongest |
| auditability           | shell + cargo only; no opaque dependencies | implicit `node_modules` | Dockerfile review surface |
| trust-boundary clarity | every step visible in `action.yml` | bundled JS hides logic | Dockerfile is opaque to caller |

For Phase 0 the auditability and trust-boundary-clarity columns
dominate: the action's reviewability *is* the asymmetric-trust
mechanism. Once cold-start cost starts mattering at Phase 1 alpha
volumes, the move is to ship a `duhem` binary release and pull it
here, not to switch action runtimes.

## Versioning

Action tags track Duhem CLI tags one-to-one. `duhem/run@v0.1`
points at a Duhem repo state where the CLI is at `v0.1`. Pre-`v1`
the contract is unstable (`docs/duhem-spec.md` §11.3). On a
breaking change to the action's input/output shape, this README
gets a `## Breaking changes` section and the new tag bumps the
minor version.

## See also

- [`duhem-spec.md`](../../../docs/duhem-spec.md) §11.2 (trust boundary),
  §12.1 (GitHub integration), §14 Phase 1 milestones.
- [`onsager-dogfood` skill](../../../.claude/skills/onsager-dogfood/SKILL.md) —
  the cross-repo seam Duhem and Onsager share.
- [`verifications/onsager-dashboard-create-project/`](../../../verifications/onsager-dashboard-create-project) —
  the first Verification Definition this action surfaces.
