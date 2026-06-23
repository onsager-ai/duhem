# A finished task records a terminal status in the database

Acceptance criterion for the `db/*` worked example (#101), shaped like
the Crawlab dogfood it exists for: after a task finishes, its row in the
tasks table records a terminal status. This is the kind of distributed-
task-lifecycle fact that lives in **database state** the `ui/*` +
`api/*` catalog can't see.

The criterion is the stable human commitment; `duhem.yml` is the
mechanism. It runs against a **real** SQL database via `db/seed`
(precondition) and `db/query` (read-back) — no mock of the store
(`docs/duhem-spec.md` §8). The committed default targets a real SQLite
file so the example is reproducible anywhere; point `db_url` at
Crawlab's Postgres to run it as the live dogfood (see `README.md`).

## AC-1

After a task has finished, querying the tasks store returns exactly
that task, and its recorded status is the terminal value `finished`.
