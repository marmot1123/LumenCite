//! PDF 座標（source_fragments）と座標系記述子。

use serde::{Deserialize, Serialize};

/// PDF user space（左下原点・y 上・単位 pt）のバウンディングボックス。
/// 既存 `highlights` テーブルと同じ座標系（x/y は左下角）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BBox {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// document_version.metadata_json に載せる座標系記述子。将来 top-left / pixel 系の
/// layout model（Phase 2）と混同しないための明示。既定は pdfium ネイティブ空間。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoordinateSpace {
    pub space: String,
    pub origin: String,
    pub unit: String,
    pub y_axis: String,
}

impl Default for CoordinateSpace {
    fn default() -> Self {
        Self {
            space: "pdf_user_space".to_string(),
            origin: "bottom_left".to_string(),
            unit: "pt".to_string(),
            y_axis: "up".to_string(),
        }
    }
}

/// source_fragment の種別（DB の fragment_type 列）。
/// `Block` は Phase 2 の論理ブロック（段落・見出し・caption 等）の統合領域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentType {
    Page,
    Block,
    TextBlock,
    Line,
}

impl FragmentType {
    pub fn as_str(self) -> &'static str {
        match self {
            FragmentType::Page => "page",
            FragmentType::Block => "block",
            FragmentType::TextBlock => "text_block",
            FragmentType::Line => "line",
        }
    }
}
