//! LCIR `assets` / `node_assets` テーブルのアクセサ（図表アセット・migration 0019・Phase 8a）。
//! 図領域のページ crop PNG 等のバイナリをファイルシステムに置き、DB は相対パス + SHA-256 参照を
//! 持つ。build のトランザクション内で挿入し、read 面（`load_lcir_document`）が版単位で引く。
//! PDF 版のみ。ファイルの存在は保証しない（欠損許容）。

use crate::models::{Asset, NodeAsset};
use sqlx::SqlitePool;

/// アセットの挿入用パラメータ。
pub struct NewAsset<'a> {
    pub document_version_id: i64,
    pub sha256: &'a str,
    pub mime_type: &'a str,
    pub relative_path: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub size_bytes: Option<i64>,
    pub metadata_json: Option<&'a str>,
}

/// ノード ↔ アセット紐づけの挿入用パラメータ。
pub struct NewNodeAsset {
    pub node_id: i64,
    pub asset_id: i64,
}

/// アセットを挿入して id を返す。木構築のためトランザクション内でも使えるよう executor を取る。
pub async fn insert_asset<'e, E>(executor: E, a: &NewAsset<'_>) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO assets
            (document_version_id, sha256, mime_type, relative_path, width, height,
             size_bytes, metadata_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(a.document_version_id)
    .bind(a.sha256)
    .bind(a.mime_type)
    .bind(a.relative_path)
    .bind(a.width)
    .bind(a.height)
    .bind(a.size_bytes)
    .bind(a.metadata_json)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// ノード ↔ アセット紐づけを挿入する。
pub async fn insert_node_asset<'e, E>(
    executor: E,
    n: &NewNodeAsset,
    role: &str,
) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query("INSERT INTO node_assets (node_id, asset_id, role) VALUES (?, ?, ?)")
        .bind(n.node_id)
        .bind(n.asset_id)
        .bind(role)
        .execute(executor)
        .await?
        .last_insert_rowid();
    Ok(id)
}

/// self-heal（Phase 8a・reuse 経路）: 再レンダリングしたファイルのメタデータを
/// relative_path 一致で更新する。行が無ければ 0 を返す（新規行は作らない）。
pub async fn refresh_asset_file(
    pool: &SqlitePool,
    version_id: i64,
    relative_path: &str,
    sha256: &str,
    dims: (i64, i64),
    size_bytes: i64,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE assets SET sha256 = ?, width = ?, height = ?, size_bytes = ?
         WHERE document_version_id = ? AND relative_path = ?",
    )
    .bind(sha256)
    .bind(dims.0)
    .bind(dims.1)
    .bind(size_bytes)
    .bind(version_id)
    .bind(relative_path)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 1 バージョンの全アセットを返す（read 面の `LcirNode.assets` 組み立て用）。
pub async fn assets_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<Asset>, sqlx::Error> {
    sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE document_version_id = ? ORDER BY id")
        .bind(version_id)
        .fetch_all(pool)
        .await
}

/// 1 バージョンの全ノード ↔ アセット紐づけを返す（`document_nodes` を JOIN して版でスコープ）。
pub async fn node_assets_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<NodeAsset>, sqlx::Error> {
    sqlx::query_as::<_, NodeAsset>(
        "SELECT na.* FROM node_assets na
         JOIN document_nodes dn ON dn.id = na.node_id
         WHERE dn.document_version_id = ?
         ORDER BY na.node_id, na.id",
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

    async fn setup_node(pool: &SqlitePool) -> (i64, i64) {
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
                node_kind: NodeKind::Figure.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: Some(r#"{"figure_index":1}"#),
            },
        )
        .await
        .unwrap();
        (vid, node)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_fetch_asset_and_link(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: vid,
                sha256: "abc123",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/deadbeef/fig-p001-00.png",
                width: Some(800),
                height: Some(600),
                size_bytes: Some(12345),
                metadata_json: Some(r#"{"page":1,"region_index":0}"#),
            },
        )
        .await
        .unwrap();
        insert_node_asset(&pool, &NewNodeAsset { node_id: node, asset_id: aid }, "page_crop")
            .await
            .unwrap();

        let assets = assets_for_version(&pool, vid).await.unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].sha256, "abc123");
        assert_eq!(assets[0].relative_path, "attachments/1/.lcir/1/deadbeef/fig-p001-00.png");
        assert_eq!(assets[0].width, Some(800));
        assert_eq!(assets[0].size_bytes, Some(12345));

        let links = node_assets_for_version(&pool, vid).await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].node_id, node);
        assert_eq!(links[0].asset_id, aid);
        assert_eq!(links[0].role, "page_crop");
    }

    /// 同一 (node, asset, role) の重複紐づけは UNIQUE 制約で拒否される。
    #[sqlx::test(migrations = "./migrations")]
    async fn duplicate_link_is_rejected(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: vid,
                sha256: "abc",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/deadbeef/fig-p001-00.png",
                width: None,
                height: None,
                size_bytes: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let link = NewNodeAsset { node_id: node, asset_id: aid };
        insert_node_asset(&pool, &link, "page_crop").await.unwrap();
        assert!(insert_node_asset(&pool, &link, "page_crop").await.is_err());
        // 別 role なら許される。
        insert_node_asset(&pool, &link, "thumbnail").await.unwrap();
    }

    /// version 削除でアセットが、ノード削除で紐づけが CASCADE 削除される。
    #[sqlx::test(migrations = "./migrations")]
    async fn cascades_on_delete(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: vid,
                sha256: "abc",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/deadbeef/fig-p001-00.png",
                width: None,
                height: None,
                size_bytes: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        insert_node_asset(&pool, &NewNodeAsset { node_id: node, asset_id: aid }, "page_crop")
            .await
            .unwrap();

        // ノード削除 → 紐づけだけ消え、アセット行は残る。
        sqlx::query("DELETE FROM document_nodes WHERE id = ?")
            .bind(node)
            .execute(&pool)
            .await
            .unwrap();
        assert!(node_assets_for_version(&pool, vid).await.unwrap().is_empty());
        assert_eq!(assets_for_version(&pool, vid).await.unwrap().len(), 1);

        // version 削除 → アセット行も消える。
        sqlx::query("DELETE FROM document_versions WHERE id = ?")
            .bind(vid)
            .execute(&pool)
            .await
            .unwrap();
        assert!(assets_for_version(&pool, vid).await.unwrap().is_empty());
    }
}
