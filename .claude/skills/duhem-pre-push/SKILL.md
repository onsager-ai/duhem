---
name: duhem-pre-push
description: Run before pushing code to the Duhem repo to catch what reviewers and CI will catch later, and confirm the branch has a linked spec issue in a valid state. Reproduces the merge-preview environment, walks through this repo's common merge-conflict patterns, and runs whatever build/test/validator tooling exists at the time. Triggers include "before push", "ready to push", "pre-push check", "push readiness", "prep for PR", "resolve merge conflict", "merge conflict", "branch has conflicts", "sync with main", or proactively before any git push on a Duhem branch.
---

# duhem-pre-push

Mechanical checklist that catches the reviewer / CI failures this
repo has actually had, plus a spec-link check that enforces the SDD
loop locally.

This is Duhem's analogue of Onsager's `onsager-pre-push` skill. The
discipline is the same and so is the shape of the toolchain: the
default pre-push gate is `just check` (`just lint` then `just test`),
exactly what reviewers and CI run. Schema-, VD-, and UI-touching PRs
add a gate apiece (see step 2). When a new product surface lands,
fold its check in here — the checklist tracks the toolchain, it does
not lag it.

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

4. **Verify before committing the merge.** Run `just check` (lint +
   workspace tests) so a textually-clean merge that doesn't actually
   compile is caught here, not in CI. If the merge touched any
   `.yml` Verification Definitions, also run them through
   `cargo run -p duhem-cli -- validate <path>`. The full adder list
   is in step 2.

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
- **Schema fixtures** (`crates/duhem-schema/fixtures/**`,
  `crates/duhem-actions/tests/fixtures/**`): YAML key-order
  conflicts are usually false alarms; verify the fixture still
  parses by running it through `duhem validate` or the owning
  crate's tests.
- **Action-type registry** (`crates/duhem-actions/`): both branches
  added a new action type. Both arms should land; check for name
  collisions explicitly.
- **`Cargo.lock` / `package-lock.json`**: always resolve by
  re-running the install / build, never hand-edit.

### 2. Build / validate

**Default gate — every PR:**

```bash
just check        # = just lint (fmt-check + clippy -D warnings +
                  #   xtask check-file-budget) then just test
                  #   (cargo test --workspace)
```

This is exactly what CI and reviewers run. Run it on the merged tree
from step 1, not the branch alone.

Then add the gates the diff calls for:

- **Schema-touching PRs** (anything under `crates/duhem-schema/**`,
  `crates/duhem-evidence/**`, or that bumps `SCHEMA_VERSION`):

  ```bash
  cargo run -p xtask -- schema-drift            # docs §10 ↔ code
  cargo run -p xtask -- schema-changelog-check  # CHANGELOG.md touch gate
  ```

  These fail loudly if `SCHEMA_VERSION` and `CHANGELOG.md` don't
  agree with the diff.
- **VD-touching PRs**: run each modified Verification Definition
  through the validator —

  ```bash
  cargo run -p duhem-cli -- validate verifications/onsager-dashboard-create-spec-plan/duhem.yml
  ```

- **UI-touching PRs** (`crates/duhem-actions/**` `ui/*` or the
  Playwright sidecar): `just test-ui` — the `ui/*` + `api/observe`
  browser smoke suites. They're `#[ignore]`'d by default, so
  `just check` skips them; run them explicitly when UI is in the
  diff. Requires `npx playwright install chromium` once per host
  (see the `test-ui` recipe header in the `justfile`).

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
