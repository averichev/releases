CREATE TABLE users (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK(length(username) BETWEEN 3 AND 64),
    password_hash TEXT,
    role TEXT NOT NULL CHECK(role IN ('admin', 'operator')),
    status TEXT NOT NULL CHECK(status IN ('invited', 'active', 'disabled')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_login_at INTEGER,
    CHECK(status != 'active' OR password_hash IS NOT NULL)
);
CREATE INDEX users_active_admins ON users(role, status);
CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK(expires_at > created_at)
);
CREATE INDEX sessions_user ON sessions(user_id);
CREATE INDEX sessions_expiry ON sessions(expires_at);
CREATE TABLE user_tokens (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    purpose TEXT NOT NULL CHECK(purpose IN ('invitation', 'reset')),
    expires_at INTEGER NOT NULL
);
CREATE INDEX user_tokens_expiry ON user_tokens(expires_at);
CREATE TABLE login_attempts (
    bucket TEXT PRIMARY KEY NOT NULL,
    attempts INTEGER NOT NULL CHECK(attempts > 0),
    expires_at INTEGER NOT NULL
);
-- A database invariant also covers concurrent requests and future code paths.
CREATE TRIGGER preserve_last_admin_update BEFORE UPDATE OF role, status ON users
WHEN OLD.role = 'admin' AND OLD.status = 'active'
 AND (NEW.role != 'admin' OR NEW.status != 'active')
 AND (SELECT count(*) FROM users WHERE role = 'admin' AND status = 'active') = 1
BEGIN SELECT RAISE(ABORT, 'last_active_admin'); END;
CREATE TRIGGER preserve_last_admin_delete BEFORE DELETE ON users
WHEN OLD.role = 'admin' AND OLD.status = 'active'
 AND (SELECT count(*) FROM users WHERE role = 'admin' AND status = 'active') = 1
BEGIN SELECT RAISE(ABORT, 'last_active_admin'); END;
