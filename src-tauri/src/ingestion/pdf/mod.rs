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

/// ページ境界 box（`CropBox ∩ MediaBox`）の原点と寸法。図領域の計算に渡す 4 値を束ねたもの
/// （`f64` を 4 つ並べて渡すと取り違えても型が通ってしまうため）。
/// [`ExtractedPage`] は同じ 4 値を平坦に持つ ── そちらは構造認識・ページ payload の読み手向け。
#[derive(Clone, Copy, Debug)]
pub struct PageBox {
    /// 絶対 user space（MediaBox 基準）での左下角。
    pub left: f64,
    pub bottom: f64,
    /// box の寸法（`page.width()`/`height()`）。**絶対座標の bbox と比べるときは原点を足すこと。**
    pub width: f64,
    pub height: f64,
}

/// 1 ページの抽出結果。
pub struct ExtractedPage {
    /// 1 始まりのページ番号。
    pub page_number: i64,
    pub width_pt: f64,
    pub height_pt: f64,
    /// ページ境界 box（`CropBox ∩ MediaBox`）の左下角。**`width_pt`/`height_pt` はこの box の
    /// 「寸法」でしかなく、bbox は絶対 user space（MediaBox 基準）**なので、両者を比べる計算は
    /// 必ずこの原点を足すこと（debt-14 / debt-18 はどちらもこれを落としたバグ）。
    /// 読み手: `box_bottom` は `structure::classify_block` の走り柱判定、
    /// 両方が図領域のクランプ（`extract_page_image_regions` へ同じ値を渡している）。
    pub box_left: f64,
    pub box_bottom: f64,
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
        // ページ境界 box の原点。テキスト側（走り柱判定）と図領域側（クランプ）が
        // **同じ値**を使う必要があるので、ページごとにここで 1 回だけ引く。
        let (box_left, box_bottom) = page_box_origin(&page);

        let text = match page.text() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("page {page_number}: text extraction failed: {e}"));
                pages.push(ExtractedPage {
                    page_number,
                    width_pt,
                    height_pt,
                    box_left,
                    box_bottom,
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
                PageBox {
                    left: box_left,
                    bottom: box_bottom,
                    width: width_pt,
                    height: height_pt,
                },
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
            box_left,
            box_bottom,
            rotation_deg,
            plain_text,
            blocks,
            image_regions,
        });
    }

    Ok(ExtractedDocument { pages, warnings })
}

