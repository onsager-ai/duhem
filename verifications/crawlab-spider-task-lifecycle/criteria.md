# Crawlab — schedule a spider task and read the lifecycle from Mongo

Acceptance criteria for Crawlab Pro's spider → task scheduling path: an
authenticated user creates a spider and runs it, and Crawlab's task
store reflects a real, correctly-linked task in its initial lifecycle
state. Crawlab (`crawlab-team/crawlab-pro`) is Duhem's third dogfood
customer and the first genuinely independent one (a different vendor),
so the asymmetric-trust seam is real: Duhem authors these checks against
Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified
against the real Crawlab REST API backed by a real MongoDB — no mocks at
the web boundary (`docs/duhem-spec.md` §8). This is the deeper companion
to `../crawlab-create-project/`: where that VD verifies a flat resource,
this one reaches the distributed task lifecycle and asserts the linkage
between collections (`tasks.spider_id == spiders._id`) that only a direct
database read can prove — enabled by `db/query`'s MongoDB read path
(#121).

Scope boundary: these criteria cover task **scheduling** — the
`pending` task the master's spider-admin Schedule service writes to
Mongo — not task **execution** (`pending → running → finished`).
Execution requires Crawlab's gRPC worker coordination, whose server is
started in the Pro layer (`crawlab-pro/core/apps/grpc.go`); the
license-free OSS-core stack this VD provisions never starts it, so a
scheduled task stays `pending` with an unassigned node (confirmed live).
A deeper execution VD waits on a licensed or worker-bearing stack — see
`./README.md`.

## AC-1

Creating a spider persists it to the database: the spider exists as a
real document in Crawlab's Mongo `spiders` collection with the supplied
name and command — the same identifier the API returned, read straight
from the store rather than echoed by the handler.

## AC-2

Running a spider schedules a task: the run request persists a real task
document in Mongo's `tasks` collection, in the initial `pending` state,
carrying the spider's command and the identifier the run reported. This
is the entry point of the distributed task lifecycle, verified at the
database layer.

## AC-3

The scheduled task references exactly the spider that was created: the
task's `spider_id` in Mongo equals the created spider's `_id`. The
spider → task link is real in the database, not just implied by the API
call sequence — one holistic slice spanning a REST create, a REST run,
and two direct Mongo reads across two collections.
