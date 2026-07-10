use sqlx::SqlitePool;

#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct Highlight {
    pub id: i64,
    pub entry_id: i64,
    /// どの添付 PDF に属すか（CR-015）。旧データ移行で NULL が残り得る。
    pub attachment_id: Option<i64>,
    pub page: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub color: String,
    pub text: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct HighlightInput {
    pub entry_id: i64,
    /// ハイライトを付けている添付 PDF（CR-015）。
    #[serde(default)]
    pub attachment_id: Option<i64>,
    pub page: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub color: String,
    pub text: String,
    pub note: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct HighlightUpdate {
    pub color: Option<String>,
    pub note: Option<String>,
}

fn valid_color(color: &str) -> bool {
    matches!(color, "yellow" | "green" | "blue")
}

/// SELECT で使う列並び。`Highlight` の順序と一致させること。
const HIGHLIGHT_COLUMNS: &str =
    "id, entry_id, attachment_id, page, x, y, width, height, color, text, note, created_at";

pub async fn list_by_entry(pool: &SqlitePool, entry_id: i64) -> Result<Vec<Highlight>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {HIGHLIGHT_COLUMNS}
         FROM highlights
         WHERE entry_id = ?
         ORDER BY page ASC, y DESC, id ASC"
    ))
    .bind(entry_id)
    .fetch_all(pool)
    .await
}

/// 添付 PDF 単位でハイライトを返す（CR-015）。UI は選択中の添付でこれを使う。
pub async fn list_by_attachment(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<Vec<Highlight>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {HIGHLIGHT_COLUMNS}
         FROM highlights
         WHERE attachment_id = ?
         ORDER BY page ASC, y DESC, id ASC"
    ))
    .bind(attachment_id)
    .fetch_all(pool)
    .await
}

