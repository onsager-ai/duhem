# Native runs

Add `native:` to a leaf Verification Definition or root manifest to run your
project's own tests, formatters, linters, or other command-line verification.
Commands run in declaration order after provisioning and before setup and criteria.
A manifest command runs once before any leaf; each leaf can also declare commands.

```yaml
native:
  - id: lint
    layer: static
    command: [sh, -c, 'printf "checked\n"']
    cwd: .
    env: {LANG: C}
    timeout: 2m
    required: true
    expect:
      exit_code: 0
      stdout: {matches: '^checked'}
      stderr: {not_matches: '^warning:'}
```

`id` must be nonempty and unique within its document. `command` is a nonempty
argv list; use `[sh, -c, '...']` explicitly when shell syntax is needed. `cwd`
is relative to the declaring document's directory and defaults to that directory.
`env` adds explicit child environment values to the usual sanitized CLI environment.
`timeout` accepts milliseconds or a duration such as `30s` or `2m`; omitted, the
CLI subprocess runner's default is 5 seconds.

`layer` is optional and accepts `unit`, `integration`, `e2e`, `static`, or `other`.
`required` defaults to true. `expect.exit_code` defaults to zero. `matches` requires
a regex match and `not_matches` forbids one, independently for stdout and stderr.
Patterns use the same Rust regex semantics as `matches:` assertions.

A matching exit code and all output expectations produce `pass`; an unexpected
exit or output produces `fail`. Timeout and spawn failures produce `inconclusive`.
Every command counts as one test. Output is recorded as masked evidence blobs;
failure details are masked in JSON and terminal reports.

Required commands that fail or are inconclusive block the overall run and return
a nonzero CLI exit. `required: false` keeps the outcome visible and counted while
leaving the gate unchanged. Native results never satisfy criteria and cannot be
referenced in assertions or runtime expressions. Criteria keep their independent
Duhem check verdicts.

`RunSummary.native` and `RunSetSummary.native` list command outcomes, layer, duration,
required status, exit code, failure detail, and output blob addresses. `layers`
contains test counts and durations per native layer, plus a `duhem` row for checks.
These fields are omitted when no native commands ran.

Report parsing will follow in a later release. `report: {format: junit, path: ...}`
and `report: {format: ctrf, path: ...}` are recognized shapes, but validation currently
rejects both with `report parsing not yet supported`. No report is silently ignored.
