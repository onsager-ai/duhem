---
name: onsager-dogfood
description: Run Duhem against onsager-ai/onsager — Duhem's first dogfood customer. Use when authoring or maintaining Verification Definitions that target the Onsager dashboard / API / events, when wiring Duhem verdicts into Onsager's PR checks, when triaging a verdict that fired on an Onsager PR, or when you're unsure which side of the Duhem/Onsager seam owns a given change. Covers the cross-repo discipline (which spec lives where, which PR opens where), the asymmetric trust boundary (Duhem authors checks; Onsager only consumes verdicts), and the milestones in `docs/duhem-spec.md` Appendix D / §14. Triggers include "dogfood on Onsager", "verify against Onsager", "Onsager PR check", "Duhem on Onsager", "cross-repo spec", "which repo owns this", "Onsager-Duhem boundary".
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
| Verification Definitions that exercise Onsager features       | `onsager-ai/duhem`        | `verification-authoring` (`area:dogfood`)                          |
| Onsager features themselves (forge, stiglab, synodic, dashboard, etc.) | `onsager-ai/onsager` | Onsager's `onsager-dev-process`, `issue-spec`, `dashboard-ui`, etc. |
| The GitHub Action / webhook that surfaces Duhem verdicts on Onsager PRs | `onsager-ai/onsager` (consumer side) + `onsager-ai/duhem` (publisher side) | Onsager-side: `onsager-pr-lifecycle`. Duhem-side: `duhem-pr-lifecycle` + this skill. |
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

## Asymmetric trust boundary

Duhem-Quine has a structural consequence for dogfooding: **the
verifier of AI claims must be structurally independent of the AI
making them.** That principle binds the Duhem-Onsager pair too,
and asymmetrically:

- **Duhem authors checks against Onsager.** AI assistance writing
  these checks is fine; humans review them; once frozen they are
  frozen (`docs/duhem-spec.md` §7.3, §11.2).
- **Onsager does not author its own Duhem checks.** Even though
  the same human builds both, an Onsager-side PR cannot
  unilaterally relax or override a verdict on itself. The
  `area:dogfood` Verification Definitions are Duhem artifacts, not
  Onsager artifacts; their PRs go through Duhem's review.
- **Onsager cannot self-attest pass.** A `fail` verdict on an
  Onsager PR cannot be overridden by another commit on the same
  PR. Override requires explicit human action with an audit trail
  (per §9 Stage 5), exactly as it would for any external customer.

If you find yourself reaching for "I'll just relax the check from
the Onsager side this once", **stop**. That's the trust boundary
you'd refuse to break for a paying customer; the same applies to
the dogfood. Resolve the failure, escalate the verdict, or update
the criterion explicitly via a Duhem spec — never bypass.

## Authoring a dogfood Verification Definition

When the trigger is "verify <Onsager feature> with Duhem":

1. Confirm the Onsager feature you're verifying. Read its
   acceptance criteria (the Onsager spec issue's `## Test`
   section is the right starting point — Onsager and Duhem agree
   on the lean-spec format, and the criteria there should be
   shaped exactly like Duhem criteria already).
2. File a Duhem spec issue (via `issue-spec`) with
   `area:dogfood` and a title like `spec(dogfood): verify
   <onsager-feature>`. Body links the Onsager spec by URL (cross-
   repo: spell out `onsager-ai/onsager#N`, not just `#N`).
3. Author the Verification Definition under `verification-authoring`.
   Inputs include the staging URL of the Onsager environment, a
   test user fixture, and any data preconditions. The
   environment is **Onsager's real staging**, with the real
   Postgres event spine, the real dashboard, the real subsystem
   binaries — not a mock. Per §8 / §11.2, Duhem is exercising
   the actual web.
4. Open the Duhem PR with `Closes #N` against the Duhem dogfood
   spec. Link the Onsager spec for context.
5. Onsager's side, separately: ensure the Onsager environment
   exposes whatever observation seam the Duhem check needs (e.g.
   a webhook is reachable, the test user exists, a feature flag
   is on in staging). That's an Onsager spec, on the Onsager
   repo, owned by Onsager's `issue-spec` skill — **not** this one.

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
- **Letting dogfood specs drift.** When Onsager's feature
  changes, the criterion may not, but the check might. Keep the
  Duhem `area:dogfood` Verification Definitions in sync via
  Duhem-side PRs; do not edit them from Onsager-side PRs.

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
| [`duhem-pr-lifecycle`](../duhem-pr-lifecycle/SKILL.md)                                 | Manages the Duhem-side PR for a dogfood change.                                     |
| Onsager's `onsager-dev-process`, `issue-spec`, `onsager-pr-lifecycle` (other repo)     | Manage the Onsager-side change when work spans the seam. Don't invoke from here — the Onsager session has its own loaded skills. |

## References

- `docs/duhem-spec.md` Appendix D — Why Onsager dogfoods Duhem
- `docs/duhem-spec.md` §14 — Roadmap (Onsager-Duhem milestones)
- `docs/duhem-spec.md` §8 — Holistic Verification Principle
  (no mocks, even for the dogfood)
- `docs/duhem-spec.md` §11.2 — Trust Boundary (asymmetric)
- `docs/duhem-brand.md` §3 — Relationship to Onsager (the brand-side
  framing of the same relationship)
