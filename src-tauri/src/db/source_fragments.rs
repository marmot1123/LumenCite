//! LCIR `source_fragments` テーブルのアクセサ（ノード↔PDF 領域）。migration 0014。
//! 座標は既存 `highlights` と同一系（PDF user space・左下原点・pt）。

use crate::models::SourceFragment;
use sqlx::SqlitePool;

/// 新規 fragment の挿入用パラメータ。
pub struct NewSourceFragment<'a> {
    pub node_id: i64,
    pub page_number: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub reading_order: Option<i64>,
    pub fragment_type: Option<&'a str>,
}

/// fragment を挿入して id を返す。トランザクション内でも使えるよう executor を取る。
pub async fn insert_fragment<'e, E>(
    executor: E,
    f: &NewSourceFragment<'_>,
) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO source_fragments
            (node_id, page_number, x, y, width, height, rotation, reading_order, fragment_type)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(f.node_id)
    .bind(f.page_number)
    .bind(f.x)
    .bind(f.y)
    .bind(f.width)
    .bind(f.height)
    .bind(f.rotation)
    .bind(f.reading_order)
    .bind(f.fragment_type)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// 1 バージョンの全 fragment（ページ→読み順で安定ソート）。read 面用。
pub async fn fragments_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<SourceFragment>, sqlx::Error> {
    sqlx::query_as::<_, SourceFragment>(
        "SELECT sf.* FROM source_fragments sf
         JOIN document_nodes dn ON dn.id = sf.node_id
         WHERE dn.document_version_id = ?
         ORDER BY sf.page_number, sf.reading_order, sf.id",
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

    async fn setup_page_node(pool: &SqlitePool) -> (i64, i64) {
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
        let node = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Page.as_str(),
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
        (vid, node)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_fetch_by_version(pool: SqlitePool) {
        let (vid, node) = setup_page_node(&pool).await;
        insert_fragment(
            &pool,
            &NewSourceFragment {
                node_id: node,
                page_number: 1,
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 12.0,
                rotation: 0.0,
                reading_order: Some(0),
                fragment_type: Some("text_block"),
            },
        )
        .await
        .unwrap();

        let frags = fragments_for_version(&pool, vid).await.unwrap();
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].page_number, 1);
        assert_eq!(frags[0].x, 10.0);
        assert_eq!(frags[0].height, 12.0);
        assert_eq!(frags[0].fragment_type.as_deref(), Some("text_block"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn fragment_cascades_on_node_delete(pool: SqlitePool) {
        let (vid, node) = setup_page_node(&pool).await;
        insert_fragment(
            &pool,
            &NewSourceFragment {
                node_id: node,
                page_number: 1,
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                rotation: 0.0,
                reading_order: None,
                fragment_type: Some("page"),
            },
        )
        .await
        .unwrap();
        sqlx::query("DELETE FROM document_nodes WHERE id = ?")
            .bind(node)
            .execute(&pool)
            .await
            .unwrap();
        assert!(fragments_for_version(&pool, vid).await.unwrap().is_empty());
    }
}
