---
name: duhem-dev-process
description: The end-to-end spec-issue-driven dev loop for Duhem — spec → branch → implement → PR → merge → closure. Use when asked "how do I start work", "what's the process", "SDD loop", "spec-driven development", "how do we ship a change on Duhem", "from scratch what do I do", or when you're about to begin a non-trivial change on the Duhem repo and haven't yet decided how to split spec/PR. Delegates to `issue-spec` (spec writing), `duhem-pre-push` (pre-push checks), `duhem-pr-lifecycle` (post-push), `verification-authoring` (authoring Verification Definitions for the platform itself), and `onsager-dogfood` (running Duhem against the Onsager repo).
---

# duhem-dev-process

The spec-issue-driven development (SDD) loop on Duhem. Every non-trivial
change starts as a GitHub spec issue on `onsager-ai/duhem`, proceeds
through a PR that references it, and closes when the PR merges.

Duhem is in **Phase 0 — Foundation** (per `docs/duhem-spec.md` §14). The
repo currently contains spec docs only; tooling (CLI, runtime, judge)
is being stood up. The dev loop below is intentionally lean — it
mirrors the discipline used on `onsager-ai/onsager`, but does not
inherit Onsager's seam rule, area taxonomy, or Rust toolchain checks.
Add those skill sections only when the corresponding code lands.

## The loop

```
     ┌─────────────────────────────────────────────────────────────────┐
     │                                                                 │
     │   idea/request                                                  │
     │        ↓                                                        │
     │   spec(<area>): ...    ← issue-spec skill                       │
     │        │                                                        │
     │        │ label: draft                                           │
     │        ↓                                                        │
     │   human review   ← alignment gate (human sets label=planned)    │
     │        │                                                        │
     │        │ label: planned                                         │
     │        ↓                                                        │
     │   branch + implement                                            │
     │        │                                                        │
     │        ↓                                                        │
     │   duhem-pre-push skill  ← merge preview, build/test (when wired)│
     │        │                                                        │
     │        ↓                                                        │
     │   git push → open PR (body: "Closes #N" or "Part of #N")        │
     │        │                                                        │
     │        ↓                                                        │
     │   duhem-pr-lifecycle skill  ← CI triage, review, iterate        │
     │        │                                                        │
     │        ↓                                                        │
     │   merge                                                         │
     │        │                                                        │
     │        │ Closes #N → GitHub auto-closes spec                    │
     │        │ Part of #N → tick Plan items manually (pr-lifecycle)   │
     │        ↓                                                        │
     │   spec closed (Closes) OR Plan items ticked (Part of)           │
     │                                                                 │
     └─────────────────────────────────────────────────────────────────┘
```

This is the same shape as Onsager's loop. The two products share an
SDD discipline by design: Duhem is built using Claude Code under the
same process discipline it will eventually verify (see `onsager-dogfood`).

## Stages

### 1. Write the spec

Trigger `issue-spec` (or say "spec this"). It creates a GitHub issue
on `onsager-ai/duhem` with:

- `## Overview`, `## Design`, `## Plan`, `## Test`, `## Alignment`, `## Notes`
- Labels: `spec`, one type (`feat` / `fix` / `refactor` / `perf`),
  one or more `area:*`, one `priority:*`, status `draft`. The full
  area taxonomy lives in `issue-spec`'s SKILL.md and
  `references/spec-format.md`.

Hard rule: no spec → no PR, unless the PR is labeled `trivial` (typos,
doc-only fixes, one-line obvious bug repair).

Body size: <~2000 tokens. Larger features split into parent + sub-issues
via `mcp__github__sub_issue_write`. The SDD loop runs independently on
each sub-issue; the parent tracks overall progress.

If the spec proposes new **product surface** (new action type, new
schema field, new CLI command, new judge behavior), the spec must
either include or link a worked Verification Definition example
showing how the surface is exercised — see `verification-authoring`.
A surface that has no example by the time the spec moves to
`planned` is a surface we cannot dogfood, which means we cannot ship
it on Onsager, which means we cannot validate it. Skip this only
for purely internal scaffolding (build configuration, repo
hygiene).

### 2. The alignment gate (`draft → planned`)

Only a human moves the `draft` label to `planned`. This signals:

- Open questions resolved (answered via comments; Alignment section
  updated).
- Design approach approved.
- Scope and priority accepted.

Never bypass this gate automatically. An AI may draft the spec and
propose the flip; it may not execute the flip.

### 3. Branch and implement

Branch naming convention:

- Human-owned branches: any name.
- Claude-owned branches: `claude/spec-<N>-<slug>` or
  `claude/<descriptor>`. The harness enforces the `claude/` prefix
  on cloud sessions.

Implement the spec's Plan items in order. Keep commits small and
focused. Commit messages should be imperative and under 72 chars.

**Schema-stability discipline.** While the schema is in pre-1.0
iteration (Phase 0 / Phase 1), every change to the Verification
Definition format — fields added, renamed, removed, semantics
shifted — must be flagged in the spec under a `## Schema impact`
subsection. The CHANGELOG entry on merge calls it out. Once the
schema is OSS'd in Phase 2, this hardens into a formal deprecation
policy; until then the discipline is informal but tracked.

### 4. Pre-push

Trigger `duhem-pre-push` (or say "ready to push"). At Phase 0 the
checklist is short:

1. Sync `origin/main` into the branch (CI tests a merge preview, not
   the branch alone). Resolve conflicts locally, never on the PR
   web editor.
2. Run whatever build/test exists in the repo at the time. If
   nothing is wired yet, the step is a no-op — it grows as the CLI,
   schema validator, and judge land.
