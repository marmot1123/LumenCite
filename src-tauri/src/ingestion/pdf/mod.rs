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

/// 図領域の判定が使うテキスト側の認識結果（Phase 8d-2）。`f64` を並べるのと同じ理由で
/// 束ねてある ── `&[BBox]` を 2 つ並べて渡す形は取り違えても型が通る。
struct PageTextLayout<'a> {
    /// 図 caption のブロック bbox（`structure::is_figure_caption_label` で絞ったもの）。
    /// **ベクター図の探索はこれが 1 つも余っていないページでは走らない。**
    figure_captions: &'a [BBox],
    /// 本文と読めるブロックの bbox（`structure::is_prose_block_kind`）。誤検出ガードの入力。
    prose_blocks: &'a [BBox],
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
    /// 埋込画像由来（8a / 8d-8）か、ベクター path のクラスタ由来（8d-2）か。
    /// build 側が confidence と caption ペアリングの段を分けるのに使う。
    pub source: figures::RegionSource,
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
    // Phase 8d-2 の caption アンカー用。**build 側（`ingestion::insert_pdf_version_tx`）が
    // 自分の `RecognizerState` で回すのと同じ結果**になる（同じ純関数・同じページ順・同じ初期状態）。
    // `asset_dir` が無い呼び出し（テキストだけの抽出）では構造認識自体を走らせない。
    let mut recognizer = crate::ingestion::structure::RecognizerState::new();

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
        let blocks = text_segments_to_blocks(&text);

        let image_regions = match asset_dir {
            Some(dir) => {
                // Phase 8d-2: ベクター図の探索面を「同一ページに図 caption があるページ」に
                // 絞るためのアンカーと、本文被覆ガードの入力。
                let structured = crate::ingestion::structure::recognize_blocks(
                    &blocks,
                    height_pt,
                    box_bottom,
                    &mut recognizer,
                );
                let captions: Vec<BBox> = structured
                    .iter()
                    .filter(|b| {
                        b.kind == crate::document_ir::NodeKind::FigureCaption
                            && crate::ingestion::structure::is_figure_caption_label(
                                b.caption_label.as_deref(),
                            )
                    })
                    .map(|b| b.bbox)
                    .collect();
                let prose: Vec<BBox> = structured
                    .iter()
                    .filter(|b| crate::ingestion::structure::is_prose_block_kind(b.kind))
                    .map(|b| b.bbox)
                    .collect();
                extract_page_image_regions(
                    &page,
                    page_number,
                    PageBox {
                        left: box_left,
                        bottom: box_bottom,
                        width: width_pt,
                        height: height_pt,
                    },
                    rotation_deg,
                    PageTextLayout {
                        figure_captions: &captions,
                        prose_blocks: &prose,
                    },
                    dir,
                    &mut warnings,
                )
            }
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
/// - **ベクター path のクラスタ**も図領域の候補にする（Phase 8d-2）。ただし探索するのは
///   「同一ページに図 caption があり、そのうちラスタ図と結ばれなかったものが残る」ページだけで、
///   採るのはその余った caption と相互最近でペアになったクラスタだけ。
/// - 回転ページ（`/Rotate` ≠ 0）は座標変換の検証ができないためスキップする。
/// - 個別の失敗は warning + 欠損で継続し、build 全体は止めない。
fn extract_page_image_regions(
    page: &pdfium_render::prelude::PdfPage<'_>,
    page_number: i64,
    page_box: PageBox,
    rotation_deg: f64,
    text_layout: PageTextLayout<'_>,
    asset_dir: &std::path::Path,
    warnings: &mut Vec<String>,
) -> Vec<ExtractedImageRegion> {
    use pdfium_render::prelude::*;

    let PageTextLayout {
        figure_captions,
        prose_blocks,
    } = text_layout;

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
        // ページ境界 box へクランプ。大きく食み出す矩形（変換異常の兆候）は捨てる（誤配置 crop 回避）。
        if let Some(clamped) =
            top_level_image_rect(object.bounds().ok().map(quad_to_bbox), page_box)
        {
            rects.push(clamped);
        }
    }

    // 1b. XObjectForm 内の Image（Phase 8d-8）。**ページを丸ごと捨てる条件に当たるページでは
    //     辿らない**（`figures::should_scan_forms`）── 走査が無駄になるのと、そうしておけば
    //     「回転ページ」「画像過多ページ」の出力と warning が 8a 当時から 1 ビットも変わらないため。
    if figures::should_scan_forms(rotation_deg, raw_count) {
        // 生 Image の枚数は **top-level と form 内で 1 つの予算を共有する**（別枠にはしない）。
        // `merge_image_regions` の fixpoint マージは最悪 O(n^3) なので、上限を実質 2 倍にすると
        // 最悪ケースが 8 倍になる。ただし**超過を「ページを捨てる」判定には混ぜない** ──
        // 混ぜると「250 枚 + form 5 枚」のページが新たに丸ごと skip され、今出ている図が消える。
        // 予算が尽きたら溢れた form 内画像を捨てて warning を出すだけ。
        let budget = figures::MAX_RAW_RECTS_PER_PAGE.saturating_sub(raw_count);
        rects.extend(collect_form_image_rects(
            page,
            page_number,
            page_box,
            budget,
            warnings,
        ));
    }

    // 回転ページと画像過多ページは Phase 8a 当時から「図領域を作らない」ページ。
    // **ベクター走査もしない** ── そのページの出力と warning を 1 ビットも動かさないため
    // （警告は `rects` が空でないときだけ出す、という 8a の非対称もそのまま）。
    if rotation_deg != 0.0 {
        if !rects.is_empty() {
            warnings.push(format!(
                "page {page_number}: rotated page ({rotation_deg} deg); figure regions skipped"
            ));
        }
        return Vec::new();
    }
    if raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
        if !rects.is_empty() {
            warnings.push(format!(
                "page {page_number}: too many image objects ({raw_count}); figure regions skipped"
            ));
        }
        return Vec::new();
    }

    // 2. フィルタ + マージで図領域へ（ラスタ側は 8a のまま）。
    let merged = figures::merge_image_regions(&rects, page_box.width, page_box.height);

    // 2b. Phase 8d-2: ラスタ図と結ばれなかった図 caption が残るページだけ path を走査する。
    //     caption が 1 つも無いページ・全部結ばれたページでは 1 オブジェクトも触らない。
    let mut vector_rects: Vec<BBox> = Vec::new();
    if should_probe_vector_paths(&merged, figure_captions) {
        let (path_rects, path_count) = collect_path_rects(page, page_box);
        match accept_vector_rects(rotation_deg, path_count, path_rects) {
            Some(accepted) => vector_rects = accepted,
            None => warnings.push(format!(
                "page {page_number}: too many path objects ({path_count}); vector figure detection skipped"
            )),
        }
    }
    let regions = figures::compose_figure_regions(
        &merged,
        &vector_rects,
        prose_blocks,
        figure_captions,
        page_box.width,
        page_box.height,
    );
    if regions.is_empty() {
        return Vec::new();
    }

    // 3. ページ全体を 1 回レンダリングし、各領域を crop する（`clip()` はビットマップを
    //    縮めないため使わない）。失敗はページ単位の warning + アセット無し領域で継続。
    let assetless = |regions: Vec<figures::FigureRegion>| -> Vec<ExtractedImageRegion> {
        regions
            .into_iter()
            .map(|r| ExtractedImageRegion {
                bbox: r.bbox,
                source: r.source,
                file: None,
            })
            .collect()
    };
    if let Err(e) = std::fs::create_dir_all(asset_dir) {
        warnings.push(format!(
            "page {page_number}: asset dir creation failed: {e}; figure assets skipped"
        ));
        return assetless(regions);
    }
    let config = PdfRenderConfig::new().set_target_width(RENDER_TARGET_WIDTH);
    let img = match page.render_with_config(&config) {
        Ok(bitmap) => bitmap.as_image(),
        Err(e) => {
            warnings.push(format!(
                "page {page_number}: page render failed: {e}; figure assets skipped"
            ));
            return assetless(regions);
        }
    };

    // crop のファイル名は**由来ごとに独立に採番する**（`fig-` / `vec-`）。連番を共有すると、
    // ベクター領域が 1 つ増えただけで既存ラスタ図のファイル名がずれ、bbox が 1pt も
    // 動いていない図まで `assets.relative_path` が書き換わる。
    // インデックスは `region_to_pixel_rect` の失敗より**前**に進める（8a の `enumerate` と同じ
    // 意味 ── 潰れた領域は番号を消費する）。
    let (mut raster_index, mut vector_index) = (0usize, 0usize);
    let mut out = Vec::new();
    for region in regions {
        let (prefix, i) = match region.source {
            figures::RegionSource::Raster => {
                let i = raster_index;
                raster_index += 1;
                ("fig", i)
            }
            figures::RegionSource::Vector => {
                let i = vector_index;
                vector_index += 1;
                ("vec", i)
            }
        };
        let bbox = region.bbox;
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
                let file_name = format!("{prefix}-p{page_number:03}-{i:02}.png");
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
        out.push(ExtractedImageRegion {
            bbox,
            source: region.source,
            file,
        });
    }
    out
}

