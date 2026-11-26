-- Extend the role CHECK while preserving published roles, IDs and signatures.
-- This application trigger references artifacts while SQLite validates table renames.
DROP TRIGGER standalone_key_retained;

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
    distribution_role TEXT NOT NULL DEFAULT 'installer' CHECK(distribution_role IN ('executable','installer','script','updater','archive')),
    updater_signature TEXT CHECK(updater_signature IS NULL OR (distribution_role='updater' AND length(updater_signature) BETWEEN 1 AND 16384)),
    standalone_signature TEXT
    CHECK(standalone_signature IS NULL OR (
        length(standalone_signature) BETWEEN 1 AND 4096 AND
        ((distribution_role='executable' AND target IN (
            'windows-x86_64','windows-aarch64','linux-x86_64','linux-aarch64','darwin-x86_64','darwin-aarch64'
        )) OR
        (distribution_role='installer' AND (
            (target IN ('windows-x86_64','windows-aarch64') AND file_type='exe') OR
            (target IN ('linux-x86_64','linux-aarch64','darwin-x86_64','darwin-aarch64') AND file_type='bin')
        )))
    )),
    UNIQUE(release_id,target,file_type,distribution_role)
);
INSERT INTO artifacts_next (
    id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at,
    distribution_role,updater_signature,standalone_signature
)
SELECT id,release_id,filename,target,file_type,size_bytes,sha256,storage_key,created_at,
       distribution_role,updater_signature,standalone_signature
FROM artifacts;

-- Move counters first: dropping the old parent would cascade to its counters.
CREATE TABLE artifact_download_counts_next (
    artifact_id TEXT PRIMARY KEY NOT NULL REFERENCES artifacts_next(id) ON DELETE CASCADE,
    download_count INTEGER NOT NULL CHECK(typeof(download_count)='integer' AND download_count>=0)
);
INSERT INTO artifact_download_counts_next (artifact_id,download_count)
SELECT artifact_id,download_count FROM artifact_download_counts;

DROP TABLE artifact_download_counts;
DROP TABLE artifacts;
ALTER TABLE artifacts_next RENAME TO artifacts;
ALTER TABLE artifact_download_counts_next RENAME TO artifact_download_counts;

CREATE UNIQUE INDEX artifacts_one_updater_per_target ON artifacts(release_id,target) WHERE distribution_role='updater';

CREATE INDEX artifacts_release_created ON artifacts(release_id,created_at,id);

CREATE UNIQUE INDEX artifacts_one_standalone_per_target ON artifacts(release_id,target)
    WHERE standalone_signature IS NOT NULL;

CREATE TRIGGER artifact_immutable BEFORE UPDATE ON artifacts
WHEN NEW.id!=OLD.id OR NEW.release_id!=OLD.release_id OR NEW.filename!=OLD.filename OR NEW.target!=OLD.target OR NEW.file_type!=OLD.file_type OR NEW.size_bytes!=OLD.size_bytes OR NEW.sha256!=OLD.sha256 OR NEW.storage_key!=OLD.storage_key OR NEW.created_at!=OLD.created_at
BEGIN SELECT RAISE(ABORT,'artifact_immutable'); END;

CREATE TRIGGER published_artifact_delete BEFORE DELETE ON artifacts
WHEN (SELECT status FROM releases WHERE id=OLD.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;

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

CREATE TRIGGER published_artifact_update BEFORE UPDATE ON artifacts
WHEN (SELECT status FROM releases WHERE id=OLD.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;

CREATE TRIGGER published_standalone_no_replacement BEFORE INSERT ON artifacts
WHEN NEW.standalone_signature IS NOT NULL AND EXISTS (
    SELECT 1 FROM artifacts a JOIN releases r ON r.id=a.release_id
    WHERE r.status='published' AND a.release_id=NEW.release_id AND a.target=NEW.target
        AND a.standalone_signature IS NOT NULL
)
BEGIN SELECT RAISE(ABORT,'artifact_exists'); END;

CREATE TRIGGER standalone_key_retained BEFORE UPDATE OF standalone_public_key ON applications
WHEN NEW.standalone_public_key IS NOT OLD.standalone_public_key AND EXISTS (
    SELECT 1 FROM artifacts a JOIN releases r ON r.id=a.release_id
    WHERE r.application_id=OLD.id AND a.standalone_signature IS NOT NULL
)
BEGIN SELECT RAISE(ABORT,'standalone_key_locked'); END;

CREATE TRIGGER standalone_product_required_insert BEFORE INSERT ON artifacts
WHEN NEW.standalone_signature IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM applications p JOIN releases r ON r.application_id=p.id
    WHERE r.id=NEW.release_id AND p.product_id IS NOT NULL
)
BEGIN SELECT RAISE(ABORT,'standalone_product_required'); END;

CREATE TRIGGER standalone_product_required_update BEFORE UPDATE OF standalone_signature ON artifacts
WHEN NEW.standalone_signature IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM applications p JOIN releases r ON r.application_id=p.id
    WHERE r.id=NEW.release_id AND p.product_id IS NOT NULL
)
BEGIN SELECT RAISE(ABORT,'standalone_product_required'); END;

CREATE TRIGGER standalone_product_pin_insert AFTER INSERT ON artifacts
WHEN NEW.standalone_signature IS NOT NULL
BEGIN
    UPDATE applications SET product_id_locked=1
    WHERE id=(SELECT application_id FROM releases WHERE id=NEW.release_id)
        AND product_id_locked=0;
END;

CREATE TRIGGER standalone_product_pin_update AFTER UPDATE OF standalone_signature ON artifacts
WHEN NEW.standalone_signature IS NOT NULL
BEGIN
    UPDATE applications SET product_id_locked=1
    WHERE id=(SELECT application_id FROM releases WHERE id=NEW.release_id)
        AND product_id_locked=0;
END;
