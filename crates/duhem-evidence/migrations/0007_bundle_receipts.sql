-- Content-hash receipts commit atomically with the replicated run and projections.
CREATE TABLE bundle_receipts (
    content_hash TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(run_id)
);

CREATE TRIGGER bundle_receipts_no_update BEFORE UPDATE ON bundle_receipts
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on bundle_receipts'); END;
CREATE TRIGGER bundle_receipts_no_delete BEFORE DELETE ON bundle_receipts
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on bundle_receipts'); END;
