# Crawlab — schedule management (API-008)

Acceptance criteria for Crawlab Pro's schedule-management surface, ported
from crawlab-team/crawlab-test's `specs/api/API-008-schedule-management.md`.
Crawlab (`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an
independent vendor, so the asymmetric-trust seam is real: Duhem authors
these checks against Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified against a
real licensed crawlab-pro cluster with no mocks at the web boundary
(`docs/duhem-spec.md` §8): every check exercises the real REST surface, and
schedule persistence + the enable/disable toggle are read straight from the
database rather than echoed by the handler.

## AC-1

A schedule has a real CRUD lifecycle backed by the database: a cron
schedule created for a spider over REST persists in Crawlab's Mongo
`schedules` collection with the same identifier, cron expression, and
spider binding; is listed and fetchable by id; can have its cron updated
(the change lands in the database); and once deleted is gone. Schedule CRUD
is verified at the database layer, not echoed by the handler.

## AC-2

The enable/disable toggle is real and durable: disabling a schedule flips
its `enabled` flag to false in the database, and enabling it flips it back
to true. The toggle is verified at the store, not by the control endpoint's
own 200.

## AC-3

Schedule creation validates its inputs: a schedule with a malformed cron
expression is refused with a client error rather than silently stored. The
cron is parsed, not taken on faith.
