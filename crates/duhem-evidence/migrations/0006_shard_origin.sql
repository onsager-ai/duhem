-- Preserve the append-only table and its existing foreign keys/triggers.
-- SQLite cannot widen an existing CHECK using ALTER TABLE. A shard is encoded
-- as the legacy suite origin plus this discriminator; readers expose `shard`.
ALTER TABLE runs ADD COLUMN shard_origin INTEGER NOT NULL DEFAULT 0 CHECK (shard_origin IN (0, 1));
