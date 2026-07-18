#!/usr/bin/env bash
# PreToolUse launcher for the branch-switch guard. Fails OPEN: if the fast-path
# check passes but python3 is missing or anything errors, the Bash call is
# allowed — a guard rail must never block every command.
set -uo pipefail

input="$(cat)"

# Fast path: only engage when a checkout/switch might be present (keeps overhead
# off every other Bash call). Deliberately loose — `git -C x checkout` breaks a
# literal "git checkout" match — so the Python tokenizer makes the real call.
case "$input" in
  *checkout*|*switch*) : ;;
  *) exit 0 ;;
esac

command -v python3 >/dev/null 2>&1 || exit 0

printf '%s' "$input" | python3 "${BASH_SOURCE%.sh}.py"
exit $?
