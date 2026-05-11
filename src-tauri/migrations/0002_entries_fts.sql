-- entries_fts: メタデータ統合 FTS インデックス
-- rowid = entries.id にバインド。同期は Rust 側の create/update/delete 内で実施。
CREATE VIRTUAL TABLE entries_fts USING fts5(
    title,
    authors_text,
    tags_text,
    abstract_text,
    identifiers,
    tokenize = 'trigram'
);

-- 既存エントリのバックフィル
INSERT INTO entries_fts (rowid, title, authors_text, tags_text, abstract_text, identifiers)
SELECT
    e.id,
    COALESCE(e.title, ''),
    COALESCE((
        SELECT GROUP_CONCAT(a.name, ' ')
        FROM entry_authors ea
        JOIN authors a ON a.id = ea.author_id
        WHERE ea.entry_id = e.id
    ), ''),
    COALESCE((
        SELECT GROUP_CONCAT(t.name, ' ')
        FROM entry_tags et
        JOIN tags t ON t.id = et.tag_id
        WHERE et.entry_id = e.id
    ), ''),
    COALESCE(e.abstract, ''),
    TRIM(
        COALESCE(e.doi, '')         || ' ' ||
        COALESCE(e.isbn, '')        || ' ' ||
        COALESCE(e.arxiv_id, '')    || ' ' ||
        COALESCE(CAST(e.year AS TEXT), '')
    )
FROM entries e;
