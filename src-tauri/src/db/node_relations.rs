//! LCIR `node_relations` テーブルのアクセサ（ノード間の型付き関係・migration 0017・Phase 6a）。
//! 参照グラフ（本文 → 数式/定理/図/文献・proof → theorem）を有向辺で持つ。`ingestion::graph` が
//! 解決した辺をトランザクション内で挿入し、read 面（`load_lcir_document`）が版単位で引く。

use crate::models::NodeRelation;
use sqlx::SqlitePool;

/// 関係辺の挿入用パラメータ。
pub struct NewNodeRelation<'a> {
    pub document_version_id: i64,
    pub from_node_id: i64,
    pub relation_type: &'a str,
    pub to_node_id: i64,
    pub confidence: Option<f64>,
    pub origin: Option<&'a str>,
    pub metadata_json: Option<&'a str>,
}

/// 関係辺を挿入して id を返す。木構築のためトランザクション内でも使えるよう executor を取る。
pub async fn insert_relation<'e, E>(
    executor: E,
    r: &NewNodeRelation<'_>,
) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO node_relations
            (document_version_id, from_node_id, relation_type, to_node_id, confidence, origin, metadata_json)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(r.document_version_id)
    .bind(r.from_node_id)
    .bind(r.relation_type)
    .bind(r.to_node_id)
    .bind(r.confidence)
    .bind(r.origin)
    .bind(r.metadata_json)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// 1 バージョンの全関係辺を返す（read 面の `LcirDocument.relations` 組み立て用）。
pub async fn relations_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<NodeRelation>, sqlx::Error> {
    sqlx::query_as::<_, NodeRelation>(
        "SELECT * FROM node_relations
         WHERE document_version_id = ?
         ORDER BY from_node_id, id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::document_nodes::{insert_node, NewDocumentNode};
    use crate::db::document_versions::{insert_version, NewDocumentVersion};
    use crate::db::entries::create_entry;
    use crate::document_ir::{schema, ExtractionStatus, NodeKind};
    use crate::models::EntryInput;

    /// entry → attachment → version → (paragraph, theorem) の 2 ノードを用意して version_id と
    /// (from, to) node_id を返す。
    async fn setup_two_nodes(pool: &SqlitePool) -> (i64, i64, i64) {
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
        let root = insert_node(
            pool,
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
        let from = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Paragraph.as_str(),
                ordinal: 0,
                plain_text: Some("by Theorem 2.3"),
                language: None,
                confidence: Some(0.9),
                origin: Some("layout_model"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let to = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Theorem.as_str(),
                ordinal: 1,
                plain_text: Some("Theorem 2.3. ..."),
                language: None,
                confidence: Some(0.7),
                origin: Some("layout_model"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        (vid, from, to)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_fetch_relation(pool: SqlitePool) {
        let (vid, from, to) = setup_two_nodes(&pool).await;
        insert_relation(
            &pool,
            &NewNodeRelation {
                document_version_id: vid,
                from_node_id: from,
                relation_type: "refers_to_theorem",
                to_node_id: to,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                metadata_json: Some(r#"{"number":"2.3"}"#),
            },
        )
        .await
        .unwrap();

        let rels = relations_for_version(&pool, vid).await.unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].from_node_id, from);
        assert_eq!(rels[0].to_node_id, to);
        assert_eq!(rels[0].relation_type, "refers_to_theorem");
        assert_eq!(rels[0].origin.as_deref(), Some("layout_model"));
        assert_eq!(rels[0].metadata_json.as_deref(), Some(r#"{"number":"2.3"}"#));
    }

    /// ノード削除（= バージョン/添付削除のカスケード）で関係辺も消える（FK ON DELETE CASCADE）。
    #[sqlx::test(migrations = "./migrations")]
    async fn relation_cascades_on_node_delete(pool: SqlitePool) {
        let (vid, from, to) = setup_two_nodes(&pool).await;
        insert_relation(
            &pool,
            &NewNodeRelation {
                document_version_id: vid,
                from_node_id: from,
                relation_type: "refers_to_theorem",
                to_node_id: to,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        // from ノードを消すと辺も消える。
        sqlx::query("DELETE FROM document_nodes WHERE id = ?")
            .bind(from)
            .execute(&pool)
            .await
            .unwrap();
        let rels = relations_for_version(&pool, vid).await.unwrap();
        assert!(rels.is_empty(), "from ノード削除で辺が CASCADE 削除される");
    }
}
