#!/usr/bin/env python3
"""PreToolUse guard: keep agents from switching branches in a shared checkout.

Two Claude sessions pointed at the same working tree stomp each other's branch
state (this cost us a real incident). The fix is one session per git worktree.
This hook denies in-place branch switches and points at `git worktree add`.

Repo-agnostic: byte-identical across onsager-ai repos (crawlab-pro, chreode,
duhem, ...). Nothing here names a specific repo — the worktree hint is derived
from the checkout at runtime.

Contract (Claude Code PreToolUse):
  - stdin: JSON with .tool_input.command (the Bash command about to run)
  - exit 0        -> allow
  - exit 2 + stderr -> deny, stderr shown to the agent

Fail-open by construction: any parse error / unexpected shape -> allow. A guard
rail must never brick every Bash call.

Blocks:  git switch <branch|-c|-C|->,  git checkout -b/-B,  git checkout <branch|->
Allows:  git checkout -- <path>,  git checkout <file|.|sha>,  git worktree add,
         and anything with the ALLOW_BRANCH_SWITCH=1 escape hatch.
"""
import json
import os
import re
import shlex
import subprocess
import sys

OVERRIDE = "ALLOW_BRANCH_SWITCH=1"

GLOBAL_OPTS_WITH_VALUE = {"-C", "-c", "--namespace", "--work-tree", "--git-dir", "--exec-path"}


def allow():
    sys.exit(0)


def repo_name():
    try:
        top = subprocess.check_output(
            ["git", "rev-parse", "--show-toplevel"],
            stderr=subprocess.DEVNULL,
        ).decode().strip()
        return os.path.basename(top) or "repo"
    except Exception:
        return "repo"


def deny(what):
    repo = repo_name()
    sys.stderr.write(
        "✋ Blocked: `" + what + "` switches branches in place.\n"
        "   This repo runs ONE Claude session per git worktree so concurrent\n"
        "   sessions can't stomp each other's checkout (see .claude/hooks/README.md).\n"
        "   Start branch work in its own worktree instead:\n"
        "     git worktree add ../" + repo + "-wt/<branch> -b <branch>\n"
        "   then open a session in that directory.\n"
        "   Rare, deliberate override:  prefix the command with " + OVERRIDE + "\n"
    )
    sys.exit(2)


def is_branch(name):
    for ref in ("refs/heads/" + name, "refs/remotes/" + name):
        try:
            if subprocess.run(
                ["git", "show-ref", "--verify", "--quiet", ref],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            ).returncode == 0:
                return True
        except Exception:
            return False
    return False


def check_segment(seg):
    """Return a reason string if this shell segment switches branches, else None."""
    try:
        toks = shlex.split(seg)
    except Exception:
        toks = seg.split()
    if "git" not in toks:
        return None
    rest = toks[toks.index("git") + 1:]
    # Skip git global options (some take a value).
    i = 0
    while i < len(rest) and rest[i].startswith("-"):
        i += 2 if rest[i] in GLOBAL_OPTS_WITH_VALUE else 1
    if i >= len(rest):
        return None
    sub, args = rest[i], rest[i + 1:]

    if sub == "switch":
        # git switch is always branch-oriented; block if it names a target.
        if any(not a.startswith("-") for a in args) or "-" in args \
                or any(a in ("-c", "-C", "--create") for a in args):
            return "git switch"
        return None

    if sub == "checkout":
        if "--" in args:
            return None  # explicit pathspec checkout
        if any(a in ("-b", "-B", "--orphan") for a in args):
            return "git checkout -b"
        positional = [a for a in args if not a.startswith("-")]
        if not positional:
            return None
        first = positional[0]
        if first == "-" or is_branch(first):
            return "git checkout " + first
        return None  # file / tag / sha -> allow

    return None


def main():
    try:
        data = json.load(sys.stdin)
    except Exception:
        allow()
    cmd = ((data.get("tool_input") or {}).get("command") or "")
    if not cmd or OVERRIDE in cmd:
        allow()
    # Loose early-out; the per-segment tokenizer below makes the real decision
    # (so `git -C x checkout` isn't missed by a literal "git checkout" test).
    if "checkout" not in cmd and "switch" not in cmd:
        allow()
    for seg in re.split(r"&&|\|\||;|\n|\|", cmd):
        reason = check_segment(seg)
        if reason:
            deny(reason)
    allow()


if __name__ == "__main__":
    main()
