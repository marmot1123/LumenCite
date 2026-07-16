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

/// PDF を抽出して全文索引に取り込む（添付成功後の自動索引・CR-027）。best-effort。
/// テキストレイヤーが無い / 抽出失敗（スキャン PDF 等）は黙って諦める（OCR で後追い可能）。
/// 全経路（手動添付・arXiv 取得・クリッパー）が同じ post-attach 索引を通るよう共有する。
pub async fn extract_and_index(pool: &SqlitePool, abs_path: std::path::PathBuf, attachment_id: i64) {
    let extracted =
        tokio::task::spawn_blocking(move || pdf_extract::extract_text_by_pages(&abs_path)).await;
    if let Ok(Ok(pages_text)) = extracted {
        let pages: Vec<(i64, String)> = pages_text
            .into_iter()
            .enumerate()
            .map(|(i, t)| ((i + 1) as i64, t))
            .collect();
        let _ = index_attachment(pool, attachment_id, &pages).await;
    }
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

/// `fulltext` FTS5（trigram）の逆索引を起動時に 1 回だけ再構築する自己修復。
///
/// 一部の既存ライブラリでは `fulltext` の逆索引が malformed になっており、新しい
/// SQLite では `PRAGMA integrity_check` が "malformed inverted index for FTS5 table
/// main.fulltext" を返す（アプリ内蔵の古い SQLite では検出できないため素通りしていた）。
/// これを放置すると全文検索が誤動作し得るので、`settings.fts.fulltext_rebuilt` が未セット
/// なら FTS5 の `'rebuild'` コマンドで %_content から索引を作り直し、完了後にフラグを立てる。
/// 2 回目以降は no-op。malformed でない健全な索引でも rebuild は安全（同じ索引を作り直すだけ）。
///
/// 戻り値: 実際に再構築が走ったら `true`、フラグ既設で skip したら `false`。
/// `rebuild_authors_fts_once` と同じく起動時に background で呼ぶ。失敗時はフラグを立てず
/// Err を返すので次回起動でリトライされる。
pub async fn rebuild_fulltext_fts_once(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    use crate::db::settings;

    if settings::get_setting(pool, settings::FTS_FULLTEXT_REBUILT_KEY)
        .await?
        .is_some()
    {
        return Ok(false);
    }

    sqlx::query("INSERT INTO fulltext(fulltext) VALUES('rebuild')")
        .execute(pool)
        .await?;

    settings::set_setting(pool, settings::FTS_FULLTEXT_REBUILT_KEY, "1").await?;
    Ok(true)
}

pub async fn is_indexed(pool: &SqlitePool, attachment_id: i64) -> Result<bool, sqlx::Error> {
    let row =
        sqlx::query("SELECT COUNT(*) AS cnt FROM fulltext WHERE attachment_id = ?")
            .bind(attachment_id)
            .fetch_one(pool)
            .await?;
    Ok(row.get::<i64, _>("cnt") > 0)
}

/// まだ全文索引が無い PDF 添付を `(attachment_id, file_path)` で返す（ゴミ箱のエントリは除外）。
/// 「未索引の添付を一括索引」バッチが処理対象を集めるのに使う。順序は id 昇順で安定。
pub async fn attachments_without_fulltext(
    pool: &SqlitePool,
) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT a.id AS id, a.file_path AS file_path
         FROM attachments a
         JOIN entries e ON e.id = a.entry_id
         WHERE e.deleted_at IS NULL
           AND a.mime_type LIKE '%pdf%'
           AND NOT EXISTS (SELECT 1 FROM fulltext f WHERE f.attachment_id = a.id)
         ORDER BY a.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<String, _>("file_path")))
        .collect())
}

/// エントリに紐づく（索引済み PDF の）全文を `(page, content)` のリストで返す。
/// 添付ごとの `attachment_id, page` 順で並べる。索引が無ければ空を返す。
/// `generate_summary`（fulltext ソース）と MCP の `get_fulltext` が共有する。
pub async fn get_entry_fulltext(
    pool: &SqlitePool,
    entry_id: i64,
) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT f.page AS page, f.content AS content
         FROM fulltext f
         JOIN attachments a ON a.id = f.attachment_id
         WHERE a.entry_id = ?
         ORDER BY f.attachment_id, f.page",
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<i64, _>("page"), r.get::<String, _>("content")))
        .collect())
}

/// エントリの索引済み全文ページ数（0 なら全文なし）。`get_entry` の `has_fulltext`
/// フラグや `get_fulltext` の総ページ数表示に使う軽量カウント。
pub async fn entry_fulltext_page_count(
    pool: &SqlitePool,
    entry_id: i64,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS cnt
         FROM fulltext f
         JOIN attachments a ON a.id = f.attachment_id
         WHERE a.entry_id = ?",
    )
    .bind(entry_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("cnt"))
}

