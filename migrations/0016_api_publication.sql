CREATE TABLE api_tokens (
    id TEXT PRIMARY KEY NOT NULL CHECK(length(id)=36),
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    token_hash TEXT NOT NULL UNIQUE CHECK(length(token_hash)=64 AND token_hash NOT GLOB '*[^0-9a-f]*'),
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 80),
    created_at INTEGER NOT NULL,
    created_by TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    expires_at INTEGER CHECK(expires_at IS NULL OR expires_at>created_at),
    revoked_at INTEGER CHECK(revoked_at IS NULL OR revoked_at>=created_at),
    revoked_by TEXT REFERENCES users(id) ON DELETE RESTRICT,
    CHECK((revoked_at IS NULL)=(revoked_by IS NULL)),
    UNIQUE(id,application_id)
);
CREATE INDEX api_tokens_application ON api_tokens(application_id,created_at,id);
CREATE TRIGGER api_token_limit BEFORE INSERT ON api_tokens
WHEN NEW.revoked_at IS NULL AND (NEW.expires_at IS NULL OR NEW.expires_at>unixepoch())
 AND (SELECT count(*) FROM api_tokens WHERE application_id=NEW.application_id
      AND revoked_at IS NULL AND (expires_at IS NULL OR expires_at>unixepoch()))>=2
BEGIN SELECT RAISE(ABORT,'api_token_limit'); END;
CREATE TRIGGER api_token_immutable BEFORE UPDATE ON api_tokens
WHEN NEW.id IS NOT OLD.id OR NEW.application_id IS NOT OLD.application_id
 OR NEW.token_hash IS NOT OLD.token_hash OR NEW.name IS NOT OLD.name
 OR NEW.created_at IS NOT OLD.created_at OR NEW.created_by IS NOT OLD.created_by
 OR NEW.expires_at IS NOT OLD.expires_at OR OLD.revoked_at IS NOT NULL
 OR NEW.revoked_at IS NULL
BEGIN SELECT RAISE(ABORT,'api_token_immutable'); END;
CREATE TRIGGER api_token_retained BEFORE DELETE ON api_tokens
BEGIN SELECT RAISE(ABORT,'api_token_retained'); END;

-- Preserve legacy IDs, timestamps and user authors, including on an existing DB.
CREATE TABLE publication_history_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    release_id TEXT NOT NULL UNIQUE,
    user_id TEXT REFERENCES users(id) ON DELETE RESTRICT,
    token_id TEXT,
    created_at INTEGER NOT NULL,
    CHECK((user_id IS NOT NULL)+(token_id IS NOT NULL)=1),
    FOREIGN KEY(release_id,application_id) REFERENCES releases(id,application_id),
    FOREIGN KEY(token_id,application_id) REFERENCES api_tokens(id,application_id)
);
INSERT INTO publication_history_next(id,application_id,release_id,user_id,created_at)
SELECT id,application_id,release_id,user_id,created_at FROM publication_history;
DROP TABLE publication_history;
ALTER TABLE publication_history_next RENAME TO publication_history;
CREATE INDEX publication_history_application ON publication_history(application_id,id DESC);
CREATE TRIGGER publication_history_no_update BEFORE UPDATE ON publication_history
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;
CREATE TRIGGER publication_history_no_delete BEFORE DELETE ON publication_history
BEGIN SELECT RAISE(ABORT,'publication_history_immutable'); END;

-- The original accepted bundle is independent of later administrative additions.
CREATE TABLE api_publications (
    release_id TEXT PRIMARY KEY NOT NULL REFERENCES releases(id) ON DELETE RESTRICT,
    manifest TEXT NOT NULL CHECK(json_valid(manifest))
);
CREATE TRIGGER api_publication_published BEFORE INSERT ON api_publications
WHEN (SELECT status FROM releases WHERE id=NEW.release_id) IS NOT 'published'
BEGIN SELECT RAISE(ABORT,'release_locked'); END;
CREATE TRIGGER api_publication_immutable BEFORE UPDATE ON api_publications
BEGIN SELECT RAISE(ABORT,'api_publication_immutable'); END;
CREATE TRIGGER api_publication_retained BEFORE DELETE ON api_publications
BEGIN SELECT RAISE(ABORT,'api_publication_immutable'); END;
