---
name: verification-authoring
description: Author Duhem Verification Definitions (criteria + checks YAML) for features under verification. Use when asked to "write a verification", "spec acceptance criteria", "author checks", "verify this feature", "translate criteria to checks", "write criteria", "draft a Verification Definition", or when a Duhem product-surface spec needs a worked example, or when adding a Verification Definition to dogfood a feature on onsager-ai/onsager. Enforces Duhem's Holistic Verification Principle (no mocks of the web), the criteria-vs-checks separation (criteria stable, checks derivative), and the mechanical-judgment rule (no LLM-in-the-loop verdicts).
---

# verification-authoring

Author Verification Definitions for Duhem — the structured YAML
artifact containing a feature's acceptance criteria and the checks
that mechanically verify them. This is the central authoring
discipline that any Duhem-shaped customer (including Duhem itself,
when it dogfoods on Onsager) eventually internalizes.

The canonical product spec is in `docs/duhem-spec.md`, especially:

- §7 — Core Concepts (verification, criterion, check, assertion,
  step, verdict, evidence, artifact, run)
- §8 — Holistic Verification Principle
- §10 — Verification Definition (file org, structure, action types,
  assertions, runtime expressions, extensibility)
- §11.2 — Trust Boundary (what AI may and may not do at run time)

This skill is the *authoring* counterpart to those sections.

## When to use this skill

Use when:

- A Duhem product-surface spec needs a worked Verification
  Definition example (the requirement is enforced in `issue-spec`'s
  Philosophy #4).
- You're writing a Verification Definition for the Duhem repo
  itself (for example, a verification of `duhem run`'s CLI
  behavior).
- You're writing a Verification Definition that targets the
  Onsager repo as Duhem's first dogfood customer (see also
  `onsager-dogfood` for the cross-repo wiring).
- A user asks to "write criteria" or "translate criteria to
  checks" or similar.

Skip when:

- The change is purely internal scaffolding (build config, repo
  hygiene, doc rewording with no schema implication).
- The user asked for an *Onsager* spec — that uses Onsager's own
  testing patterns; Duhem Verification Definitions for Onsager
  features live on the Duhem side of the seam (in
  `area:dogfood`).

## The two-layer structure

Duhem deliberately separates a feature's commitments from the
mechanism that verifies them:

```
Criterion  →  natural language, human-authored, stable across
              implementations.

Checks     →  structured YAML, AI-translatable, frozen after
              human review, one-or-many per criterion.
```

**Criteria are stable; checks are derivative.** When the
implementation changes, criteria do not. When criteria change,
that's a real change to the contract.

When you draft a Verification Definition, write criteria first,
then translate each into checks. If you're translating a criterion
that already exists (e.g. lifted from a spec issue's `## Test`
section), don't re-author it — copy verbatim and translate
mechanically.

