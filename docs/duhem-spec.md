# Duhem — Product Specification

> **Status**: Draft v0.3
> **Last updated**: 2026-05-07

-----

## 1. Why Duhem

Pierre Duhem argued that no scientific hypothesis can be tested in isolation. When an experiment fails, you cannot determine from the failure alone whether the hypothesis was wrong or one of the unstated auxiliary assumptions was. The entire web of theory, apparatus, and assumption is what gets tested — never a single proposition by itself. W.V.O. Quine generalized this: any belief can be held true if we are willing to make adjustments elsewhere in the web.

AI product delivery is the engineering instance of the Duhem-Quine thesis.

When an AI agent ships a feature, what gets delivered is not just code. It is code × prompt × tool configuration × data state × runtime environment × upstream service contracts. When something behaves wrongly in production, no single element is the unique cause; the web as a whole produced the behavior.

This means **verification of AI delivery cannot be reduced to component testing**. It must be holistic — exercising the actual behavior the user will experience, against the commitments the team has made, in an environment that includes all the auxiliary elements the system actually depends on.

We named the platform Duhem because that thesis is the foundation of what we do.

## 2. Vision

Software is delivered to people who must trust the claim that it works. The guiding principle behind everything we make, therefore, is and must remain, **the truth-of-claim**.

When AI says “done,” users live with the result. Trust is precious — once broken, hard to rebuild. Every AI delivery shipped without true verification is trust being spent. When trust runs out, the AI-driven development paradigm collapses.

Duhem exists to honor truth so users can trust AI delivery.

## 3. Problem

AI coding agents have made shipping faster, but verification has not kept up — and the existing verification stack was never built for AI-paced delivery in the first place.

The dominant pattern today: AI generates code, claims “done,” opens a PR. Human reviews the code (not the behavior), runs unit tests (not end-to-end), merges, ships. Days later, a user reports the feature does not actually work. The team realizes no one verified the behavior against the intent before shipping.

Three structural failures are at play:

**Component testing assumes isolatable units.** Unit tests pass, integration tests pass, the feature is broken anyway. The AI-delivered web is not decomposable into independent units that, individually verified, imply system correctness. Auxiliary assumptions about prompts, context, tool wiring, and runtime state are exactly what unit tests don’t cover.

**AI claims completion based on syntactic plausibility, not behavioral correctness.** “Done” means “code looks right,” not “system does the thing.” The AI cannot tell you whether the prompt template it relied on is misaligned with the data shape it will encounter. Only the running web can.

**Trust in AI delivery cannot be built by the AI itself.** The verifier of AI claims must be structurally independent of the AI doing the claiming. Otherwise the verdict is circular.

The cost is real: users lose trust, engineers spend hours manually verifying AI work, AI-driven workflows stall at the merge gate, and the promise of AI-accelerated delivery is undermined by the absence of a matching verification capability.

## 4. Solution Overview

Duhem is a **holistic verification platform** that sits between AI coding agents and production. For every feature an AI delivers, Duhem:

1. Captures the human’s intent as **acceptance criteria** — natural-language commitments about what “done” means.
1. Translates criteria into **executable checks** — structured verification units written in YAML, exercising the full delivery web.
1. Executes checks against a real environment, observing the system’s actual behavior end-to-end.
1. Produces a **mechanical verdict** (pass / fail / inconclusive) backed by **evidence** that humans can audit.
1. Gates merge/deploy on the verdict — AI cannot self-attest its way past verification.

Duhem is opinionated about three things:

**Verification is holistic.** A check exercises code + prompts + tool wiring + data + runtime together. We do not pretend the web decomposes into independent units. Failure attribution is post-hoc — the verdict is on the whole, and evidence supports human investigation.

**Verification judgment is mechanical.** AI may help author criteria and checks; humans review them; but the final judgment is produced by deterministic execution of structured assertions, not by an AI grading itself.

**Criteria are stable; checks are derivative.** The human commitment about what “done” means is the durable artifact. Checks are how we verify that commitment. When implementation drifts, criteria do not. When criteria change, that’s a real change to the contract.

## 5. Positioning

### What Duhem is

- **The verification layer your AI coding agent doesn’t have.**
- Holistic, spec-driven, mechanical-judgment, evidence-grounded verification of AI delivery.
- Closed source today, with a free CLI and free hosted tier. Schema and reference judge open-sourced once they stabilize (see Section 11.3).

### What Duhem is not

- **Not a testing tool.** Testing tools assume humans write tests against units. Duhem assumes AI authors checks under human review against whole-system behavior.
- **Not a code review tool.** Code review evaluates source. Duhem evaluates behavior of the delivery web.
- **Not a QA replacement service.** Services like QA Wolf provide test-writing labor. Duhem provides verification infrastructure.
- **Not an AI judge.** Duhem never asks an LLM “did this pass?” The verdict is mechanical.
- **Not a unit test framework.** Unit tests verify decomposed components. Duhem verifies the web that the AI actually delivered.

### How Duhem differs from adjacent products

|Product             |Their angle                          |Duhem’s angle                                                    |
|--------------------|-------------------------------------|-----------------------------------------------------------------|
|Qodo                |AI-assisted code review              |Spec-driven holistic behavioral verification                     |
|Antithesis          |Deterministic simulation finding bugs|Spec-driven assertion of intended behavior across the web        |
|QA Wolf             |Managed test-writing service         |Self-serve verification platform                                 |
|Octomind            |AI generates E2E tests               |AI generates checks; humans own criteria; checks exercise the web|
|Mabl / TestRigor    |UI test automation                   |Full-stack verification (UI + API + events + DB)                 |
|GitHub Actions      |CI orchestration                     |Verification semantics on top of CI primitives                   |
|Unit test frameworks|Component-level isolation            |Holistic web exercise                                            |

## 6. Ideal Customer Profile

**Initial wedge**: SaaS web applications with both backend and frontend, where engineering teams use AI coding agents heavily.

**Indicators of fit**:

