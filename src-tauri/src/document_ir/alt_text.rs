//! LCIR 図の代替テキスト（Phase 8c・`node_alt_texts`）の DB 非依存な型。
//!
//! alt text は**原資料に無い生成物**なので、`origin`（`llm_inference` / `user_edited`）・
//! `confidence`・`model`（生成モデル名）・`source_asset_sha256`（説明した画像の指紋）を必ず
//! 併記し、原文 caption（`figure_caption` ノード）とは別フィールドで併存させる（roadmap §16）。

use serde::{Deserialize, Serialize};

/// LCIR JSON の派生ビューに載せる代替テキスト（正本は SQLite の `node_alt_texts`）。
/// `figure` ノードに 1 つだけぶら下がる（手編集がある場合はそちらが優先される）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirAltText {
    pub text: String,
    /// `llm_inference`（LLM Vision 生成）/ `user_edited`（手編集）。
    pub origin: String,
    /// AI 推定であることの表明（意味の正しさの尺度ではない）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// 生成に使ったモデル名（手編集では None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 説明した crop PNG の SHA-256（どの画像を見た説明かの provenance）。
    pub source_asset_sha256: String,
    /// 指紋一致で引き継いだ場合の由来版（None = この版で生成/編集された）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carried_from_version_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcir_alt_text_serde_roundtrips() {
        let a = LcirAltText {
            text: "Line plot of fidelity versus time for three coupling strengths.".to_string(),
            origin: "llm_inference".to_string(),
            confidence: Some(0.5),
            model: Some("gpt-4o-mini".to_string()),
            source_asset_sha256: "deadbeef".to_string(),
            carried_from_version_id: Some(41),
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: LcirAltText = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    /// 生成でない（手編集）場合は model/confidence/carry が省略され、origin で由来が分かる。
    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let a = LcirAltText {
            text: "Hand written description.".to_string(),
            origin: "user_edited".to_string(),
            confidence: None,
            model: None,
            source_asset_sha256: "abc".to_string(),
            carried_from_version_id: None,
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("model"));
        assert!(!json.contains("confidence"));
        assert!(!json.contains("carried_from_version_id"));
        assert!(json.contains("user_edited"));
    }
}
