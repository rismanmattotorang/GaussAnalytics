-- Analytical content (MySQL): collections, saved questions, and dashboards.

CREATE TABLE IF NOT EXISTS content (
    id            VARCHAR(36)  PRIMARY KEY,
    kind          VARCHAR(32)  NOT NULL,
    collection_id VARCHAR(36),
    name          VARCHAR(255) NOT NULL,
    body_json     TEXT         NOT NULL,
    created_at    VARCHAR(40)  NOT NULL,
    INDEX idx_content_kind (kind)
);
