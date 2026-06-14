-- Analytical content: collections, saved questions (cards), and dashboards.
-- The typed payload is stored as JSON in `body_json`; `kind` discriminates.

CREATE TABLE IF NOT EXISTS content (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    collection_id TEXT,
    name          TEXT NOT NULL,
    body_json     TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_content_kind ON content (kind);
