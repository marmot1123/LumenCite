use std::collections::HashMap;

use crate::models::{Attachment, Author, Collection, EntryDetail, EntryRelation, EntrySummary, EntryInput, SidebarCounts, Tag};
use sqlx::{Row, SqlitePool};

#[derive(sqlx::FromRow)]
struct ExtraFieldRow {
    field_name: String,
    field_value: String,
}

pub(crate) async fn sync_entries_fts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    entry_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM entries_fts WHERE rowid = ?")
        .bind(entry_id)
        .execute(&mut **tx)
        .await?;

    sqlx::query(
        "INSERT INTO entries_fts (rowid, title, authors_text, tags_text, abstract_text, identifiers)
         SELECT
             e.id,
             COALESCE(e.title, ''),
             COALESCE((
                 SELECT GROUP_CONCAT(a.name, ' ')
                 FROM entry_authors ea
                 JOIN authors a ON a.id = ea.author_id
                 WHERE ea.entry_id = e.id
             ), ''),
             COALESCE((
                 SELECT GROUP_CONCAT(t.name, ' ')
                 FROM entry_tags et
                 JOIN tags t ON t.id = et.tag_id
                 WHERE et.entry_id = e.id
             ), ''),
             COALESCE(e.abstract, ''),
             TRIM(
                 COALESCE(e.doi, '')      || ' ' ||
                 COALESCE(e.isbn, '')     || ' ' ||
                 COALESCE(e.arxiv_id, '') || ' ' ||
                 COALESCE(CAST(e.year AS TEXT), '')
             )
         FROM entries e
         WHERE e.id = ?",
    )
    .bind(entry_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

fn build_fts_match_expr(tokens: &[&str]) -> String {
    // 空白区切りの各トークンをダブルクォートで括る。trigram トークナイザはクエリも
    // 3-gram に分解するため、フレーズ括りだけで部分文字列検索になる。複数トークンは暗黙 AND。
    tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub async fn search_entries(
    pool: &SqlitePool,
    query: &str,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
) -> Result<Vec<EntrySummary>, sqlx::Error> {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return get_entries(pool, collection_id, tag_id, None).await;
    }

    // trigram は 3 文字未満のクエリを正しく扱えないため、最短トークンが 3 文字未満の場合は
    // entries_fts の各列を LIKE でスキャンするフォールバックパスを使う。
    let ids = if tokens.iter().any(|t| t.chars().count() < 3) {
        search_ids_like(pool, &tokens, collection_id, tag_id).await?
    } else {
        search_ids_fts(pool, &tokens, collection_id, tag_id).await?
    };

    let mut summaries = Vec::new();
    for id in ids {
        summaries.push(load_summary(pool, id).await?);
    }
    Ok(summaries)
}

async fn search_ids_fts(
    pool: &SqlitePool,
    tokens: &[&str],
    collection_id: Option<i64>,
    tag_id: Option<i64>,
) -> Result<Vec<i64>, sqlx::Error> {
    let match_expr = build_fts_match_expr(tokens);
    let mut sql = String::from(
        "SELECT e.id AS id
         FROM entries e
         JOIN entries_fts f ON f.rowid = e.id
         WHERE entries_fts MATCH ? AND e.deleted_at IS NULL",
    );
    if collection_id.is_some() {
        sql.push_str(
            " AND e.id IN (SELECT entry_id FROM entry_collections WHERE collection_id = ?)",
        );
    }
    if tag_id.is_some() {
        sql.push_str(" AND e.id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ?)");
    }
    sql.push_str(" ORDER BY bm25(entries_fts)");

    let mut q = sqlx::query(&sql).bind(&match_expr);
    if let Some(cid) = collection_id {
        q = q.bind(cid);
    }
    if let Some(tid) = tag_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_all(pool).await?.iter().map(|r| r.get("id")).collect())
}

async fn search_ids_like(
    pool: &SqlitePool,
    tokens: &[&str],
    collection_id: Option<i64>,
    tag_id: Option<i64>,
) -> Result<Vec<i64>, sqlx::Error> {
    let mut sql = String::from(
        "SELECT e.id AS id
         FROM entries e
         JOIN entries_fts f ON f.rowid = e.id
         WHERE e.deleted_at IS NULL",
    );
    for _ in tokens {
        sql.push_str(
            " AND (f.title LIKE ? OR f.authors_text LIKE ? OR f.tags_text LIKE ?
                   OR f.abstract_text LIKE ? OR f.identifiers LIKE ?)",
        );
    }
    if collection_id.is_some() {
        sql.push_str(
            " AND e.id IN (SELECT entry_id FROM entry_collections WHERE collection_id = ?)",
        );
    }
    if tag_id.is_some() {
        sql.push_str(" AND e.id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ?)");
    }
    sql.push_str(" ORDER BY e.created_at DESC");

    let mut q = sqlx::query(&sql);
    for token in tokens {
        let pattern = format!("%{}%", token);
        for _ in 0..5 {
            q = q.bind(pattern.clone());
        }
    }
    if let Some(cid) = collection_id {
        q = q.bind(cid);
    }
    if let Some(tid) = tag_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_all(pool).await?.iter().map(|r| r.get("id")).collect())
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

    let has_attachment: bool =
        sqlx::query("SELECT COUNT(*) as cnt FROM attachments WHERE entry_id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .map(|r| r.get::<i64, _>("cnt") > 0)
            .unwrap_or(false);

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
        has_attachment,
        journal,
        starred: row.get::<i64, _>("starred") != 0,
    })
}

