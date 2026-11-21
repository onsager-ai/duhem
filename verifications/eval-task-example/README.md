# Eval task adapter example

This is a small eval-style task with two frozen promises:

- `AC-1`: the candidate prints `ready`.
- `AC-2`: the candidate exits successfully.

`candidates/reference/` fulfills both promises. `candidates/wrong/` exits
successfully but prints `not ready`, so it fails only `AC-1`. The VD reads the
candidate directory from `$inputs.workspace`; `TASK_WORKSPACE` is its local
fallback. The candidate does not contain or select its verifier.

Validate and run from the repository root with a built Duhem CLI and Node.js:

```sh
duhem validate verifications/eval-task-example
duhem run verifications/eval-task-example \
  --inputs workspace="$PWD/verifications/eval-task-example/candidates/reference"
```

Harbor copies this directory into each task's `tests/` directory and invokes
`test.sh` from its verifier working directory. Build the image with an exact
Duhem pin, for example `npm install --global duhem@0.9.3`. The adapter accepts
`DUHEM_BIN`, `VD`, and `VERIFIER_LOGS` overrides for local use and CI. Harbor
supplies `TASK_WORKSPACE`; the adapter writes `reward.json` in its current
working directory and saves the RunSummary, stderr and exported run bundle in
`verifier-logs/`.

A pass writes `{"duhem":1,"AC-1":1,"AC-2":1}`. The wrong candidate writes
`{"duhem":0,"AC-1":0,"AC-2":1}`. If the workspace is missing, Duhem returns
an inconclusive environment error; the adapter writes no reward and exits
non-zero so Harbor records a verifier error instead of a zero score.

CI exercises both candidates and all three adapter outcomes. It uses the
checkout's freshly built CLI through `DUHEM_BIN`, while a Harbor task pins the
published CLI version in its image.
