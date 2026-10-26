-- Common SQLite / D1 schema. Artifacts and execution inputs are append-only.
CREATE TABLE IF NOT EXISTS platform_records (
 dataset TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
 version INTEGER NOT NULL CHECK(version > 0), deleted INTEGER NOT NULL DEFAULT 0,
 PRIMARY KEY(dataset,key)
);
CREATE TABLE IF NOT EXISTS platform_artifacts (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
 dataset TEXT NOT NULL, installation TEXT NOT NULL, application TEXT NOT NULL,
 output TEXT NOT NULL, project TEXT, status TEXT, created INTEGER NOT NULL,
 body TEXT NOT NULL, idempotency TEXT, fingerprint TEXT NOT NULL,
 UNIQUE(dataset,installation,idempotency)
);
CREATE INDEX IF NOT EXISTS platform_discovery ON platform_artifacts(dataset,sequence DESC);
CREATE TABLE IF NOT EXISTS platform_resolutions (
 dataset TEXT NOT NULL, installation TEXT NOT NULL, execution TEXT NOT NULL,
 slot TEXT NOT NULL, reference TEXT NOT NULL, ids TEXT NOT NULL,
 PRIMARY KEY(dataset,installation,execution,slot)
);
-- Quotas include metadata and tombstones. An overflowing statement is rolled back.
CREATE TRIGGER IF NOT EXISTS platform_record_insert_quota AFTER INSERT ON platform_records
BEGIN
 SELECT CASE WHEN (SELECT coalesce(sum(length(CAST(value AS BLOB))+length(CAST(key AS BLOB))+128),0) FROM platform_records WHERE dataset=NEW.dataset)>8388608 THEN RAISE(ABORT,'dataset record quota exceeded (8 MiB)') END;
END;
CREATE TRIGGER IF NOT EXISTS platform_record_update_quota AFTER UPDATE ON platform_records
BEGIN
 SELECT CASE WHEN (SELECT coalesce(sum(length(CAST(value AS BLOB))+length(CAST(key AS BLOB))+128),0) FROM platform_records WHERE dataset=NEW.dataset)>8388608 THEN RAISE(ABORT,'dataset record quota exceeded (8 MiB)') END;
END;
CREATE TRIGGER IF NOT EXISTS platform_artifact_quota AFTER INSERT ON platform_artifacts
BEGIN
 SELECT CASE WHEN (SELECT coalesce(sum(length(CAST(body AS BLOB))+length(CAST(fingerprint AS BLOB))+256),0) FROM platform_artifacts WHERE dataset=NEW.dataset)>67108864 THEN RAISE(ABORT,'dataset artifact quota exceeded (64 MiB)') END;
END;
CREATE TRIGGER IF NOT EXISTS platform_resolution_quota AFTER INSERT ON platform_resolutions
BEGIN
 SELECT CASE WHEN (SELECT count(*) FROM platform_resolutions WHERE dataset=NEW.dataset AND installation=NEW.installation)>10000 THEN RAISE(ABORT,'execution input quota exceeded (10000 slots)') END;
END;
CREATE TRIGGER IF NOT EXISTS platform_artifact_immutable_update BEFORE UPDATE ON platform_artifacts BEGIN SELECT RAISE(ABORT,'published artifacts are immutable'); END;
CREATE TRIGGER IF NOT EXISTS platform_artifact_immutable_delete BEFORE DELETE ON platform_artifacts BEGIN SELECT RAISE(ABORT,'published artifacts are immutable'); END;
