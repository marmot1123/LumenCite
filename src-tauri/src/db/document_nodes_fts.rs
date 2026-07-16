//! LCIR ノード単位 FTS（`document_nodes_fts` / migration 0015・Phase 2）。
//!
//! 既存 `fulltext`（ページ粒度）と併存する派生索引で、段落・見出し・caption 等の**ブロック粒度**で
//! 当てる。正本は LCIR（`document_nodes`）で、これは `ingestion::regenerate_node_fts_from_lcir` で
//! 再生成できる。FTS5 仮想表なので attachments への FK は無く、削除時は `attachment_id` 指定で
//! 手動クリーンアップする（`fulltext` と同じ作法）。検索面は `search_fulltext` を踏襲する。

use crate::db::fulltext::{build_match_expr, load_summary};
use crate::db::source_fragments;
use crate::document_ir::BBox;
use crate::models::NodeFtsHit;
use sqlx::{Row, SqlitePool};

/// node-FTS へ入れる 1 行（ブロックノード 1 個）。
pub struct NodeFtsInput {
    pub node_id: i64,
    pub page: i64,
    pub node_kind: String,
    pub content: String,
}

/// 添付の node-FTS を丸ごと張り直す（既存行を消してから入れ直す）。空 content はスキップ。
pub async fn index_nodes(
    pool: &SqlitePool,
    attachment_id: i64,
    rows: &[NodeFtsInput],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM document_nodes_fts WHERE attachment_id = ?")
        .bind(attachment_id)
        .execute(&mut *tx)
        .await?;

    for r in rows {
        if r.content.trim().is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO document_nodes_fts (content, node_id, attachment_id, page, node_kind)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&r.content)
        .bind(r.node_id)
        .bind(attachment_id)
        .bind(r.page)
        .bind(&r.node_kind)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 添付の node-FTS 行を全消去する。
pub async fn unindex_attachment(pool: &SqlitePool, attachment_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM document_nodes_fts WHERE attachment_id = ?")
        .bind(attachment_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// ノード単位の全文検索。ヒットごとに `node_kind`・`page`・（あれば）ブロック領域 `bbox` を返す。
/// `search_fulltext` と同じく短い/CJK トークンは LIKE フォールバックする。
pub async fn search_nodes(
    pool: &SqlitePool,
    query: &str,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
) -> Result<Vec<NodeFtsHit>, sqlx::Error> {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // trigram は 3 文字未満を扱えない。短いトークンがあれば LIKE フォールバック。
    let use_like = tokens.iter().any(|t| t.chars().count() < 3);

    let mut sql = String::from(
        "SELECT dnf.node_id AS node_id, dnf.attachment_id AS attachment_id, \
                dnf.page AS page, dnf.node_kind AS node_kind, ",
    );
    if use_like {
        sql.push_str("substr(dnf.content, 1, 200) AS snippet, ");
    } else {
        sql.push_str("snippet(document_nodes_fts, 0, '⟨', '⟩', '…', 12) AS snippet, ");
    }
    sql.push_str(
        "a.entry_id AS entry_id
         FROM document_nodes_fts dnf
         JOIN attachments a ON a.id = dnf.attachment_id
         WHERE ",
    );

    if use_like {
        let likes: Vec<&str> = tokens
            .iter()
            .map(|_| "dnf.content LIKE ? ESCAPE '\\'")
            .collect();
        sql.push_str(&likes.join(" AND "));
    } else {
        sql.push_str("document_nodes_fts MATCH ?");
    }

    // view スコープ（trash ビュー時はゴミ箱内、それ以外は現役のみ）。
    if matches!(view, Some("trash")) {
        sql.push_str(" AND a.entry_id IN (SELECT id FROM entries WHERE deleted_at IS NOT NULL)");
    } else {
        sql.push_str(" AND a.entry_id IN (SELECT id FROM entries WHERE deleted_at IS NULL)");
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
        sql.push_str(" ORDER BY dnf.attachment_id, dnf.page, dnf.node_id");
    } else {
        sql.push_str(" ORDER BY bm25(document_nodes_fts)");
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
        let node_id: i64 = row.get("node_id");
        let summary = load_summary(pool, entry_id).await?;
        let bbox = source_fragments::primary_fragment_for_node(pool, node_id)
            .await?
            .map(|f| BBox::new(f.x, f.y, f.width, f.height));
        hits.push(NodeFtsHit {
            entry: summary,
            attachment_id: row.get("attachment_id"),
            node_id,
            page: row.get("page"),
            node_kind: row.get("node_kind"),
            snippet: row.get("snippet"),
            bbox,
        });
    }

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::document_nodes::{insert_node, NewDocumentNode};
    use crate::db::document_versions::{insert_version, NewDocumentVersion};
    use crate::db::entries::create_entry;
    use crate::db::source_fragments::{insert_fragment, NewSourceFragment};
    use crate::document_ir::{schema, ExtractionStatus, NodeKind};
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
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();
        (entry.id, att.id)
    }

    /// 実ノード + fragment を用意し、その node_id を index する（bbox 検証用）。
    async fn setup_block_node(pool: &SqlitePool, att: i64, kind: &str) -> i64 {
        let vid = insert_version(
            pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let node = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: kind,
                ordinal: 0,
                plain_text: Some("Transformer architecture is described in detail here."),
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        insert_fragment(
            pool,
            &NewSourceFragment {
                node_id: node,
                page_number: 4,
                x: 72.0,
                y: 500.0,
                width: 400.0,
                height: 30.0,
                rotation: 0.0,
                reading_order: Some(0),
                fragment_type: Some("block"),
            },
        )
        .await
        .unwrap();
        node
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn indexed_node_is_searchable_with_kind_and_bbox(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Paper").await;
        let node = setup_block_node(&pool, att, NodeKind::Paragraph.as_str()).await;
        index_nodes(
            &pool,
            att,
            &[NodeFtsInput {
                node_id: node,
                page: 4,
                node_kind: NodeKind::Paragraph.as_str().to_string(),
                content: "Transformer architecture is described in detail here.".to_string(),
            }],
        )
        .await
        .unwrap();

        let hits = search_nodes(&pool, "transformer", None, None, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, node);
        assert_eq!(hits[0].node_kind, "paragraph");
        assert_eq!(hits[0].page, 4);
        assert!(hits[0].snippet.to_lowercase().contains("transformer"));
        let bbox = hits[0].bbox.as_ref().expect("block fragment → bbox");
        assert_eq!(bbox.y, 500.0);
        assert_eq!(bbox.width, 400.0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reindex_replaces_old_rows(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Paper").await;
        index_nodes(
            &pool,
            att,
            &[NodeFtsInput {
                node_id: 1,
                page: 1,
                node_kind: "paragraph".to_string(),
                content: "old obsolete keyword".to_string(),
            }],
        )
        .await
        .unwrap();
        index_nodes(
            &pool,
            att,
            &[NodeFtsInput {
                node_id: 2,
                page: 1,
                node_kind: "paragraph".to_string(),
                content: "fresh replacement content".to_string(),
            }],
        )
        .await
        .unwrap();

        assert!(search_nodes(&pool, "obsolete", None, None, None)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            search_nodes(&pool, "replacement", None, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unindex_removes_rows(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Paper").await;
        index_nodes(
            &pool,
            att,
            &[NodeFtsInput {
                node_id: 1,
                page: 1,
                node_kind: "heading".to_string(),
                content: "needle in the haystack".to_string(),
            }],
        )
        .await
        .unwrap();
        unindex_attachment(&pool, att).await.unwrap();
        assert!(search_nodes(&pool, "needle", None, None, None)
            .await
            .unwrap()
            .is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn short_and_cjk_queries_use_like_fallback(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "和文").await;
        index_nodes(
            &pool,
            att,
            &[
                NodeFtsInput {
                    node_id: 1,
                    page: 1,
                    node_kind: "paragraph".to_string(),
                    content: "本論文では深層学習モデルを評価する。".to_string(),
                },
                NodeFtsInput {
                    node_id: 2,
                    page: 1,
                    node_kind: "paragraph".to_string(),
                    content: "AI models are evolving rapidly.".to_string(),
                },
            ],
        )
        .await
        .unwrap();

        assert_eq!(
            search_nodes(&pool, "深層学習", None, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
        // 短いトークン（<3 文字）→ LIKE。
        assert_eq!(
            search_nodes(&pool, "AI", None, None, None).await.unwrap().len(),
            1
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_query_returns_empty(pool: SqlitePool) {
        assert!(search_nodes(&pool, "   ", None, None, None)
            .await
            .unwrap()
            .is_empty());
    }

    /// hard delete で node-FTS 行も消える（FK 無しの mirror を entries の delete 経路が掃除する）。
    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_removes_node_fts_rows(pool: SqlitePool) {
        let (entry_id, att) = setup_attachment(&pool, "Paper").await;
        index_nodes(
            &pool,
            att,
            &[NodeFtsInput {
                node_id: 1,
                page: 1,
                node_kind: "paragraph".to_string(),
                content: "needle in the node index".to_string(),
            }],
        )
        .await
        .unwrap();

        crate::db::entries::delete_entry(&pool, entry_id).await.unwrap();

        let orphans: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM document_nodes_fts WHERE attachment_id = ?")
                .bind(att)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(orphans, 0, "hard delete must not orphan node-FTS rows");
    }
}
