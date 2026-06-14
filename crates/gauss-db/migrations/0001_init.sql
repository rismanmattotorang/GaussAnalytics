-- GaussAnalytics application metadata store — initial schema.
-- UUIDs and timestamps are stored as TEXT (RFC 4122 / RFC 3339) for portability
-- across SQLite and (later) Postgres.

CREATE TABLE IF NOT EXISTS users (
    id            TEXT    PRIMARY KEY,
    email         TEXT    NOT NULL UNIQUE,
    display_name  TEXT    NOT NULL,
    is_admin      INTEGER NOT NULL DEFAULT 0,
    password_hash TEXT    NOT NULL,
    created_at    TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token      TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions (user_id);

CREATE TABLE IF NOT EXISTS data_sources (
    id         TEXT    PRIMARY KEY,
    name       TEXT    NOT NULL,
    kind       TEXT    NOT NULL,
    is_synced  INTEGER NOT NULL DEFAULT 0,
    created_at TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS source_tables (
    id          TEXT NOT NULL PRIMARY KEY,
    database_id TEXT NOT NULL,
    name        TEXT NOT NULL,
    fields_json TEXT NOT NULL,
    UNIQUE (database_id, name)
);
CREATE INDEX IF NOT EXISTS idx_source_tables_db ON source_tables (database_id);
