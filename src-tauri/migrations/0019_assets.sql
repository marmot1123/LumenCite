-- LCIR Phase 8a（図表アセット基盤）: 図領域のページ crop PNG 等のバイナリアセットを参照する
-- `assets` と、ノード（`figure` 等）との紐づけ `node_assets` を保存する。
--
-- バイナリ本体は **ファイルシステム**（`attachments/<entry_id>/.lcir/<attachment_id>/<content_key16>/`）
-- に置き、DB は相対パス + SHA-256 参照のみを持つ（`attachments` と同じ方針・ADR #3）。
-- ファイルの存在は保証しない（欠損許容・読み手はファイル欠損に耐えること。reuse 経路の
-- self-heal とライフサイクルは DATA_MODEL.md「assets / node_assets」参照）。
--
-- 図領域は埋込画像 bbox からの**レイアウト推定**（origin='layout_model'・confidence 付き）で、
-- 原文由来と推定を区別する。roadmap §5.5-5.6 の TEXT-UUID DDL を LumenCite 規約
-- （INTEGER PK・FK ON DELETE CASCADE・datetime('now')）に適応。version 削除
-- （=添付削除のカスケード）でアセット行・紐づけも消える。
-- 実験フラグ lcir.enabled が OFF の間は空のまま。
CREATE TABLE assets (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    document_version_id INTEGER NOT NULL REFERENCES document_versions(id) ON DELETE CASCADE,
    sha256              TEXT    NOT NULL,      -- ファイル本体の SHA-256（小文字 hex）
    mime_type           TEXT    NOT NULL,      -- "image/png"（8a）
    relative_path       TEXT    NOT NULL,      -- app data dir 相対・'/' 区切り
    width               INTEGER,               -- ピクセル寸法
    height              INTEGER,
    size_bytes          INTEGER,               -- ファイルサイズ（容量集計用）
    metadata_json       TEXT,                  -- {"page", "region_index", "render_target_width"} 等
    created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_assets_version ON assets(document_version_id);

CREATE TABLE node_assets (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id    INTEGER NOT NULL REFERENCES document_nodes(id) ON DELETE CASCADE,
    asset_id   INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    role       TEXT    NOT NULL,               -- page_crop（8a）。将来 original/vector/thumbnail/...
    created_at TEXT    NOT NULL DEFAULT (datetime('now'))
);
-- 新規テーブルなので UNIQUE を migration 内で張れる（既存 DB に重複が存在し得ない）。
CREATE UNIQUE INDEX idx_node_assets_unique ON node_assets(node_id, asset_id, role);
CREATE INDEX idx_node_assets_asset ON node_assets(asset_id);
