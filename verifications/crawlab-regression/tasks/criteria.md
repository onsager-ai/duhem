# Crawlab — task CRUD & execution (API-006)

Acceptance criteria for Crawlab Pro's task lifecycle surface, ported from
crawlab-team/crawlab-test's
`specs/api/API-006-task-crud-execution.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an
independent vendor: Duhem authors these checks against Crawlab; Crawlab
never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). This is the
deepest leaf in the suite: it exercises the real distributed task
lifecycle on a master + worker cluster (no mocks at the web boundary —
`docs/duhem-spec.md` §8) and reads the persisted task state straight from
Mongo's `tasks` collection. The execution path reuses the proven recipe
from `verifications/crawlab-spider-task-lifecycle`.

Posture: these criteria encode the correct contract API-006 describes — a
real defect surfaces as a red verdict (Duhem #160 / #167).

## AC-1

Running a spider executes a task to completion: the run is claimed by a
worker, the spider's files are synced, the command runs, and the task
drives to the terminal `finished` state — the full distributed lifecycle,
with the terminal state and the spider→task link read from the real
database, not inferred from the API.

## AC-2

The dedicated run endpoint creates a task bound to its spider: POST
`/api/tasks/run` with a `spider_id` returns a task id, and fetching that
task reports a real lifecycle status and links back to the spider.

## AC-3

Tasks are listable with pagination and fetchable by id: the list endpoint
returns a non-empty array with a total count, a page-sized request returns
no more than the page size, and GET by id returns the task. Read works for
both the collection and single-resource shape.

## AC-4

Tasks can be deleted: deleting a task by id succeeds and a subsequent fetch
is refused with 404. The task is really removed, not flagged.

## Out of scope for batch 1

API-006 also covers cancel (§4), restart (§5), task PATCH/PUT (§11/§12),
and batch update/delete (§14/§15). Cancel needs a reliably long-running
task (non-deterministic to express as a mechanical check); restart and the
batch operations are deferred to a later batch. Tracked under epic #163.
