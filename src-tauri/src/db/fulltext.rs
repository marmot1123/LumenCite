use crate::models::{Author, EntrySummary, FulltextHit, Tag};
use sqlx::{Row, SqlitePool};

/// 抽出済みの本文を attachment_id に紐付けて fulltext テーブルへ書き込む。
/// `pages` は (page_number, text) のリスト。空文字列のページはスキップする。
pub async fn index_attachment(
    pool: &SqlitePool,
    attachment_id: i64,
    pages: &[(i64, String)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM fulltext WHERE attachment_id = ?")
        .bind(attachment_id)
        .execute(&mut *tx)
        .await?;

    for (page, content) in pages {
        if content.trim().is_empty() {
            continue;
        }
        sqlx::query("INSERT INTO fulltext (content, attachment_id, page) VALUES (?, ?, ?)")
            .bind(content)
            .bind(attachment_id)
            .bind(page)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 指定ページのみを差し替える（他ページの行は保持）。部分 OCR の保存用。
/// 空文字列のページは削除のみ行う（再処理の結果テキストが無かった場合）。
pub async fn update_attachment_pages(
    pool: &SqlitePool,
    attachment_id: i64,
    pages: &[(i64, String)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    for (page, content) in pages {
        sqlx::query("DELETE FROM fulltext WHERE attachment_id = ? AND page = ?")
            .bind(attachment_id)
            .bind(page)
            .execute(&mut *tx)
            .await?;
        if content.trim().is_empty() {
            continue;
        }
        sqlx::query("INSERT INTO fulltext (content, attachment_id, page) VALUES (?, ?, ?)")
            .bind(content)
            .bind(attachment_id)
            .bind(page)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn unindex_attachment(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM fulltext WHERE attachment_id = ?")
        .bind(attachment_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn is_indexed(pool: &SqlitePool, attachment_id: i64) -> Result<bool, sqlx::Error> {
    let row =
        sqlx::query("SELECT COUNT(*) AS cnt FROM fulltext WHERE attachment_id = ?")
            .bind(attachment_id)
            .fetch_one(pool)
            .await?;
    Ok(row.get::<i64, _>("cnt") > 0)
}

fn build_match_expr(tokens: &[&str]) -> String {
    tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub async fn search_fulltext(
    pool: &SqlitePool,
    query: &str,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
) -> Result<Vec<FulltextHit>, sqlx::Error> {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // trigram は 3 文字未満を処理できない。短いトークンが含まれていれば LIKE フォールバック。
    let use_like = tokens.iter().any(|t| t.chars().count() < 3);

    let mut sql = String::new();
    sql.push_str(
        "SELECT f.attachment_id AS attachment_id, f.page AS page, ",
    );
    if use_like {
        sql.push_str("substr(f.content, 1, 200) AS snippet, ");
    } else {
        sql.push_str(
            "snippet(fulltext, 0, '⟨', '⟩', '…', 12) AS snippet, ",
        );
    }
    sql.push_str(
        "a.entry_id AS entry_id
         FROM fulltext f
         JOIN attachments a ON a.id = f.attachment_id
         WHERE ",
    );

    if use_like {
        // 各トークンが content に含まれること（AND）
        let likes: Vec<&str> = tokens.iter().map(|_| "f.content LIKE ?").collect();
        sql.push_str(&likes.join(" AND "));
    } else {
        sql.push_str("fulltext MATCH ?");
    }

    sql.push_str(
        " AND a.entry_id IN (SELECT id FROM entries WHERE deleted_at IS NULL)",
    );

    if collection_id.is_some() {
        sql.push_str(
            " AND a.entry_id IN (SELECT entry_id FROM entry_collections WHERE collection_id = ?)",
        );
    }
    if tag_id.is_some() {
        sql.push_str(" AND a.entry_id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ?)");
    }

    if use_like {
        sql.push_str(" ORDER BY f.attachment_id, f.page");
    } else {
        sql.push_str(" ORDER BY bm25(fulltext)");
    }

    let mut q = sqlx::query(&sql);
    if use_like {
        for token in &tokens {
            q = q.bind(format!("%{}%", token));
        }
    } else {
        q = q.bind(build_match_expr(&tokens));
    }
    if let Some(cid) = collection_id {
        q = q.bind(cid);
    }
    if let Some(tid) = tag_id {
        q = q.bind(tid);
    }

    let rows = q.fetch_all(pool).await?;

    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        let entry_id: i64 = row.get("entry_id");
        let summary = load_summary(pool, entry_id).await?;
        hits.push(FulltextHit {
            entry: summary,
            attachment_id: row.get("attachment_id"),
            page: row.get("page"),
            snippet: row.get("snippet"),
        });
    }

    Ok(hits)
}

async fn load_summary(pool: &SqlitePool, id: i64) -> Result<EntrySummary, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, title, year, entry_type, created_at, starred FROM entries WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    let authors: Vec<Author> = sqlx::query_as(
        "SELECT a.id, a.name,
                a.given_name, a.middle_name, a.family_name, a.suffix, a.name_particle,
                a.name_original, a.given_name_original, a.family_name_original, a.original_script,
                a.reading_family, a.reading_given,
                a.is_organization,
                a.email, a.homepage_url, a.notes,
                a.orcid, a.updated_at
         FROM authors a
         JOIN entry_authors ea ON ea.author_id = a.id
         WHERE ea.entry_id = ?
         ORDER BY ea.position",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let tags: Vec<Tag> = sqlx::query_as(
        "SELECT t.id, t.name FROM tags t
         JOIN entry_tags et ON et.tag_id = t.id
         WHERE et.entry_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let journal: Option<String> = sqlx::query_scalar(
        "SELECT field_value FROM extra_fields WHERE entry_id = ? AND field_name = 'journal'",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(EntrySummary {
        id: row.get("id"),
        title: row.get("title"),
        year: row.get("year"),
        entry_type: row.get("entry_type"),
        created_at: row.get("created_at"),
        authors,
        tags,
        has_attachment: true,
        journal,
        starred: row.get::<i64, _>("starred") != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    async fn setup_attachment(pool: &SqlitePool, title: &str) -> (i64, i64) {
        let entry = create_entry(
            pool,
            &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let att = add_attachment(
            pool,
            entry.id,
            "attachments/x/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();

        (entry.id, att.id)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn indexed_text_is_searchable(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(
            &pool,
            att_id,
            &[(1, "Transformer architecture is described here.".to_string())],
        )
        .await
        .unwrap();

        let hits = search_fulltext(&pool, "transformer", None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attachment_id, att_id);
        assert_eq!(hits[0].page, 1);
        assert!(hits[0].snippet.to_lowercase().contains("transformer"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn index_returns_one_hit_per_page(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(
            &pool,
            att_id,
            &[
                (1, "Introduction to attention mechanisms.".to_string()),
                (2, "Attention layer details and equations.".to_string()),
                (3, "Conclusion section without the keyword.".to_string()),
            ],
        )
        .await
        .unwrap();

        let hits = search_fulltext(&pool, "attention", None, None).await.unwrap();
        let pages: Vec<i64> = hits.iter().map(|h| h.page).collect();
        assert!(pages.contains(&1));
        assert!(pages.contains(&2));
        assert!(!pages.contains(&3));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reindexing_replaces_old_rows(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(&pool, att_id, &[(1, "old keyword".to_string())])
            .await
            .unwrap();
        index_attachment(&pool, att_id, &[(1, "fresh content only".to_string())])
            .await
            .unwrap();

        let stale = search_fulltext(&pool, "old", None, None).await.unwrap();
        let fresh = search_fulltext(&pool, "fresh", None, None).await.unwrap();
        assert!(stale.is_empty());
        assert_eq!(fresh.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_attachment_pages_replaces_only_given_pages(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(
            &pool,
            att_id,
            &[
                (1, "page one original".to_string()),
                (2, "page two original".to_string()),
                (3, "page three original".to_string()),
            ],
        )
        .await
        .unwrap();

        // ページ 2 だけ差し替える。1 と 3 は保持される。
        update_attachment_pages(&pool, att_id, &[(2, "page two replaced".to_string())])
            .await
            .unwrap();

        assert_eq!(search_fulltext(&pool, "original", None, None).await.unwrap().len(), 2);
        assert_eq!(search_fulltext(&pool, "replaced", None, None).await.unwrap().len(), 1);
        assert!(search_fulltext(&pool, "two original", None, None).await.unwrap().is_empty());

        // 空文字列に差し替えた場合はそのページの行が消える（再OCRで空だったケース）
        update_attachment_pages(&pool, att_id, &[(3, "".to_string())]).await.unwrap();
        let row_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
                .bind(att_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row_count, 2); // page 1 original + page 2 replaced
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_removes_fulltext_rows(pool: SqlitePool) {
        let (entry_id, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att_id, &[(1, "needle".to_string())])
            .await
            .unwrap();

        crate::db::entries::delete_entry(&pool, entry_id).await.unwrap();

        let orphans: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
                .bind(att_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(orphans, 0, "hard delete must not orphan fulltext rows");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unindex_removes_rows(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att_id, &[(1, "needle".to_string())])
            .await
            .unwrap();

        unindex_attachment(&pool, att_id).await.unwrap();

        let hits = search_fulltext(&pool, "needle", None, None).await.unwrap();
        assert!(hits.is_empty());
        assert!(!is_indexed(&pool, att_id).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_pages_are_skipped(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(
            &pool,
            att_id,
            &[
                (1, "".to_string()),
                (2, "  \n  ".to_string()),
                (3, "real content".to_string()),
            ],
        )
        .await
        .unwrap();

        let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
            .bind(att_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row_count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_query_returns_empty(pool: SqlitePool) {
        let hits = search_fulltext(&pool, "  ", None, None).await.unwrap();
        assert!(hits.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_respects_collection_filter(pool: SqlitePool) {
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Inbox')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let (entry_in, att_in) = setup_attachment(&pool, "Inside").await;
        let (_, att_out) = setup_attachment(&pool, "Outside").await;

        sqlx::query("INSERT INTO entry_collections (entry_id, collection_id) VALUES (?, ?)")
            .bind(entry_in)
            .bind(col_id)
            .execute(&pool)
            .await
            .unwrap();

        index_attachment(&pool, att_in, &[(1, "transformer paper".to_string())])
            .await
            .unwrap();
        index_attachment(&pool, att_out, &[(1, "transformer review".to_string())])
            .await
            .unwrap();

        let hits = search_fulltext(&pool, "transformer", Some(col_id), None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attachment_id, att_in);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_matches_japanese_substring(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "和文").await;
        index_attachment(
            &pool,
            att_id,
            &[(1, "本論文では深層学習モデルの精度を評価する。".to_string())],
        )
        .await
        .unwrap();

        let hits = search_fulltext(&pool, "深層学習", None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_removes_fulltext_of_all_attachments(pool: SqlitePool) {
        let (entry_id, att1) = setup_attachment(&pool, "Paper").await;
        let att2 = add_attachment(
            &pool,
            entry_id,
            "attachments/x/p2.pdf",
            "p2.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;

        index_attachment(&pool, att1, &[(1, "alpha".to_string())])
            .await
            .unwrap();
        index_attachment(&pool, att2, &[(1, "beta".to_string())])
            .await
            .unwrap();

        crate::db::entries::delete_entry(&pool, entry_id).await.unwrap();

        let hits_a = search_fulltext(&pool, "alpha", None, None).await.unwrap();
        let hits_b = search_fulltext(&pool, "beta", None, None).await.unwrap();
        assert!(hits_a.is_empty());
        assert!(hits_b.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn short_query_uses_like_fallback(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att_id, &[(1, "AI models are evolving rapidly".to_string())])
            .await
            .unwrap();

        let hits = search_fulltext(&pool, "AI", None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
    }
}
