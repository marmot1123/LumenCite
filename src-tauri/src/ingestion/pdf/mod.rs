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

    // 0. ページ境界 box の原点（CropBox が (0,0) 始まりでない雑誌 PDF の補正）。
    //    `width_pt`/`height_pt` はこの box の**寸法**でしかないので、クランプ（1.）も
    //    ピクセル変換（3.）もこの原点を基準にしないと座標系がずれる（debt-14）。
    let (box_left, box_bottom) = page_box_origin(page);

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
        // ページ境界 box へクランプ。大きく食み出す矩形（変換異常の兆候）は捨てる（誤配置 crop 回避）。
        let Some(clamped) = figures::clamp_rect_to_page_box(
            BBox::new(x, y, w, h),
            box_left,
            box_bottom,
            width_pt,
            height_pt,
        ) else {
            continue;
        };
        rects.push(clamped);
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

    // 3. ページ全体を 1 回レンダリングし、各領域を crop する（`clip()` はビットマップを
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

/// ページ境界 box（`CropBox ∩ MediaBox`）の原点。取れなければ (0,0)。
///
/// **まず `bounding()`（`FPDF_GetPageBoundingBox` → `CPDF_Page::GetBBox`）に聞く。**
/// これは pdfium が `page.width()`/`height()` とレンダリングに使う矩形そのもので、
/// ①`/Pages` からの box の継承 ②空の `/CropBox` の無視 ③CropBox と MediaBox の交差 を
/// すべて pdfium 側で解決済み。対して `FPDFPage_GetCropBox`/`GetMediaBox` は**ページ辞書しか
/// 読まない**ので、継承された box を持つ PDF では両方とも取得失敗になる（実ライブラリにも
/// 3 添付 37 頁ある。今はその継承 box の原点がたまたま (0,0) なので実害が出ていないだけ）。
/// crate の doc コメントは「内容の外接矩形」と書いているが誤り ── 実測で生存 138 版 7,345 頁
/// すべてについて、寸法が `page.width()`/`height()` と一致し、原点が
/// `CropBox ∩ MediaBox` の原点と一致した（食い違い 0 頁）。
///
/// 取れなかったときだけ 2 つの box から組む（[`figures::effective_page_box_origin`]）。
fn page_box_origin(page: &pdfium_render::prelude::PdfPage<'_>) -> (f64, f64) {
    let boundaries = page.boundaries();
    // box の 4 値は PDF 仕様上「順序は意味を持たない」ので左下角は成分ごとの min で取る。
    let origin_of = |r: pdfium_render::prelude::PdfRect| {
        (
            r.left().value.min(r.right().value) as f64,
            r.bottom().value.min(r.top().value) as f64,
        )
    };
    if let Ok(b) = boundaries.bounding() {
        return origin_of(b.bounds);
    }
    figures::effective_page_box_origin(
        boundaries.crop().ok().map(|b| origin_of(b.bounds)),
        boundaries.media().ok().map(|b| origin_of(b.bounds)),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 手動 pdfium 実機確認: 実 PDF 1 本の図領域を、**ページ境界 box の原点を効かせた場合と
    /// 落とした場合**の両方で数えて突き合わせる（原点 (0,0) で同じ純関数を呼べば debt-14
    /// 修正前の挙動になる）。再構築せずに実データで効果と副作用を測るための道具で、
    /// 8d-8 / 8d-2 が矩形を足したときの回帰確認にもそのまま使える。
    ///
    /// アセットは書き出さない（DB にも app data dir にも触れない・PDF を開くだけ）。
    ///
    /// `LCIR_FIG_PDF=/path/to.pdf cargo test --lib figure_regions_real_pdf -- --ignored --nocapture`
    #[test]
    #[ignore = "manual pdfium smoke test; needs LCIR_FIG_PDF + libpdfium"]
    fn figure_regions_real_pdf() {
        use pdfium_render::prelude::*;

        let Ok(path) = std::env::var("LCIR_FIG_PDF") else {
            eprintln!("skip: set LCIR_FIG_PDF=/path/to.pdf");
            return;
        };
        let bindings = pdfium::bind_pdfium().expect("libpdfium");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium.load_pdf_from_file(&path, None).expect("open PDF");

        let (mut pages, mut nonzero_origin_pages, mut skipped) = (0usize, 0usize, 0usize);
        let (mut regions_new, mut regions_old) = (0usize, 0usize);
        let (mut changed, mut added, mut removed) = (0usize, 0usize, 0usize);

        for (idx, page) in doc.pages().iter().enumerate() {
            pages += 1;
            let page_number = idx + 1;
            let w = page.width().value as f64;
            let h = page.height().value as f64;
            if page.rotation().map_or(0.0, |r| r.as_degrees() as f64) != 0.0 {
                skipped += 1; // 回転頁は本番でも図領域を作らない。
                continue;
            }
            let (box_left, box_bottom) = page_box_origin(&page);
            let raw_crop = page
                .boundaries()
                .crop()
                .ok()
                .map(|b| (b.bounds.left().value as f64, b.bounds.bottom().value as f64));
            if box_left != 0.0 || box_bottom != 0.0 {
                nonzero_origin_pages += 1;
            }

            let (mut rects_new, mut rects_old, mut raw_count) = (Vec::new(), Vec::new(), 0usize);
            for object in page.objects().iter() {
                if object.as_image_object().is_none() {
                    continue;
                }
                raw_count += 1;
                if raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
                    continue;
                }
                let Ok(quad) = object.bounds() else { continue };
                let r = quad.to_rect();
                let rect = BBox::new(
                    r.left().value as f64,
                    r.bottom().value as f64,
                    (r.right().value - r.left().value) as f64,
                    (r.top().value - r.bottom().value) as f64,
                );
                if let Some(c) = figures::clamp_rect_to_page_box(rect, box_left, box_bottom, w, h) {
                    rects_new.push(c);
                }
                // 原点 (0,0) = debt-14 修正前のクランプ。
                if let Some(c) = figures::clamp_rect_to_page_box(rect, 0.0, 0.0, w, h) {
                    rects_old.push(c);
                }
            }
            if raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
                skipped += 1;
                continue;
            }
            let merged_new = figures::merge_image_regions(&rects_new, w, h);
            let merged_old = figures::merge_image_regions(&rects_old, w, h);
            regions_new += merged_new.len();
            regions_old += merged_old.len();
            if merged_new == merged_old {
                continue;
            }
            // 「増えた図」と「動いた図」を分けて数える。単純な集合差だと、bbox が動いた 1 図が
            // 新規 +1 と消滅 -1 に二重計上され、carry 破壊（動いた図の数）を読み違える。
            // 片方にしか無いものどうしを重なりで対応づけ、対応が付いたものを「移動」とする。
            let mut only_new: Vec<BBox> = merged_new
                .iter()
                .filter(|b| !merged_old.contains(b))
                .copied()
                .collect();
            let mut only_old: Vec<BBox> = merged_old
                .iter()
                .filter(|b| !merged_new.contains(b))
                .copied()
                .collect();
            let overlaps = |a: &BBox, c: &BBox| {
                (a.x + a.width).min(c.x + c.width) > a.x.max(c.x)
                    && (a.y + a.height).min(c.y + c.height) > a.y.max(c.y)
            };
            let mut moved_here = 0usize;
            only_old.retain(|o| {
                match only_new.iter().position(|n| overlaps(n, o)) {
                    Some(i) => {
                        only_new.remove(i);
                        moved_here += 1;
                        false // 対応が付いた ＝ 消滅ではなく移動
                    }
                    None => true,
                }
            });
            changed += moved_here;
            added += only_new.len();
            removed += only_old.len();
            eprintln!(
                "p{page_number}: page={w:.2}x{h:.2} origin=({box_left:.3},{box_bottom:.3}) \
                 raw_crop={raw_crop:?} images={raw_count}\n  old={merged_old:?}\n  new={merged_new:?}"
            );
        }

        // skipped は回転頁と画像過多頁。**回転頁を本番で扱うようにしたら（8d-8）ここも直すこと** ──
        // このツールは無条件に skip するので、回転頁で増えた図は差分に出ない。
        eprintln!(
            "\n== {path}\n   pages={pages} (nonzero-origin {nonzero_origin_pages}, skipped {skipped})\n   \
             regions: old={regions_old} new={regions_new}  (新規 +{added} / 消滅 -{removed} / 移動 {changed})"
        );
        assert!(pages > 0, "PDF に 1 ページも無い");
    }
}
