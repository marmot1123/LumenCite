//! PDF → LCIR の生データ抽出（pdfium・同期）。ページ全文・text_block セグメント・座標を取る。
//! Phase 8a: ページ内の埋込画像（トップレベル Image オブジェクト）から図領域を検出し、
//! ページレンダリングの crop PNG を `asset_dir` へ原子的に書き出す。
//! native lib 依存のため呼び出しは `spawn_blocking` 下で。座標は PDF user space（左下原点・pt）。

pub mod pdfium;

use crate::document_ir::BBox;
use crate::ingestion::figures;

/// ページレンダリングの目標幅（px）。OCR（`llm/tools/ocr.rs`）と同値。
/// 変更は crop の見た目だけでなく assets の再現性に効くので extractor_version と併せて上げる。
pub const RENDER_TARGET_WIDTH: i32 = 1600;

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
    /// 図領域（Phase 8a・埋込画像 bbox のマージ結果）。`asset_dir` 無しの呼び出しでは常に空。
    pub image_regions: Vec<ExtractedImageRegion>,
}

/// text_block 1 個（テキスト + PDF 上の bbox）。
pub struct ExtractedBlock {
    pub text: String,
    pub bbox: BBox,
    pub reading_order: i64,
}

/// 図領域 1 個（Phase 8a）。bbox は PDF user space（左下原点・pt）。
pub struct ExtractedImageRegion {
    pub bbox: BBox,
    /// 書き出した crop PNG。レンダリング/書き込み失敗時は None（warning 済み・欠損許容）。
    pub file: Option<ExtractedAssetFile>,
}

/// 書き出した crop PNG のメタデータ（ファイル本体は `asset_dir` 直下）。
pub struct ExtractedAssetFile {
    /// `asset_dir` 相対のファイル名（`fig-p003-00.png`）。
    pub file_name: String,
    pub width_px: u32,
    pub height_px: u32,
    pub sha256: String,
    pub size_bytes: u64,
}

/// PDF 1 件分の抽出結果。
pub struct ExtractedDocument {
    pub pages: Vec<ExtractedPage>,
    /// ページ単位の抽出失敗など、致命的でない警告。
    pub warnings: Vec<String>,
}

/// PDF を pdfium で抽出する。同期・CPU/native 依存なので `spawn_blocking` 下で呼ぶこと。
///
/// `asset_dir` を渡すと図領域の crop PNG をそのディレクトリへ書き出す（Phase 8a）。
/// ファイルは決定的な名前（`fig-p{page:03}-{idx:02}.png`）で tmp+rename の原子的
/// パターンで書く（同一 content_key の再抽出は同一パスへの上書き＝冪等）。
pub fn extract_document(
    path: &std::path::Path,
    asset_dir: Option<&std::path::Path>,
) -> Result<ExtractedDocument, String> {
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
                    image_regions: Vec::new(),
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

        let image_regions = match asset_dir {
            Some(dir) => extract_page_image_regions(
                &page,
                page_number,
                width_pt,
                height_pt,
                rotation_deg,
                dir,
                &mut warnings,
            ),
            None => Vec::new(),
        };

        pages.push(ExtractedPage {
            page_number,
            width_pt,
            height_pt,
            rotation_deg,
            plain_text,
            blocks,
            image_regions,
        });
    }

    Ok(ExtractedDocument { pages, warnings })
}

