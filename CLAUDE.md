# Duhem

A holistic verification platform for AI-delivered software. Sits
between AI coding agents and production: captures human intent as
acceptance criteria, translates them into mechanically-judged checks
that exercise the real delivery web (code + prompts + tools + data +
runtime), and gates merge/deploy on the verdict.

> **Status:** Phase 0 — Foundation. The repo currently contains the
> product spec and brand docs only; the CLI, runtime, and judge are
> being stood up. See `docs/duhem-spec.md` §14 for the roadmap.

## What makes Duhem Duhem

These commitments define the platform's identity. A change that
weakens or contradicts any of them is an identity change and needs
explicit rationale in the spec.

- **Holistic Verification Principle.** A check exercises code + prompts
  + tool wiring + data + runtime together. We do not pretend the web
  decomposes into independently testable units, and we do not mock
  the web at verification time — not even for the dogfood. See
  `docs/duhem-spec.md` §8 / §11.2.
- **Mechanical judgment, not LLM judgment.** AI may help author
  criteria and checks; humans review them; the verdict is produced
  by deterministic evaluation of structured assertions. The judge
  has no LLM in the loop. `docs/duhem-spec.md` §7.6 / §11.2.
- **Criteria are stable; checks are derivative.** Criteria are the
  human commitment about what "done" means and survive
  implementation churn. Checks are how we verify that commitment
  and may change as the implementation does. Conflating the two is
  a defect. `docs/duhem-spec.md` §7.2 / §7.3.
- **Asymmetric trust at the dogfood seam.** The verifier of AI claims
  must be structurally independent of the AI making them. Concretely:
  Duhem authors checks against Onsager; Onsager never authors its
  own Duhem checks; an Onsager PR cannot self-attest past a Duhem
  `fail`. See `.claude/skills/onsager-dogfood/SKILL.md` and
  `docs/duhem-spec.md` §11.2.

Changes to those four bullets are spec-level changes to
`docs/duhem-spec.md` and require an explicit Alignment note.

## Reading order

1. **`docs/duhem-spec.md`** — canonical product specification. Start
   with §1 (Why), §4 (Solution Overview), §7 (Core Concepts), §8
   (Holistic Verification Principle), §11 (Architecture). Appendix D
   covers the Onsager dogfood relationship.
2. **`docs/duhem-brand.md`** — the mark, design rationale, and
   relationship to Onsager. §3 carries the visual statement of the
   sister-product relationship.
3. **`.claude/skills/`** — dev process under Claude Code. Start with
   `duhem-dev-process`; it delegates to the rest.

## Onsager — the first customer

Duhem's first dogfood customer is
[`onsager-ai/onsager`](https://github.com/onsager-ai/onsager). The
two repos move together because the same person is building both,
but they are **parallel, not shared**:

- Each repo has its own `.claude/skills/` and its own SDD loop.
- Duhem schema, CLI, runtime, judge, dashboard, integrations, and
  the Verification Definitions that exercise Onsager features all
  live on `onsager-ai/duhem`.
- Onsager's product surfaces (forge, stiglab, synodic, dashboard,
  events, migrations) live on `onsager-ai/onsager`.
- Cross-repo work is two specs (one on each repo) with a contract in
  both, never one PR straddling both repos.

Onsager is the **workload** Duhem verifies. Onsager is not (yet) the
production line that builds Duhem — that's a Phase 4+ future state
in `docs/duhem-spec.md` §14. Today, Onsager has features that need
verifying and Duhem has the verifier; the dogfood pair is how we
exercise Duhem at real complexity from day one.

When in doubt about which side of the seam owns a change, invoke
the `onsager-dogfood` skill — it has the decision rule and the
asymmetric-trust discipline written out.

## Contributing

Non-trivial changes start as a GitHub spec issue on this repo and
follow the SDD loop. The skills enforce the discipline; this file
just points at them.

| Stage                          | Skill                                    |
|--------------------------------|------------------------------------------|
| Decide what to build           | `.claude/skills/duhem-dev-process`       |
| Write the spec                 | `issue-spec` (global, from `onsager-ai/dev-skills`) |
| Author Verification Definitions| `.claude/skills/verification-authoring`  |
| Pre-push checks                | `.claude/skills/duhem-pre-push`          |
| PR triage / review / merge     | `.claude/skills/duhem-pr-lifecycle`      |
| Dogfood on Onsager             | `.claude/skills/onsager-dogfood`         |

Hard rule: **no spec, no PR**, unless the PR is labeled `trivial`
(typo, doc-only, one-line obvious fix). Schema-impacting changes
carry a `## Schema impact` callout in the spec and a `CHANGELOG.md`
entry on merge — Phase-0/1 schema-stability discipline; see
`duhem-dev-process`. The repo has no `CHANGELOG.md` yet because
there is no schema in code yet; the first schema-impact PR creates
the file.

If the change introduces new product surface (a new action type, a
new schema field, a new CLI command, new judge behavior), the spec
must include or link a worked Verification Definition example that
exercises it. A surface with no example is a surface we cannot
dogfood, which means we cannot ship it on Onsager, which means we
cannot validate it. See `verification-authoring`.

## File editing (Claude Code tools)

Prefer `Edit` over `Write` for any change to an existing file. Full
rewrites with `Write` can hit a stream idle timeout on files larger
than ~150 lines and there is no automatic retry — a stalled `Write`
silently leaves the file in its previous state or, worse,
half-written. If a rewrite is genuinely necessary, split it: write
a smaller initial version, then extend with follow-up `Edit` calls.

## Session defaults (Claude Code cloud)

If the current branch name starts with `claude/` (the prefix cloud
sessions create), treat PR creation and CI auto-fix as part of
finishing the task — do not wait to be asked:

1. Push the branch.
2. Open a pull request as ready for review (not a draft). Before
   calling `mcp__github__create_pull_request`, answer the
   spec-vs-trivial gate and bake the answer into the PR at creation
   time:
   - If a spec issue exists or you should write one, include
     `Closes #N` or `Part of #N` in the PR body.
   - If the change is genuinely `trivial` (typo, doc-only,
     formatting, one-line obvious fix — see `duhem-dev-process` for
     the full list), pass `labels: ["trivial"]` on creation.
   - Default is spec, not trivial. When in doubt, create the spec
     issue first via the `issue-spec` skill, then open the PR with
     `Closes #N`.
3. Subscribe to PR activity via
   `mcp__github__subscribe_pr_activity` so CI failures and review
   comments are auto-fixed.

Skip this for branches that don't start with `claude/`
(local/manual work).
