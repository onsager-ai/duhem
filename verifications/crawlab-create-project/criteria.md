# Crawlab — create a project and read it back

Acceptance criteria for Crawlab Pro's core resource flow: an
authenticated user can create a project and the project store reflects
it. Crawlab (`crawlab-team/crawlab-pro`) is Duhem's third dogfood
customer — and the first genuinely independent one (a different vendor),
so the asymmetric-trust seam is real: Duhem authors these checks against
Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified
against the real Crawlab REST API backed by a real MongoDB — no mocks at
the web boundary (`docs/duhem-spec.md` §8). The flow exercises auth
(login → token), a write to Mongo, and an authenticated read back, in
one holistic slice.

Scope note: this dogfood targets Crawlab's REST + Mongo surface. AC-5
reads the project store directly with `db/query` (#121 added a MongoDB
read path), so the verdict no longer rests only on what the REST handler
echoes back. Crawlab's distributed task lifecycle and worker gRPC are
still out of reach for the action catalog — those are a later, deeper
VD that builds on this Mongo read path.

## AC-1

Logging in with valid credentials and submitting a new project persists
it: the API accepts the request and returns the created project with a
well-formed identifier and the name that was supplied.

## AC-2

An authenticated request for the project list returns a well-formed
listing — the create above is reflected by a non-empty, correctly
shaped response from the real database.

## AC-3

After a project is created, it becomes visible in the project listing
within a bounded window — the write propagates to the read path without
the check having to guess a fixed delay.

## AC-4

A created project is individually retrievable by its identifier —
fetching it by id returns exactly that project.

## AC-5

A created project is actually persisted in the database, not merely
echoed by the API: the project exists as a document in Crawlab's real
MongoDB store, with the same identifier the API returned. This is the
deep DB-state slice the REST-only criteria above can't reach — it
asserts what landed in the database, read straight from the store, not
what the handler chose to return.
