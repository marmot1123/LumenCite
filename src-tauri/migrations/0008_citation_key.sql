-- BibTeX エントリキー（cite key）をエントリごとに永続化する。
-- NULL = export 時に自動生成、非 NULL = ユーザーがピン留めした固定キー。
ALTER TABLE entries ADD COLUMN citation_key TEXT;

-- 固定キー（非 NULL）はグローバル一意。NULL（=自動）は複数行で許容するため部分インデックス。
CREATE UNIQUE INDEX ux_entries_citation_key
    ON entries(citation_key)
    WHERE citation_key IS NOT NULL;
