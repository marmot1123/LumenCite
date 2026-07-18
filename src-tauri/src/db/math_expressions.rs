//! LCIR `math_expressions` テーブルのアクセサ（数式の複数表現・migration 0016・Phase 3）。
//! inline_math/display_math ノードに 1:1 で付く。PDF 由来は表層（normalized_text）のみ埋め、
//! LaTeX/MathML/AST は後続フェーズ（TeX 取込・意味）で埋める。

use crate::models::MathExpression;
use sqlx::SqlitePool;

/// 数式表現の挿入用パラメータ。
pub struct NewMathExpression<'a> {
    pub node_id: i64,
    pub display_mode: &'a str,
    pub equation_label: Option<&'a str>,
    pub latex: Option<&'a str>,
    pub presentation_mathml: Option<&'a str>,
    pub content_mathml: Option<&'a str>,
    pub openmath_json: Option<&'a str>,
    pub normalized_text: Option<&'a str>,
    pub ast_json: Option<&'a str>,
    pub semantic_status: &'a str,
    pub confidence: Option<f64>,
    pub origin: Option<&'a str>,
}

/// 数式表現を挿入して id を返す。木構築のためトランザクション内でも使えるよう executor を取る。
pub async fn insert_math<'e, E>(executor: E, m: &NewMathExpression<'_>) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO math_expressions
            (node_id, display_mode, equation_label, latex, presentation_mathml, content_mathml,
             openmath_json, normalized_text, ast_json, semantic_status, confidence, origin)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(m.node_id)
    .bind(m.display_mode)
    .bind(m.equation_label)
    .bind(m.latex)
    .bind(m.presentation_mathml)
    .bind(m.content_mathml)
    .bind(m.openmath_json)
    .bind(m.normalized_text)
    .bind(m.ast_json)
    .bind(m.semantic_status)
    .bind(m.confidence)
    .bind(m.origin)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// 1 バージョンの全数式を `node_id` で引けるよう返す（read 面の LcirNode.math 組み立て用）。
pub async fn math_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<MathExpression>, sqlx::Error> {
    sqlx::query_as::<_, MathExpression>(
        "SELECT me.* FROM math_expressions me
         JOIN document_nodes dn ON dn.id = me.node_id
         WHERE dn.document_version_id = ?
         ORDER BY me.node_id, me.id",
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
    use crate::document_ir::{schema, ExtractionStatus, MathSemanticStatus, NodeKind};
    use crate::models::EntryInput;

    async fn setup_math_node(pool: &SqlitePool) -> (i64, i64) {
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
                node_kind: NodeKind::DisplayMath.as_str(),
                ordinal: 0,
                plain_text: Some("U = S2 C2 S1 C1"),
                language: None,
                confidence: Some(0.6),
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        (vid, node)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_fetch_surface_math(pool: SqlitePool) {
        let (vid, node) = setup_math_node(&pool).await;
        insert_math(
            &pool,
            &NewMathExpression {
                node_id: node,
                display_mode: "display",
                equation_label: Some("(2.1)"),
                latex: None,
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("U = S2 C2 S1 C1"),
                ast_json: None,
                semantic_status: MathSemanticStatus::SurfaceOnly.as_str(),
                confidence: Some(0.6),
                origin: Some("pdf_text_layer"),
            },
        )
        .await
        .unwrap();

        let all = math_for_version(&pool, vid).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].display_mode, "display");
        assert_eq!(all[0].equation_label.as_deref(), Some("(2.1)"));
        assert_eq!(all[0].semantic_status, "surface_only");
        assert!(all[0].latex.is_none(), "PDF 由来では LaTeX は未確定");
        assert_eq!(all[0].normalized_text.as_deref(), Some("U = S2 C2 S1 C1"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn math_cascades_on_node_delete(pool: SqlitePool) {
        let (vid, node) = setup_math_node(&pool).await;
        insert_math(
            &pool,
            &NewMathExpression {
                node_id: node,
                display_mode: "display",
                equation_label: None,
                latex: None,
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("x + y"),
                ast_json: None,
                semantic_status: MathSemanticStatus::SurfaceOnly.as_str(),
                confidence: Some(0.5),
                origin: Some("pdf_text_layer"),
            },
        )
        .await
        .unwrap();
        sqlx::query("DELETE FROM document_nodes WHERE id = ?")
            .bind(node)
            .execute(&pool)
            .await
            .unwrap();
        assert!(math_for_version(&pool, vid).await.unwrap().is_empty());
    }
}
