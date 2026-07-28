-- Runtime-owned run lifecycle (#349), kept separate from judge-owned
-- verdicts. Terminality is an insert-only projection: `run_finished`
-- (with or without a verdict) records `finished`; `run_aborted` records
-- `aborted`. Unterminated runs are classified as running/orphaned at
-- read time from their heartbeat age.

CREATE TABLE run_terminals (
    run_id      TEXT PRIMARY KEY REFERENCES runs(run_id),
    status      TEXT NOT NULL CHECK (status IN ('finished', 'aborted')),
    terminal_at TEXT NOT NULL,
    duration_ms INTEGER NOT NULL
);

-- Existing stores already folded every historical `run_finished` into
-- `run_verdicts`. Backfill terminal markers additively so those traces
-- read as finished without rewriting any evidence or run row.
INSERT INTO run_terminals (run_id, status, terminal_at, duration_ms)
SELECT run_id, 'finished', finished_at, duration_ms FROM run_verdicts;

CREATE TRIGGER run_terminals_no_update BEFORE UPDATE ON run_terminals
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on run_terminals'); END;
CREATE TRIGGER run_terminals_no_delete BEFORE DELETE ON run_terminals
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on run_terminals'); END;

-- Terminality, not judgment, seals a run. A finished unjudged suite
-- therefore rejects later events exactly like a judged leaf run.
DROP TRIGGER events_sealed;
CREATE TRIGGER events_sealed BEFORE INSERT ON events
WHEN EXISTS (SELECT 1 FROM run_terminals WHERE run_id = NEW.run_id)
BEGIN SELECT RAISE(ABORT, 'run is sealed: no events may follow a terminal event'); END;
