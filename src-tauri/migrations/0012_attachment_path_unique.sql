-- 添付ファイルパスの一意制約（CR-008）。
-- これまで保存先ファイル名の採番が exists() チェック依存で、並行追加の TOCTOU により
-- 1 つのファイルを 2 行が共有し得た。保存側は O_EXCL の原子的予約に変えたうえで、
-- DB 側でも file_path を UNIQUE にして二重登録を防ぐ。
--
-- 念のため既存の重複行（通常は存在しない）を最小 id を残して掃除してから索引を張る。
-- 重複行の全文索引もあわせて消す（fulltext は attachments への FK を持たないため明示削除）。

DELETE FROM fulltext WHERE attachment_id IN (
    SELECT id FROM attachments
    WHERE id NOT IN (SELECT MIN(id) FROM attachments GROUP BY file_path)
);

DELETE FROM attachments
WHERE id NOT IN (SELECT MIN(id) FROM attachments GROUP BY file_path);

CREATE UNIQUE INDEX idx_attachments_file_path ON attachments(file_path);
