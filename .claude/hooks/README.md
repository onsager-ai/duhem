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
git worktree add ../<repo>-wt/<branch> -b <branch>
# open a Claude session in that directory
# when done:  git worktree remove ../<repo>-wt/<branch>
```

### Scope & guarantees
- **Claude Code only.** Humans and other harnesses (Codex, OpenCode, …) are
  unaffected — git has no `pre-checkout` hook, so this can't be enforced there.
- **Fail-open.** If `python3` is missing or anything errors, the command is
  allowed; the guard never bricks Bash.
- **Override:** prefix `ALLOW_BRANCH_SWITCH=1` for the rare legitimate in-place
  switch (e.g. `ALLOW_BRANCH_SWITCH=1 git checkout main`).
