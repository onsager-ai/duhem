-- Recorded run hierarchy (#348): nested runs point at the run that
-- created them and identify whether they came from manifest expansion
-- (`suite`) or a step-spawned `duhem run` (`invocation`).
--
-- Additive and insert-only. Historical rows keep NULL lineage and read
-- as independent top-level runs. The existing runs_no_update /
-- runs_no_delete triggers continue to enforce append-only storage.

ALTER TABLE runs ADD COLUMN parent_run_id TEXT REFERENCES runs(run_id);
ALTER TABLE runs ADD COLUMN origin TEXT CHECK (origin IN ('suite', 'invocation'));

CREATE INDEX runs_parent ON runs(parent_run_id);