pub async fn create(pool: &SqlitePool, input: &HighlightInput) -> Result<Highlight, sqlx::Error> {
    if !valid_color(&input.color) {
        return Err(sqlx::Error::Protocol(format!(
            "invalid color: {}",
            input.color
        )));
    }
    let id = sqlx::query(
        "INSERT INTO highlights (entry_id, attachment_id, page, x, y, width, height, color, text, note)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.entry_id)
    .bind(input.attachment_id)
    .bind(input.page)
    .bind(input.x)
    .bind(input.y)
    .bind(input.width)
    .bind(input.height)
    .bind(&input.color)
    .bind(&input.text)
    .bind(&input.note)
    .execute(pool)
    .await?
    .last_insert_rowid();

    sqlx::query_as(&format!(
        "SELECT {HIGHLIGHT_COLUMNS} FROM highlights WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
}

/// `color` と `note` の部分更新を行う。指定されなかったフィールドは変更しない。
/// note に空文字列を渡すと NULL（ノート削除）とみなす。
pub async fn update(
    pool: &SqlitePool,
    id: i64,
    patch: &HighlightUpdate,
) -> Result<Highlight, sqlx::Error> {
    if let Some(c) = &patch.color {
        if !valid_color(c) {
            return Err(sqlx::Error::Protocol(format!("invalid color: {}", c)));
        }
    }

    if let Some(c) = &patch.color {
        sqlx::query("UPDATE highlights SET color = ? WHERE id = ?")
            .bind(c)
            .bind(id)
            .execute(pool)
            .await?;
    }
    if let Some(n) = &patch.note {
        if n.is_empty() {
            sqlx::query("UPDATE highlights SET note = NULL WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await?;
        } else {
            sqlx::query("UPDATE highlights SET note = ? WHERE id = ?")
                .bind(n)
                .bind(id)
                .execute(pool)
                .await?;
        }
    }

    sqlx::query_as(&format!(
        "SELECT {HIGHLIGHT_COLUMNS} FROM highlights WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let rows = sqlx::query("DELETE FROM highlights WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    async fn make_entry(pool: &SqlitePool) -> i64 {
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        entry.id
    }

    /// entry に PDF 添付を 1 つ作り、その attachment_id を返す。
    async fn make_attachment(pool: &SqlitePool, entry_id: i64, name: &str) -> i64 {
        crate::db::attachments::add_attachment(
            pool,
            entry_id,
            &format!("attachments/{entry_id}/{name}"),
            name,
            "application/pdf",
        )
        .await
        .unwrap()
        .id
    }

    fn input(entry_id: i64, page: i64, color: &str, text: &str) -> HighlightInput {
        HighlightInput {
            entry_id,
            attachment_id: None,
            page,
            x: 10.0,
            y: 720.0,
            width: 120.0,
            height: 14.0,
            color: color.to_string(),
            text: text.to_string(),
            note: None,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_and_list_highlights(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let h = create(&pool, &input(entry_id, 1, "yellow", "lorem")).await.unwrap();
        assert!(h.id > 0);
        assert_eq!(h.entry_id, entry_id);
        assert_eq!(h.page, 1);
        assert_eq!(h.color, "yellow");

        let all = list_by_entry(&pool, entry_id).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_orders_by_page_then_y_desc(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        // page 2 のハイライトを先に作る
        create(&pool, &HighlightInput { page: 2, y: 500.0, ..input(entry_id, 2, "yellow", "p2") })
            .await.unwrap();
        // page 1 の y=600 と y=300 を作る（y=600 が上）
        create(&pool, &HighlightInput { page: 1, y: 300.0, ..input(entry_id, 1, "green", "low") })
            .await.unwrap();
        create(&pool, &HighlightInput { page: 1, y: 600.0, ..input(entry_id, 1, "blue", "high") })
            .await.unwrap();

        let list = list_by_entry(&pool, entry_id).await.unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].page, 1);
        assert_eq!(list[0].text, "high"); // y=600 が先
        assert_eq!(list[1].text, "low");
        assert_eq!(list[2].page, 2);
    }

    /// CR-015: ハイライトは添付単位で分離され、別 PDF の同ページには現れない。
    #[sqlx::test(migrations = "./migrations")]
    async fn highlights_are_scoped_per_attachment(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let primary = make_attachment(&pool, entry_id, "primary.pdf").await;
        let supplement = make_attachment(&pool, entry_id, "supplement.pdf").await;

        // 両添付の 3 ページ目にハイライトを付ける。
        create(&pool, &HighlightInput { attachment_id: Some(primary), ..input(entry_id, 3, "yellow", "on primary") })
            .await.unwrap();
        create(&pool, &HighlightInput { attachment_id: Some(supplement), ..input(entry_id, 3, "green", "on supplement") })
            .await.unwrap();

        let p = list_by_attachment(&pool, primary).await.unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].text, "on primary");

        let s = list_by_attachment(&pool, supplement).await.unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text, "on supplement");
    }

    /// CR-015: 添付を消すとその添付のハイライトも CASCADE で消える。
    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_attachment_cascades_highlights(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let att = make_attachment(&pool, entry_id, "a.pdf").await;
        create(&pool, &HighlightInput { attachment_id: Some(att), ..input(entry_id, 1, "yellow", "x") })
            .await.unwrap();

        sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(att).execute(&pool).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM highlights WHERE attachment_id = ?")
            .bind(att).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_invalid_color(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let res = create(&pool, &input(entry_id, 1, "red", "x")).await;
        assert!(res.is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_changes_color_and_note(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let h = create(&pool, &input(entry_id, 1, "yellow", "t")).await.unwrap();

        let updated = update(
            &pool,
            h.id,
            &HighlightUpdate {
                color: Some("blue".to_string()),
                note: Some("important".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.color, "blue");
        assert_eq!(updated.note.as_deref(), Some("important"));

        // 空文字列でノート削除
        let cleared = update(
            &pool,
            h.id,
            &HighlightUpdate {
                color: None,
                note: Some(String::new()),
            },
        )
        .await
        .unwrap();
        assert_eq!(cleared.color, "blue", "color は変更されないべき");
        assert!(cleared.note.is_none(), "空文字列で note は NULL になるべき");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_removes_highlight(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let h = create(&pool, &input(entry_id, 1, "yellow", "t")).await.unwrap();

        delete(&pool, h.id).await.unwrap();
        let all = list_by_entry(&pool, entry_id).await.unwrap();
        assert!(all.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_not_found_errors(pool: SqlitePool) {
        let res = delete(&pool, 9999).await;
        assert!(matches!(res, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cascade_delete_when_entry_removed(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        create(&pool, &input(entry_id, 1, "yellow", "t")).await.unwrap();

        // ハード削除（FK CASCADE で highlights も消えるべき）
        sqlx::query("DELETE FROM entries WHERE id = ?")
            .bind(entry_id)
            .execute(&pool)
            .await
            .unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM highlights")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
