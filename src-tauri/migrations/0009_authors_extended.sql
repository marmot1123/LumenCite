-- v0.3.0: authors テーブルを多言語名・国際識別子対応へ拡張する。
-- 詳細: docs/DATA_MODEL.md § authors / author_identifiers
--       ~/.claude/plans/v0-3-0-authors-radiant-kana.md

-- === 1. authors への列追加 ===
-- 名前構造（CSL 互換）
ALTER TABLE authors ADD COLUMN middle_name          TEXT;
ALTER TABLE authors ADD COLUMN suffix               TEXT;
ALTER TABLE authors ADD COLUMN name_particle        TEXT;

-- オリジナル言語表記（漢字名 / ハングル / キリル等）
ALTER TABLE authors ADD COLUMN name_original        TEXT;
ALTER TABLE authors ADD COLUMN given_name_original  TEXT;
ALTER TABLE authors ADD COLUMN family_name_original TEXT;
ALTER TABLE authors ADD COLUMN original_script      TEXT;   -- ISO 15924 (Hani / Hang / Cyrl / ...)

-- 読み仮名（五十音ソート・かな検索のため必須）
ALTER TABLE authors ADD COLUMN reading_family       TEXT;
ALTER TABLE authors ADD COLUMN reading_given        TEXT;

-- 団体著者フラグ（CSL の literal 相当）
ALTER TABLE authors ADD COLUMN is_organization      INTEGER NOT NULL DEFAULT 0;

-- 追加属性
ALTER TABLE authors ADD COLUMN email                TEXT;
ALTER TABLE authors ADD COLUMN homepage_url         TEXT;
ALTER TABLE authors ADD COLUMN notes                TEXT;
ALTER TABLE authors ADD COLUMN updated_at           TEXT;

-- updated_at は既存行では NULL のままだとソートに不便なので created_at で埋める
UPDATE authors SET updated_at = created_at WHERE updated_at IS NULL;

-- 五十音検索のためのインデックス（接頭辞検索を意識した部分インデックス）
CREATE INDEX idx_authors_reading_family
    ON authors(reading_family)
    WHERE reading_family IS NOT NULL;

-- === 2. 国際識別子テーブル ===
-- ORCID 以外（DBLP / Scopus / Wikidata / ISNI / VIAF / ResearcherID / Google Scholar 等）を
-- 正規化保持する。追加スキームのたびに migration を切らずに済む。
CREATE TABLE author_identifiers (
    author_id INTEGER NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
    scheme    TEXT    NOT NULL,   -- 'orcid' / 'scopus' / 'dblp' / 'semantic_scholar' / 'wikidata' / 'isni' / 'viaf' / 'researcher_id' / 'google_scholar'
    value     TEXT    NOT NULL,
    url       TEXT,
    PRIMARY KEY (author_id, scheme)
);

-- 同じ識別子が複数著者に紐づくのは禁止（マージ作業の起点として利用）
CREATE UNIQUE INDEX idx_author_identifiers_scheme_value
    ON author_identifiers(scheme, value);

-- === 3. 既存 authors.orcid を author_identifiers にバックフィル ===
-- authors.orcid 列は互換のため v0.3.0 では残し、両方に書く運用に切り替える（README 参照）。
INSERT INTO author_identifiers (author_id, scheme, value)
SELECT id, 'orcid', TRIM(orcid)
  FROM authors
 WHERE orcid IS NOT NULL
   AND TRIM(orcid) <> ''
ON CONFLICT DO NOTHING;