- Team uses Cursor, Claude Code, Devin, or similar AI coding agents for ≥30% of development.
- Team ships to production at least weekly.
- Stack includes a web frontend (React/Vue/Svelte) and an HTTP backend.
- At least one painful incident in the last six months where an AI-delivered feature reached production broken.
- Engineering leadership has explicitly raised “AI delivery quality” as a concern.
- The team has experienced the “all unit tests pass, feature is broken anyway” pattern with AI-delivered work.

**Indicators of poor fit (initial)**:

- Pure-backend services with no UI surface (Duhem will support these eventually, but UI is part of the wedge).
- Heavily regulated industries requiring formal-verification-grade rigor (Duhem is holistic behavioral verification, not formal methods).
- Teams not yet using AI coding agents (Duhem’s value proposition is conditional on AI in the loop).

## 7. Core Concepts

### 7.1 Verification

A complete verification cycle for an artifact (a feature, a PR, a deploy candidate). One verification consists of one or more criteria, each backed by one or more checks. The verification exercises the artifact in a real environment containing the actual auxiliary elements the system depends on.

### 7.2 Criterion

A human-authored, free-form, natural-language statement of what “done” means for a feature.

Criteria express intent, not procedure. They are the **commitment** the team makes about the feature. They are stable across implementation changes — when implementation drifts, criteria do not.

Authoring rule: a criterion should be a single coherent commitment, in 1–3 sentences, that a non-technical stakeholder can read and validate.

Example criterion (for a “create workspace” feature):

> A user can create a workspace from the dashboard. The new workspace becomes immediately visible in their workspace list, and the user is navigated to the workspace’s home page. No errors are shown.

A feature typically has 2–6 criteria.

### 7.3 Check

A structured, executable verification unit derived from a criterion. Multiple checks may collectively verify a single criterion.

A check is a sequence of `steps` (actions to perform on the system) followed by `assertions` (mechanical predicates over what the steps observed).

Each check produces one verdict: `pass`, `fail`, or `inconclusive`.

Authoring rule: checks are authored by AI from criteria, then reviewed once by a human. After review, checks are frozen — they are not re-authored on every run.

Crucially: a single check may exercise multiple components together (UI click → API call observation → DB query → event arrival). This is intentional. The check is verifying a slice of the delivery web, not a single component.

### 7.4 Assertion

A mechanical predicate evaluated against observable system state. The atomic unit of judgment.

Assertions are pure boolean predicates over named values produced by steps. They never invoke an LLM. They are evaluated by Duhem’s deterministic judge.

### 7.5 Step

A single action Duhem performs on the system under test, or an observation it captures. Steps are typed by the action they invoke (`uses:` field), e.g., `ui/click`, `api/observe`, `db/query`, `event/wait`.

Steps may produce named outputs that subsequent steps and assertions reference.

### 7.6 Verdict

The aggregated outcome of a verification run.

- **Per-check verdict**: produced by deterministic evaluation of the check’s assertions against observed state.
- **Per-criterion verdict**: aggregated from its checks (any check `fail` → criterion `fail`; any `inconclusive` and no `fail` → criterion `inconclusive`; all `pass` → criterion `pass`).
- **Per-run verdict**: aggregated from all criteria, same rules.

The three-state model is non-negotiable. `inconclusive` exists because real systems have flakes, environment failures, and observability gaps. Forcing flaky outcomes into `pass` corrupts trust; forcing them into `fail` blocks delivery without cause. `inconclusive` triggers escalation to human review without false signals.

A `fail` verdict on a holistic check tells you the web is broken. It does not tell you which strand is wrong. That attribution is the human’s job, supported by evidence — exactly as Duhem-Quine predicts.

### 7.7 Evidence

The structured trace produced during a verification run. Append-only. Never edited.

Evidence includes: each step’s inputs and outputs, each assertion’s predicate and evaluated result, screenshots/DOM snapshots/HAR files where applicable, video recordings (UI runs), and timing data.

Evidence is the artifact humans audit when a verdict needs to be questioned. Duhem’s commitment: every verdict can be traced back to evidence sufficient for a human to reason about which part of the web caused the failure.

### 7.8 Artifact

The thing being verified. Typically a Git ref (commit, PR, deploy candidate). The verification run is parameterized over an artifact and an environment.

### 7.9 Run

One execution of a verification against an artifact. Runs are idempotent given a frozen check spec and a stable environment, but Duhem records run history because environments do drift — and environment drift is part of the web Duhem-Quine talks about.

## 8. The Holistic Verification Principle

This section explains a foundational design choice that follows from Duhem-Quine and that distinguishes Duhem from component-testing tools.

### What “holistic” means in practice

A Duhem check, by default, exercises the actual user-visible behavior end-to-end. It does not stub the database. It does not mock the LLM. It does not bypass the auth layer. It does not patch the event bus. It runs against an environment that contains the real (or production-equivalent) versions of all the things the feature depends on.

This is a deliberate inversion of the unit-test pyramid for AI-delivered features. The case for unit-testing what the AI generated is weaker than for human-written code, because:

- The cost of writing the unit test is no longer human-borne (AI writes them too), so the discipline that made unit tests cheap-relative-to-integration-tests does not hold.
- The failure modes of AI-generated code concentrate in interface mismatch, prompt drift, and tool-wiring errors — exactly the failure modes that unit tests mask.
- The auxiliary assumptions (prompts, configs, tool definitions) are not test-coverable in the unit-test framing.

So Duhem’s checks are integration-or-larger by design. We don’t oppose unit tests; teams may keep them. But unit tests are not what Duhem provides, and unit-test pass is not evidence Duhem accepts.

### What this implies for environments

Verification environments must be production-equivalent. Duhem provides primitives for spinning up environments that include the real database engine (not in-memory mock), the real message bus (not stubs), the real upstream services (or production-faithful contract tests of them), and the real authentication layer.

This makes verification expensive relative to unit tests. That cost is acknowledged. It is the unavoidable price of holistic verification — and it is much cheaper than a production incident.

