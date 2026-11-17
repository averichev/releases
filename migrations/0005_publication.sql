-- Rebuild both tables with foreign keys enabled. Copy children before dropping
-- their old parent; no PRAGMA foreign_keys=OFF and no changed migration checksums.
CREATE TABLE releases_next (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=36),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    version TEXT NOT NULL CHECK(length(version) BETWEEN 5 AND 200),
    version_key TEXT NOT NULL CHECK(length(version_key) BETWEEN 5 AND 200),
    title TEXT CHECK(title IS NULL OR length(title) BETWEEN 1 AND 120),
    release_notes TEXT NOT NULL DEFAULT '' CHECK(length(release_notes)<=20000),
    status TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','published')),
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision>=0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK(updated_at>=created_at),
    published_at INTEGER,
    CHECK((status='draft' AND published_at IS NULL) OR (status='published' AND published_at IS NOT NULL AND published_at>=created_at)),
    CHECK(version_key = CASE WHEN instr(version,'+')>0 THEN substr(version,1,instr(version,'+')-1) ELSE version END),
    UNIQUE(application_id,version_key),
    UNIQUE(id,application_id)
);
INSERT INTO releases_next(id,application_id,version,version_key,title,release_notes,status,created_at,updated_at,published_at) SELECT * FROM releases;
CREATE TABLE artifacts_next (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=36),
    release_id TEXT NOT NULL REFERENCES releases_next(id) ON DELETE CASCADE,
    filename TEXT NOT NULL CHECK(length(filename) BETWEEN 1 AND 255),
    target TEXT NOT NULL CHECK(length(target) BETWEEN 1 AND 64),
    file_type TEXT NOT NULL CHECK(length(file_type) BETWEEN 1 AND 32),
    size_bytes INTEGER NOT NULL CHECK(size_bytes>0),
    sha256 TEXT NOT NULL CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    storage_key TEXT NOT NULL UNIQUE CHECK(length(storage_key)=37 AND substr(storage_key,33)='.blob' AND substr(storage_key,1,32) NOT GLOB '*[^0-9a-f]*'),
    created_at INTEGER NOT NULL,
    distribution_role TEXT NOT NULL DEFAULT 'installer' CHECK(distribution_role IN ('installer','updater')),
    updater_signature TEXT CHECK(updater_signature IS NULL OR (distribution_role='updater' AND length(updater_signature) BETWEEN 1 AND 16384)),
    UNIQUE(release_id,target,file_type)
);
INSERT INTO artifacts_next(id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at)
SELECT id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at FROM artifacts;
DROP TABLE artifacts;
DROP TABLE releases;
ALTER TABLE releases_next RENAME TO releases;
ALTER TABLE artifacts_next RENAME TO artifacts;
CREATE INDEX releases_application_created ON releases(application_id,created_at DESC,id);
CREATE INDEX artifacts_release_created ON artifacts(release_id,created_at,id);
CREATE TRIGGER release_identity_immutable BEFORE UPDATE ON releases
WHEN NEW.id!=OLD.id OR NEW.application_id!=OLD.application_id OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'release_identity_immutable'); END;
CREATE TRIGGER published_release_immutable BEFORE UPDATE ON releases WHEN OLD.status='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
CREATE TRIGGER published_release_retained BEFORE DELETE ON releases WHEN OLD.status='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
CREATE TRIGGER artifact_immutable BEFORE UPDATE ON artifacts
WHEN NEW.id!=OLD.id OR NEW.release_id!=OLD.release_id OR NEW.filename!=OLD.filename OR NEW.target!=OLD.target OR NEW.file_type!=OLD.file_type OR NEW.size_bytes!=OLD.size_bytes OR NEW.sha256!=OLD.sha256 OR NEW.storage_key!=OLD.storage_key OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'artifact_immutable'); END;
CREATE TRIGGER published_artifact_update BEFORE UPDATE ON artifacts
WHEN (SELECT status FROM releases WHERE id=OLD.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
CREATE TRIGGER published_artifact_insert BEFORE INSERT ON artifacts
WHEN (SELECT status FROM releases WHERE id=NEW.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
CREATE TRIGGER published_artifact_delete BEFORE DELETE ON artifacts
WHEN (SELECT status FROM releases WHERE id=OLD.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;

CREATE TABLE release_channels (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=36),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 64 AND name NOT GLOB '*[^a-z0-9-]*'),
    current_release_id TEXT,
    revision INTEGER NOT NULL DEFAULT 0 CHECK(revision>=0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(application_id,name), UNIQUE(id,application_id),
    FOREIGN KEY(current_release_id,application_id) REFERENCES releases(id,application_id) ON DELETE RESTRICT,
    FOREIGN KEY(id,current_release_id) REFERENCES channel_releases(channel_id,release_id) ON DELETE RESTRICT
);
CREATE TABLE channel_releases (
    channel_id TEXT NOT NULL,
    release_id TEXT NOT NULL,
    application_id TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY(channel_id,release_id),
    FOREIGN KEY(channel_id,application_id) REFERENCES release_channels(id,application_id) ON DELETE RESTRICT,
    FOREIGN KEY(release_id,application_id) REFERENCES releases(id,application_id) ON DELETE RESTRICT
);
CREATE INDEX channel_releases_release ON channel_releases(release_id,channel_id);
CREATE TRIGGER channel_release_published BEFORE INSERT ON channel_releases
WHEN (SELECT status FROM releases WHERE id=NEW.release_id)!='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
CREATE TRIGGER channel_release_retained BEFORE DELETE ON channel_releases
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;
CREATE TRIGGER channel_release_immutable BEFORE UPDATE ON channel_releases
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;
CREATE TRIGGER channel_identity_immutable BEFORE UPDATE ON release_channels
WHEN NEW.id!=OLD.id OR NEW.application_id!=OLD.application_id OR NEW.name!=OLD.name OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'channel_identity_immutable'); END;

-- Backfill and new applications share the same default-channel invariant.
INSERT INTO release_channels(id,application_id,name,created_at,updated_at)
SELECT lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-4'||substr(lower(hex(randomblob(2))),2)||'-a'||substr(lower(hex(randomblob(2))),2)||'-'||lower(hex(randomblob(6))),id,'stable',created_at,created_at FROM applications;
INSERT INTO release_channels(id,application_id,name,created_at,updated_at)
SELECT lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-4'||substr(lower(hex(randomblob(2))),2)||'-a'||substr(lower(hex(randomblob(2))),2)||'-'||lower(hex(randomblob(6))),id,'beta',created_at,created_at FROM applications;
CREATE TRIGGER application_default_channels AFTER INSERT ON applications BEGIN
    INSERT INTO release_channels(id,application_id,name,created_at,updated_at) VALUES(lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-4'||substr(lower(hex(randomblob(2))),2)||'-a'||substr(lower(hex(randomblob(2))),2)||'-'||lower(hex(randomblob(6))),NEW.id,'stable',NEW.created_at,NEW.created_at);
    INSERT INTO release_channels(id,application_id,name,created_at,updated_at) VALUES(lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-4'||substr(lower(hex(randomblob(2))),2)||'-a'||substr(lower(hex(randomblob(2))),2)||'-'||lower(hex(randomblob(6))),NEW.id,'beta',NEW.created_at,NEW.created_at);
END;

CREATE TABLE publication_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    channel_id TEXT NOT NULL,
    release_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('release_published','channel_promoted','channel_rolled_back')),
    previous_release_id TEXT,
    new_release_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    FOREIGN KEY(channel_id,application_id) REFERENCES release_channels(id,application_id),
    FOREIGN KEY(release_id,application_id) REFERENCES releases(id,application_id),
    FOREIGN KEY(previous_release_id,application_id) REFERENCES releases(id,application_id),
    FOREIGN KEY(new_release_id,application_id) REFERENCES releases(id,application_id)
);
CREATE INDEX publication_history_application ON publication_history(application_id,id DESC);
CREATE TRIGGER publication_history_no_update BEFORE UPDATE ON publication_history
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;
CREATE TRIGGER publication_history_no_delete BEFORE DELETE ON publication_history
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;
