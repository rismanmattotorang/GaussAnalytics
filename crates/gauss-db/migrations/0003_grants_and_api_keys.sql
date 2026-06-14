-- Persisted per-user permission grants and DB-backed API keys.
-- `scope` holds the entity UUID for scoped permissions, or '' for unscoped.

CREATE TABLE IF NOT EXISTS permission_grants (
    user_id TEXT NOT NULL,
    kind    TEXT NOT NULL,
    scope   TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, kind, scope)
);

CREATE TABLE IF NOT EXISTS api_keys (
    id         TEXT    PRIMARY KEY,
    user_id    TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    key_hash   TEXT    NOT NULL UNIQUE,
    created_at TEXT    NOT NULL,
    revoked    INTEGER NOT NULL DEFAULT 0
);
