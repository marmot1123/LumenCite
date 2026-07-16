//! LCIR `document_nodes` テーブルのアクセサ（型付きノード木）。migration 0014。

use crate::models::DocumentNode;
use sqlx::SqlitePool;

/// 新規ノードの挿入用パラメータ。
pub struct NewDocumentNode<'a> {
    pub document_version_id: i64,
    pub parent_id: Option<i64>,
    pub node_kind: &'a str,
    pub ordinal: i64,
    pub plain_text: Option<&'a str>,
    pub language: Option<&'a str>,
    pub confidence: Option<f64>,
    pub origin: Option<&'a str>,
    pub payload_json: Option<&'a str>,
}

/// ノードを挿入して id を返す。木構築のためトランザクション内でも使えるよう executor を取る。
pub async fn insert_node<'e, E>(executor: E, n: &NewDocumentNode<'_>) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO document_nodes
            (document_version_id, parent_id, node_kind, ordinal, plain_text, language,
             confidence, origin, payload_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(n.document_version_id)
    .bind(n.parent_id)
    .bind(n.node_kind)
    .bind(n.ordinal)
    .bind(n.plain_text)
    .bind(n.language)
    .bind(n.confidence)
    .bind(n.origin)
    .bind(n.payload_json)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// バージョンの全ノード（親→子、読み順で安定ソート）。
pub async fn nodes_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<DocumentNode>, sqlx::Error> {
    sqlx::query_as::<_, DocumentNode>(
        "SELECT * FROM document_nodes
         WHERE document_version_id = ?
         ORDER BY COALESCE(parent_id, 0), ordinal, id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
}

/// バージョンの page ノードのみ（ordinal 昇順 = ページ順）。FTS 再生成・ページ数用。
pub async fn page_nodes_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<DocumentNode>, sqlx::Error> {
    sqlx::query_as::<_, DocumentNode>(
        "SELECT * FROM document_nodes
         WHERE document_version_id = ? AND node_kind = 'page'
         ORDER BY ordinal, id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::document_versions::{insert_version, NewDocumentVersion};
    use crate::db::entries::create_entry;
    use crate::document_ir::{schema, ExtractionStatus, NodeKind};
    use crate::models::EntryInput;

    async fn setup_version(pool: &SqlitePool) -> i64 {
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "P".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = add_attachment(pool, entry.id, "attachments/1/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap()
            .id;
        insert_version(
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
        .unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_tree_and_query(pool: SqlitePool) {
        let vid = setup_version(&pool).await;
        let doc = insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: None,
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(doc),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("page text"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(page),
                node_kind: NodeKind::TextBlock.as_str(),
                ordinal: 0,
                plain_text: Some("block"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();

        let all = nodes_for_version(&pool, vid).await.unwrap();
        assert_eq!(all.len(), 3);
        let pages = page_nodes_for_version(&pool, vid).await.unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].plain_text.as_deref(), Some("page text"));
        assert_eq!(pages[0].id, page);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn nodes_cascade_on_version_delete(pool: SqlitePool) {
        let vid = setup_version(&pool).await;
        insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: None,
                payload_json: None,
            },
        )
        .await
        .unwrap();
        sqlx::query("DELETE FROM document_versions WHERE id = ?")
            .bind(vid)
            .execute(&pool)
            .await
            .unwrap();
        assert!(nodes_for_version(&pool, vid).await.unwrap().is_empty());
    }
}
