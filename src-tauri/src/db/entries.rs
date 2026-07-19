use std::collections::HashMap;

use crate::db::authors::{attach_identifiers, link_entry_authors};
use crate::models::{
    Attachment, Author, Collection, EntryDetail, EntryFilter, EntryInput, EntryRelation,
    EntrySummary, SidebarCounts, Tag, TagMatch,
};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};

/// `EntryFilter` の各軸を ` AND ...` 句として QueryBuilder に追記する（v0.6.0）。
/// エントリのエイリアスは `e` を前提とする。空フィルタなら何も追記しない。
/// `get_entries_filtered` / 検索の両経路で共有する。
fn push_filter<'a>(qb: &mut QueryBuilder<'a, Sqlite>, filter: &'a EntryFilter) {
    if !filter.entry_types.is_empty() {
        qb.push(" AND e.entry_type IN (");
        let mut sep = qb.separated(", ");
        for t in &filter.entry_types {
            sep.push_bind(t);
        }
        sep.push_unseparated(")");
    }
    if let Some(y) = filter.year_min {
        qb.push(" AND e.year >= ").push_bind(y);
    }
    if let Some(y) = filter.year_max {
        qb.push(" AND e.year <= ").push_bind(y);
    }
    match filter.starred {
        Some(true) => { qb.push(" AND e.starred = 1"); }
        Some(false) => { qb.push(" AND e.starred = 0"); }
        None => {}
    }
    // 「添付」= PDF 添付（UI ラベル・CLI ヘルプとも「PDF 添付」）。arXiv TeX ソース
    // （application/gzip・LCIR Phase 4）は数えない — TeX ソースだけのエントリを
    // 「PDF あり」と偽らないため。
    match filter.has_attachment {
        Some(true) => {
            qb.push(
                " AND EXISTS (SELECT 1 FROM attachments a WHERE a.entry_id = e.id \
                 AND a.mime_type LIKE '%pdf%')",
            );
        }
        Some(false) => {
            qb.push(
                " AND NOT EXISTS (SELECT 1 FROM attachments a WHERE a.entry_id = e.id \
                 AND a.mime_type LIKE '%pdf%')",
            );
        }
        None => {}
    }
    if !filter.tag_ids.is_empty() {
        match filter.tag_match {
            TagMatch::Or => {
                qb.push(" AND e.id IN (SELECT entry_id FROM entry_tags WHERE tag_id IN (");
                let mut sep = qb.separated(", ");
                for tid in &filter.tag_ids {
                    sep.push_bind(tid);
                }
                sep.push_unseparated("))");
            }
            TagMatch::And => {
                // 各タグごとに存在を要求することで「すべて含む」を表現する。
                for tid in &filter.tag_ids {
                    qb.push(" AND e.id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ")
                        .push_bind(tid)
                        .push(")");
                }
            }
        }
    }
}

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
                 -- v0.3.0: 表記 (name) / 原語 (name_original) / 読み仮名 (reading_*) を
                 -- 同じセルに入れて、trigram tokenizer で「関」「せき」「Seki」のいずれにも
                 -- ヒットさせる。GROUP_CONCAT のセパレータはスペース。
                 SELECT GROUP_CONCAT(
                     TRIM(
                         COALESCE(a.name, '')              || ' ' ||
                         COALESCE(a.name_original, '')     || ' ' ||
                         COALESCE(a.reading_family, '')    || ' ' ||
                         COALESCE(a.reading_given, '')
                     ),
                     ' '
                 )
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

/// v0.3.0 アップグレード時の一括 FTS 再構築。
///
/// migration 0009 は authors 列の追加と author_identifiers のバックフィルしか行わず、
/// 既存 entry の `entries_fts.authors_text` は v0.2.x のときの古い形 (name のみ) のまま
/// 残っている。本関数は `settings.fts.authors_v030_rebuilt` フラグが未セットなら
/// 全 entry の FTS を 1 回だけ再構築し、完了後にフラグを立てる。2 回目以降は no-op。
///
/// 戻り値: 実際に再構築が走ったら `true`、フラグ既設で skip したら `false`。
///
/// 起動時に `lib.rs::run` の setup 内で呼ぶ。再構築は SELECT + 個別 sync を
/// 単一 tx でまとめる（数万件規模でも数秒で完了する想定）。失敗時はフラグを
/// 立てずに Err を返すので、次回起動でリトライされる。
pub async fn rebuild_authors_fts_once(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    use crate::db::settings;

    if settings::get_setting(pool, settings::FTS_AUTHORS_V030_REBUILT_KEY)
        .await?
        .is_some()
    {
        return Ok(false);
    }

    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM entries")
        .fetch_all(pool)
        .await?;

    let mut tx = pool.begin().await?;
    for id in ids {
        sync_entries_fts(&mut tx, id).await?;
    }
    tx.commit().await?;

    settings::set_setting(pool, settings::FTS_AUTHORS_V030_REBUILT_KEY, "1").await?;
    Ok(true)
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
    search_entries_filtered(pool, query, collection_id, tag_id, None, &EntryFilter::default()).await
}

/// `search_entries` に `EntryFilter`（v0.6.0）を AND 合成した版。FTS/LIKE のヒットを
/// さらにフィルタ条件で絞り込む。空クエリ時は `get_entries_filtered` にフォールバックする。
pub async fn search_entries_filtered(
    pool: &SqlitePool,
    query: &str,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
    filter: &EntryFilter,
) -> Result<Vec<EntrySummary>, sqlx::Error> {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return get_entries_filtered(pool, collection_id, tag_id, view, filter).await;
    }

    // trigram は 3 文字未満のクエリを正しく扱えないため、最短トークンが 3 文字未満の場合は
    // entries_fts の各列を LIKE でスキャンするフォールバックパスを使う。
    let ids = if tokens.iter().any(|t| t.chars().count() < 3) {
        search_ids_like(pool, &tokens, collection_id, tag_id, view, filter).await?
    } else {
        search_ids_fts(pool, &tokens, collection_id, tag_id, view, filter).await?
    };

    let mut summaries = Vec::new();
    for id in ids {
        summaries.push(load_summary(pool, id).await?);
    }
    Ok(summaries)
}

/// 検索経路の view スコープ（CR-001）。`view = "trash"` ならゴミ箱内、それ以外は現役のみ。
fn push_view_scope(qb: &mut QueryBuilder<Sqlite>, view: Option<&str>) {
    if matches!(view, Some("trash")) {
        qb.push(" AND e.deleted_at IS NOT NULL");
    } else {
        qb.push(" AND e.deleted_at IS NULL");
    }
}

async fn search_ids_fts(
    pool: &SqlitePool,
    tokens: &[&str],
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
    filter: &EntryFilter,
) -> Result<Vec<i64>, sqlx::Error> {
    let match_expr = build_fts_match_expr(tokens);
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT e.id FROM entries e JOIN entries_fts f ON f.rowid = e.id WHERE entries_fts MATCH ",
    );
    qb.push_bind(match_expr);
    push_view_scope(&mut qb, view);
    if let Some(cid) = collection_id {
        qb.push(" AND e.id IN (SELECT entry_id FROM entry_collections WHERE collection_id = ")
            .push_bind(cid)
            .push(")");
    }
    if let Some(tid) = tag_id {
        qb.push(" AND e.id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ")
            .push_bind(tid)
            .push(")");
    }
    push_filter(&mut qb, filter);
    qb.push(" ORDER BY bm25(entries_fts)");
    qb.build_query_scalar().fetch_all(pool).await
}

