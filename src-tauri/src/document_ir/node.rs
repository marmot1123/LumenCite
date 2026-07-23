//! LCIR ノードの型（NodeKind/Origin/ExtractionStatus）と、LCIR JSON の派生ビュー型。
//!
//! これらの enum は serde を導出せず、DB の TEXT 列と 1:1 の `as_str`/`from_db` を持つ。
//! JSON 側（`Lcir*`）は String フィールドとして扱うため、未知種別も無損失で往復できる。

use super::source::{BBox, CoordinateSpace};
use serde::{Deserialize, Serialize};

/// 文書ノードの型。
///
/// - **第一段（Phase 1）**: document/page/text_block/line/unknown_block。
/// - **Phase 2（論理構造）**: heading/section/subsection/abstract/paragraph/list/list_item/
///   figure_caption/table_caption/footnote/citation/bibliography/bibliography_entry/
///   code_block/front_matter。
/// - **Phase 3（数式表層）**: inline_math/display_math/equation_group。
/// - **Phase 5（定理・定義・証明）**: definition/theorem/lemma/proposition/corollary/remark/
///   example/proof。
/// - **Phase 8a（図表アセット基盤）**: figure（図領域・PDF 版のみ合成）/ table（enum 予約・
///   8b の表セル構造化で合成）。
///
/// DB は自由 TEXT なので variant 追加に migration は要らない。認識に確信が持てないブロックは
/// 誤った型を確定せず `UnknownBlock` にする（roadmap「欠損を許容」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    // Phase 1: 構造の骨格。
    Document,
    Page,
    TextBlock,
    Line,
    UnknownBlock,
    // Phase 2: 論理構造。
    FrontMatter,
    Abstract,
    Section,
    Subsection,
    Heading,
    Paragraph,
    List,
    ListItem,
    FigureCaption,
    TableCaption,
    Footnote,
    Citation,
    Bibliography,
    BibliographyEntry,
    CodeBlock,
    // Phase 3: 数式表層。
    InlineMath,
    DisplayMath,
    EquationGroup,
    // Phase 5: 定理・定義・証明。
    Definition,
    Theorem,
    Lemma,
    Proposition,
    Corollary,
    Remark,
    Example,
    Proof,
    // Phase 8a: 図表。
    Figure,
    Table,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Document => "document",
            NodeKind::Page => "page",
            NodeKind::TextBlock => "text_block",
            NodeKind::Line => "line",
            NodeKind::UnknownBlock => "unknown_block",
            NodeKind::FrontMatter => "front_matter",
            NodeKind::Abstract => "abstract",
            NodeKind::Section => "section",
            NodeKind::Subsection => "subsection",
            NodeKind::Heading => "heading",
            NodeKind::Paragraph => "paragraph",
            NodeKind::List => "list",
            NodeKind::ListItem => "list_item",
            NodeKind::FigureCaption => "figure_caption",
            NodeKind::TableCaption => "table_caption",
            NodeKind::Footnote => "footnote",
            NodeKind::Citation => "citation",
            NodeKind::Bibliography => "bibliography",
            NodeKind::BibliographyEntry => "bibliography_entry",
            NodeKind::CodeBlock => "code_block",
            NodeKind::InlineMath => "inline_math",
            NodeKind::DisplayMath => "display_math",
            NodeKind::EquationGroup => "equation_group",
            NodeKind::Definition => "definition",
            NodeKind::Theorem => "theorem",
            NodeKind::Lemma => "lemma",
            NodeKind::Proposition => "proposition",
            NodeKind::Corollary => "corollary",
            NodeKind::Remark => "remark",
            NodeKind::Example => "example",
            NodeKind::Proof => "proof",
            NodeKind::Figure => "figure",
            NodeKind::Table => "table",
        }
    }

    /// DB の TEXT から復元。未知種別は `UnknownBlock` にフォールバック（旧バイナリで新種別を
    /// 読む劣化ケース。desktop app はバイナリと migration が同梱なので実際には稀）。
    pub fn from_db(s: &str) -> Self {
        match s {
            "document" => NodeKind::Document,
            "page" => NodeKind::Page,
            "text_block" => NodeKind::TextBlock,
            "line" => NodeKind::Line,
            "front_matter" => NodeKind::FrontMatter,
            "abstract" => NodeKind::Abstract,
            "section" => NodeKind::Section,
            "subsection" => NodeKind::Subsection,
            "heading" => NodeKind::Heading,
            "paragraph" => NodeKind::Paragraph,
            "list" => NodeKind::List,
            "list_item" => NodeKind::ListItem,
            "figure_caption" => NodeKind::FigureCaption,
            "table_caption" => NodeKind::TableCaption,
            "footnote" => NodeKind::Footnote,
            "citation" => NodeKind::Citation,
            "bibliography" => NodeKind::Bibliography,
            "bibliography_entry" => NodeKind::BibliographyEntry,
            "code_block" => NodeKind::CodeBlock,
            "inline_math" => NodeKind::InlineMath,
            "display_math" => NodeKind::DisplayMath,
            "equation_group" => NodeKind::EquationGroup,
            "definition" => NodeKind::Definition,
            "theorem" => NodeKind::Theorem,
            "lemma" => NodeKind::Lemma,
            "proposition" => NodeKind::Proposition,
            "corollary" => NodeKind::Corollary,
            "remark" => NodeKind::Remark,
            "example" => NodeKind::Example,
            "proof" => NodeKind::Proof,
            "figure" => NodeKind::Figure,
            "table" => NodeKind::Table,
            _ => NodeKind::UnknownBlock,
        }
    }
}

