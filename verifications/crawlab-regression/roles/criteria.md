# Crawlab — role management (API-018)

Acceptance criteria for Crawlab Pro's role-management surface, ported from
crawlab-team/crawlab-test's `specs/api/API-018-role-management.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an independent
vendor, so the asymmetric-trust seam is real: Duhem authors these checks
against Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified against a
real licensed crawlab-pro cluster with no mocks at the web boundary
(`docs/duhem-spec.md` §8): roles are not created via the API (they are
system-seeded), so the role records are read straight from the database
rather than trusted from the API's own response. API-018 exposes no create
endpoint, so this leaf exercises the read / update / delete surface over
the default roles.

## AC-1

The system ships with real roles, backed by the database: the role list is
non-empty, a role is fetchable by id with its permission shape (pages and
permissions arrays), and the database confirms a root-admin role exists in
the `roles` collection. Roles are genuine records, not a synthetic API
response.

## AC-2

A role update is durable: changing a role's description over REST lands in
the database. Role edits are persisted, not merely acknowledged by the
handler.

## AC-3

Invalid and protected role operations are refused: fetching or updating a
non-existent role is a 404, and deleting a protected system (root-admin)
role is rejected with a client error. The system's access-control records
are guarded, not freely mutable.
