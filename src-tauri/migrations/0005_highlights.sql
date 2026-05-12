-- PDF ハイライト (v0.1.0)
-- 座標は pdf.js の PDF ポイント系（左下原点）。
CREATE TABLE highlights (
    id          INTEGER PRIMARY KEY,
    entry_id    INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    page        INTEGER NOT NULL,
    x           REAL NOT NULL,
    y           REAL NOT NULL,
    width       REAL NOT NULL,
    height      REAL NOT NULL,
    color       TEXT NOT NULL CHECK (color IN ('yellow', 'green', 'blue')),
    text        TEXT NOT NULL,
    note        TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_highlights_entry_page ON highlights(entry_id, page);
