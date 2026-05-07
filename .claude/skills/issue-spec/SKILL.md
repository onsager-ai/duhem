---
name: issue-spec
description: Create lean-spec style GitHub issues as specs for human-AI aligned implementation on Duhem. Use when asked to "create a spec", "write a spec issue", "spec this feature", "spec this", or when planning work that needs a specification before implementation. Follows the lean-spec SDD methodology — small focused specs (<2000 tokens), intent over implementation, context economy. Creates GitHub issues on `onsager-ai/duhem` with Overview, Design, Plan, Test, Alignment, and Notes sections. Paired with `duhem-dev-process` (the SDD loop), `duhem-pre-push` (pre-push checks), `duhem-pr-lifecycle` (post-push), and `verification-authoring` (worked examples for product-surface specs).
allowed-tools: Read, Write, Edit, Glob, Grep, Bash(git diff:*), Bash(git log:*), Bash(git show:*), mcp__github__issue_write, mcp__github__issue_read, mcp__github__list_issues, mcp__github__search_issues, mcp__github__sub_issue_write, mcp__github__get_label
---

# issue-spec

Create GitHub issues as lean-spec style specifications for human-AI
aligned implementation on Duhem (`onsager-ai/duhem`). GitHub issues
are the sole spec medium — no spec files.

This is Duhem's analogue of Onsager's `issue-spec` skill. The
discipline is the same; the area taxonomy and the architectural
invariants are different (Duhem doesn't have Onsager's seam rule —
it has its own product-surface invariants instead, see below).

## Why GitHub Issues, Not Files

lean-spec uses Markdown files with YAML frontmatter for metadata. We
replace that entirely with GitHub issues because:

- **Status** → Issue state (open/closed) + status labels (`draft`,
  `planned`, `in-progress`)
- **Priority** → Labels (`priority:critical`, `priority:high`,
  `priority:medium`, `priority:low`)
- **Tags** → Labels (`area:*`, `feat`, `fix`, `refactor`, `perf`)
- **Dependencies** → Issue references (`depends on #42`) and
  sub-issues
- **Parent/Child** → Sub-issues via `mcp__github__sub_issue_write`
- **Transitions** → Issue timeline (automatic, auditable)
- **Collaboration** → Comments, reactions, assignments, mentions

GitHub gives us versioned metadata, collaboration, and relationship
tracking for free. No CLI needed, no frontmatter to manage, no sync
problems.

## Philosophy

Three principles from lean-spec:

1. **Context Economy** — Keep issue body under ~2000 tokens. Larger
   features split into parent + child issues. Small specs produce
   better AI output and better human review.
2. **Intent Over Implementation** — Document the *why* and *what*,
   not the *how*. Implementation details belong in PRs, not spec
   issues. The spec captures human intent that isn't in the code.
3. **Living Documents** — Specs evolve via issue comments and edits.
   Status labels track lifecycle. The issue thread becomes the
   decision record.

Plus two Duhem-specific principles:

4. **A product-surface spec ships with a worked example.** Any spec
   that introduces or changes user-visible Duhem surface — a new
   action type (`uses:`), a new schema field, a new CLI command, a
   new judge behavior, a new assertion form — must include or link a
   minimal Verification Definition that exercises the surface
   end-to-end. Without it, the spec describes a feature we cannot
   dogfood through `onsager-dogfood`, which means we cannot
   actually verify it works in the same loop our customers will use.
   See `verification-authoring` for the template.

5. **Schema impact is a first-class section.** While the schema is
   pre-1.0 (Phase 0 / Phase 1 of the roadmap in `docs/duhem-spec.md`
   §14), every spec that touches the Verification Definition format
   declares the impact in `## Schema impact`. This feeds the
   breaking-change-rate signal that gates the Phase 2 schema OSS
   trigger (open question in §15). Skipping the section corrupts
   that signal.

## When to use this skill

Use when:

- A change touches multiple files or areas.
- Multiple stakeholders need alignment before implementation.
- The AI needs explicit boundaries for a non-trivial feature.
- Work will span multiple PRs (parent + child specs).
- The change touches the schema, the judge, or any externally
  observable contract — those are *always* spec-worthy regardless of
  diff size.

Skip when:

- A typo or doc-only fix. Use the `trivial` label on the PR instead.
- A one-line bug fix with an obvious reproduction. Just open a PR
  with `Fixes #existing`.
- The feature already has a spec issue — extend that spec, don't
  create another.

**Default is spec, not trivial.** If invocation of this skill is
itself the decision — the user said "spec this" or the change
clearly isn't a typo/one-liner — proceed straight to Discover. Do
not stop to confirm spec-vs-`trivial`.

## Workflow

