-- LCIR (LumenCite Document Intermediate Representation) 基盤（Phase 0/1・Milestone A）。
-- 原資料→正規化文書層の第一歩。既存 fulltext(FTS5) は派生索引としてそのまま。
-- この increment は 3 表のみ。math/assets/relations/symbols は後続 phase の別 migration。
-- 実験フラグ settings.'lcir.enabled' が OFF の間、どの経路もこれらの表に触れない（空のまま）。

-- 添付ごとの抽出/変換結果 1 回分。provenance と再現性の正本。
CREATE TABLE document_versions (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    attachment_id     INTEGER NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    -- 再現可能な内容由来 ID = sha256(source_sha256|extractor_name|extractor_version|config_hash)。
    -- INTEGER PK は SQLite 採番で再現不能なため、roadmap の「同一 PDF→同一 version id」は
    -- この列で満たす。UNIQUE は既存重複で brick し得るので起動時 best-effort
    -- (attachment_id, content_key)（db::document_versions::try_create_content_key_unique_index）。
    content_key       TEXT    NOT NULL,
    schema_version    TEXT    NOT NULL,           -- document_ir::SCHEMA_VERSION 例 '0.1.0'
    source_sha256     TEXT    NOT NULL,           -- 原ファイル本体の SHA-256（attachments に列は無く都度計算）
    source_mime_type  TEXT    NOT NULL,
    extractor_name    TEXT    NOT NULL,           -- 例 'lumencite-pdfium'
    extractor_version TEXT    NOT NULL,           -- 抽出ロジックの semver（const・supersede 判定基準）
    config_hash       TEXT    NOT NULL DEFAULT '',-- 抽出設定ハッシュ（既定設定は空→定数）
    parent_version_id INTEGER REFERENCES document_versions(id), -- supersede チェーン
    extraction_status TEXT    NOT NULL,           -- pending/processing/completed/completed_with_warnings/failed/superseded
    warnings_json     TEXT,                        -- 抽出失敗・警告ログ（Phase1）
    metadata_json     TEXT,                        -- 座標系記述子・ページ数・pdfium/クレート版・計測値
    created_at        TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_document_versions_attachment  ON document_versions(attachment_id);
CREATE INDEX idx_document_versions_content_key ON document_versions(content_key);

-- 文書の型付きノード木（この increment: document/page/text_block/line/unknown_block）。
CREATE TABLE document_nodes (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    document_version_id INTEGER NOT NULL REFERENCES document_versions(id) ON DELETE CASCADE,
    parent_id           INTEGER REFERENCES document_nodes(id) ON DELETE CASCADE,
    node_kind           TEXT    NOT NULL,          -- document_ir::NodeKind の snake_case
    ordinal             INTEGER NOT NULL,          -- 同一親内の読み順（page は 0 始まり = page_number - 1）
    plain_text          TEXT,                       -- page ノードはページ全文（=FTS 再生成元）
    language            TEXT,
    confidence          REAL,
    origin              TEXT,                       -- document_ir::Origin（pdf_text_layer 等）
    payload_json        TEXT,                       -- 型固有（page_width_pt/height_pt/rotation_deg 等）
    created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_document_nodes_version      ON document_nodes(document_version_id);
CREATE INDEX idx_document_nodes_parent       ON document_nodes(parent_id);
CREATE INDEX idx_document_nodes_version_kind ON document_nodes(document_version_id, node_kind);

-- ノード↔PDF 領域。座標は highlights と同一系（PDF user space・左下原点・pt）。
CREATE TABLE source_fragments (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id       INTEGER NOT NULL REFERENCES document_nodes(id) ON DELETE CASCADE,
    page_number   INTEGER NOT NULL,               -- 1 始まり（fulltext.page / highlights.page と同じ）
    x             REAL    NOT NULL,
    y             REAL    NOT NULL,
    width         REAL    NOT NULL,
    height        REAL    NOT NULL,
    rotation      REAL    NOT NULL DEFAULT 0,      -- ページ /Rotate（0/90/180/270）
    reading_order INTEGER,
    fragment_type TEXT                              -- 'page' | 'text_block' | 'line'
);
CREATE INDEX idx_source_fragments_node ON source_fragments(node_id);
CREATE INDEX idx_source_fragments_page ON source_fragments(page_number);
