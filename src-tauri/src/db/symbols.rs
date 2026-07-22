//! LCIR `symbols` / `symbol_occurrences` テーブルのアクセサ（記号系・migration 0018・Phase 6b）。
//! 定義文から取り出した記号定義と、その記号の出現（display 数式内の表層一致）を持つ。
//! `ingestion::symbols` が解決した結果をトランザクション内で挿入し、read 面
//! （`load_lcir_document`）が版単位で引く。TeX 版のみ。

use crate::models::{Symbol, SymbolOccurrence};
use sqlx::SqlitePool;

/// 記号定義の挿入用パラメータ。
pub struct NewSymbol<'a> {
    pub document_version_id: i64,
    pub surface_form: &'a str,
    pub normalized_form: Option<&'a str>,
    pub description: Option<&'a str>,
    pub symbol_type: Option<&'a str>,
    pub defined_at_node_id: Option<i64>,
    pub scope_node_id: Option<i64>,
    pub semantic_json: Option<&'a str>,
    pub confidence: Option<f64>,
    pub origin: Option<&'a str>,
}

/// 記号出現の挿入用パラメータ。
pub struct NewSymbolOccurrence<'a> {
    pub symbol_id: i64,
    pub node_id: i64,
    pub local_offset_json: Option<&'a str>,
    pub surface_form: &'a str,
    pub confidence: Option<f64>,
    pub origin: Option<&'a str>,
}

/// 記号定義を挿入して id を返す。木構築のためトランザクション内でも使えるよう executor を取る。
pub async fn insert_symbol<'e, E>(executor: E, s: &NewSymbol<'_>) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO symbols
            (document_version_id, surface_form, normalized_form, description, symbol_type,
             defined_at_node_id, scope_node_id, semantic_json, confidence, origin)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(s.document_version_id)
    .bind(s.surface_form)
    .bind(s.normalized_form)
    .bind(s.description)
    .bind(s.symbol_type)
    .bind(s.defined_at_node_id)
    .bind(s.scope_node_id)
    .bind(s.semantic_json)
    .bind(s.confidence)
    .bind(s.origin)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// 記号出現を挿入する。
pub async fn insert_occurrence<'e, E>(
    executor: E,
    o: &NewSymbolOccurrence<'_>,
) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO symbol_occurrences
            (symbol_id, node_id, local_offset_json, surface_form, confidence, origin)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(o.symbol_id)
    .bind(o.node_id)
    .bind(o.local_offset_json)
    .bind(o.surface_form)
    .bind(o.confidence)
    .bind(o.origin)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// 1 バージョンの全記号を返す（read 面の `LcirDocument.symbols` 組み立て用）。
pub async fn symbols_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<Symbol>, sqlx::Error> {
    sqlx::query_as::<_, Symbol>(
        "SELECT * FROM symbols WHERE document_version_id = ? ORDER BY id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
}

/// 1 バージョンの全記号出現を返す（`symbols` を JOIN して版でスコープ）。
pub async fn occurrences_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<SymbolOccurrence>, sqlx::Error> {
    sqlx::query_as::<_, SymbolOccurrence>(
        "SELECT so.* FROM symbol_occurrences so
         JOIN symbols s ON s.id = so.symbol_id
         WHERE s.document_version_id = ?
         ORDER BY so.symbol_id, so.id",
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
        let att = add_attachment(pool, entry.id, "attachments/1/p.gz", "p.gz", "application/gzip")
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
                source_mime_type: "application/gzip",
                extractor_name: schema::TEX_EXTRACTOR_NAME,
                extractor_version: schema::TEX_EXTRACTOR_VERSION,
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
                node_kind: NodeKind::Paragraph.as_str(),
                ordinal: 0,
                plain_text: Some("let $U$ be the time evolution operator"),
                language: None,
                confidence: Some(0.9),
                origin: Some("tex_source"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        (vid, node)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_fetch_symbol_and_occurrence(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let sid = insert_symbol(
            &pool,
            &NewSymbol {
                document_version_id: vid,
                surface_form: "U",
                normalized_form: Some("U"),
                description: Some("the time evolution operator"),
                symbol_type: Some("operator"),
                defined_at_node_id: Some(node),
                scope_node_id: None,
                semantic_json: None,
                confidence: Some(0.6),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();
        insert_occurrence(
            &pool,
            &NewSymbolOccurrence {
                symbol_id: sid,
                node_id: node,
                local_offset_json: None,
                surface_form: "U",
                confidence: Some(0.5),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();

        let syms = symbols_for_version(&pool, vid).await.unwrap();
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].surface_form, "U");
        assert_eq!(syms[0].description.as_deref(), Some("the time evolution operator"));
        assert_eq!(syms[0].symbol_type.as_deref(), Some("operator"));

        let occs = occurrences_for_version(&pool, vid).await.unwrap();
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].symbol_id, sid);
    }

    /// 記号削除で出現も CASCADE 削除。ノード削除で記号も CASCADE 削除。
    #[sqlx::test(migrations = "./migrations")]
    async fn cascades_on_delete(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let sid = insert_symbol(
            &pool,
            &NewSymbol {
                document_version_id: vid,
                surface_form: "U",
                normalized_form: None,
                description: None,
                symbol_type: None,
                defined_at_node_id: Some(node),
                scope_node_id: None,
                semantic_json: None,
                confidence: Some(0.6),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();
        insert_occurrence(
            &pool,
            &NewSymbolOccurrence {
                symbol_id: sid,
                node_id: node,
                local_offset_json: None,
                surface_form: "U",
                confidence: Some(0.5),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();
        // ノード削除 → 記号（defined_at_node_id CASCADE）→ 出現（symbol_id CASCADE）が消える。
        sqlx::query("DELETE FROM document_nodes WHERE id = ?")
            .bind(node)
            .execute(&pool)
            .await
            .unwrap();
        assert!(symbols_for_version(&pool, vid).await.unwrap().is_empty());
        assert!(occurrences_for_version(&pool, vid).await.unwrap().is_empty());
    }
}
