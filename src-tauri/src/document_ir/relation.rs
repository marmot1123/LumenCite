//! LCIR ノード間の型付き関係（Phase 6a・`node_relations`）の DB 非依存な型。
//!
//! ノードは相互に参照し合う（本文が定理・数式・図・文献を参照する / 証明が定理を証明する）。
//! これを有向辺 `(from_node, relation_type, to_node)` として持つ。`NodeKind`/`Origin` と同じく
//! serde を導出せず DB の TEXT と 1:1 の `as_str` を持ち、JSON 側（`LcirRelation`）は String
//! として扱うため未知種別も無損失で往復できる。

use serde::{Deserialize, Serialize};

/// ノード間の関係の型（roadmap §5.7）。参照グラフ（Phase 6a）で実際に張るのは
/// `cites` / `refers_to_*` / `proves`。残り（`defines_symbol`/`uses_symbol` は Phase 6b、
/// `depends_on` 等は後続フェーズ）は語彙として先に持たせておく（DB は自由 TEXT なので variant
/// 追加に migration は要らない）。
///
/// roadmap のリストに無い `refers_to_theorem`/`refers_to_section`/`refers_to`（一般）は、`\ref`
/// 先ノードの種別に応じて張り分けるために足した拡張。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationType {
    /// 本文 → 参考文献エントリ（TeX `\cite{key}` → `\bibitem{key}`）。
    Cites,
    /// 本文 → 数式（`\eqref`/`\ref` の数式 label、または PDF "Eq. (2.1)" → 数式番号）。
    RefersToEquation,
    /// 本文 → 図（TeX `\ref{fig:..}`）。
    RefersToFigure,
    /// 本文 → 表（TeX `\ref{tab:..}`）。
    RefersToTable,
    /// 本文 → 定理系ノード（TeX `\ref{thm:..}`、または PDF "Theorem 2.3" → 定理番号）。
    RefersToTheorem,
    /// 本文 → 節（TeX `\ref{sec:..}`）。
    RefersToSection,
    /// 上記のどれにも当てはまらない一般の相互参照（`\ref` 先の種別が特定カテゴリでないとき）。
    RefersTo,
    /// 証明 → それが証明する定理系ノード（proof → 直前の定理 / 番号一致）。
    Proves,
    // ── 以降は後続フェーズで使う語彙（Phase 6a では張らない） ──
    /// 本文/数式 → 記号定義（Phase 6b）。
    DefinesSymbol,
    /// 記号使用 → 記号定義（Phase 6b）。
    UsesSymbol,
    /// 依存関係（ある定理が別の定理に依存する等）。
    DependsOn,
    /// caption → 対象の図表。
    CaptionOf,
    /// 脚注 → 対象。
    FootnoteOf,
    /// 継続（分割された同一論理単位）。
    Continues,
    /// 同一対象の別表現（例: PDF 版と TeX 版の同一数式）。
    AlternativeRepresentationOf,
}

impl RelationType {
    pub fn as_str(self) -> &'static str {
        match self {
            RelationType::Cites => "cites",
            RelationType::RefersToEquation => "refers_to_equation",
            RelationType::RefersToFigure => "refers_to_figure",
            RelationType::RefersToTable => "refers_to_table",
            RelationType::RefersToTheorem => "refers_to_theorem",
            RelationType::RefersToSection => "refers_to_section",
            RelationType::RefersTo => "refers_to",
            RelationType::Proves => "proves",
            RelationType::DefinesSymbol => "defines_symbol",
            RelationType::UsesSymbol => "uses_symbol",
            RelationType::DependsOn => "depends_on",
            RelationType::CaptionOf => "caption_of",
            RelationType::FootnoteOf => "footnote_of",
            RelationType::Continues => "continues",
            RelationType::AlternativeRepresentationOf => "alternative_representation_of",
        }
    }
}

/// LCIR JSON の派生ビューに載せるノード間関係（正本は SQLite の `node_relations`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirRelation {
    pub from_node_id: i64,
    pub relation_type: String,
    pub to_node_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// 生の参照文字列・突き合わせたキー/番号など（`node_relations.metadata_json` を透過パース）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_type_strings_are_snake_case() {
        assert_eq!(RelationType::Cites.as_str(), "cites");
        assert_eq!(RelationType::RefersToEquation.as_str(), "refers_to_equation");
        assert_eq!(RelationType::RefersToTheorem.as_str(), "refers_to_theorem");
        assert_eq!(RelationType::Proves.as_str(), "proves");
        assert_eq!(
            RelationType::AlternativeRepresentationOf.as_str(),
            "alternative_representation_of"
        );
    }

    #[test]
    fn lcir_relation_serde_roundtrips() {
        let r = LcirRelation {
            from_node_id: 5,
            relation_type: "cites".to_string(),
            to_node_id: 9,
            confidence: Some(0.9),
            origin: Some("tex_source".to_string()),
            metadata: Some(serde_json::json!({"cite_key": "smith2020"})),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: LcirRelation = serde_json::from_str(&json).unwrap();
        assert_eq!(r, back);
    }
}
