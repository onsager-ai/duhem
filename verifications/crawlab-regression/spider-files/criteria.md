# Crawlab — spider file management (API-005)

Acceptance criteria for Crawlab Pro's spider-file surface, ported from
crawlab-team/crawlab-test's
`specs/api/API-005-spider-file-management.md`. Crawlab
(`crawlab-team/crawlab-pro`) is a Duhem dogfood customer and an
independent vendor: Duhem authors these checks against Crawlab; Crawlab
never authors its own.

These criteria are the stable human commitment; `duhem.yml` is the
derivative mechanism (`docs/duhem-spec.md` §7.2 / §7.3). A spider's files
live in the worker/master workspace on disk, so the verification depth
here is the real content round-trip through that workspace (no mocks at
the web boundary — `docs/duhem-spec.md` §8); a `db/query` confirms the
parent spider is a real DB-backed document.

Posture: these criteria encode the correct contract API-005 describes —
a real defect surfaces as a red verdict (Duhem #160 / #167).

## AC-1

A saved spider file round-trips through the real workspace: after saving a
file with content, fetching it back returns exactly that content, and its
metadata reports a file (not a directory). The save hit the disk and the
get read it back — not a handler echo. The parent spider is confirmed to
be a real DB-backed document.

## AC-2

Directories and nested files are real: a created directory and a file
saved inside it both appear when listing, and the listing distinguishes
the file from the directory. The workspace tree is navigable, not flat.

## AC-3

Copy, rename, and delete operate on real files: a copy produces an
independent duplicate whose content matches the original, a rename makes
the file readable under the new path, and a delete removes it so a
subsequent get is refused.

## AC-4

The file API reports a missing file as missing: fetching a path that was
never created is refused with a client error, not served as empty content.
Without this, the round-trip checks could pass against a handler that
returns success for anything.
