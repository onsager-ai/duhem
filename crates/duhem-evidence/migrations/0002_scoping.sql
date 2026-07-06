-- Scoping + provenance (#190): address every run as
-- workspace → project → verification → run, and record the two-repo
-- provenance (`verifier_repo@sha VERIFIES target_repo@sha`) that the
-- asymmetric-trust seam queries.
--
-- Additive over 0001: new dimension tables, new nullable columns on
-- `runs` (pre-existing rows keep NULLs / the local sentinel), no data
-- rewritten. Append-only discipline extends to the new tables.
--
-- Locally `workspace_id` is the `local` sentinel — the hub (#188)
-- enforces real workspaces. `project_id` is a best-effort hint stored
-- AS-IS (the raw declared `project:` / normalized remote from #191's
-- resolution ladder); the hub reconciles it to a forge repo-ID.

CREATE TABLE workspaces (
    workspace_id TEXT PRIMARY KEY,
    name         TEXT NOT NULL
);
INSERT INTO workspaces (workspace_id, name) VALUES ('local', 'local');

CREATE TABLE projects (
    project_id   TEXT PRIMARY KEY,             -- the identity hint, as-is
    workspace_id TEXT NOT NULL DEFAULT 'local' REFERENCES workspaces(workspace_id)
);

-- One row per (project, verification-name). `verification_id` is the
-- deterministic composite `<project>#<name>` (bare `<name>` when the
-- run carried no project hint), so history queries key on it without
-- a surrogate-id lookup.
CREATE TABLE verifications (
    verification_id TEXT PRIMARY KEY,
    project_id      TEXT REFERENCES projects(project_id),
    name            TEXT NOT NULL,             -- leaf name (CLI `leaf_name` twin)
    definition_path TEXT NOT NULL              -- first-seen path, informational
);

-- No REFERENCES clauses on the added columns: SQLite rejects
-- ALTER-added FK columns with non-NULL defaults under enforced
-- foreign keys, and the store folds the dimension rows itself in the
-- same transaction as `begin_run` — referential integrity is the
-- writer's invariant here.
ALTER TABLE runs ADD COLUMN workspace_id TEXT NOT NULL DEFAULT 'local';
ALTER TABLE runs ADD COLUMN project_id TEXT;
ALTER TABLE runs ADD COLUMN verification_id TEXT;
ALTER TABLE runs ADD COLUMN verifier_repo TEXT;
ALTER TABLE runs ADD COLUMN verifier_sha TEXT;
ALTER TABLE runs ADD COLUMN target_repo TEXT;
ALTER TABLE runs ADD COLUMN target_sha TEXT;

CREATE INDEX runs_project ON runs(project_id);
CREATE INDEX runs_verification ON runs(verification_id);
CREATE INDEX runs_target ON runs(target_repo, target_sha);

-- Append-only enforcement for the new dimension tables. Dimension
-- rows are INSERT OR IGNORE upserts keyed by identity — never
-- updated, never deleted.
CREATE TRIGGER workspaces_no_update BEFORE UPDATE ON workspaces
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on workspaces'); END;
CREATE TRIGGER workspaces_no_delete BEFORE DELETE ON workspaces
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on workspaces'); END;

CREATE TRIGGER projects_no_update BEFORE UPDATE ON projects
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on projects'); END;
CREATE TRIGGER projects_no_delete BEFORE DELETE ON projects
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on projects'); END;

CREATE TRIGGER verifications_no_update BEFORE UPDATE ON verifications
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: UPDATE rejected on verifications'); END;
CREATE TRIGGER verifications_no_delete BEFORE DELETE ON verifications
BEGIN SELECT RAISE(ABORT, 'duhem store is append-only: DELETE rejected on verifications'); END;
