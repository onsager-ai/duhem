---
name: verification-authoring
description: Author Duhem Verification Definitions (criteria + checks YAML) that verify your product's features. Use when asked to "write a verification", "spec acceptance criteria", "author checks", "verify this feature", "translate criteria to checks", "write criteria", or "draft a Verification Definition" against a product you verify with Duhem. Enforces Duhem's Holistic Verification Principle (no mocks of the web), the criteria-vs-checks separation (criteria stable, checks derivative), and the mechanical-judgment rule (no LLM-in-the-loop verdicts).
---

# verification-authoring

Author Verification Definitions for your product — the structured YAML
artifact holding a feature's acceptance criteria and the checks that
mechanically verify them. This is the central authoring discipline for
using Duhem to verify a product from the outside.

A Verification Definition exercises the **real, deployed web** — code +
prompts + tool wiring + data + runtime — together, and produces a verdict
by deterministic evaluation of structured assertions. No LLM is in the
verdict loop.

**Retrieve, don't recall.** Duhem's action catalog is versioned and
evolves; don't guess `with:` keys or output names from memory. Ask the
CLI for the version-exact contract as you author:

```bash
duhem actions                 # list the action catalog
duhem describe api/call       # one action's with: / outputs / example
duhem validate <dir>          # field-check your VD against the catalog
duhem mcp                     # expose describe/actions/validate to your
                              #   coding agent over MCP, for AI-authoring
```

Author → `validate` → fix converges to a correct VD without any prior
Duhem knowledge. That's retrieval plus verification, not pretraining.

## When to use this skill

Use when:

- You're writing a Verification Definition for a feature of your product.
- Someone asks to "write criteria" or "translate criteria to checks".
- A feature needs a mechanically-judged acceptance test that exercises
  the real system end-to-end.

Skip when:

- The change is purely internal (build config, refactor with no
  externally observable behavior change). Those are unit/integration
  tests in your own toolchain, not Verification Definitions.

## Where the VD lives

Verification Definitions live **in your product repo**, co-located with
the product under a `.duhem/` suite. Scaffold one with:

```bash
duhem init --name <slug> .duhem/<slug>   # runnable skeleton in .duhem/
```

(`duhem init` defaults its target to `./verifications/<slug>/`, so pass
the `.duhem/<slug>` path explicitly to land it in your co-located suite.)

The skeleton is a single passing check against a known-good baseline —
mutate it toward your feature. `duhem run` discovers the `.duhem/`
manifest by walking up from the current directory (capped at the
enclosing `.git`), so `duhem run` from anywhere in the repo finds it;
`-f path/to/duhem.yml` overrides discovery.

Self-gate the suite in your product's own CI: run `duhem run` on the PR
and let the verdict gate merge.

## The two-layer structure

Duhem deliberately separates a feature's commitments from the mechanism
that verifies them:

```
Criterion  →  natural language, human-authored, stable across
              implementations.

Checks     →  structured YAML, AI-translatable, frozen after human
              review, one-or-many per criterion.
```

**Criteria are stable; checks are derivative.** When the implementation
changes, criteria do not. When criteria change, that's a real change to
the contract. Write criteria first, then translate each into checks. If a
criterion already exists (lifted from a spec / acceptance test), copy it
verbatim and translate mechanically — don't re-author it.

## Authoring loop

```
0. Scaffold: `duhem init --name <slug> .duhem/<slug>` → a runnable
   skeleton in `.duhem/` (a single passing baseline check to mutate).
   Pass the path explicitly — `init`'s default target is
   `./verifications/<slug>/`.
1. Lift criteria from the spec / acceptance test / PRD.
2. Validate the criteria are stable, intent-bearing, one-commitment-each.
3. Translate each criterion into one or more checks (steps + assertions).
4. Review the holistic-environment tax — no mocks of the web.
5. Self-validate: every assertion mechanical, every referenced output
   reachable, every check has a verdict.
6. Save the file (self-identifies via top-level `verification:`).
7. Register it in `.duhem/duhem.yml` if the suite uses a manifest.
```

### 1. Lift criteria

Find the source of intent for the feature — a spec's acceptance items, a
PRD, or the "what does done mean" answer a stakeholder gives. Each
criterion is **a single coherent commitment, in 1–3 sentences, that a
non-technical stakeholder can read and validate.** A feature typically
has 2–6 criteria.

A criterion expresses *intent*, not *procedure*:

