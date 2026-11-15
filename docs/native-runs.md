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
native commands not selected this way are excluded from the gate. If native-only
filters select at least one native run and no criteria/checks, the empty body is
vacuously passing and the overall verdict is the selected native runs' gate
contribution. No `body_verdict` is reported, and claim scope still reports zero
selected checks and excluded criteria. If no native run matches, the existing
empty-selection inconclusive behavior remains. Excluded IDs appear in JSON
selection/claim-scope output and as `excluded native runs: ...` in the default
terminal report. A native pattern matching no declared ID behaves like an
unmatched criterion filter: the selected checks (or no checks) still follow normal
filter behavior, and no native command runs for that pattern.

## Dashboard navigation

The run page lists native runs with layer, outcome, required/diagnostic status,
counts, duration and failing tests. Evidence links open masked stdout, stderr,
source report and normalized results through the existing artifact route. A
parent page combines its own native runs with all manifest descendants, retaining
`run_id` on each summary so identical native IDs in different leaves stay distinct.
Invocation runs inside checks remain scoped to those checks.

`GET /api/runs/:id` adds optional `native` and `layers` arrays. Each native entry
contains its summary fields, source `run_id` and `evidence` addresses. The layer
table includes Duhem's executed checks and their summed step durations. Native
test counts and durations use the shared summary aggregation. These arrays are
omitted when no native commands ran. During a live run, native completion refreshes
the recorded detail; the normal run loading/error states also cover this data.

The existing [failure envelope](failure-envelope-contract.md) adds `failing_native`
for every non-passing native run, including diagnostic runs. A required native
failure on a suite parent blocks the tree verdict even when all leaves pass.
Static exports retain native summaries, failure envelopes and evidence links.

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
even on exit zero; timeout is also inconclusive.

For report runs, `native[].tests` is the testcase total and `counts` contains
`passed`, `failed`, `errored`, and `skipped`. `report_path` records the declared
path, `report_blob_sha256` addresses the masked source report, and
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

## CTRF reports

Use `report: {format: ctrf, path: ctrf.json}` for UTF-8 JSON reports. The parser
is pinned to [CTRF v0.2.0](https://github.com/ctrf-io/ctrf/blob/v0.2.0/schema/ctrf.schema.json).
It reads `results.tests[]`: required `name`, `status`, and integer `duration`
(milliseconds), plus optional `message`, `trace`, `filePath` and `suite` hierarchy.
`passed` → pass, `failed` → fail, `skipped` and `pending` → skip, and `other` → error.
Unknown statuses, missing required test fields and malformed values make the report
inconclusive. Counts come from test records; producer summary totals are ignored.
Unused producer metadata is ignored; this is ingestion, not full schema validation.

Missing `specVersion` is accepted for producers that omit it. When present it must
be a three-part numeric version; a major newer than the pinned major (0) makes the
report inconclusive with an explicit version detail. `reportFormat`, when present,
must be `CTRF`. A suite array is JSON-encoded as the normalized `classname` to
preserve its hierarchy in stable test IDs. Message and trace are combined and
masked through the same evidence pipeline as JUnit. Counts, gate behavior, blobs
and bounded failure navigation are shared by both formats.

## Detecting existing tests with init

`duhem init verifications/hello --name hello` walks from the target directory to
the nearest project with detected tests, stopping at a repository or project
boundary. Both scaffold patterns print a ready-to-paste YAML `native:` block as an
informational note on stderr. Commands assume the detected project root as cwd;
put the entries in a manifest at that root, or set `cwd` relative to your manifest.
Init never runs tests or edits runner configuration. Detection creates no files
and changes neither the existing scaffold files nor the command's exit status.
Review commands and report paths before adding the suggestions to your manifest.

| Detected setup | Proposed command | Report |
| --- | --- | --- |
| `Cargo.toml` | `cargo test --workspace` | Exit code |
| Cargo plus `.config/nextest.toml` with `[profile.ci.junit] path = "junit.xml"` | `cargo nextest run --profile ci` | JUnit `target/nextest/ci/junit.xml` |
| `package.json` with `scripts.test` | `npm test`, `pnpm test` for `pnpm-lock.yaml`, or `yarn test` for `yarn.lock` | Exit code |
| `pytest.ini`, `conftest.py`, or `[tool.pytest.ini_options]` in `pyproject.toml` | `pytest --junitxml=duhem-pytest.xml` | JUnit `duhem-pytest.xml` |
| `go.mod` | `go test ./...` | Exit code |

All suggestions use `layer: unit` and are required by default. If both JavaScript
lockfiles exist, pnpm takes precedence. Missing or malformed nextest JUnit config
uses `cargo test --workspace`; init does not assume nextest will emit a report.
The path follows [nextest's documented JUnit configuration](https://nexte.st/docs/machine-readable/junit/).
Review installed tools, command options and report paths before running. With no
detected tests, init prints no suggestion. Scaffold files are unchanged either way,
including Pattern B's stub.
