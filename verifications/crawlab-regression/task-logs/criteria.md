# Crawlab — task logs & results (API-007)

Acceptance criteria for Crawlab Pro's task logs and results surface,
ported from crawlab-team/crawlab-test's
`specs/api/API-007-task-logs-results.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an
independent vendor: Duhem authors these checks against Crawlab; Crawlab
never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Each check drives
a real task to `finished` on the master + worker cluster (no mocks at the
web boundary — `docs/duhem-spec.md` §8) before reading its logs/results.

Posture: these criteria encode the correct contract API-007 describes — a
real defect surfaces as a red verdict (Duhem #160 / #167).

## AC-1

A finished task exposes its execution logs: after a task runs to
completion, the logs endpoint returns a non-empty body carrying the
script's output, and the paginated form is accepted. Logs are the real
captured stdout of the worker run, not an empty stub.

## AC-2

A finished task exposes a well-formed results endpoint: the results
endpoint answers 200 with a `data` payload, even when the task saved no
records (the empty-results contract from API-007 §3.2). This pins that the
endpoint exists and is well-formed; asserting non-empty results needs a
result-saving spider (a Python result-SDK spider in the worker image),
deferred and flagged in `duhem.yml`.

## AC-3

The logs and results endpoints reject an unknown task id: requesting logs
or results for a non-existent task is refused with a client error, not
served as an empty success. Without this, the positive checks could pass
against a handler that answers 200 for anything.

## Expressiveness limits (flagged, not faked)

Logs may be returned as a string or an array (API-007 explicitly), so a
content-substring assertion is not robustly expressible today —
`$runtime.matches` needs a string operand and there is no array `contains`
helper (gap noted on #166). AC-1 asserts non-emptiness, which holds for
both forms.
