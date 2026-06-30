# Crawlab — user management (API-003)

Acceptance criteria for Crawlab Pro's user-management surface, ported from
crawlab-team/crawlab-test's `specs/api/API-003-user-management.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an independent
vendor, so the asymmetric-trust seam is real: Duhem authors these checks
against Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified against a
real licensed crawlab-pro cluster with no mocks at the web boundary
(`docs/duhem-spec.md` §8): every check exercises the real REST surface, and
user persistence is read straight from the database rather than echoed by
the handler.

## AC-1

A user has a real CRUD lifecycle backed by the database: a user created
over REST persists in Crawlab's Mongo `users` collection with the same
identifier and fields, is listed and fetchable by id, can be updated (the
change lands in the database), and once deleted is gone — a subsequent
fetch is refused. The password is never echoed back in any read. User CRUD
is verified at the database layer, not by the handler's own response.

## AC-2

Password management is real, not nominal: after an admin changes a user's
password, the old password no longer authenticates and the new password
does. The credential store is actually rewritten, verified by logging in
end to end — not by trusting the change handler's own 200.

## AC-3

Invalid user operations are refused with the right client errors: a create
missing the required fields is rejected, a duplicate username is rejected,
and a fetch of a non-existent user is a 404. The validation and uniqueness
rules are enforced, not nominal.
