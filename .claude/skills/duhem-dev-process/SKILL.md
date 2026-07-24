---
name: duhem-dev-process
description: The end-to-end spec-issue-driven dev loop for Duhem — spec → branch → implement → PR → merge → closure. Use when asked "how do I start work", "what's the process", "SDD loop", "spec-driven development", "how do we ship a change on Duhem", "from scratch what do I do", or when you're about to begin a non-trivial change on the Duhem repo and haven't yet decided how to split spec/PR. Delegates to `issue-spec` (spec writing), the global `pre-push` (pre-push checks) and `pr-lifecycle` (post-push) skills, `verification-authoring` (authoring Verification Definitions for the platform itself), and `onsager-dogfood` (running Duhem against the Onsager repo). This skill carries Duhem's overlay for `pre-push` / `pr-lifecycle` — the check gate, merge-collision patterns, and CI-failure table.
---

# duhem-dev-process

The spec-issue-driven development (SDD) loop on Duhem. Every non-trivial
change starts as a GitHub spec issue on `onsager-ai/duhem`, proceeds
through a PR that references it, and closes when the PR merges.

Duhem is in **Phase 0 — Foundation** (per `docs/duhem-spec.md` §14). The
Cargo workspace ships nine product crates (`duhem-cli`,
`duhem-runtime`, `duhem-judge`, `duhem-schema`, `duhem-actions`,
`duhem-evidence`, `duhem-summary`, `duhem-reporter-pretty`,
`duhem-reporter-junit`) plus an internal `xtask` build helper; the
CLI exposes `init` / `run` / `validate` / `--version`; the `ui/*` and
`api/*` action families (`ui/navigate`, `ui/click`, `ui/type`,
`ui/select`, `ui/assert-*`, `api/call`, `api/observe`) and the
`up:` / `down:` environment hooks are wired in; and product
Verification Definitions are co-located with the products they verify
(Chreode ships them in `onsager-ai/chreode/.duhem/`; epic #225). The
dev loop
below is intentionally lean — it mirrors the discipline used on
`onsager-ai/onsager`, but does not inherit Onsager's seam rule, area
taxonomy, or Rust toolchain checks beyond what `cargo`, `clippy`, and
the `xtask` gates already enforce.

## The loop

```
     ┌─────────────────────────────────────────────────────────────────┐
     │                                                                 │
     │   idea/request                                                  │
     │        ↓                                                        │
     │   spec(<area>): ...    ← issue-spec skill                       │
     │        │                                                        │
     │        ↓                                                        │
     │   branch + implement                                            │
     │        │                                                        │
     │        ↓                                                        │
     │   pre-push skill        ← merge preview, build/test (when wired)│
     │        │                                                        │
     │        ↓                                                        │
     │   git push → open PR (body: "Closes #N" or "Part of #N")        │
     │        │                                                        │
     │        ↓                                                        │
     │   pr-lifecycle skill    ← CI triage, review, iterate            │
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
- Open questions live under `## Alignment` as a `### Open questions`
  subsection (omit if none) — `pre-push` blocks on unresolved
  items there.
- Labels: `spec`, one type (`feat` / `fix` / `refactor` / `perf`),
  one or more `area:*`, one `priority:*`. The full area taxonomy
  lives in `issue-spec`'s SKILL.md and `references/spec-format.md`.

Hard rule: no spec → no PR, unless the PR is labeled `trivial` (typos,
doc-only fixes, one-line obvious bug repair).

Body size: <~2000 tokens. Larger features split into parent + sub-issues
via `mcp__github__sub_issue_write`. The SDD loop runs independently on
each sub-issue; the parent tracks overall progress.

If the spec proposes new **product surface** (new action type, new
schema field, new CLI command, new judge behavior), the spec must
either include or link a worked Verification Definition example
showing how the surface is exercised — see `verification-authoring`.
A surface that has no example by the time implementation starts is
a surface we cannot dogfood, which means we cannot ship it on
Onsager, which means we cannot validate it. Skip this only for
purely internal scaffolding (build configuration, repo hygiene).

### 2. Resolve open questions

Before opening a PR, resolve any open questions on the spec issue
thread. A spec with unanswered `### Open questions` is not ready to
implement — its design isn't pinned yet.

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
section with these required keys:

```markdown
## Schema impact

- **Category:** breaking | additive | clarifying
- **Surfaces touched:** [VD schema, evidence schema, action-type
  catalog, runtime expressions, manifest schema, judge contract]
- **Fields added/renamed/removed:** [...]
- **Migration:** none | manual (describe) | tool-supported (describe)
- **CHANGELOG.md entry:** [exact line for the `## Unreleased` section]
```

The CHANGELOG entry uses the form
`- [breaking|additive|clarifying] one-line summary. (#N)` and appends
to `## Unreleased` on merge. A version-bump commit later renames
`## Unreleased` to `## v0.x.y — YYYY-MM-DD` and advances
`duhem_schema::SCHEMA_VERSION`. Under v0.x, **breaking → minor**;
**additive → patch**; **clarifying → no bump**. Major (`1.0`) is
reserved for the Phase-2 schema-OSS milestone.

Category is mechanical, not aesthetic: a field rename is breaking
regardless of whether the new name is "obviously better." When in
doubt, the `cargo xtask schema-drift` and `cargo xtask
schema-changelog-check` CI gates catch the cheap mistakes.

A `clarifying` PR that touches `crates/duhem-schema/src/**` or
`crates/duhem-evidence/src/**` bypasses the changelog-touch gate by
setting `DUHEM_CHANGELOG_CLARIFYING=1` (CI sets it when the PR body
carries an explicit `clarifying` annotation). Don't use the escape
hatch to dodge tracking a real schema event.

**DX-currency discipline.** User-facing surfaces drift when the product
changes and the docs that teach it don't (that's how the authoring skill
sat stale behind the terse-authoring epic — spec #288). Any change to
user-visible surface — a new/changed action type, a new CLI command or
flag, a new schema field, a changed authoring form — carries a
`## DX impact` section in the spec, the DX analogue of `## Schema
impact`:

```markdown
## DX impact

- **Surfaces touched:** [public authoring skill | adoption template |
  README | docs/getting-started | docs/duhem-spec | CLI --help/describe |
  action-reference | CHANGELOG]
- **Updates landing with this change:** [per surface, or "none (rationale)"]
```

Default is *not* "none": if you added an action, changed the authoring
form, or added a CLI flag, the matching DX surface updates in the same
PR, or the callout says why not. Label the spec `dx-impact` when this
section is non-empty (like `schema-impact`).

The `cargo xtask dx-drift` gate is the mechanical backstop. It fires
when a *surface-declaring* file changes (`crates/duhem-schema/src/**`,
the action catalog / `with:` params, the CLI command defs, or the
generated `docs/action-reference.md`) with no DX doc touched. It's narrow
by design — internal refactors of those crates don't trip it — and ships
**warn-only** for now (flip to `--mode=fail` after a bake). A deliberate
no-op is declared with a `<!-- dx:none -->` marker in the PR body (CI
reads it into `DUHEM_DX_IMPACT_NONE`).

Separately, `cargo xtask skill-scrub` and `dx-drift`'s readme-framing
check are **hard** content gates: the published authoring skill
(`templates/product-repo/.claude/skills/`) and the adoption README must
never carry internal dev vocabulary (dogfood / customer names / seam /
dev-skill names). That's the firewall for user-facing artifacts — a
user should never read how Duhem is *built*. This is distinct from the
docs-site drift gate (#279 = docs↔site sourcing; this = product↔DX
content currency).

### 4. Pre-push

Trigger the global `pre-push` skill (or say "ready to push"). It owns
the generic flow — sync the merge preview, the conflict walkthrough,
the spec-link check, the push. Duhem's overlay (the check gate, the
collision patterns it should watch for) is in the "Pre-push & PR
overlay" section at the bottom of this skill; `pre-push` reads it.

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
on the parent spec — see `pr-lifecycle`.

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

Trigger the global `pr-lifecycle` skill (or say "triage PR" / "CI is
failing" / respond to a webhook). It covers:

- CI triage: build / test / schema-validation failures (the Duhem
  failure table is in the overlay section below).
- Review-comment discipline: fix the code, don't reply per comment.
- Webhook subscription + the post-push CI sweep.

### 7. Merge

- `Closes #N` PRs auto-close the spec on merge.
- `Part of #N` / `Refs #N` PRs leave the spec open; tick the
  delivered Plan items manually on the parent spec, and if all
  sub-issues of a parent are closed, ping the parent. See
  `pr-lifecycle`.
- For schema-impacting PRs, also append the change to `CHANGELOG.md`
  under the next-version heading.

### 8. Closed-unmerged path

If you close a PR without merging (e.g. abandoned approach), the
spec issue stays open as-is — the next implementer can pick it up
from there.

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

A spec issue's open/closed state plus its Plan checkboxes are the
source of truth. Use `Closes #N` only on a PR that delivers the final
unticked Plan items, so GitHub's auto-close fires once the spec is
actually complete; use `Part of #N` for partial slices that leave
items behind, then tick the delivered checkboxes manually on merge.
If a multi-PR spec finishes via `Part of` PRs only, a human closes
the parent once the last Plan item ticks. Plan-item ticks on merge
are manual; `pr-lifecycle` covers the mechanics.

## Anti-patterns (don't)

- **PR without a spec and no `trivial` label.** Reviewers will ask;
  the PR should not merge until the author either adds a spec link
  or the `trivial` label.
- **Closing a spec manually when you meant `Closes #N`.** Let GitHub
  do it via the PR merge so the timeline has the auditable link.
- **Editing Plan checkboxes to mark items done before the PR
  merges.** Tick them on merge, not before.
- **Schema change without `## Schema impact` callout.** The pre-1.0
  schema's breaking-change rate is what determines when we OSS the
  spec; mis-tracking a change skews that signal.
- **Product-surface change without a `## DX impact` callout.** A new
  action / flag / schema field whose authoring skill, README, or
  getting-started stays stale ships a surface users can't learn. The
  `dx-drift` gate reminds you; the callout records the decision.
- **Internal vocabulary in a user-facing artifact.** A `dogfood` /
  customer name / `seam` / dev-skill reference in the published skill or
  adoption README leaks how Duhem is built to someone using it.
  `skill-scrub` / `dx-drift` readme-framing hard-fail on it — fix, don't
  annotate.
- **Skipping `pre-push`.** Even a thin checklist catches the
  cheap mistakes.
- **Shipping a CLI / schema feature without a worked example.** A
  feature that has no Verification Definition demonstrating it is
  a feature we cannot dogfood, full stop. See
  `verification-authoring`.

## Delegation map

| Stage                                       | Skill / workflow                                                |
|---------------------------------------------|-----------------------------------------------------------------|
| Write the spec                              | [`issue-spec`](https://github.com/onsager-ai/dev-skills/blob/main/skills/issue-spec/SKILL.md) (installed globally from `onsager-ai/dev-skills`) |
| Pre-push checks                             | [`pre-push`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pre-push/SKILL.md) (global) + the overlay below |
| CI triage, review, iterate                  | [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (global) + the overlay below |
| On PR merge → tick Plan items               | [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (global, manual) |
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

## Pre-push & PR overlay (for the global `pre-push` / `pr-lifecycle` skills)

The global `pre-push` and `pr-lifecycle` skills carry the generic
methodology. This is Duhem's repo-specific overlay — the gate command,
the collision patterns to watch, and the CI-failure table they reference.

### Check gate

The default pre-push gate (exactly what reviewers and CI run) is:

```bash
just check        # = just lint (fmt-check + clippy -D warnings +
                  #   xtask check-file-budget + skill-scrub +
                  #   dx-drift --mode=warn) then just test
                  #   (cargo test --workspace)
```

Run it on the merged tree (after `pre-push` step 1), not the branch
alone. Then add the gates the diff calls for:

- **Schema-touching** (`crates/duhem-schema/**`, `crates/duhem-evidence/**`,
  or a `SCHEMA_VERSION` bump):

  ```bash
  cargo run -p xtask -- schema-drift            # docs §10 ↔ code
  cargo run -p xtask -- schema-changelog-check  # CHANGELOG.md touch gate
  ```

- **VD-touching**: run each modified Verification Definition through
  `cargo run -p duhem-cli -- validate <path>`.
- **Browser-action-touching** (`crates/duhem-actions/**` `ui/*` or the
  Playwright sidecar): `just test browser-actions` (the `#[ignore]`'d
  browser smoke suites; `just check` skips them). Requires
  `npx playwright install chromium` once per host.

Treat any warning as a blocker; don't `#[allow(dead_code)]` / `@ts-ignore`
past it.

### Merge-collision patterns to watch

- **`docs/duhem-spec.md`**: merge by section / by intent, not line-by-line.
- **`CHANGELOG.md`**: both branches' entries land under the same
  next-version heading — concatenate, don't pick one.
- **Schema fixtures** (`crates/duhem-schema/fixtures/**`,
  `crates/duhem-actions/tests/fixtures/**`): YAML key-order conflicts are
  usually false alarms; re-validate via `duhem validate` or the owning
  crate's tests.
- **Action-type registry** (`crates/duhem-actions/`): both arms land;
  check for name collisions explicitly.
- **`Cargo.lock` / `package-lock.json`**: regenerate by re-running the
  install / build, never hand-edit.

### CI-failure table

| Symptom | Usual cause |
|---------|-------------|
| Build fails on CI, passes locally | CI built the merge preview; main drifted. `git fetch origin main && git merge origin/main`. |
| Schema validator rejects a VD fixture | Fixture authored against an older schema. Update it or document a migration. |
| `CHANGELOG.md` lint fails | Schema-impact PR with no CHANGELOG entry. Add one before re-running. |
| Doc-link check fails | A relative `docs/` link points outside the repo. Resolve to a full URL or fix the path. |
| `skill-scrub` fails | Internal vocabulary in a published skill under `templates/product-repo/.claude/skills/`. Cut or generalize it. Hard gate. |
| `dx-drift` readme-framing fails | Internal framing (dogfood / customer name / `seam` / `docs/duhem-spec.md` ref) in `templates/product-repo/README.md`. Rewrite it user-facing. Hard gate. |
| `dx-drift` warns (surface, no DX doc) | Product surface changed with no DX doc updated. Update one, or add `<!-- dx:none -->` to the PR body. Warn-only today. |

### Schema-impact in the PR body

If the linked spec is labeled `schema-impact`, the PR body must include a
`## Schema impact` subsection (copy the spec's verbatim), and a breaking
change must touch `CHANGELOG.md`. See the schema-stability discipline in
§3 above.

### DX-impact in the PR body

If the linked spec is labeled `dx-impact`, the PR body copies the spec's
`## DX impact` subsection. When the change touches user-visible surface
but deliberately updates no DX doc, add a `<!-- dx:none -->` marker so
CI's `dx-drift` currency check treats it as declared (warn-only today).
`skill-scrub` and `dx-drift`'s readme-framing are hard gates with no such
escape — a published skill or adoption README that leaks internal
vocabulary must be fixed, not annotated. See the DX-currency discipline
in §3 above.
