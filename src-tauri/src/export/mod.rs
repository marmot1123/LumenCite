//! Phase 9a: LCIR エクスポート（外部書き出し面）。
//!
//! 正本は SQLite（`document_versions`/`document_nodes`/…）。ここは `LcirDocument`
//! 派生ビューをファイルに出すだけの決定的な変換で、DB には一切書かない。
//! JATS/TEI/HTML+MathML は Phase 9b（Presentation MathML = Phase 7 が前提）。

pub mod markdown;
pub mod warning;

pub use markdown::{render_markdown, MarkdownHeader};
pub use warning::{ExportSeverity, ExportWarning, ExportWarningCode};

use crate::document_ir::LcirDocument;

/// エクスポート 1 回分の結果。`text` が書き出す本文で、`warnings` は
/// 「**この形式では運べなかった LCIR 固有情報**」（Phase 9 完了条件・debt-8）。
/// **警告はエラーではない** — 書き出しは成功している。9b の全形式が同じ形を返す。
#[derive(Debug, Clone, PartialEq)]
pub struct ExportReport {
    pub text: String,
    pub warnings: Vec<ExportWarning>,
}

/// LCIR JSON（pretty）。書き出し前に schema validation を必ず通す —
/// 不正な LCIR を外部形式として確定させない（Phase 1 完了条件の validation を流用）。
/// validation（不正を弾く・エラー）と警告（正しいが形式で落ちる）は別物。
pub fn lcir_json_pretty(doc: &LcirDocument) -> Result<ExportReport, String> {
    crate::document_ir::validation::validate(doc)
        .map_err(|errs| format!("LCIR validation failed: {}", errs.join("; ")))?;
    let text = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    let mut sink = warning::WarningSink::new();
    warning::collect_document_warnings(doc, &warning::LCIR_JSON, &mut sink);
    Ok(ExportReport {
        text,
        warnings: sink.finish(),
    })
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
                alt_text: None,
            }],
            relations: Vec::new(),
            symbols: Vec::new(),
        }
    }

    #[test]
    fn json_export_is_valid_and_round_trips() {
        let doc = minimal_doc();
        let report = lcir_json_pretty(&doc).unwrap();
        let back: LcirDocument = serde_json::from_str(&report.text).unwrap();
        assert_eq!(back, doc, "pretty JSON はラウンドトリップする");
    }

    /// LCIR JSON は派生ビューとして無損失なので、アセットが無ければ警告は 1 件も出ない。
    #[test]
    fn json_export_reports_no_warnings_without_assets() {
        let report = lcir_json_pretty(&minimal_doc()).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    /// 唯一の欠落はアセット**実体**の非同梱（`relative_path` はメタデータ参照）。
    #[test]
    fn json_export_warns_only_about_unembedded_assets() {
        let mut doc = minimal_doc();
        doc.nodes[0].assets = vec![crate::document_ir::LcirAsset {
            role: "page_crop".to_string(),
            mime_type: "image/png".to_string(),
            relative_path: "a.png".to_string(),
            width: None,
            height: None,
            size_bytes: None,
            sha256: "x".to_string(),
            metadata: None,
        }];
        let report = lcir_json_pretty(&doc).unwrap();
        let codes: Vec<&str> = report.warnings.iter().map(|w| w.code.as_str()).collect();
        assert_eq!(codes, vec!["assets_not_embedded"], "{:?}", report.warnings);
    }

    #[test]
    fn json_export_rejects_invalid_document() {
        let mut doc = minimal_doc();
        doc.schema_version = String::new();
        let err = lcir_json_pretty(&doc).unwrap_err();
        assert!(err.contains("validation failed"), "{err}");
        // validation の失敗は「警告 0 件で成功」ではなく Err（不正な LCIR を確定させない）。
    }
}
