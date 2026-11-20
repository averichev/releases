-- Transitional compatibility: expanding beta-only (or unassigned) publications
-- into the public catalog requires an explicit owner decision. Abort atomically;
-- see docs/migration-0009.md for the preflight and per-release opt-in policy.
CREATE TABLE publication_migration_guard (value INTEGER);
CREATE TRIGGER publication_migration_policy BEFORE INSERT ON publication_migration_guard
WHEN EXISTS (
    SELECT 1 FROM releases r WHERE r.status='published' AND NOT EXISTS (
        SELECT 1 FROM channel_releases cr JOIN release_channels c ON c.id=cr.channel_id
        WHERE cr.release_id=r.id AND c.name='stable'
    )
)
BEGIN SELECT RAISE(ABORT,'publication_migration_policy: review non-public releases; see docs/migration-0009.md'); END;
INSERT INTO publication_migration_guard VALUES(1);
DROP TABLE publication_migration_guard;

-- Retain facts of first publication, including IDs, actors and timestamps.
-- Pointer promotions/rollbacks did not publish releases and are retired.
CREATE TABLE publication_history_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    release_id TEXT NOT NULL UNIQUE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY(release_id,application_id) REFERENCES releases(id,application_id)
);
INSERT INTO publication_history_next(id,application_id,release_id,user_id,created_at)
SELECT id,application_id,release_id,user_id,created_at FROM publication_history WHERE action='release_published';
DROP TABLE publication_history;
ALTER TABLE publication_history_next RENAME TO publication_history;
CREATE INDEX publication_history_application ON publication_history(application_id,id DESC);
CREATE TRIGGER publication_history_no_update BEFORE UPDATE ON publication_history
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;
CREATE TRIGGER publication_history_no_delete BEFORE DELETE ON publication_history
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;

DROP TRIGGER application_default_channels;
DROP TRIGGER channel_release_retained;
UPDATE release_channels SET current_release_id=NULL;
DROP TABLE channel_releases;
DROP TABLE release_channels;
DROP TABLE application_distribution;
CREATE INDEX releases_published ON releases(application_id,published_at DESC,id) WHERE status='published';
