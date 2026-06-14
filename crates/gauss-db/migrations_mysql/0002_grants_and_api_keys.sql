-- Persisted per-user permission grants and DB-backed API keys (MySQL).

CREATE TABLE IF NOT EXISTS permission_grants (
    user_id VARCHAR(36) NOT NULL,
    kind    VARCHAR(32) NOT NULL,
    scope   VARCHAR(36) NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, kind, scope)
);

CREATE TABLE IF NOT EXISTS api_keys (
    id         VARCHAR(36)  PRIMARY KEY,
    user_id    VARCHAR(36)  NOT NULL,
    name       VARCHAR(255) NOT NULL,
    key_hash   VARCHAR(64)  NOT NULL UNIQUE,
    created_at VARCHAR(40)  NOT NULL,
    revoked    INT          NOT NULL DEFAULT 0
);
