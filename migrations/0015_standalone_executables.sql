-- Extend standalone opt-in to the executable classification. Keep the artifacts
-- table itself, foreign keys and download counters; copy JWS bytes unchanged.
DROP TRIGGER published_artifact_update;
DROP TRIGGER published_standalone_no_replacement;
DROP TRIGGER standalone_key_retained;
DROP TRIGGER standalone_product_required_insert;
DROP TRIGGER standalone_product_required_update;
DROP TRIGGER standalone_product_pin_insert;
DROP TRIGGER standalone_product_pin_update;
DROP INDEX artifacts_one_standalone_per_target;

ALTER TABLE artifacts ADD COLUMN standalone_signature_next TEXT
    CHECK(standalone_signature_next IS NULL OR (
        length(standalone_signature_next) BETWEEN 1 AND 4096 AND
        ((distribution_role='executable' AND target IN (
            'windows-x86_64','windows-aarch64','linux-x86_64','linux-aarch64','darwin-x86_64','darwin-aarch64'
        )) OR
        (distribution_role='installer' AND (
            (target IN ('windows-x86_64','windows-aarch64') AND file_type='exe') OR
            (target IN ('linux-x86_64','linux-aarch64','darwin-x86_64','darwin-aarch64') AND file_type='bin')
        )))
    ));
UPDATE artifacts SET standalone_signature_next=standalone_signature;
ALTER TABLE artifacts DROP COLUMN standalone_signature;
ALTER TABLE artifacts RENAME COLUMN standalone_signature_next TO standalone_signature;

CREATE TRIGGER published_artifact_update BEFORE UPDATE ON artifacts
WHEN (SELECT status FROM releases WHERE id=OLD.release_id)='published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;

CREATE UNIQUE INDEX artifacts_one_standalone_per_target ON artifacts(release_id,target)
    WHERE standalone_signature IS NOT NULL;
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
