---
name: duhem-pre-push
description: Run before pushing code to the Duhem repo to catch what reviewers and CI will catch later, and confirm the branch has a linked spec issue in a valid state. Reproduces the merge-preview environment, walks through this repo's common merge-conflict patterns, and runs whatever build/test/validator tooling exists at the time. Triggers include "before push", "ready to push", "pre-push check", "push readiness", "prep for PR", "resolve merge conflict", "merge conflict", "branch has conflicts", "sync with main", or proactively before any git push on a Duhem branch.
---

# duhem-pre-push

Mechanical checklist that catches the reviewer / CI failures this
repo has actually had, plus a spec-link check that enforces the SDD
loop locally.

This is Duhem's analogue of Onsager's `onsager-pre-push` skill. The
discipline is the same; the toolchain is different and intentionally
thinner, because Duhem is in Phase 0 and there's not much to build
yet. As the CLI, validator, runtime, and judge land, fold their
checks into this checklist — don't let "we don't have a build yet"
calcify into "we don't run a build before pushing."

## Why

CI on `pull_request` checks out a **merge of `origin/main` + the PR
branch**, not the branch alone. Local `cargo build` (or whatever the
toolchain becomes) without that merge is insufficient.

The spec-link step enforces "no PR without a spec or a `trivial`
label" at push time, before the PR is open — so the author sees the
problem locally instead of hearing about it from a reviewer.

## Steps

Run all of these from the repo root.

### 1. Sync main into the branch

```bash
git fetch origin main
git merge origin/main --no-edit
```

Resolve conflicts **locally**, before push — never on the PR
"Resolve conflicts" web editor (it bypasses any local validation).
If the merge aborts cleanly, skip to step 2.

#### Resolving conflicts

1. **Inventory** what conflicted:

   ```bash
   git status --short                   # U* lines = unresolved paths
   git diff --name-only --diff-filter=U
   ```

2. **Work by pattern, not by file.** A single logical conflict
   often spans several files. Match what you see against the
   patterns below before touching conflict markers — the right
   fix is often "take main's version and re-apply your change on
   top", not a line-by-line merge.

3. **Resolve**, then stage each resolved path with `git add <path>`.
   Re-run `git status` until no `U*` entries remain.

4. **Verify before committing the merge.** Run whatever build /
   validator tooling exists for this repo at the time. At Phase 0
   the relevant checks are limited; once a CLI lands, this should
   include `cargo build` (or equivalent), the schema validator on
   any `.yml` Verification Definitions touched, and the test suite.

5. Only then:

   ```bash
   git commit --no-edit                 # default "Merge branch 'main' ..." message
   ```

6. **If you get lost**, bail and retry:

   ```bash
   git merge --abort
   ```

   This restores pre-merge state. Never `git reset --hard` or
   `git checkout --` without confirming nothing is staged you
   care about — the merge carries uncommitted resolutions.

   Prefer `merge` over `rebase` for syncing main here: the branch
   is likely already pushed, rebase rewrites history, and force-push
   is a destructive action.

#### Common collision patterns to watch for

These are anticipated based on the product shape in
`docs/duhem-spec.md`. Add to this list as actual collisions accrue.

- **`docs/duhem-spec.md` simultaneous edits**: two branches both
  edited the spec doc. Merge by section — figure out which
  section each branch was changing and merge by intent, not by
  line.
- **`CHANGELOG.md`**: both branches added entries under the same
  next-version heading. Both entries should land — concatenate
  alphabetically by category.
- **Schema fixtures (`tests/fixtures/**/*.yml` once these exist)**:
  YAML key-order conflicts are usually false alarms; verify the
  fixture still parses by running it through the validator.
- **Action-type registry (when this exists)**: both branches added
  a new action type. Both arms should land; check for name
  collisions explicitly.
- **`Cargo.lock` / `pnpm-lock.yaml` (once these exist)**: always
  resolve by re-running the install / build, never hand-edit.

### 2. Build / validate

Run whatever build, lint, and test commands the repo has at the
time. As of Phase 0, that's near-empty; treat the absence as a
**known gap** to fix when the corresponding code lands, not as a
permission to skip the step:

- If the repo has a `Cargo.toml`: `cargo build --workspace` and
  `cargo test --workspace --lib`.