/// LIKE 検索用の部分一致パターン。`%` `_` `\` をエスケープしてリテラル扱いにする
/// （クエリ側で `ESCAPE '\'` を併記すること）。
pub(crate) fn like_pattern(token: &str) -> String {
    format!(
        "%{}%",
        token
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

async fn search_ids_like(
    pool: &SqlitePool,
    tokens: &[&str],
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
    filter: &EntryFilter,
) -> Result<Vec<i64>, sqlx::Error> {
    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT e.id FROM entries e JOIN entries_fts f ON f.rowid = e.id WHERE 1 = 1",
    );
    push_view_scope(&mut qb, view);
    for token in tokens {
        let pattern = like_pattern(token);
        qb.push(" AND (f.title LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\'");
        qb.push(" OR f.authors_text LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\'");
        qb.push(" OR f.tags_text LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\'");
        qb.push(" OR f.abstract_text LIKE ").push_bind(pattern.clone()).push(" ESCAPE '\\'");
        qb.push(" OR f.identifiers LIKE ").push_bind(pattern).push(" ESCAPE '\\')");
    }
    if let Some(cid) = collection_id {
        qb.push(" AND e.id IN (SELECT entry_id FROM entry_collections WHERE collection_id = ")
            .push_bind(cid)
            .push(")");
    }
    if let Some(tid) = tag_id {
        qb.push(" AND e.id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ")
            .push_bind(tid)
            .push(")");
    }
    push_filter(&mut qb, filter);
    qb.push(" ORDER BY e.created_at DESC");
    qb.build_query_scalar().fetch_all(pool).await
}

async fn load_summary(pool: &SqlitePool, id: i64) -> Result<EntrySummary, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, title, year, entry_type, created_at, starred FROM entries WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    let mut authors: Vec<Author> = sqlx::query_as(
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
    attach_identifiers(pool, &mut authors).await?;

    let tags: Vec<Tag> = sqlx::query_as(
        "SELECT t.id, t.name FROM tags t
         JOIN entry_tags et ON et.tag_id = t.id
         WHERE et.entry_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let has_attachment: bool = sqlx::query(
        "SELECT COUNT(*) as cnt FROM attachments WHERE entry_id = ? AND mime_type LIKE '%pdf%'",
    )
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
    // 全経路（UI 追加 / import / LLM / clipper）で識別子の重複を作らせない（CR-019）。
    // 同一 DOI/arXiv/ISBN の現役エントリが既にあれば新規作成せず既存を返す（clipper の
    // apply_clip と同じ冪等挙動）。ゴミ箱内は対象外なので、trash 済みと同一識別子の新規は作れる。
    if let Some(existing_id) = find_duplicate_entry(
        pool,
        input.doi.as_deref(),
        input.arxiv_id.as_deref(),
        input.isbn.as_deref(),
    )
    .await?
    {
        return get_entry(pool, existing_id).await;
    }

    let mut tx = pool.begin().await?;

    let citation_key = input.citation_key.as_deref().and_then(sanitize_citation_key);
    let doi_canonical = input.doi.as_deref().and_then(canonical_doi);
    let arxiv_canonical = input.arxiv_id.as_deref().and_then(canonical_arxiv);
    let isbn_canonical = input.isbn.as_deref().and_then(canonical_isbn);

    let result = sqlx::query(
        "INSERT INTO entries (title, year, entry_type, citation_key, doi, isbn, arxiv_id, url, abstract, notes,
                              doi_canonical, arxiv_canonical, isbn_canonical)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
    .bind(&doi_canonical)
    .bind(&arxiv_canonical)
    .bind(&isbn_canonical)
    .execute(&mut *tx)
    .await?;

    let entry_id = result.last_insert_rowid();

    link_entry_authors(&mut tx, entry_id, input).await?;

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

    let mut authors: Vec<Author> = sqlx::query_as(
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
    attach_identifiers(pool, &mut authors).await?;

    let tags: Vec<Tag> = sqlx::query_as(
        "SELECT t.id, t.name FROM tags t
         JOIN entry_tags et ON et.tag_id = t.id
         WHERE et.entry_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let has_attachment: bool = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM attachments WHERE entry_id = ? AND mime_type LIKE '%pdf%'",
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

    let mut authors: Vec<Author> = sqlx::query_as(
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
    attach_identifiers(pool, &mut authors).await?;

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

    let has_attachment = attachments
        .iter()
        .any(|a| a.mime_type.to_lowercase().contains("pdf"));

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

/// `get_entries_filtered` の無フィルタ版ショートカット。テスト・非フィルタ経路の利便のため残す。
#[cfg_attr(not(test), allow(dead_code))]
pub async fn get_entries(
    pool: &SqlitePool,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
) -> Result<Vec<EntrySummary>, sqlx::Error> {
    get_entries_filtered(pool, collection_id, tag_id, view, &EntryFilter::default()).await
}

/// scope（collection/tag/view）に加えて `EntryFilter`（v0.6.0 複合フィルタ）を AND 合成して
/// 一覧を返す。`filter` が空なら従来の `get_entries` と同じ結果になる。
pub async fn get_entries_filtered(
    pool: &SqlitePool,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
    filter: &EntryFilter,
) -> Result<Vec<EntrySummary>, sqlx::Error> {
    // view = "trash" 以外はゴミ箱（deleted_at IS NOT NULL）を除外する。
    let trash_view = matches!(view, Some("trash"));

    let mut qb: QueryBuilder<Sqlite> = QueryBuilder::new("SELECT e.id FROM entries e WHERE ");
    qb.push(if trash_view {
        "e.deleted_at IS NOT NULL"
    } else {
        "e.deleted_at IS NULL"
    });

    // scope: サイドバーの単一次元選択。tag_id > collection_id > 特殊 view の優先順で 1 つだけ効く。
    if let Some(tid) = tag_id {
        qb.push(" AND e.id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ")
            .push_bind(tid)
            .push(")");
    } else if let Some(cid) = collection_id {
        qb.push(" AND e.id IN (SELECT entry_id FROM entry_collections WHERE collection_id = ")
            .push_bind(cid)
            .push(")");
    } else {
        match view {
            Some("starred") => { qb.push(" AND e.starred = 1"); }
            Some("unfiled") => {
                qb.push(" AND NOT EXISTS (SELECT 1 FROM entry_collections ec WHERE ec.entry_id = e.id)");
            }
            _ => {}
        }
    }

    // filter: 複合フィルタ（各軸 AND）。
    push_filter(&mut qb, filter);

    qb.push(if trash_view {
        " ORDER BY e.deleted_at DESC"
    } else {
        " ORDER BY e.created_at DESC"
    });

    let ids: Vec<i64> = qb.build_query_scalar().fetch_all(pool).await?;

    let mut summaries = Vec::with_capacity(ids.len());
    for id in ids {
        summaries.push(load_entry_summary(pool, id).await?);
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
    // canonical 列も raw 識別子と同期させる（CR-019）。
    let doi_canonical = input.doi.as_deref().and_then(canonical_doi);
    let arxiv_canonical = input.arxiv_id.as_deref().and_then(canonical_arxiv);
    let isbn_canonical = input.isbn.as_deref().and_then(canonical_isbn);

    let rows_affected = sqlx::query(
        "UPDATE entries
         SET title = ?, year = ?, entry_type = ?, citation_key = ?, doi = ?, isbn = ?, arxiv_id = ?,
             url = ?, abstract = ?, notes = ?, updated_at = datetime('now'),
             doi_canonical = ?, arxiv_canonical = ?, isbn_canonical = ?
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
    .bind(&doi_canonical)
    .bind(&arxiv_canonical)
    .bind(&isbn_canonical)
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

    link_entry_authors(&mut tx, id, input).await?;

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

/// 与えた（通常はゴミ箱内の）エントリを untrash したとき、現役エントリと識別子が
/// 衝突する相手 id を返す（CR-019）。dedup は現役同士の識別子重複を作らせない不変条件を
/// 保つので、restore で現役に同一 DOI/arXiv/ISBN を復活させるのは禁止する。best-effort の
/// partial UNIQUE 索引が張られていれば DB でも弾かれるが、張れていない場合でも不変条件を
/// 守るため、事前にここで検出する。
async fn live_identifier_conflict<'e, E>(exec: E, id: i64) -> Result<Option<i64>, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query_scalar(
        "SELECT e2.id FROM entries e1
         JOIN entries e2
           ON e2.id != e1.id
          AND e2.deleted_at IS NULL
          AND (
                (e1.doi_canonical   IS NOT NULL AND e2.doi_canonical   = e1.doi_canonical)
             OR (e1.arxiv_canonical IS NOT NULL AND e2.arxiv_canonical = e1.arxiv_canonical)
             OR (e1.isbn_canonical  IS NOT NULL AND e2.isbn_canonical  = e1.isbn_canonical)
          )
         WHERE e1.id = ?
         ORDER BY e2.id ASC
         LIMIT 1",
    )
    .bind(id)
    .fetch_optional(exec)
    .await
}

fn restore_conflict_error(id: i64, conflict: i64) -> sqlx::Error {
    sqlx::Error::Protocol(format!(
        "restore aborted (id={id}): 現役エントリ (id={conflict}) と同一の識別子です。\
         先に一方を統合または削除してください（CR-019）。"
    ))
}

pub async fn bulk_restore(pool: &SqlitePool, ids: &[i64]) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for id in ids {
        // tx 内で 1 件ずつ復活させるので、直前に復活したバッチ内の行も現役として見え、
        // バッチ内の重複同士も検出できる。
        if let Some(conflict) = live_identifier_conflict(&mut *tx, *id).await? {
            return Err(restore_conflict_error(*id, conflict));
        }
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
///
/// **安全策（CR-001）**: ゴミ箱にある（`deleted_at IS NOT NULL`）エントリだけを消す。
/// 現役エントリの id が混ざっても hard delete しない。実際に消えた id を返すので、
/// 呼び出し側は添付ファイルの後始末をその id にだけ行える。
pub async fn bulk_purge(pool: &SqlitePool, ids: &[i64]) -> Result<Vec<i64>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut tx = pool.begin().await?;
    let mut purged = Vec::new();
    for id in ids {
        // ゴミ箱にある行だけを対象にする。現役エントリなら fts/fulltext も触らずスキップ。
        let is_trashed: Option<i64> =
            sqlx::query_scalar("SELECT id FROM entries WHERE id = ? AND deleted_at IS NOT NULL")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        if is_trashed.is_none() {
            continue;
        }
        purge_one(&mut tx, *id).await?;
        purged.push(*id);
    }
    tx.commit().await?;
    Ok(purged)
}

/// ゴミ箱を空にする。表示中の id ではなく DB 側で `deleted_at IS NOT NULL` を評価するため、
/// 検索やフィルタで現役エントリが混ざる余地がない（CR-001）。消えた id を返す。
pub async fn purge_trash(pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM entries WHERE deleted_at IS NOT NULL")
            .fetch_all(pool)
            .await?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut tx = pool.begin().await?;
    for id in &ids {
        purge_one(&mut tx, *id).await?;
    }
    tx.commit().await?;
    Ok(ids)
}

/// 1 エントリ分の hard delete（entries_fts / fulltext / entries）。呼び出し側で対象確定済み。
async fn purge_one(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM entries_fts WHERE rowid = ?")
        .bind(id)
        .execute(&mut **tx)
        .await?;
    sqlx::query(
        "DELETE FROM fulltext WHERE attachment_id IN (
            SELECT id FROM attachments WHERE entry_id = ?
        )",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    // LCIR ノード単位 FTS も attachments への FK が無いのでここで消す（fulltext と同型）。
    sqlx::query(
        "DELETE FROM document_nodes_fts WHERE attachment_id IN (
            SELECT id FROM attachments WHERE entry_id = ?
        )",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM entries WHERE id = ?")
        .bind(id)
        .execute(&mut **tx)
        .await?;
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

// ── 識別子の canonical 化（CR-019） ────────────────────────────────────────────
//
// DOI / arXiv / ISBN の正準値を **単一のソース**（この Rust 関数群）で定義する。
// `entries.{doi,arxiv_id,isbn}_canonical` 列への書込・重複判定・起動時 backfill の
// すべてがこれらを経由する。SQL 側で LOWER/REPLACE を書いて非対称に揃える旧方式
// （stored 側が arXiv の版番号・prefix を剥がさず dedup をすり抜けた）を廃する。

/// DOI の正準値。`https://doi.org/` `http://doi.org/` `doi:` の prefix を剥がし、
/// trim して小文字化する。空になれば `None`。
pub(crate) fn canonical_doi(s: &str) -> Option<String> {
    let t = s.trim();
    let t = t
        .strip_prefix("https://doi.org/")
        .or_else(|| t.strip_prefix("http://doi.org/"))
        .or_else(|| t.strip_prefix("https://dx.doi.org/"))
        .or_else(|| t.strip_prefix("http://dx.doi.org/"))
        .or_else(|| t.strip_prefix("doi:"))
        .or_else(|| t.strip_prefix("DOI:"))
        .unwrap_or(t)
        .trim();
    let t = t.to_lowercase();
    if t.is_empty() { None } else { Some(t) }
}

/// arXiv ID の正準値。[`crate::metadata::normalize_arxiv_id`]（prefix / 版番号を除去し
/// 旧形式カテゴリは保持）に委ね、小文字化する。空になれば `None`。
pub(crate) fn canonical_arxiv(s: &str) -> Option<String> {
    let t = crate::metadata::normalize_arxiv_id(s).to_lowercase();
    if t.is_empty() { None } else { Some(t) }
}

/// ISBN の正準値。[`normalize_isbn`]（英数字のみ・大文字化）に委ね、空なら `None`。
pub(crate) fn canonical_isbn(s: &str) -> Option<String> {
    let t = normalize_isbn(s);
    if t.is_empty() { None } else { Some(t) }
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
    // 入力・stored の双方を Rust の canonical_*() で揃えて比較する（CR-019）。
    // stored 側は entries.{doi,arxiv,isbn}_canonical 列（書込時と起動時 backfill で
    // 常に canonical に保たれる）を直接引くので、旧方式の非対称性が無くなる。
    let doi_norm = doi.and_then(canonical_doi);
    let arxiv_norm = arxiv_id.and_then(canonical_arxiv);
    let isbn_norm = isbn.and_then(canonical_isbn);

    if doi_norm.is_none() && arxiv_norm.is_none() && isbn_norm.is_none() {
        return Ok(None);
    }

    // 値が None の引数は NULL を bind し、SQL 側で `? IS NOT NULL AND ...` で弾く。
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM entries
         WHERE deleted_at IS NULL
           AND (
                (?1 IS NOT NULL AND doi_canonical   = ?1)
             OR (?2 IS NOT NULL AND arxiv_canonical = ?2)
             OR (?3 IS NOT NULL AND isbn_canonical  = ?3)
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

/// canonical 列（`doi/arxiv/isbn_canonical`）を持つ 3 つの識別子。列名・非UNIQUE索引名・
/// best-effort UNIQUE 索引名を 1 か所で対応づける。
const CANONICAL_IDENTIFIERS: &[(&str, &str)] = &[
    ("doi", "doi_canonical"),
    ("arxiv", "arxiv_canonical"),
    ("isbn", "isbn_canonical"),
];

/// 既存行の canonical 列を Rust の canonical_*() で埋める（CR-019・migration 0013 の後段）。
///
/// migration は arXiv の版番号除去などを SQL で表現できないため列を足すだけにしてあり、
/// 実際の backfill はここで行う。canonical が未設定（NULL）で raw 識別子がある行だけを
/// 対象にするので、埋め終われば以降の起動では対象 0 件の no-op になり冪等。
/// 更新した行数を返す。
pub async fn backfill_canonical_identifiers(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    // (id, doi, arxiv_id, isbn) の生識別子行。
    type RawIdentifierRow = (i64, Option<String>, Option<String>, Option<String>);
    // backfill が必要な行（raw があるのに canonical が NULL）だけを引く。
    let rows: Vec<RawIdentifierRow> = sqlx::query_as(
        "SELECT id, doi, arxiv_id, isbn FROM entries
         WHERE (doi      IS NOT NULL AND doi_canonical   IS NULL)
            OR (arxiv_id IS NOT NULL AND arxiv_canonical IS NULL)
            OR (isbn     IS NOT NULL AND isbn_canonical  IS NULL)",
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    let mut updated = 0u64;
    for (id, doi, arxiv, isbn) in rows {
        let doi_c = doi.as_deref().and_then(canonical_doi);
        let arxiv_c = arxiv.as_deref().and_then(canonical_arxiv);
        let isbn_c = isbn.as_deref().and_then(canonical_isbn);
        let n = sqlx::query(
            "UPDATE entries SET doi_canonical = ?, arxiv_canonical = ?, isbn_canonical = ? WHERE id = ?",
        )
        .bind(&doi_c)
        .bind(&arxiv_c)
        .bind(&isbn_c)
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        updated += n;
    }
    tx.commit().await?;
    Ok(updated)
}

/// best-effort で識別子の partial UNIQUE 索引を張る（CR-019）。
///
/// 既存 DB に重複があると `CREATE UNIQUE INDEX` は失敗する。migration 内で張ると
/// 起動不能（brick）になるため、起動時にここで **重複が無い識別子だけ** UNIQUE を張る。
/// 重複が残る識別子は索引を張らず（既存の非UNIQUE索引のまま）、警告としてスキップ扱い。
///
/// 部分索引の条件は `deleted_at IS NULL`（= dedup が対象とする現役エントリのみ）に揃える。
/// これによりゴミ箱内の重複や trash↔現役の共存では制約が働かず、restore 側の衝突ガード
/// （[`restore_entry`]）と合わせて一貫する。作成できた canonical 列名の一覧を返す。
pub async fn try_create_identifier_unique_indexes(
    pool: &SqlitePool,
) -> Result<Vec<String>, sqlx::Error> {
    let mut created = Vec::new();
    for (short, col) in CANONICAL_IDENTIFIERS {
        // 現役エントリの中に同一 canonical 値が 2 件以上あるか。
        let has_dup: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(
                 SELECT 1 FROM entries
                 WHERE deleted_at IS NULL AND {col} IS NOT NULL
                 GROUP BY {col} HAVING COUNT(*) > 1
             )"
        ))
        .fetch_one(pool)
        .await?;

        if has_dup {
            eprintln!(
                "CR-019: {col} に現役の重複があるため UNIQUE 索引をスキップ（非UNIQUE索引のまま）"
            );
            continue;
        }

        sqlx::query(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_{short}_canonical_unique
                 ON entries({col}) WHERE {col} IS NOT NULL AND deleted_at IS NULL"
        ))
        .execute(pool)
        .await?;
        created.push((*col).to_string());
    }
    Ok(created)
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
    // 現役エントリと識別子が衝突するなら復活させない（CR-019）。
    if let Some(conflict) = live_identifier_conflict(pool, id).await? {
        return Err(restore_conflict_error(id, conflict));
    }

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

    // fulltext は attachments への FK が無く cascade では消えないため、ここで消す。
    // 呼び出し元（Tauri コマンド / チャットツール / MCP）すべてで漏れなく効く。
    sqlx::query(
        "DELETE FROM fulltext WHERE attachment_id IN (
            SELECT id FROM attachments WHERE entry_id = ?
        )",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    // LCIR ノード単位 FTS も同様に FK 無しなのでここで消す。
    sqlx::query(
        "DELETE FROM document_nodes_fts WHERE attachment_id IN (
            SELECT id FROM attachments WHERE entry_id = ?
        )",
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntryInput;

    #[sqlx::test(migrations = "./migrations")]
    async fn like_fallback_treats_wildcards_literally(pool: SqlitePool) {
        // 短いトークンは LIKE フォールバックに乗る。`_`/`%` をワイルドカードとして
        // 解釈すると無関係なエントリがヒットするため、リテラル扱いを確認する。
        for title in ["x_y notation", "xay notation"] {
            create_entry(&pool, &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            }).await.unwrap();
        }

        let hits = search_entries(&pool, "x_", None, None).await.unwrap();
        assert_eq!(hits.len(), 1, "`_` はリテラルとして扱う");
        assert_eq!(hits[0].title, "x_y notation");
    }

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

    /// CR-006: `author_ids` は既存著者を ID で直接リンクし、順序を保持する
    /// （従来は完全に無視されていた）。存在しない ID は黙って除外する。
    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_honors_author_ids(pool: SqlitePool) {
        // 先に 2 名を作成して ID を得る。
        let seed = create_entry(&pool, &EntryInput {
            title: "Seed".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Alice Smith".to_string(), "Bob Jones".to_string()],
            ..Default::default()
        }).await.unwrap();
        let alice = seed.authors[0].id;
        let bob = seed.authors[1].id;

        // author_ids で逆順にリンク（+ 存在しない ID 9999 は無視される）。
        let entry = create_entry(&pool, &EntryInput {
            title: "By IDs".to_string(),
            entry_type: "article".to_string(),
            author_ids: vec![bob, 9999, alice],
            ..Default::default()
        }).await.unwrap();

        assert_eq!(entry.authors.len(), 2, "存在しない ID は除外される");
        assert_eq!(entry.authors[0].id, bob, "author_ids の順序を保持する");
        assert_eq!(entry.authors[1].id, alice);
        // 新規著者を作っていないこと（既存を再利用）。
        assert_eq!(entry.authors[0].name, "Bob Jones");
    }

    /// CR-006: 一覧/詳細の `Author.identifiers` に構造化 identifier が反映される
    /// （従来は常に空だった）。
    #[sqlx::test(migrations = "./migrations")]
    async fn entry_loaders_populate_author_identifiers(pool: SqlitePool) {
        use crate::models::{AuthorIdentifierInput, AuthorInput};
        let created = create_entry(&pool, &EntryInput {
            title: "With identifiers".to_string(),
            entry_type: "article".to_string(),
            authors: Some(vec![AuthorInput {
                name: "Grace Hopper".to_string(),
                orcid: Some("0000-0002-1825-0097".to_string()),
                identifiers: vec![AuthorIdentifierInput {
                    scheme: "scopus".to_string(),
                    value: "12345".to_string(),
                    url: None,
                }],
                ..Default::default()
            }]),
            ..Default::default()
        }).await.unwrap();

        // detail ローダー（create_entry の戻り = get_entry）
        let ids: Vec<&str> = created.authors[0].identifiers.iter()
            .map(|i| i.scheme.as_str()).collect();
        assert!(ids.contains(&"scopus"), "detail に scopus が載る: {ids:?}");
        assert!(ids.contains(&"orcid"), "orcid も author_identifiers に併記される");

        // list ローダー（get_entries）
        let list = get_entries(&pool, None, None, None).await.unwrap();
        let row = list.iter().find(|e| e.id == created.id).unwrap();
        assert!(
            row.authors[0].identifiers.iter().any(|i| i.scheme == "scopus"),
            "一覧サマリーにも identifiers が載る",
        );
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
        // purge はゴミ箱内だけを消す（CR-001）。先に trash へ入れる。
        bulk_trash(&pool, &[a, b]).await.unwrap();

        let purged = bulk_purge(&pool, &[a, b]).await.unwrap();
        assert_eq!(purged.len(), 2);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
        let fts_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries_fts")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(fts_count, 0);
    }

    /// CR-001: 現役（未 trash）エントリの id を bulk_purge に渡しても hard delete しない。
    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_purge_skips_live_entries(pool: SqlitePool) {
        let live = make_entry(&pool, "Live paper").await;
        let trashed = make_entry(&pool, "Trashed paper").await;
        bulk_trash(&pool, &[trashed]).await.unwrap();

        // 現役 live と trashed をまとめて渡しても、消えるのは trashed だけ。
        let purged = bulk_purge(&pool, &[live, trashed]).await.unwrap();
        assert_eq!(purged, vec![trashed]);

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE id = ?")
            .bind(live)
            .fetch_one(&pool).await.unwrap();
        assert_eq!(remaining, 1, "現役エントリは残っていること");
    }

    /// CR-001: purge_trash はゴミ箱内だけを消し、現役はすべて残す。
    #[sqlx::test(migrations = "./migrations")]
    async fn purge_trash_only_removes_trashed(pool: SqlitePool) {
        let live = make_entry(&pool, "Live").await;
        let t1 = make_entry(&pool, "Trash 1").await;
        let t2 = make_entry(&pool, "Trash 2").await;
        bulk_trash(&pool, &[t1, t2]).await.unwrap();

        let mut purged = purge_trash(&pool).await.unwrap();
        purged.sort();
        let mut expected = vec![t1, t2];
        expected.sort();
        assert_eq!(purged, expected);

        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM entries")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(ids, vec![live]);
    }

    /// CR-001: trash ビューでの検索はゴミ箱内エントリを返し、現役検索はゴミ箱を除外する。
    #[sqlx::test(migrations = "./migrations")]
    async fn search_respects_trash_view(pool: SqlitePool) {
        let live = make_entry(&pool, "transformer live").await;
        let trashed = make_entry(&pool, "transformer trashed").await;
        bulk_trash(&pool, &[trashed]).await.unwrap();
        let f = EntryFilter::default();

        // 通常（view=None）は現役のみ。
        let live_hits =
            search_entries_filtered(&pool, "transformer", None, None, None, &f).await.unwrap();
        assert_eq!(live_hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![live]);

        // trash ビューはゴミ箱内のみ。
        let trash_hits =
            search_entries_filtered(&pool, "transformer", None, None, Some("trash"), &f).await.unwrap();
        assert_eq!(trash_hits.iter().map(|h| h.id).collect::<Vec<_>>(), vec![trashed]);
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

    // ── canonical_* helpers（CR-019） ─────────────────────────────────────────

    #[test]
    fn canonical_doi_strips_url_prefix_and_lowercases() {
        assert_eq!(canonical_doi("10.1234/Example").as_deref(), Some("10.1234/example"));
        assert_eq!(canonical_doi("https://doi.org/10.1234/Example").as_deref(), Some("10.1234/example"));
        assert_eq!(canonical_doi("http://dx.doi.org/10.1234/EX").as_deref(), Some("10.1234/ex"));
        assert_eq!(canonical_doi(" doi:10.1234/EX ").as_deref(), Some("10.1234/ex"));
        assert_eq!(canonical_doi("  "), None);
    }

    #[test]
    fn canonical_arxiv_strips_version_prefix_and_lowercases() {
        assert_eq!(canonical_arxiv("arXiv:2301.00001v5").as_deref(), Some("2301.00001"));
        assert_eq!(canonical_arxiv("hep-th/9901001v2").as_deref(), Some("hep-th/9901001"));
        assert_eq!(canonical_arxiv(""), None);
    }

    #[test]
    fn canonical_isbn_strips_hyphens_and_uppercases() {
        assert_eq!(canonical_isbn("978-0-387-31073-2").as_deref(), Some("9780387310732"));
        assert_eq!(canonical_isbn("0-306-40615-x").as_deref(), Some("030640615X"));
        assert_eq!(canonical_isbn(" - - "), None);
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

    /// CR-019: 版番号付き / prefix 付きの arXiv クエリでも同一エントリにヒットする。
    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_matches_arxiv_ignoring_version_and_prefix(pool: SqlitePool) {
        let existing = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();

        for q in ["2301.00001v3", "arXiv:2301.00001", "arxiv:2301.00001v9"] {
            let hit = find_duplicate_entry(&pool, None, Some(q), None).await.unwrap();
            assert_eq!(hit, Some(existing.id), "query {q} should match");
        }
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

    /// CR-019: stored 側が非正準（版番号付き arXiv）でも canonical 列経由でヒットする。
    #[sqlx::test(migrations = "./migrations")]
    async fn find_duplicate_entry_matches_stored_noncanonical_arxiv(pool: SqlitePool) {
        let existing = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("arXiv:2301.00001v2".to_string()),
            ..Default::default()
        }).await.unwrap();

        let hit = find_duplicate_entry(&pool, None, Some("2301.00001"), None).await.unwrap();
        assert_eq!(hit, Some(existing.id));
    }

    /// CR-019: create_entry は全経路で dedup する。同一 DOI の再作成は既存を返し、行を増やさない。
    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_dedups_by_doi_and_returns_existing(pool: SqlitePool) {
        let first = create_entry(&pool, &EntryInput {
            title: "First".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/Example".to_string()),
            ..Default::default()
        }).await.unwrap();

        // 表記違い（大小・URL prefix）でも同一とみなし、既存 id を返す。
        let second = create_entry(&pool, &EntryInput {
            title: "Second (should not be created)".to_string(),
            entry_type: "article".to_string(),
            doi: Some("https://doi.org/10.1234/EXAMPLE".to_string()),
            ..Default::default()
        }).await.unwrap();

        assert_eq!(second.id, first.id, "既存エントリを返すべき");
        assert_eq!(second.title, "First", "新規作成せず既存の内容を返す");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1, "行は増えない");
    }

    /// CR-019: canonical 列は作成時に埋まる。
    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_populates_canonical_columns(pool: SqlitePool) {
        let e = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/AbC".to_string()),
            arxiv_id: Some("arXiv:2301.00001v3".to_string()),
            isbn: Some("978-0-387-31073-2".to_string()),
            ..Default::default()
        }).await.unwrap();

        let (doi_c, arxiv_c, isbn_c): (Option<String>, Option<String>, Option<String>) =
            sqlx::query_as("SELECT doi_canonical, arxiv_canonical, isbn_canonical FROM entries WHERE id = ?")
                .bind(e.id).fetch_one(&pool).await.unwrap();
        assert_eq!(doi_c.as_deref(), Some("10.1234/abc"));
        assert_eq!(arxiv_c.as_deref(), Some("2301.00001"));
        assert_eq!(isbn_c.as_deref(), Some("9780387310732"));
    }

    /// CR-019: update_entry で識別子を変えると canonical 列も追随する。
    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_syncs_canonical_columns(pool: SqlitePool) {
        let e = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/old".to_string()),
            ..Default::default()
        }).await.unwrap();

        update_entry(&pool, e.id, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            doi: Some("https://doi.org/10.9999/NEW".to_string()),
            ..Default::default()
        }).await.unwrap();

        let doi_c: Option<String> =
            sqlx::query_scalar("SELECT doi_canonical FROM entries WHERE id = ?")
                .bind(e.id).fetch_one(&pool).await.unwrap();
        assert_eq!(doi_c.as_deref(), Some("10.9999/new"));
    }

    /// CR-019: ゴミ箱内の同一識別子とは重複扱いしないので、新規作成できる。
    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_allows_duplicate_of_trashed(pool: SqlitePool) {
        let first = create_entry(&pool, &EntryInput {
            title: "First".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/example".to_string()),
            ..Default::default()
        }).await.unwrap();
        trash_entry(&pool, first.id).await.unwrap();

        let second = create_entry(&pool, &EntryInput {
            title: "Second".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/example".to_string()),
            ..Default::default()
        }).await.unwrap();

        assert_ne!(second.id, first.id, "ゴミ箱内とは重複扱いしないので新規作成される");
    }

    /// 生の identifier を直接 INSERT する（legacy 行や重複を意図的に作るテスト用。
    /// create_entry は dedup してしまうので使えない）。canonical 列は指定値で埋める。
    async fn insert_raw_entry(
        pool: &SqlitePool,
        title: &str,
        arxiv_id: Option<&str>,
        arxiv_canonical: Option<&str>,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO entries (title, entry_type, arxiv_id, arxiv_canonical)
             VALUES (?, 'article', ?, ?)",
        )
        .bind(title)
        .bind(arxiv_id)
        .bind(arxiv_canonical)
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    // ── backfill_canonical_identifiers / try_create_identifier_unique_indexes（CR-019） ──

    #[sqlx::test(migrations = "./migrations")]
    async fn backfill_fills_canonical_and_is_idempotent(pool: SqlitePool) {
        // canonical 未設定の legacy 行を用意する。
        let id = insert_raw_entry(&pool, "A", Some("arXiv:2301.00001v7"), None).await;

        let n = backfill_canonical_identifiers(&pool).await.unwrap();
        assert_eq!(n, 1);
        let c: Option<String> =
            sqlx::query_scalar("SELECT arxiv_canonical FROM entries WHERE id = ?")
                .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(c.as_deref(), Some("2301.00001"));

        // 2 回目は対象 0 件。
        let n2 = backfill_canonical_identifiers(&pool).await.unwrap();
        assert_eq!(n2, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unique_index_created_on_clean_db_blocks_direct_duplicate(pool: SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();

        let created = try_create_identifier_unique_indexes(&pool).await.unwrap();
        assert!(created.contains(&"arxiv_canonical".to_string()));

        // dedup を迂回して直接同一 canonical を入れると UNIQUE 制約で弾かれる。
        let err = sqlx::query(
            "INSERT INTO entries (title, entry_type, arxiv_id, arxiv_canonical)
             VALUES ('dup', 'article', '2301.00001', '2301.00001')",
        )
        .execute(&pool)
        .await;
        assert!(err.is_err(), "partial UNIQUE が二重登録を弾くべき");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unique_index_skipped_when_existing_duplicate(pool: SqlitePool) {
        // 現役の重複を 2 件（直接 INSERT で作る）。
        insert_raw_entry(&pool, "A", Some("2301.00001"), Some("2301.00001")).await;
        insert_raw_entry(&pool, "B", Some("2301.00001"), Some("2301.00001")).await;

        let created = try_create_identifier_unique_indexes(&pool).await.unwrap();
        assert!(!created.contains(&"arxiv_canonical".to_string()), "重複ありでは張らない");

        // 索引が無いので 3 件目の直接 INSERT も通る（brick せず既存も壊さない）。
        let ok = sqlx::query(
            "INSERT INTO entries (title, entry_type, arxiv_id, arxiv_canonical)
             VALUES ('C', 'article', '2301.00001', '2301.00001')",
        )
        .execute(&pool)
        .await;
        assert!(ok.is_ok());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unique_index_allows_trashed_and_live_same_identifier(pool: SqlitePool) {
        let first = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();
        trash_entry(&pool, first.id).await.unwrap();

        // 索引を張る（現役は 0 件なので張れる）。
        try_create_identifier_unique_indexes(&pool).await.unwrap();

        // ゴミ箱の同一識別子があっても現役を新規作成できる（部分索引が deleted_at IS NULL 限定）。
        let second = create_entry(&pool, &EntryInput {
            title: "B".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();
        assert_ne!(second.id, first.id);
    }

    /// CR-019: ゴミ箱の同一識別子を現役へ restore すると、現役の相手と衝突するので拒否する。
    #[sqlx::test(migrations = "./migrations")]
    async fn restore_entry_rejected_on_live_identifier_conflict(pool: SqlitePool) {
        // A を作って trash → B（同一 arXiv）を現役で作成（trash 中は dedup 対象外なので作れる）。
        let a = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();
        trash_entry(&pool, a.id).await.unwrap();
        let _b = create_entry(&pool, &EntryInput {
            title: "B".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();

        // A を restore すると B と衝突するのでエラー。
        let err = restore_entry(&pool, a.id).await.unwrap_err();
        assert!(err.to_string().contains("識別子"), "明示的な衝突メッセージ: {err}");

        // A は依然ゴミ箱のまま。
        let still_trashed: Option<i64> =
            sqlx::query_scalar("SELECT id FROM entries WHERE id = ? AND deleted_at IS NOT NULL")
                .bind(a.id).fetch_optional(&pool).await.unwrap();
        assert_eq!(still_trashed, Some(a.id));
    }

    /// CR-019: 衝突相手がいなければ restore は通る。
    #[sqlx::test(migrations = "./migrations")]
    async fn restore_entry_succeeds_without_conflict(pool: SqlitePool) {
        let a = create_entry(&pool, &EntryInput {
            title: "A".to_string(),
            entry_type: "article".to_string(),
            arxiv_id: Some("2301.00001".to_string()),
            ..Default::default()
        }).await.unwrap();
        trash_entry(&pool, a.id).await.unwrap();

        restore_entry(&pool, a.id).await.unwrap();
        let live: Option<i64> =
            sqlx::query_scalar("SELECT id FROM entries WHERE id = ? AND deleted_at IS NULL")
                .bind(a.id).fetch_optional(&pool).await.unwrap();
        assert_eq!(live, Some(a.id));
    }

    /// CR-019: バッチ復活で同一識別子が 2 件含まれると衝突として弾く。
    #[sqlx::test(migrations = "./migrations")]
    async fn bulk_restore_rejects_intra_batch_identifier_conflict(pool: SqlitePool) {
        // 直接 INSERT で「両方 trash 済み・同一 canonical」を作る。
        let a = sqlx::query("INSERT INTO entries (title, entry_type, arxiv_id, arxiv_canonical, deleted_at)
                             VALUES ('A','article','2301.00001','2301.00001', datetime('now'))")
            .execute(&pool).await.unwrap().last_insert_rowid();
        let b = sqlx::query("INSERT INTO entries (title, entry_type, arxiv_id, arxiv_canonical, deleted_at)
                             VALUES ('B','article','2301.00001','2301.00001', datetime('now'))")
            .execute(&pool).await.unwrap().last_insert_rowid();

        let err = bulk_restore(&pool, &[a, b]).await.unwrap_err();
        assert!(err.to_string().contains("識別子"), "{err}");

        // tx はロールバックされ、両方ともゴミ箱のまま。
        let live: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE deleted_at IS NULL")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(live, 0);
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

    // ── v0.3.0 §8.4 FTS ────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn entries_fts_includes_original_name_and_reading(pool: SqlitePool) {
        // 「Seki」名義で entry を作る（この時点で authors_text には "Seki" のみ）
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Some unrelated title".to_string(),
                entry_type: "article".to_string(),
                author_names: vec!["Seki".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // 後から漢字名と読み仮名を author に付与（M7 update_author 経由を想定したシナリオ）
        sqlx::query(
            "UPDATE authors
             SET name_original = '関 元樹',
                 family_name_original = '関',
                 given_name_original = '元樹',
                 original_script = 'Hani',
                 reading_family = 'せき',
                 reading_given = 'もとき',
                 updated_at = datetime('now')
             WHERE name = 'Seki'",
        )
        .execute(&pool)
        .await
        .unwrap();

        // v0.3.0 のワンショット再構築で entries_fts.authors_text に反映される
        let rebuilt = rebuild_authors_fts_once(&pool).await.unwrap();
        assert!(rebuilt, "未実行フラグなら再構築が走るべき");

        // 「関」（1 文字 CJK / LIKE フォールバックパス）
        let hits_kanji = search_entries(&pool, "関", None, None).await.unwrap();
        assert_eq!(hits_kanji.len(), 1, "漢字 '関' でヒットすべき");
        assert_eq!(hits_kanji[0].id, entry.id);

        // 「せき」（2 文字かな / LIKE フォールバック）
        let hits_kana = search_entries(&pool, "せき", None, None).await.unwrap();
        assert_eq!(hits_kana.len(), 1, "読み仮名 'せき' でヒットすべき");
        assert_eq!(hits_kana[0].id, entry.id);

        // 「Seki」（4 文字 ASCII / trigram FTS）
        let hits_ascii = search_entries(&pool, "Seki", None, None).await.unwrap();
        assert_eq!(hits_ascii.len(), 1, "ローマ字 'Seki' でヒットすべき");
        assert_eq!(hits_ascii[0].id, entry.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rebuild_authors_fts_once_is_idempotent(pool: SqlitePool) {
        // 初回は再構築が走る
        let first = rebuild_authors_fts_once(&pool).await.unwrap();
        assert!(first, "初回は再構築が走るべき");

        // フラグが立つ
        let flag = crate::db::settings::get_setting(
            &pool,
            crate::db::settings::FTS_AUTHORS_V030_REBUILT_KEY,
        )
        .await
        .unwrap();
        assert_eq!(flag.as_deref(), Some("1"));

        // 2 回目は no-op
        let second = rebuild_authors_fts_once(&pool).await.unwrap();
        assert!(!second, "フラグ既設なら skip すべき");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn new_entry_creation_already_indexes_with_v030_composition(pool: SqlitePool) {
        // M4 で sync_entries_fts の SQL 自体を新形に変更したので、新規 create_entry の時点で
        // authors_text に name_original / reading_* が含まれていれば rebuild なしでもヒットする。
        // 先に著者を多言語フィールド付きで作っておく必要があるが、AuthorInput 経由は
        // EntryInput からは届かないため、author を先に直接 INSERT して紐付ける。
        sqlx::query(
            "INSERT INTO authors (name, name_original, reading_family, updated_at)
             VALUES ('Yamada', '山田', 'やまだ', datetime('now'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        let author_id: i64 = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'Yamada'")
            .fetch_one(&pool)
            .await
            .unwrap();

        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Title".to_string(),
                entry_type: "article".to_string(),
                // author_names に "Yamada" を渡すと get_or_create_author が既存著者を name 完全一致で見つける
                author_names: vec!["Yamada".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // 同じ著者 id が紐付いていること
        let linked: i64 = sqlx::query_scalar(
            "SELECT author_id FROM entry_authors WHERE entry_id = ?",
        )
        .bind(entry.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(linked, author_id);

        // rebuild なしで漢字 / かな / ローマ字いずれもヒット
        for q in ["山田", "やまだ", "Yamada"] {
            let hits = search_entries(&pool, q, None, None).await.unwrap();
            assert_eq!(hits.len(), 1, "{q} should hit without rebuild");
            assert_eq!(hits[0].id, entry.id);
        }
    }

    // ── get_entries_filtered（v0.6.0 複合フィルタ）─────────────────────────────
    use crate::models::{EntryFilter, TagMatch};

    async fn make_entry_full(
        pool: &SqlitePool,
        title: &str,
        entry_type: &str,
        year: Option<i64>,
        starred: bool,
    ) -> i64 {
        let e = create_entry(pool, &EntryInput {
            title: title.to_string(),
            entry_type: entry_type.to_string(),
            year,
            ..Default::default()
        }).await.unwrap();
        if starred {
            set_starred(pool, e.id, true).await.unwrap();
        }
        e.id
    }

    async fn attach_dummy(pool: &SqlitePool, entry_id: i64) {
        sqlx::query(
            "INSERT INTO attachments (entry_id, file_name, file_path, mime_type)
             VALUES (?, 'x.pdf', '/tmp/x.pdf', 'application/pdf')",
        )
        .bind(entry_id)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_filter_matches_get_entries(pool: SqlitePool) {
        make_entry_full(&pool, "A", "article", Some(2020), false).await;
        make_entry_full(&pool, "B", "book", Some(2010), true).await;
        let all = get_entries_filtered(&pool, None, None, None, &EntryFilter::default())
            .await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_by_entry_type(pool: SqlitePool) {
        make_entry_full(&pool, "A", "article", None, false).await;
        make_entry_full(&pool, "B", "book", None, false).await;
        make_entry_full(&pool, "C", "article", None, false).await;

        let f = EntryFilter { entry_types: vec!["article".into()], ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &f).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|e| e.entry_type == "article"));

        // 複数種別は OR
        let f2 = EntryFilter { entry_types: vec!["article".into(), "book".into()], ..Default::default() };
        assert_eq!(get_entries_filtered(&pool, None, None, None, &f2).await.unwrap().len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_by_year_range(pool: SqlitePool) {
        make_entry_full(&pool, "old", "article", Some(1999), false).await;
        make_entry_full(&pool, "mid", "article", Some(2010), false).await;
        make_entry_full(&pool, "new", "article", Some(2022), false).await;
        make_entry_full(&pool, "noyear", "article", None, false).await;

        let min = EntryFilter { year_min: Some(2010), ..Default::default() };
        assert_eq!(get_entries_filtered(&pool, None, None, None, &min).await.unwrap().len(), 2);

        let max = EntryFilter { year_max: Some(2010), ..Default::default() };
        assert_eq!(get_entries_filtered(&pool, None, None, None, &max).await.unwrap().len(), 2);

        let range = EntryFilter { year_min: Some(2000), year_max: Some(2015), ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &range).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "mid");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_by_starred(pool: SqlitePool) {
        make_entry_full(&pool, "s", "article", None, true).await;
        make_entry_full(&pool, "n", "article", None, false).await;

        let on = EntryFilter { starred: Some(true), ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &on).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "s");

        let off = EntryFilter { starred: Some(false), ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &off).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "n");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_by_has_attachment(pool: SqlitePool) {
        let with = make_entry_full(&pool, "with", "article", None, false).await;
        make_entry_full(&pool, "without", "article", None, false).await;
        attach_dummy(&pool, with).await;

        let yes = EntryFilter { has_attachment: Some(true), ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &yes).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "with");

        let no = EntryFilter { has_attachment: Some(false), ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &no).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "without");
    }

    /// Phase 4: 「添付」フィルタは PDF 添付の意味 — arXiv TeX ソース（gzip）だけの
    /// エントリは「添付あり」に数えない（UI/CLI の「PDF 添付」ラベルを偽らない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn filter_has_attachment_ignores_tex_sources(pool: SqlitePool) {
        let tex_only = make_entry_full(&pool, "tex-only", "article", None, false).await;
        crate::db::attachments::add_attachment(
            &pool,
            tex_only,
            &format!("attachments/{tex_only}/arxiv-src.gz"),
            "arxiv-src.gz",
            "application/gzip",
        )
        .await
        .unwrap();

        let yes = EntryFilter { has_attachment: Some(true), ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &yes).await.unwrap();
        assert!(hits.is_empty(), "TeX ソースのみは PDF 添付ありに数えない");

        let no = EntryFilter { has_attachment: Some(false), ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &no).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "tex-only");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_by_tags_or_and(pool: SqlitePool) {
        let a = make_entry_full(&pool, "a", "article", None, false).await;
        let b = make_entry_full(&pool, "b", "article", None, false).await;
        let c = make_entry_full(&pool, "c", "article", None, false).await;
        let t1: i64 = sqlx::query_scalar("INSERT INTO tags (name) VALUES ('t1') RETURNING id")
            .fetch_one(&pool).await.unwrap();
        let t2: i64 = sqlx::query_scalar("INSERT INTO tags (name) VALUES ('t2') RETURNING id")
            .fetch_one(&pool).await.unwrap();
        bulk_add_tag(&pool, &[a], t1).await.unwrap();          // a: t1
        bulk_add_tag(&pool, &[b], t1).await.unwrap();          // b: t1, t2
        bulk_add_tag(&pool, &[b], t2).await.unwrap();
        bulk_add_tag(&pool, &[c], t2).await.unwrap();          // c: t2

        // OR: t1 または t2 → a, b, c 全部
        let or = EntryFilter { tag_ids: vec![t1, t2], tag_match: TagMatch::Or, ..Default::default() };
        assert_eq!(get_entries_filtered(&pool, None, None, None, &or).await.unwrap().len(), 3);

        // AND: t1 かつ t2 → b のみ
        let and = EntryFilter { tag_ids: vec![t1, t2], tag_match: TagMatch::And, ..Default::default() };
        let hits = get_entries_filtered(&pool, None, None, None, &and).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "b");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_combines_with_scope_and_dimensions(pool: SqlitePool) {
        // コレクション scope × 種別 × 年 の AND 合成
        let cid: i64 = sqlx::query_scalar("INSERT INTO collections (name) VALUES ('C') RETURNING id")
            .fetch_one(&pool).await.unwrap();
        let a = make_entry_full(&pool, "a", "article", Some(2021), false).await; // in C, article, 2021
        let b = make_entry_full(&pool, "b", "book", Some(2021), false).await;    // in C, book
        let c = make_entry_full(&pool, "c", "article", Some(2000), false).await; // in C, article, old
        make_entry_full(&pool, "d", "article", Some(2021), false).await;         // NOT in C
        for id in [a, b, c] {
            bulk_add_to_collection(&pool, &[id], cid).await.unwrap();
        }

        let f = EntryFilter { entry_types: vec!["article".into()], year_min: Some(2020), ..Default::default() };
        let hits = get_entries_filtered(&pool, Some(cid), None, None, &f).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "a");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_entries_respects_filter(pool: SqlitePool) {
        make_entry_full(&pool, "transformer networks", "article", Some(2021), false).await;
        make_entry_full(&pool, "transformer theory", "book", Some(2021), false).await;

        // クエリ "transformer" は 2 件ヒットするが、種別 article で 1 件に絞られる
        let f = EntryFilter { entry_types: vec!["article".into()], ..Default::default() };
        let hits = search_entries_filtered(&pool, "transformer", None, None, None, &f).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry_type, "article");
    }
}
