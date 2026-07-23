//! Phase 9a: LCIR エクスポート（外部書き出し面）。
//!
//! 正本は SQLite（`document_versions`/`document_nodes`/…）。ここは `LcirDocument`
//! 派生ビューをファイルに出すだけの決定的な変換で、DB には一切書かない。
//! JATS/TEI/HTML+MathML は Phase 9b（Presentation MathML = Phase 7 が前提）。

pub mod markdown;

pub use markdown::{render_markdown, MarkdownHeader};

use crate::document_ir::LcirDocument;

/// LCIR JSON（pretty）。書き出し前に schema validation を必ず通す —
/// 不正な LCIR を外部形式として確定させない（Phase 1 完了条件の validation を流用）。
pub fn lcir_json_pretty(doc: &LcirDocument) -> Result<String, String> {
    crate::document_ir::validation::validate(doc)
        .map_err(|errs| format!("LCIR validation failed: {}", errs.join("; ")))?;
    serde_json::to_string_pretty(doc).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_ir::{LcirNode, LcirSource};

    fn minimal_doc() -> LcirDocument {
        LcirDocument {
            schema: crate::document_ir::schema::SCHEMA_URI.to_string(),
            schema_version: crate::document_ir::schema::SCHEMA_VERSION.to_string(),
            version_id: 1,
            content_key: "k".to_string(),
            source: LcirSource {
                sha256: "s".to_string(),
                mime_type: "application/pdf".to_string(),
                extractor_name: "lumencite-pdfium".to_string(),
                extractor_version: "0.5.0".to_string(),
            },
            coordinate_space: None,
            nodes: vec![LcirNode {
                id: 1,
                kind: "document".to_string(),
                ordinal: 0,
                parent_id: None,
                plain_text: None,
                origin: None,
                confidence: None,
                payload: None,
                math: None,
                source_fragments: Vec::new(),
                assets: Vec::new(),
            }],
            relations: Vec::new(),
            symbols: Vec::new(),
        }
    }

    #[test]
    fn json_export_is_valid_and_round_trips() {
        let doc = minimal_doc();
        let json = lcir_json_pretty(&doc).unwrap();
        let back: LcirDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(back, doc, "pretty JSON はラウンドトリップする");
    }

    #[test]
    fn json_export_rejects_invalid_document() {
        let mut doc = minimal_doc();
        doc.schema_version = String::new();
        let err = lcir_json_pretty(&doc).unwrap_err();
        assert!(err.contains("validation failed"), "{err}");
    }
}
