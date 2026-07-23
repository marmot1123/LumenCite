//! LCIR 図表アセット（Phase 8a・`assets`/`node_assets`）の DB 非依存な型。
//!
//! バイナリ本体はファイルシステム（`attachments/<entry_id>/.lcir/` 配下）に置き、ここは
//! 相対パス + SHA-256 の**メタデータ参照**だけを持つ。`relative_path` はファイルの存在を
//! 保証しない（欠損許容 — 読み手はファイル欠損に耐えること）。図領域はレイアウト推定
//! （`origin='layout_model'`）なので、ノード側の confidence で原文由来と区別する。

use serde::{Deserialize, Serialize};

/// LCIR JSON の派生ビューに載せるアセット参照（正本は SQLite の `assets`/`node_assets`）。
/// ノード（`figure` 等）に `role` 付きでぶら下がる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LcirAsset {
    /// ノードとの関係（8a は `page_crop` のみ。将来 original/vector/thumbnail/...）。
    pub role: String,
    pub mime_type: String,
    /// app data dir 相対・`/` 区切り。存在保証なしのメタデータ参照。
    pub relative_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    pub sha256: String,
    /// 未モデル化の属性（`{page, region_index, render_target_width}` 等）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcir_asset_serde_roundtrips() {
        let a = LcirAsset {
            role: "page_crop".to_string(),
            mime_type: "image/png".to_string(),
            relative_path: "attachments/1/.lcir/2/deadbeef/fig-p003-00.png".to_string(),
            width: Some(800),
            height: Some(600),
            size_bytes: Some(12345),
            sha256: "abc".to_string(),
            metadata: Some(serde_json::json!({"page": 3, "region_index": 0})),
        };
        let json = serde_json::to_string(&a).unwrap();
        let back: LcirAsset = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn optional_fields_are_omitted_when_absent() {
        let a = LcirAsset {
            role: "page_crop".to_string(),
            mime_type: "image/png".to_string(),
            relative_path: "attachments/1/.lcir/2/deadbeef/fig-p001-00.png".to_string(),
            width: None,
            height: None,
            size_bytes: None,
            sha256: "abc".to_string(),
            metadata: None,
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("width"));
        assert!(!json.contains("metadata"));
    }
}