/// pdfium のテキストセグメントを [`ExtractedBlock`] 列にする。
/// bounds は PDF user space（左下原点・pt）で、[`BBox`] は左下角 + 幅高さ。
fn text_segments_to_blocks(text: &pdfium_render::prelude::PdfPageText<'_>) -> Vec<ExtractedBlock> {
    let mut blocks = Vec::new();
    for (i, segment) in text.segments().iter().enumerate() {
        let s = segment.text();
        if s.trim().is_empty() {
            continue;
        }
        let r = segment.bounds();
        blocks.push(ExtractedBlock {
            text: s,
            bbox: BBox::new(
                r.left().value as f64,
                r.bottom().value as f64,
                (r.right().value - r.left().value) as f64,
                (r.top().value - r.bottom().value) as f64,
            ),
            reading_order: i as i64,
        });
    }
    blocks
}

/// ページ境界 box へのクランプ（**本番で [`figures::clamp_rect_to_page_box`] を呼ぶ唯一の口**）。
///
/// 原点は `page_box.left` / `page_box.bottom` ── ここを `(0.0, 0.0)` に戻すのが debt-14 の
/// 退行そのもの。純関数と本番の間に配線が 3 本（top-level Image / form 内 Image / path）
/// あった頃は、3 本とも原点を落としても純関数のテストが全部緑のままだった（ゲート ②a の
/// 変異 P2）。口を 1 つにして、その 1 つをテストで固定する。
fn clamp_to_page_box(rect: BBox, page_box: PageBox) -> Option<BBox> {
    figures::clamp_rect_to_page_box(
        rect,
        page_box.left,
        page_box.bottom,
        page_box.width,
        page_box.height,
    )
}

/// トップレベル Image 1 個 → 図領域候補（Phase 8a）。pdfium から読むのは `bounds` だけで、
/// 判定はここに集める（`bounds` が読めなければ捨てる・ページ box へクランプする）。
fn top_level_image_rect(bounds: Option<BBox>, page_box: PageBox) -> Option<BBox> {
    clamp_to_page_box(bounds?, page_box)
}

/// トップレベル path 1 個 → 図領域候補（Phase 8d-2）。pdfium から読むのは
/// 「インクがあるか」「bounds」「クリップ」の 3 つだけで、**判定はすべてここに集める**。
///
/// 3 段で削る。どれも「bbox がインクの位置を表していない」ケースを外すためのもの:
/// ①見えない path（白抜きストローク・透明）②クリップとの交差（pdfium の `bounds()` は
/// クリップを考慮しない）③ページ境界 box へのクランプ。
fn top_level_path_rect(
    has_ink: bool,
    bounds: Option<BBox>,
    clip: &ClipRect,
    page_box: PageBox,
) -> Option<BBox> {
    if !has_ink {
        return None;
    }
    clamp_to_page_box(apply_clip_rect(bounds?, clip)?, page_box)
}

/// そのページで path を走査する価値があるか（Phase 8d-2 の caption アンカー）。
/// **オブジェクトに触る前の門**なので、caption が 1 つも無いページと、図 caption が全部
/// ラスタ図と結ばれたページでは path を 1 本も読まない（探索面 6,321 → 477 ページ）。
fn should_probe_vector_paths(merged: &[BBox], figure_captions: &[BBox]) -> bool {
    !figure_captions.is_empty() && !figures::unpaired_caption_indices(merged, figure_captions).is_empty()
}

/// 走査し終えた path 矩形を採るか、ページごと諦めるか（Phase 8d-2 の上限の門の配線）。
/// `None` ＝ 諦め（呼び出し側が warning を出す）。
///
/// 述語 [`figures::should_scan_vector_paths`] 自体は純関数のテストで固定してあったが、
/// **本番の配線は無防備で、条件を恒真にしても全緑だった**（ゲート ②a の変異 P3）。
fn accept_vector_rects(
    rotation_deg: f64,
    path_count: usize,
    rects: Vec<BBox>,
) -> Option<Vec<BBox>> {
    figures::should_scan_vector_paths(rotation_deg, path_count).then_some(rects)
}

/// Phase 8d-2: ページ上の path オブジェクトから、ベクター図の手掛かりになる矩形を集める。
/// 戻り値は `(矩形列, 生の path オブジェクト数)`。後者は上限判定
/// （[`figures::should_scan_vector_paths`]）に使うので、上限を超えたページでも数え切る。
///
/// 3 段で削る。どれも「bbox がインクの位置を表していない」ケースを外すためのもので、
/// 誤検出より欠損（設計 §16）に倒してある:
///
/// 1. **見えない path**（白抜きストローク・透明）を外す（[`figures::path_has_visible_ink`]）。
/// 2. **クリップパスと交差**させる（pdfium の `bounds()` はクリップを考慮しない）。
/// 3. ページ境界 box へクランプする（ラスタと同じ [`figures::clamp_rect_to_page_box`]）。
fn collect_path_rects(
    page: &pdfium_render::prelude::PdfPage<'_>,
    page_box: PageBox,
) -> (Vec<BBox>, usize) {
    use pdfium_render::prelude::*;

    let mut out: Vec<BBox> = Vec::new();
    let mut count = 0usize;
    for mut object in page.objects().iter() {
        if object.as_path_object().is_some() {
            count += 1;
            if count > figures::MAX_RAW_PATHS_PER_PAGE {
                continue; // 数だけ数える（呼び出し側がページごと捨てる）。
            }
            // 判定は純関数側（[`top_level_path_rect`]）。ここは pdfium から読むだけ。
            // **見えない path のクリップは読まない** ── `clip_path_rect` は
            // `get_clip_path()` がクリップ無しにも `Some` を返す都合で番人を置くほど
            // ホットな呼び出しで、どうせ捨てる path のために払う理由が無い
            // （純関数側も `has_ink` で先に落とすので出力は同じ）。
            let has_ink = path_object_has_ink(&object);
            let clip = if has_ink {
                clip_path_rect(&object)
            } else {
                ClipRect::None
            };
            if let Some(rect) = top_level_path_rect(
                has_ink,
                object.bounds().ok().map(quad_to_bbox),
                &clip,
                page_box,
            ) {
                out.push(rect);
            }
            continue;
        }
        // **中身がベクターだけの XObjectForm は、見える子の外接矩形を 1 個の候補にする。**
        // `\includegraphics{*.pdf}` の図はこの形になる。
        //
        // **form 自身の `bounds()` は使えない。** pdfium はそれを**全子オブジェクトの union**
        // として返すので、組版が版面を消すために置いた**純白の塗り・白抜きストローク**が
        // 1 つでもあると bounds がインクの外まで広がる。実データ（vid275 p35）では
        // form bounds を単独で決めているのが純白の塗り矩形で、可視インクの外接矩形
        // 111.8×627.7+368.1×129.2 に対し bounds は 460.8×331.5 ＝ **面積の 69% が図でない**
        // （空白帯 + caption + 節見出し + 本文 2 行が crop に入る）。
        // クリップも効かない ── form 自身にクリップが付くのは実測 926 個中 11 個だけ。
        //
        // 子の bbox は 8d-8 と同じく form のコンテンツ空間で返るので、座標空間は仮説で決めず
        // **form ごとに自己校正**する（[`figures::calibrate_form_child_space`]）。
        // **Image を持つ form は対象外**（8d-8 が既にその画像をラスタ矩形として拾っており、
        // form 全体を足すと同じ図を二重に数える）。
        if object.as_x_object_form_object().is_none() {
            continue;
        }
        let Ok(form_quad) = object.bounds() else { continue };
        let form_bounds = quad_to_bbox(form_quad);
        let form_matrix = affine_of(&object);
        let (images, children) = collect_form_vector_children(&mut object, form_matrix);
        if !figures::form_is_vector_only(images, visible_path_children(&children)) {
            continue;
        }
        count += 1;
        if count > figures::MAX_RAW_PATHS_PER_PAGE {
            continue;
        }
        // 見えない子を外すのも子 path のクリップを掛けるのもここ（[`form_vector_candidates`]）。
        let candidates = form_vector_candidates(&children);
        let Some(space) = figures::calibrate_form_child_space(&candidates, form_bounds) else {
            continue; // 座標空間が測れない form は捨てる（誤配置 crop より欠損）
        };
        let in_page: Vec<BBox> = candidates
            .iter()
            .map(|(as_page, as_local)| match space {
                figures::FormChildSpace::PageSpace => *as_page,
                figures::FormChildSpace::FormLocal => *as_local,
            })
            .collect();
        let Some(hull) = figures::bbox_hull(&in_page) else {
            continue;
        };
        // form 自身にクリップが付いていれば掛ける（実測では稀だが、掛かる形は実在する）。
        // **こちらはページ空間の hull に対して掛ける** ── form のクリップは form を置く側の
        // コンテンツストリーム（＝ページ）に属するため、子のクリップとは空間が違う。
        let Some(rect) = apply_clip_rect(hull, &clip_path_rect(&object)) else {
            continue;
        };
        if let Some(clamped) = clamp_to_page_box(rect, page_box) {
            out.push(clamped);
        }
    }
    (out, count)
}

/// path オブジェクトが紙の上に見える線・面を持つか（[`figures::path_has_visible_ink`] の入力を
/// pdfium から集める）。**読めない属性は「見える」に倒す** ── 欠損させるのは色が白だと
/// 測れたときだけにする。
fn path_object_has_ink(object: &pdfium_render::prelude::PdfPageObject<'_>) -> bool {
    use pdfium_render::prelude::*;

    let Some(path) = object.as_path_object() else {
        return false;
    };
    let stroked = path.is_stroked().unwrap_or(true);
    let filled = !matches!(path.fill_mode(), Ok(PdfPathFillMode::None));
    let rgba = |c: Result<PdfColor, PdfiumError>| {
        c.map_or((0, 0, 0, 255), |c| (c.red(), c.green(), c.blue(), c.alpha()))
    };
    figures::path_has_visible_ink(
        stroked,
        rgba(object.stroke_color()),
        filled,
        rgba(object.fill_color()),
    )
}