```
1. Discover     Search existing issues and codebase
2. Design       Draft the spec issue body
3. Align        Partition human decisions vs AI work
4. Validate     Self-check before creating
5. Publish      Create GitHub issue (+ sub-issues if splitting)
```

### 1. Discover

Before writing anything:

- Search existing issues on `onsager-ai/duhem` for related or
  duplicate specs.
- Read `docs/duhem-spec.md` and `docs/duhem-brand.md` to ground the
  spec in Duhem's existing commitments. The spec doc is canonical
  for what Duhem is and isn't — disagreement between a new spec
  and the spec doc must be resolved explicitly (either the new
  spec changes `docs/duhem-spec.md` in the same PR, or the new
  spec narrows itself to fit).
- Grep the codebase (when there is one) for types, modules, files
  related to the topic.
- Check git log for recent changes in the area.

If a related spec issue already exists, reference it — don't
duplicate.

### 2. Design

Read [references/spec-format.md](references/spec-format.md) for the
section-by-section format guide.

Draft the issue body using the lean-spec structure:

```markdown
## Overview
Problem statement and motivation. Why does this matter?

## Design
Technical approach: data flow, schema changes, architecture
decisions. Keep it high-level — intent, not implementation.

## Plan
- [ ] Checklist of concrete deliverables
- [ ] Each item independently verifiable
- [ ] Order reflects implementation sequence

## Test
- [ ] How to verify each plan item
- [ ] Include: unit tests, integration tests, schema validation,
      manual checks

## Schema impact
<!-- omit only if the change provably touches no schema surface -->
- Fields added/removed/renamed
- Semantics changed
- Migration path for in-flight Verification Definitions

## Notes
Tradeoffs, context, references. Optional — omit if empty.
```

**Context economy check**: If the issue body exceeds ~2000 tokens,
split it:

- Create a parent issue with Overview + high-level Plan
- Create child issues (sub-issues), one per independent concern
- Each child has its own Design, Plan, Test sections
- Link children to parent via `mcp__github__sub_issue_write`

### 3. Align

Add an **Alignment** section to the issue body:

```markdown
## Alignment

### Human decides
- [ ] Architectural tradeoffs, scope, schema shape, go/no-go

### AI implements
- [ ] Concrete code tasks tied to Plan items

### Open questions
> Items that block AI implementation until a human decides
```

**Rules:**

- Every Plan item maps to either "Human decides" or "AI implements"
- If an item requires both, split it — the decision part is human,
  the execution is AI
- Open questions use `>` blockquotes so they're visually distinct
- Once a human answers a question (via issue comment), update the
  Alignment section

### 4. Validate

Before creating the issue, self-check:

- [ ] Body is under ~2000 tokens (context economy)
- [ ] Overview explains *why*, not just *what*
- [ ] Design captures intent, not implementation details
- [ ] Plan items are concrete and independently verifiable
- [ ] Test items map to Plan items
- [ ] `## Schema impact` section is present (or the spec
      provably touches no schema surface)
- [ ] If this introduces product surface, a worked Verification
      Definition example is included or linked
- [ ] Human/AI boundaries are explicit — no "figure it out" items
- [ ] No duplicate of an existing issue
- [ ] Dependencies are referenced by issue number

### 5. Publish

Create the issue using `mcp__github__issue_write` against
`onsager-ai/duhem`:

**Title format**: `spec(<area>): <short description>`

Examples:

- `spec(schema): add api/observe action type`
- `spec(judge): three-state verdict aggregation rules`
- `spec(cli): duhem run accepts --filter`
- `spec(docs): clarify holistic-environment principle`

**Labels**: Apply via the issue creation:

- `spec` — always
- Type: `feat`, `fix`, `refactor`, `perf`
- Area (see taxonomy below)
- Priority: `priority:critical`, `priority:high`,
  `priority:medium`, `priority:low`
- Status: `draft` (initial state)
- `schema-impact` — if `## Schema impact` is non-trivial; this
  feeds the breaking-change-rate signal

**Sub-issues**: If this is a child of a parent spec, link it using
`mcp__github__sub_issue_write`.

**After creating**, report to the user:

- Issue number and URL
- Token count estimate (flag if over 2000)
- Any open questions that need human decisions
- Sub-issue links if the spec was split

## Area label taxonomy (Duhem repo)

Pick one or more. Until Duhem grows multiple distinct subsystems,
the taxonomy is intentionally flat — based on the components in
`docs/duhem-spec.md` §11, plus a couple of cross-cutting buckets:

- `area:schema` — Verification Definition format, action-type
  catalog, runtime expressions, validator
- `area:cli` — `duhem` CLI binary (init, run, validate)
- `area:runtime` — check executor, environment provisioning
- `area:judge` — deterministic assertion evaluator, verdict
  aggregation rules
