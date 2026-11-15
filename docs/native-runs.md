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
`timeout` accepts milliseconds or a duration such as `30s` or `2m`; omitted, it
defaults to 30 minutes. A failing run's detail shows the last 20 lines of stdout
and stderr; the full masked output is stored as evidence blobs.

`layer` is optional and accepts `unit`, `integration`, `e2e`, `static`, or `other`.
`required` defaults to true. `expect.exit_code` defaults to zero. `matches` requires
a regex match and `not_matches` forbids one, independently for stdout and stderr.
Patterns use the same Rust regex semantics as `matches:` assertions.

A matching exit code and all output expectations produce `pass`; an unexpected
exit or output produces `fail`. Timeout and spawn failures produce `inconclusive`.
Without a report, each command counts as one test. Output is recorded as masked
evidence blobs; failure details are masked in JSON and terminal reports.

Required commands that fail or are inconclusive block the overall run and return
a nonzero CLI exit. `required: false` keeps the outcome visible and counted while
leaving the gate unchanged. Native results never satisfy criteria and cannot be
referenced in assertions or runtime expressions. Criteria keep their independent
Duhem check verdicts.

`RunSummary.native` and `RunSetSummary.native` list command outcomes, layer, duration,
required status, exit code, failure detail, and output blob addresses. `layers`
contains test counts and durations per native layer, plus a `duhem` row for checks.
These fields are omitted when no native commands ran.

## Selecting native runs

`duhem run --filter native:<id>` selects native run IDs on both the root manifest
and leaf Verification Definitions. The ID uses the same anchored exact-or-glob
matching as criterion IDs; `*` matches zero or more characters. Repeated
`--filter` options are ORed.

When any `--filter` is supplied, native commands run only when a `native:` filter
matches them. Criterion-only filters therefore skip every native command, and
native commands not selected this way are excluded from the gate. Excluded IDs
appear in JSON selection/claim-scope output and as `excluded native runs: ...` in
the default terminal report. A native pattern matching no declared ID behaves
like an unmatched criterion filter: the selected checks (or no checks) still
follow normal filter behavior, and no native command runs for that pattern.

## JUnit reports

Add a JUnit XML report to count individual tests and retain failure evidence:

```yaml
native:
  - id: rust-unit
    layer: unit
    command: [cargo, nextest, run, --profile, ci]
    report: {format: junit, path: target/nextest/ci/junit.xml}
```

`report.path` must be nonempty and relative to the native command's `cwd`.
Configure your runner to write the report there. JUnit XML from nextest, pytest,
jest-junit, Playwright and go-junit-report is supported, including bare
`testsuite` and nested `testsuites` roots. Reports must use UTF-8. Counts come
from `testcase` records rather than suite-level totals. Missing `time` means zero;
seconds are rounded to milliseconds. Skipped tests count without failing the run.

Passing requires the expected exit code (zero by default), all output expectations,
a readable and valid report, and no failed or errored tests. An unexpected exit
fails even with an all-pass report. A missing or malformed report is inconclusive,
even on exit zero; timeout is also inconclusive. `format: ctrf` remains rejected
with `ctrf not yet supported`.

For report runs, `native[].tests` is the testcase total and `counts` contains
`passed`, `failed`, `errored`, and `skipped`. `report_path` records the declared
path, `report_blob_sha256` addresses the masked source XML, and
`results_blob_sha256` addresses a JSON array of all normalized tests. Each result
has `test_id`, optional `classname`, `name`, optional `file`, `status`
(`pass|fail|skip|error`), `duration_ms`, and optional `message`. The stable test ID
is the JSON encoding of `[native_id, classname_or_null, name]`.

`failing_tests` contains up to ten failures/errors with `test_id`, `name`, optional
`file` and masked `message`, capped at 2,048 characters plus a truncation marker.
The complete messages remain in the results blob. Terminal output shows counts,
failure names and the first message line, followed by the number of further
failures. Layer totals use actual test statuses, with separate `errored` and
`skipped` counts when nonzero; a command crash still blocks the gate even when
all reported tests passed. Report fields are omitted for exit-code-only runs.
