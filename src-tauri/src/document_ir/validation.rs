//! LCIR JSON の最小構造 validation（golden / schema テスト用）。
//! 完全な JSON Schema 検証ではなく、不変条件（ルート 1 個・親参照の整合）を守る軽量チェック。

use super::node::LcirDocument;
use std::collections::HashSet;

/// LCIR ドキュメントの最小限の構造検証。壊れていれば理由のリストを返す。
pub fn validate(doc: &LcirDocument) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if doc.schema.is_empty() {
        errors.push("schema is empty".to_string());
    }
    if doc.schema_version.is_empty() {
        errors.push("schema_version is empty".to_string());
    }
    if doc.content_key.is_empty() {
        errors.push("content_key is empty".to_string());
    }
    if doc.source.sha256.is_empty() {
        errors.push("source.sha256 is empty".to_string());
    }
    if doc.nodes.is_empty() {
        errors.push("nodes is empty".to_string());
    }

    // ルート（parent_id なし）はちょうど 1 個。
    let roots = doc.nodes.iter().filter(|n| n.parent_id.is_none()).count();
    if !doc.nodes.is_empty() && roots != 1 {
        errors.push(format!("expected exactly 1 root node, found {roots}"));
    }

    // parent_id は存在するノードを指す。
    let ids: HashSet<i64> = doc.nodes.iter().map(|n| n.id).collect();
    for n in &doc.nodes {
        if let Some(p) = n.parent_id {
            if !ids.contains(&p) {
                errors.push(format!("node {} references missing parent {}", n.id, p));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::super::node::{LcirDocument, LcirNode, LcirSource};
    use super::super::source::CoordinateSpace;
    use super::*;

    fn minimal(nodes: Vec<LcirNode>) -> LcirDocument {
        LcirDocument {
            schema: "s".to_string(),
            schema_version: "0.1.0".to_string(),
            version_id: 1,
            content_key: "ck".to_string(),
            source: LcirSource {
                sha256: "abc".to_string(),
                mime_type: "application/pdf".to_string(),
                extractor_name: "lumencite-pdfium".to_string(),
                extractor_version: "0.1.0".to_string(),
            },
            coordinate_space: Some(CoordinateSpace::default()),
            nodes,
            relations: vec![],
            symbols: vec![],
        }
    }

    fn node(id: i64, parent: Option<i64>) -> LcirNode {
        LcirNode {
            id,
            kind: "page".to_string(),
            ordinal: 0,
            parent_id: parent,
            plain_text: None,
            origin: None,
            confidence: None,
            payload: None,
            math: None,
            source_fragments: vec![],
        }
    }

    #[test]
    fn valid_document_passes() {
        let doc = minimal(vec![node(1, None), node(2, Some(1))]);
        assert!(validate(&doc).is_ok());
    }

    #[test]
    fn missing_root_fails() {
        // どちらも親を持つ → ルート 0 個。
        let doc = minimal(vec![node(1, Some(2)), node(2, Some(1))]);
        let errs = validate(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("root")));
    }

    #[test]
    fn dangling_parent_fails() {
        let doc = minimal(vec![node(1, None), node(2, Some(99))]);
        let errs = validate(&doc).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("missing parent")));
    }

    #[test]
    fn empty_nodes_fails() {
        let doc = minimal(vec![]);
        assert!(validate(&doc).is_err());
    }
}
