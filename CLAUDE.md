@AGENTS.md

When tool mapping is needed, use the [Claude mechanics reference](.agents/skills/harness-operations/references/claude-code.md).

## Session scope (Claude Code sessions on this repo)

A session on this repo works the **duhem repo only**. It is consulted on duhem-the-tool; it is not a gatekeeper or reviewer for another repo's work unless that work bears directly on duhem. (Principal ruling, 2026-09-18.)

- **In scope.** Anything answerable by reading duhem: does an action exist at version X, what changed between versions, is this expressible at a given pin, is this a duhem defect. A bug report against duhem raised from another repo's usage is still duhem's business.
- **Out of scope.** Non-author review of another repo's PR; reviewing or authoring another repo's Verification Definitions; any gatekeeping role in another repo's merge path.

The test that divides them is **whose knowledge the question requires**, not which artifact it touches. A Verification Definition is a duhem artifact, but "does this locator bind the right component" and "is this check reachable given fixture ordering" are answered by reading *that product*; "does `ui/extract` yield page text at 0.4.0" is answered by reading this repo. If answering requires opening their repo, it is theirs.

Hand back anything already in flight rather than dropping it — a peer expecting a verdict is owed the news that it is not coming.

## File editing (Claude Code tools)

Prefer `Edit` over `Write` for any change to an existing file. Full
rewrites with `Write` can hit a stream idle timeout on files larger
than ~150 lines and there is no automatic retry — a stalled `Write`
silently leaves the file in its previous state or, worse,
half-written. If a rewrite is genuinely necessary, split it: write
a smaller initial version, then extend with follow-up `Edit` calls.

## Session defaults (Claude Code cloud)

If the current branch name starts with `claude/` (the prefix cloud
sessions create), treat PR creation and CI auto-fix as part of
finishing the task — do not wait to be asked:

1. Push the branch.
2. Open a pull request as ready for review (not a draft). Before
   calling `mcp__github__create_pull_request`, answer the
   spec-vs-trivial gate and bake the answer into the PR at creation
   time:
   - If a spec issue exists or you should write one, include
     `Closes #N` or `Part of #N` in the PR body.
   - If the change is genuinely `trivial` (typo, doc-only,
     formatting, one-line obvious fix — see `duhem-dev-process` for
     the full list), pass `labels: ["trivial"]` on creation.
   - Default is spec, not trivial. When in doubt, create the spec
     issue first via the `issue-spec` skill, then open the PR with
     `Closes #N`.
3. Subscribe to PR activity via
   `mcp__github__subscribe_pr_activity` so CI failures and review
   comments are auto-fixed.

Skip this for branches that don't start with `claude/`
(local/manual work).
