# `db-task-state`

Worked example for Duhem's `db/query` + `db/seed` actions (#101). It
asserts a distributed-task-lifecycle fact that lives in **database
state** — after a task finishes, its row records a terminal status —
the kind of check the Crawlab dogfood needs and that `ui/*` + `api/*`
can't reach.

- Criterion prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | After a task finishes, the tasks store returns exactly that task with the terminal status `finished`. |

`db/seed` (in `setup:`) and `db/query` (in the check) both run against
the **real** database — no mock of the store (`docs/duhem-spec.md` §8).
The committed default targets a real SQLite file so the example is
reproducible anywhere; SQLite is a real database engine, not a double.
The assertion reaches a column via #104 nested navigation:
`$steps.read.outputs.rows[0].status == "finished"`.

## Running

Default (real SQLite file under `/tmp`, created on first use), **from
the duhem repo root**:

```sh
duhem run verifications/db-task-state/duhem.yml
```

Live Crawlab dogfood — point `db_url` at Crawlab's real Postgres:

```sh
duhem run verifications/db-task-state/duhem.yml \
  --inputs db_url='postgres://user:pass@localhost:5432/crawlab'
```

The query is portable SQL; the seed is too, but adjust column types if
your target's dialect is strict.

## Prerequisites

- A Playwright Chromium for Duhem's browser. The check has **no UI
  step**, but `db/*` (like `api/call`) reports `requires_page = true`
  today, so the runtime opens a browser anyway — stripping it for
  non-UI checks is a deferred optimization (#105 tracks browser
  provisioning). On a host where the bundled Playwright can't install
  Chromium:
  ```sh
  export DUHEM_BROWSER_EXECUTABLE=/path/to/chrome
  ```

## Status

Proven green end-to-end against a real SQLite database: `verdict:
pass`, all three assertions (`row_count == 1`, `rows[0].id == 1`,
`rows[0].status == "finished"`).

## Value coverage (v1)

`db/query` decodes column values by trying `i64`, `f64`, `bool`,
`String`; SQL `NULL` → JSON `null`; a type outside that set (timestamp,
numeric, uuid, json) currently decodes to `null`. Widening the decode
set is a follow-up.
