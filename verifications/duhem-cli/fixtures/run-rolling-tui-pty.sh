#!/usr/bin/env bash
set -euo pipefail

duhem_bin=$1
repo_dir=$2
report_file=$(mktemp)
db_file=$(mktemp)
typescript_file=$(mktemp)
trap 'rm -f "$report_file" "$db_file" "$typescript_file"' EXIT

printf -v duhem_command '%q ' \
  "$duhem_bin" run \
  "$repo_dir/verifications/duhem-cli/fixtures/rolling-tui.yml" \
  --reporter json \
  --db "$db_file"
duhem_command+="> $(printf '%q' "$report_file")"
command="stty cols 100 rows 12; $duhem_command"

TERM=xterm-256color script -qefc "$command" "$typescript_file"

if grep -Fq "collapsed" "$typescript_file" && ! grep -Fq $'\e[2K' "$typescript_file"; then
  printf '\nTUI_BOUNDED_DIFF=yes'
else
  printf '\nTUI_BOUNDED_DIFF=no'
fi
if [[ $(head -c 1 "$report_file") == "{" ]]; then
  printf '\nREPORTER_STDOUT_CLEAN=yes'
else
  printf '\nREPORTER_STDOUT_CLEAN=no'
fi
printf '\nREPORTER='
cat "$report_file"
