---
name: onsager-dogfood
description: Dogfood Duhem against the products it verifies (Onsager, Chreode, Crawlab) — Duhem-as-tool with co-located `.duhem/` VDs. Use when authoring or maintaining a product's Verification Definitions, when wiring a product self-gate (Mode A) or Duhem's drift-monitoring lane (Mode B), when triaging a verdict that fired on a product PR, or when you're unsure which side of the Duhem/product seam owns a change. Covers the cross-repo discipline (which spec lives where, which PR opens where), the trust model (mechanical judgment + a self-verified Duhem contract + optional CODEOWNERS/hub — not hoarded checks), and `docs/duhem-spec.md` §10.1 Pattern D / Appendix D / §14. Triggers include "dogfood on Onsager", "dogfood on Chreode", "verify against a product", "product PR check", "drift monitoring", "co-locate VDs", "cross-repo spec", "which repo owns this", "Duhem/product boundary".
---

# onsager-dogfood

Duhem's first customer is `onsager-ai/onsager`. This skill captures
the cross-repo discipline: which artifacts live where, which side
of the seam owns each kind of change, and the asymmetric trust
boundary that makes the dogfood relationship sound.

The product framing is in `docs/duhem-spec.md` Appendix D
("Why Onsager dogfoods Duhem") and the milestones in §14
(roadmap). Read those before invoking this skill if you haven't
already — the discipline below makes more sense once you've seen
why the relationship is structured the way it is.