/// XObjectForm の子 path 1 個ぶんの**生データ**（Phase 8d-2）。
///
/// **可視性の判定もクリップの適用もせず、読んだままを運ぶ。** 判定は
/// [`form_vector_candidates`] に閉じ込めてあり、①プローブ（`form_child_clip_probe`）が
/// 同じ 1 つの走査結果から座標空間の両仮説を測れる ②pdfium 無しのテストが
/// 「見えない子を外す」「クリップを掛ける」配線に届く、の 2 つを両立させるため
/// （ゲート ②a の変異 P5 は、この配線がテスト 0 本だったために生き残った）。
struct FormVectorChild {
    /// 子が属するコンテンツストリーム（= form のコンテンツ空間）で返る生 bbox。
    raw: BBox,
    /// その子に効いているクリップ。**`raw` と同じ空間**（`form_child_clip_probe` で実測）。
    /// `has_ink` が false の子は下流で必ず落ちるので、そこでは読まずに `None` を入れてある。
    clip: ClipRect,
    /// `raw` をページ空間へ移す合成行列。
    to_page: figures::Affine,
    /// 紙の上に見える線・面を持つか（[`path_object_has_ink`]）。
    has_ink: bool,
}

/// 生 bbox にクリップを掛ける（Phase 8d-2）。`None` = 紙に出ない（クリップが空・交差が空）。
/// **トップレベル path と form の子で同じ規則**を使うための seam。
///
/// 8d-2 の初版は form の子でこれを呼んでいなかった（ゲート ②a の confirmed 指摘）。
/// `\includegraphics{matplotlib.pdf}` のように **axes クリップ付きの巨大 data path** を持つ
/// included-PDF で、可視インクの hull がクリップ前の生 bbox ぶん膨らむ ── 実測（§2.14）では
/// vid137 p14 の form が 43,462×18,575pt という桁違いの hull になっており、
/// クランプの 50% ルールに落ちて**図が丸ごと消えていた**。
fn apply_clip_rect(raw: BBox, clip: &ClipRect) -> Option<BBox> {
    match clip {
        ClipRect::None => Some(raw),
        ClipRect::Empty => None,
        ClipRect::Rect(c) => figures::intersect_rect(raw, *c),
    }
}

/// 純ベクター form かどうかの判定に使う「**見える** path の子の数」（Phase 8d-2）。
fn visible_path_children(children: &[FormVectorChild]) -> usize {
    children.iter().filter(|c| c.has_ink).count()
}

/// form の子 path から自己校正用の候補 `(そのまま, 合成行列を当てたもの)` を作る（Phase 8d-2）。
///
/// ここが form 側の判定を全部持つ: **①見えない子（白抜き・透明）を外す**
/// **②クリップを掛ける**（ページ空間へ移す前の生空間で交差させるので、
/// 呼び出し側の `FormChildSpace` がどちらに転んでも同じ 1 回の交差で一貫する）。
fn form_vector_candidates(children: &[FormVectorChild]) -> Vec<(BBox, BBox)> {
    children
        .iter()
        .filter(|c| c.has_ink)
        .filter_map(|c| {
            let raw = apply_clip_rect(c.raw, &c.clip)?;
            Some((raw, figures::transform_bbox(raw, c.to_page)))
        })
        .collect()
}

