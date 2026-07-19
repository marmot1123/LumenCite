//! LCIR (LumenCite Document Intermediate Representation) の DB 非依存な型と純関数。
//!
//! 一次仕様は Rust 型（serde でシリアライズ）。正本は SQLite（`document_versions` /
//! `document_nodes` / `source_fragments`）で、`LcirDocument` はその派生ビュー。
//! ここは pdfium にも sqlx にも依存しないので CI で完全にテストできる。

pub mod math;
pub mod node;
pub mod schema;
pub mod source;
pub mod validation;

pub use math::{LcirMath, MathDisplayMode, MathSemanticStatus};
pub use node::{
    ExtractionStatus, LcirDocument, LcirFragment, LcirNode, LcirSource, NodeKind, Origin,
};
pub use source::{BBox, CoordinateSpace, FragmentType};

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// バイト列の SHA-256（小文字 hex）。
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes.as_ref());
    to_hex_lower(hasher.finalize())
}

/// ファイル本体の SHA-256（小文字 hex）。8 KiB ストリームで読む。
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex_lower(hasher.finalize()))
}

fn to_hex_lower(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// 再現可能な内容由来 ID。同一 PDF バイト + 同一抽出器（名前・版・設定）→ 同一 content_key。
/// row id は SQLite 採番で再現不能なため、これで roadmap の「同一 PDF → 同一 version」を満たす。
/// 添付には依存しない（同一ファイルを別添付にしても同じ値になる。一意性は
/// `(attachment_id, content_key)` の best-effort UNIQUE で担保する）。
pub fn content_key(
    source_sha256: &str,
    extractor_name: &str,
    extractor_version: &str,
    config_hash: &str,
) -> String {
    let material = format!(
        "lcir-content-key-v1\n{source_sha256}\n{extractor_name}\n{extractor_version}\n{config_hash}"
    );
    sha256_hex(material.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vector() {
        // sha256("abc")
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_file_matches_bytes() {
        let dir = std::env::temp_dir();
        let path = dir.join("lcir_sha256_file_test.bin");
        std::fs::write(&path, b"abc").unwrap();
        let from_file = sha256_file(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(from_file, sha256_hex("abc"));
    }

    #[test]
    fn content_key_is_deterministic_and_sensitive() {
        let a = content_key("sha1", "lumencite-pdfium", "0.1.0", "");
        let same = content_key("sha1", "lumencite-pdfium", "0.1.0", "");
        assert_eq!(a, same, "同一入力は同一 content_key");

        let diff_version = content_key("sha1", "lumencite-pdfium", "0.2.0", "");
        assert_ne!(a, diff_version, "extractor_version が変われば別");

        let diff_sha = content_key("sha2", "lumencite-pdfium", "0.1.0", "");
        assert_ne!(a, diff_sha, "source_sha256 が変われば別");

        let diff_cfg = content_key("sha1", "lumencite-pdfium", "0.1.0", "cfg=1");
        assert_ne!(a, diff_cfg, "config_hash が変われば別");
    }

    /// Golden: 手組みの LCIR ドキュメントが、コミット済み fixture と構造一致し、
    /// validation を通ること。抽出器更新で構造が変わったら差分で気付ける（roadmap §13.2）。
    #[test]
    fn golden_minimal_document_matches_fixture() {
        let built = LcirDocument {
            schema: schema::SCHEMA_URI.to_string(),
            schema_version: schema::SCHEMA_VERSION.to_string(),
            version_id: 1,
            content_key: "abc123".to_string(),
            source: LcirSource {
                sha256: "deadbeef".to_string(),
                mime_type: "application/pdf".to_string(),
                extractor_name: schema::EXTRACTOR_NAME.to_string(),
                extractor_version: schema::EXTRACTOR_VERSION.to_string(),
            },
            coordinate_space: Some(CoordinateSpace::default()),
            nodes: vec![
                LcirNode {
                    id: 1,
                    kind: "document".to_string(),
                    ordinal: 0,
                    parent_id: None,
                    plain_text: None,
                    origin: Some("pdf_text_layer".to_string()),
                    confidence: None,
                    payload: None,
                    math: None,
                    source_fragments: vec![],
                },
                LcirNode {
                    id: 2,
                    kind: "page".to_string(),
                    ordinal: 0,
                    parent_id: Some(1),
                    plain_text: Some("Hello world".to_string()),
                    origin: Some("pdf_text_layer".to_string()),
                    confidence: None,
                    payload: None,
                    math: None,
                    source_fragments: vec![LcirFragment {
                        page: 1,
                        bbox: BBox::new(0.0, 0.0, 595.0, 842.0),
                        fragment_type: Some("page".to_string()),
                    }],
                },
            ],
        };
        let fixture: LcirDocument =
            serde_json::from_str(include_str!("testdata/minimal_lcir.json")).unwrap();
        assert_eq!(built, fixture, "手組みの LCIR が golden fixture と一致する");
        assert!(validation::validate(&built).is_ok());
    }

    /// Phase 2 Golden: document > page > (section|paragraph) > line の構造化ツリーが、
    /// コミット済み fixture と一致し validation を通る。見出しの payload（heading_level/
    /// section_number）と confidence が派生ビューに載ることも固定する。
    #[test]
    fn golden_structured_document_matches_fixture() {
        let page_frag = |ft: &str, bbox: BBox| LcirFragment {
            page: 1,
            bbox,
            fragment_type: Some(ft.to_string()),
        };
        let heading_bbox = BBox::new(72.0, 780.0, 120.0, 14.0);
        let para_bbox = BBox::new(72.0, 740.0, 400.0, 12.0);
        let built = LcirDocument {
            schema: schema::SCHEMA_URI.to_string(),
            schema_version: schema::SCHEMA_VERSION.to_string(),
            version_id: 7,
            content_key: "struct123".to_string(),
            source: LcirSource {
                sha256: "deadbeef".to_string(),
                mime_type: "application/pdf".to_string(),
                extractor_name: schema::EXTRACTOR_NAME.to_string(),
                extractor_version: schema::EXTRACTOR_VERSION.to_string(),
            },
            coordinate_space: Some(CoordinateSpace::default()),
            nodes: vec![
                LcirNode {
                    id: 1,
                    kind: "document".to_string(),
                    ordinal: 0,
                    parent_id: None,
                    plain_text: None,
                    origin: Some("pdf_text_layer".to_string()),
                    confidence: None,
                    payload: None,
                    math: None,
                    source_fragments: vec![],
                },
                LcirNode {
                    id: 2,
                    kind: "page".to_string(),
                    ordinal: 0,
                    parent_id: Some(1),
                    plain_text: Some("1 Introduction Deep learning has advanced rapidly.".to_string()),
                    origin: Some("pdf_text_layer".to_string()),
                    confidence: None,
                    payload: None,
                    math: None,
                    source_fragments: vec![page_frag("page", BBox::new(0.0, 0.0, 595.0, 842.0))],
                },
                LcirNode {
                    id: 3,
                    kind: "section".to_string(),
                    ordinal: 0,
                    parent_id: Some(2),
                    plain_text: Some("1 Introduction".to_string()),
                    origin: Some("layout_model".to_string()),
                    confidence: Some(0.75),
                    payload: Some(serde_json::json!({"heading_level": 1, "section_number": "1"})),
                    math: None,
                    source_fragments: vec![page_frag("block", heading_bbox)],
                },
                LcirNode {
                    id: 4,
                    kind: "line".to_string(),
                    ordinal: 0,
                    parent_id: Some(3),
                    plain_text: Some("1 Introduction".to_string()),
                    origin: Some("pdf_text_layer".to_string()),
                    confidence: None,
                    payload: None,
                    math: None,
                    source_fragments: vec![page_frag("line", heading_bbox)],
                },
                LcirNode {
                    id: 5,
                    kind: "paragraph".to_string(),
                    ordinal: 1,
                    parent_id: Some(2),
                    plain_text: Some("Deep learning has advanced rapidly.".to_string()),
                    origin: Some("layout_model".to_string()),
                    confidence: Some(0.6),
                    payload: None,
                    math: None,
                    source_fragments: vec![page_frag("block", para_bbox)],
                },
                LcirNode {
                    id: 6,
                    kind: "line".to_string(),
                    ordinal: 0,
                    parent_id: Some(5),
                    plain_text: Some("Deep learning has advanced rapidly.".to_string()),
                    origin: Some("pdf_text_layer".to_string()),
                    confidence: None,
                    payload: None,
                    math: None,
                    source_fragments: vec![page_frag("line", para_bbox)],
                },
            ],
        };
        let fixture: LcirDocument =
            serde_json::from_str(include_str!("testdata/structured_lcir.json")).unwrap();
        assert_eq!(built, fixture, "構造化 LCIR が golden fixture と一致する");
        assert!(validation::validate(&built).is_ok());
        // 見出しの payload が派生ビューから読める。
        let section = built.nodes.iter().find(|n| n.kind == "section").unwrap();
        assert_eq!(section.payload.as_ref().unwrap()["section_number"], "1");
    }
}
