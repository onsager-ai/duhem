# Crawlab — projects & environments (API-019)

Acceptance criteria for Crawlab Pro's project and environment management
surface, ported from crawlab-team/crawlab-test's
`specs/api/API-019-projects-environments.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an independent
vendor, so the asymmetric-trust seam is real: Duhem authors these checks
against Crawlab; Crawlab never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). Verified against a
real licensed crawlab-pro cluster with no mocks at the web boundary
(`docs/duhem-spec.md` §8): both project and environment persistence are
read straight from the database rather than echoed by the handler.

API-019 models environments as a standalone key/value resource (not as
variables nested on a project); these criteria follow the spec's contract.

## AC-1

A project has a real CRUD lifecycle backed by the database: a project
created over REST persists in Crawlab's Mongo `projects` collection with
the same identifier and name, is listed and fetchable by id, can have its
description updated (the change lands in the database), and once deleted is
gone. Project CRUD is verified at the database layer, not echoed by the
handler.

## AC-2

An environment has a real CRUD lifecycle backed by the database: an
environment created over REST with a key and value persists in Crawlab's
Mongo `environments` collection with those exact fields (key and value, not
name/description), is listed and fetchable by id, can have its value updated
(the change lands in the database), and once deleted is gone. Environment
CRUD is verified at the database layer.

## AC-3

Operations on non-existent resources are refused: fetching a project or an
environment by a well-formed but non-existent id is a 404. The
organizational resources return the right error, not a synthetic empty
record.