> **Reframe (epic #225 — Duhem is a tool).** Product Verification
> Definitions no longer live in `onsager-ai/duhem`; they live **with
> the product** in a co-located `.duhem/` suite (`docs/duhem-spec.md`
> §10.1 Pattern D). The dogfood is now **drift monitoring**: run each
> product's real VDs with the current Duhem to catch a Duhem change
> that would break a consumer. A `pass` is trustworthy because Duhem's
> judge is mechanical (no LLM) and Duhem's own contract stays
> self-verified — **not** because the checks are hoarded here. The
> worked, shipped example is **Chreode**
> (`onsager-ai/chreode/.duhem/`): self-gated in its own CI (**Mode A**)
> and drift-monitored here via `.github/workflows/drift-chreode.yml`
> (**Mode B**). Onsager is the same pattern with its VD still in-tree
> until P4 (Onsager is paused). Where this skill below says "the
> dogfood VDs are Duhem artifacts in `onsager-ai/duhem`", read it
> through this reframe — the cross-repo/two-specs/no-mocks discipline
> still holds; only the VD's *home* and the trust *rationale* changed.

## The relationship in one paragraph

Onsager is the **workload** Duhem verifies. Onsager is not the
production line that builds Duhem (that distinction matters: a
Phase-4 future state has Onsager produce other products with
Duhem as the verification gate; we are nowhere near that yet).
Today, Onsager has features that need verification, Duhem has the
verification platform, and the two products move together because
the same person is building both. Duhem-Quine on the inside:
Duhem-Onsager as the dogfood pair instantiates the thesis
empirically.

## Repo seam

Two repos, one seam. Each repo has its own `.claude/skills/`
directory, its own SDD loop, its own pre-push and PR-lifecycle
discipline. They are **parallel, not shared**.

| Concern                                                       | Lives on                  | Skills involved                                                   |
|---------------------------------------------------------------|---------------------------|-------------------------------------------------------------------|
| Duhem schema, CLI, runtime, judge, dashboard, integrations    | `onsager-ai/duhem`        | All Duhem-side skills (`duhem-*`, `issue-spec`, `verification-authoring`) |
| Verification Definitions that exercise a product (Onsager, Chreode, Crawlab) | The **product's** repo, co-located `.duhem/` (Chreode moved; Onsager P4-pending, still in-tree) | `verification-authoring` (Pattern D)      |
| Duhem's own self-verification VDs                             | `onsager-ai/duhem` (`verifications/`) | `verification-authoring`                              |
| Onsager features themselves (forge, stiglab, synodic, dashboard, etc.) | `onsager-ai/onsager` | Onsager's `onsager-dev-process`, `issue-spec`, `dashboard-ui`, etc. |
| The GitHub Action / webhook that surfaces Duhem verdicts on Onsager PRs | `onsager-ai/onsager` (consumer side) + `onsager-ai/duhem` (publisher side) | Both sides use the global `pr-lifecycle` skill (with each repo's overlay); Duhem-side also this skill. |
| Spec for "Duhem ought to be able to verify behavior X on Onsager" | `onsager-ai/duhem`     | `issue-spec` here, label `area:dogfood`                            |
| Spec for "Onsager exposes hook Y so Duhem can observe it"     | `onsager-ai/onsager`      | Onsager's `issue-spec`                                             |

### The "which repo" decision rule

If you're not sure which repo a change belongs on, ask:

> Which artifact is being modified — a Duhem one (schema, action
> type, judge rule, Verification Definition) or an Onsager one
> (subsystem, dashboard component, event, migration)?

The answer is the repo. If the answer is "both", the change is
two specs (one on each repo) with a contract in between. The
contract belongs in **both** specs by reference — the Duhem-side
spec describes what it observes; the Onsager-side spec describes
what it exposes. Land both before either side's PR can merge.

This mirrors Onsager's own internal "split cross-subsystem specs"
rule — at the dogfood seam, the two subsystems happen to be
separate repos.

## Trust boundary — independence without hoarding

The verifier of AI claims must be structurally independent of the AI
making them. Under the reframe (#225), that independence does **not**
come from Duhem hoarding the product's checks — the product owns its
co-located `.duhem/` VD. It comes from two things that survive
co-location (`docs/duhem-spec.md` §11.2):

- **Mechanical judgment.** The verdict is deterministic evaluation of
  structured assertions — no LLM in the judge. A product can't argue
  its way to a `pass`; the assertions either hold against the real
  web or they don't (§7.6, §11.2).
- **A self-consistent, self-verified Duhem contract.** What must stay
  trustworthy is Duhem's own schema / judge / docs, gated by Duhem's
  self-verification suite — not the location of any product's VD.

Two lightweight guards keep a product from quietly weakening its own
gate (review/evidence discipline, not a structural wall):

- **CODEOWNERS on `/.duhem/`.** A product PR that edits its own VD to
  dodge a failure routes to a verifier reviewer. Optional and
  per-repo; Chreode ships the stanza (inert until the verifier team +
  branch protection exist).
- **Hub-recorded verdicts.** Duhem records each verdict with
  `(verifier_repo/sha, target_repo/sha)` provenance the product PR
  can't rewrite, so a product can't self-attest past a `fail`.

If you find yourself reaching for "I'll just relax the check from the
product side this once", **stop** — resolve the failure, escalate the
verdict, or change the criterion explicitly (a reviewed VD edit),
never a silent bypass. Same discipline you'd keep for a paying
customer.

## Authoring a co-located Verification Definition

When the trigger is "verify <product feature> with Duhem", the VD
lives in the **product's** repo under `.duhem/` (Pattern D). Chreode
is the worked example (`onsager-ai/chreode#288` / `#289`).

1. Confirm the product feature and its acceptance criteria (the
   product spec's `## Test` section — lean-spec criteria map straight
   onto Duhem criteria).
2. File the spec on the **product's** repo (its own `issue-spec`),
   e.g. `spec(...): adopt/extend the Duhem .duhem/ suite`. If Duhem
   needs a new surface (action type, schema field) to express the
   check, that's a *separate* Duhem spec (`area:dogfood` /
   `area:integration`) with its own worked-VD example.
3. Author the VD in the product's `.duhem/<slug>/` via
   `verification-authoring` (Pattern D). It drives the product's
   **real** environment — real API, real dashboard, real binaries,
   no mock (§8 / §11.2). `templates/product-repo/` is the drop-in
   skeleton.
4. Wire it to run:
   - **Mode A (product self-gate).** A `duhem` workflow in the product
     repo runs `duhem run .duhem/…` on its PRs — via the release
     binary (see chreode's `.github/workflows/duhem.yml`) or
     `duhem/run` with `verification-source: workspace`.
   - **Mode B (drift monitoring, this repo).** Duhem's CI runs the
     product's suite from a checked-out ref with a freshly-built
     `duhem` (`.github/workflows/drift-chreode.yml`; copy per
     product, swapping the target + env bring-up). A red here = a
     Duhem regression against that product.
5. Open the product PR (`Closes #N` on the product spec). Any
   Duhem-side wiring (a new drift lane, a new action) is its own Duhem
   PR under the Duhem spec from step 2. Cross-repo: spell out
   `owner/repo#N`, land both before either merges, chreode-style.

## Verdict surfacing on Onsager PRs

When Duhem reports a verdict on an Onsager PR, the verdict
appears as a check on the PR. The plumbing (GitHub Action,
webhook receiver, status posting) is Duhem's responsibility on
Duhem's repo (`area:integrations`). Triage when a verdict fires:

| Verdict     | Where to look first                                              | Discipline                                                                 |
|-------------|------------------------------------------------------------------|----------------------------------------------------------------------------|
| `pass`      | Nowhere — let it through.                                        | Don't comment unless the user asks.                                        |
| `fail`      | The evidence trace for the failing check.                        | Verdict is on the **web**, not a single component (per §8). Use evidence to localize, but don't claim certainty Duhem-Quine says doesn't exist. |
| `inconclusive` | The check that produced it; the environment state at run time.| `inconclusive` blocks by default per §7.6 / §9 Stage 5. Triage the cause (env flake, observability gap, real ambiguity) and either rerun, fix the env, or escalate to a human. |

A `fail` localized to "the prompt template drifted from the data
shape" or "the tool wiring lost a parameter" is exactly the
failure mode Duhem exists to surface. Capture the localization in
the Onsager-side commit / spec amendment, not as a Duhem-side
"loosen the check".

## Anti-patterns (don't)

- **Cross-posting.** Don't open one PR that touches both repos.
  Two repos, two PRs, contract in both specs.
- **Mocking Onsager's web inside a Duhem check.** Verifying
  against an Onsager mock is not dogfooding; it's still
  component testing. If the holistic environment is too
  expensive to run on every PR, that's an `area:runtime` spec
  on Duhem about environment provisioning — not a license to
  mock.
- **Onsager-specific surface in the Duhem schema.** A new action
  type whose only motivation is "Onsager needs this" is a
  warning sign. Generalize before merging — Duhem's commercial
  value depends on the schema being broadly applicable, and
  Appendix D §"What Onsager provides Duhem" warns explicitly
  against optimizing for Onsager idiosyncrasies.
- **Self-attesting on Onsager.** An Onsager PR that overrides a
  Duhem `fail` without a human override on record breaks the
  trust boundary. Don't.
- **Skipping the Onsager-side spec.** If Duhem needs Onsager to
  expose something to be verified, that's an Onsager change with
  an Onsager spec. "Just edit Onsager from the Duhem session" is
  a process violation; route the work through Onsager's own
  SDD loop.
- **Letting dogfood specs drift.** When a product's feature
  changes, the criterion may not, but the check might. Keep the
  product's `.duhem/` Verification Definitions in sync via
  **product-side** PRs (gated by the `/.duhem/` CODEOWNERS), not by
  editing them from a Duhem PR — the VD lives with the product now.
  Duhem's drift lane catches the *other* direction: a Duhem change
  that breaks the product's existing VD.

## When you don't know which side owns the work

Default to filing the spec on **the repo where the artifact lives**
(see "Repo seam" above). If the answer is "both", the answer is
"two specs". When in doubt, file on Duhem and link an Onsager-side
question — the Onsager skill on the other repo can pick it up if
the work needs to migrate.

## Relationship to other skills

| Related surface                                                                        | Role                                                                                |
|----------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------|
| [`duhem-dev-process`](../duhem-dev-process/SKILL.md)                                   | Top-level SDD loop on Duhem; this skill is invoked from its delegation map.         |
| [`issue-spec`](https://github.com/onsager-ai/dev-skills/blob/main/skills/issue-spec/SKILL.md) | Files dogfood specs (`area:dogfood`). Installed globally from `onsager-ai/dev-skills`.       |
| [`verification-authoring`](../verification-authoring/SKILL.md)                         | Writes the Verification Definitions this skill registers.                           |
| [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (global) | Manages the Duhem-side PR for a dogfood change.                                     |
| Onsager's `onsager-dev-process`, `issue-spec`, global `pr-lifecycle` (other repo)      | Manage the Onsager-side change when work spans the seam. Don't invoke from here — the Onsager session has its own loaded skills. |

## References

- `docs/duhem-spec.md` Appendix D — Why Onsager dogfoods Duhem
- `docs/duhem-spec.md` §14 — Roadmap (Onsager-Duhem milestones)
- `docs/duhem-spec.md` §8 — Holistic Verification Principle
  (no mocks, even for the dogfood)
- `docs/duhem-spec.md` §11.2 — Trust Boundary (asymmetric)
- `docs/duhem-brand.md` §3 — Relationship to Onsager (the brand-side
  framing of the same relationship)
