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

Use justfile and owning workflows for exact stages; warn-only and strict checks differ.

## Conditional reading and skill discovery

Schema/judge/identity work loads
relevant duhem-spec sections. Branding work loads duhem-brand, not every task.
Development workflow loads duhem-dev-process; authoring loads verification-authoring.
Dogfood ownership loads the detailed reference and onsager-dogfood only within the
session's authorized scope. Claude's existing repo-only scope is in its adapter.

Local workflow overlays remain in the repo-owned skills. The Claude-only
session restriction remains exactly in CLAUDE.md. Common publication authority,
reporting and discovery conventions are generated below from the pinned shared
source. The manifest identifies selected shared and repo-owned skill names.

For native capability mapping, use harness-operations; Codex-specific reference
routing is in .agents/adapters/codex.md. These adapters grant no extra scope.

<!-- agent-config:begin -->
## Shared agent conventions (generated)

- **authority:** Opening, updating or merging a pull request requires authority from the task or declared repository policy. Shared procedures grant no authority themselves; opening or updating authority does not authorize merging.
- **checks:** Run checks appropriate to the affected behavior. Report commands, actual results, blocked prerequisites and remaining scope. A quick check does not replace a declared merge gate.
- **discovery:** Before editing a module, locate applicable ancestor/module instruction files and load only relevant references. Shared workflows and their dependencies are checked in under .agents/skills; Claude discovery copies are generated under .claude/skills.
- **ownership:** Edit repo-owned contracts and local skills at their canonical paths. Shared skills, Claude projections, this managed section and synchronization tooling are generated: change the upstream source or manifest selection and regenerate; do not hand-edit generated copies.
- **workflow-policy:** Repository policy owns contribution process, scope and gates; shared methods do not impose an SDD spec requirement where the repo has none. Native tool names in shared workflows illustrate operations: use equivalent available connected tools, and report unavailable capabilities. No global skill install or personal MCP setup is required for discovery.
<!-- agent-config:end -->
