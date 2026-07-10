-- ハイライトを添付（attachment）単位で識別する（CR-015 / v0.8 複数 PDF 対応）。
-- これまで highlights は entry_id + page でしか識別できず、同じエントリに複数 PDF が
-- ぶら下がると primary PDF 3 ページ目のハイライトが supplement PDF 3 ページ目にも
-- 表示・削除されてしまった。attachment_id を追加して PDF ごとに分離する。

ALTER TABLE highlights ADD COLUMN attachment_id INTEGER
    REFERENCES attachments(id) ON DELETE CASCADE;

-- 既存行は各エントリの primary（最小 id = 先頭）添付へ移行する。
-- 添付が 1 つも無いエントリのハイライト（通常は発生しない）は NULL のまま残る。
UPDATE highlights
SET attachment_id = (
    SELECT MIN(a.id) FROM attachments a WHERE a.entry_id = highlights.entry_id
)
WHERE attachment_id IS NULL;

CREATE INDEX idx_highlights_attachment_page ON highlights(attachment_id, page);
