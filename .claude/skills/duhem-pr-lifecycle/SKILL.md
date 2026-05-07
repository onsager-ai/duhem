---
name: duhem-pr-lifecycle
description: Manage a Duhem PR after it's been pushed — spec-issue linking, manual planned→in-progress label flip, CI triage, review-comment discipline, merge-conflict recovery on open PRs, webhook subscription, and schema/CHANGELOG follow-through on merge. Triggers include "CI is failing", "check is red", "link this issue", "Closes vs Part of", "respond to review", "subscribe to PR", "triage PR", "the PR is ready", "PR has conflicts", "branch has conflicts with main", "merge conflict on the PR", or when a github-webhook-activity event arrives on a Duhem PR. Paired with `duhem-dev-process` (overall loop), `issue-spec` (spec creation), and `duhem-pre-push` (which owns the pre-push conflict walkthrough).
---

# duhem-pr-lifecycle

Everything that happens after `git push` on a Duhem PR. Covers
spec-issue linking and manual label upkeep, CI triage,
review-comment discipline, webhook subscription, and the manual
ticking of Plan items / CHANGELOG entries on merge.

This is Duhem's analogue of Onsager's `onsager-pr-lifecycle`
skill. The discipline is the same; this repo doesn't yet have the
`pr-spec-sync` workflow, so a few label transitions Onsager
automates are still manual here.

## Tool discipline

- **No `gh` CLI, no `hub`, no direct GitHub API.** Always use
  `mcp__github__*`.
- Scope is restricted to `onsager-ai/duhem` and `onsager-ai/onsager`.
  This skill operates on the Duhem repo. Cross-repo work that
  changes how Onsager surfaces Duhem (e.g. a new dashboard widget
  in Onsager that displays Duhem verdicts) lives on
  `onsager-ai/onsager`'s side and uses Onsager's PR-lifecycle skill.
  See `onsager-dogfood` for which side of the seam owns what.
- Don't open PRs unless the user explicitly asks. Creating one is a
  one-way door in this project's workflow.

## Spec-issue linking (mandatory)

Every PR must either:

1. Link to a spec issue in its body via `Closes #N` / `Fixes #N` /
   `Resolves #N` (slice complete) or `Part of #N` / `Refs #N`
   (scaffolding), **OR**
2. Carry the `trivial` label (typo, doc-only, one-line obvious
   fix).

If neither, the PR is out of process. Comment on the PR asking the
author to add a spec link — creating one via `issue-spec` if none
exists — or apply the `trivial` label.

### Which keyword to use

GitHub closes issues on merge when the PR body contains one of:
`close`, `closes`, `closed`, `fix`, `fixes`, `fixed`, `resolve`,
`resolves`, `resolved` — followed by `#N`.

Pick the keyword based on **what this PR actually delivers**:

| PR delivers                                                  | Use         |
| ------------------------------------------------------------ | ----------- |
| The acceptance test / vertical slice the spec asks for       | `Closes #N` |
| A bug fix for a specific defect                              | `Fixes #N`  |
| Scaffolding / one phase of a multi-phase spec                | `Part of #N`|
| Related work that shouldn't close the spec                   | `Refs #N`   |

`Part of` / `Refs` are **not** auto-close keywords — they just
cross-link in the UI. Use them for scaffolding so the spec stays
open for the real slice.

Edit the PR body via `mcp__github__update_pull_request` (don't open
a new PR just to fix the link). Put the linking line at the top of
the body.

### Multi-issue PRs — enumerate every closure

If a single PR delivers acceptance for more than one issue, write
**one `Closes` keyword per issue**:

```markdown
Closes #27, Closes #30, Closes #33
```

GitHub only honors auto-close on each `#N` individually;
`Closes #27, #30, #33` closes #27 and leaves #30/#33 open.

### The `## Delivers` subsection

For `Part of #N` PRs (and ideally all PRs), include a `## Delivers`
subsection in the body listing the exact Plan items this PR ticks.
Copy the item text verbatim from the spec's `## Plan`, but mark
each as `- [x]`. Use this list to tick the parent spec's
checkboxes after merge — see "Issue progress labels" below.

Example PR body:

```markdown
Closes #42

## Delivers
- [x] Add `--filter` flag handling in `duhem-cli/src/run.rs`
- [x] Update CLI documentation in `docs/cli-reference.md`

## Schema impact
None.

## Summary
Adds a name-pattern filter for Pattern B repos. Backwards-compatible.
```

