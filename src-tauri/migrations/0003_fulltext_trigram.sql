-- 0001 で unicode61 で作成済みだったが、PDF 添付フェーズで CJK・部分一致対応のため
-- trigram tokenizer に切り替える。これまで PDF 全文インデックスは実装していないため
-- 既存データを破棄して再作成する。
DROP TABLE IF EXISTS fulltext;

CREATE VIRTUAL TABLE fulltext USING fts5(
    content,
    attachment_id UNINDEXED,
    page          UNINDEXED,
    tokenize      = 'trigram'
);