pub async fn create_entry(
    pool: &SqlitePool,
    input: &EntryInput,
) -> Result<EntryDetail, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let citation_key = input.citation_key.as_deref().and_then(sanitize_citation_key);

    let result = sqlx::query(
        "INSERT INTO entries (title, year, entry_type, citation_key, doi, isbn, arxiv_id, url, abstract, notes)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.title)
    .bind(input.year)
    .bind(&input.entry_type)
    .bind(&citation_key)
    .bind(&input.doi)
    .bind(&input.isbn)
    .bind(&input.arxiv_id)
    .bind(&input.url)
    .bind(&input.abstract_)
    .bind(&input.notes)
    .execute(&mut *tx)
    .await?;

    let entry_id = result.last_insert_rowid();

    for (pos, name) in input.author_names.iter().enumerate() {
        let author = get_or_create_author(&mut tx, name).await?;
        sqlx::query(
            "INSERT INTO entry_authors (entry_id, author_id, position) VALUES (?, ?, ?)",
        )
        .bind(entry_id)
        .bind(author.id)
        .bind(pos as i64)
        .execute(&mut *tx)
        .await?;
    }

    for tag_id in &input.tag_ids {
        sqlx::query("INSERT INTO entry_tags (entry_id, tag_id) VALUES (?, ?)")
            .bind(entry_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }

    for (field_name, field_value) in &input.extra_fields {
        sqlx::query(
            "INSERT INTO extra_fields (entry_id, field_name, field_value) VALUES (?, ?, ?)",
        )
        .bind(entry_id)
        .bind(field_name)
        .bind(field_value)
        .execute(&mut *tx)
        .await?;
    }

    sync_entries_fts(&mut tx, entry_id).await?;

    tx.commit().await?;

    get_entry(pool, entry_id).await
}

/// 単一 ID から EntrySummary を組み立てる。`get_entries` の内部ループと同じ
/// クエリ群を 1 エントリに対して実行する。関連エントリ・将来の検索結果など
/// ID リストから一括にサマリーを取りたいケースで使う。
async fn load_entry_summary(pool: &SqlitePool, id: i64) -> Result<EntrySummary, sqlx::Error> {
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

    let has_attachment: bool = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM attachments WHERE entry_id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map(|n| n > 0)
    .unwrap_or(false);

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
        has_attachment,
        journal,
        starred: row.get::<i64, _>("starred") != 0,
    })
}

