//! LCIR 記号系（Phase 6b・`symbols`/`symbol_occurrences`）の DB 非依存な型。
//!
//! 論文が定義する記号（"let $U$ be ...", "$H := ...$"）とその出現を持つ。surface_form/description
//! は TeX 本文の verbatim（`origin='tex_source'`）だが、「この文がこの記号を定義している」という
//! 対応づけはヒューリスティック推定なので `confidence`（検出の確からしさ）で区別する。JSON 側
//! （`Lcir*`）は String として未知種別も無損失で往復できる（`NodeKind`/`RelationType` と同型）。

use serde::{Deserialize, Serialize};

/// 記号の型（推定・任意）。定義文中の語から best-effort で分類する。DB は自由 TEXT。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolType {
    Operator,
    Matrix,
    Set,
    Group,
    Space,
    Function,
    Map,
    Vector,
    Field,
    Graph,
    Constant,
    Number,
}

impl SymbolType {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolType::Operator => "operator",
            SymbolType::Matrix => "matrix",
            SymbolType::Set => "set",
            SymbolType::Group => "group",
            SymbolType::Space => "space",
            SymbolType::Function => "function",
            SymbolType::Map => "map",
            SymbolType::Vector => "vector",
            SymbolType::Field => "field",
            SymbolType::Graph => "graph",
            SymbolType::Constant => "constant",
            SymbolType::Number => "number",
        }
    }
}

/// LCIR JSON の派生ビューに載せる記号定義（正本は SQLite の `symbols`）。出現は `occurrences` に
/// 平坦化して持つ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirSymbol {
    pub id: i64,
    pub surface_form: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_form: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defined_at_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_node_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// この記号の出現（現状は display 数式ノード内の表層一致）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrences: Vec<LcirSymbolOccurrence>,
}

/// 記号の出現（`symbol_occurrences` の派生ビュー）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirSymbolOccurrence {
    pub node_id: i64,
    pub surface_form: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_type_strings_are_snake_case() {
        assert_eq!(SymbolType::Operator.as_str(), "operator");
        assert_eq!(SymbolType::Graph.as_str(), "graph");
    }

    #[test]
    fn lcir_symbol_serde_roundtrips() {
        let s = LcirSymbol {
            id: 3,
            surface_form: "U".to_string(),
            normalized_form: Some("U".to_string()),
            description: Some("the time evolution operator".to_string()),
            symbol_type: Some("operator".to_string()),
            defined_at_node_id: Some(12),
            scope_node_id: Some(5),
            confidence: Some(0.6),
            origin: Some("tex_source".to_string()),
            occurrences: vec![LcirSymbolOccurrence {
                node_id: 20,
                surface_form: "U".to_string(),
                confidence: Some(0.5),
                origin: Some("tex_source".to_string()),
            }],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: LcirSymbol = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