/// データの由来。原文由来と AI 推定を常に区別するための列（origin）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    PublisherSource,
    TexSource,
    PdfTextLayer,
    Ocr,
    LayoutModel,
    MathRecognition,
    LlmInference,
    UserEdited,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::PublisherSource => "publisher_source",
            Origin::TexSource => "tex_source",
            Origin::PdfTextLayer => "pdf_text_layer",
            Origin::Ocr => "ocr",
            Origin::LayoutModel => "layout_model",
            Origin::MathRecognition => "math_recognition",
            Origin::LlmInference => "llm_inference",
            Origin::UserEdited => "user_edited",
        }
    }
}

/// document_version の抽出状態。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionStatus {
    Pending,
    Processing,
    Completed,
    CompletedWithWarnings,
    Failed,
    Superseded,
}

impl ExtractionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ExtractionStatus::Pending => "pending",
            ExtractionStatus::Processing => "processing",
            ExtractionStatus::Completed => "completed",
            ExtractionStatus::CompletedWithWarnings => "completed_with_warnings",
            ExtractionStatus::Failed => "failed",
            ExtractionStatus::Superseded => "superseded",
        }
    }

    /// completed / completed_with_warnings のどちらか。
    pub fn is_completed(s: &str) -> bool {
        matches!(s, "completed" | "completed_with_warnings")
    }
}

// ---- LCIR JSON の派生ビュー（export/デバッグ/テスト/交換用。正本は SQLite） ----

/// LCIR JSON のソース記述。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirSource {
    pub sha256: String,
    pub mime_type: String,
    pub extractor_name: String,
    pub extractor_version: String,
}

/// LCIR JSON の source fragment（PDF 上の領域）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirFragment {
    pub page: i64,
    pub bbox: BBox,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fragment_type: Option<String>,
}

/// LCIR JSON のノード（document_nodes + source_fragments を平坦化した派生形）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirNode {
    pub id: i64,
    pub kind: String,
    pub ordinal: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// 型固有の属性（Phase 2 の見出しは `heading_level`/`section_number` 等）。DB の
    /// `payload_json` を透過的にパースしたもの。無ければ省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// 数式表現（Phase 3・inline_math/display_math ノードのみ）。正本は `math_expressions`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub math: Option<super::math::LcirMath>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_fragments: Vec<LcirFragment>,
    /// バイナリアセット参照（Phase 8a・figure ノードのみ）。正本は `assets`/`node_assets`。
    /// `relative_path` はメタデータ参照でファイルの存在は保証しない（欠損許容）。無ければ省略。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<super::asset::LcirAsset>,
}