pub(crate) fn build_match_expr(tokens: &[&str]) -> String {
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
    view: Option<&str>,
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
        let likes: Vec<&str> = tokens
            .iter()
            .map(|_| "f.content LIKE ? ESCAPE '\\'")
            .collect();
        sql.push_str(&likes.join(" AND "));
    } else {
        sql.push_str("fulltext MATCH ?");
    }

    // view スコープ（CR-001）。trash ビュー時はゴミ箱内、それ以外は現役のみ。
    if matches!(view, Some("trash")) {
        sql.push_str(
            " AND a.entry_id IN (SELECT id FROM entries WHERE deleted_at IS NOT NULL)",
        );
    } else {
        sql.push_str(
            " AND a.entry_id IN (SELECT id FROM entries WHERE deleted_at IS NULL)",
        );
    }

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
            q = q.bind(crate::db::entries::like_pattern(token));
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

/// エントリ要約を読む。全文検索系（`fulltext` / `document_nodes_fts`）のヒットで共有する。
pub(crate) async fn load_summary(pool: &SqlitePool, id: i64) -> Result<EntrySummary, sqlx::Error> {
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

        // file_path は UNIQUE（CR-008）なので entry.id を含めて一意にする。
        let att = add_attachment(
            pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
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

        let hits = search_fulltext(&pool, "transformer", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attachment_id, att_id);
        assert_eq!(hits[0].page, 1);
        assert!(hits[0].snippet.to_lowercase().contains("transformer"));
    }

    /// 自己修復は 1 回だけ走り（`true`）、2 回目以降は flag で skip（`false`）。
    /// 再構築後も既存の索引内容は検索でき、FTS5 integrity-check を通る。
    #[sqlx::test(migrations = "./migrations")]
    async fn rebuild_fulltext_fts_once_is_idempotent_and_healthy(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(
            &pool,
            att_id,
            &[(1, "Transformer architecture is described here.".to_string())],
        )
        .await
        .unwrap();

        let first = rebuild_fulltext_fts_once(&pool).await.unwrap();
        assert!(first, "初回は再構築が走る");
        let second = rebuild_fulltext_fts_once(&pool).await.unwrap();
        assert!(!second, "2 回目は flag で skip");

        // 再構築後も検索でき、FTS5 の integrity-check を通る。
        let hits = search_fulltext(&pool, "transformer", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        sqlx::query("INSERT INTO fulltext(fulltext) VALUES('integrity-check')")
            .execute(&pool)
            .await
            .expect("rebuild 後は FTS5 integrity-check を通る");
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

        let hits = search_fulltext(&pool, "attention", None, None, None).await.unwrap();
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

        let stale = search_fulltext(&pool, "old", None, None, None).await.unwrap();
        let fresh = search_fulltext(&pool, "fresh", None, None, None).await.unwrap();
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

        assert_eq!(search_fulltext(&pool, "original", None, None, None).await.unwrap().len(), 2);
        assert_eq!(search_fulltext(&pool, "replaced", None, None, None).await.unwrap().len(), 1);
        assert!(search_fulltext(&pool, "two original", None, None, None).await.unwrap().is_empty());

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

        let hits = search_fulltext(&pool, "needle", None, None, None).await.unwrap();
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
        let hits = search_fulltext(&pool, "  ", None, None, None).await.unwrap();
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

        let hits = search_fulltext(&pool, "transformer", Some(col_id), None, None)
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

        let hits = search_fulltext(&pool, "深層学習", None, None, None).await.unwrap();
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

        let hits_a = search_fulltext(&pool, "alpha", None, None, None).await.unwrap();
        let hits_b = search_fulltext(&pool, "beta", None, None, None).await.unwrap();
        assert!(hits_a.is_empty());
        assert!(hits_b.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn like_fallback_treats_wildcards_literally(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(
            &pool,
            att_id,
            &[
                (1, "uses a_b indexing".to_string()),
                (2, "uses acb indexing".to_string()),
            ],
        )
        .await
        .unwrap();

        // 短いトークン → LIKE フォールバック。`_` はリテラル扱いであること。
        let hits = search_fulltext(&pool, "a_", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1, "`_` must not act as a wildcard");
        assert_eq!(hits[0].page, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn short_query_uses_like_fallback(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att_id, &[(1, "AI models are evolving rapidly".to_string())])
            .await
            .unwrap();

        let hits = search_fulltext(&pool, "AI", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attachments_without_fulltext_lists_only_unindexed(pool: SqlitePool) {
        let (_, indexed) = setup_attachment(&pool, "Indexed").await;
        let (_, unindexed) = setup_attachment(&pool, "Unindexed").await;
        index_attachment(&pool, indexed, &[(1, "some text".to_string())])
            .await
            .unwrap();

        let missing = attachments_without_fulltext(&pool).await.unwrap();
        let ids: Vec<i64> = missing.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&unindexed));
        assert!(!ids.contains(&indexed));
        // file_path も返る。
        assert!(missing.iter().any(|(_, p)| p.ends_with("/p.pdf")));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attachments_without_fulltext_excludes_trashed_entries(pool: SqlitePool) {
        let (entry_id, att_id) = setup_attachment(&pool, "Trashed").await;
        crate::db::entries::trash_entry(&pool, entry_id).await.unwrap();

        let missing = attachments_without_fulltext(&pool).await.unwrap();
        assert!(!missing.iter().any(|(id, _)| *id == att_id));
    }
}
