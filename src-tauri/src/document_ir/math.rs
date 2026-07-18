//! 数式の表層表現（Phase 3）。数式は「単一形式に統一しない」方針で、複数表現を併存させる。
//!
//! PDF 由来では LaTeX/MathML は確実に復元できない（グリフの羅列）ため、第一歩は **surface**
//! （正規化した Unicode 線形文字列）だけを持ち、`semantic_status = surface_only` にする。
//! 本物の LaTeX は Phase 4（TeX 取込）、Content MathML/OpenMath/AST は Phase 7（意味）で埋める。
//! **原文由来と推定を必ず区別する**ため `origin` と `confidence` を持たせる。

use serde::{Deserialize, Serialize};

/// 数式の表示モード（display=別行の独立式 / inline=本文中）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathDisplayMode {
    Inline,
    Display,
}

impl MathDisplayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            MathDisplayMode::Inline => "inline",
            MathDisplayMode::Display => "display",
        }
    }
}

/// 数式の意味的な確からしさ（roadmap §5.4）。表層のみか、推定か、原資料由来か等を区別する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathSemanticStatus {
    /// まだ意味解析を試みていない。
    NotAttempted,
    /// 表層（LaTeX/線形文字列）はあるが意味は未確定。PDF 抽出の既定。
    SurfaceOnly,
    /// AI 等で意味を推定した（人手未確認）。
    Inferred,
    /// 人手または検証で確認済み。
    Verified,
    /// 原資料（TeX/MathML）がその表現を直接提供している。
    SourceProvided,
}

impl MathSemanticStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MathSemanticStatus::NotAttempted => "not_attempted",
            MathSemanticStatus::SurfaceOnly => "surface_only",
            MathSemanticStatus::Inferred => "inferred",
            MathSemanticStatus::Verified => "verified",
            MathSemanticStatus::SourceProvided => "source_provided",
        }
    }
}

/// LCIR JSON の派生ビューに載せる数式表現（`document_nodes` の inline_math/display_math ノードに
/// 付く）。正本は SQLite の `math_expressions`。埋まっていない表現は省略される。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirMath {
    pub display_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equation_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation_mathml: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_mathml: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openmath: Option<String>,
    /// 検索用の正規化線形文字列（PDF 表層のクリーンな Unicode 表現）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_text: Option<String>,
    pub semantic_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_strings_match_roadmap() {
        assert_eq!(MathDisplayMode::Display.as_str(), "display");
        assert_eq!(MathDisplayMode::Inline.as_str(), "inline");
        assert_eq!(MathSemanticStatus::SurfaceOnly.as_str(), "surface_only");
        assert_eq!(MathSemanticStatus::SourceProvided.as_str(), "source_provided");
    }

    #[test]
    fn lcir_math_omits_empty_representations() {
        let m = LcirMath {
            display_mode: "display".to_string(),
            equation_label: Some("(2.1)".to_string()),
            latex: None,
            presentation_mathml: None,
            content_mathml: None,
            openmath: None,
            normalized_text: Some("U = S2 C2 S1 C1".to_string()),
            semantic_status: "surface_only".to_string(),
            confidence: Some(0.6),
            origin: Some("pdf_text_layer".to_string()),
        };
        let json = serde_json::to_string(&m).unwrap();
        // 未確定の表現（latex/mathml 等）はキーごと省略される。
        assert!(!json.contains("latex"));
        assert!(!json.contains("presentation_mathml"));
        assert!(json.contains("normalized_text"));
        let back: LcirMath = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }
}
