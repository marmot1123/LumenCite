use crate::models::Attachment;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct AttachmentWithPath {
    pub file_path: String,
    pub file_name: String,
}

pub async fn add_attachment(
    pool: &SqlitePool,
    entry_id: i64,
    file_path: &str,
    file_name: &str,
    mime_type: &str,
) -> Result<Attachment, sqlx::Error> {
    // 親エントリが存在しないと FK 違反になるため事前に確認してわかりやすいエラーを返す
    let exists: bool = sqlx::query("SELECT 1 AS x FROM entries WHERE id = ?")
        .bind(entry_id)
        .fetch_optional(pool)
        .await?
        .is_some();
    if !exists {
        return Err(sqlx::Error::RowNotFound);
    }

    let result = sqlx::query(
        "INSERT INTO attachments (entry_id, file_path, file_name, mime_type)
         VALUES (?, ?, ?, ?)",
    )
    .bind(entry_id)
    .bind(file_path)
    .bind(file_name)
    .bind(mime_type)
    .execute(pool)
    .await?;

    let id = result.last_insert_rowid();
    get_attachment(pool, id).await
}

pub async fn get_attachment(pool: &SqlitePool, id: i64) -> Result<Attachment, sqlx::Error> {
    sqlx::query_as::<_, Attachment>(
        "SELECT id, entry_id, file_name, mime_type, created_at
         FROM attachments WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_attachment_with_path(
    pool: &SqlitePool,
    id: i64,
) -> Result<AttachmentWithPath, sqlx::Error> {
    let row = sqlx::query("SELECT file_path, file_name FROM attachments WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;

    Ok(AttachmentWithPath {
        file_path: row.get("file_path"),
        file_name: row.get("file_name"),
    })
}

/// 添付レコードと全文索引（fulltext）を **単一トランザクションで**削除する（CR-008）。
/// これまでは fulltext 削除 → attachments 削除が別クエリで、index 削除失敗を握りつぶすと
/// orphan なテキストが残り得た。まとめて原子的に消し、ファイル本体の削除だけ呼び出し側の
/// best-effort に残す。
pub async fn delete_attachment_with_fulltext(
    pool: &SqlitePool,
    id: i64,
) -> Result<AttachmentWithPath, sqlx::Error> {
    let att = get_attachment_with_path(pool, id).await?;

    let mut tx = pool.begin().await?;
    // fulltext は attachments への FK を持たないため明示削除（cascade では拾えない）。
    sqlx::query("DELETE FROM fulltext WHERE attachment_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let rows = sqlx::query("DELETE FROM attachments WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if rows == 0 {
        // tx は drop でロールバックされる（fulltext 削除も巻き戻る）。
        return Err(sqlx::Error::RowNotFound);
    }
    tx.commit().await?;
    Ok(att)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    async fn make_entry(pool: &SqlitePool, title: &str) -> i64 {
        create_entry(
            pool,
            &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_attachment_inserts_row(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper").await;

        let att = add_attachment(
            &pool,
            entry_id,
            "attachments/1/paper.pdf",
            "paper.pdf",
            "application/pdf",
        )
        .await
        .unwrap();

        assert!(att.id > 0);
        assert_eq!(att.entry_id, entry_id);
        assert_eq!(att.file_name, "paper.pdf");
        assert_eq!(att.mime_type, "application/pdf");
    }

    /// CR-008: file_path は UNIQUE。同じパスを 2 度登録できない。
    #[sqlx::test(migrations = "./migrations")]
    async fn add_attachment_rejects_duplicate_path(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper").await;
        add_attachment(&pool, entry_id, "attachments/1/paper.pdf", "paper.pdf", "application/pdf")
            .await
            .unwrap();
        let dup = add_attachment(
            &pool, entry_id, "attachments/1/paper.pdf", "paper.pdf", "application/pdf",
        )
        .await;
        assert!(dup.is_err(), "重複 file_path は UNIQUE 制約で拒否される");
    }

    /// CR-008: 添付削除は fulltext と attachments 行を原子的に消す。
    #[sqlx::test(migrations = "./migrations")]
    async fn delete_with_fulltext_removes_both(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper").await;
        let att = add_attachment(
            &pool, entry_id, "attachments/1/p.pdf", "p.pdf", "application/pdf",
        )
        .await
        .unwrap();
        crate::db::fulltext::index_attachment(&pool, att.id, &[(1, "hello world".to_string())])
            .await
            .unwrap();

        delete_attachment_with_fulltext(&pool, att.id).await.unwrap();

        let att_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE id = ?")
            .bind(att.id).fetch_one(&pool).await.unwrap();
        let ft_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
            .bind(att.id).fetch_one(&pool).await.unwrap();
        assert_eq!(att_rows, 0);
        assert_eq!(ft_rows, 0, "orphan な全文索引が残ってはいけない");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_attachment_unknown_entry_returns_not_found(pool: SqlitePool) {
        let result = add_attachment(
            &pool,
            9999,
            "x.pdf",
            "x.pdf",
            "application/pdf",
        )
        .await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_attachment_with_path_returns_path(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper").await;
        let att = add_attachment(
            &pool,
            entry_id,
            "attachments/42/paper.pdf",
            "paper.pdf",
            "application/pdf",
        )
        .await
        .unwrap();

        let detail = get_attachment_with_path(&pool, att.id).await.unwrap();
        assert_eq!(detail.file_path, "attachments/42/paper.pdf");
        assert_eq!(detail.file_name, "paper.pdf");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_attachment_removes_row_and_returns_path(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper").await;
        let att = add_attachment(
            &pool,
            entry_id,
            "attachments/1/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();

        let removed = delete_attachment_with_fulltext(&pool, att.id).await.unwrap();
        assert_eq!(removed.file_path, "attachments/1/p.pdf");

        let result = get_attachment(&pool, att.id).await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_attachment_not_found_returns_error(pool: SqlitePool) {
        let result = delete_attachment_with_fulltext(&pool, 9999).await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_entry_cascades_attachments(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper").await;
        let att = add_attachment(
            &pool,
            entry_id,
            "attachments/1/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();

        crate::db::entries::delete_entry(&pool, entry_id).await.unwrap();

        let result = get_attachment(&pool, att.id).await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }
}
