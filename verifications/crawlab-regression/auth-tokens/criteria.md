# Crawlab — authentication & token management (API-002)

Acceptance criteria for Crawlab Pro's authentication and API-token surface,
ported from crawlab-team/crawlab-test's
`specs/api/API-002-authentication-token-management.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an
independent vendor, so the asymmetric-trust seam is real: Duhem authors
these checks against Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified against
a real licensed crawlab-pro cluster (master + worker + MongoDB) with no
mocks at the web boundary (`docs/duhem-spec.md` §8): every check exercises
the real REST surface, and token persistence is read straight from the
database rather than echoed by the handler.

## AC-1

Logging in with valid credentials issues a usable session token, and that
token authenticates the caller: an authenticated request for the current
user is accepted and returns the same admin identity. Auth works end to
end, not just at the login call.

## AC-2

API tokens have a real lifecycle backed by the database: a created token
is returned by the API, persists as a document in Crawlab's Mongo `tokens`
collection with the same identifier, is listed and fetchable by id, and
once deleted it is gone — a subsequent fetch is refused. Token CRUD is
verified at the database layer, not echoed by the handler.

## AC-3

Logging out invalidates the session: after a logout, the same token no
longer authenticates a gated request. The session boundary is enforced on
the way out, not just on the way in. (Behavioral claim lifted from API-002
step 11 — confirm against crawlab-pro source: if logout is client-only
with a stateless JWT, this criterion must be reframed.)

## AC-4

The gated endpoint enforces authentication: a request carrying no valid
session — neither a missing `Authorization` header nor a malformed token —
is refused with 401. Without this, the positive checks could pass against
an endpoint that never validates the token.

## AC-5

Login refuses invalid credentials: a wrong password is rejected with a
client error and issues no token. The credential check is real, not
nominal.
