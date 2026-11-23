#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
vd="${VD:-$script_dir/duhem.yml}"
duhem_bin="${DUHEM_BIN:-duhem}"
verifier_logs="${VERIFIER_LOGS:-$PWD/verifier-logs}"
rm -f reward.json
mkdir -p "$verifier_logs"
run_dir="$(mktemp -d)"
trap 'rm -rf "$run_dir"' EXIT
report="$run_dir/summary.json"
db="$run_dir/evidence.db"

set +e
"$duhem_bin" run "$vd" --db "$db" --reporter json >"$report" 2>"$verifier_logs/duhem.stderr"
run_status=$?
set -e
cp "$report" "$verifier_logs/run-summary.json"

if [[ ! -s "$report" ]]; then
    echo "Duhem did not produce a RunSummary" >&2
    exit 1
fi

run_id="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["run_id"])' "$report")"
rm -rf "$verifier_logs/run-bundle"
"$duhem_bin" export "$run_id" --db "$db" --out "$verifier_logs/run-bundle"

verdict_file="$run_dir/verdict"
set +e
python3 - "$report" "$verdict_file" <<'PY'
import json
import os
import sys
import tempfile
from collections import Counter

summary_path, verdict_path = sys.argv[1:]
with open(summary_path, encoding="utf-8") as source:
    summary = json.load(source)
verdict = summary["verdict"]
with open(verdict_path, "w", encoding="utf-8") as target:
    target.write(verdict)

if verdict.startswith("inconclusive"):
    try:
        os.unlink("reward.json")
    except FileNotFoundError:
        pass
    raise SystemExit(3)
if verdict not in ("pass", "fail"):
    raise SystemExit(f"unsupported Duhem verdict: {verdict}")

reward = {"duhem": int(verdict == "pass")}
for criterion in summary["criteria"]:
    criterion_verdict = criterion["verdict"]
    # An inconclusive criterion is ungraded: omit its key, keep the run's 0 or 1.
    if criterion_verdict.startswith("inconclusive"):
        continue
    if criterion_verdict not in ("pass", "fail"):
        raise SystemExit(f"unsupported criterion verdict: {criterion_verdict}")
    reward[criterion["id"]] = int(criterion_verdict == "pass")

scores = [
    (criterion_id, check_id, step_id, score)
    for criterion_id, checks in summary.get("scores", {}).items()
    for check_id, steps in checks.items()
    for step_id, score in steps.items()
]
step_counts = Counter(step_id for _, _, step_id, _ in scores)
verdict_keys = {"duhem"} | {criterion["id"] for criterion in summary["criteria"]}
reserved = verdict_keys | {f"llm_{step_id}" for step_id in step_counts}
for criterion_id, check_id, step_id, score in scores:
    if "value" not in score:
        continue
    key = f"llm_{step_id}"
    if step_counts[step_id] > 1 or key in verdict_keys:
        identity = json.dumps([criterion_id, check_id, step_id], separators=(",", ":"))
        key = f"llm_{identity}"
        while key in reserved:
            key += "#"
        reserved.add(key)
    reward[key] = score["value"]

fd, temporary = tempfile.mkstemp(prefix="reward.", dir=".", text=True)
try:
    with os.fdopen(fd, "w", encoding="utf-8") as target:
        json.dump(reward, target, separators=(",", ":"))
        target.write("\n")
    os.replace(temporary, "reward.json")
except BaseException:
    try:
        os.unlink(temporary)
    except FileNotFoundError:
        pass
    raise
PY
mapping_status=$?
set -e
if (( mapping_status == 3 )); then
    echo "Duhem verdict is inconclusive; no reward was written" >&2
    exit 1
fi
if (( mapping_status != 0 )); then
    exit "$mapping_status"
fi

verdict="$(cat "$verdict_file")"
if [[ "$verdict" == "fail" ]]; then
    # Duhem returns nonzero for a fail; Harbor receives a valid zero reward.
    exit 0
fi
if (( run_status != 0 )); then
    echo "Duhem exited nonzero despite a passing verdict" >&2
    exit 1
fi