### `## Schema impact` in the PR body

If the linked spec is labeled `schema-impact`, the PR body must
include a `## Schema impact` subsection (it's fine to copy the
spec's own subsection verbatim). This makes schema changes visible
in the PR diff itself, not just on the linked issue, so reviewers
don't have to context-switch.

If `Breaking change? yes`, the PR must also touch `CHANGELOG.md`.
Comment on the PR if either is missing.

## Issue progress labels — manual on this repo

Onsager has a `pr-spec-sync` workflow that flips spec labels
automatically. Duhem doesn't have one yet. Until it does, **this
skill is responsible for the flips**:

| Spec label    | What it means                              | Who flips it (Duhem)                       |
|---------------|--------------------------------------------|--------------------------------------------|
| `draft`       | AI/human-drafted, not yet reviewed         | Human (via `planned` move — alignment gate)|
| `planned`     | Ready for implementation                   | Human (alignment gate)                     |
| `in-progress` | At least one open PR                       | **This skill, manually, on PR open**       |
| closed        | Delivered, tests passing                   | GitHub (via `Closes` keyword on merge)     |

Concretely, when a PR is opened:

1. Read the PR body. If it contains a `Closes #N` / `Part of #N` /
   `Fixes #N` / `Refs #N` line, look up issue `#N` on
   `onsager-ai/duhem`.
2. If the issue's status label is `planned`, flip it to
   `in-progress`: remove `planned`, add `in-progress`. Use
   `mcp__github__issue_write` (replace-vs-merge label semantics
   matter — see Onsager's canonical mechanics at
   <https://github.com/onsager-ai/onsager/blob/main/.claude/skills/onsager-pr-lifecycle/references/github-ops.md>;
   the same MCP semantics apply on `onsager-ai/duhem`).
3. If the issue's status label is `draft`, **don't flip it**.
   Comment on the PR instead, asking the author to drive the spec
   through human review first.

When a PR is closed unmerged:

1. Search for any other open PR referencing the same issue. If one
   exists, leave the spec at `in-progress`.
2. If no other PR references it, flip the spec back to `planned`:
   remove `in-progress`, add `planned`.

When a PR is merged with `Part of #N` (parent stays open):

1. Read the merged PR's `## Delivers` subsection.
2. For each item, find the matching `- [ ]` line in the parent
   spec's `## Plan` section and flip it to `- [x]`.
3. Edit the spec body via `mcp__github__issue_write`.

When a PR is merged with `Closes #N`:

1. GitHub auto-closes the spec.
2. If the spec is part of a tracker (umbrella) issue, see the
   "Tracker refresh" section below.

## Tracker refresh

Some issues are **umbrella trackers** that reference several
sub-issues as a checklist — identified by a `[Tracking]` title
prefix, a `tracking` label, or a `## Progress` section whose items
are `- [ ] #N` lines. When a PR closes a sub-issue, the tracker
does **not** update itself.

After merge, for each auto-closed or explicitly-closed issue:

1. Search for umbrella trackers that reference it:
   `mcp__github__search_issues` with
   `repo:onsager-ai/duhem #N in:body is:issue is:open`.
2. For each match, read the tracker body. If there's a matching
   `- [ ] ... #N ...` line in a Progress / Plan section, flip it
   to `- [x]`.
3. Post one tracker comment summarizing the delta, not one per
   issue: "PR #<pr> landed #N1, #N2, #N3; ticked in Progress."
4. If after the tick every sub-issue is closed, note that the
   tracker itself is now a candidate for closure — don't close it
   unilaterally, just flag it.

## CI triage

CI on this repo is thinner than Onsager's; what runs depends on
what tooling has landed. Common patterns to anticipate:

| Symptom                                                      | Usual cause                                                      |
|--------------------------------------------------------------|------------------------------------------------------------------|
| Build fails on CI, passes locally                            | CI built the merge preview; main has drifted. `git fetch origin main && git merge origin/main` on the branch. |
| Schema validator rejects a Verification Definition fixture   | Fixture was authored against an older schema. Update the fixture or document a migration. |
| `CHANGELOG.md` lint fails                                    | Schema-impact PR but no CHANGELOG entry. Add one before re-running. |
| Doc-link check fails                                         | A relative link in `docs/` points outside the repo. Resolve to a full URL or fix the path. |

### Accessing logs

`WebFetch` **cannot read authenticated GitHub Actions logs** — both
the run pages and the API logs endpoint return 403. Don't waste
time on them. Work instead from:

1. `mcp__github__pull_request_read` with `method: get_check_runs` —
   gives step name, status, timings.
2. **Local reproduction** after syncing main. Re-run the failing
   step with the exact flags from the workflow yaml.

### When CI is missing entirely

If the repo has no CI workflow that exercises the change you're
making, that's a `area:infra` spec to file — not a license to skip
local verification. Run whatever you have locally and note the gap
in the PR body so a reviewer can decide whether to merge ahead of
the missing CI or wait.

## Merge conflicts on an open PR

When GitHub shows "This branch has conflicts that must be resolved"
or `mcp__github__pull_request_read` reports `mergeable: false`,
resolve **locally** — the GitHub web editor bypasses any local
validation and routinely lands broken merges.

1. **Don't** use `mcp__github__update_pull_request_branch` to
   auto-merge main in via GitHub. That surfaces the same conflicts
   without giving you the resolution workspace, then commits a
   broken merge if you accept the default.

2. Check out the branch locally and run the full conflict
   walkthrough in
   [`duhem-pre-push`](../duhem-pre-push/SKILL.md) (step 1,
   "Resolving conflicts").

3. After the merge commit lands, continue with the rest of
   `duhem-pre-push` (build / validate, spec-link check, schema
   impact) before pushing.

4. Push the merge commit to the same branch with `git push` (no
   `--force`). The existing PR updates in place; the conflict
   banner clears when GitHub re-evaluates.

5. If the PR is tied to a `Closes #N` / `Part of #N` line and the
   merge touched the spec's surface area (schema fields, judge
   semantics), comment on the spec flagging what drifted, so the
   parent stays accurate.

