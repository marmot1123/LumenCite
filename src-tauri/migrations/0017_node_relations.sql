-- LCIR Phase 6a（参照グラフ）: ノード間の型付き関係を保存する。paragraph/theorem/proof 等の
-- ノードから、それが参照する equation/theorem/figure/section/bibliography_entry ノードへの
-- 有向辺 (from_node, relation_type, to_node)。
--
-- 原資料に生で残る参照（TeX の `\ref`/`\eqref`/`\cite`）と PDF 本文の "Theorem 2.3"/"Eq. (2.1)"
-- を、`\label`・定理番号・数式番号・cite key と突き合わせて張る（`ingestion::graph`）。
-- **原文由来（TeX・origin=tex_source・高信頼）と推定（PDF レイアウト・origin=layout_model・
-- 中信頼）を区別**するため origin と confidence を必ず持たせる。proof→theorem の proves もここ。
--
-- roadmap §5.7 の TEXT-UUID DDL を LumenCite 規約（INTEGER PK・FK ON DELETE CASCADE・
-- datetime('now')）に適応。`document_version_id` を持たせ、版削除（=添付削除のカスケード）で
-- 関係も消える。記号系の `symbols`/`symbol_occurrences` は Phase 6b の別 migration で追加する
-- （0014 の「relations/symbols は後続 phase の別 migration」方針を踏襲）。
-- 実験フラグ lcir.enabled が OFF の間は空のまま。
CREATE TABLE node_relations (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    document_version_id INTEGER NOT NULL REFERENCES document_versions(id) ON DELETE CASCADE,
    from_node_id        INTEGER NOT NULL REFERENCES document_nodes(id) ON DELETE CASCADE,
    relation_type       TEXT    NOT NULL,   -- cites/refers_to_equation/refers_to_theorem/refers_to_figure/refers_to_table/refers_to_section/refers_to/proves/...
    to_node_id          INTEGER NOT NULL REFERENCES document_nodes(id) ON DELETE CASCADE,
    confidence          REAL,                -- 参照解決の確からしさ（意味の確からしさではない）
    origin              TEXT,                -- tex_source/layout_model/...
    metadata_json       TEXT,                -- 生の参照文字列・突き合わせたキー/番号など
    created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_node_relations_version ON node_relations(document_version_id);
CREATE INDEX idx_node_relations_from ON node_relations(from_node_id);
CREATE INDEX idx_node_relations_to ON node_relations(to_node_id);