**Where the VD lives (Pattern D).** A VD that verifies a *product*
(Onsager, Chreode, Crawlab) lives **in the product's repo** under a
co-located `.duhem/` suite — Duhem is used as a tool, not a host
(`docs/duhem-spec.md` §10.1 Pattern D; epic #225). Only Duhem's own
**self-verification** VDs live here in `verifications/`. Chreode is the
worked example (`onsager-ai/chreode/.duhem/`); `templates/product-repo/`
is the drop-in `.duhem/` skeleton (manifest + example leaf + CODEOWNERS)
to copy into a product repo. Author the VD there, self-gate it in the
product's own CI (Mode A), and let Duhem's drift lane monitor it (Mode
B) — see `onsager-dogfood`.

## Authoring loop

```
0. Scaffold the skeleton: `duhem init --name <slug>` produces a
   runnable Pattern A skeleton (or `--pattern B` for co-located)
   under `./verifications/<slug>/` (Duhem self-verification), or
   `.duhem/<slug>/` in the product repo for a product VD (Pattern D;
   copy `templates/product-repo/`). The skeleton is a single passing
   check against https://example.com — your known-good baseline to
   mutate. Spec on issue #48.
1. Lift criteria from the spec / acceptance test / PRD
2. Validate criteria are stable, intent-bearing, scoped to one commitment
3. Translate each criterion to one or more checks (steps + assertions)
4. Review the holistic-environment tax — no mocks of the web
5. Self-validate: every assertion is mechanical, every step is named, every
   ID is referenced
6. Save the file (any name; self-identifies via top-level `verification:`)
7. Update duhem.yml (if Pattern B/C) to register the file
```

Start every new Verification Definition with `duhem init`; don't
copy-paste from `verifications/onsager-dashboard-create-spec-plan/`
or hand-write a fresh tree from scratch. The skeleton bakes the
criteria-vs-checks two-document discipline into the first commit
and gives you a passing run to confirm your environment works
before you've authored anything.

### 1. Lift criteria

Find the source of intent for the feature:

- A spec issue's `## Test` items, when the spec is well-formed.
- A PRD or feature description in plain English.
- The "what does done mean for this PR" answer the human gives
  when you ask.

Each criterion is **a single coherent commitment, in 1–3 sentences,
that a non-technical stakeholder can read and validate.** A feature
typically has 2–6 criteria.

A criterion expresses *intent*, not *procedure*:

> ✅ A user can create a workspace from the dashboard. The new
> workspace becomes immediately visible in their workspace list,
> and the user is navigated to the workspace's home page. No
> errors are shown.
>
> ❌ When the user clicks the "Create Workspace" button, the
> system POSTs to `/workspaces` with `{name, owner_id}`, receives
> a 200 with the workspace ID, then redirects to
> `/workspaces/<id>`.

The first describes what "done" means. The second describes how —
and would have to be rewritten any time the implementation changes.
That's a check, not a criterion.

### 2. Validate criteria

Before translating, sanity-check each criterion:

- [ ] One coherent commitment (not "and also and also …")
- [ ] 1–3 sentences
- [ ] Free of implementation language (no endpoint paths, no
      function names, no DB tables)
- [ ] Free of step-by-step procedure
- [ ] A non-technical stakeholder could read it and say yes/no
- [ ] Stable across plausible implementation changes

If a criterion violates any of these, rewrite it before
translating.

### 3. Translate to checks

Each criterion gets one or more checks. A check is a sequence of
`steps` (named actions) followed by `assertions` (mechanical
predicates over named outputs).

**A single check should exercise a slice of the holistic web** —
not a single component. The example in `docs/duhem-spec.md` §10.3
(`AC-1.1`) exercises five layers (UI input capture, UI button
activation, network observation, API response shape, ID semantics)
in one check, and that's the *intended* shape, not over-reach. Per
the Holistic Verification Principle, decomposing a check into per-
component sub-checks loses what makes Duhem Duhem.

Action types live in §10.5. Use the existing catalog when one
fits; if you genuinely need a new type, that's an
`area:schema` spec — see `issue-spec`. Do not silently mint new
`uses:` strings.

Common shape:

```yaml
- id: AC-1.1
  description: <what this check verifies — paraphrase the criterion slice>
  steps:
    - uses: ui/click
      with: {role: "button", name: "Create Workspace"}
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
```

Authoring rules:

- Every step that produces output gets an `id:` so assertions can
  reference it.
- Outputs are explicit — `outputs: { name: <expression> }` —
  never implicit.
- Assertions reference outputs by their fully-qualified path:
  `$steps.<id>.outputs.<name>`.
- Timeouts (`within:`) are explicit on steps that observe
  something asynchronous.
- Use role-based locators (`{role: "button", name: "..."}`)
  rather than CSS or XPath — UI churn invalidates the latter
  while role-based selectors track the user-visible affordance.
- A check that has no `assertions:` is a script, not a check.
  Reject it.
- The `capture/` output-name prefix is reserved for runner-emitted
  failure evidence (specs #202 / #204): a failing ui check
  automatically records `capture/screenshot` + `capture/dom` +
  `capture/network` (a HAR 1.2 log of the page's traffic) blob
  observations (`duhem run --capture` controls the policy). An
  authored output under `capture/` is rejected at validate time, and
  captures are never recorded as `$steps.<id>.outputs.*` bindings —
  so nothing can forge a capture and no assertion can bind one.
  Captures are evidence for humans/agents, never judge input. The
  network HAR redacts sensitive headers and auth request bodies, but
  response bodies are captured verbatim (the repair signal) — like the
  DOM snapshot, a page that echoes a secret in a response carries it
  into the evidence, which is shipped to the hub; keep that in mind
  for capture-sensitive targets (use `--capture off`).

### 4. The holistic-environment tax

Per §8, a Duhem check, by default, exercises real behavior
end-to-end. **No mocking the web.** When you author a check:

- Don't propose an `api/mock` or `db/stub` action — they don't
  exist by design.
- Don't write a check that runs against an in-memory test double
  of any subsystem the artifact depends on.
- If a check requires data preconditions (a seeded user, a
  pre-existing workspace), use `db/seed` or `event/publish` in
  the verification's `setup:` block, against the **real**
  database / event bus.
- Use `setup:` for once-per-verification preconditions; don't
  duplicate them inside every check.

If verifying the criterion would require mocking the web, that's
either:

1. A signal the feature's auxiliary assumptions are unstated and
   the criterion should be reformulated against the real web,
   **or**
2. A signal that this check is the wrong shape for Duhem and a
   different check (or the spec being verified) needs to change.

Don't paper over it with a mock. Stop and escalate.

### 5. Mechanical judgment

Per §11.2, **no LLM is in the verdict loop.** Every assertion must
be a deterministic predicate evaluable by the judge. Allowed forms
(§10.6):

- Boolean expression: `$steps.X.outputs.Y == 200`
- Type check: `type_check: {value: ..., is: uuid|email|datetime|...}`
- Pattern match: `matches: {value: ..., pattern: ...}`
- Set membership: `in: {value: ..., set: [...]}`
- Existence: `exists: $steps.X.outputs.Y`
- Cross-step consistency: `equal: [$steps.A.outputs.X, $steps.B.outputs.X]`

Things that look like assertions but aren't:

- "The response makes sense" — not mechanical.
- "An LLM grades the output" — explicitly forbidden by §11.2.
- "The screenshot looks right" — no L3 visual baseline yet
  (`docs/duhem-spec.md` §14 Phase 3 roadmap).

If a criterion seems to require non-mechanical judgment, that's
either a criterion in the wrong shape (split it until each piece is
mechanical) or a criterion that genuinely cannot be Duhem-verified
today — note that explicitly in the spec, don't paper over it.

### 6. Self-validate

Before saving, walk through:

- [ ] Each criterion appears verbatim under `criteria:` with an
      `id:` (`AC-1`, `AC-2`, …)
- [ ] Each criterion has at least one check
- [ ] Each check has a non-empty `steps:` and a non-empty
      `assertions:` block
- [ ] Every step output that's referenced is actually declared
- [ ] Every assertion is one of the six allowed forms
- [ ] No mock action types
- [ ] No LLM-grading anywhere
- [ ] Action types come from the documented catalog (or are
      called out as new in the linked spec)

### 7. Save and register

The file can be named anything (§10.2 self-identification by
top-level `verification:`/`criteria:`). Conventional names:
`<feature>.verification.yml`, `<feature>.yml`, `verification.yml`.

If the project uses Pattern B/C with a root `duhem.yml` manifest,
add the file to `verifications:` (§10.4). If it's a standalone
file run via `duhem run <file>`, no manifest update needed.

`duhem run` discovers the manifest by walking the current directory
and its ancestors (capped at the enclosing `.git`), so `cd
anywhere-in-the-repo && duhem run` finds the repo-root `duhem.yml`
(or `.duhem.yml`) without a path argument (#69). `-f path/to/manifest.yml`
overrides discovery for an out-of-tree manifest.

## Worked example template

Use this as the skeleton for any spec that needs a worked example:

```yaml
verification: <descriptive name>
spec_ref: <link to spec issue or doc>

inputs:
  # named inputs — e.g. test fixture values, defaults
  example_input:
    type: string
    default: "fixture-{{$runtime.uuid()}}"

setup:
  # once-per-verification preconditions — real environment, no mocks
  - uses: db/seed
    with:
      table: users
      rows:
        - {id: $runtime.uuid(), email: "test@example.com"}

criteria:
  - id: AC-1
    description: |
      <verbatim from spec; 1-3 sentences; intent over implementation>
    checks:
      - id: AC-1.1
        description: <what slice of the web this check exercises>
        steps:
          - uses: <action-type>
            id: <step-id>
            with: { ... }
            outputs:
              <name>: <expression>
        assertions:
          - <mechanical predicate over $steps.<id>.outputs.<name>>
```

## Anti-patterns (don't)

- **Criteria with implementation details.** "Click the button at
  `#create-workspace`" is a check pretending to be a criterion.
  Rewrite as "User can create a workspace from the dashboard."
- **A check with no assertions.** It's a recording, not a check.
  Add the assertion or delete the check.
- **Implicit outputs.** `outputs:` is mandatory for any value an
  assertion reads. Magic field access like
  `$steps.api_call.response.body.id` without a declared output is
  a bug; the schema validator should reject it.
- **Mocks anywhere.** No `api/mock`, no `db/stub`, no
  `time/freeze` short-circuits. The web Duhem verifies is the
  real one.
- **LLM-grading assertions.** Non-mechanical judgment is
  explicitly out per §11.2.
- **One mega-criterion.** Each criterion should be one coherent
  commitment. "User can create, list, edit, archive, and delete
  workspaces" is five criteria.
- **Re-authoring frozen checks every run.** §7.3: checks are
  authored once, reviewed, then frozen. Regenerating them from
  scratch every run reintroduces drift Duhem exists to prevent.
- **Verifications that smuggle in unit tests.** A check that
  exercises a single function in isolation isn't holistic — it's a
  unit test in a Verification Definition's clothing. Either expand
  its scope to a real web slice or remove it.

## Relationship to other skills

| Related surface                                              | Role                                                              |
|--------------------------------------------------------------|-------------------------------------------------------------------|
| [`issue-spec`](https://github.com/onsager-ai/dev-skills/blob/main/skills/issue-spec/SKILL.md) | Specs that introduce product surface link a worked example here. Installed globally from `onsager-ai/dev-skills`. |
| [`duhem-dev-process`](../duhem-dev-process/SKILL.md)         | Top-level SDD loop — the dogfood discipline that requires worked examples. |
| [`onsager-dogfood`](../onsager-dogfood/SKILL.md)             | Verifications that target `onsager-ai/onsager`; this skill writes them. |
| [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (global) | Verifies the worked-example check on schema-impacting PRs.        |

## References

- `docs/duhem-spec.md` — canonical product spec
  - §7 Core Concepts
  - §8 Holistic Verification Principle
  - §10 Verification Definition (full schema)
  - §11.2 Trust Boundary
- `docs/duhem-brand.md` — design discipline; rarely directly
  relevant to Verification Definitions, but worth reading for the
  Duhem-Quine grounding (§2 Design rationale)
