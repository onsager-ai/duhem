# Crawlab — spider CRUD operations (API-004)

Acceptance criteria for Crawlab Pro's spider management surface, ported
from crawlab-team/crawlab-test's
`specs/api/API-004-spider-crud-operations.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an
independent vendor, so the asymmetric-trust seam is real: Duhem authors
these checks against Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified against
a real licensed crawlab-pro cluster with no mocks at the web boundary
(`docs/duhem-spec.md` §8): every check exercises the real REST surface,
and persistence/update/deletion are read straight from Mongo's `spiders`
collection rather than echoed by the handler.

Posture: these criteria encode the CORRECT contract API-004 describes,
not crawlab-pro's observed behavior — a real defect surfaces as a red
verdict (Duhem #160 / #167).

## AC-1

Creating a spider persists it to the database with every field it was
given: the spider exists as a real document in Crawlab's Mongo `spiders`
collection — same `_id` the API returned, and the supplied name, command,
mode, description, priority and result-collection — not merely echoed by
the handler.

## AC-2

A created spider is retrievable by id and appears in the collection
listing: GET by id returns the same spider, and the list endpoint serves
it with a total count. Read works for both the single-resource and the
collection shape.

## AC-3

Updates are real and persisted at the database layer: a partial update
(PATCH) changes only the named fields and leaves the rest intact, and a
full replace (PUT) replaces the fields while keeping the same `_id`.
Verified by reading the `spiders` document back from Mongo.

## AC-4

Deletion is permanent and verifiable at the database layer: a deleted
spider is gone from the API (a re-fetch is refused with 404) and gone from
Mongo (no document with its name remains). Cleanup is real, not a soft
flag the handler hides.

## AC-5

The API reports a missing spider as missing: fetching a syntactically
valid but non-existent id returns 404. Without this, the positive reads
could pass against a handler that never checks existence.

## AC-6

Spider names are unique: creating a second spider with a name that already
exists is refused with a 4xx client error (API-004 §8). This is a contract
claim from the crawlab-test spec — the proven lifecycle VD reuses one
spider name across checks and runs green, so crawlab-pro may not enforce
uniqueness. If it does not, this criterion goes red on the live run: a
genuine finding (or a spec the product declines to honour), to be
reconciled with the maintainer rather than papered over.