### What this implies for failure attribution

When a Duhem check fails, the verdict tells you the web is broken. Pinpointing which strand is wrong is human-led, supported by evidence. Duhem will not pretend to localize failures with confidence Duhem-Quine says cannot exist.

This is honesty about what verification can and cannot do. Localizing a holistic failure to a specific cause requires further investigation — log inspection, hypothesis testing, sometimes reproducing in a debugger. Duhem provides the evidence trail that makes that investigation tractable; it does not produce false certainty about cause.

## 9. Workflow Stages

A verification cycle proceeds through five stages. Each stage has a defined automation level.

### Stage 1 — Define Criteria

**Automation**: Semi-automatic
**Trust property**: Human authoritative

The team authors criteria in natural language, optionally with AI assistance. AI may suggest criteria from feature spec, PR description, or design doc; humans accept, reject, or rewrite. A human review and explicit acceptance is required before criteria become the commitment for verification.

Output: a criteria file (`<feature>.criteria.md`) or criteria inline in a verification YAML file.

### Stage 2 — Author Checks

**Automation**: Semi-automatic
**Trust property**: Human review of mechanical translation

AI translates each criterion into one or more checks, generating the structured YAML. The translation is mechanical (deterministic predicates, named action types) so a human can read and verify the translation captures the criterion’s intent.

Authored checks are frozen after human review. They become the spec against which verification will run.

Output: YAML files containing verification definitions, organized by team convention. Filenames are free (`create-workspace.yml`, `verification.yml`, `tests.yml` — any). A root `duhem.yml` manifest at the project root may aggregate verification files and provide shared defaults.

### Stage 3 — Provision Environment

**Automation**: Semi-automatic
**Trust property**: AI provisions, human fallback

Duhem provisions the holistic environment needed for verification: deploy the artifact, ensure real database/messagebus/upstream services are present, seed the database, configure feature flags, set up authentication. AI agents may automate this with appropriate permissions; for cases where automation fails, Duhem exposes the same operations for human use.

Output: a verification environment that includes the actual web the artifact depends on.

### Stage 4 — Execute Checks

**Automation**: Fully automatic
**Trust property**: Mechanical

Duhem executes each check against the holistic environment. Steps perform actions and capture observations across the web. Assertions are evaluated as pure predicates. Each check produces a verdict and evidence.

No AI is in the execution loop. No AI evaluates whether a check passed.

Output: per-check verdicts, per-criterion aggregations, run-level verdict, evidence trace.

### Stage 5 — Deliver Verdict

**Automation**: Semi-automatic
**Trust property**: State-machine enforced

The verdict is published to the configured delivery surface (PR check, Slack, dashboard). Merge/deploy gating is enforced by the state machine — a `fail` verdict blocks; `inconclusive` triggers configurable escalation (block by default, can be relaxed); `pass` allows progression.

Override of `fail` to allow merge requires explicit human action with an audit trail. AI agents cannot self-grant override.

Output: a delivered verdict, an enforced gate, an audit record.

## 10. The Verification Definition

A **Verification Definition** is the structured YAML artifact that contains criteria and their checks for a given feature. It is the primary structured input Duhem consumes.

The Verification Definition is a *format*, not a filename. Files containing a Verification Definition can be named any way the team prefers (`create-workspace.yml`, `tests.yml`, `verification.yml`, `acceptance-checks.yaml` — all valid). Duhem identifies Verification Definitions by content, not extension.

The Duhem schema underlying Verification Definitions is YAML, inspired by GitHub Actions’ surface form (jobs/steps/uses/with) and borrowing Arazzo’s declarative outputs and runtime expressions, but bound to neither standard.

### 10.1 File organization

Duhem supports three file organization patterns. Teams choose what fits.

#### Pattern A: Single file, direct execution

Suitable for small projects or quick verification. One YAML file contains the full Verification Definition; the user invokes Duhem against it directly.

```bash
duhem run create-workspace.yml
```

No root manifest needed. The file’s content self-identifies as a Verification Definition (by presence of top-level `verification:` and/or `criteria:` fields).

#### Pattern B: Co-located per-feature, with root manifest

Suitable for SDD repositories where each feature is a directory containing its own spec, code, and verification.

```
features/
  create-workspace/
    spec.md                # SDD feature spec (free-form markdown)
    verification.yml       # Verification Definition for this feature
    src/                   # implementation
  login/
    spec.md
    verification.yml
    src/
duhem.yml                  # root manifest, aggregates all verifications
```

The root `duhem.yml` manifest declares which files to aggregate and provides shared defaults. Tool execution:

```bash
duhem run                     # runs all verifications declared in duhem.yml
duhem run --filter login::*   # runs a subset
```

`--filter` takes a three-axis selector `[<verification>::]<criterion>[::<check>]` (glob-aware) — see §10.4. A bare `login` is read as a *criterion* glob, not a feature name; to scope to the `login` verification use `login::*`.

#### Pattern C: Centralized verification directory

Suitable for legacy structures where features are not first-class directories.

```
verifications/
  create-workspace.yml
  login.yml
  billing/
    subscription.yml
    invoice.yml
duhem.yml                  # root manifest
```

Same root manifest pattern as B, just different file layout.

### 10.2 Self-identification

Any `.yml` or `.yaml` file containing a top-level `verification:` field (or `criteria:` field) is recognized as a Verification Definition. Files lacking these fields are ignored even if they happen to be in a glob pattern’s path.

This means Duhem coexists with other YAML in the repo (CI configs, Helm charts, etc.) without filename conflicts.

### 10.3 Verification Definition structure

