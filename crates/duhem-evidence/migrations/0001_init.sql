-- Store schema v1 (#189): the DB single source of truth for run evidence.
--
-- Design invariants, enforced here rather than by convention:
--
-- * Append-only. Every table is insert-only; UPDATE and DELETE are
--   rejected by triggers. A run's history can grow, never change.
-- * A run is sealed once its `run_verdicts` row lands: further event
--   inserts for that run are rejected (`events_sealed` trigger).
-- * `events` is the source of truth — the JSONL-row successor (#10).
--   Each `payload` holds the full serialized wire-format event line.
--   `criteria` / `checks` / `assertions` / `run_verdicts` are derived
--   projections folded from events in the same transaction, kept for
--   queryability (the #190 scoping/history queries build on them).

CREATE TABLE runs (
    run_id         TEXT PRIMARY KEY,
    verification   TEXT NOT NULL,             -- definition path
    schema_version TEXT NOT NULL,             -- trace wire version ("v1")
    inputs         TEXT NOT NULL DEFAULT '{}',-- JSON object
    started_at     TEXT NOT NULL              -- RFC 3339, ms precision
);

CREATE INDEX runs_started_at ON runs(started_at);

-- Verdicts are a separate insert-only table (not an UPDATE on `runs`)
-- so strict append-only holds everywhere. No row here = run in flight
-- (or crashed before judgment) — same semantics as a trace without a
-- `run_finished` line.
CREATE TABLE run_verdicts (
    run_id      TEXT PRIMARY KEY REFERENCES runs(run_id),
    verdict     TEXT NOT NULL,                -- "pass" | "fail" | "inconclusive:<cause>"
    finished_at TEXT NOT NULL,
    duration_ms INTEGER NOT NULL
);

CREATE TABLE events (
    run_id  TEXT NOT NULL REFERENCES runs(run_id),
    seq     INTEGER NOT NULL,
    ts      TEXT NOT NULL,
    kind    TEXT NOT NULL,                    -- payload discriminant, for filters
    payload TEXT NOT NULL,                    -- full wire-format event JSON line
    PRIMARY KEY (run_id, seq)
);

CREATE TABLE criteria (
    run_id       TEXT NOT NULL REFERENCES runs(run_id),
    criterion_id TEXT NOT NULL,
    verdict      TEXT NOT NULL,
    PRIMARY KEY (run_id, criterion_id)
);

CREATE TABLE checks (
    run_id       TEXT NOT NULL REFERENCES runs(run_id),
    check_id     TEXT NOT NULL,
    criterion_id TEXT,                        -- resolved from step_started events; NULL if unresolvable
    verdict      TEXT NOT NULL,
    PRIMARY KEY (run_id, check_id)
);

-- Keyed by (run_id, seq): each row derives from exactly one
-- assertion_evaluated event, and the same assertion index may be
-- evaluated more than once (polling actions re-evaluate).
CREATE TABLE assertions (
    run_id          TEXT NOT NULL REFERENCES runs(run_id),
    seq             INTEGER NOT NULL,
    check_id        TEXT NOT NULL,
    assertion_index INTEGER NOT NULL,
    state           TEXT NOT NULL,
    detail          TEXT,
    PRIMARY KEY (run_id, seq)
);

-- Content-addressed blobs (screenshots, captured stdio, large
-- observations). Global across runs: identical content dedupes.
CREATE TABLE artifacts (
    sha256 TEXT PRIMARY KEY,                  -- 64-char lowercase hex
    size   INTEGER NOT NULL,
    bytes  BLOB NOT NULL
);

-- ---------------------------------------------------------------
-- Append-only enforcement. One UPDATE + one DELETE rejection trigger
-- per table.
-- ---------------------------------------------------------------

CREATE TRIGGER runs_no_update BEFORE UPDATE ON runs
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on runs'); END;
CREATE TRIGGER runs_no_delete BEFORE DELETE ON runs
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on runs'); END;

CREATE TRIGGER run_verdicts_no_update BEFORE UPDATE ON run_verdicts
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on run_verdicts'); END;
CREATE TRIGGER run_verdicts_no_delete BEFORE DELETE ON run_verdicts
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on run_verdicts'); END;

CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on events'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on events'); END;

CREATE TRIGGER criteria_no_update BEFORE UPDATE ON criteria
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on criteria'); END;
CREATE TRIGGER criteria_no_delete BEFORE DELETE ON criteria
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on criteria'); END;

CREATE TRIGGER checks_no_update BEFORE UPDATE ON checks
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on checks'); END;
CREATE TRIGGER checks_no_delete BEFORE DELETE ON checks
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on checks'); END;

CREATE TRIGGER assertions_no_update BEFORE UPDATE ON assertions
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on assertions'); END;
CREATE TRIGGER assertions_no_delete BEFORE DELETE ON assertions
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on assertions'); END;

CREATE TRIGGER artifacts_no_update BEFORE UPDATE ON artifacts
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on artifacts'); END;
CREATE TRIGGER artifacts_no_delete BEFORE DELETE ON artifacts
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on artifacts'); END;

-- A sealed run accepts no further events. The RunFinished fold inserts
-- the event row before the run_verdicts row (same transaction), so the
-- finishing event itself passes this check.
CREATE TRIGGER events_sealed BEFORE INSERT ON events
WHEN EXISTS (SELECT 1 FROM run_verdicts WHERE run_id = NEW.run_id)
BEGIN SELECT RAISE(ABORT, 'run is sealed: no events may follow run_finished'); END;
