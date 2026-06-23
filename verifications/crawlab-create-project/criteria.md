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

Scope note: this v1 dogfood targets Crawlab's REST + Mongo surface.
Crawlab's distributed task lifecycle and multi-DB ORM live in MongoDB
metadata and worker gRPC, which Duhem's `db/*` (SQL-only) and action
catalog don't reach yet — those are a later, deeper VD.

## AC-1

Logging in with valid credentials and submitting a new project persists
it: the API accepts the request and returns the created project with a
well-formed identifier and the name that was supplied.

## AC-2

An authenticated request for the project list returns a well-formed
listing — the create above is reflected by a non-empty, correctly
shaped response from the real database.