/// XObjectForm の子孫から `(Image の数, path の [`FormVectorChild`])` を集める（Phase 8d-2）。
/// 入れ子は [`figures::MAX_FORM_DEPTH`] まで行列を合成しながら降り、走査する子の総数は
/// [`figures::MAX_FORM_CHILDREN_SCANNED`] で打ち切る（打ち切ると「Image は無い」と誤って
/// 結論しうるので、打ち切り時は Image ありとみなして捨てる）。
///
/// **この関数は pdfium から読むだけで、何も捨てない。** 可視性とクリップの判定は
/// [`form_vector_candidates`] にある ── pdfium 無しのテストが判定に届くようにするため。
/// なお「見えない子を外す」こと自体は 8d-2 の要（pdfium の form `bounds()` は全子の union
/// なので、白抜き 1 つで図の外接矩形が壊れる）。
fn collect_form_vector_children(
    form_object: &mut pdfium_render::prelude::PdfPageObject<'_>,
    form_matrix: figures::Affine,
) -> (usize, Vec<FormVectorChild>) {
    use pdfium_render::prelude::*;

    let mut images = 0usize;
    let mut paths: Vec<FormVectorChild> = Vec::new();
    let mut stack: Vec<(PdfPageObject<'_>, figures::Affine, usize)> = Vec::new();
    if let Some(form) = form_object.as_x_object_form_object_mut() {
        for i in form.as_range() {
            if let Ok(child) = form.get(i) {
                stack.push((child, form_matrix, 1));
            }
        }
    }
    let mut scanned = 0usize;
    while let Some((mut child, to_page, depth)) = stack.pop() {
        scanned += 1;
        if scanned > figures::MAX_FORM_CHILDREN_SCANNED {
            // 全部は見られなかった ＝ Image の有無を言い切れない。捨てる側に倒す。
            return (usize::MAX, paths);
        }
        match child.object_type() {
            PdfPageObjectType::Image => images += 1,
            PdfPageObjectType::Path => {
                // トップレベルと同じく、**見えない子のクリップは読まない**
                // （`form_vector_candidates` が `has_ink` で先に落とすので出力は同じ。
                // form は子を 4,096 個まで辿るので、1 個あたりの FFI を無駄に増やさない）。
                let has_ink = path_object_has_ink(&child);
                if let Ok(quad) = child.bounds() {
                    paths.push(FormVectorChild {
                        raw: quad_to_bbox(quad),
                        clip: if has_ink {
                            clip_path_rect(&child)
                        } else {
                            ClipRect::None
                        },
                        to_page,
                        has_ink,
                    });
                }
            }
            PdfPageObjectType::XObjectForm => {
                if depth >= figures::MAX_FORM_DEPTH {
                    continue;
                }
                let composed = figures::compose_affine(affine_of(&child), to_page);
                if let Some(inner) = child.as_x_object_form_object_mut() {
                    for i in inner.as_range() {
                        if let Ok(g) = inner.get(i) {
                            stack.push((g, composed, depth + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (images, paths)
}

/// クリップパスの本数が「クリップ無し」を意味するか（Phase 8d-2 の番人）。
///
/// **クリップが無いオブジェクトでも `get_clip_path()` は `Some` を返す。**
/// pdfium の `FPDFClipPath_CountPaths` はクリップ無しに `-1` を返し、pdfium-render は
/// それを `as u16` で受けるので（`pdf/path/clip_path.rs:52-53`）**65,535 になる**。
/// 番人を置かないと、クリップの無い path 1 個につき 65,535 回の空ループを回す
/// （実測: 実ライブラリの path の 96% がこの値。1 ページ 500 本なら 3,300 万回）。
/// サブパスが 65,535 個ある実在のクリップは無いので、この値は「クリップ無し」の別名。
fn clip_path_count_is_absent(count: u16) -> bool {
    count == 0 || count == u16::MAX
}

/// サブパスのセグメント数が「読めない」を意味するか（Phase 8d-2 の番人・1 段内側）。
///
/// **こちらで実際に効いているのは `0` の方だけ。** 外側と違い pdfium-render の
/// `PdfClipPathSegments::len()` は `FPDFClipPath_CountPathSegments` の `-1` を
/// `.try_into().unwrap_or(0)` で受けるので（`pdf/path/clip_path.rs:170-176`・0.8.37 で確認）、
/// `u32::MAX` にはならない。外側と同じ崩れがあると書いていた元のコメントは誤りだった
/// （ゲート ②a の指摘）。`u32::MAX` 側は crate がキャストに戻った場合の保険として残す。
fn clip_segment_count_is_absent(count: u32) -> bool {
    count == 0 || count == u32::MAX
}

/// そのセグメントの**端点だけでクリップ領域を下から抑えられる**か（Phase 8d-2・ゲート ②a）。
///
/// `FPDFPathSegment_GetPoint` は 1 セグメント 1 点しか返さないので、ベジエの制御点は取れない。
/// 曲線は端点の外側へ膨らむため、端点だけで組んだ矩形は**真のクリップ領域より小さい**。
/// 種別が読めない（`Unknown`）ときも同じ扱いにする ── 抑えられると言い切れないものを
/// 「抑えられる」に倒すと、クリップが切っていないインクを削る側に外れる。
fn clip_segment_is_boundable(t: pdfium_render::prelude::PdfPathSegmentType) -> bool {
    use pdfium_render::prelude::PdfPathSegmentType;
    matches!(t, PdfPathSegmentType::LineTo | PdfPathSegmentType::MoveTo)
}

/// オブジェクトに効いているクリップ領域の外接矩形（Phase 8d-2）。
enum ClipRect {
    /// クリップパスが付いていない。
    None,
    /// クリップはあるが領域が空（サブパスの交差が空）＝そのオブジェクトは描かれない。
    Empty,
    Rect(BBox),
}

/// クリップパスの矩形近似。**サブパスは AND（交差）** ── PDF の `W` は現在のクリップと
/// 交差するので、全サブパスの点の hull を取ると領域を過大評価する。
///
/// **直線だけでできたクリップしか使わない。** ベジエの制御点は取れない
/// （`FPDFPathSegment_GetPoint` は 1 セグメント 1 点）ので、曲線を含むクリップを端点の hull で
/// 代表すると**真の領域より小さい矩形**になり、交差に使うとクリップが切っていないインクまで削る。
/// ~~「小さい側に外すのは安全側」~~ ── これは 8d-2 の初版が top-level path にだけ
/// クリップを掛けていたときの理屈で、**form の hull に掛けると図が切れる**（ゲート ②a・§2.14。
/// vid123 p171 と vid112 p9 でグラフの曲線の端が crop から欠けた）。曲線を含むクリップは
/// [`ClipRect::None`] にして削るのをやめる。
///
/// 残る近似は矩形化だけ ── 斜めのクリップは外接矩形になる（真の領域より**大きい**側なので
/// インクは削らない）。実データのクリップは軸並行の矩形だった。
fn clip_path_rect(object: &pdfium_render::prelude::PdfPageObject<'_>) -> ClipRect {
    use pdfium_render::prelude::*;

    let Some(clip) = object.get_clip_path() else {
        return ClipRect::None;
    };
    if clip_path_count_is_absent(clip.len()) {
        return ClipRect::None;
    }
    let mut acc: Option<BBox> = None;
    let mut any = false;
    for i in clip.as_range() {
        let Ok(segments) = clip.get(i) else { continue };
        if clip_segment_count_is_absent(segments.len()) {
            continue;
        }
        let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
        let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut points = 0usize;
        for j in segments.as_range() {
            let Ok(seg) = segments.get(j) else { continue };
            // **曲線を含むクリップは矩形で抑えられない。** `FPDFPathSegment_GetPoint` は
            // 1 セグメント 1 点しか返さず、ベジエの制御点は取れないので、端点だけで組んだ
            // 矩形は真のクリップ領域より**小さい**。これを交差に使うと、クリップが
            // 切っていないインクまで削る ── 実データ（vid123 p171・vid112 p9）で
            // グラフの曲線の端が crop から欠けた（crop を焼いて初めて分かった）。
            // 下から抑えられない以上、**この形のクリップは使わない**（削るのをやめる）。
            if !clip_segment_is_boundable(seg.segment_type()) {
                return ClipRect::None;
            }
            let (x, y) = (seg.x().value as f64, seg.y().value as f64);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            points += 1;
        }
        if points == 0 {
            continue;
        }
        any = true;
        let sub = BBox::new(x0, y0, x1 - x0, y1 - y0);
        acc = match acc {
            None => Some(sub),
            Some(a) => match figures::intersect_rect(a, sub) {
                Some(r) => Some(r),
                None => return ClipRect::Empty,
            },
        };
    }
    match acc {
        Some(r) if any => ClipRect::Rect(r),
        _ => ClipRect::None,
    }
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
        // 予算切れの判定はループの**先頭**に置く。末尾に置くと、画像を持たない form の
        // `continue` に飛ばされて予算枯渇後も残りの form を全部走査してしまう。
        if over_budget {
            break;
        }
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
            if let Some(clamped) = clamp_to_page_box(rect, page_box) {
                out.push(clamped);
            }
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

    // ---------------------------------------------------------------------
    // pdfium アダプタ層の配線（ゲート ②a）。
    //
    // このモジュールは長らく**非 `#[ignore]` のテストが 1 本も無かった**。純関数層
    // （`figures.rs` / `structure.rs`）の変異はすべて killed になる一方で、
    // 「純関数を本番がどう呼んでいるか」＝配線の変異は 14 件すべて生き残った。
    // 以下は本番が通る seam を pdfium 無しで固定するためのテスト。
    // ---------------------------------------------------------------------

    fn b(x: f64, y: f64, w: f64, h: f64) -> BBox {
        BBox::new(x, y, w, h)
    }

    /// クランプも交差も幅を `(x+w) - x` で組み直すので、10 進で書いた値は最下位ビットが
    /// ずれる（`figures::tests::assert_bbox_near` と同じ理由）。
    #[track_caller]
    fn assert_bbox_near(got: Option<BBox>, want: BBox) {
        let got = got.expect("領域が捨てられた");
        let d = |a: f64, e: f64| (a - e).abs() < 1e-9;
        assert!(
            d(got.x, want.x) && d(got.y, want.y) && d(got.width, want.width) && d(got.height, want.height),
            "got {got:?}, want {want:?}"
        );
    }

    /// 実データ由来のページ box（vid119 p1: CropBox の左下が原点でない PDF）。
    fn shifted_page_box() -> PageBox {
        PageBox {
            left: 0.0,
            bottom: -51.0,
            width: 495.4,
            height: 739.2,
        }
    }

    /// debt-14: クランプの原点は `page_box.left/bottom` であって `(0,0)` ではない。
    ///
    /// **これがゲート ②a の変異 P2** ── 本番の 3 つの呼び出し口を全部 `(0.0, 0.0)` に
    /// 戻しても 1,051 本が全緑だった。純関数 `clamp_rect_to_page_box` 側は原点付きで
    /// テストされていたが、本番がその原点を渡していることは誰も見ていなかった。
    #[test]
    fn clamp_to_page_box_passes_the_real_origin_not_zero() {
        let pb = shifted_page_box();
        // ページ box の下端は y=-51。y=-37 の図はページの中にあり、切られず残る。
        assert_bbox_near(clamp_to_page_box(b(458.4, -37.0, 25.3, 33.5), pb), b(458.4, -37.0, 25.3, 33.5));
        // 原点を (0,0) に戻すと同じ矩形はページの下端より下＝面積 0 で捨てられる。
        // つまり上の 1 行は「原点が効いている」ことの証拠になっている。
        assert_eq!(
            figures::clamp_rect_to_page_box(b(458.4, -37.0, 25.3, 33.5), 0.0, 0.0, 495.4, 739.2),
            None
        );
    }

    /// トップレベル Image の配線（8a）: bounds が読めなければ捨て、読めればページ box へ
    /// クランプする。クランプの原点は上と同じく `page_box` 由来。
    #[test]
    fn top_level_image_rect_drops_unreadable_bounds_and_clamps_with_the_origin() {
        let pb = shifted_page_box();
        assert_eq!(top_level_image_rect(None, pb), None);
        assert_bbox_near(
            top_level_image_rect(Some(b(458.4, -37.0, 25.3, 33.5)), pb),
            b(458.4, -37.0, 25.3, 33.5),
        );
        // ページ box から大きく食み出す矩形は捨てる（MIN_CLAMPED_AREA_RATIO）。
        assert_eq!(top_level_image_rect(Some(b(458.4, -90.0, 25.3, 33.5)), pb), None);
    }

    /// トップレベル path の配線（8d-2）: 見えない path は 1 本も拾わない。
    #[test]
    fn top_level_path_rect_needs_visible_ink() {
        let pb = shifted_page_box();
        assert_eq!(
            top_level_path_rect(false, Some(b(100.0, 100.0, 200.0, 150.0)), &ClipRect::None, pb),
            None
        );
        assert_eq!(
            top_level_path_rect(true, Some(b(100.0, 100.0, 200.0, 150.0)), &ClipRect::None, pb),
            Some(b(100.0, 100.0, 200.0, 150.0))
        );
        assert_eq!(top_level_path_rect(true, None, &ClipRect::None, pb), None);
    }

    /// トップレベル path の配線（8d-2）: **クリップと交差させてから**使う。
    ///
    /// **これがゲート ②a の変異 P6** ── `ClipRect` の分岐を生の bbox に置き換えても全緑
    /// だった。数字は実データ（vid238 p6）の「クリップで小さく見せている巨大パス」:
    /// 生 bounds は 96.9×1,845.9pt でページ 3 枚ぶんの高さがあり、クリップ後は 96.9×124.6pt。
    #[test]
    fn top_level_path_rect_intersects_the_clip_path() {
        let pb = PageBox {
            left: 0.0,
            bottom: 0.0,
            width: 595.0,
            height: 842.0,
        };
        let raw = b(120.0, -500.0, 96.9, 1845.9);
        let clip = ClipRect::Rect(b(120.0, 600.0, 96.9, 124.6));
        assert_bbox_near(
            top_level_path_rect(true, Some(raw), &clip, pb),
            b(120.0, 600.0, 96.9, 124.6),
        );
        // クリップを掛けないと生 bounds はページ box から大きく食み出し、
        // クランプの 50% ルールでページごと捨てられる（= 図が消える側の失敗）。
        assert_eq!(top_level_path_rect(true, Some(raw), &ClipRect::None, pb), None);
        // クリップ領域が空 = そのオブジェクトは描かれない。
        assert_eq!(top_level_path_rect(true, Some(raw), &ClipRect::Empty, pb), None);
        // クリップと生 bounds が交わらない = 紙に出ない。
        assert_eq!(
            top_level_path_rect(true, Some(raw), &ClipRect::Rect(b(400.0, 600.0, 50.0, 50.0)), pb),
            None
        );
    }

    fn child(raw: BBox, clip: ClipRect, to_page: figures::Affine, has_ink: bool) -> FormVectorChild {
        FormVectorChild {
            raw,
            clip,
            to_page,
            has_ink,
        }
    }

    /// form の子の配線（8d-2）: 見えない子（白抜き・透明）は hull に入れない。
    ///
    /// **これがゲート ②a の変異 P5** ── `path_object_has_ink` の呼び出しを消しても全緑
    /// だった。述語自体（`figures::path_has_visible_ink`）はテスト済みで、配線が無防備だった。
    #[test]
    fn form_vector_candidates_drop_inkless_children() {
        let m = figures::AFFINE_IDENTITY;
        let kids = vec![
            child(b(10.0, 10.0, 20.0, 20.0), ClipRect::None, m, true),
            // 版面を消すための純白の塗り。これを入れると hull が図の外まで広がる。
            child(b(0.0, 0.0, 500.0, 700.0), ClipRect::None, m, false),
        ];
        assert_eq!(visible_path_children(&kids), 1);
        let got = form_vector_candidates(&kids);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, b(10.0, 10.0, 20.0, 20.0));
        assert_eq!(
            figures::bbox_hull(&got.iter().map(|(r, _)| *r).collect::<Vec<_>>()),
            Some(b(10.0, 10.0, 20.0, 20.0)),
            "白抜きを外さないと hull が 500×700 に膨らむ"
        );
    }

    /// form の子の配線（ゲート ②a の confirmed 指摘）: 子 path にも**クリップを掛ける**。
    ///
    /// 8d-2 の初版はトップレベルにだけクリップを掛けており、`\includegraphics{*.pdf}` の
    /// 内側にある axes クリップ付きの巨大 data path を生 bbox のまま hull に入れていた。
    /// 数字は実データ（vid137 p14 の form）を縮めたもので、生のままだと hull が
    /// 43,462×18,575pt になりページ box へのクランプで**図が丸ごと消えていた**。
    ///
    /// 交差は**合成行列を当てる前の生空間**で行う（クリップも子 bbox と同じコンテンツ空間で
    /// 返ることを `form_child_clip_probe` で実測した）。
    #[test]
    fn form_vector_candidates_clip_children_before_transforming_them() {
        // 生空間 → ページ空間は 0.5 倍 + 平行移動。
        let m: figures::Affine = [0.5, 0.0, 0.0, 0.5, 100.0, 200.0];
        let kids = vec![child(
            b(-43_000.0, -17_000.0, 43_400.0, 18_500.0),
            ClipRect::Rect(b(0.0, 0.0, 400.0, 300.0)),
            m,
            true,
        )];
        let got = form_vector_candidates(&kids);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, b(0.0, 0.0, 400.0, 300.0), "生空間で交差させる");
        assert_eq!(
            got[0].1,
            b(100.0, 200.0, 200.0, 150.0),
            "ページ空間の候補もクリップ後の矩形から作る（先に変換すると桁違いの hull になる）"
        );
    }

    /// form の子の配線: クリップが空・交差が空の子は落とす（トップレベルと同じ規則）。
    #[test]
    fn form_vector_candidates_drop_children_that_are_clipped_away() {
        let m = figures::AFFINE_IDENTITY;
        let kids = vec![
            child(b(10.0, 10.0, 20.0, 20.0), ClipRect::Empty, m, true),
            child(
                b(10.0, 10.0, 20.0, 20.0),
                ClipRect::Rect(b(500.0, 500.0, 50.0, 50.0)),
                m,
                true,
            ),
            child(b(60.0, 60.0, 20.0, 20.0), ClipRect::None, m, true),
        ];
        let got = form_vector_candidates(&kids);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, b(60.0, 60.0, 20.0, 20.0));
        // 「見える子の数」はクリップの前に数える（form が純ベクターかの判定材料であって、
        // 紙に出るかとは別の話）。
        assert_eq!(visible_path_children(&kids), 3);
    }

    /// 8d-2 の caption アンカー: caption が無いページと、全部ラスタ図と結ばれたページでは
    /// path オブジェクトに 1 つも触らない（探索面を 6,321 → 477 ページに落とす門）。
    #[test]
    fn vector_paths_are_probed_only_when_a_figure_caption_is_left_unpaired() {
        let fig = b(100.0, 500.0, 200.0, 200.0);
        let cap_under_fig = b(100.0, 480.0, 200.0, 12.0);
        let cap_elsewhere = b(100.0, 100.0, 200.0, 12.0);
        assert!(!should_probe_vector_paths(&[fig], &[]), "caption 0 件なら走査しない");
        assert!(
            !should_probe_vector_paths(&[fig], &[cap_under_fig]),
            "caption が全部ラスタ図と結ばれたら走査しない"
        );
        assert!(
            should_probe_vector_paths(&[fig], &[cap_under_fig, cap_elsewhere]),
            "余った caption があるページだけ走査する"
        );
        assert!(should_probe_vector_paths(&[], &[cap_elsewhere]));
    }

    /// 8d-2 の上限の門の**配線**（ゲート ②a の変異 P3）: 述語 `should_scan_vector_paths` は
    /// 純関数として固定済みだったが、本番が結果を捨てていることは無防備で、条件を恒真に
    /// しても全緑だった。
    #[test]
    fn too_many_paths_makes_the_page_give_up_its_vector_rects() {
        let rects = vec![b(10.0, 10.0, 100.0, 100.0)];
        assert_eq!(
            accept_vector_rects(0.0, figures::MAX_RAW_PATHS_PER_PAGE, rects.clone()),
            Some(rects.clone())
        );
        assert_eq!(
            accept_vector_rects(0.0, figures::MAX_RAW_PATHS_PER_PAGE + 1, rects.clone()),
            None,
            "上限超過ページは矩形を捨てて warning にする"
        );
        assert_eq!(
            accept_vector_rects(90.0, 1, rects),
            None,
            "回転ページはベクター走査の結果も採らない"
        );
    }

    /// クリップ番人（ゲート ②a の変異 P1）: `get_clip_path()` は**クリップの無い
    /// オブジェクトにも `Some` を返す**。番人を外すと出力は変わらないが、path 1 本につき
    /// 65,535 回の空ループが復活する（1 ページ 500 本で約 3,300 万回）。
    #[test]
    fn clip_path_counts_that_mean_no_clip() {
        assert!(clip_path_count_is_absent(0));
        assert!(clip_path_count_is_absent(u16::MAX), "pdfium の -1 が u16 に落ちた値");
        assert!(!clip_path_count_is_absent(1));
        assert!(!clip_path_count_is_absent(4));
        // 内側は crate が -1 を 0 に丸めるので、実際に効くのは 0 の方だけ。
        assert!(clip_segment_count_is_absent(0));
        assert!(clip_segment_count_is_absent(u32::MAX));
        assert!(!clip_segment_count_is_absent(1));
    }

    /// **曲線を含むクリップは使わない**（ゲート ②a・§2.14）。端点だけで組んだ矩形は
    /// 真のクリップ領域より小さいので、交差に使うとクリップが切っていないインクを削る。
    /// 実データ（vid123 p171 / vid112 p9）でグラフの曲線の端が crop から欠けた。
    #[test]
    fn only_straight_clip_segments_can_bound_a_clip_rect() {
        use pdfium_render::prelude::PdfPathSegmentType;
        assert!(clip_segment_is_boundable(PdfPathSegmentType::LineTo));
        assert!(clip_segment_is_boundable(PdfPathSegmentType::MoveTo));
        assert!(
            !clip_segment_is_boundable(PdfPathSegmentType::BezierTo),
            "ベジエは制御点が取れないので端点の hull は下からの抑えにならない"
        );
        assert!(
            !clip_segment_is_boundable(PdfPathSegmentType::Unknown),
            "読めない種別を『抑えられる』に倒すとインクを削る側に外れる"
        );
    }

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


    /// Phase 8d-2 の着手前プローブ: ページ上の**全オブジェクトを種別ごとに測る**。
    /// ベクター図の手掛かり（path クラスタ）と誤検出源（本文の罫線・段組の枠）を
    /// 設計の前に実データで数えるための道具で、判定ロジックは一切持たない
    /// （設計は測ってから決める ── 8d-8 でプローブが母集団の見積りを覆した前例がある）。
    ///
    /// `LCIR_FIG_PDF=/path/to.pdf [LCIR_TAG=vid] [LCIR_DUMP=1] \
    ///  cargo test --lib page_object_census -- --ignored --nocapture`
    ///
    /// 既定は**ページ 1 行の census**（TSV）。`LCIR_DUMP=1` を足すと
    /// オブジェクト 1 個 1 行の明細も出す（クラスタリングの試作を Rust の外で回すため）。
    #[test]
    #[ignore = "manual pdfium probe; needs LCIR_FIG_PDF + libpdfium"]
    fn page_object_census() {
        use pdfium_render::prelude::*;

        let Ok(path) = std::env::var("LCIR_FIG_PDF") else {
            eprintln!("skip: set LCIR_FIG_PDF=/path/to.pdf");
            return;
        };
        let tag = std::env::var("LCIR_TAG").unwrap_or_else(|_| "-".to_string());
        // **libtest は各実行の 1 行目を食う。** 番人を置かないと文書ごとに 1 ページぶんの
        // `CENSUS` 行が静かに落ちる（138 本回して 7,345 頁が 7,207 頁に見えた）。
        eprintln!("CENSUS_BEGIN\t{tag}");
        let dump = std::env::var("LCIR_DUMP").is_ok();
        // 明細は「図 caption のあるページ」だけに絞れる（caption アンカーの設計上、
        // 出力が出うるのはそのページだけなので、それ以外を吐いても読むものが増えるだけ）。
        let dump_pages: std::collections::HashSet<usize> = std::env::var("LCIR_FIG_CAPTIONS")
            .ok()
            .map(|p| {
                std::fs::read_to_string(&p)
                    .expect("caption TSV")
                    .lines()
                    .filter_map(|l| l.split('\t').next()?.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        let bindings = pdfium::bind_pdfium().expect("libpdfium");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium.load_pdf_from_file(&path, None).expect("open PDF");

        for (idx, page) in doc.pages().iter().enumerate() {
            let p = idx + 1;
            let dump = dump && (dump_pages.is_empty() || dump_pages.contains(&p));
            let w = page.width().value as f64;
            let h = page.height().value as f64;
            let rot = page.rotation().map_or(0.0, |r| r.as_degrees() as f64);
            let (ox, oy) = page_box_origin(&page);
            let (mut n_img, mut n_path, mut n_text, mut n_form, mut n_other) = (0, 0, 0, 0, 0);
            let mut color_err = 0;
            // path の粗い形状分布（短辺で刻む）。0 は水平/垂直の罫線。
            let (mut hair, mut tiny, mut small, mut med, mut large) = (0, 0, 0, 0, 0);

            for mut object in page.objects().iter() {
                let kind = object.object_type();
                let bounds = object.bounds().ok().map(quad_to_bbox);
                match kind {
                    PdfPageObjectType::Image => n_img += 1,
                    PdfPageObjectType::Text => n_text += 1,
                    PdfPageObjectType::XObjectForm => n_form += 1,
                    PdfPageObjectType::Path => {
                        n_path += 1;
                        // `path_object_has_ink` は色が読めない path を「黒インク」に倒す
                        // （見える側に倒す）。倒した先が「巨大な白消し矩形」だと 8d-2 が
                        // 実データで潰したクラスタ橋渡しが再発しうる（ゲート ②a の指摘）ので、
                        // **その入口がコーパスに何件あるか**をここで数える。
                        if object.stroke_color().is_err() || object.fill_color().is_err() {
                            color_err += 1;
                        }
                        if let Some(b) = bounds {
                            let m = b.width.min(b.height);
                            match m {
                                _ if m < 0.05 => hair += 1,
                                _ if m < 3.0 => tiny += 1,
                                _ if m < 16.0 => small += 1,
                                _ if m < 60.0 => med += 1,
                                _ => large += 1,
                            }
                        }
                    }
                    _ => n_other += 1,
                }
                if !dump {
                    continue;
                }
                let Some(b) = bounds else { continue };
                let head = format!(
                    "{tag}\t{p}\t{:.2}\t{:.2}\t{:.2}\t{:.2}",
                    b.x, b.y, b.width, b.height
                );
                match kind {
                    PdfPageObjectType::Text => eprintln!("TEXT\t{head}"),
                    PdfPageObjectType::Image => eprintln!("IMG\t{head}"),
                    PdfPageObjectType::Path => {
                        let (segs, stroked, fill) = match object.as_path_object() {
                            Some(po) => (
                                po.segments().len(),
                                po.is_stroked().map_or(-1, i32::from),
                                po.fill_mode().map_or(-1, |f| match f {
                                    PdfPathFillMode::None => 0,
                                    PdfPathFillMode::EvenOdd => 1,
                                    PdfPathFillMode::Winding => 2,
                                }),
                            ),
                            None => (0, -1, -1),
                        };
                        // **本番と同じ関数**を呼ぶ（プローブ側に第 2 の実装を作らない）。
                        let c = match clip_path_rect(&object) {
                            ClipRect::None => "-\t-\t-\t-".to_string(),
                            ClipRect::Empty => "empty\t-\t-\t-".to_string(),
                            ClipRect::Rect(c) => {
                                format!("{:.2}\t{:.2}\t{:.2}\t{:.2}", c.x, c.y, c.width, c.height)
                            }
                        };
                        let col = |r: Result<PdfColor, PdfiumError>| {
                            r.map_or("-".to_string(), |c| {
                                format!("{},{},{},{}", c.red(), c.green(), c.blue(), c.alpha())
                            })
                        };
                        let sw = object.stroke_width().map_or(-1.0, |w| w.value as f64);
                        eprintln!(
                            "PATH\t{head}\t{segs}\t{stroked}\t{fill}\t{c}\t{}\t{}\t{sw:.3}",
                            col(object.stroke_color()),
                            col(object.fill_color())
                        );
                    }
                    PdfPageObjectType::XObjectForm => {
                        let m = affine_of(&object);
                        let (mut ci, mut cp, mut ct, mut cf) = (0, 0, 0, 0);
                        // path の子を**合成行列でページ空間へ移した**外接矩形（form の bounds を
                        // そのまま図領域にしてよいかの判断材料）。
                        let mut hull: Option<BBox> = None;
                        if let Some(form) = object.as_x_object_form_object_mut() {
                            let mut stack: Vec<(PdfPageObject<'_>, figures::Affine, usize)> = form
                                .as_range()
                                .filter_map(|i| form.get(i).ok())
                                .map(|c| (c, m, 1usize))
                                .collect();
                            let mut guard = 0;
                            while let Some((mut child, to_page, depth)) = stack.pop() {
                                guard += 1;
                                if guard > 50_000 {
                                    break;
                                }
                                match child.object_type() {
                                    PdfPageObjectType::Image => ci += 1,
                                    PdfPageObjectType::Text => ct += 1,
                                    PdfPageObjectType::Path => {
                                        cp += 1;
                                        if let Ok(q) = child.bounds() {
                                            let b =
                                                figures::transform_bbox(quad_to_bbox(q), to_page);
                                            hull = Some(match hull {
                                                None => b,
                                                Some(a) => BBox::new(
                                                    a.x.min(b.x),
                                                    a.y.min(b.y),
                                                    (a.x + a.width).max(b.x + b.width)
                                                        - a.x.min(b.x),
                                                    (a.y + a.height).max(b.y + b.height)
                                                        - a.y.min(b.y),
                                                ),
                                            });
                                        }
                                    }
                                    PdfPageObjectType::XObjectForm => {
                                        cf += 1;
                                        if depth >= figures::MAX_FORM_DEPTH {
                                            continue;
                                        }
                                        let composed =
                                            figures::compose_affine(affine_of(&child), to_page);
                                        if let Some(inner) = child.as_x_object_form_object_mut() {
                                            for i in inner.as_range() {
                                                if let Ok(g) = inner.get(i) {
                                                    stack.push((g, composed, depth + 1));
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        let hull = hull.map_or_else(
                            || "-\t-\t-\t-".to_string(),
                            |b| format!("{:.2}\t{:.2}\t{:.2}\t{:.2}", b.x, b.y, b.width, b.height),
                        );
                        eprintln!(
                            "FORM\t{head}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.2}\t{:.2}\t\
                             {ci}\t{cp}\t{ct}\t{cf}\t{hull}",
                            m[0], m[1], m[2], m[3], m[4], m[5]
                        );
                    }
                    _ => {}
                }
            }
            eprintln!(
                "CENSUS\t{tag}\t{p}\t{rot}\t{w:.2}\t{h:.2}\t{ox:.3}\t{oy:.3}\t\
                 {n_img}\t{n_path}\t{n_text}\t{n_form}\t{n_other}\t\
                 {hair}\t{tiny}\t{small}\t{med}\t{large}\t{color_err}"
            );
        }
        eprintln!("CENSUS_DONE\t{tag}\t{path}");
    }

    /// ゲート ②a の着手前プローブ: **form の子 path に付くクリップが、子 bbox と同じ
    /// コンテンツ空間で返るのか、ページ空間で返るのか**を実データで決める。
    ///
    /// 仮説で決められない。間違った空間で交差すると交差が空になり、**図が丸ごと消える**
    /// （欠損は集計値では気づけない）。判別は 8d-8 / 8d-2 の自己校正と同じ理屈で、
    /// 実測が `FormLocal` 43 / `PageSpace` 0 ＝ 生 bbox とページ空間が別物である以上、
    /// クリップ矩形がどちらに寄るかは被覆率で分かれる。
    ///
    /// 本番と同じ `collect_form_vector_children` を呼ぶ（プローブ側に第 2 の走査を作らない）。
    /// クリップを掛けた／掛けない hull の差も出すので、修正の効き幅もここで分かる。
    ///
    /// `LCIR_FIG_PDF=/path/to.pdf [LCIR_TAG=vid] \
    ///  cargo test --lib form_child_clip_probe -- --ignored --nocapture`
    #[test]
    #[ignore = "manual pdfium probe; needs LCIR_FIG_PDF + libpdfium"]
    fn form_child_clip_probe() {
        use pdfium_render::prelude::*;

        let Ok(path) = std::env::var("LCIR_FIG_PDF") else {
            eprintln!("skip: set LCIR_FIG_PDF=/path/to.pdf");
            return;
        };
        let tag = std::env::var("LCIR_TAG").unwrap_or_else(|_| "-".to_string());
        // **libtest は各実行の 1 行目を食う。** 集計行が静かに落ちないよう先に捨て行を置く。
        eprintln!("FCLIP_BEGIN\t{tag}");
        let bindings = pdfium::bind_pdfium().expect("libpdfium");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium.load_pdf_from_file(&path, None).expect("open PDF");

        let area = |b: BBox| b.width.max(0.0) * b.height.max(0.0);
        let cover = |a: BBox, b: BBox| {
            let d = area(a);
            if d <= 0.0 {
                return -1.0; // 測れない（0 除算を「被覆 0」に落とさない）
            }
            figures::intersect_rect(a, b).map_or(0.0, area) / d
        };
        // 集計: 純ベクター form の数 / 子 path の総数 / クリップ Rect / クリップ Empty /
        //       「生空間仮説の方が被覆が高い子」/「ページ空間仮説の方が高い子」/ hull が縮んだ form
        let (mut forms, mut vec_forms, mut kids, mut clipped, mut empty) = (0, 0, 0, 0, 0);
        let (mut win_raw, mut win_page, mut tie) = (0usize, 0usize, 0usize);
        let (mut hull_shrunk, mut hull_lost) = (0usize, 0usize);

        for (idx, page) in doc.pages().iter().enumerate() {
            let p = idx + 1;
            for (fi, mut object) in page.objects().iter().enumerate() {
                if object.as_x_object_form_object().is_none() {
                    continue;
                }
                forms += 1;
                let Ok(form_quad) = object.bounds() else { continue };
                let form_bounds = quad_to_bbox(form_quad);
                let form_matrix = affine_of(&object);
                let (images, children) = collect_form_vector_children(&mut object, form_matrix);
                if !figures::form_is_vector_only(images, visible_path_children(&children)) {
                    continue;
                }
                vec_forms += 1;
                kids += visible_path_children(&children);
                let mut form_clipped = 0usize;
                for (ci, c) in children.iter().filter(|c| c.has_ink).enumerate() {
                    let as_page = figures::transform_bbox(c.raw, c.to_page);
                    let cr = match &c.clip {
                        ClipRect::None => continue,
                        ClipRect::Empty => {
                            empty += 1;
                            eprintln!("FCHILD\t{tag}\t{p}\t{fi}\t{ci}\tempty");
                            continue;
                        }
                        ClipRect::Rect(r) => *r,
                    };
                    clipped += 1;
                    form_clipped += 1;
                    // 「クリップは自分が包む対象と大きく重なる」ことを使って空間を当てる。
                    let ov_raw = cover(c.raw, cr);
                    let ov_page = cover(as_page, cr);
                    match (ov_raw, ov_page) {
                        (a, b) if a > b => win_raw += 1,
                        (a, b) if b > a => win_page += 1,
                        _ => tie += 1,
                    }
                    eprintln!(
                        "FCHILD\t{tag}\t{p}\t{fi}\t{ci}\trect\t\
                         {:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t\
                         {:.2}\t{:.2}\t{:.2}\t{:.2}\t{ov_raw:.4}\t{ov_page:.4}",
                        c.raw.x, c.raw.y, c.raw.width, c.raw.height,
                        as_page.x, as_page.y, as_page.width, as_page.height,
                        cr.x, cr.y, cr.width, cr.height
                    );
                }
                if form_clipped == 0 {
                    continue;
                }
                // クリップを掛けない場合と掛けた場合で、本番が採る hull がどう変わるか。
                let pick = |cands: &[(BBox, BBox)], space: figures::FormChildSpace| {
                    let v: Vec<BBox> = cands
                        .iter()
                        .map(|(as_page, as_local)| match space {
                            figures::FormChildSpace::PageSpace => *as_page,
                            figures::FormChildSpace::FormLocal => *as_local,
                        })
                        .collect();
                    figures::bbox_hull(&v)
                };
                // before = クリップを掛けない（8d-2 初版の挙動）/ after = 本番の純関数そのもの。
                let raw_cands: Vec<(BBox, BBox)> = children
                    .iter()
                    .filter(|c| c.has_ink)
                    .map(|c| (c.raw, figures::transform_bbox(c.raw, c.to_page)))
                    .collect();
                let clipped_cands = form_vector_candidates(&children);
                let before = figures::calibrate_form_child_space(&raw_cands, form_bounds)
                    .and_then(|s| pick(&raw_cands, s));
                let after = figures::calibrate_form_child_space(&clipped_cands, form_bounds)
                    .and_then(|s| pick(&clipped_cands, s));
                match (before, after) {
                    (Some(b), Some(a)) => {
                        if area(a) < area(b) - 0.01 {
                            hull_shrunk += 1;
                        }
                        eprintln!(
                            "FCFORM\t{tag}\t{p}\t{fi}\t{}\t{form_clipped}\t\
                             {:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{:.4}",
                            children.len(),
                            b.x, b.y, b.width, b.height,
                            a.x, a.y, a.width, a.height,
                            if area(b) > 0.0 { area(a) / area(b) } else { -1.0 }
                        );
                    }
                    (Some(_), None) => {
                        hull_lost += 1;
                        eprintln!("FCFORM\t{tag}\t{p}\t{fi}\t{}\t{form_clipped}\tLOST", children.len());
                    }
                    _ => {}
                }
            }
        }
        eprintln!(
            "\nFCLIP\t{tag}\tforms={forms}\tvec_forms={vec_forms}\tkids={kids}\t\
             clipped={clipped}\tempty={empty}\t\
             win_raw={win_raw}\twin_page={win_page}\ttie={tie}\t\
             hull_shrunk={hull_shrunk}\thull_lost={hull_lost}"
        );
        eprintln!("FCLIP_DONE\t{tag}\t{path}");
    }

    /// プローブ用: 与えた矩形を実際に crop PNG に焼いて目で確かめる（8d-2 の設計検証）。
    /// 件数と面積だけでは「その矩形が本当に図か」は分からないので、候補を目視する道具。
    ///
    /// `LCIR_FIG_PDF=/path.pdf LCIR_CROP_TSV=<page\tx\ty\tw\th の TSV> LCIR_CROP_OUT=<dir> \
    ///  cargo test --lib render_region_crops -- --ignored --nocapture`
    #[test]
    #[ignore = "manual pdfium probe; needs LCIR_FIG_PDF + LCIR_CROP_TSV"]
    fn render_region_crops() {
        use pdfium_render::prelude::*;

        let (Ok(path), Ok(tsv), Ok(out)) = (
            std::env::var("LCIR_FIG_PDF"),
            std::env::var("LCIR_CROP_TSV"),
            std::env::var("LCIR_CROP_OUT"),
        ) else {
            eprintln!("skip: set LCIR_FIG_PDF / LCIR_CROP_TSV / LCIR_CROP_OUT");
            return;
        };
        let tag = std::env::var("LCIR_TAG").unwrap_or_else(|_| "x".to_string());
        std::fs::create_dir_all(&out).expect("out dir");
        let mut wanted: std::collections::BTreeMap<i64, Vec<BBox>> = Default::default();
        for line in std::fs::read_to_string(&tsv).expect("TSV").lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 5 {
                continue;
            }
            let v: Vec<f64> = f[1..5].iter().map(|s| s.parse().expect("number")).collect();
            wanted
                .entry(f[0].parse().expect("page"))
                .or_default()
                .push(BBox::new(v[0], v[1], v[2], v[3]));
        }
        let bindings = pdfium::bind_pdfium().expect("libpdfium");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium.load_pdf_from_file(&path, None).expect("open PDF");
        let config = PdfRenderConfig::new().set_target_width(RENDER_TARGET_WIDTH);

        for (idx, page) in doc.pages().iter().enumerate() {
            let p = idx as i64 + 1;
            let Some(rects) = wanted.get(&p) else { continue };
            let (w, h) = (page.width().value as f64, page.height().value as f64);
            let (bl, bb) = page_box_origin(&page);
            let Ok(bitmap) = page.render_with_config(&config) else {
                eprintln!("CROP_FAIL\t{tag}\t{p}\trender");
                continue;
            };
            let img = bitmap.as_image();
            for (i, r) in rects.iter().enumerate() {
                let Some((px, py, pw, ph)) =
                    figures::region_to_pixel_rect(*r, bl, bb, w, h, img.width(), img.height())
                else {
                    eprintln!("CROP_FAIL\t{tag}\t{p}\t{i}\tdegenerate");
                    continue;
                };
                let name = format!("{out}/{tag}-p{p:03}-{i:02}.png");
                img.crop_imm(px, py, pw, ph).save(&name).expect("save png");
                eprintln!("CROP\t{tag}\t{p}\t{i}\t{name}");
            }
        }
    }

    /// 手動 pdfium 実機確認: 実 PDF 1 本の図領域を、**出荷中（ラスタのみ）の場合と
    /// ベクター path クラスタも足した場合**の両方で数えて突き合わせる（8d-2）。
    /// 再構築せずに実データで効果と副作用を測るための道具。
    ///
    /// **「old」は出荷中（抽出器 0.11.0）の挙動そのもの**なので、生存 138 版の合計
    /// `old=1248` が §11 の基準値と一致することが「この計測系が本当に 138 本走った」ことの
    /// 検算になる（これが無いと「1 本も走っていない」を「差分ゼロ」と読む）。
    ///
    /// caption と本文ブロックは **PDF から本番と同じ純関数で作る**
    /// （`structure::recognize_blocks` をページ順に 1 回・DB は引かない）。したがって
    /// この道具の caption 集合は本番の `page_captions` と同じもので、実 DB のスナップショット
    /// （抽出器 0.6.0 時点）とは独立している。
    ///
    /// アセットは書き出さない（DB にも app data dir にも触れない・PDF を開くだけ）。
    /// `LCIR_AB_VERBOSE=1` でページごとの明細も出す。
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
        let tag = std::env::var("LCIR_TAG").unwrap_or_else(|_| "-".to_string());
        let verbose = std::env::var("LCIR_AB_VERBOSE").is_ok();
        let bindings = pdfium::bind_pdfium().expect("libpdfium");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium.load_pdf_from_file(&path, None).expect("open PDF");

        let (mut pages, mut skipped) = (0usize, 0usize);
        let (mut regions_old, mut regions_new) = (0usize, 0usize);
        let (mut added, mut moved, mut removed) = (0usize, 0usize, 0usize);
        let (mut captions_total, mut paired_old, mut paired_new) = (0usize, 0usize, 0usize);
        let (mut vector_rects_total, mut path_warn_pages) = (0usize, 0usize);
        let (mut stolen, mut vec_big, mut vec_billable) = (0usize, 0usize, 0usize);
        let mut recognizer = crate::ingestion::structure::RecognizerState::new();

        for (idx, page) in doc.pages().iter().enumerate() {
            pages += 1;
            let page_number = idx as i64 + 1;
            let w = page.width().value as f64;
            let h = page.height().value as f64;
            let rotation_deg = page.rotation().map_or(0.0, |r| r.as_degrees() as f64);
            let (box_left, box_bottom) = page_box_origin(&page);
            let page_box = PageBox {
                left: box_left,
                bottom: box_bottom,
                width: w,
                height: h,
            };
            // 本番と同じ手順で caption / 本文ブロックを作る（state はページ順に持ち回る）。
            // **本番はテキスト抽出に失敗したページを図領域ごと諦める**（`extract_document`）。
            // ハーネスもそこで打ち切らないと、そのページの領域を old にも new にも数えてしまい
            // 「old = 出荷中の挙動そのもの」が崩れる。
            let Ok(text) = page.text() else {
                skipped += 1;
                continue;
            };
            let blocks = text_segments_to_blocks(&text);
            let structured =
                crate::ingestion::structure::recognize_blocks(&blocks, h, box_bottom, &mut recognizer);
            let captions: Vec<BBox> = structured
                .iter()
                .filter(|b| {
                    b.kind == crate::document_ir::NodeKind::FigureCaption
                        && crate::ingestion::structure::is_figure_caption_label(
                            b.caption_label.as_deref(),
                        )
                })
                .map(|b| b.bbox)
                .collect();
            let prose: Vec<BBox> = structured
                .iter()
                .filter(|b| crate::ingestion::structure::is_prose_block_kind(b.kind))
                .map(|b| b.bbox)
                .collect();
            captions_total += captions.len();

            // old = 出荷中の図領域（トップレベル Image + form 内 Image）。
            let (mut rects, mut raw_count) = (Vec::new(), 0usize);
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
                    rects.push(c);
                }
            }
            if rotation_deg != 0.0 || raw_count > figures::MAX_RAW_RECTS_PER_PAGE {
                // 本番も図領域を作らないページ。ベクター走査もしない。
                skipped += 1;
                continue;
            }
            let mut warnings = Vec::new();
            if figures::should_scan_forms(rotation_deg, raw_count) {
                rects.extend(collect_form_image_rects(
                    &page,
                    page_number,
                    page_box,
                    figures::MAX_RAW_RECTS_PER_PAGE.saturating_sub(raw_count),
                    &mut warnings,
                ));
            }
            let merged_old = figures::merge_image_regions(&rects, w, h);

            // new = old + ベクター領域（本番と同じ関数・同じ門）。
            let mut vector_rects: Vec<BBox> = Vec::new();
            if !captions.is_empty()
                && !figures::unpaired_caption_indices(&merged_old, &captions).is_empty()
            {
                let (r, path_count) = collect_path_rects(&page, page_box);
                if figures::should_scan_vector_paths(rotation_deg, path_count) {
                    vector_rects = r;
                } else {
                    path_warn_pages += 1;
                }
            }
            vector_rects_total += vector_rects.len();
            let composed =
                figures::compose_figure_regions(&merged_old, &vector_rects, &prose, &captions, w, h);
            let merged_new: Vec<BBox> = composed.iter().map(|r| r.bbox).collect();
            let vectors: Vec<BBox> = composed
                .iter()
                .filter(|r| r.source == figures::RegionSource::Vector)
                .map(|r| r.bbox)
                .collect();
            let page_area = (w * h).max(1.0);
            vec_big += vectors
                .iter()
                .filter(|b| (b.width * b.height) / page_area > 0.5)
                .count();
            // 8c の Vision バッチは crop の短辺 200px 以上を対象にする（`DEFAULT_MIN_CROP_PX`）。
            // ページ幅 `w`pt を `RENDER_TARGET_WIDTH`px でレンダリングするので、
            // 200px ⟺ `w / 8`pt。**増えた図のうち何件が課金対象になるか**の見積り。
            let billable_pt = w * 200.0 / RENDER_TARGET_WIDTH as f64;
            vec_billable += vectors
                .iter()
                .filter(|b| b.width.min(b.height) >= billable_pt)
                .count();
            for b in &vectors {
                eprintln!(
                    "VEC\t{tag}\t{page_number}\t{:.3}\t{:.3}\t{:.3}\t{:.3}",
                    b.x, b.y, b.width, b.height
                );
            }
            // **crop の画素位置まで出す**（ゲート ②a の指摘）。bbox が 1pt も動かないのに
            // crop の画素だけがずれる変更は bbox の集合差では原理的に検出できない ──
            // #2（debt-14）の `page_box_origin` を `crop()`/`media()` から `bounding()` へ
            // 付け替えた変更がまさにその型で、carry の鍵は bbox ではなく crop の sha256 の方。
            // 2 回の実行の `REG` 行を突き合わせれば「bbox 一致・画素移動」が拾える。
            // レンダリングはしない（7,345 頁ぶんは重い）ので画像サイズは本番と同じ
            // `RENDER_TARGET_WIDTH` 基準の公称値を使う。**pdfium の丸めとは 1px ずれうるが、
            // 両方の実行で同じ式なので差分の検出能力は変わらない。**
            let img_w = RENDER_TARGET_WIDTH as u32;
            let img_h = ((RENDER_TARGET_WIDTH as f64) * h / w.max(1.0)).round().max(1.0) as u32;
            for (i, r) in composed.iter().enumerate() {
                let px = figures::region_to_pixel_rect(r.bbox, box_left, box_bottom, w, h, img_w, img_h)
                    .map_or_else(
                        || "-\t-\t-\t-".to_string(),
                        |(x, y, pw, ph)| format!("{x}\t{y}\t{pw}\t{ph}"),
                    );
                eprintln!(
                    "REG\t{tag}\t{page_number}\t{i}\t{:?}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{px}",
                    r.source, r.bbox.x, r.bbox.y, r.bbox.width, r.bbox.height
                );
            }

            regions_old += merged_old.len();
            regions_new += merged_new.len();
            let pold = figures::pair_captions(&merged_old, &captions);
            let pnew = figures::pair_captions_two_stage(&merged_old, &vectors, &captions);
            paired_old += pold.len();
            paired_new += pnew.len();
            // **この差は構造上必ず 0 になる**（`pair_captions_two_stage` は 1 段目の結果を
            // そのまま返り値の先頭に積むため）。したがってこれは「2 段ペアリングが正しい」証拠
            // ではなく、**2 段の実装が 1 段に退化していないことの番人**にすぎない。
            // 「1 段だと実際に奪われる」ことの証拠は純関数のテスト
            // `figures::tests::a_vector_region_cannot_steal_a_caption_from_a_raster_figure` の方。
            let old_pairs: std::collections::HashSet<(usize, usize)> = pold.into_iter().collect();
            stolen += old_pairs
                .iter()
                .filter(|p| !pnew.contains(p))
                .count();

            if merged_new == merged_old {
                continue;
            }
            // 「増えた図」と「動いた図」を分けて数える。単純な集合差だと、bbox が動いた 1 図が
            // 新規 +1 と消滅 -1 に二重計上され、carry 破壊（動いた図の数）を読み違える。
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
            only_old.retain(|o| match only_new.iter().position(|n| overlaps(n, o)) {
                Some(i) => {
                    only_new.remove(i);
                    moved_here += 1;
                    false // 対応が付いた ＝ 消滅ではなく移動
                }
                None => true,
            });
            moved += moved_here;
            added += only_new.len();
            removed += only_old.len();
            if verbose {
                eprintln!(
                    "p{page_number}: page={w:.2}x{h:.2} origin=({box_left:.3},{box_bottom:.3}) \
                     images={raw_count} paths={} caps={}\n  old={merged_old:?}\n  vec={vectors:?}",
                    vector_rects.len(),
                    captions.len()
                );
            }
        }

        eprintln!(
            "\nAB\t{tag}\tpages={pages}\tskipped={skipped}\t\
             old={regions_old}\tnew={regions_new}\tadded={added}\tmoved={moved}\tremoved={removed}\t\
             caps={captions_total}\tpaired_old={paired_old}\tpaired_new={paired_new}\t\
             stolen={stolen}\tvec_rects={vector_rects_total}\tvec_big={vec_big}\tvec_billable={vec_billable}\t\
             path_warn_pages={path_warn_pages}"
        );
        eprintln!("AB_DONE\t{tag}\t{path}");
        assert!(pages > 0, "PDF に 1 ページも無い");
    }
}