3. Verify a spec issue is linked, or that the PR will be labeled
   `trivial`.

Don't paper over warnings with `--no-verify`. If a hook fails,
investigate.

### 5. Open the PR

PR body must begin with a linking line:

| PR delivers                                         | Use            |
| --------------------------------------------------- | -------------- |
| The full spec / acceptance test / vertical slice    | `Closes #N`    |
| A bug fix for a specific defect                     | `Fixes #N`     |
| Scaffolding / one phase of a multi-phase spec       | `Part of #N`   |
| Related work that shouldn't close the spec          | `Refs #N`      |

Under `## Delivers`, list the Plan items this PR ticks (exact text
from the spec's Plan). After merge, tick those checkboxes manually
on the parent spec — see `duhem-pr-lifecycle`.

If the PR is genuinely trivial (typo, doc-only, one-line obvious
fix), apply the `trivial` label and skip the spec-linking
requirement. Use sparingly — if reviewers flag it as needing
context, escalate to a spec.

**Decide before opening, not after.** Answer the spec-vs-trivial
gate at PR creation: pass `Closes #N` / `Part of #N` in the PR
body, or pass `labels: ["trivial"]` to
`mcp__github__create_pull_request`. Don't push and let a reviewer
ask.

### 6. During review

Trigger `duhem-pr-lifecycle` (or say "triage PR" / "CI is failing"
/ respond to a webhook). It covers:

- CI triage: build / test / schema-validation failures.
- Review-comment discipline: fix the code, don't reply per comment.
- Webhook subscription to stream CI + review events.

### 7. Merge

- `Closes #N` PRs auto-close the spec on merge.
- `Part of #N` / `Refs #N` PRs leave the spec open; tick the
  delivered Plan items manually on the parent spec, and if all
  sub-issues of a parent are closed, ping the parent. See
  `duhem-pr-lifecycle`.
- For schema-impacting PRs, also append the change to `CHANGELOG.md`
  under the next-version heading.

### 8. Closed-unmerged path

If you close a PR without merging (e.g. abandoned approach), check
whether any other PR still references the spec. If none, flip the
spec back to `planned` so the next implementer can pick it up. (No
`pr-spec-sync` workflow yet on this repo — until one lands, this
flip is manual.)

## The `trivial` escape hatch

Not every change needs a spec. The `trivial` label on a PR
explicitly opts out. Use for:

- Typos in comments, docs, commit messages.
- One-line obvious bug fixes where the repro is in the diff itself.
- Formatting-only changes.
- Dependency version bumps (unless they break APIs).

Do NOT use for:

- Anything touching multiple files.
- Anything that changes `docs/duhem-spec.md` semantics (schema,
  judge contract, source-posture).
- Anything that could plausibly merit a follow-up.

When in doubt, write the spec.

## Issue progress is the source of truth

The labels on a spec issue must reflect reality at all times:

| Label         | Meaning                                          |
|---------------|--------------------------------------------------|
| `draft`       | AI-drafted or human-drafted, human review pending. |
| `planned`     | Ready for implementation. Preconditions met.     |
| `in-progress` | At least one PR is open against this spec.       |
| (closed)      | All Plan items delivered, spec closed.           |

Until automation lands on this repo, the `planned → in-progress`
flip on PR open is **manual** — `duhem-pr-lifecycle` covers the
mechanics.

## Anti-patterns (don't)

- **PR without a spec and no `trivial` label.** Reviewers will ask;
  the PR should not merge until the author either adds a spec link
  or the `trivial` label.
- **Moving `draft → planned` as the AI.** Human-only transition.
- **Closing a spec manually when you meant `Closes #N`.** Let GitHub
  do it via the PR merge so the timeline has the auditable link.
- **Editing Plan checkboxes to mark items done before the PR
  merges.** Tick them on merge, not before.
- **Schema change without `## Schema impact` callout.** The pre-1.0
  schema's breaking-change rate is what determines when we OSS the
  spec; mis-tracking a change skews that signal.
- **Skipping `duhem-pre-push`.** Even a thin checklist catches the
  cheap mistakes.
- **Shipping a CLI / schema feature without a worked example.** A
  feature that has no Verification Definition demonstrating it is
  a feature we cannot dogfood, full stop. See
  `verification-authoring`.

## Delegation map

| Stage                                       | Skill / workflow                                                |
|---------------------------------------------|-----------------------------------------------------------------|
| Write the spec                              | [`issue-spec`](../issue-spec/SKILL.md)                          |
| Pre-push checks                             | [`duhem-pre-push`](../duhem-pre-push/SKILL.md)                  |
| CI triage, review, iterate                  | [`duhem-pr-lifecycle`](../duhem-pr-lifecycle/SKILL.md)          |
| On PR merge → tick Plan items               | [`duhem-pr-lifecycle`](../duhem-pr-lifecycle/SKILL.md) (manual) |
| Author Verification Definitions             | [`verification-authoring`](../verification-authoring/SKILL.md)  |
| Run Duhem against the Onsager repo (dogfood)| [`onsager-dogfood`](../onsager-dogfood/SKILL.md)                |

## Relationship to Onsager's dev process

Duhem and Onsager share the SDD shape but live in separate repos with
separate skills. When working on Duhem, use **this** loop. When
working on Onsager, use the parallel `onsager-dev-process` skill in
the Onsager repo. The two only meet at the dogfood seam — see
`onsager-dogfood` for what that means in practice (Duhem's
verifications run against Onsager PRs; Onsager's PRs surface
Duhem verdicts as a check).
