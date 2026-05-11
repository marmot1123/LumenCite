-- お気に入り（starred）とソフト削除（deleted_at）の追加。
-- starred は 0/1 のフラグ。deleted_at は NULL でない場合に「ゴミ箱」内のエントリ。

ALTER TABLE entries ADD COLUMN starred    INTEGER NOT NULL DEFAULT 0;
ALTER TABLE entries ADD COLUMN deleted_at TEXT;

-- 部分インデックス: お気に入りビュー / ゴミ箱ビューでのフィルタを高速化する。
CREATE INDEX idx_entries_starred    ON entries(starred)    WHERE starred = 1;
CREATE INDEX idx_entries_deleted_at ON entries(deleted_at) WHERE deleted_at IS NOT NULL;
