-- 識別子（DOI / arXiv / ISBN）の canonical 列を追加する（CR-019）。
--
-- これまで一意性はアプリの find_duplicate_entry 頼みで、
--  ① stored 側の正規化が非対称（SQL の LOWER のみ。arXiv は版番号/prefix を剥がさない）で
--     非正準な既存値が dedup をすり抜け、
--  ② 一般の create_entry（UI 追加 / import / LLM）は dedup を一切せず、
--  ③ DB レベルの UNIQUE 制約が無かった。
--
-- 対策として entries に正準値を持つ列を足し、書込・重複判定・起動時 backfill の
-- すべてを Rust の canonical_{doi,arxiv,isbn}() に一元化する。
--
-- backfill（既存行の canonical 埋め）は SQL では arXiv の版番号除去・旧形式カテゴリ保持を
-- 表現できないため、**全て Rust の起動時 backfill に委ねる**（db::entries::backfill_canonical_identifiers）。
-- ここでは列と非 UNIQUE の部分インデックスのみ作る。UNIQUE 制約は既存重複があると
-- migration が失敗して起動不能（brick）になり得るため張らず、起動時に best-effort で
-- （重複が無いときだけ）張る（db::entries::try_create_identifier_unique_indexes）。

ALTER TABLE entries ADD COLUMN doi_canonical   TEXT;
ALTER TABLE entries ADD COLUMN arxiv_canonical TEXT;
ALTER TABLE entries ADD COLUMN isbn_canonical  TEXT;

-- 重複判定（find_duplicate_entry）の lookup を速くするための非 UNIQUE 部分インデックス。
-- NULL は索引に含めない（識別子が無いエントリで肥大化させない）。
CREATE INDEX idx_entries_doi_canonical
    ON entries(doi_canonical)   WHERE doi_canonical   IS NOT NULL;
CREATE INDEX idx_entries_arxiv_canonical
    ON entries(arxiv_canonical) WHERE arxiv_canonical IS NOT NULL;
CREATE INDEX idx_entries_isbn_canonical
    ON entries(isbn_canonical)  WHERE isbn_canonical  IS NOT NULL;