- If the repo has a `package.json`: `pnpm install` and `pnpm test`.
- If the repo has a schema validator binary: run it against any
  `.yml` Verification Definitions touched in the diff.
- If the repo has a CHANGELOG and this PR has `## Schema impact`:
  confirm the CHANGELOG was updated.

Treat **any** warning as a blocker. Do not `#[allow(dead_code)]` or
`@ts-ignore` your way past it; fix the root cause.

### 3. Spec-issue link check

Before pushing, confirm this branch corresponds to a known spec
issue (or is explicitly trivial). This is the local enforcement of
the SDD loop's spec-link rule.

1. **Find the spec issue.** Search open issues with the `spec`
   label on `onsager-ai/duhem`:

   ```
   mcp__github__list_issues  repo=onsager-ai/duhem  labels=[spec]   state=open
   ```

   Or read your commit messages (`git log origin/main..HEAD`) for a
   `#N` reference.

   If you can't find one, stop and create one via `issue-spec` (or
   triage whether this is truly `trivial`).

2. **Confirm any open questions on the spec are resolved.** Open
   questions live under `## Alignment` as a `### Open questions`
   subsection (see `duhem-dev-process` § "Write the spec"). If any
   are unanswered, stop and resolve them in the issue thread first
   — the design isn't pinned yet.

3. **Draft the PR body linking line** so you can paste it in:

   - `Closes #N` if this PR delivers the full spec.
   - `Part of #N` if it's one slice of a multi-PR spec.
   - `Fixes #N` for a defect referenced by a bug spec.

   Also draft a `## Delivers` subsection listing the exact Plan
   items you tick with this PR.

4. **Scan the branch's commit messages for implicit issue
   references** (advisory, not blocking):

   ```bash
   git log --format='%s%n%b' origin/main..HEAD | grep -oE '#[0-9]+' | sort -u
   ```

   For each `#N` returned, decide deliberately:

   - **PR delivers that issue's acceptance** → add `Closes #N` to
     the body. Multi-issue `Closes` lines are fine
     (`Closes #27, Closes #30, Closes #33`). Auto-close doesn't
     fire for issues that are only *mentioned* in commit subjects
     — without an explicit `Closes` line, those issues stay open
     after merge.
   - **PR only touches that issue** → use `Refs #N` so it
     cross-links without claiming closure.
   - **False positive** (issue number inside a code identifier,
     commit hash, etc.) → ignore.

5. **If this is genuinely trivial** (typo, doc-only, one-line
   obvious fix), skip the spec-link substeps above and plan to
   apply the `trivial` label to the PR immediately after
   `mcp__github__create_pull_request`.

### 4. Schema-impact and worked-example check

If the spec issue this PR closes is labeled `schema-impact`:

- Confirm `## Schema impact` is filled in on the spec.
- Confirm a CHANGELOG entry is staged for this PR.
- If the change introduces user-visible product surface, confirm a
  worked Verification Definition example is included in the spec
  or in the PR — see `verification-authoring`.

This is what stops us from quietly drifting the schema while the
product is pre-1.0; the breaking-change-rate signal is only useful
if every change that should count, counts.

### 5. Push

```bash
git push -u origin <branch>
```

Retry up to 4 times with exponential backoff on transient network
errors. **Never** use `--force` on `main` or long-lived branches
without explicit ask.

After push, open the PR with the spec-link line in its body (or apply
the `trivial` label). The spec issue stays open until the PR closes
it — no status labels to flip.

## Fast path

If nothing under tracked source paths changed (e.g. docs-only
edits), step 2 can be near-trivial. Step 1 is not — main may have
moved. Step 3 is not — the spec-link requirement applies to all
non-trivial PRs.

## What this skill does NOT cover

- Writing the spec issue — see [`issue-spec`](https://github.com/onsager-ai/dev-skills/blob/main/skills/issue-spec/SKILL.md) (installed globally from `onsager-ai/dev-skills`).
- Opening or managing the PR — see [`duhem-pr-lifecycle`](../duhem-pr-lifecycle/SKILL.md).
- The end-to-end dev loop — see [`duhem-dev-process`](../duhem-dev-process/SKILL.md).
- Authoring Verification Definitions — see [`verification-authoring`](../verification-authoring/SKILL.md).
- Running Duhem against the Onsager repo — see [`onsager-dogfood`](../onsager-dogfood/SKILL.md).