> ✅ A user can create a workspace from the dashboard. The new workspace
> becomes immediately visible in their workspace list, and the user is
> navigated to the workspace's home page. No errors are shown.
>
> ❌ When the user clicks "Create Workspace", the system POSTs to
> `/workspaces` with `{name, owner_id}`, receives a 200 with the
> workspace ID, then redirects to `/workspaces/<id>`.

The first describes what "done" means. The second describes *how* — and
would have to be rewritten any time the implementation changes. That's a
check, not a criterion.

### 2. Validate criteria

Before translating, sanity-check each criterion:

- [ ] One coherent commitment (not "and also and also …").
- [ ] 1–3 sentences.
- [ ] Free of implementation language (no endpoint paths, function names,
      DB tables).
- [ ] Free of step-by-step procedure.
- [ ] A non-technical stakeholder could read it and say yes/no.
- [ ] Stable across plausible implementation changes.

If a criterion violates any of these, rewrite it before translating.

### 3. Translate to checks (terse by default)

Each criterion gets one or more checks. A check is a sequence of `steps`
(named actions) followed by `assertions` (mechanical predicates over
named outputs). **A single check should exercise a slice of the holistic
web** — UI input, network, API shape, data — not a single component.
Decomposing a check into per-component sub-checks loses what makes Duhem
Duhem.

Author the **terse form** — Duhem infers the ceremony:

- **`outputs:` is optional.** Reference any output an action declares
  directly as `$steps.<id>.outputs.<name>` — including nested paths
  (`$steps.home.outputs.body.data._id`). No `outputs:` block needed just
  to name a field. Add `outputs:` only to *rename* a field
  (`outputs: { code: status }`) or bind a *deep extraction* to a short
  alias (`outputs: { project_id: body.data._id }`) so you write the path
  once. An identity binding like `outputs: { foo: foo }` is a redundant
  no-op — `validate` flags it.
- **`assertions:` is optional for a judging step.** A `ui/assert-*` (or
  `api/poll`) step *is* the judgment — it emits `satisfied` and Duhem
  implicitly asserts `satisfied == true`. An all-assert check needs no
  `assertions:` block at all. Bind `satisfied` and assert it yourself
  only for manual control (e.g. a disjunction across steps). A check with
  neither `assertions:` nor a judging step is rejected at validate time.

Retrieve each action's real `with:` fields and outputs with
`duhem describe <uses>` before writing the step — don't guess. If you
genuinely need an action type the catalog doesn't have, that's a Duhem
engine change, not something to invent locally; don't silently mint new
`uses:` strings.

Common shape (terse):

```yaml
- id: AC-1.1
  description: <what slice of the web this check exercises>
  steps:
    - id: create
      uses: api/call
      with: { method: POST, url: $inputs.api_base/workspaces, within: 3s }
      # no outputs: block — status / body / body.id resolve directly
  assertions:
    - $steps.create.outputs.status == 200
    - type_check: { value: $steps.create.outputs.body.id, is: uuid }
```

Authoring rules:

- Reference outputs by their fully-qualified path,
  `$steps.<id>.outputs.<name>`; add `outputs:` only for a rename or a
  deep-extraction alias.
- Timeouts (`within:`) are explicit on steps that observe something
  asynchronous.
- Use role-based locators (`{ role: "button", name: "…" }`) for `ui/*`
  rather than CSS or XPath — UI churn invalidates the latter while
  role-based selectors track the user-visible affordance.
- A check with no verdict — neither `assertions:` nor a judging step — is
  a script, not a check. Reject it.
- The `capture/` output-name prefix is reserved for runner-emitted
  failure evidence (a failing `ui/*` check auto-records a screenshot,
  DOM, and network HAR). Captures are evidence for humans/agents, never
  judge input; an authored output under `capture/` is rejected at
  validate time.

### 4. The holistic-environment tax — no mocks of the web

A Duhem check exercises real behavior end-to-end. **No mocking the web.**

- There is no `api/mock` or `db/stub` action — they don't exist by
  design.
- Don't write a check that runs against an in-memory test double of any
  subsystem the artifact depends on.
- If a check needs data preconditions (a seeded user, a pre-existing
  record), use `db/seed` (or an equivalent setup action) in the
  verification's `setup:` block, against the **real** database. Use
  `setup:` for once-per-verification preconditions; don't duplicate them
  inside every check.

If verifying the criterion would require mocking the web, that's a
signal either the criterion's assumptions are unstated (reformulate it
against the real web) or the check is the wrong shape. Don't paper over
it with a mock — stop and reconsider the criterion.

### 5. Mechanical judgment — no LLM in the verdict

Every assertion must be a deterministic predicate the judge can evaluate.
Allowed forms:

- Boolean expression: `$steps.X.outputs.Y == 200`
- Type check: `type_check: { value: …, is: uuid|email|datetime|… }`
- Pattern match: `$runtime.matches(value, "regex")`
- Membership: `$runtime.contains(haystack, needle)` — literal substring
  on a string, element membership on an array
- Existence: `exists: $steps.X.outputs.Y`
- Cross-step consistency: `equal: [$steps.A.outputs.X, $steps.B.outputs.X]`

Things that look like assertions but aren't: "the response makes sense",
"an LLM grades the output", "the screenshot looks right". None are
mechanical; none are allowed. A type error in an assertion (e.g.
`contains` against a number) is a `fail` that names the mismatch, not a
silent pass. If a criterion seems to require non-mechanical judgment,
split it until each piece is mechanical, or note explicitly that it
can't be Duhem-verified today — don't paper over it.

### 6. Self-validate

Before saving, walk through — then let `duhem validate` close the loop:

- [ ] Each criterion appears verbatim under `criteria:` with an `id:`
      (`AC-1`, `AC-2`, …).
- [ ] Each criterion has at least one check.
- [ ] Each check has a non-empty `steps:` and a verdict (an `assertions:`
      block **or** a judging step).
- [ ] Every referenced output is one the action declares (or is bound via
      `outputs:` for a rename/extraction).
- [ ] Every assertion is one of the allowed mechanical forms.
- [ ] No mock action types; no LLM-grading anywhere.
- [ ] Action types come from the catalog (`duhem actions`).

### 7. Save and register

The file can be named anything (it self-identifies via a top-level
`verification:` / `criteria:`). Conventional names:
`<feature>.verification.yml`, `<feature>.yml`, `verification.yml`. If the
suite uses a `.duhem/duhem.yml` manifest, add the file to
`verifications:`; a standalone file run via `duhem run <file>` needs no
manifest entry.

## Worked example template

```yaml
verification: <descriptive name>
spec_ref: <link to the spec / doc this verifies>

inputs:
  api_base:
    type: string
    default: https://staging.example.com

setup:
  # once-per-verification preconditions — real environment, no mocks
  - uses: db/seed
    with:
      table: users
      rows:
        - { id: $runtime.uuid(), email: "test@example.com" }

criteria:
  - id: AC-1
    description: |
      <verbatim from the spec; 1-3 sentences; intent over implementation>
    checks:
      - id: AC-1.1
        description: <what slice of the web this check exercises>
        steps:
          - id: <step-id>
            uses: <action-type>
            with: { … }
            # add outputs: only to rename or deep-extract a field
        assertions:
          - <mechanical predicate over $steps.<id>.outputs.<name>>
```

## Anti-patterns (don't)

- **Criteria with implementation details.** "Click the button at
  `#create-workspace`" is a check pretending to be a criterion. Rewrite
  as "A user can create a workspace from the dashboard."
- **A check with no verdict.** Neither an `assertions:` block nor a
  judging step — it's a recording, not a check.
- **Redundant identity bindings.** `outputs: { foo: foo }` does nothing —
  reference `$steps.<id>.outputs.foo` directly. `outputs:` is for a
  *rename* or a *deep extraction*, not for re-declaring a native output.
- **Mocks anywhere.** No `api/mock`, no `db/stub`, no time-freeze
  short-circuits. The web Duhem verifies is the real one.
- **LLM-grading assertions.** Non-mechanical judgment is out.
- **One mega-criterion.** "User can create, list, edit, archive, and
  delete workspaces" is five criteria.
- **Re-authoring frozen checks every run.** Checks are authored once,
  reviewed, then frozen. Regenerating them from scratch reintroduces the
  drift Duhem exists to prevent.
- **Verifications that smuggle in unit tests.** A check that exercises a
  single function in isolation isn't holistic — expand its scope to a
  real web slice or remove it.

## References

- `duhem actions` / `duhem describe <uses>` — the version-exact action
  catalog and per-action contract. Author against these, not memory.
- `duhem validate <dir>` — field-checks your VD and names valid options
  on a miss.
- `duhem run <dir>` — runs the suite (exit 0 == pass). Add
  `--reporter json` for a per-criterion verdict breakdown; the default
  reporter prints only the overall pass/fail.
- `duhem mcp` — exposes describe/actions/validate to your coding agent
  over MCP for AI-assisted authoring.
- The Duhem documentation for the Holistic Verification Principle, the
  criteria-vs-checks separation, and the full Verification Definition
  schema.
