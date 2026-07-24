# Claude Code hooks

## One session per worktree (branch-switch guard)

**Rule: never run two Claude sessions against the same checkout.** Two sessions
sharing a working tree stomp each other's branch state — a switch in one session
lands the other session's commits on the wrong branch. Give each session its own
git worktree instead.

`guard-branch-switch.sh` (+ `.py`) is a `PreToolUse` hook wired in
`../settings.json`. It **denies** in-place branch switches and points at
`git worktree add`:

| Command | Result |
|---|---|
| `git switch <branch>` / `-c` / `-` | ✋ blocked |
| `git checkout <branch>` / `-b` / `-B` | ✋ blocked |
| `git worktree add ../wt/<b> -b <b>` | ✅ allowed — the intended path |
| `git checkout -- <path>`, `git checkout .`, `git checkout <sha>` | ✅ allowed (not a branch switch) |
| any command prefixed `ALLOW_BRANCH_SWITCH=1` | ✅ allowed (deliberate override) |

The scripts are **repo-agnostic** (byte-identical across onsager-ai repos —
crawlab-pro, chreode, duhem, …); the worktree hint is derived from the checkout
at runtime.

### Start branch work in a worktree

```sh
just worktree-add <branch>
# `worktree-add` prints the path; enter it, then use the normal commands:
just dev  # or: just build / just lint / just test
# open an agent session in that directory
# when done:  git worktree remove ../<repo>-wt/<branch>
```

### Scope & guarantees
- **Executable enforcement is Claude Code only.** Git has no `pre-checkout`
  hook, so other harnesses cannot use this hook directly. Codex receives the
  equivalent primary-checkout/worktree rule through the root `AGENTS.md`
  (`CLAUDE.md` is its source); humans and other harnesses remain unaffected.
- **Fail-open.** If `python3` is missing or anything errors, the command is
  allowed; the guard never bricks Bash.
- **Override:** prefix `ALLOW_BRANCH_SWITCH=1` for the rare legitimate in-place
  switch (e.g. `ALLOW_BRANCH_SWITCH=1 git checkout main`).
