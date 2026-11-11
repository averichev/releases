CREATE TABLE applications (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id) = 36),
    slug TEXT NOT NULL UNIQUE
        CHECK(length(slug) BETWEEN 1 AND 64
          AND slug NOT GLOB '*[^a-z0-9-]*'
          AND substr(slug, 1, 1) != '-' AND substr(slug, -1, 1) != '-'
          AND slug NOT IN ('api','admin','assets','auth','login','logout','static','health','activate','favicon','index')),
    name TEXT NOT NULL CHECK(length(trim(name)) BETWEEN 1 AND 120),
    -- Rust Unicode lowercase supports case-insensitive Cyrillic search too.
    name_search TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '' CHECK(length(description) <= 2000),
    icon_key TEXT CHECK(icon_key IS NULL OR (length(icon_key) = 36
        AND substr(icon_key, 33) = '.png' AND substr(icon_key, 1, 32) NOT GLOB '*[^a-f0-9]*')),
    visibility TEXT NOT NULL DEFAULT 'private' CHECK(visibility IN ('public','private')),
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK(updated_at >= created_at)
);
CREATE INDEX applications_status_name ON applications(status, name_search, id);
CREATE INDEX applications_visibility ON applications(visibility, status);
CREATE INDEX applications_created ON applications(created_at, id);
CREATE INDEX applications_updated ON applications(updated_at, id);
CREATE TRIGGER applications_identity_immutable BEFORE UPDATE OF id, slug, created_at ON applications
WHEN NEW.id != OLD.id OR NEW.slug != OLD.slug OR NEW.created_at != OLD.created_at
BEGIN SELECT RAISE(ABORT, 'application_identity_immutable'); END;
