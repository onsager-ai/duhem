# `duhem/run` — composite GitHub Action

Runs a Duhem Verification Definition against the calling workflow's
environment and surfaces a verdict suitable for a required-check
status gate.

Spec: [`onsager-ai/duhem#36`](https://github.com/onsager-ai/duhem/issues/36).
Product context: `docs/duhem-spec.md` §12.1 (GitHub integration)
and §11.2 (trust boundary).

> **Phase 0 status.** This action is the wiring for the first
> deployment — the Onsager dashboard create-spec-plan Verification
> Definition. The action surface generalizes to additional VDs and
> additional consumers as Phase 1 alpha customers adopt it
> (`docs/duhem-spec.md` §14). Pre-`v1`, the action lives at
> `onsager-ai/duhem/.github/actions/run@v0.<n>` tags; consumers
> should pin to a tag, never `@main`.

## Inputs

| Name                  | Required | Default   | Description |
|-----------------------|----------|-----------|-------------|
| `verification-path`   | yes      | —         | Path to a `.yml` Verification Definition, resolved relative to the root selected by `verification-source`. Absolute paths and paths that normalize outside that root are rejected before invoking `duhem run`; see "Trust contract" below. |
| `verification-source` | no       | `duhem`   | Which root `verification-path` resolves against — the run's trust posture. `duhem`: the `onsager-ai/duhem` checkout at the pinned tag (the caller cannot substitute the VD — centralized seam). `workspace`: the caller's own checkout (`$GITHUB_WORKSPACE`), for a product self-gating on its co-located `.duhem/` VD (Mode A). See "Two trust postures" below. |
| `inputs`              | no       | `""`      | Newline-separated `key=value` pairs forwarded as repeated `--inputs key=value` flags to `duhem run`. Coerced per the Verification Definition's typed input catalog. |
| `reporter`            | no       | `json`    | Stdout reporter (`default` / `quiet` / `json`, plus plugin reporters from `.duhem.toml`). The action's `verdict` + `store` outputs depend on parsing the json summary, so leave at the default unless you have a plugin that emits the same single-line JSON contract. |

## Outputs

| Name           | Description |
|----------------|-------------|
| `verdict` | `pass` / `fail` / `inconclusive:<cause>` as emitted by `duhem run --reporter json`. Empty when the CLI failed before producing a summary (e.g. browser launch failure). |
| `store`   | Path on the runner to the evidence store (SQLite DB) the run was recorded into (`.duhem/duhem.db` inside the Duhem checkout). Empty when no summary was produced. |
| `run-id`  | The run's ULID inside the store. Empty when no summary was produced. |

## Exit code contract

The action propagates `duhem run`'s exit code: `0` on `Pass`,
non-zero on `Fail` or `Inconclusive`. That makes the step turn red on
a failing verdict, which is what required-check gating relies on.
Outputs are set in both cases, so downstream `if: failure()` steps
can still read `verdict` / `store` / `run-id` to post comments or
upload evidence.

## Usage (from `onsager-ai/onsager`)

The intended consumer pattern. The Onsager workflow checks out
the PR, builds and serves Onsager locally, then calls the action
pinned to a Duhem release tag:

```yaml
# onsager-ai/onsager/.github/workflows/duhem-create-spec-plan.yml
name: duhem / create-spec-plan

on:
  pull_request:

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Build + serve Onsager
        run: |
          # Onsager-side build steps that bring the `just dev` stack up
          # (dashboard on http://localhost:5173, portal on :3002) and
          # seed a workflow so the Create Plan compile gate has a valid
          # spec kind. The specifics live in the Onsager-side companion
          # work.
          ./scripts/serve-for-duhem.sh &

      - name: Verify create-spec-plan
        id: duhem
        uses: onsager-ai/duhem/.github/actions/run@v0.1
        with:
          verification-path: verifications/onsager-dashboard-create-spec-plan/duhem.yml
          inputs: |
            plan_id=duhem-fixture-${{ github.run_id }}

      - name: Surface evidence on red
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: duhem-evidence
          path: ${{ steps.duhem.outputs.store }}
```

Onsager-side branch protection on `main` then adds the
`duhem / create-spec-plan` status check as required. An Onsager PR
cannot merge past a Duhem `fail`.

## Two trust postures (`verification-source`)

The action runs a VD from one of two roots, and the choice *is* the
trust posture. Both build the `duhem` CLI from the pinned Duhem clone
and enforce the same containment gate (no absolute paths, no `..`
escapes); they differ only in which repo owns the VD.

| `verification-source` | VD resolved from | Who owns the VD | Use for |
|-----------------------|------------------|-----------------|---------|
| `duhem` (default)     | the `onsager-ai/duhem` checkout at the pinned tag | Duhem maintainers | Centralized VDs in `onsager-ai/duhem`. The caller cannot substitute the VD body — the asymmetric-trust seam (below). |
| `workspace`           | the caller's own checkout (`$GITHUB_WORKSPACE`) | the product | A product self-gating on its co-located `.duhem/` VD (**Mode A**, §10.1 Pattern D). |

**Why `workspace` is not a seam violation.** The §11.2 Alignment note
(Duhem-as-tool) decoupled trust from VD *location*: what makes a `pass`
trustworthy is mechanical judgment (no LLM in the judge) plus a
self-consistent Duhem contract — not where the VD file lives. In Mode A
the product legitimately owns its VD and gates its own PRs on it; the
lightweight guard against silent self-weakening is a CODEOWNERS stanza
on `/.duhem/` routing VD edits to a verifier reviewer, plus
hub-recorded verdicts (`duhem ship`) that a product PR can't rewrite.
A drop-in `.duhem/` skeleton + CODEOWNERS + Mode A CI snippet lives in
[`templates/product-repo/`](../../../templates/product-repo/).

Mode A adopters who want no GitHub Action at all can skip this action
entirely and run the CLI directly in their CI —
`duhem run .duhem/duhem.yml` resolves workspace-local paths natively.
The action's value over direct-CLI is the batteries (Node + Chromium
setup, verdict parsing, opt-in hub shipping), which matter most for
UI-heavy suites.

**Mode B** (Duhem's dogfood CI monitors drift by running a product's
co-located suite with a freshly-built `duhem`) does not use this action
at all — it invokes the CLI directly against a checked-out product ref.
See [`.github/workflows/drift-chreode.yml`](../../workflows/drift-chreode.yml).

## Trust contract (§11.2) — the default `duhem` seam

Duhem's identity rests on the verifier being structurally
independent of the AI being verified. In the default
`verification-source: duhem` posture, this action is the
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

What the action does **not** do (in the default `duhem` posture):

- It does not read the Verification Definition from the caller's
  workspace. A caller cannot land a PR that "fixes" a failing
  verdict by editing the VD inline; that edit lives in the caller's
  PR and is not visible to this action's CLI invocation. (This is
  exactly what `verification-source: workspace` opts out of, for the
  Mode A self-gating case above — where the product is *meant* to own
  its VD.)
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
- [`verifications/onsager-dashboard-create-spec-plan/`](../../../verifications/onsager-dashboard-create-spec-plan) —
  the first Verification Definition this action surfaces.
