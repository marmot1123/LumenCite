//! PDF → LCIR の生データ抽出（pdfium・同期）。ページ全文・text_block セグメント・座標を取る。
//! native lib 依存のため呼び出しは `spawn_blocking` 下で。座標は PDF user space（左下原点・pt）。

pub mod pdfium;

use crate::document_ir::BBox;

/// 1 ページの抽出結果。
pub struct ExtractedPage {
    /// 1 始まりのページ番号。
    pub page_number: i64,
    pub width_pt: f64,
    pub height_pt: f64,
    pub rotation_deg: f64,
    /// ページ全文（FTS 再生成元）。
    pub plain_text: String,
    /// text_block（pdfium のテキストセグメント）。
    pub blocks: Vec<ExtractedBlock>,
}

/// text_block 1 個（テキスト + PDF 上の bbox）。
pub struct ExtractedBlock {
    pub text: String,
    pub bbox: BBox,
    pub reading_order: i64,
}

/// PDF 1 件分の抽出結果。
pub struct ExtractedDocument {
    pub pages: Vec<ExtractedPage>,
    /// ページ単位の抽出失敗など、致命的でない警告。
    pub warnings: Vec<String>,
}

/// PDF を pdfium で抽出する。同期・CPU/native 依存なので `spawn_blocking` 下で呼ぶこと。
pub fn extract_document(path: &std::path::Path) -> Result<ExtractedDocument, String> {
    use pdfium_render::prelude::*;

    let bindings = pdfium::bind_pdfium()?;
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| format!("failed to open PDF: {e}"))?;

    let mut pages = Vec::new();
    let mut warnings = Vec::new();

    for (idx, page) in doc.pages().iter().enumerate() {
        let page_number = idx as i64 + 1;
        let width_pt = page.width().value as f64;
        let height_pt = page.height().value as f64;
        let rotation_deg = page.rotation().map_or(0.0, |r| r.as_degrees() as f64);

        let text = match page.text() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("page {page_number}: text extraction failed: {e}"));
                pages.push(ExtractedPage {
                    page_number,
                    width_pt,
                    height_pt,
                    rotation_deg,
                    plain_text: String::new(),
                    blocks: Vec::new(),
                });
                continue;
            }
        };

        let plain_text = text.all();
        let mut blocks = Vec::new();
        for (i, segment) in text.segments().iter().enumerate() {
            let s = segment.text();
            if s.trim().is_empty() {
                continue;
            }
            // pdfium の bounds は PDF user space（左下原点・pt）。BBox は左下角 + 幅高さ。
            let r = segment.bounds();
            let x = r.left().value as f64;
            let y = r.bottom().value as f64;
            let width = (r.right().value - r.left().value) as f64;
            let height = (r.top().value - r.bottom().value) as f64;
            blocks.push(ExtractedBlock {
                text: s,
                bbox: BBox::new(x, y, width, height),
                reading_order: i as i64,
            });
        }

        pages.push(ExtractedPage {
            page_number,
            width_pt,
            height_pt,
            rotation_deg,
            plain_text,
            blocks,
        });
    }

    Ok(ExtractedDocument { pages, warnings })
}
