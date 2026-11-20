-- The publisher owns product identity; local application IDs remain foreign keys.
ALTER TABLE applications ADD COLUMN product_id TEXT
    CHECK(product_id IS NULL OR (
        length(product_id)=36 AND length(CAST(product_id AS BLOB))=36 AND substr(product_id,9,1)='-' AND
        substr(product_id,14,1)='-' AND substr(product_id,19,1)='-' AND
        substr(product_id,24,1)='-' AND length(replace(product_id,'-',''))=32 AND
        replace(product_id,'-','') NOT GLOB '*[^0-9a-f]*'
    ));
ALTER TABLE applications ADD COLUMN product_id_locked INTEGER NOT NULL DEFAULT 0
    CHECK(product_id_locked IN (0,1) AND (product_id_locked=0 OR product_id IS NOT NULL));

-- Preserve original JWS bytes, including signed drafts. A key alone is not identity.
UPDATE applications SET product_id=id, product_id_locked=1
WHERE EXISTS (
    SELECT 1 FROM artifacts a JOIN releases r ON r.id=a.release_id
    WHERE r.application_id=applications.id AND a.standalone_signature IS NOT NULL
);
CREATE UNIQUE INDEX applications_product_id ON applications(product_id)
    WHERE product_id IS NOT NULL;

-- The first signature pins identity permanently, even if a draft is later deleted.
CREATE TRIGGER standalone_product_retained BEFORE UPDATE OF product_id,product_id_locked ON applications
WHEN OLD.product_id_locked=1 AND (
    NEW.product_id IS NOT OLD.product_id OR NEW.product_id_locked!=1
)
BEGIN SELECT RAISE(ABORT,'standalone_product_locked'); END;
CREATE TRIGGER standalone_product_no_replacement BEFORE INSERT ON applications
WHEN EXISTS (
    SELECT 1 FROM applications WHERE product_id_locked=1 AND
        (id=NEW.id OR slug=NEW.slug OR product_id=NEW.product_id)
)
BEGIN SELECT RAISE(ABORT,'standalone_product_locked'); END;

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
