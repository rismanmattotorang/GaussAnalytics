-- GaussAnalytics application metadata store — MySQL schema.
-- MySQL needs explicit VARCHAR lengths for indexed/keyed columns and uses
-- `ON DUPLICATE KEY UPDATE` (handled in queries) rather than `ON CONFLICT`.

CREATE TABLE IF NOT EXISTS users (
    id            VARCHAR(36)  PRIMARY KEY,
    email         VARCHAR(320) NOT NULL UNIQUE,
    display_name  VARCHAR(255) NOT NULL,
    is_admin      INT          NOT NULL DEFAULT 0,
    password_hash TEXT         NOT NULL,
    created_at    VARCHAR(40)  NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    token      VARCHAR(128) PRIMARY KEY,
    user_id    VARCHAR(36)  NOT NULL,
    created_at VARCHAR(40)  NOT NULL,
    expires_at VARCHAR(40)  NOT NULL
);

CREATE TABLE IF NOT EXISTS data_sources (
    id             VARCHAR(36)  PRIMARY KEY,
    name           VARCHAR(255) NOT NULL,
    kind           VARCHAR(32)  NOT NULL,
    is_synced      INT          NOT NULL DEFAULT 0,
    connection_uri TEXT,
    created_at     VARCHAR(40)  NOT NULL
);

CREATE TABLE IF NOT EXISTS source_tables (
    id          VARCHAR(36)  NOT NULL PRIMARY KEY,
    database_id VARCHAR(36)  NOT NULL,
    name        VARCHAR(255) NOT NULL,
    fields_json TEXT         NOT NULL,
    UNIQUE (database_id, name)
);