/// LCIR ドキュメント（派生ビュー）。正本は SQLite の document_versions/nodes/fragments。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirDocument {
    pub schema: String,
    pub schema_version: String,
    pub version_id: i64,
    pub content_key: String,
    pub source: LcirSource,
    /// PDF 由来（pdfium）のみ。TeX 由来（Phase 4）は座標を持たないため省略される。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinate_space: Option<CoordinateSpace>,
    pub nodes: Vec<LcirNode>,
    /// ノード間の型付き関係（Phase 6a・参照グラフ）。辺は特定ノードに属さないので文書レベルに持つ。
    /// 正本は SQLite の `node_relations`。無ければ省略（既存 fixture との後方互換）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<super::relation::LcirRelation>,
    /// 記号定義（Phase 6b・記号系）。定義文から取り出した記号 + 出現。TeX 版のみ。
    /// 正本は SQLite の `symbols`/`symbol_occurrences`。無ければ省略。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<super::symbol::LcirSymbol>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_kind_roundtrips_through_db_string() {
        for k in [
            NodeKind::Document,
            NodeKind::Page,
            NodeKind::TextBlock,
            NodeKind::Line,
            NodeKind::UnknownBlock,
            NodeKind::FrontMatter,
            NodeKind::Abstract,
            NodeKind::Section,
            NodeKind::Subsection,
            NodeKind::Heading,
            NodeKind::Paragraph,
            NodeKind::List,
            NodeKind::ListItem,
            NodeKind::FigureCaption,
            NodeKind::TableCaption,
            NodeKind::Footnote,
            NodeKind::Citation,
            NodeKind::Bibliography,
            NodeKind::BibliographyEntry,
            NodeKind::CodeBlock,
            NodeKind::InlineMath,
            NodeKind::DisplayMath,
            NodeKind::EquationGroup,
            NodeKind::Definition,
            NodeKind::Theorem,
            NodeKind::Lemma,
            NodeKind::Proposition,
            NodeKind::Corollary,
            NodeKind::Remark,
            NodeKind::Example,
            NodeKind::Proof,
            NodeKind::Figure,
            NodeKind::Table,
        ] {
            assert_eq!(NodeKind::from_db(k.as_str()), k);
        }
        // Phase 2/3/5 の snake_case が期待どおり（DB 列・LCIR JSON と 1:1）。
        assert_eq!(NodeKind::FigureCaption.as_str(), "figure_caption");
        assert_eq!(NodeKind::BibliographyEntry.as_str(), "bibliography_entry");
        assert_eq!(NodeKind::DisplayMath.as_str(), "display_math");
        // Phase 5: 定理系は from_db で解決する。
        assert_eq!(NodeKind::from_db("theorem"), NodeKind::Theorem);
        assert_eq!(NodeKind::from_db("proof"), NodeKind::Proof);
        // Phase 8a: 図・表も from_db で解決する。
        assert_eq!(NodeKind::from_db("figure"), NodeKind::Figure);
        assert_eq!(NodeKind::from_db("table"), NodeKind::Table);
        // 未実装種別（8b の表セル等）は引き続き UnknownBlock にフォールバック。
        assert_eq!(NodeKind::from_db("table_cell"), NodeKind::UnknownBlock);
    }

    #[test]
    fn origin_and_status_strings_are_snake_case() {
        assert_eq!(Origin::PdfTextLayer.as_str(), "pdf_text_layer");
        assert_eq!(Origin::LlmInference.as_str(), "llm_inference");
        assert_eq!(
            ExtractionStatus::CompletedWithWarnings.as_str(),
            "completed_with_warnings"
        );
        assert!(ExtractionStatus::is_completed("completed"));
        assert!(ExtractionStatus::is_completed("completed_with_warnings"));
        assert!(!ExtractionStatus::is_completed("failed"));
    }

    #[test]
    fn lcir_document_serde_roundtrips() {
        let doc = LcirDocument {
            schema: super::super::schema::SCHEMA_URI.to_string(),
            schema_version: "0.1.0".to_string(),
            version_id: 7,
            content_key: "abc".to_string(),
            source: LcirSource {
                sha256: "deadbeef".to_string(),
                mime_type: "application/pdf".to_string(),
                extractor_name: "lumencite-pdfium".to_string(),
                extractor_version: "0.1.0".to_string(),
            },
            coordinate_space: Some(CoordinateSpace::default()),
            nodes: vec![LcirNode {
                id: 2,
                kind: "page".to_string(),
                ordinal: 0,
                parent_id: Some(1),
                plain_text: Some("hi".to_string()),
                origin: Some("pdf_text_layer".to_string()),
                confidence: None,
                payload: None,
                math: None,
                source_fragments: vec![LcirFragment {
                    page: 1,
                    bbox: BBox::new(0.0, 0.0, 595.0, 842.0),
                    fragment_type: Some("page".to_string()),
                }],
                assets: vec![],
            }],
            relations: vec![],
            symbols: vec![],
        };
        let json = serde_json::to_string(&doc).unwrap();
        let back: LcirDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(doc, back);
    }
}
