-- LCIR Phase 2（論理構造）: ノード単位の全文索引（段落・見出し・caption 等のブロック粒度）。
-- 既存 fulltext(ページ粒度) と併存する派生索引で、LCIR の block ノードから再生成できる
-- （ingestion::regenerate_node_fts_from_lcir）。attachments への FK は FTS5 仮想表なので張れず、
-- 削除時は db::entries の hard-delete 経路で attachment_id 指定の手動クリーンアップを行う
-- （fulltext と同じ作法）。tokenize=trigram で CJK・部分一致に対応（fulltext と同一）。
-- 実験フラグ lcir.enabled が OFF の間は空のまま（LCIR build が走らないため）。
CREATE VIRTUAL TABLE document_nodes_fts USING fts5(
    content,
    node_id       UNINDEXED,
    attachment_id UNINDEXED,
    page          UNINDEXED,
    node_kind     UNINDEXED,
    tokenize      = 'trigram'
);