pub async fn get_entry(pool: &SqlitePool, id: i64) -> Result<EntryDetail, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, title, year, entry_type, citation_key, doi, isbn, arxiv_id, url, abstract, notes,
                summary, summary_model, summary_generated_at,
                created_at, starred, deleted_at
         FROM entries WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(sqlx::Error::RowNotFound)?;

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

    let extra_fields: HashMap<String, String> = sqlx::query_as::<_, ExtraFieldRow>(
        "SELECT field_name, field_value FROM extra_fields WHERE entry_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.field_name, r.field_value))
    .collect();

    let attachments: Vec<Attachment> = sqlx::query_as(
        "SELECT id, entry_id, file_name, mime_type, created_at FROM attachments WHERE entry_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let has_attachment = !attachments.is_empty();

    let collections: Vec<Collection> = sqlx::query(
        "SELECT c.id, c.name, c.parent_id FROM collections c
         JOIN entry_collections ec ON ec.collection_id = c.id
         WHERE ec.entry_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| Collection {
        id: r.get("id"),
        name: r.get("name"),
        parent_id: r.get("parent_id"),
        children: vec![],
    })
    .collect();

    // 関連エントリ。current が from 側か to 側かで direction を決め、もう一方の
    // エントリ ID を取り出す。相手側がゴミ箱に入っていたら除外する。
    let relation_rows = sqlx::query(
        "SELECT
             CASE WHEN r.from_entry_id = ?1 THEN r.to_entry_id ELSE r.from_entry_id END AS other_id,
             CASE WHEN r.from_entry_id = ?1 THEN 'from' ELSE 'to' END AS direction,
             r.relation_type AS relation_type
         FROM entry_relations r
         JOIN entries e ON e.id =
             CASE WHEN r.from_entry_id = ?1 THEN r.to_entry_id ELSE r.from_entry_id END
         WHERE (r.from_entry_id = ?1 OR r.to_entry_id = ?1)
           AND e.deleted_at IS NULL",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let mut relations: Vec<EntryRelation> = Vec::with_capacity(relation_rows.len());
    for r in relation_rows {
        let other_id: i64 = r.get("other_id");
        let direction: String = r.get("direction");
        let relation_type: String = r.get("relation_type");
        let entry = load_entry_summary(pool, other_id).await?;
        relations.push(EntryRelation { entry, relation_type, direction });
    }

    Ok(EntryDetail {
        id: row.get("id"),
        title: row.get("title"),
        year: row.get("year"),
        entry_type: row.get("entry_type"),
        citation_key: row.get("citation_key"),
        doi: row.get("doi"),
        isbn: row.get("isbn"),
        arxiv_id: row.get("arxiv_id"),
        url: row.get("url"),
        abstract_: row.get("abstract"),
        notes: row.get("notes"),
        summary: row.get("summary"),
        summary_model: row.get("summary_model"),
        summary_generated_at: row.get("summary_generated_at"),
        created_at: row.get("created_at"),
        starred: row.get::<i64, _>("starred") != 0,
        deleted_at: row.get("deleted_at"),
        authors,
        tags,
        has_attachment,
        extra_fields,
        attachments,
        relations,
        collections,
    })
}

/// LLM 生成要約を保存する。
pub async fn set_summary(
    pool: &SqlitePool,
    id: i64,
    summary: &str,
    model: &str,
) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE entries
         SET summary = ?, summary_model = ?, summary_generated_at = datetime('now'),
             updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(summary)
    .bind(model)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn get_entries(
    pool: &SqlitePool,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
) -> Result<Vec<EntrySummary>, sqlx::Error> {
    // view = "trash" 以外はゴミ箱（deleted_at IS NOT NULL）を除外する。
    let trash_view = matches!(view, Some("trash"));
    let trash_clause = if trash_view {
        "e.deleted_at IS NOT NULL"
    } else {
        "e.deleted_at IS NULL"
    };

    // フィルタ条件に応じて対象エントリIDを取得
    let ids: Vec<i64> = if let Some(tid) = tag_id {
        let sql = format!(
            "SELECT e.id FROM entries e
             JOIN entry_tags et ON et.entry_id = e.id
             WHERE et.tag_id = ? AND {trash_clause}
             ORDER BY e.created_at DESC"
        );
        sqlx::query(&sql)
            .bind(tid)
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| r.get("id"))
            .collect()
    } else if let Some(cid) = collection_id {
        let sql = format!(
            "SELECT e.id FROM entries e
             JOIN entry_collections ec ON ec.entry_id = e.id
             WHERE ec.collection_id = ? AND {trash_clause}
             ORDER BY e.created_at DESC"
        );
        sqlx::query(&sql)
            .bind(cid)
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| r.get("id"))
            .collect()
    } else {
        match view {
            Some("starred") => sqlx::query(
                "SELECT id FROM entries
                 WHERE starred = 1 AND deleted_at IS NULL
                 ORDER BY created_at DESC",
            )
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| r.get("id"))
            .collect(),
            Some("unfiled") => sqlx::query(
                "SELECT e.id FROM entries e
                 WHERE e.deleted_at IS NULL
                   AND NOT EXISTS (
                     SELECT 1 FROM entry_collections ec WHERE ec.entry_id = e.id
                   )
                 ORDER BY e.created_at DESC",
            )
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| r.get("id"))
            .collect(),
            Some("trash") => sqlx::query(
                "SELECT id FROM entries
                 WHERE deleted_at IS NOT NULL
                 ORDER BY deleted_at DESC",
            )
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| r.get("id"))
            .collect(),
            _ => sqlx::query(
                "SELECT id FROM entries
                 WHERE deleted_at IS NULL
                 ORDER BY created_at DESC",
            )
            .fetch_all(pool)
            .await?
            .iter()
            .map(|r| r.get("id"))
            .collect(),
        }
    };

    // 各IDについてサマリーを組み立てる
    let mut summaries = Vec::new();
    for id in ids {
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

        let has_attachment: bool =
            sqlx::query("SELECT COUNT(*) as cnt FROM attachments WHERE entry_id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .map(|r| r.get::<i64, _>("cnt") > 0)
                .unwrap_or(false);

        let journal: Option<String> = sqlx::query_scalar(
            "SELECT field_value FROM extra_fields WHERE entry_id = ? AND field_name = 'journal'",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        summaries.push(EntrySummary {
            id: row.get("id"),
            title: row.get("title"),
            year: row.get("year"),
            entry_type: row.get("entry_type"),
            created_at: row.get("created_at"),
            authors,
            tags,
            has_attachment,
            journal,
            starred: row.get::<i64, _>("starred") != 0,
        });
    }

    Ok(summaries)
}

pub async fn update_entry(
    pool: &SqlitePool,
    id: i64,
    input: &EntryInput,
) -> Result<EntryDetail, sqlx::Error> {
    let mut tx = pool.begin().await?;

    let citation_key = input.citation_key.as_deref().and_then(sanitize_citation_key);

    let rows_affected = sqlx::query(
        "UPDATE entries
         SET title = ?, year = ?, entry_type = ?, citation_key = ?, doi = ?, isbn = ?, arxiv_id = ?,
             url = ?, abstract = ?, notes = ?, updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(&input.title)
    .bind(input.year)
    .bind(&input.entry_type)
    .bind(&citation_key)
    .bind(&input.doi)
    .bind(&input.isbn)
    .bind(&input.arxiv_id)
    .bind(&input.url)
    .bind(&input.abstract_)
    .bind(&input.notes)
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Err(sqlx::Error::RowNotFound);
    }

    sqlx::query("DELETE FROM entry_authors WHERE entry_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    for (pos, name) in input.author_names.iter().enumerate() {
        let author = get_or_create_author(&mut tx, name).await?;
        sqlx::query(
            "INSERT INTO entry_authors (entry_id, author_id, position) VALUES (?, ?, ?)",
        )
        .bind(id)
        .bind(author.id)
        .bind(pos as i64)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("DELETE FROM entry_tags WHERE entry_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    for tag_id in &input.tag_ids {
        sqlx::query("INSERT INTO entry_tags (entry_id, tag_id) VALUES (?, ?)")
            .bind(id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query("DELETE FROM extra_fields WHERE entry_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    for (field_name, field_value) in &input.extra_fields {
        sqlx::query(
            "INSERT INTO extra_fields (entry_id, field_name, field_value) VALUES (?, ?, ?)",
        )
        .bind(id)
        .bind(field_name)
        .bind(field_value)
        .execute(&mut *tx)
        .await?;
    }

    sync_entries_fts(&mut tx, id).await?;

    tx.commit().await?;

    get_entry(pool, id).await
}

pub async fn get_sidebar_counts(pool: &SqlitePool) -> Result<SidebarCounts, sqlx::Error> {
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM entries WHERE deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;

    let starred: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM entries WHERE starred = 1 AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;

    let trash: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM entries WHERE deleted_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;

    let unfiled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM entries e
         WHERE e.deleted_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM entry_collections ec WHERE ec.entry_id = e.id)",
    )
    .fetch_one(pool)
    .await?;

    let collections: std::collections::HashMap<i64, i64> = sqlx::query(
        "SELECT ec.collection_id AS collection_id, COUNT(*) AS cnt
         FROM entry_collections ec
         JOIN entries e ON e.id = ec.entry_id
         WHERE e.deleted_at IS NULL
         GROUP BY ec.collection_id",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.get::<i64, _>("collection_id"), r.get::<i64, _>("cnt")))
    .collect();

    let tags: std::collections::HashMap<i64, i64> = sqlx::query(
        "SELECT et.tag_id AS tag_id, COUNT(*) AS cnt
         FROM entry_tags et
         JOIN entries e ON e.id = et.entry_id
         WHERE e.deleted_at IS NULL
         GROUP BY et.tag_id",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.get::<i64, _>("tag_id"), r.get::<i64, _>("cnt")))
    .collect();

    Ok(SidebarCounts { total, starred, unfiled, trash, collections, tags })
}

pub async fn bulk_trash(pool: &SqlitePool, ids: &[i64]) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query(
            "UPDATE entries
             SET deleted_at = datetime('now'), updated_at = datetime('now')
             WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn bulk_restore(pool: &SqlitePool, ids: &[i64]) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query(
            "UPDATE entries
             SET deleted_at = NULL, updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// 永久削除（hard delete）の一括版。entries_fts と fulltext も同時にクリーンアップする。
pub async fn bulk_purge(pool: &SqlitePool, ids: &[i64]) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query("DELETE FROM entries_fts WHERE rowid = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM fulltext WHERE attachment_id IN (
                SELECT id FROM attachments WHERE entry_id = ?
            )",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM entries WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn bulk_add_to_collection(
    pool: &SqlitePool,
    ids: &[i64],
    collection_id: i64,
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query(
            "INSERT OR IGNORE INTO entry_collections (entry_id, collection_id) VALUES (?, ?)",
        )
        .bind(id)
        .bind(collection_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn bulk_add_tag(
    pool: &SqlitePool,
    ids: &[i64],
    tag_id: i64,
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query(
            "INSERT OR IGNORE INTO entry_tags (entry_id, tag_id) VALUES (?, ?)",
        )
        .bind(id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
        // タグの変更は entries_fts.tags_text に影響するので再同期する
        sync_entries_fts(&mut tx, *id).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// ISBN は表記揺れ（ハイフン・空白・ハイフン無し）が多いので、英数字のみに
/// 正規化したうえで大文字化（末尾のチェック桁 X 対策）する。
fn normalize_isbn(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()
}

/// BibTeX エントリキーとして安全な文字（英数字と `_ : - . / +`）のみを残す。
/// トリム後に空になった場合は `None`（= 自動生成扱い）を返す。
pub fn sanitize_citation_key(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-' | '.' | '/' | '+'))
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// 固定 cite key が使用可能か（サニタイズ後に他エントリと重複しないか）を返す。
/// 空キー（サニタイズ後 None）は自動生成扱いとして常に `true`。`exclude_id` には
/// 編集中エントリ自身の id を渡し、自分との衝突を除外する。
pub async fn is_citation_key_available(
    pool: &SqlitePool,
    key: &str,
    exclude_id: Option<i64>,
) -> Result<bool, sqlx::Error> {
    let Some(k) = sanitize_citation_key(key) else {
        return Ok(true);
    };
    let count: i64 = match exclude_id {
        Some(id) => sqlx::query_scalar(
            "SELECT COUNT(*) FROM entries WHERE citation_key = ? AND id != ?",
        )
        .bind(&k)
        .bind(id)
        .fetch_one(pool)
        .await?,
        None => sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE citation_key = ?")
            .bind(&k)
            .fetch_one(pool)
            .await?,
    };
    Ok(count == 0)
}

/// DOI / arXiv ID / ISBN のいずれかが既存エントリと一致するか確認し、最初に
/// 見つかった `entries.id` を返す。すべて None なら None。比較は DOI/arXiv は
/// 大文字小文字無視、ISBN は記号を除いた英数字で正規化する。ゴミ箱内のエン
/// トリは対象外。
pub async fn find_duplicate_entry(
    pool: &SqlitePool,
    doi: Option<&str>,
    arxiv_id: Option<&str>,
    isbn: Option<&str>,
) -> Result<Option<i64>, sqlx::Error> {
    let doi_norm = doi.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let arxiv_norm = arxiv_id.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let isbn_norm = isbn.map(normalize_isbn).filter(|s| !s.is_empty());

    if doi_norm.is_none() && arxiv_norm.is_none() && isbn_norm.is_none() {
        return Ok(None);
    }

    // 値が None の引数は NULL を bind し、SQL 側で `? IS NOT NULL AND ...` で
    // 弾く。`REPLACE(...)` で ISBN のハイフン／空白を除去してから比較する。
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM entries
         WHERE deleted_at IS NULL
           AND (
                (?1 IS NOT NULL AND doi IS NOT NULL AND LOWER(doi) = ?1)
             OR (?2 IS NOT NULL AND arxiv_id IS NOT NULL AND LOWER(arxiv_id) = ?2)
             OR (?3 IS NOT NULL AND isbn IS NOT NULL
                   AND UPPER(REPLACE(REPLACE(REPLACE(isbn, '-', ''), ' ', ''), char(9), '')) = ?3)
           )
         ORDER BY id ASC
         LIMIT 1",
    )
    .bind(doi_norm)
    .bind(arxiv_norm)
    .bind(isbn_norm)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(id,)| id))
}

pub async fn set_starred(pool: &SqlitePool, id: i64, starred: bool) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE entries SET starred = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(if starred { 1_i64 } else { 0_i64 })
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn trash_entry(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE entries
         SET deleted_at = datetime('now'), updated_at = datetime('now')
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows == 0 {
        // 既にゴミ箱に入っている、または存在しない
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE id = ?")
                .bind(id)
                .fetch_one(pool)
                .await?;
        if exists == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
    }
    Ok(())
}

pub async fn restore_entry(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE entries
         SET deleted_at = NULL, updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn delete_entry(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM entries_fts WHERE rowid = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    let rows_affected = sqlx::query("DELETE FROM entries WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    if rows_affected == 0 {
        return Err(sqlx::Error::RowNotFound);
    }

    tx.commit().await?;
    Ok(())
}

async fn get_or_create_author(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name: &str,
) -> Result<Author, sqlx::Error> {
    // M2 では従来どおり `name` 完全一致のみで照合する（ORCID / NFKC 正規化照合は M3）。
    let existing: Option<Author> = sqlx::query_as(AUTHOR_SELECT_BY_NAME)
        .bind(name)
        .fetch_optional(&mut **tx)
        .await?;

    if let Some(author) = existing {
        return Ok(author);
    }

    let result = sqlx::query("INSERT INTO authors (name) VALUES (?)")
        .bind(name)
        .execute(&mut **tx)
        .await?;

    // 列 DEFAULT（is_organization=0 など）を取り違えないよう、挿入後に同一行を再フェッチする。
    let inserted: Author = sqlx::query_as(AUTHOR_SELECT_BY_ID)
        .bind(result.last_insert_rowid())
        .fetch_one(&mut **tx)
        .await?;
    Ok(inserted)
}

// `authors` 1 行の全カラムを Author 構造体の field 名で SELECT する SQL。
// FromRow が field 名でマッチングするため、SELECT する列名と Author 構造体の field 名を揃える。
// `identifiers` は別テーブル JOIN で詰めるため対象外（M3 で db/authors.rs ヘルパーに切り出す）。
const AUTHOR_SELECT_BY_NAME: &str =
    "SELECT id, name,
            given_name, middle_name, family_name, suffix, name_particle,
            name_original, given_name_original, family_name_original, original_script,
            reading_family, reading_given,
            is_organization,
            email, homepage_url, notes,
            orcid, updated_at
       FROM authors WHERE name = ?";

const AUTHOR_SELECT_BY_ID: &str =
    "SELECT id, name,
            given_name, middle_name, family_name, suffix, name_particle,
            name_original, given_name_original, family_name_original, original_script,
            reading_family, reading_given,
            is_organization,
            email, homepage_url, notes,
            orcid, updated_at
       FROM authors WHERE id = ?";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntryInput;

    // ── create_entry ────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_saves_basic_fields(pool: SqlitePool) {
        let input = EntryInput {
            title: "Deep Learning".to_string(),
            year: Some(2024),
            entry_type: "article".to_string(),
            ..Default::default()
        };

        let entry = create_entry(&pool, &input).await.unwrap();

        assert!(entry.id > 0);
        assert_eq!(entry.title, "Deep Learning");
        assert_eq!(entry.year, Some(2024));
        assert_eq!(entry.entry_type, "article");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_with_new_authors(pool: SqlitePool) {
        let input = EntryInput {
            title: "Attention Is All You Need".to_string(),
            entry_type: "article".to_string(),
            author_names: vec![
                "Ashish Vaswani".to_string(),
                "Noam Shazeer".to_string(),
            ],
            ..Default::default()
        };

        let entry = create_entry(&pool, &input).await.unwrap();

        assert_eq!(entry.authors.len(), 2);
        assert_eq!(entry.authors[0].name, "Ashish Vaswani");
        assert_eq!(entry.authors[1].name, "Noam Shazeer");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_reuses_existing_author(pool: SqlitePool) {
        let input1 = EntryInput {
            title: "Paper 1".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Alice Smith".to_string()],
            ..Default::default()
        };
        let input2 = EntryInput {
            title: "Paper 2".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Alice Smith".to_string()],
            ..Default::default()
        };

        let entry1 = create_entry(&pool, &input1).await.unwrap();
        let entry2 = create_entry(&pool, &input2).await.unwrap();

        assert_eq!(entry1.authors[0].id, entry2.authors[0].id);
    }

    // ── citation_key ─────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_stores_pinned_citation_key(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Pinned".to_string(),
            entry_type: "article".to_string(),
            citation_key: Some("smith2020".to_string()),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(entry.citation_key, Some("smith2020".to_string()));

        let fetched = get_entry(&pool, entry.id).await.unwrap();
        assert_eq!(fetched.citation_key, Some("smith2020".to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_sanitizes_citation_key(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Dirty".to_string(),
            entry_type: "article".to_string(),
            citation_key: Some("  smith {2020}, x ".to_string()),
            ..Default::default()
        }).await.unwrap();
        // 空白・カンマ・波括弧は除去される
        assert_eq!(entry.citation_key, Some("smith2020x".to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_blank_citation_key_becomes_none(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Auto".to_string(),
            entry_type: "article".to_string(),
            citation_key: Some("   ".to_string()),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(entry.citation_key, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_changes_citation_key(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "P".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(entry.citation_key, None);

        let updated = update_entry(&pool, entry.id, &EntryInput {
            title: "P".to_string(),
            entry_type: "article".to_string(),
            citation_key: Some("custom:key-1".to_string()),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(updated.citation_key, Some("custom:key-1".to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn pinned_citation_key_must_be_unique(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            citation_key: Some("dup2020".to_string()),
            ..Default::default()
        }).await.unwrap();

        let second = create_entry(&pool, &EntryInput {
            title: "B".to_string(),
            entry_type: "article".to_string(),
            citation_key: Some("dup2020".to_string()),
            ..Default::default()
        }).await;
        assert!(second.is_err(), "重複した固定キーは UNIQUE 制約で拒否されるべき");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn null_citation_keys_can_coexist(pool: SqlitePool) {
        // citation_key NULL（自動）は複数行で許容される
        create_entry(&pool, &EntryInput { title: "A".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        create_entry(&pool, &EntryInput { title: "B".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        let all = get_entries(&pool, None, None, None).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn is_citation_key_available_checks(pool: SqlitePool) {
        let a = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            citation_key: Some("taken2020".to_string()),
            ..Default::default()
        }).await.unwrap();

        // 既存と重複 → false
        assert!(!is_citation_key_available(&pool, "taken2020", None).await.unwrap());
        // 未使用 → true
        assert!(is_citation_key_available(&pool, "free2021", None).await.unwrap());
        // 自分自身を除外すれば自分のキーは使用可能
        assert!(is_citation_key_available(&pool, "taken2020", Some(a.id)).await.unwrap());
        // 空キー（自動）は常に true
        assert!(is_citation_key_available(&pool, "   ", None).await.unwrap());
    }

    // ── get_entry ────────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_returns_full_detail(pool: SqlitePool) {
        let input = EntryInput {
            title: "Test Paper".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/test".to_string()),
            year: Some(2023),
            ..Default::default()
        };
        let created = create_entry(&pool, &input).await.unwrap();

        let fetched = get_entry(&pool, created.id).await.unwrap();

        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.title, "Test Paper");
        assert_eq!(fetched.doi, Some("10.1234/test".to_string()));
        assert_eq!(fetched.year, Some(2023));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_returns_authors_in_order(pool: SqlitePool) {
        let input = EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["First Author".to_string(), "Second Author".to_string()],
            ..Default::default()
        };
        let created = create_entry(&pool, &input).await.unwrap();

        let fetched = get_entry(&pool, created.id).await.unwrap();

        assert_eq!(fetched.authors.len(), 2);
        assert_eq!(fetched.authors[0].name, "First Author");
        assert_eq!(fetched.authors[1].name, "Second Author");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_not_found_returns_error(pool: SqlitePool) {
        let result = get_entry(&pool, 9999).await;

        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    // ── get_entries ──────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entries_returns_all(pool: SqlitePool) {
        create_entry(&pool, &EntryInput { title: "Paper A".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        create_entry(&pool, &EntryInput { title: "Paper B".to_string(), entry_type: "book".to_string(), ..Default::default() }).await.unwrap();

        let entries = get_entries(&pool, None, None, None).await.unwrap();

        assert_eq!(entries.len(), 2);
    }

    // ── bulk operations ──────────────────────────────────────────────────────

    async fn make_entry(pool: &SqlitePool, title: &str) -> i64 {
        create_entry(pool, &EntryInput {
            title: title.to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap().id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_trash_marks_each_id(pool: SqlitePool) {
        let a = make_entry(&pool, "A").await;
        let b = make_entry(&pool, "B").await;
        let c = make_entry(&pool, "C").await;

        bulk_trash(&pool, &[a, b]).await.unwrap();

        let visible = get_entries(&pool, None, None, None).await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, c);
        let trashed = get_entries(&pool, None, None, Some("trash")).await.unwrap();
        assert_eq!(trashed.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_trash_empty_input_is_noop(pool: SqlitePool) {
        make_entry(&pool, "A").await;
        bulk_trash(&pool, &[]).await.unwrap();
        let visible = get_entries(&pool, None, None, None).await.unwrap();
        assert_eq!(visible.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_restore_clears_deleted_at(pool: SqlitePool) {
        let a = make_entry(&pool, "A").await;
        let b = make_entry(&pool, "B").await;
        bulk_trash(&pool, &[a, b]).await.unwrap();

        bulk_restore(&pool, &[a, b]).await.unwrap();
        let visible = get_entries(&pool, None, None, None).await.unwrap();
        assert_eq!(visible.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_purge_removes_rows_and_fts(pool: SqlitePool) {
        let a = make_entry(&pool, "Disposable A").await;
        let b = make_entry(&pool, "Disposable B").await;

        bulk_purge(&pool, &[a, b]).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
        let fts_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries_fts")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(fts_count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_add_to_collection_inserts_each(pool: SqlitePool) {
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Reading')")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let a = make_entry(&pool, "A").await;
        let b = make_entry(&pool, "B").await;

        bulk_add_to_collection(&pool, &[a, b], col_id).await.unwrap();

        let entries = get_entries(&pool, Some(col_id), None, None).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_add_to_collection_is_idempotent(pool: SqlitePool) {
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Reading')")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let a = make_entry(&pool, "A").await;

        bulk_add_to_collection(&pool, &[a, a], col_id).await.unwrap();
        bulk_add_to_collection(&pool, &[a], col_id).await.unwrap();

        let entries = get_entries(&pool, Some(col_id), None, None).await.unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_add_tag_syncs_fts_for_tag_name_search(pool: SqlitePool) {
        let tag_id = sqlx::query("INSERT INTO tags (name) VALUES ('transformer-architecture')")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let a = make_entry(&pool, "Paper A").await;
        let b = make_entry(&pool, "Paper B").await;

        bulk_add_tag(&pool, &[a, b], tag_id).await.unwrap();

        let hits = search_entries(&pool, "transformer", None, None).await.unwrap();
        assert_eq!(hits.len(), 2, "bulk_add_tag 後にタグ名で検索できるべき");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_add_tag_inserts_each(pool: SqlitePool) {
        let tag_id = sqlx::query("INSERT INTO tags (name) VALUES ('ml')")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let a = make_entry(&pool, "A").await;
        let b = make_entry(&pool, "B").await;

        bulk_add_tag(&pool, &[a, b], tag_id).await.unwrap();

        let entries = get_entries(&pool, None, Some(tag_id), None).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    // ── sidebar counts ───────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn get_sidebar_counts_excludes_trashed_from_total(pool: SqlitePool) {
        let a = create_entry(&pool, &EntryInput {
            title: "A".to_string(), entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let b = create_entry(&pool, &EntryInput {
            title: "B".to_string(), entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        trash_entry(&pool, b.id).await.unwrap();

        let counts = get_sidebar_counts(&pool).await.unwrap();
        assert_eq!(counts.total, 1, "total should exclude trashed");
        assert_eq!(counts.trash, 1);
        // sanity: starred zero by default
        assert_eq!(counts.starred, 0);
        // a is not in any collection → unfiled = 1
        assert_eq!(counts.unfiled, 1);
        let _ = a;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_sidebar_counts_starred_only_counts_non_trashed(pool: SqlitePool) {
        let a = create_entry(&pool, &EntryInput {
            title: "A".to_string(), entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let b = create_entry(&pool, &EntryInput {
            title: "B".to_string(), entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        set_starred(&pool, a.id, true).await.unwrap();
        set_starred(&pool, b.id, true).await.unwrap();
        trash_entry(&pool, b.id).await.unwrap();

        let counts = get_sidebar_counts(&pool).await.unwrap();
        assert_eq!(counts.starred, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_sidebar_counts_returns_per_collection_counts(pool: SqlitePool) {
        let col_a = sqlx::query("INSERT INTO collections (name) VALUES ('A')")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let col_b = sqlx::query("INSERT INTO collections (name) VALUES ('B')")
            .execute(&pool).await.unwrap().last_insert_rowid();

        let e1 = create_entry(&pool, &EntryInput { title: "1".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        let e2 = create_entry(&pool, &EntryInput { title: "2".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        let e3 = create_entry(&pool, &EntryInput { title: "3".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        for (eid, cid) in [(e1.id, col_a), (e2.id, col_a), (e3.id, col_b)] {
            sqlx::query("INSERT INTO entry_collections (entry_id, collection_id) VALUES (?, ?)")
                .bind(eid).bind(cid).execute(&pool).await.unwrap();
        }
        // trashing one should drop the collection count
        trash_entry(&pool, e2.id).await.unwrap();

        let counts = get_sidebar_counts(&pool).await.unwrap();
        assert_eq!(counts.collections.get(&col_a).copied().unwrap_or(0), 1);
        assert_eq!(counts.collections.get(&col_b).copied().unwrap_or(0), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_sidebar_counts_returns_per_tag_counts(pool: SqlitePool) {
        let tag_id = sqlx::query("INSERT INTO tags (name) VALUES ('ml')")
            .execute(&pool).await.unwrap().last_insert_rowid();

        let e1 = create_entry(&pool, &EntryInput {
            title: "Tagged".to_string(), entry_type: "article".to_string(),
            tag_ids: vec![tag_id], ..Default::default()
        }).await.unwrap();
        let e2 = create_entry(&pool, &EntryInput {
            title: "TaggedTrashed".to_string(), entry_type: "article".to_string(),
            tag_ids: vec![tag_id], ..Default::default()
        }).await.unwrap();
        trash_entry(&pool, e2.id).await.unwrap();

        let counts = get_sidebar_counts(&pool).await.unwrap();
        assert_eq!(counts.tags.get(&tag_id).copied().unwrap_or(0), 1);
        let _ = e1;
    }

    // ── starred / trash ──────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn new_entry_is_not_starred(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Plain".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        assert!(!entry.starred);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_starred_toggles_flag(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Starable".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        set_starred(&pool, entry.id, true).await.unwrap();
        let detail = get_entry(&pool, entry.id).await.unwrap();
        assert!(detail.starred);

        set_starred(&pool, entry.id, false).await.unwrap();
        let detail = get_entry(&pool, entry.id).await.unwrap();
        assert!(!detail.starred);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn trash_entry_removes_from_default_listing(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "To trash".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        trash_entry(&pool, entry.id).await.unwrap();

        let entries = get_entries(&pool, None, None, None).await.unwrap();
        assert!(!entries.iter().any(|e| e.id == entry.id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn restore_entry_brings_back_to_default_listing(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Restorable".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        trash_entry(&pool, entry.id).await.unwrap();
        restore_entry(&pool, entry.id).await.unwrap();

        let entries = get_entries(&pool, None, None, None).await.unwrap();
        assert!(entries.iter().any(|e| e.id == entry.id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entries_starred_view_returns_only_starred(pool: SqlitePool) {
        let starred = create_entry(&pool, &EntryInput {
            title: "Starred".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Plain".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        set_starred(&pool, starred.id, true).await.unwrap();

        let entries = get_entries(&pool, None, None, Some("starred")).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, starred.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entries_trash_view_returns_only_trashed(pool: SqlitePool) {
        let kept = create_entry(&pool, &EntryInput {
            title: "Kept".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let trashed = create_entry(&pool, &EntryInput {
            title: "Trashed".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        trash_entry(&pool, trashed.id).await.unwrap();

        let entries = get_entries(&pool, None, None, Some("trash")).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, trashed.id);

        let normal = get_entries(&pool, None, None, None).await.unwrap();
        assert_eq!(normal.len(), 1);
        assert_eq!(normal[0].id, kept.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entries_unfiled_view_returns_entries_without_collection(pool: SqlitePool) {
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Box')")
            .execute(&pool).await.unwrap().last_insert_rowid();

        let unfiled = create_entry(&pool, &EntryInput {
            title: "Unfiled".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let filed = create_entry(&pool, &EntryInput {
            title: "Filed".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        sqlx::query("INSERT INTO entry_collections (entry_id, collection_id) VALUES (?, ?)")
            .bind(filed.id).bind(col_id).execute(&pool).await.unwrap();

        let entries = get_entries(&pool, None, None, Some("unfiled")).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, unfiled.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_excludes_trashed(pool: SqlitePool) {
        let trashed = create_entry(&pool, &EntryInput {
            title: "Hidden Transformer Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Visible Transformer Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        trash_entry(&pool, trashed.id).await.unwrap();

        let hits = search_entries(&pool, "Transformer", None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Visible Transformer Paper");
    }

    // ── journal extra field ──────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entries_includes_journal_from_extra_fields(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "Article with journal".to_string(),
            entry_type: "article".to_string(),
            extra_fields: [("journal".to_string(), "Nature".to_string())].into(),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Article without journal".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let entries = get_entries(&pool, None, None, None).await.unwrap();

        let with_j = entries.iter().find(|e| e.title == "Article with journal").unwrap();
        let without_j = entries.iter().find(|e| e.title == "Article without journal").unwrap();
        assert_eq!(with_j.journal.as_deref(), Some("Nature"));
        assert_eq!(without_j.journal, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_includes_journal_from_extra_fields(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "Searchable Article".to_string(),
            entry_type: "article".to_string(),
            extra_fields: [("journal".to_string(), "Science".to_string())].into(),
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "Searchable", None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].journal.as_deref(), Some("Science"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entries_filtered_by_tag(pool: SqlitePool) {
        let tag_id = sqlx::query("INSERT INTO tags (name) VALUES ('ML')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let ml_entry = create_entry(&pool, &EntryInput {
            title: "ML Paper".to_string(),
            entry_type: "article".to_string(),
            tag_ids: vec![tag_id],
            ..Default::default()
        }).await.unwrap();

        create_entry(&pool, &EntryInput {
            title: "Other Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let entries = get_entries(&pool, None, Some(tag_id), None).await.unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, ml_entry.id);
    }

    // ── update_entry ─────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_changes_basic_fields(pool: SqlitePool) {
        let created = create_entry(&pool, &EntryInput {
            title: "Original Title".to_string(),
            entry_type: "article".to_string(),
            year: Some(2020),
            ..Default::default()
        }).await.unwrap();

        let updated = update_entry(&pool, created.id, &EntryInput {
            title: "Updated Title".to_string(),
            entry_type: "book".to_string(),
            year: Some(2024),
            doi: Some("10.9999/new".to_string()),
            ..Default::default()
        }).await.unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.title, "Updated Title");
        assert_eq!(updated.entry_type, "book");
        assert_eq!(updated.year, Some(2024));
        assert_eq!(updated.doi, Some("10.9999/new".to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_replaces_authors(pool: SqlitePool) {
        let created = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Old Author".to_string()],
            ..Default::default()
        }).await.unwrap();

        let updated = update_entry(&pool, created.id, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["New Author A".to_string(), "New Author B".to_string()],
            ..Default::default()
        }).await.unwrap();

        assert_eq!(updated.authors.len(), 2);
        assert_eq!(updated.authors[0].name, "New Author A");
        assert_eq!(updated.authors[1].name, "New Author B");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_replaces_tags(pool: SqlitePool) {
        let tag_a = sqlx::query("INSERT INTO tags (name) VALUES ('TagA')")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let tag_b = sqlx::query("INSERT INTO tags (name) VALUES ('TagB')")
            .execute(&pool).await.unwrap().last_insert_rowid();

        let created = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            tag_ids: vec![tag_a],
            ..Default::default()
        }).await.unwrap();

        let updated = update_entry(&pool, created.id, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            tag_ids: vec![tag_b],
            ..Default::default()
        }).await.unwrap();

        assert_eq!(updated.tags.len(), 1);
        assert_eq!(updated.tags[0].id, tag_b);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_replaces_extra_fields(pool: SqlitePool) {
        let created = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            extra_fields: [("journal".to_string(), "Nature".to_string())].into(),
            ..Default::default()
        }).await.unwrap();

        let updated = update_entry(&pool, created.id, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            extra_fields: [("volume".to_string(), "42".to_string())].into(),
            ..Default::default()
        }).await.unwrap();

        assert!(!updated.extra_fields.contains_key("journal"));
        assert_eq!(updated.extra_fields.get("volume").map(|s| s.as_str()), Some("42"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_not_found_returns_error(pool: SqlitePool) {
        let result = update_entry(&pool, 9999, &EntryInput {
            title: "X".to_string(),
            entry_type: "misc".to_string(),
            ..Default::default()
        }).await;

        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    // ── delete_entry ─────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_removes_entry(pool: SqlitePool) {
        let created = create_entry(&pool, &EntryInput {
            title: "To Be Deleted".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        delete_entry(&pool, created.id).await.unwrap();

        let result = get_entry(&pool, created.id).await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_not_found_returns_error(pool: SqlitePool) {
        let result = delete_entry(&pool, 9999).await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    // ── find_duplicate_entry ──────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_matches_doi_case_insensitive(pool: SqlitePool) {
        let existing = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/Example".to_string()),
            ..Default::default()
        }).await.unwrap();

        let hit = find_duplicate_entry(&pool, Some("10.1234/EXAMPLE"), None, None).await.unwrap();
        assert_eq!(hit, Some(existing.id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_matches_arxiv(pool: SqlitePool) {
        let existing = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();

        let hit = find_duplicate_entry(&pool, None, Some("2301.00001"), None).await.unwrap();
        assert_eq!(hit, Some(existing.id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_matches_isbn_ignoring_hyphens(pool: SqlitePool) {
        let existing = create_entry(&pool, &EntryInput {
            title: "Book".to_string(),
            entry_type: "book".to_string(),
            isbn: Some("978-0-387-31073-2".to_string()),
            ..Default::default()
        }).await.unwrap();

        let hit = find_duplicate_entry(&pool, None, None, Some("9780387310732")).await.unwrap();
        assert_eq!(hit, Some(existing.id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_excludes_trashed(pool: SqlitePool) {
        let existing = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/example".to_string()),
            ..Default::default()
        }).await.unwrap();
        trash_entry(&pool, existing.id).await.unwrap();

        let hit = find_duplicate_entry(&pool, Some("10.1234/example"), None, None).await.unwrap();
        assert_eq!(hit, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_returns_none_when_no_inputs(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/example".to_string()),
            ..Default::default()
        }).await.unwrap();

        let hit = find_duplicate_entry(&pool, None, None, None).await.unwrap();
        assert_eq!(hit, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_returns_none_when_no_match(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/example".to_string()),
            ..Default::default()
        }).await.unwrap();

        let hit = find_duplicate_entry(&pool, Some("10.9999/other"), None, None).await.unwrap();
        assert_eq!(hit, None);
    }

    // ── get_entry: relations ──────────────────────────────────────────────────

    async fn link_relation(
        pool: &SqlitePool,
        from_id: i64,
        to_id: i64,
        relation_type: &str,
    ) {
        sqlx::query(
            "INSERT INTO entry_relations (from_entry_id, to_entry_id, relation_type)
             VALUES (?, ?, ?)",
        )
        .bind(from_id)
        .bind(to_id)
        .bind(relation_type)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_loads_outbound_relation(pool: SqlitePool) {
        let preprint = create_entry(&pool, &EntryInput {
            title: "Preprint".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let published = create_entry(&pool, &EntryInput {
            title: "Published".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        link_relation(&pool, preprint.id, published.id, "preprint_of").await;

        let loaded = get_entry(&pool, preprint.id).await.unwrap();
        assert_eq!(loaded.relations.len(), 1);
        let rel = &loaded.relations[0];
        assert_eq!(rel.entry.id, published.id);
        assert_eq!(rel.relation_type, "preprint_of");
        assert_eq!(rel.direction, "from");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_loads_inbound_relation(pool: SqlitePool) {
        let preprint = create_entry(&pool, &EntryInput {
            title: "Preprint".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let published = create_entry(&pool, &EntryInput {
            title: "Published".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        link_relation(&pool, preprint.id, published.id, "preprint_of").await;

        let loaded = get_entry(&pool, published.id).await.unwrap();
        assert_eq!(loaded.relations.len(), 1);
        let rel = &loaded.relations[0];
        assert_eq!(rel.entry.id, preprint.id);
        assert_eq!(rel.relation_type, "preprint_of");
        assert_eq!(rel.direction, "to");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_excludes_trashed_related_entries(pool: SqlitePool) {
        let a = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let b = create_entry(&pool, &EntryInput {
            title: "B".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        link_relation(&pool, a.id, b.id, "cites").await;
        trash_entry(&pool, b.id).await.unwrap();

        let loaded = get_entry(&pool, a.id).await.unwrap();
        assert!(loaded.relations.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_cascades_relations(pool: SqlitePool) {
        let created = create_entry(&pool, &EntryInput {
            title: "Paper with extras".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Author".to_string()],
            extra_fields: [("journal".to_string(), "Science".to_string())].into(),
            ..Default::default()
        }).await.unwrap();

        delete_entry(&pool, created.id).await.unwrap();

        let author_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_authors WHERE entry_id = ?",
        )
        .bind(created.id)
        .fetch_one(&pool)
        .await
        .unwrap();

        let extra_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM extra_fields WHERE entry_id = ?",
        )
        .bind(created.id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(author_rows, 0);
        assert_eq!(extra_rows, 0);
    }

    // ── search_entries ───────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_matches_title(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "Attention Is All You Need".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "BERT Pretraining".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "attention", None, None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Attention Is All You Need");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_matches_author_name(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "Paper One".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Ashish Vaswani".to_string()],
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Paper Two".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Jane Doe".to_string()],
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "Vaswani", None, None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Paper One");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_matches_tag_name(pool: SqlitePool) {
        let tag_id = sqlx::query("INSERT INTO tags (name) VALUES ('machine-learning')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        create_entry(&pool, &EntryInput {
            title: "Tagged Paper".to_string(),
            entry_type: "article".to_string(),
            tag_ids: vec![tag_id],
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Untagged Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "machine", None, None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Tagged Paper");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_matches_doi(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "DOI Paper".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/example".to_string()),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Other Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "10.1234/example", None, None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "DOI Paper");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_matches_abstract(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "Paper A".to_string(),
            entry_type: "article".to_string(),
            abstract_: Some("We propose a novel transformer architecture".to_string()),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Paper B".to_string(),
            entry_type: "article".to_string(),
            abstract_: Some("A study on graph neural networks".to_string()),
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "transformer", None, None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Paper A");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_matches_japanese_substring(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "深層学習の最近の進展".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "古典的機械学習手法".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "深層", None, None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "深層学習の最近の進展");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_multiple_tokens_are_anded(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "Attention Is All You Need".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Ashish Vaswani".to_string()],
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "BERT Attention Heads".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Jacob Devlin".to_string()],
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "Attention Vaswani", None, None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "Attention Is All You Need");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_respects_collection_filter(pool: SqlitePool) {
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Reading List')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let inside = create_entry(&pool, &EntryInput {
            title: "Attention Paper Inside".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "Attention Paper Outside".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        sqlx::query("INSERT INTO entry_collections (entry_id, collection_id) VALUES (?, ?)")
            .bind(inside.id)
            .bind(col_id)
            .execute(&pool)
            .await
            .unwrap();

        let hits = search_entries(&pool, "attention", Some(col_id), None).await.unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, inside.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_empty_query_returns_all(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        create_entry(&pool, &EntryInput {
            title: "B".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let hits = search_entries(&pool, "   ", None, None).await.unwrap();

        assert_eq!(hits.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_reflects_updates(pool: SqlitePool) {
        let created = create_entry(&pool, &EntryInput {
            title: "Original".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        update_entry(&pool, created.id, &EntryInput {
            title: "Renamed Transformer Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let stale = search_entries(&pool, "original", None, None).await.unwrap();
        let fresh = search_entries(&pool, "transformer", None, None).await.unwrap();

        assert!(stale.is_empty(), "stale title should no longer match");
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].id, created.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_removes_after_delete(pool: SqlitePool) {
        let created = create_entry(&pool, &EntryInput {
            title: "Disposable Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        delete_entry(&pool, created.id).await.unwrap();

        let hits = search_entries(&pool, "Disposable", None, None).await.unwrap();
        assert!(hits.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entries_filtered_by_collection(pool: SqlitePool) {
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('My Collection')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let col_entry = create_entry(&pool, &EntryInput {
            title: "In Collection".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        sqlx::query("INSERT INTO entry_collections (entry_id, collection_id) VALUES (?, ?)")
            .bind(col_entry.id)
            .bind(col_id)
            .execute(&pool)
            .await
            .unwrap();

        create_entry(&pool, &EntryInput {
            title: "Not In Collection".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let entries = get_entries(&pool, Some(col_id), None, None).await.unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, col_entry.id);
    }
}
