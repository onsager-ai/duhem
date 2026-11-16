-- Optional CI provenance for runs recorded after #673.
ALTER TABLE runs ADD COLUMN branch TEXT;
ALTER TABLE runs ADD COLUMN pr TEXT;
ALTER TABLE runs ADD COLUMN ci_url TEXT;
