# Spec Format Reference

Section-by-section guide for writing lean-spec style GitHub issue specs on `onsager-ai/duhem`. Based on the [lean-spec SDD methodology](https://github.com/codervisor/lean-spec), adapted to use GitHub issues as the sole spec medium and to fit Duhem's product invariants.

## Metadata via GitHub Issue Features

No YAML frontmatter — all metadata lives in native GitHub features:

| lean-spec field | GitHub equivalent     | Example                                  |
|-----------------|-----------------------|------------------------------------------|
| `status`        | Labels                | `draft`, `planned`, `in-progress`        |
| `priority`      | Labels                | `priority:high`                          |
| `tags`          | Labels                | `area:schema`, `feat`                    |
| `depends_on`    | Issue body reference  | `depends on #42`                         |
| `parent/child`  | Sub-issues            | Created via `mcp__github__sub_issue_write` |
| `assignee`      | Issue assignee        | `@username`                              |
| `created/updated` | Issue timestamps    | Automatic                                |
| `transitions`  | Issue timeline         | Automatic audit trail                    |

### Label Taxonomy

Apply labels when creating the issue:

**Required:**

- `spec` — marks this as a spec issue (always present)

**Type** (pick one):

- `feat` — new capability
- `fix` — bug fix
- `refactor` — restructuring without behavior change
- `perf` — performance improvement

**Area** (pick one or more — see SKILL.md "Area label taxonomy"):

- `area:schema`, `area:cli`, `area:runtime`, `area:judge`, `area:generation`, `area:dashboard`, `area:integrations`, `area:evidence`, `area:dogfood`, `area:infra`, `area:docs`

**Priority** (pick one):

- `priority:critical` — blocks other work, needs immediate attention
- `priority:high` — important, should be next
- `priority:medium` — default
- `priority:low` — nice to have

**Status** (pick one, update as lifecycle progresses):

- `draft` — initial state, AI-generated, human review pending
- `planned` — human reviewed, decisions made, ready for implementation
- `in-progress` — actively being worked on (PR open)

**Cross-cutting:**

- `schema-impact` — apply whenever the spec includes a non-empty `## Schema impact` section. That covers *any* change to the Verification Definition format, action-type catalog, runtime expressions, or judge semantics — not just breaking ones. The label is the discoverability signal for "which specs touch schema surface?". Breaking-vs-non-breaking is tracked inside the section via `Breaking change? yes/no` and feeds CHANGELOG; the filter "schema-impact specs with `Breaking change? yes`" is what gates the Phase 2 schema OSS decision (`docs/duhem-spec.md` §15).
- `trivial` (PR-only label, not a spec label) — opts a PR out of the spec-link requirement. See `duhem-dev-process`.

### Status Lifecycle

```
open + draft  →  open + planned  →  open + in-progress  →  closed
```

The `draft → planned` transition is the **human-AI alignment gate**. Only a human moves a spec to `planned` — this confirms:

- Open questions are resolved
- Design approach is approved
- Scope and priority are accepted

`planned → in-progress` happens **manually on this repo** when a PR referencing the issue opens. (Onsager has automation for this; Duhem does not yet.) See `duhem-pr-lifecycle`.

`in-progress → closed` happens automatically on PR merge with a `Closes #N` keyword. `Part of #N` PRs don't close the parent; the merger ticks the parent's Plan checkboxes manually.

## Sections

### Overview

**Purpose**: Why does this work matter? What problem does it solve?

**Good overview** (note: prose, list items, and blockquote lines are *not* hard-wrapped — GitHub renders single newlines as `<br>` in issue bodies; see SKILL.md step 2):

```markdown
## Overview

Today, `duhem run` accepts a single Verification Definition file as its only input. Teams using Pattern B (per-feature directories) need to invoke `duhem run` against a glob, which loops in the shell — slow and breaks shared environment provisioning.

This adds a `--filter` flag that selects a subset of verifications declared in `duhem.yml` by name pattern, so a team can run a focused subset without bypassing the manifest's shared defaults.
```

**Bad overview:** describes the solution before the problem; spends two paragraphs on context already in `docs/duhem-spec.md`; mentions a specific function name.

### Design

**Purpose**: Capture intent, not implementation. A reader should understand the *shape* of the change without reading the diff.

Cover:

- Data flow (or YAML flow) at intent level
- Schema changes (link to `docs/duhem-spec.md` sections)
- Judge contract changes
- Out-of-scope: what this spec deliberately doesn't do

Don't:

- Quote 50 lines of code
- Specify exact function signatures (those go in the PR)
- Describe how to write the test (that's `## Test`)

### Plan

A checklist of concrete deliverables. Each item:

- Starts with a verb
- Is independently verifiable
- Has a single owner (AI or human, not both)
- Order reflects implementation sequence (top-down dependencies)

A 2–6 item Plan is healthy. A Plan with 12 items is two specs hiding in a trench coat — split it.

### Test

How each Plan item is verified. Map 1:1 to Plan items where possible. Include:

- Unit tests (file path / function name)
- Integration tests
- Schema validation (the validator should accept the new shape and reject older malformed shapes)
- Manual checks (only when automation is genuinely impossible)
- A worked Verification Definition (when this is a product-surface change — see SKILL.md Philosophy #4)

### Schema impact

Required when the change touches the Verification Definition format, action-type catalog, runtime expressions, judge semantics, or any externally observable contract.

Format:

```markdown
## Schema impact

- Fields added: `verification.continue_on_failure: bool`, default `false`
- Fields renamed: none
- Fields removed: none
- Semantics changed: `setup:` block now runs once per criterion instead of once per verification (was undocumented; ratifying observed behavior)
- Migration path: existing verifications continue to work; field is optional. No tooling migration needed.
- Breaking change? no
```

Apply the `schema-impact` label whenever this section is present on the spec (and mirror it onto the PR). Whether the change is breaking is a separate signal answered by the `Breaking change?` line. If `Breaking change? yes`, also add a CHANGELOG entry on merge. Pre-1.0 (Phase 0/1), breaking changes are allowed but counted — the filter "schema-impact specs with `Breaking change? yes`" is the rate-of-change signal that gates Phase 2 schema OSS.

### Alignment

Three sub-sections:

#### Human decides

Decisions requiring judgment, scope authority, or domain knowledge the AI doesn't have. Examples:

- Which of two schema shapes to commit to
- Whether a behavior is in scope for this spec or a follow-up
- Whether a breaking change is acceptable now vs deferred

#### AI implements

Concrete code tasks tied to Plan items. Examples:

- "Add `--filter` flag handling in `duhem-cli/src/run.rs`"
- "Update validator schema to accept `continue_on_failure`"

#### Open questions

`>` blockquoted questions that block `draft → planned`. Each must:

- State the question
- Note the impact (which Plan items are affected)
- Include enough context that a human can answer without rereading the whole spec

### Worked example

Required when the spec introduces or modifies user-visible product surface. A minimal Verification Definition demonstrating the change. See `verification-authoring` for the template.

### Notes

Optional. Tradeoffs, related issues, prior art. Omit if empty.

## Sub-issue decomposition

When a spec exceeds ~2000 tokens or covers more than one independent concern, split it into a parent + child specs:

```
spec(schema): action-type catalog v1            ← parent
├── spec(schema): ui/* action types              ← child sub-issue
├── spec(schema): api/* action types             ← child sub-issue
├── spec(schema): db/* action types              ← child sub-issue
└── spec(schema): event/* action types           ← child sub-issue
```

The parent spec carries the overarching intent and a high-level Plan that's a list of the children. Each child is independently implementable and has its own Design, Plan, Test, Schema impact, Alignment.

Use `mcp__github__sub_issue_write` to attach children to the parent.

## Cross-references

- `docs/duhem-spec.md` — canonical product spec; cite section numbers when grounding a spec issue
- `docs/duhem-brand.md` — brand mark, design discipline (rarely relevant to engineering specs)
- Onsager's spec for the same domain (when work spans the dogfood seam) — see the `onsager-dogfood` skill for guidance on which side of the seam owns the spec