If the branch is so far behind main that the conflict set is large
(>10 files), close the PR, rebase the work into a fresh branch
from `origin/main`, and open a new PR with the same linking line.
Note the close reason on the old PR.

## Review comments

**Fix the code. Don't reply per comment.** Multiple reviewers
(Copilot + human) often flag the same defect; a single commit that
fixes it resolves all of them at once.

Reply *only* when:

- Declining a suggestion (explain why, briefly).
- The comment is a question, not a bug report.
- Asking for clarification before acting.

Use `mcp__github__add_reply_to_pull_request_comment` for threaded
replies, never top-level comments unless summarizing multiple
responses at once.

If a review comment raises a design concern that the spec didn't
address, pause and update the linked spec issue (add an open
question under `## Alignment`, comment on the spec, let a human
decide). Don't silently expand scope in the PR.

## Webhook subscription

Events from CI and reviewers arrive wrapped in
`<github-webhook-activity>` tags. The harness forwards them as user
messages.

- Subscribe once per PR with `mcp__github__subscribe_pr_activity`
  after the PR is created (or the user asks you to watch it).
- Unsubscribe with `mcp__github__unsubscribe_pr_activity` when done
  — not strictly necessary but cleaner.
- Events are already filtered to CI failures + reviews. Treat each
  as actionable; skip only if it's a duplicate of one you just
  addressed.

## Reporting back to the user

After handling a webhook event, end with one or two sentences: what
the failure was, what you changed, whether CI is re-running. Don't
dump the full commit message in chat — the user can see it on the
PR.

## Relationship to other skills

| Related surface                                              | Role                                                                                              |
|--------------------------------------------------------------|---------------------------------------------------------------------------------------------------|
| [`duhem-dev-process`](../duhem-dev-process/SKILL.md)         | Top-level SDD loop; points here for the post-push stage.                                          |
| [`issue-spec`](../issue-spec/SKILL.md)                       | Creates the spec issue this PR links to.                                                          |
| [`duhem-pre-push`](../duhem-pre-push/SKILL.md)               | Runs before `git push`; enforces the spec-link check locally.                                     |
| [`verification-authoring`](../verification-authoring/SKILL.md) | Worked Verification Definitions for product-surface PRs.                                          |
| [`onsager-dogfood`](../onsager-dogfood/SKILL.md)             | When a Duhem change affects how the platform runs against Onsager, coordinate with Onsager-side. |
