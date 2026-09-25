-- Sparkwell library schema, version 1.
-- Applied inside a transaction by storage::migrations. Never edit a released
-- migration; add a new numbered file instead.

CREATE TABLE sparks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    title           TEXT    NOT NULL,
    summary         TEXT    NOT NULL DEFAULT '',
    body            TEXT    NOT NULL,
    body_hash       TEXT    NOT NULL,
    favorite        INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    favorited_at    INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    usage_count     INTEGER NOT NULL DEFAULT 0,
    last_copied_at  INTEGER,
    source_note     TEXT,
    category        TEXT
);

CREATE INDEX idx_sparks_favorite ON sparks (favorite, favorited_at);
CREATE INDEX idx_sparks_body_hash ON sparks (body_hash);

CREATE TABLE tags (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    name  TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE spark_tags (
    spark_id  INTEGER NOT NULL REFERENCES sparks (id) ON DELETE CASCADE,
    tag_id    INTEGER NOT NULL REFERENCES tags (id) ON DELETE CASCADE,
    position  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (spark_id, tag_id)
);

CREATE INDEX idx_spark_tags_tag ON spark_tags (tag_id);

-- One vector per Spark per embedding model. Vectors are little-endian f32,
-- L2-normalised. content_hash identifies the exact text that was embedded.
CREATE TABLE embeddings (
    spark_id      INTEGER NOT NULL REFERENCES sparks (id) ON DELETE CASCADE,
    model         TEXT    NOT NULL,
    dimensions    INTEGER NOT NULL,
    vector        BLOB    NOT NULL,
    content_hash  TEXT    NOT NULL,
    generated_at  INTEGER NOT NULL,
    PRIMARY KEY (spark_id, model)
);

-- Library-scoped key/value metadata (travels with the library file).
CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

-- Full-text index maintained by the application in the same transaction as
-- every Spark write. rowid = sparks.id.
CREATE VIRTUAL TABLE sparks_fts USING fts5 (
    title,
    summary,
    tags,
    body,
    tokenize = 'porter unicode61 remove_diacritics 2'
);
