-- entries
CREATE TABLE entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT    NOT NULL,
    year        INTEGER,
    entry_type  TEXT    NOT NULL DEFAULT 'misc',
    doi         TEXT,
    isbn        TEXT,
    arxiv_id    TEXT,
    url         TEXT,
    abstract    TEXT,
    notes       TEXT,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- authors (normalized)
CREATE TABLE authors (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL,
    given_name  TEXT,
    family_name TEXT,
    orcid       TEXT    UNIQUE,
    created_at  TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- entry_authors (many-to-many, ordered)
CREATE TABLE entry_authors (
    entry_id   INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    author_id  INTEGER NOT NULL REFERENCES authors(id) ON DELETE RESTRICT,
    position   INTEGER NOT NULL,
    PRIMARY KEY (entry_id, author_id)
);

-- collections (nested via parent_id)
CREATE TABLE collections (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT    NOT NULL,
    parent_id  INTEGER REFERENCES collections(id) ON DELETE CASCADE,
    created_at TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- entry_collections (many-to-many)
CREATE TABLE entry_collections (
    entry_id      INTEGER NOT NULL REFERENCES entries(id)      ON DELETE CASCADE,
    collection_id INTEGER NOT NULL REFERENCES collections(id)  ON DELETE CASCADE,
    PRIMARY KEY (entry_id, collection_id)
);

-- tags
CREATE TABLE tags (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL UNIQUE
);

-- entry_tags (many-to-many)
CREATE TABLE entry_tags (
    entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    tag_id   INTEGER NOT NULL REFERENCES tags(id)    ON DELETE CASCADE,
    PRIMARY KEY (entry_id, tag_id)
);

-- entry_relations (preprint → published version, etc.)
CREATE TABLE entry_relations (
    from_entry_id INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    to_entry_id   INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    relation_type TEXT    NOT NULL,
    PRIMARY KEY (from_entry_id, to_entry_id, relation_type)
);

-- attachments (file body stored in app data dir, only path in DB)
CREATE TABLE attachments (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    entry_id   INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    file_path  TEXT    NOT NULL,
    file_name  TEXT    NOT NULL,
    mime_type  TEXT    NOT NULL DEFAULT 'application/pdf',
    created_at TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- extra_fields (BibTeX type-specific fields: journal, volume, pages, etc.)
CREATE TABLE extra_fields (
    entry_id    INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    field_name  TEXT    NOT NULL,
    field_value TEXT    NOT NULL,
    PRIMARY KEY (entry_id, field_name)
);

-- settings (non-sensitive app config; API keys go to OS keychain)
CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- fulltext search index (FTS5, page-level granularity)
CREATE VIRTUAL TABLE fulltext USING fts5(
    content,
    attachment_id UNINDEXED,
    page          UNINDEXED,
    tokenize      = 'unicode61'
);

-- indexes
CREATE INDEX idx_entries_year         ON entries(year);
CREATE INDEX idx_entries_entry_type   ON entries(entry_type);
CREATE INDEX idx_entries_doi          ON entries(doi);
CREATE INDEX idx_entries_arxiv_id     ON entries(arxiv_id);
CREATE INDEX idx_entry_authors_author ON entry_authors(author_id);
CREATE INDEX idx_attachments_entry    ON attachments(entry_id);
CREATE INDEX idx_collections_parent   ON collections(parent_id);