```yaml
# Any filename. Self-identifies by top-level fields.
verification: Create workspace E2E
spec_ref: docs/features/create-workspace.md  # link to feature spec (optional)

inputs:
  workspace_name:
    type: string
    default: "test-ws-{{uuid}}"

setup:                                  # runs once before all criteria
  - uses: ui/navigate
    with: {url: "/dashboard"}
  - uses: ui/assert-state
    with: {role: "page", state: "authenticated"}

criteria:
  - id: AC-1
    description: |
      A user can create a workspace from the dashboard. The new workspace
      becomes immediately visible in their workspace list, and the user
      is navigated to the workspace's home page. No errors are shown.
    checks:
      - id: AC-1.1
        description: API responds correctly to creation request
        steps:
          - uses: ui/click
            with: {role: "button", name: "Create Workspace"}
          - uses: ui/type
            with:
              role: "textbox"
              label: "Workspace Name"
              value: $inputs.workspace_name
          - uses: ui/click
            with: {role: "button", name: "Create"}
          - uses: api/observe
            id: api_call
            with:
              method: POST
              path: /workspaces
              within: 3s
            outputs:
              status: response.status
              workspace_id: response.body.id
        assertions:
          - $steps.api_call.outputs.status == 200
          - type_check:
              value: $steps.api_call.outputs.workspace_id
              is: uuid

      - id: AC-1.2
        description: Workspace appears in user's workspace list
        steps:
          - uses: ui/assert-element
            id: list_check
            with:
              locator:
                role: listitem
                text: $inputs.workspace_name
              scope: {role: "list", name: "Workspaces"}
              expected: exists
              within: 5s
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.list_check.outputs.satisfied == true

      - id: AC-1.3
        description: User is navigated to workspace home
        steps:
          - uses: ui/assert-url
            id: nav_check
            with:
              matches: "/workspaces/{{$steps.api_call.outputs.workspace_id}}"
              within: 3s
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.nav_check.outputs.satisfied == true

      - id: AC-1.4
        description: No error UI is shown
        steps:
          - uses: ui/assert-element
            id: error_check
            with:
              locator: {role: "alert"}
              expected: not_exists
            outputs:
              satisfied: satisfied
        assertions:
          - $steps.error_check.outputs.satisfied == true
```

Notice that AC-1.1 alone exercises five different layers: UI input capture, UI button activation, network observation, API response shape, and ID semantics. That is intentional. The check verifies a slice of the holistic web.

### 10.4 Root manifest (`duhem.yml`)

The root manifest is a single canonical file at the project root that aggregates Verification Definitions and provides shared configuration.

```yaml
# duhem.yml at project root
version: "1"

defaults:
  environment: staging        # default environment for runs
  timeout: 30s                # default per-step timeout
  inconclusive_policy: block  # block | warn | pass
  retry:
    max: 1
    backoff: exponential

verifications:
  - features/create-workspace/verification.yml
  - features/login/verification.yml
  - features/billing/*.yml          # glob supported
  - "verifications/**/*.yml"        # recursive glob

includes:                           # composition: shared config from other files
  - .duhem.shared.yml               # team-shared defaults
  - .duhem.local.yml                # gitignored, per-developer overrides

environments:                       # named environment configs
  staging:
    base_url: https://staging.example.com
    db_url: postgres://staging-db
  prod:
    base_url: https://example.com
    db_url: postgres://prod-db
```

The root manifest is canonical: Duhem auto-discovers `duhem.yml` (or `.duhem.yml`) at the project root or its ancestors. Users can override with `duhem run -f path/to/manifest.yml`.

If no root manifest is present, Duhem still works on individual Verification Definition files passed directly.

### 10.5 Action types

Verification Definitions invoke pre-defined action types via `uses:`. Each action defines a typed `with:` schema (its internal `With` struct) and named outputs; the dispatch boundary itself is untyped YAML that the action downcasts inside `invoke`.

**UI actions** (Playwright primitives)

- `ui/navigate` — go to a URL
- `ui/click` — click an element (role-based locators preferred)
- `ui/type` — type into an input
- `ui/select` — select an option
- `ui/assert-element` — observe whether an element exists/is visible/has text
- `ui/assert-url` — observe URL state
- `ui/assert-state` — observe page-level state (authenticated, loaded, etc.)

**API actions**

- `api/call` — make an HTTP request actively
- `api/observe` — passively observe an HTTP request the UI triggers (network sniffing)

**Database actions**

- `db/query` — execute a read query, capture rows
- `db/seed` — seed data for setup

**Event actions**

- `event/wait` — wait for an event on a topic, capture payload
- `event/publish` — publish an event for setup

**Generic**

- `wait` — wait for a duration
- `assert` — top-level assertion (when not tied to a specific step’s observable)

### 10.6 Assertion forms

Assertions evaluate to `true`, `false`, or `inconclusive` (e.g., when timeouts hit or referenced state is missing).

- **Boolean expression**: `$steps.X.outputs.Y == 200`
- **Type check**: `type_check: {value: ..., is: uuid|email|datetime|...}`
- **Pattern match**: `matches: {value: ..., pattern: ...}`
- **Set membership**: `in: {value: ..., set: [...]}`
- **Existence**: `exists: $steps.X.outputs.Y`
- **Cross-step consistency**: `equal: [$steps.A.outputs.X, $steps.B.outputs.X]`

An `inconclusive` result always carries a cause, distinguishing "the check could not be evaluated" from a genuine `false`. The verdict-level catalog is closed at v1 and surfaces as `inconclusive:<cause>` (snake-case wire tokens): `timeout`, `missing_observation`, `environment_error`, `empty_aggregation`. The runtime evaluator tracks finer internal causes (missing input, missing env, type mismatch, invalid pattern, unknown runtime helper) that collapse into these when the judge aggregates the verdict (§7.6).

### 10.7 Runtime expressions

Borrowed from Arazzo. References available in expressions:

- `$inputs.<name>` — inputs to the verification run
- `$steps.<id>.outputs.<name>` — outputs from a prior step
- `$env.<name>` — environment variables (whitelisted)
- `$runtime.uuid()` / `$runtime.now()` — common helpers
- `$runtime.format(fmt, args...)` — **pure** string composition: the
  `{}` placeholders in `fmt` are filled, in order, by the remaining
  scalar args (coerced to their string form). The sanctioned way to
  compose a value — e.g. a dynamic URL `$runtime.format("{}/{}",
  $inputs.base, $steps.create.outputs.body.data._id)` — without
  scripting. Deterministic and reproducible: it is a pure function of
  its arguments (no I/O, clock, or randomness), so it preserves the
  mechanical-judgment and reproducible-run commitments (§11.2). Helpers
  may compute and compose values; they never *are* the verdict, which
  remains the closed assertion set of §10.6.

### 10.8 Extensibility

User-defined action types are a **Phase 2+ goal** (§14). The v0.1 action catalog is closed (`crates/duhem-actions`) and the dispatch registry is internal — external crates cannot register a new `uses:` today. The target design for a custom action type is a published unit with:

- Name (`<scope>/<name>`, e.g., `acme/stripe-charge-observe`)
- Input schema (JSON Schema for `with:` keys)
- Output schema (named outputs the action produces)
- Implementation (how Duhem invokes it)

Custom actions are intended to follow a marketplace mental model similar to GitHub Actions’ `actions/*`. This anchors the design; none of it is authorable in Phase 1.

## 11. Architecture

### 11.1 Components

**Authoring Surface**

- CLI for local authoring (`duhem init`, `duhem validate`, `duhem run`)
- VS Code extension for inline criterion editing and check preview
- Web UI for browsing past runs, evidence, and verdicts

**Generation Service**

- AI-powered translation of criteria into checks
- Uses Duhem’s structured action-type catalog as constraints
- Outputs YAML, never invoked at run time

**Runtime**

- Executes checks against an environment
- Produces evidence
- Stateless except for run records

**Judge**

- Pure deterministic evaluator of assertions over observed state
- No LLM in the loop
- Open source, auditable

**Evidence Store**

- Append-only structured log + binary blobs (screenshots, videos, HAR)
- Queryable via dashboard

**Delivery Layer**

- GitHub Action / GitLab CI integration
- Slack / Linear / email notifications
- Webhooks for custom integrations

### 11.2 Trust Boundary

The critical architectural commitment: **the judge is structurally independent of any AI**. The judge’s input is observed state and frozen check spec; its output is a verdict produced by deterministic evaluation. AI may participate in stages 1, 2, 3 (authoring, translation, environment provisioning) but never in stage 4 (execution and judgment).

This is enforced architecturally — the judge service has no LLM dependency. It can run fully air-gapped from AI infrastructure.

Because Duhem is closed-source in Phase 1, additional measures are taken to make this commitment auditable:

- **Documented judge logic**: The decision rules used by the judge are documented even though the implementation source is not — today they live in §10.6 (assertion forms), §10.7 (runtime expressions), and the `crates/duhem-judge` aggregation doc comments. Customers can reason about how a verdict is computed without reading the production source. (Extracting these into a single standalone "judge decision reference" is a Phase 2+ goal.)
- **Reproducible runs**: Every run produces a complete evidence trace. Given identical environment state and a frozen check spec, replays must produce identical verdicts. Determinism is verifiable empirically by customers.
- **Self-hosted judge for enterprise** (Phase 2+): For customers requiring stronger guarantees, an enterprise license will include a self-hostable judge binary. The binary is closed-source but runs entirely within customer infrastructure, removing the cloud-trust dependency. No standalone judge binary or enterprise tier ships in Phase 1 — `duhem-judge` is a library crate today.
- **Future-state OSS judge**: When schema stabilizes (Phase 2), a reference judge implementation is open-sourced. This serves as the public standard against which the production judge is compliance-tested.

### 11.3 Source posture and opening strategy

Duhem follows a **closed-source-first, phased-opening** strategy. The phases reflect product maturity, not ideological position.

#### Phase 1 — Closed (months 0–12)

Everything is closed source. This includes the runtime, judge, generation service, dashboard, evidence storage, CLI, and schema definition. The CLI binary is freely distributed; nothing is self-hostable except for paid enterprise customers.

Rationale:

- The schema is in active iteration. Premature OSS would lock in design decisions that are not yet correct.
- There is no external community at the scale that benefits from OSS yet.
- Solo founder bandwidth cannot absorb community management overhead.
- Closed source preserves all options for commercialization while we discover product-market fit.

Public surfaces during Phase 1:

- Schema specification published as **documentation**, not as a formally licensed standard. Free to read and reference; not formally OSS.
- Action type catalog documented in reference docs.
- CLI binary distributed freely (closed source, free to use).
- Examples and tutorials published openly.

#### Phase 2 — Schema OSS (months 12–24)

When the schema has stabilized — measured by breaking-change rate dropping below threshold, and at least one full enterprise customer in production — the schema specification is moved under a permissive open-source license (Apache 2.0 or MIT). A reference judge implementation is released alongside.

Rationale:

- A stable schema is more valuable as an open standard than as proprietary IP. It encourages ecosystem tooling, third-party action types, and external auditing.
- A reference judge codifies the trust commitment and lets enterprise customers verify production judge compliance.
- Schema-as-spec mirrors successful precedents: OpenAPI, GraphQL, JSON Schema.

What stays closed in Phase 2:

- Production runtime (optimized for hosted service)
- Production judge (optimized; reference judge is the public spec)
- Generation service (proprietary AI-powered authoring)
- Dashboard, evidence storage, hosted infrastructure
- Enterprise features (SSO, audit, compliance)

#### Phase 3 — Selective deeper opening (months 24+)

Once commercial traction is established and core revenue does not depend on closed CLI source:

- CLI source may be open-sourced (it has minimal commercial value)
- Action type SDK opened to allow community-contributed actions
- Selected non-critical action types open-sourced for transparency

Core commercial value (production runtime, generation service, hosted infrastructure) remains closed indefinitely. This is the open-core model, with the open-core boundary drawn around the schema and reference implementation rather than the runtime.

#### What is never open-sourced