/// 1 ページの図領域を検出し、crop PNG を書き出す（Phase 8a + 8d-8）。
///
/// - トップレベルの Image オブジェクトに加え、**XObjectForm 内の Image**も辿る（Phase 8d-8）。
///   子 bounds は form のコンテンツ空間で返るので、合成行列でページ空間へ移す。
///   どの座標空間かは仮説で決めず form ごとに**自己校正**する
///   （[`figures::calibrate_form_child_space`]）。
/// - 回転ページ（`/Rotate` ≠ 0）は座標変換の検証ができないためスキップする。
/// - 個別の失敗は warning + 欠損で継続し、build 全体は止めない。
fn extract_page_image_regions(
    page: &pdfium_render::prelude::PdfPage<'_>,
    page_number: i64,
    page_box: PageBox,
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
        // ページ境界 box へクランプ。大きく食み出す矩形（変換異常の兆候）は捨てる（誤配置 crop 回避）。
        let Some(clamped) = figures::clamp_rect_to_page_box(
            quad_to_bbox(quad),
            page_box.left,
            page_box.bottom,
            page_box.width,
            page_box.height,
        ) else {
            continue;
        };
        rects.push(clamped);
    }

    // 1b. XObjectForm 内の Image（Phase 8d-8）。**ページを丸ごと捨てる条件に当たるページでは
    //     辿らない** ── 走査が無駄になるのと、そうしておけば「回転ページ」「画像過多ページ」の
    //     出力（と warning の有無）が 8a 当時から 1 ビットも変わらないため。
    let skip_page = rotation_deg != 0.0 || raw_count > figures::MAX_RAW_RECTS_PER_PAGE;
    if !skip_page {
        // 生 Image の枚数は **top-level と form 内の合計**で既存の上限に収める。`merge_image_regions`
        // の fixpoint マージは最悪 O(n^3) なので、上限を実質 2 倍にすると最悪ケースが 8 倍になる。
        // **超過しても「ページを捨てる」側の判定には混ぜない**（混ぜると「250 枚 + form 5 枚」の
        // ページが新たに丸ごと skip され、今出ている図が消える）。溢れた form 内画像を捨てるだけ。
        let budget = figures::MAX_RAW_RECTS_PER_PAGE.saturating_sub(raw_count);
        rects.extend(collect_form_image_rects(
            page,
            page_number,
            page_box,
            budget,
            warnings,
        ));
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
    let merged = figures::merge_image_regions(&rects, page_box.width, page_box.height);
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
            page_box.left,
            page_box.bottom,
            page_box.width,
            page_box.height,
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

/// Phase 8d-8: トップレベルの XObjectForm を辿り、その中の Image の bbox を
/// **ページ空間へ移してクランプした矩形**として返す。
///
/// `\includegraphics` の図が form に包まれている PDF（実ライブラリで 8 版）では、
/// トップレベル列挙だけだと図が 1 枚も見つからない。ただし pdfium が返す子の `bounds()` は
/// **その子が属するコンテンツストリームの空間**（= form のコンテンツ空間）なので、そのまま
/// ページ座標として扱うと誤配置 crop になる。座標空間を仮説で決めず、form ごとに
/// 「そのまま」と「合成行列を当てた」2 通りを form 自身の bounds への包含率で比べ、
/// 勝った方を採る（[`figures::calibrate_form_child_space`]・どちらも閾値未満なら捨てる）。
///
/// `budget` は**このページで追加で拾ってよい生 Image の枚数**（呼び出し側が top-level の
/// 枚数を引いて渡す）。尽きたら残りを捨てて warning にする ── ページ自体は捨てない。
fn collect_form_image_rects(
    page: &pdfium_render::prelude::PdfPage<'_>,
    page_number: i64,
    page_box: PageBox,
    budget: usize,
    warnings: &mut Vec<String>,
) -> Vec<BBox> {
    use pdfium_render::prelude::*;

    let mut out: Vec<BBox> = Vec::new();
    let mut budget = budget;
    let mut dropped_forms = 0usize;
    let mut over_budget = false;

    for mut object in page.objects().iter() {
        if object.as_x_object_form_object().is_none() {
            continue;
        }
        // form 自身の bounds はページ空間（自己校正の基準になる）。取れない form は捨てる。
        let Ok(form_quad) = object.bounds() else {
            continue;
        };
        let form_bounds = quad_to_bbox(form_quad);
        let form_matrix = affine_of(&object);
        let children = collect_form_image_children(
            &mut object,
            form_matrix,
            &mut budget,
            &mut over_budget,
        );
        if children.is_empty() {
            continue;
        }
        // (そのまま, 合成行列を当てたもの)。
        let candidates: Vec<(BBox, BBox)> = children
            .iter()
            .map(|(raw, m)| (*raw, figures::transform_bbox(*raw, *m)))
            .collect();
        let Some(space) = figures::calibrate_form_child_space(&candidates, form_bounds) else {
            dropped_forms += 1;
            continue;
        };
        for (as_page, as_local) in candidates {
            let rect = match space {
                figures::FormChildSpace::PageSpace => as_page,
                figures::FormChildSpace::FormLocal => as_local,
            };
            if let Some(clamped) = figures::clamp_rect_to_page_box(
                rect,
                page_box.left,
                page_box.bottom,
                page_box.width,
                page_box.height,
            ) {
                out.push(clamped);
            }
        }
        if over_budget {
            break;
        }
    }

    if over_budget {
        warnings.push(format!(
            "page {page_number}: raw image budget exhausted (max {} per page incl. top-level); \
             the remaining images inside XObjectForms are skipped",
            figures::MAX_RAW_RECTS_PER_PAGE
        ));
    }
    if dropped_forms > 0 {
        warnings.push(format!(
            "page {page_number}: {dropped_forms} XObjectForm(s) dropped; \
             neither coordinate reading of their images fits the form bounds"
        ));
    }
    out
}

/// 1 個の XObjectForm の下にある Image を `(生 bbox, ページ空間へ移す合成行列)` で集める。
/// 入れ子の form は行列を合成しながら [`figures::MAX_FORM_DEPTH`] まで降りる
/// （実ライブラリの実測では深さ 2 に 12 枚ある）。
fn collect_form_image_children(
    form_object: &mut pdfium_render::prelude::PdfPageObject<'_>,
    form_matrix: figures::Affine,
    budget: &mut usize,
    over_budget: &mut bool,
) -> Vec<(BBox, figures::Affine)> {
    use pdfium_render::prelude::*;

    let mut out = Vec::new();
    // (子オブジェクト, その子が居る空間をページ空間へ移す行列, 深さ)
    let mut stack: Vec<(pdfium_render::prelude::PdfPageObject<'_>, figures::Affine, usize)> =
        Vec::new();
    // `_mut` を使うのは、そちらの戻り値だけがページと同じ寿命を持ち入れ子へ降りられるため。
    if let Some(form) = form_object.as_x_object_form_object_mut() {
        for i in form.as_range() {
            if let Ok(child) = form.get(i) {
                stack.push((child, form_matrix, 1));
            }
        }
    }

    while let Some((mut child, to_page, depth)) = stack.pop() {
        if child.as_x_object_form_object().is_some() {
            if depth >= figures::MAX_FORM_DEPTH {
                continue;
            }
            // 内側 form の子は内側の空間に居るので、内側 → 外側 の順で合成する。
            let composed = figures::compose_affine(affine_of(&child), to_page);
            if let Some(inner) = child.as_x_object_form_object_mut() {
                for i in inner.as_range() {
                    if let Ok(grandchild) = inner.get(i) {
                        stack.push((grandchild, composed, depth + 1));
                    }
                }
            }
            continue;
        }
        if child.as_image_object().is_none() {
            continue;
        }
        if *budget == 0 {
            *over_budget = true;
            break;
        }
        *budget -= 1;
        if let Ok(quad) = child.bounds() {
            out.push((quad_to_bbox(quad), to_page));
        }
    }
    out
}

/// pdfium の bounds（四辺形）を軸並行 [`BBox`] にする。
fn quad_to_bbox(quad: pdfium_render::prelude::PdfQuadPoints) -> BBox {
    let r = quad.to_rect();
    BBox::new(
        r.left().value as f64,
        r.bottom().value as f64,
        (r.right().value - r.left().value) as f64,
        (r.top().value - r.bottom().value) as f64,
    )
}

/// オブジェクトの変換行列。読めなければ恒等（自己校正が「そのまま」と同じ扱いに倒す）。
fn affine_of(object: &pdfium_render::prelude::PdfPageObject<'_>) -> figures::Affine {
    match object.matrix() {
        Ok(m) => [
            m.a() as f64,
            m.b() as f64,
            m.c() as f64,
            m.d() as f64,
            m.e() as f64,
            m.f() as f64,
        ],
        Err(_) => figures::AFFINE_IDENTITY,
    }
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

    /// XObjectForm の座標空間プローブ（8d-8・doc §2.6-4 / §2.12）: form の bounds / matrix と、
    /// その中の Image 子孫の bounds が**どの座標空間で返るか**を測る。
    ///
    /// 判定は本番と同じ純関数（[`figures::containment_ratio`] / [`figures::transform_bbox`] /
    /// [`figures::calibrate_form_child_space`]）を呼ぶので、この道具と出荷コードで式が食い違うことはない。
    /// 新しいコーパスで「本当に form ローカルか」を測り直したくなったらこれを回す。
    ///
    /// - `children_fit_*` は**子 1 枚ごと**に「その解釈なら form の中に収まるか」を数えたもの。
    ///   `as_is`（＝pdfium がページ空間で返している説）が 0 なら form ローカル説が支持される。
    /// - `forms_*` は**form 1 個ごと**の自己校正の結論（本番と同じ粒度）。
    ///
    /// `LCIR_FIG_PDF=/path/to.pdf cargo test --lib xobject_form_probe -- --ignored --nocapture`
    /// `LCIR_PROBE_VERBOSE=1` で form 1 個ずつの明細も出す。
    #[test]
    #[ignore = "manual pdfium probe; needs LCIR_FIG_PDF + libpdfium"]
    fn xobject_form_probe() {
        use pdfium_render::prelude::*;

        let Ok(path) = std::env::var("LCIR_FIG_PDF") else {
            eprintln!("skip: set LCIR_FIG_PDF=/path/to.pdf");
            return;
        };
        let verbose = std::env::var("LCIR_PROBE_VERBOSE").is_ok();
        let bindings = pdfium::bind_pdfium().expect("libpdfium");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium.load_pdf_from_file(&path, None).expect("open PDF");

        let (mut pages, mut rotated_pages, mut top_forms, mut top_images) = (0, 0, 0usize, 0usize);
        let (mut identity_forms, mut form_images, mut pages_with_form_images) = (0usize, 0, 0);
        let (mut fit_as_is, mut fit_composed, mut fit_neither) = (0usize, 0usize, 0usize);
        let (mut forms_local, mut forms_page, mut forms_dropped) = (0usize, 0usize, 0usize);

        for (idx, page) in doc.pages().iter().enumerate() {
            pages += 1;
            let page_number = idx + 1;
            if page.rotation().map_or(0.0, |r| r.as_degrees() as f64) != 0.0 {
                rotated_pages += 1;
            }
            let mut page_form_images = 0usize;

            for mut object in page.objects().iter() {
                if object.as_image_object().is_some() {
                    top_images += 1;
                    continue;
                }
                if object.as_x_object_form_object().is_none() {
                    continue;
                }
                top_forms += 1;
                let Ok(form_quad) = object.bounds() else { continue };
                let form_bounds = quad_to_bbox(form_quad);
                let form_matrix = affine_of(&object);
                if form_matrix == figures::AFFINE_IDENTITY {
                    identity_forms += 1;
                }
                let (mut budget, mut over) = (figures::MAX_RAW_RECTS_PER_PAGE, false);
                let children =
                    collect_form_image_children(&mut object, form_matrix, &mut budget, &mut over);
                if children.is_empty() {
                    continue;
                }
                form_images += children.len();
                page_form_images += children.len();

                let candidates: Vec<(BBox, BBox)> = children
                    .iter()
                    .map(|(raw, m)| (*raw, figures::transform_bbox(*raw, *m)))
                    .collect();
                for (as_is, composed) in &candidates {
                    let (ra, rc) = (
                        figures::containment_ratio(*as_is, form_bounds),
                        figures::containment_ratio(*composed, form_bounds),
                    );
                    match (
                        ra >= figures::FORM_CONTAINMENT_MIN,
                        rc >= figures::FORM_CONTAINMENT_MIN,
                    ) {
                        (true, _) => fit_as_is += 1,
                        (false, true) => fit_composed += 1,
                        (false, false) => fit_neither += 1,
                    }
                    if verbose {
                        eprintln!(
                            "p{page_number} form={form_bounds:?} m={form_matrix:?}\n  \
                             as_is={as_is:?} contained={ra:.4}\n  composed={composed:?} contained={rc:.4}"
                        );
                    }
                }
                match figures::calibrate_form_child_space(&candidates, form_bounds) {
                    Some(figures::FormChildSpace::FormLocal) => forms_local += 1,
                    Some(figures::FormChildSpace::PageSpace) => forms_page += 1,
                    None => forms_dropped += 1,
                }
            }
            if page_form_images > 0 {
                pages_with_form_images += 1;
            }
        }

        eprintln!(
            "PROBE\t{path}\tpages={pages}\trot={rotated_pages}\ttop_forms={top_forms}\t\
             identity_forms={identity_forms}\ttop_images={top_images}\t\
             pages_form_img={pages_with_form_images}\tform_images={form_images}\t\
             children_fit_as_is={fit_as_is}\tchildren_fit_composed={fit_composed}\t\
             children_fit_neither={fit_neither}\t\
             forms_local={forms_local}\tforms_page={forms_page}\tforms_dropped={forms_dropped}"
        );
        assert!(pages > 0, "PDF に 1 ページも無い");
    }

    /// 手動 pdfium 実機確認: 実 PDF 1 本の図領域を、**トップレベル Image だけの場合と
    /// XObjectForm 内の Image も足した場合**の両方で数えて突き合わせる（8d-8）。
    /// 再構築せずに実データで効果と副作用を測るための道具。
    ///
    /// **「old」は出荷中の挙動そのもの**なので、138 版の合計 `old=1202` が
    /// §2.10 の基準値と一致することが「この計測系が本当に走った」ことの検算になる。
    /// 矩形を足す変更（次は 8d-2）を入れたら、この 2 本のリストの作り方だけを差し替えて使う。
    ///
    /// アセットは書き出さない（DB にも app data dir にも触れない・PDF を開くだけ）。
    ///
    /// `LCIR_FIG_CAPTIONS=/path/to.tsv` を渡すと **caption ペアリングの増分**も出す
    /// （TSV は `page_number<TAB>x<TAB>y<TAB>width<TAB>height` で、実 DB の
    /// `figure_caption` ノードの fragment を吸い出したもの）。矩形を増やす変更の本当の
    /// 効き目は「図が増えた数」ではなく「未結合だった図 caption が結ばれた数」なので、
    /// こちらを主指標にする。判定には本番と同じ [`figures::pair_captions`] を使う。
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
        // page_number → caption bbox 群。
        let mut captions: std::collections::HashMap<i64, Vec<BBox>> =
            std::collections::HashMap::new();
        if let Ok(tsv) = std::env::var("LCIR_FIG_CAPTIONS") {
            let body = std::fs::read_to_string(&tsv).expect("caption TSV");
            for line in body.lines() {
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() != 5 {
                    continue;
                }
                let v: Vec<f64> = f[1..].iter().map(|s| s.parse().expect("number")).collect();
                captions
                    .entry(f[0].parse().expect("page"))
                    .or_default()
                    .push(BBox::new(v[0], v[1], v[2], v[3]));
            }
        }
        let bindings = pdfium::bind_pdfium().expect("libpdfium");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium.load_pdf_from_file(&path, None).expect("open PDF");

        let (mut pages, mut nonzero_origin_pages, mut skipped) = (0usize, 0usize, 0usize);
        let (mut regions_new, mut regions_old) = (0usize, 0usize);
        let (mut changed, mut added, mut removed) = (0usize, 0usize, 0usize);
        let (mut form_rects_total, mut form_warnings) = (0usize, 0usize);
        let (mut paired_old, mut paired_new, mut captions_total) = (0usize, 0usize, 0usize);

        for (idx, page) in doc.pages().iter().enumerate() {
            pages += 1;
            let page_number = idx + 1;
            let w = page.width().value as f64;
            let h = page.height().value as f64;
            if page.rotation().map_or(0.0, |r| r.as_degrees() as f64) != 0.0 {
                // 回転頁は本番でも図領域を作らない（8d-8 でも据え置き・debt-9）。
                skipped += 1;
                continue;
            }
            let (box_left, box_bottom) = page_box_origin(&page);
            if box_left != 0.0 || box_bottom != 0.0 {
                nonzero_origin_pages += 1;
            }
            let page_box = PageBox {
                left: box_left,
                bottom: box_bottom,
                width: w,
                height: h,
            };

            let (mut rects_old, mut raw_count) = (Vec::new(), 0usize);
            for object in page.objects().iter() {
                if object.as_image_object().is_none() {
                    continue;
                }
                raw_count += 1;
                if raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
                    continue;
                }
                let Ok(quad) = object.bounds() else { continue };
                if let Some(c) = figures::clamp_rect_to_page_box(
                    quad_to_bbox(quad),
                    box_left,
                    box_bottom,
                    w,
                    h,
                ) {
                    rects_old.push(c);
                }
            }
            if raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
                skipped += 1;
                continue;
            }
            // new = 出荷中の矩形 + XObjectForm 内 Image（本番と同じ関数を呼ぶ）。
            let mut warnings = Vec::new();
            let form_rects = collect_form_image_rects(
                &page,
                page_number as i64,
                page_box,
                figures::MAX_RAW_RECTS_PER_PAGE.saturating_sub(raw_count),
                &mut warnings,
            );
            form_rects_total += form_rects.len();
            form_warnings += warnings.len();
            for wmsg in &warnings {
                eprintln!("  warn: {wmsg}");
            }
            let mut rects_new = rects_old.clone();
            rects_new.extend(form_rects);

            let merged_new = figures::merge_image_regions(&rects_new, w, h);
            let merged_old = figures::merge_image_regions(&rects_old, w, h);
            regions_new += merged_new.len();
            regions_old += merged_old.len();
            if let Some(caps) = captions.get(&(page_number as i64)) {
                captions_total += caps.len();
                paired_old += figures::pair_captions(&merged_old, caps).len();
                paired_new += figures::pair_captions(&merged_new, caps).len();
            }
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
                 images={raw_count}\n  old={merged_old:?}\n  new={merged_new:?}"
            );
        }

        // skipped は回転頁と画像過多頁。**回転頁を本番で扱うようにしたら（debt-9）ここも直すこと** ──
        // このツールは無条件に skip するので、回転頁で増えた図は差分に出ない。
        eprintln!(
            "\n== {path}\n   pages={pages} (nonzero-origin {nonzero_origin_pages}, skipped {skipped})\n   \
             regions: old={regions_old} new={regions_new}  (新規 +{added} / 消滅 -{removed} / 移動 {changed}) \
             form_rects={form_rects_total} form_warnings={form_warnings}\n   \
             captions={captions_total} paired_old={paired_old} paired_new={paired_new}"
        );
        assert!(pages > 0, "PDF に 1 ページも無い");
    }
}
