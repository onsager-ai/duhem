---
name: duhem-dev-process
description: The end-to-end spec-issue-driven dev loop for Duhem — spec → branch → implement → PR → merge → closure. Use when asked "how do I start work", "what's the process", "SDD loop", "spec-driven development", "how do we ship a change on Duhem", "from scratch what do I do", or when you're about to begin a non-trivial change on the Duhem repo and haven't yet decided how to split spec/PR. Delegates to `issue-spec` (spec writing), the shared `pre-push` (pre-push checks) and `pr-lifecycle` (post-push) skills, `verification-authoring` (authoring Verification Definitions for the platform itself), and `onsager-dogfood` (running Duhem against the Onsager repo). This skill carries Duhem's overlay for `pre-push` / `pr-lifecycle` — the check gate, merge-collision patterns, and CI-failure table.
---

# duhem-dev-process

The spec-issue-driven development (SDD) loop on Duhem. Every non-trivial
change starts as a GitHub spec issue on `onsager-ai/duhem`, proceeds
through a PR that references it, and closes when the PR merges.

Duhem is in **Phase 0 — Foundation** (per `docs/duhem-spec.md` §14). The
Cargo workspace ships ten product crates (`duhem-cli`,
`duhem-runtime`, `duhem-judge`, `duhem-schema`, `duhem-actions`,
`duhem-evidence`, `duhem-summary`, `duhem-dashboard`,
`duhem-reporter-pretty`, `duhem-reporter-junit`) plus an internal
`xtask` build helper; the CLI exposes `init` / `actions` / `describe` /
`validate` / `resolve` / `run` / `browser` / `dashboard` / `export` /
`ship` / `mcp` / `--version`; the `ui/*`, `api/*`, `db/*` and `cli/*`
action families (`ui/navigate` / `click` / `type` / `select` / `wait` /
`extract` / `capture-session` / `assert-*`,
`api/call` / `observe` / `poll` / `stream`,
`db/query` / `observe` / `seed`, `cli/invoke`) and the
`up:` / `down:` environment hooks are wired in; and product
Verification Definitions are co-located with the products they verify
(Chreode ships them in `onsager-ai/chreode/.duhem/`; epic #225). The
dev loop
below is intentionally lean — it mirrors the discipline used on
`onsager-ai/onsager`, but does not inherit Onsager's seam rule, area
taxonomy, or Rust toolchain checks beyond what `cargo`, `clippy`, and
the `xtask` gates already enforce.

## Shared workflow and Duhem overlay

Use the checkout-local `issue-spec` → `pre-push` → `pr-lifecycle` workflow.
Those skills own generic spec structure, alignment handling, issue/PR linking,
review and CI lifecycle, and progress updates. This file owns Duhem's delta.

- Non-trivial changes require a spec issue on `onsager-ai/duhem`; `trivial` is
  limited to typos, formatting and one-line obvious fixes, never multi-file work
  or product-identity changes.
- Use area/priority labels from this repository. Spec bodies stay under ~2000
  tokens; use the shared issue-spec structure and reconcile answered decisions.
- Product-surface changes require a worked Verification Definition; skip this
  only for internal scaffolding/build configuration/repository hygiene.
- Claude-owned branches retain the native `claude/` convention. Other harnesses
  use their declared task branch and the root contract's worktree parking policy.
- The Claude-only session restriction is in CLAUDE.md. Skill availability does
  not expand that scope. Repo-owned gates and public-surface overlays follow.

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
to `## Unreleased` on merge. A release-cut bump commit later advances
`duhem_schema::SCHEMA_VERSION`, updates the Cargo workspace version,
and propagates it to the npm platform packages with
`npm/scripts/sync-versions.mjs`. The cut inserts a new
`## vX.Y.Z — YYYY-MM-DD` heading immediately below `## Unreleased`,
leaving `## Unreleased` empty, then tags that commit `vX.Y.Z`.
`schema-changelog-check --lint` enforces the dated release section and
tag coverage. Under v0.x, **breaking → minor**; **additive → patch**;
**clarifying → no bump**. Major (`1.0`) is reserved for the Phase-2
schema-OSS milestone.

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

## Workflow integration

`pre-push` runs the merge-preview gate and applicable additional checks below.
`pr-lifecycle` owns spec linking, review/CI triage and issue progress. Link full
spec completion with `Closes #N`; link partial work with `Part of #N` and list
exact delivered Plan items. Tick spec Plan items after merge, not before it.
The shared workflow grants no publication or merge authority.

Use the schema/DX overlays above and PR-body requirements below. Do not suppress
warnings, weaken verification, leak internal development vocabulary into
published adoption artifacts, or omit a worked example for new product surface.

## Delegation map

| Stage                                       | Skill / workflow                                                |
|---------------------------------------------|-----------------------------------------------------------------|
| Write the spec                              | [`issue-spec`](https://github.com/onsager-ai/dev-skills/blob/main/skills/issue-spec/SKILL.md) (checkout-local from `onsager-ai/dev-skills`) |
| Pre-push checks                             | [`pre-push`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pre-push/SKILL.md) (checkout-local) + the overlay below |
| CI triage, review, iterate                  | [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (checkout-local) + the overlay below |
| On PR merge → tick Plan items               | [`pr-lifecycle`](https://github.com/onsager-ai/dev-skills/blob/main/skills/pr-lifecycle/SKILL.md) (checkout-local, manual) |
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

## Pre-push & PR overlay (for the shared `pre-push` / `pr-lifecycle` skills)

The shared `pre-push` and `pr-lifecycle` skills carry the generic
methodology. This is Duhem's repo-specific overlay — the gate command,
the collision patterns to watch, and the CI-failure table they reference.

### Check gate

The pre-push gate is:

```bash
just preflight    # = just lint + just test + just self-verify
                  #   + xtask schema-changelog-check (strict)
                  #   + xtask schema-drift
                  #   + xtask action-reference --check
                  # ...all run against a merge preview, not the branch.
```

**Do not substitute `just check`.** It is the fast inner-loop gate and
green there does *not* imply green CI. Two gaps, both of which have
shipped red branches:

- `just lint` runs `schema-changelog-check` advisory (`--lint`); CI runs
  it strict. A missing CHANGELOG entry passes locally and fails CI.
- `just check` never runs Duhem's own self-verification suite. On #437 a
  schema change rejected an in-tree example VD while every static check
  and unit test stayed green; only CI caught it.

**`preflight` gates the merge result for you** (#475). It fetches
`origin/main`, merges it into a scratch worktree, and runs every stage
there — you do not merge by hand first. A fetch failure *refuses* rather
than silently falling back to a branch-only gate.

This matters because a branch-only green does not compose. #468 and #470
each added to the same file, each passed its own `preflight`, and each
was honestly green against a `main` that lacked the other; together they
went over the file budget and broke `main` (#472). Any check whose
subject is a property of the *merged tree* — the file budget, schema
drift, the generated action reference, and every test — is measured
wrongly on the branch alone.

Two things the gate does not do: it cannot see a branch that lands
*after* your fetch, so a green means "green against `main` as of this
fetch"; and it refuses outright when offline. Then add the gates the
diff calls for:

- **Schema-touching** (`crates/duhem-schema/**`, `crates/duhem-evidence/**`,
  or a `SCHEMA_VERSION` bump):

  (Both `schema-drift` and `schema-changelog-check` are already in
  `just preflight`; no extra command needed.)

- **VD-touching**: run each modified Verification Definition through
  `cargo run -p duhem-cli -- validate <path>`.
- **Browser-action-touching** (`crates/duhem-actions/**` `ui/*` or the
  Playwright sidecar): `just test browser-actions` (the `#[ignore]`'d
  browser smoke suites; neither `just check` nor `just preflight` runs
  them). Requires **both** `npm ci` in
  `crates/duhem-actions/sidecar` *and* `npx playwright install chromium`,
  and each git worktree needs its own `npm ci` — worktrees get their own
  working copy, so a sidecar installed in one is absent in the next. The
  usual symptom is `ERR_MODULE_NOT_FOUND: playwright`, which looks like a
  missing browser and is almost always a missing `node_modules/`.

Treat any warning as a blocker; don't `#[allow(dead_code)]` / `@ts-ignore`
past it.

### Merge-collision patterns to watch

- **`docs/duhem-spec.md`**: merge by section / by intent, not line-by-line.
- **`CHANGELOG.md`**: both branches' entries land under
  `## Unreleased` — concatenate, don't pick one. `.gitattributes` sets
  `merge=union` so local merges and rebases do this automatically, but
  **GitHub's merge button does not read merge drivers**: with parallel
  branches open, each one after the first will report a changelog
  conflict on GitHub and must be rebased locally, where the driver
  applies.
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
the schema/DX requirements above.

### DX-impact in the PR body

If the linked spec is labeled `dx-impact`, the PR body copies the spec's
`## DX impact` subsection. When the change touches user-visible surface
but deliberately updates no DX doc, add a `<!-- dx:none -->` marker so
CI's `dx-drift` currency check treats it as declared (warn-only today).
`skill-scrub` and `dx-drift`'s readme-framing are hard gates with no such
escape — a published skill or adoption README that leaks internal
vocabulary must be fixed, not annotated. See the DX-currency discipline
in the schema/DX requirements above.