The hosted service infrastructure, the production-grade generation models and prompts, customer evidence data, and proprietary action types remain closed indefinitely. These are the surfaces where Duhem captures commercial value.

## 12. Integration Surface

### 12.1 GitHub

A GitHub Action: `duhem/run@v1`. Runs verification on a PR; reports status checks. Supports required-checks gating natively.

### 12.2 Other CI

The CLI is the universal integration point. Any CI system that can run a binary can run Duhem.

### 12.3 IDE

VS Code extension provides inline preview of checks generated from criteria, run-from-IDE, and verdict surfacing.

### 12.4 AI Coding Agents

Duhem exposes an MCP server. AI coding agents (Claude Code, Cursor, etc.) can:

- Read the criteria for a feature being implemented
- Run verification on their work in progress
- Read verdicts and adjust implementation
- **Cannot** override verdicts or self-attest pass.

## 13. Pricing Model (Initial)

Pricing reflects the closed-source-first posture. There is no self-hosted OSS option in Phase 1; instead, a generous hosted free tier provides the same low-friction adoption path that OSS would have given.

**Hosted Cloud**: per-seat-per-month, anchored at $30 (validated by Qodo $30, CodeRabbit $24).

**Tiers**:

- **Free**: 1 user, 100 verifications/month, public repos. Goal: zero-friction adoption for individual developers and OSS projects. Free tier is permanent, not a trial.
- **Team**: $30/seat/month, unlimited verifications, private repos, generation service, evidence storage 90 days.
- **Enterprise**: custom pricing. SSO, longer retention, audit features, and self-hostable judge binary (closed-source enterprise license) for customers requiring infrastructure isolation.

The CLI binary is free to download and use across all tiers. It is not formally open-source but is freely distributable and does not require a license key for offline use against a customer-controlled judge endpoint.

## 14. Roadmap

The roadmap reflects a solo founder building with Claude Code as the primary development assistant, with Onsager as the first dogfooding customer. It is not a typical funded startup roadmap; it is intentionally calibrated to a single-builder reality.

### Phase 0 — Foundation (months 0–3)

**Goal**: stand up minimum viable Duhem; first real verification running against Onsager.