/// 1 ページの図領域を検出し、crop PNG を書き出す（Phase 8a）。
///
/// - **トップレベルの Image オブジェクトのみ**列挙する。XObjectForm 内の画像は追わない
///   （子 bounds が form ローカル座標で返り、ページ内に収まる平行移動はガードを素通りして
///   「誤配置 crop」を生むため。欠損 > 誤り）。
/// - 回転ページ（`/Rotate` ≠ 0）は座標変換の検証ができないためスキップする。
/// - 個別の失敗は warning + 欠損で継続し、build 全体は止めない。
fn extract_page_image_regions(
    page: &pdfium_render::prelude::PdfPage<'_>,
    page_number: i64,
    width_pt: f64,
    height_pt: f64,
    rotation_deg: f64,
    asset_dir: &std::path::Path,
    warnings: &mut Vec<String>,
) -> Vec<ExtractedImageRegion> {
    use pdfium_render::prelude::*;

    // 1. トップレベル Image オブジェクトの bbox を集める。
    let mut rects: Vec<BBox> = Vec::new();
    let mut raw_count = 0usize;
    for object in page.objects().iter() {
        if object.as_image_object().is_none() {
            continue;
        }
        raw_count += 1;
        if raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
            continue; // 数だけ数えて後で警告（スライス化ラスタ）。
        }
        let Ok(quad) = object.bounds() else { continue };
        let r = quad.to_rect();
        let x = r.left().value as f64;
        let y = r.bottom().value as f64;
        let w = (r.right().value - r.left().value) as f64;
        let h = (r.top().value - r.bottom().value) as f64;
        if w <= 0.0 || h <= 0.0 {
            continue;
        }
        // ページ矩形へクランプ。大きく食み出す矩形（変換異常の兆候）は捨てる（誤配置 crop 回避）。
        let cx0 = x.max(0.0);
        let cy0 = y.max(0.0);
        let cx1 = (x + w).min(width_pt);
        let cy1 = (y + h).min(height_pt);
        let cw = (cx1 - cx0).max(0.0);
        let ch = (cy1 - cy0).max(0.0);
        if cw * ch < 0.5 * w * h {
            continue;
        }
        rects.push(BBox::new(cx0, cy0, cw, ch));
    }
    if rects.is_empty() {
        return Vec::new();
    }
    if rotation_deg != 0.0 {
        warnings.push(format!(
            "page {page_number}: rotated page ({rotation_deg} deg); figure regions skipped"
        ));
        return Vec::new();
    }
    if raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
        warnings.push(format!(
            "page {page_number}: too many image objects ({raw_count}); figure regions skipped"
        ));
        return Vec::new();
    }

    // 2. フィルタ + マージで図領域へ。
    let merged = figures::merge_image_regions(&rects, width_pt, height_pt);
    if merged.is_empty() {
        return Vec::new();
    }

    // 3. ページ境界 box の原点（CropBox が (0,0) 始まりでない雑誌 PDF の補正）。
    let (box_left, box_bottom) = page_box_origin(page);

    // 4. ページ全体を 1 回レンダリングし、各領域を crop する（`clip()` はビットマップを
    //    縮めないため使わない）。失敗はページ単位の warning + アセット無し領域で継続。
    if let Err(e) = std::fs::create_dir_all(asset_dir) {
        warnings.push(format!(
            "page {page_number}: asset dir creation failed: {e}; figure assets skipped"
        ));
        return merged
            .into_iter()
            .map(|bbox| ExtractedImageRegion { bbox, file: None })
            .collect();
    }
    let config = PdfRenderConfig::new().set_target_width(RENDER_TARGET_WIDTH);
    let img = match page.render_with_config(&config) {
        Ok(bitmap) => bitmap.as_image(),
        Err(e) => {
            warnings.push(format!(
                "page {page_number}: page render failed: {e}; figure assets skipped"
            ));
            return merged
                .into_iter()
                .map(|bbox| ExtractedImageRegion { bbox, file: None })
                .collect();
        }
    };

    let mut regions = Vec::new();
    for (i, bbox) in merged.into_iter().enumerate() {
        let Some((px, py, pw, ph)) = figures::region_to_pixel_rect(
            bbox,
            box_left,
            box_bottom,
            width_pt,
            height_pt,
            img.width(),
            img.height(),
        ) else {
            // クランプで潰れた領域は図として作らない（誤検出より欠損）。
            continue;
        };
        let crop = img.crop_imm(px, py, pw, ph);
        let mut buf = Vec::new();
        let file = match crop.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        {
            Ok(()) => {
                let file_name = format!("fig-p{page_number:03}-{i:02}.png");
                match write_atomic(asset_dir, &file_name, &buf) {
                    Ok(()) => Some(ExtractedAssetFile {
                        file_name,
                        width_px: crop.width(),
                        height_px: crop.height(),
                        sha256: crate::document_ir::sha256_hex(&buf),
                        size_bytes: buf.len() as u64,
                    }),
                    Err(e) => {
                        warnings.push(format!(
                            "page {page_number}: figure asset write failed: {e}"
                        ));
                        None
                    }
                }
            }
            Err(e) => {
                warnings.push(format!("page {page_number}: PNG encode failed: {e}"));
                None
            }
        };
        regions.push(ExtractedImageRegion { bbox, file });
    }
    regions
}

/// ページ境界 box（CropBox → MediaBox の順）の原点。取れなければ (0,0)（大半の PDF は原点ゼロ）。
fn page_box_origin(page: &pdfium_render::prelude::PdfPage<'_>) -> (f64, f64) {
    let boundaries = page.boundaries();
    let rect = boundaries
        .crop()
        .map(|b| b.bounds)
        .or_else(|_| boundaries.media().map(|b| b.bounds));
    match rect {
        Ok(r) => (r.left().value as f64, r.bottom().value as f64),
        Err(_) => (0.0, 0.0),
    }
}

/// tmp 名に書いて `sync_all` → rename の原子的書き込み。並行ビルドの truncate 窓と
/// 電源断の torn file を防ぐ（rename は同一ディレクトリ内なので原子的）。
fn write_atomic(
    dir: &std::path::Path,
    file_name: &str,
    bytes: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = dir.join(format!("{file_name}.tmp"));
    let dest = dir.join(file_name);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, &dest) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}
