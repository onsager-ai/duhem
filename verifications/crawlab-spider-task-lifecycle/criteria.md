# Crawlab — run a spider task to completion and read the lifecycle from Mongo

Acceptance criteria for Crawlab Pro's distributed task lifecycle: an
authenticated user creates a spider, runs it, and a worker executes the
task to completion — and Crawlab's task store reflects the real terminal
state. Crawlab (`crawlab-team/crawlab-pro`) is Duhem's third dogfood
customer and the first genuinely independent one (a different vendor),
so the asymmetric-trust seam is real: Duhem authors these checks against
Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified
against a real Crawlab Pro cluster — a master plus a worker, coordinating
over gRPC — backed by a real MongoDB, with no mocks at the web boundary
(`docs/duhem-spec.md` §8). This is the deepest Crawlab VD: where
create-project verifies a flat REST resource, this one drives Crawlab's
distinctive value — the distributed lifecycle where the master schedules
a task, a worker claims it over gRPC, syncs the spider's files, executes
the command, and reports the outcome. The verdict reads the terminal
state straight from the database (`db/query`'s MongoDB read path, #121),
not from what the API chose to echo.

Why Pro: task execution needs the gRPC worker coordination whose server
is started only in Crawlab's Pro layer (`crawlab-pro/core/apps/grpc.go`).
The license-free OSS core never starts it, so a task there stays
`pending` with no claimant — which is why the sibling create-project VD
stops at scheduling and this one provisions the Pro cluster. See
`./README.md` for the (license-free, dogfooding) operator setup.

## AC-1

Creating a spider persists it to the database: the spider exists as a
real document in Crawlab's Mongo `spiders` collection with the supplied
name and command — the same identifier the API returned, read straight
from the store rather than echoed by the handler.

## AC-2

Running a spider executes a task to completion: the run is claimed by a
worker, the spider's files are synced, the command runs, and the task
drives to the terminal `finished` state. The lifecycle is awaited over
REST and the terminal state is then confirmed in MongoDB — the full
distributed path, verified at the database layer rather than inferred
from the API.

## AC-3

The executed task references exactly the spider that was created and is
the task the run reported: `tasks.spider_id == spiders._id` and the
task's `_id` matches the run response. The spider → task link is real in
the database, not just implied by the API call sequence — one holistic
slice spanning a REST create, a file save, a run, an executed task, and
a direct Mongo read.