- Solo build the schema, CLI, runtime, and judge using Claude Code as primary development assistant. ✅ shipped (Cargo workspace landed via #7; nine crates `duhem-cli`, `duhem-runtime`, `duhem-judge`, `duhem-schema`, `duhem-actions`, `duhem-evidence`, `duhem-summary`, `duhem-reporter-pretty`, `duhem-reporter-junit`; CLI verbs `init` / `run` / `validate` / `--version` via #13, #16, #17, #18, #19, #24, #40, #42, #43, #58, #60).
- Schema spec v0.1 → v0.5 (rapid iteration; expected breaking changes). ⏳ in progress (current: v0.1.0 via `duhem_schema::SCHEMA_VERSION`; per-landing ledger in `CHANGELOG.md`; v0.5 lockdown criteria tracked under #51 / shipped via #59).
- Built-in action library: minimum useful subset (UI click/type/assert, API call/observe, basic assertions). ✅ shipped (`ui/*` slice via #14, #41; `api/observe` via #44; `api/call` v1 via #25; full catalog under `duhem-actions`).
- First Onsager feature verified using Duhem (manually-authored checks, no AI generation yet). ✅ shipped (#46, #47; refreshed to the spec-plan flow via #79; in-tree at [`verifications/onsager-dashboard-create-spec-plan/`](../verifications/onsager-dashboard-create-spec-plan/); environment `up:` / `down:` hooks via #61).
- Parallel: 5–10 customer interviews with AI-coding-agent power users (Cursor, Claude Code, Devin) to validate market hypothesis beyond Onsager. ⏳ outstanding.

**Onsager dependency**: Onsager has at least one feature in active development to verify against. If Onsager is not at this stage yet, Phase 0 starts with a smaller toy app instead.

### Phase 1 — MVP (months 3–6)

**Goal**: Duhem reliably verifies Onsager’s outputs; first external alpha users.

- Schema v1.0 (still subject to change, but with deprecation policy).
- Generation service alpha: AI translates criteria → checks for simple cases.
- GitHub Action integration: Duhem runs on Onsager’s PRs.
- Web dashboard for runs and evidence (minimum viable).
- 3–5 friendly external alpha users from the customer-interview cohort.

**Onsager dependency**: Onsager has CI; Onsager-Duhem integration is daily-use during this phase.

### Phase 2 — Public alpha and schema OSS (months 6–12)

**Goal**: schema stabilizes; broader external adoption begins.

- Schema breaking-change rate drops; lock down v1.0 contract.
- VS Code extension.
- MCP server for AI coding agents.
- Public alpha: open signup, generous free tier.
- Schema specification opened under permissive OSS license. Reference judge implementation released.
- Hosted commercial tier launches (Team plan).

**Onsager dependency**: Duhem is mature enough that Onsager development would be slower without it.

### Phase 3 — Expansion (months 12–24)

**Goal**: enterprise customers; production-grade reliability.

- Custom action type SDK (community can write actions).
- Action marketplace (alpha).
- L3 visual baseline assertions.
- Multi-environment management.
- Enterprise features (SSO, audit, self-hostable judge under enterprise license).
- Adapter to Arazzo (export verifications as Arazzo workflows for interop).

**Onsager dependency**: minimal at this point; Duhem’s growth becomes independent of Onsager’s pace.

### Phase 4 — AI Factory layer (months 24+)

**Goal**: when Onsager’s AI factory paradigm matures and begins producing other products at scale, Duhem becomes the verification infrastructure for that production line.

- Duhem-Onsager integration deepens: Onsager-generated artifacts come with auto-generated criteria; Duhem verification is the merge gate for Onsager’s factory output.
- The pattern Onsager demonstrates (AI factory + Duhem verification gate) becomes proof of a broader paradigm sellable to other organizations adopting AI-native development.

This phase is contingent on Onsager’s own roadmap. It is not blocking for Duhem’s market viability.

### Onsager-Duhem milestones (interleaved)

|Window     |Duhem milestone                                  |Onsager dependency                                                   |
|-----------|-------------------------------------------------|---------------------------------------------------------------------|
|Weeks 1–4  |Schema v0.1, CLI scaffold, basic runtime ✅ shipped (#7, #13, #17, #19) |None                                                                 |
|Weeks 5–8  |Judge implementation, 5–10 action types ✅ shipped (#14, #16, #25, #41, #44) |Onsager has 1 active feature                                         |
|Weeks 9–12 |First Onsager check shipped (manual authoring) ✅ shipped (#46, #47; refreshed to the spec-plan flow #79) — [`verifications/onsager-dashboard-create-spec-plan/`](../verifications/onsager-dashboard-create-spec-plan/) |Onsager begins using Duhem on selected PRs                           |
|Weeks 13–20|Generation service alpha; expanded action library|Onsager’s PRs require Duhem verdict                                  |
|Weeks 21–28|External alpha; schema lock-down preparation     |Onsager continues daily-use dogfood                                  |
|Months 7–12|Schema OSS; public alpha; commercial tier        |Duhem sufficiently mature that Onsager’s pace would suffer without it|

## 15. Risks and Open Questions

### Risks

- **Bandwidth dilution between Onsager and Duhem.** As a solo builder, simultaneous active development of two complex products risks shallow progress on both. Mitigation: explicit weekly designation of which project is “lead”; time-box Duhem MVP build (12 weeks); accept that Onsager’s pace and Duhem’s pace will not be balanced — one will drag the other in alternating windows.
- **Onsager dogfooding scope mismatch.** Onsager’s verification needs may not represent the broader market. Risk: Duhem optimizes for Onsager’s idiosyncrasies. Mitigation: parallel customer interviews from Phase 0 ensure external signal; resist Onsager-specific features that don’t generalize.
- **Closed source signaling problem.** AI dev tool community partly expects OSS. Closed-source-first may slow community traction. Mitigation: free CLI binary, generous free tier, public schema documentation, and explicit roadmap commitment to schema OSS in Phase 2.
- **Holistic verification is expensive.** Real environments cost more than mocks. Mitigation: cost is acknowledged and framed (production incident is more expensive); environment-share architecture amortizes setup; long-term, environment caching reduces per-run cost.
- **Generation quality.** AI-generated checks may miss edge cases. Mitigation: human review is mandatory; checks are frozen, not regenerated each run.
- **Maintenance burden.** Checks need updating when implementation changes intentionally. Mitigation: tight criteria-to-check coupling means impact is predictable; UI-snapshot churn handled by role-based locators.
- **Market category formation.** “Verification” as a distinct category from “testing” must be communicated. Mitigation: positioning around AI delivery is sharp; market formation already underway (Qodo, Antithesis raising at scale).
- **Schema lock-in before maturity.** If schema is committed too early to OSS license, future breaking changes carry community cost. Mitigation: keep schema closed (as documentation) through Phase 1; only open-source it after breaking-change rate drops below threshold.

### Open Questions

- **Schema OSS trigger.** What measurable criteria signal that schema is stable enough to OSS? Candidates: breaking-change rate < 1 per quarter; first paying enterprise customer in production; N external users at given scale. Decision needed before Phase 2.
- **Schema OSS license.** Apache 2.0 (permissive, like OpenAPI) vs. MIT (most permissive). Decision deferrable to Phase 2.
- **CLI source disposition.** When (if ever) is CLI source itself open-sourced? Probably Phase 3+; commercial value is low, but signaling value of opening it is real.
- **Self-hosted judge license terms.** Enterprise customers requiring self-hosting need a license. Source-available with restrictive terms (BSL-style) or pure proprietary binary distribution? Decision needed when first enterprise customer requires self-host.
- **L3 visual assertion scope.** When do we add visual-baseline checks? What’s the cost of running a visual diff infrastructure?
- **Inconclusive escalation policy defaults.** Block on inconclusive (safe) or pass with warning (fast)? Probably a per-criterion configurable, but defaults matter.
- **Action marketplace governance.** Who reviews community-contributed action types? What’s the review bar?
- **MCP server semantics for AI agents.** Should AI agents be able to *propose* checks (which humans then review), or only *consume* checks?
- **Multi-tenancy model for evidence storage.** Per-org isolation seems right, but cross-org analytics for action-type quality could be valuable — how to balance?
- **Environment provisioning at scale.** How do we make production-equivalent environments cheap enough to run on every PR? Containerized snapshots? Ephemeral cluster slices?
- **Environment provisioning at scale**: how do we make production-equivalent environments cheap enough to run on every PR? Containerized snapshots? Ephemeral cluster slices?

-----

## Appendix A — Glossary

|Term                   |Definition                                                                                                                                                            |
|-----------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------|
|Verification           |A complete cycle that produces a verdict for an artifact in a holistic environment                                                                                    |
|Verification Definition|The structured YAML format containing criteria and their checks for a feature; identified by content (top-level `verification:` or `criteria:` field), not by filename|
|Root Manifest          |The canonical `duhem.yml` at project root that aggregates Verification Definitions and provides shared defaults; auto-discovered by the tool                          |
|Criterion              |A human-authored natural-language commitment about a feature                                                                                                          |
|Check                  |A structured executable verification unit derived from a criterion, exercising a slice of the delivery web                                                            |
|Step                   |A single action or observation within a check                                                                                                                         |
|Assertion              |A mechanical predicate over observed state                                                                                                                            |
|Verdict                |The aggregated outcome (pass / fail / inconclusive)                                                                                                                   |
|Evidence               |The append-only structured trace from a run                                                                                                                           |
|Artifact               |The thing being verified (typically a Git ref)                                                                                                                        |
|Run                    |One execution of a verification                                                                                                                                       |
|Action type            |A reusable, named operation invoked via `uses:`                                                                                                                       |
|Judge                  |The deterministic evaluator producing verdicts                                                                                                                        |
|Web                    |The full set of components, configurations, prompts, data, and runtime context that the artifact depends on; never decomposable into independently testable units     |

## Appendix B — Design Decisions

|Decision                |Choice                                    |Rationale                                                                  |
|------------------------|------------------------------------------|---------------------------------------------------------------------------|
|Name                    |Duhem                                     |After Pierre Duhem; AI delivery is the engineering instance of Duhem-Quine |
|Two-layer AC structure  |Criterion (free-form) + Check (structured)|Stable commitments separate from volatile implementation                   |
|Verification scope      |Holistic (the web)                        |Component testing is incompatible with AI-delivered systems per Duhem-Quine|
|Schema basis            |GitHub Actions style + Arazzo borrowings  |Cognitive familiarity; AI generation reliability; CI-native                |
|Verdict states          |Three-state (pass/fail/inconclusive)      |Real systems have flakes; binary forces wrong choices                      |
|Failure attribution     |Post-hoc, evidence-supported, human-led   |Localizing holistic failure cannot be automated honestly                   |
|Judge implementation    |Deterministic, no LLM                     |Trust requires AI not be the verifier of AI                                |
|UI testing scope (MVP)  |DOM-level + behavioral                    |Covers most user-visible failure modes; visual-baseline post-MVP           |
|Source posture (Phase 1)|Closed source, free CLI binary            |Schema iteration freedom; solo founder bandwidth; commercial optionality   |
|Schema OSS (Phase 2)    |Open under permissive license once stable |Ecosystem benefits exceed proprietary IP value once schema is stable       |
|First customer          |Onsager (dogfood)                         |Real complexity, real urgency, continuous use, free testimonial            |
|Pricing anchor          |$30/seat/month                            |Validated by adjacent products (Qodo, CodeRabbit)                          |
|Build approach          |Solo founder + Claude Code                |Realistic given Sydney migration and parallel Onsager work                 |

## Appendix C — On Duhem-Quine

Pierre Duhem (1861–1916), French physicist and philosopher of science, argued in *La théorie physique* (1906) that physical theory is tested as a whole, not as individual hypotheses. When a prediction fails, the experimenter cannot tell from the failure alone whether the central hypothesis is wrong, or whether one of the auxiliary assumptions (about the apparatus, the measurement procedure, the broader theoretical framework) is what broke. He called this the “non-decisiveness of experimentum crucis.”

W.V.O. Quine (1908–2000), American logician and philosopher, generalized this in *Two Dogmas of Empiricism* (1951): “Any statement can be held true come what may, if we make drastic enough adjustments elsewhere in the system. Conversely, by the same token, no statement is immune to revision.”

The combined thesis — confirmation holism, or the Duhem-Quine thesis — has consequences across philosophy of science. For our purposes: any complex deployed software system instantiates this thesis. AI delivery, where the system includes prompts, contexts, tools, model versions, runtime infrastructure, and the code itself, instantiates it sharply. Verification of such systems must be holistic, must produce evidence sufficient for human investigation rather than false certainty about cause, and must accept the post-hoc nature of failure attribution as a fact, not a flaw.

Duhem the platform is named in honor of the man whose thesis tells us why we are necessary.

## Appendix D — Why Onsager dogfoods Duhem

Onsager is Duhem’s first customer. This is not a placeholder relationship until “real customers” arrive; it is a strategically chosen dogfooding arrangement that gives Duhem capabilities no other early-stage validation could provide.

### What Onsager provides Duhem

**Real complexity reflective of target customers.** Onsager is not a toy app. It is a multi-package monorepo with agent sessions, an event-driven spine substrate, workflow orchestration, governance layers, and a shaping executor. The verification needs Onsager generates resemble what Duhem’s eventual paying customers will need. Verifying Onsager forces Duhem to handle non-trivial cases from day one.

**Real urgency.** Onsager’s development velocity depends on trustworthy verification of AI-delivered features. The builder of Duhem is the user of Duhem. The cost of Duhem being broken is felt by the same person who can fix it. This collapses the feedback loop that normally takes external customers months to close.

**Continuous use.** Duhem is exercised every PR, every day. Design flaws surface fast because they cause friction in the builder’s own work. This is the strongest possible signal-to-noise ratio for early product iteration. The first dogfood Verification Definition lives in-tree at [`verifications/onsager-dashboard-create-spec-plan/`](../verifications/onsager-dashboard-create-spec-plan/) and runs through the `duhem/run` composite GitHub Action.

**Proof point for external sales.** When Duhem is ready for external customers, “we use it ourselves to verify a complex AI-orchestration platform” is a stronger claim than any synthetic case study. The product carries its own proof.

### What Duhem provides Onsager

**Verification of AI-generated artifacts.** Onsager’s own development uses AI coding assistants heavily. Duhem provides the structured verification layer that prevents AI-claimed completeness from masking actual failure.

**A demonstrable AI factory pattern.** When Onsager’s AI factory paradigm matures and begins producing other products at scale, Duhem is the verification gate that makes that production trustworthy. The Onsager-Duhem pair is itself a sellable pattern: AI factory + verification gate = trustworthy AI-delivered software at scale.

### Why this is not bootstrapping deadlock

A naive reading might worry that Onsager and Duhem cannot both be early simultaneously — each depends on the other being mature. This concern is mistaken because **Duhem in Phase 0 and Phase 1 is built without Onsager-paradigm dependencies**. Duhem is built directly using Claude Code as the development assistant, in the same way any solo founder would build a product today. It does not require Onsager to exist as an AI factory.

What Onsager provides Duhem is not infrastructure but customer use. Onsager is the workload Duhem verifies, not the production line that builds Duhem.

The factory paradigm — Onsager producing other products — is a future state (Phase 4+) where Duhem’s role becomes that of the production line’s quality gate. In the present, Onsager is Duhem’s most valuable customer, and Duhem is built the conventional way.
