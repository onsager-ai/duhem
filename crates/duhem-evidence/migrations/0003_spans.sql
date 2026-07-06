-- Delivery-web spans (#192): one row per layer-tagged step, folded
-- from `step_started`/`setup_step_started` (the tag) and the matching
-- `*_finished` (the outcome) in the same append transaction. A
-- check's span set is the ordered layers it crossed with pass/fail —
-- the exact shape the ④ delivery-web view renders (#193).
--
-- Honesty constraint: rows exist only for steps whose *executed*
-- action carried a catalog-derived layer tag. Untagged steps (and
-- whole pre-tag runs) simply have no rows — the view degrades to
-- "layer unknown" instead of guessing.

CREATE TABLE spans (
    run_id   TEXT NOT NULL REFERENCES runs(run_id),
    seq      INTEGER NOT NULL,          -- seq of the step_started event
    check_id TEXT,                      -- NULL for setup-phase steps
    layer    TEXT NOT NULL,             -- 'ui' | 'api' | 'data' | 'runtime'
    ok       INTEGER NOT NULL,          -- step outcome: 1 = ok, 0 = error/timeout
    detail   TEXT,                      -- outcome token when not ok
    PRIMARY KEY (run_id, seq)
);

CREATE INDEX spans_check ON spans(run_id, check_id);

CREATE TRIGGER spans_no_update BEFORE UPDATE ON spans
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on spans'); END;
CREATE TRIGGER spans_no_delete BEFORE DELETE ON spans
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on spans'); END;