- `area:generation` — AI-powered criteria → checks translation
- `area:dashboard` — web UI for runs, evidence, verdicts
- `area:integrations` — GitHub Action, MCP server, IDE extension
- `area:evidence` — append-only evidence store, replay
- `area:dogfood` — Verification Definitions that target
  `onsager-ai/onsager` (see `onsager-dogfood`)
- `area:infra` — CI, build tooling, repo hygiene
- `area:docs` — README, `docs/duhem-spec.md`, brand docs

A spec that legitimately spans two areas should be split unless the
change is genuinely a single contract crossing the seam (e.g. an
action-type addition lands in `area:schema` plus `area:runtime`
together — that's one spec, two area labels).

## Status Lifecycle via Labels

```
open + draft  →  open + planned  →  open + in-progress  →  closed
```

- **draft**: Spec created, open questions may remain. AI wrote it,
  human hasn't reviewed.
- **planned**: Human reviewed, decisions made, ready for
  implementation. Remove `draft`, add `planned`.
- **in-progress**: Someone/something is actively working (PR
  opened). Remove `planned`, add `in-progress`. *Currently
  manual* on this repo — no `pr-spec-sync` workflow yet (see
  `duhem-pr-lifecycle`).
- **closed**: All plan items done, tests passing. PR merge with
  `Closes #N` closes it automatically.

**Key rule**: `draft → planned` is the human-AI alignment gate. The
AI does not flip this label unprompted.

## Spec Relationships via Sub-Issues

| Relationship    | GitHub mechanism                                        | When to use                                        |
|-----------------|---------------------------------------------------------|----------------------------------------------------|
| **Parent/Child**| Sub-issues (`mcp__github__sub_issue_write`)             | Large feature decomposed into pieces               |
| **Depends On**  | Issue body reference (`depends on #N`)                  | Spec blocked until another finishes                |
| **Related**     | Issue body reference (`related: #N`)                    | Loosely connected specs                            |

**Decision rule**: Remove the dependency — does the spec still make
sense? If no → sub-issue (child). If yes but blocked → depends on.

**Example decomposition:**

```
spec(schema): action-type catalog v1            ← parent issue
├── spec(schema): ui/* action types              ← sub-issue
├── spec(schema): api/* action types             ← sub-issue
├── spec(schema): db/* action types              ← sub-issue
└── spec(schema): event/* action types           ← sub-issue
```

## Guidance

- **Small is better.** A 500-token spec that captures intent
  clearly beats a 3000-token spec that tries to cover everything.
  Split into sub-issues early.
- **Discover first.** Always search existing issues before
  creating. Duplicate specs create confusion.
- **Status labels reflect reality.** Don't label `planned` if
  decisions are still open. Don't label `in-progress` until a PR is
  open.
- **One concern per issue.** If a spec covers two independent
  changes, split into sub-issues with a shared parent.
- **Reference code, not concepts.** Once code exists, point to
  actual types, functions, files — not abstract ideas.
- **Open questions are alignment points.** These are where AI must
  stop and ask a human. Make them explicit, specific, and include
  the impact of each decision.
- **Comments are the decision record.** When a human resolves an
  open question, they comment on the issue. The thread becomes the
  audit trail.
- **Use specs for alignment, not for everything.** Regular bugs and
  small tasks don't need specs. Use specs when: multiple
  stakeholders need alignment, intent needs persistence, or the AI
  needs clear boundaries.

## Handoff to implementation

Once a spec moves to `planned`:

1. Create a branch referencing the issue: `claude/spec-<N>-<slug>`
   or similar.
2. Follow the SDD loop in `duhem-dev-process`.
3. Pre-push via `duhem-pre-push` (includes a spec-link check).
4. PR body must include `Closes #N` (slice complete) or
   `Part of #N` (scaffolding).
5. After merge, see `duhem-pr-lifecycle` for Plan-item ticks and
   parent-spec maintenance.

## Relationship to Onsager's `issue-spec`

This skill and `onsager-ai/onsager`'s `issue-spec` skill are
**parallel, not shared**. Each is scoped to its own repo, with its
own area taxonomy and architectural invariants. A spec for a
Duhem-on-Onsager dogfood Verification Definition lives on
`onsager-ai/duhem` (because the artifact being changed is a Duhem
Verification Definition); a spec for an Onsager-side change that
changes how Onsager surfaces Duhem verdicts lives on
`onsager-ai/onsager`. See `onsager-dogfood` for which side of the
seam to put cross-repo work on.

## References

| Reference                                          | When to read                                |
|----------------------------------------------------|---------------------------------------------|
| [references/spec-format.md](references/spec-format.md) | Always — section-by-section guide           |

## Templates

| Template                                                | Purpose                                |
|---------------------------------------------------------|----------------------------------------|
| [templates/issue-spec-template.md](templates/issue-spec-template.md) | Issue body template — copy and fill |
