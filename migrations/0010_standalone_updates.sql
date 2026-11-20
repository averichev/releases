-- Opt-in only: legacy installers and Tauri signatures retain their meaning.
ALTER TABLE applications ADD COLUMN standalone_public_key TEXT
    CHECK(standalone_public_key IS NULL OR (length(standalone_public_key)=43 AND standalone_public_key NOT GLOB '*[^A-Za-z0-9_-]*'));
CREATE UNIQUE INDEX applications_standalone_key ON applications(standalone_public_key)
    WHERE standalone_public_key IS NOT NULL;

ALTER TABLE artifacts ADD COLUMN standalone_signature TEXT
    CHECK(standalone_signature IS NULL OR (
        length(standalone_signature) BETWEEN 1 AND 4096 AND distribution_role='installer' AND
        ((target IN ('windows-x86_64','windows-aarch64') AND file_type='exe') OR
         (target IN ('linux-x86_64','linux-aarch64','darwin-x86_64','darwin-aarch64') AND file_type='bin'))
    ));
CREATE UNIQUE INDEX artifacts_one_standalone_per_target ON artifacts(release_id,target)
    WHERE standalone_signature IS NOT NULL;

-- REPLACE must not remove an existing public artifact through the new index.
CREATE TRIGGER published_standalone_no_replacement BEFORE INSERT ON artifacts
WHEN NEW.standalone_signature IS NOT NULL AND EXISTS (
    SELECT 1 FROM artifacts a JOIN releases r ON r.id=a.release_id
    WHERE r.status='published' AND a.release_id=NEW.release_id AND a.target=NEW.target
        AND a.standalone_signature IS NOT NULL
)
BEGIN SELECT RAISE(ABORT,'artifact_exists'); END;

-- One pinned key. Rotation needs an explicit client trust migration protocol.
CREATE TRIGGER standalone_key_retained BEFORE UPDATE OF standalone_public_key ON applications
WHEN NEW.standalone_public_key IS NOT OLD.standalone_public_key AND EXISTS (
    SELECT 1 FROM artifacts a JOIN releases r ON r.id=a.release_id
    WHERE r.application_id=OLD.id AND a.standalone_signature IS NOT NULL
)
BEGIN SELECT RAISE(ABORT,'standalone_key_locked'); END;
