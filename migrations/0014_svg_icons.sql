-- Replace only the icon constraint; keep application identities, references,
-- indexes and triggers in place, with foreign keys enabled throughout.
ALTER TABLE applications ADD COLUMN icon_key_next TEXT
    CHECK(icon_key_next IS NULL OR (length(icon_key_next) = 36
        AND substr(icon_key_next, 33) IN ('.png', '.svg')
        AND substr(icon_key_next, 1, 32) NOT GLOB '*[^a-f0-9]*'));
UPDATE applications SET icon_key_next = icon_key;
ALTER TABLE applications DROP COLUMN icon_key;
ALTER TABLE applications RENAME COLUMN icon_key_next TO icon_key;
