-- LCIR Phase 8c（図の代替テキスト）: `figure` ノード（Phase 8a）のページ crop PNG を
-- LLM Vision に説明させた alt text を保存する。
--
-- **AI 推定を原資料由来と区別する**（roadmap §16）: alt text は原文に存在しない生成物なので
-- `origin='llm_inference'` + `confidence` + `model`（使ったモデル名）を必ず持たせる。原文 caption
-- （`figure_caption` ノード）は上書きせず別 provenance で併存させ、生成文は `fulltext` /
-- `document_nodes_fts` にも索引しない（検索結果の由来が曖昧になるため）。
--
-- **ノード本体（document_nodes）に相乗りせず satellite 表にする理由**: `figure` ノードの
-- `origin`/`confidence` は既に「図領域検出」（layout_model / 0.6）の確からしさで占有されており、
-- そこへ alt text の provenance を混ぜると「どの値がどの主張の確からしさか」が opaque になる。
--
-- **版跨ぎ**: 抽出器版を上げて再構築すると `figure` ノードの id は変わるが、crop PNG の SHA-256 が
-- 同一なら同じ絵なので、`source_asset_sha256` を鍵に過去の全版から alt text を引き継ぐ
-- （`carried_from_version_id` に由来版を記録）。引き継ぎ後、現版以外の `llm_inference` 行は刈る。
-- `user_edited`（将来の手編集）は carry も削除も上書きもしない。
--
-- 実験フラグ `lcir.enabled` と `lcir.vision_alt_text.enabled` の両方が ON でバッチを回すまで空。
CREATE TABLE node_alt_texts (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id                 INTEGER NOT NULL REFERENCES document_nodes(id) ON DELETE CASCADE,
    document_version_id     INTEGER NOT NULL REFERENCES document_versions(id) ON DELETE CASCADE,
    source_asset_sha256     TEXT    NOT NULL,  -- 説明した crop PNG の SHA-256（provenance + carry キー）
    text                    TEXT    NOT NULL,  -- 生成された説明文
    origin                  TEXT    NOT NULL,  -- llm_inference（生成）/ user_edited（将来の手編集）
    confidence              REAL,              -- AI 推定であることの表明（意味の正しさの尺度ではない）
    model                   TEXT,              -- 生成に使ったモデル名
    carried_from_version_id INTEGER REFERENCES document_versions(id) ON DELETE SET NULL,
    created_at              TEXT    NOT NULL DEFAULT (datetime('now')),
    -- 1 ノードにつき生成 1 件 + 手編集 1 件まで（読み出しは user_edited を優先）。
    -- 新規テーブルなのでここで張っても既存 DB を壊さない。
    UNIQUE (node_id, origin)
);
CREATE INDEX idx_node_alt_texts_version ON node_alt_texts(document_version_id);
CREATE INDEX idx_node_alt_texts_asset ON node_alt_texts(source_asset_sha256);
CREATE INDEX idx_node_alt_texts_node ON node_alt_texts(node_id);
