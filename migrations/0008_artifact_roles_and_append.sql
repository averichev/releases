-- Rebuild only artifacts: no table references artifact IDs. Preserve every value.
-- A legacy release with multiple updaters for one target must be repaired before
-- migrating; the unique index deliberately fails atomically instead of losing data.
CREATE TABLE artifacts_next (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=36),
    release_id TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    filename TEXT NOT NULL CHECK(length(filename) BETWEEN 1 AND 255),
    target TEXT NOT NULL CHECK(length(target) BETWEEN 1 AND 64),
    file_type TEXT NOT NULL CHECK(length(file_type) BETWEEN 1 AND 32),
    size_bytes INTEGER NOT NULL CHECK(size_bytes>0),
    sha256 TEXT NOT NULL CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    storage_key TEXT NOT NULL UNIQUE CHECK(length(storage_key)=37 AND substr(storage_key,33)='.blob' AND substr(storage_key,1,32) NOT GLOB '*[^0-9a-f]*'),
    created_at INTEGER NOT NULL,
    distribution_role TEXT NOT NULL DEFAULT 'installer' CHECK(distribution_role IN ('installer','updater')),
    updater_signature TEXT CHECK(updater_signature IS NULL OR (distribution_role='updater' AND length(updater_signature) BETWEEN 1 AND 16384)),
    UNIQUE(release_id,target,file_type,distribution_role)
);
INSERT INTO artifacts_next SELECT * FROM artifacts;
CREATE UNIQUE INDEX artifacts_one_updater_per_target ON artifacts_next(release_id,target) WHERE distribution_role='updater';
DROP TABLE artifacts;
ALTER TABLE artifacts_next RENAME TO artifacts;
CREATE INDEX artifacts_release_created ON artifacts(release_id,created_at,id);
CREATE TRIGGER artifact_immutable BEFORE UPDATE ON artifacts
WHEN NEW.id!=OLD.id OR NEW.release_id!=OLD.release_id OR NEW.filename!=OLD.filename OR NEW.target!=OLD.target OR NEW.file_type!=OLD.file_type OR NEW.size_bytes!=OLD.size_bytes OR NEW.sha256!=OLD.sha256 OR NEW.storage_key!=OLD.storage_key OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'artifact_immutable'); END;
CREATE TRIGGER published_artifact_update BEFORE UPDATE ON artifacts
WHEN (SELECT status FROM releases WHERE id=OLD.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
CREATE TRIGGER published_artifact_delete BEFORE DELETE ON artifacts
WHEN (SELECT status FROM releases WHERE id=OLD.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
-- REPLACE can bypass DELETE triggers when recursive_triggers is off. Reject
-- collisions before SQLite can remove an already distributed object.
CREATE TRIGGER published_artifact_no_replacement BEFORE INSERT ON artifacts
WHEN EXISTS (
    SELECT 1 FROM artifacts a JOIN releases r ON r.id=a.release_id
    WHERE r.status='published' AND (
        a.id=NEW.id OR a.storage_key=NEW.storage_key OR
        (a.release_id=NEW.release_id AND a.target=NEW.target AND a.distribution_role=NEW.distribution_role
            AND (a.file_type=NEW.file_type OR a.distribution_role='updater'))
    )
)
BEGIN SELECT RAISE(ABORT,'artifact_exists'); END;
CREATE TRIGGER published_updater_requires_signature BEFORE INSERT ON artifacts
WHEN NEW.distribution_role='updater' AND NEW.updater_signature IS NULL
    AND (SELECT status FROM releases WHERE id=NEW.release_id)='published'
BEGIN SELECT RAISE(ABORT,'updater_signature_required'); END;
DROP TRIGGER published_release_immutable;
-- Only a technical revision increment is allowed after first publication.
CREATE TRIGGER published_release_immutable BEFORE UPDATE ON releases
WHEN OLD.status='published' AND (
    NEW.id IS NOT OLD.id OR NEW.application_id IS NOT OLD.application_id OR
    NEW.version IS NOT OLD.version OR NEW.version_key IS NOT OLD.version_key OR
    NEW.title IS NOT OLD.title OR NEW.release_notes IS NOT OLD.release_notes OR
    NEW.status IS NOT OLD.status OR NEW.created_at IS NOT OLD.created_at OR
    NEW.updated_at IS NOT OLD.updated_at OR NEW.published_at IS NOT OLD.published_at OR
    NEW.revision!=OLD.revision+1
)
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
