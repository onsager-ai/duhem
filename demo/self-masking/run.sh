#!/usr/bin/env bash
# Self-contained demo of the self-masking pattern: /health stays green
# while the web front is quietly broken. A real `duhem run` catches it,
# then confirms the fix. This is the source the README demo renders from
# — every line it shows is real output from these two runs.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
DUHEM="${DUHEM:-duhem}"
PORT="${PORT:-8477}"

app() { APP_FIXED="$1" PORT="$PORT" node "$here/app.js" >/dev/null 2>/dev/null & echo $!; }
wait_health() { for _ in $(seq 1 50); do curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1 && return 0; sleep 0.1; done; return 1; }

# 1) The app reports healthy — but ships with a broken web front.
pid=$(app 0); wait_health
echo "\$ curl -s localhost:$PORT/health"; curl -s "http://127.0.0.1:$PORT/health"; echo
echo; echo "\$ duhem run"; "$DUHEM" run "$here"
kill "$pid" 2>/dev/null

# 2) Ship the fix; re-run the same gate.
echo; echo "# fix shipped — the front serves the real app again"
pid=$(app 1); wait_health
echo "\$ duhem run"; "$DUHEM" run "$here"
kill "$pid" 2>/dev/null
