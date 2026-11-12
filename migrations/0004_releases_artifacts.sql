CREATE TABLE releases (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=36),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    version TEXT NOT NULL CHECK(length(version) BETWEEN 5 AND 200),
    version_key TEXT NOT NULL CHECK(length(version_key) BETWEEN 5 AND 200),
    title TEXT CHECK(title IS NULL OR length(title) BETWEEN 1 AND 120),
    release_notes TEXT NOT NULL DEFAULT '' CHECK(length(release_notes)<=20000),
    status TEXT NOT NULL DEFAULT 'draft' CHECK(status='draft'),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK(updated_at>=created_at),
    published_at INTEGER CHECK(published_at IS NULL),
    CHECK(version_key = CASE WHEN instr(version,'+')>0 THEN substr(version,1,instr(version,'+')-1) ELSE version END),
    UNIQUE(application_id, version_key)
);
CREATE INDEX releases_application_created ON releases(application_id, created_at DESC, id);
CREATE TRIGGER release_identity_immutable BEFORE UPDATE ON releases
WHEN NEW.id!=OLD.id OR NEW.application_id!=OLD.application_id OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT, 'release_identity_immutable'); END;

CREATE TABLE artifacts (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=36),
    release_id TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    filename TEXT NOT NULL CHECK(length(filename) BETWEEN 1 AND 255),
    target TEXT NOT NULL CHECK(length(target) BETWEEN 1 AND 64),
    file_type TEXT NOT NULL CHECK(length(file_type) BETWEEN 1 AND 32),
    size_bytes INTEGER NOT NULL CHECK(size_bytes>0),
    sha256 TEXT NOT NULL CHECK(length(sha256)=64 AND sha256 NOT GLOB '*[^0-9a-f]*'),
    storage_key TEXT NOT NULL UNIQUE CHECK(length(storage_key)=37 AND substr(storage_key,33)='.blob' AND substr(storage_key,1,32) NOT GLOB '*[^0-9a-f]*'),
    created_at INTEGER NOT NULL,
    UNIQUE(release_id, target, file_type)
);
CREATE INDEX artifacts_release_created ON artifacts(release_id, created_at, id);
CREATE TRIGGER artifact_immutable BEFORE UPDATE ON artifacts
BEGIN SELECT RAISE(ABORT, 'artifact_immutable'); END;
