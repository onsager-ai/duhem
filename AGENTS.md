# Duhem

Mechanical verification of the real delivery system. Detailed repository identity,
contribution and dogfood context is in docs/agent-development-reference.md;
canonical product specification remains docs/duhem-spec.md.

## Identity invariants

- Holistic verification exercises code, prompts, tools, data and runtime together;
  do not mock the delivery web at verification time, including dogfood.
- Verdicts come from deterministic structured assertions; no LLM judges a verdict.
- Criteria are human intent and stable; checks are derivative implementations.
- Preserve independent verification and the documented asymmetric trust seam.
  Product VDs live with their products; only Duhem self-verification lives here.
- Changes to these commitments amend duhem-spec with explicit Alignment rationale.

## Change requirements

Non-trivial work starts as a spec issue here. No spec, no PR unless legitimately
trivial under duhem-dev-process. Schema-impacting work requires its Schema impact
callout and changelog; public-surface changes require DX impact and corresponding
learning surfaces. New action/schema/CLI/judge surface includes a worked VD.
Read the detailed reference's Contributing section and duhem-dev-process before
such changes; those requirements are not replaced by this summary.

## Workspace isolation

The primary checkout is main-only parking. Do not switch branches or create a
task branch there. Before editing, check git status --short --branch and git
worktree list. Start branch work with `just worktree add <branch>` and run one
session from the resulting worktree. Preserve a primary checkout found dirty or
off main exactly as found; create a separate worktree and report its condition.
Claude additionally has native branch guards; those hooks do not enforce Codex.

## Checks and completion

- `just check`: fast lint/test development loop, not the full merge gate.
- `just preflight`: committed, clean-tree merge-preview gate; origin access required.
  Do not report passed if fetch/setup fails or substitute just check.
- Browser-action changes additionally require `just test browser-actions`, installed
  sidecar tooling and Chromium in the relevant worktree.
- Dashboard changes use applicable `just dashboard` checks and dashboard CI.
- Apply modified-VD validation and all additional gates in duhem-dev-process.

Use justfile and owning workflows for exact stages. Record commands/results,
blocked checks and remaining scope accurately; warn-only and strict checks differ.

## Conditional reading and skill discovery

Find applicable module contracts before editing. Schema/judge/identity work loads
relevant duhem-spec sections. Branding work loads duhem-brand, not every task.
Development workflow loads duhem-dev-process; authoring loads verification-authoring.
Dogfood ownership loads the detailed reference and onsager-dogfood only within the
session's authorized scope. Claude's existing repo-only scope is in its adapter.

Repo skills are canonical under .agents/skills with Claude discovery projections.
Shared issue-spec/pre-push/pr-lifecycle/ci-triage still depend on existing global
installation in this pilot. Pinned checkout-local distribution is migration debt;
do not claim those workflows are available without checking the session's catalog.
Opening, updating or merging a pull request requires authority supplied by the
task or this repository's declared workflow policy. Shared procedures grant no
authority themselves. Once authorized, complete the spec/trivial decision,
relevant checks and accurate reporting before creating or updating the PR.
This resolves adoption decision D1; older global pr-lifecycle installations may
still contain the conflicting explicit-request-only rule. Update them from the
reviewed dev-skills change before relying on that workflow. Decision D2 preserves
the Claude-only session restriction exactly; no scope expansion is implied.
