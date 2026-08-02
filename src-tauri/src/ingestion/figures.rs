//! Phase 8a: 図領域の幾何処理（pdfium 非依存の純関数）。
//!
//! - `merge_image_regions`: ページ内の埋込画像 bbox 群をフィルタ + 近接マージして「図領域」にする。
//! - `region_to_pixel_rect`: PDF user space（左下原点・pt）の領域を、ページレンダリング画像の
//!   ピクセル矩形（左上原点）へ変換する。ページ境界 box の原点（CropBox が (0,0) 始まりでない
//!   雑誌 PDF がある）を補正する。
//! - `pair_captions`: 図領域と caption ブロックを幾何ペアリングする（相互最近のみ・曖昧なら
//!   張らない＝誤検出より欠損）。
//!
//! すべて座標のみを扱い pdfium/sqlx に依存しないので CI で完全にテストできる。

use crate::document_ir::BBox;

/// 図領域候補として拾う最小の短辺（pt）。ロゴ・飾り罫・小アイコンを除外する。
pub const MIN_DIM_PT: f64 = 16.0;
/// これ以上ページ面積を占める画像は背景・透かしとみなして除外する。
pub const MAX_PAGE_AREA_RATIO: f64 = 0.9;
/// このギャップ（pt）以内で隣接する画像 bbox は同一の図としてマージする。
pub const MERGE_GAP_PT: f64 = 12.0;
/// 1 ページから拾う図領域の上限（面積上位を残す）。
pub const MAX_REGIONS_PER_PAGE: usize = 8;
/// 1 ページの生画像オブジェクト数の上限。超えるページはスライス化ラスタ（スキャン・
/// グラデーション帯）とみなして図領域検出をスキップする（O(n^3) マージの暴走も防ぐ）。
pub const MAX_RAW_RECTS_PER_PAGE: usize = 256;
/// caption ペアリングで許す図と caption の垂直ギャップ（pt）。
pub const CAPTION_GAP_MAX_PT: f64 = 60.0;
/// caption ペアリングで要求する水平重なり（短い方の幅に対する比）。
pub const CAPTION_OVERLAP_RATIO: f64 = 0.3;
/// クランプ後に元面積のこの比率を下回った矩形は捨てる（座標変換異常の兆候）。
pub const MIN_CLAMPED_AREA_RATIO: f64 = 0.5;

/// ページ box の原点（左下角）を CropBox / MediaBox の左下角から組む**フォールバック**。
/// 通常は pdfium に直接聞く（`pdf::page_box_origin` の `bounding()`）ので、ここへ来るのは
/// それが失敗したときだけ。
///
/// **生の CropBox ではなく `CropBox ∩ MediaBox` を採る。** pdfium のページ寸法
/// （`FPDF_GetPageWidthF` = `width_pt`/`height_pt`）はこの交差の寸法だからで、
/// 原点だけ生の CropBox から取ると寸法と原点の出所が食い違う。軸並行矩形の交差の
/// 左下角は各成分の大きい方。
///
/// 実測（生存 138 版 7,345 頁・pdfium に直接問い合わせ）: 交差モデルは box を持つ
/// 7,281 頁すべてで pdfium のページ寸法を再現し、生 CropBox モデルは 406 頁で外れる。
/// 原点が食い違うのは 392 頁 / 2 版（CropBox が (0,0) 始まりで MediaBox の原点が
/// 非ゼロという形。ずれ幅 77.8–90.0pt）。交差が空になる頁は 0 件。
///
/// **交差モデルでも当てられない形が 2 つある**（どちらも `bounding()` なら正しく出る）:
/// ①box が `/Pages` から継承されていると `FPDFPage_Get*Box` は両方とも失敗し `(0,0)` に落ちる
/// ②空の `/CropBox`（`[0 0 0 0]`）を pdfium は無視するが、この関数は max に参加させてしまう。
/// フォールバックである以上「pdfium と厳密に同じ」までは保証しない（欠損 > 誤り）。
pub fn effective_page_box_origin(
    crop_origin: Option<(f64, f64)>,
    media_origin: Option<(f64, f64)>,
) -> (f64, f64) {
    match (crop_origin, media_origin) {
        (Some((cl, cb)), Some((ml, mb))) => (cl.max(ml), cb.max(mb)),
        (Some(o), None) | (None, Some(o)) => o,
        (None, None) => (0.0, 0.0),
    }
}

/// 埋込画像の bbox をページ境界 box へクランプする。範囲外に出た分を落とし、
/// 元面積の [`MIN_CLAMPED_AREA_RATIO`] を下回るまで削られた矩形は捨てる（`None`）。
///
/// **クランプ範囲は `[0, page_w] × [0, page_h]` ではない**（debt-14）。pdfium が返す
/// オブジェクトの bounds は絶対 user space（MediaBox 基準）である一方、`page_w`/`page_h` は
/// ページ境界 box（CropBox）の**寸法**でしかない。したがって原点が非ゼロの PDF
/// （裁ち落とし付きの雑誌・紀要）での有効範囲は
/// `[box_left, box_left + page_w] × [box_bottom, box_bottom + page_h]` になる。
/// 原点を落とすと ①ページ右端・上端の図が原点ぶんだけ切り落とされ、削られた面積が
/// 半分を超えれば図そのものが消える ②逆に box の外（トンボ・裁ち落とし）の画像が
/// 素通りして「ページに表示されない内容の crop」になる。
///
/// 戻り値は入力と同じ絶対 user space。box 原点を引いてページ画像のピクセルへ移すのは
/// [`region_to_pixel_rect`] の仕事で、**両者は同じ原点を使わなければならない**。
pub fn clamp_rect_to_page_box(
    rect: BBox,
    box_left: f64,
    box_bottom: f64,
    page_w: f64,
    page_h: f64,
) -> Option<BBox> {
    if rect.width <= 0.0 || rect.height <= 0.0 || page_w <= 0.0 || page_h <= 0.0 {
        return None;
    }
    let x0 = rect.x.max(box_left);
    let y0 = rect.y.max(box_bottom);
    let x1 = (rect.x + rect.width).min(box_left + page_w);
    let y1 = (rect.y + rect.height).min(box_bottom + page_h);
    let w = (x1 - x0).max(0.0);
    let h = (y1 - y0).max(0.0);
    if w * h < MIN_CLAMPED_AREA_RATIO * rect.width * rect.height {
        return None;
    }
    Some(BBox::new(x0, y0, w, h))
}

/// 埋込画像 bbox 群を図領域へ: 小さすぎる/大きすぎる矩形を除外し、近接矩形を union で
/// fixpoint マージし、上から順（y 降順→x 昇順）に返す。面積上位 `MAX_REGIONS_PER_PAGE` 件に
/// 制限する（超過分の有無は呼び出し側が入力数と出力数から判断して warning にする）。
pub fn merge_image_regions(rects: &[BBox], page_w: f64, page_h: f64) -> Vec<BBox> {
    let page_area = (page_w * page_h).max(1.0);
    let mut regions: Vec<BBox> = rects
        .iter()
        .copied()
        .filter(|r| r.width.min(r.height) >= MIN_DIM_PT)
        .filter(|r| (r.width * r.height) / page_area <= MAX_PAGE_AREA_RATIO)
        .collect();

    // fixpoint マージ: ギャップ MERGE_GAP_PT 以内で接する矩形を union に畳む。
    // 1 マージで要素が 1 個減るので必ず停止する。入力は MAX_RAW_RECTS_PER_PAGE で
    // 上限済み（呼び出し側）なので計算量は許容範囲。
    loop {
        let mut merged_any = false;
        'outer: for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                if gap_reachable(&regions[i], &regions[j], MERGE_GAP_PT) {
                    let b = regions.remove(j);
                    let a = regions[i];
                    regions[i] = union(a, b);
                    merged_any = true;
                    break 'outer;
                }
            }
        }
        if !merged_any {
            break;
        }
    }

    // 面積上位 MAX_REGIONS_PER_PAGE 件に制限してから、読み順（上→下・左→右）に並べる。
    if regions.len() > MAX_REGIONS_PER_PAGE {
        regions.sort_by(|a, b| {
            (b.width * b.height)
                .partial_cmp(&(a.width * a.height))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        regions.truncate(MAX_REGIONS_PER_PAGE);
    }
    regions.sort_by(|a, b| {
        let top_a = a.y + a.height;
        let top_b = b.y + b.height;
        top_b
            .partial_cmp(&top_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    regions
}

/// 2 矩形が gap 以内で接する（gap ぶん膨らませた矩形が交差する）か。
fn gap_reachable(a: &BBox, b: &BBox, gap: f64) -> bool {
    a.x - gap < b.x + b.width
        && b.x - gap < a.x + a.width
        && a.y - gap < b.y + b.height
        && b.y - gap < a.y + a.height
}

fn union(a: BBox, b: BBox) -> BBox {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    BBox::new(x0, y0, x1 - x0, y1 - y0)
}

/// PDF user space（左下原点・pt）の領域を、ページレンダリング画像（左上原点・px）の
/// `(x, y, width, height)` へ変換する。`box_left`/`box_bottom` はページ境界 box の原点
/// （CropBox が (0,0) 始まりでない PDF の補正）。画像範囲へクランプし、2px 未満に潰れたら
/// `None`（誤 crop より欠損）。
pub fn region_to_pixel_rect(
    bbox: BBox,
    box_left: f64,
    box_bottom: f64,
    page_w_pt: f64,
    page_h_pt: f64,
    img_w: u32,
    img_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if page_w_pt <= 0.0 || page_h_pt <= 0.0 || img_w == 0 || img_h == 0 {
        return None;
    }
    let scale_x = img_w as f64 / page_w_pt;
    let scale_y = img_h as f64 / page_h_pt;
    // 左下原点 → 左上原点: 上端の pt 座標（box 原点補正済み）を画像の y にする。
    let x0 = (bbox.x - box_left) * scale_x;
    let y0 = (box_bottom + page_h_pt - (bbox.y + bbox.height)) * scale_y;
    let x1 = (bbox.x + bbox.width - box_left) * scale_x;
    let y1 = (box_bottom + page_h_pt - bbox.y) * scale_y;

    let x0 = x0.max(0.0).min(img_w as f64);
    let y0 = y0.max(0.0).min(img_h as f64);
    let x1 = x1.max(0.0).min(img_w as f64);
    let y1 = y1.max(0.0).min(img_h as f64);
    let w = (x1 - x0).floor() as i64;
    let h = (y1 - y0).floor() as i64;
    if w < 2 || h < 2 {
        return None;
    }
    Some((x0.floor() as u32, y0.floor() as u32, w as u32, h as u32))
}

/// 図領域と caption ブロックを幾何ペアリングする。条件: 垂直ギャップが
/// `CAPTION_GAP_MAX_PT` 以内（caption は図の下でも上でもよい・わずかな重なりは許容）かつ
/// 水平重なりが短い方の幅の `CAPTION_OVERLAP_RATIO` 以上。**相互最近**（図から最近の caption
/// であり、かつその caption から最近の図でもある）のみ採用する（曖昧なら張らない）。
/// 戻り値は `(figures のインデックス, captions のインデックス)` のペア。
pub fn pair_captions(figures: &[BBox], captions: &[BBox]) -> Vec<(usize, usize)> {
    let dist = |f: &BBox, c: &BBox| -> Option<f64> {
        // 水平重なり。
        let overlap = (f.x + f.width).min(c.x + c.width) - f.x.max(c.x);
        if overlap < CAPTION_OVERLAP_RATIO * f.width.min(c.width) {
            return None;
        }
        // 垂直ギャップ（caption が下: 図の下端 − caption の上端 / caption が上: その逆）。
        let below = f.y - (c.y + c.height); // caption が図の下にあるとき ≥ 0
        let above = c.y - (f.y + f.height); // caption が図の上にあるとき ≥ 0
        let gap = below.max(above); // どちらか一方だけが正になる（重なりなら両方負）
        if gap > CAPTION_GAP_MAX_PT {
            return None;
        }
        // わずかな重なり（マージ後の領域が caption に食い込むケース）は許容し距離 0 扱い。
        Some(gap.max(0.0))
    };

    let best_caption: Vec<Option<(usize, f64)>> = figures
        .iter()
        .map(|f| {
            captions
                .iter()
                .enumerate()
                .filter_map(|(ci, c)| dist(f, c).map(|d| (ci, d)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        })
        .collect();
    let best_figure: Vec<Option<(usize, f64)>> = captions
        .iter()
        .map(|c| {
            figures
                .iter()
                .enumerate()
                .filter_map(|(fi, f)| dist(f, c).map(|d| (fi, d)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        })
        .collect();

    let mut pairs = Vec::new();
    for (fi, bc) in best_caption.iter().enumerate() {
        if let Some((ci, _)) = bc {
            if let Some((fi2, _)) = best_figure[*ci] {
                if fi2 == fi {
                    pairs.push((fi, *ci));
                }
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(x: f64, y: f64, w: f64, h: f64) -> BBox {
        BBox::new(x, y, w, h)
    }

    /// クランプは切られていなくても幅を `(x+w) - x` で組み直すので、10 進で書いた実データの
    /// 値は最下位ビットがずれることがある（出荷中のコードも同じ算術なので挙動の変化ではない）。
    #[track_caller]
    fn assert_bbox_near(got: Option<BBox>, want: BBox) {
        let got = got.expect("領域が捨てられた");
        let d = |a: f64, e: f64| (a - e).abs() < 1e-9;
        assert!(
            d(got.x, want.x)
                && d(got.y, want.y)
                && d(got.width, want.width)
                && d(got.height, want.height),
            "got {got:?} want {want:?}"
        );
    }

    // ---- merge_image_regions ----

    #[test]
    fn nearby_rects_merge_into_one_region() {
        // 8pt ギャップで縦に並ぶ 2 枚（サブ図 a/b）→ 1 領域。
        let regions = merge_image_regions(
            &[b(100.0, 500.0, 200.0, 100.0), b(100.0, 392.0, 200.0, 100.0)],
            595.0,
            842.0,
        );
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0], b(100.0, 392.0, 200.0, 208.0));
    }

    #[test]
    fn distant_rects_stay_separate_and_sorted_top_down() {
        let regions = merge_image_regions(
            &[b(100.0, 100.0, 200.0, 100.0), b(100.0, 600.0, 200.0, 100.0)],
            595.0,
            842.0,
        );
        assert_eq!(regions.len(), 2);
        // 上（y が大きい）の領域が先。
        assert_eq!(regions[0].y, 600.0);
        assert_eq!(regions[1].y, 100.0);
    }

    #[test]
    fn tiny_and_near_fullpage_rects_are_dropped() {
        let regions = merge_image_regions(
            &[
                b(10.0, 10.0, 12.0, 200.0),   // 短辺 12pt < 16pt: 飾り罫
                b(0.0, 0.0, 590.0, 840.0),    // ページ面積 ~99%: 背景
                b(100.0, 400.0, 200.0, 150.0), // 正当な図
            ],
            595.0,
            842.0,
        );
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0], b(100.0, 400.0, 200.0, 150.0));
    }

    #[test]
    fn caps_regions_per_page_keeping_largest() {
        // 交差しない離れた 10 領域。小さい 2 つが落ちる。
        let mut rects = Vec::new();
        for i in 0..8 {
            rects.push(b(50.0, 40.0 + 100.0 * i as f64, 150.0, 60.0));
        }
        rects.push(b(400.0, 40.0, 20.0, 20.0));
        rects.push(b(400.0, 200.0, 20.0, 20.0));
        let regions = merge_image_regions(&rects, 595.0, 842.0);
        assert_eq!(regions.len(), MAX_REGIONS_PER_PAGE);
        assert!(regions.iter().all(|r| r.width == 150.0));
    }

    #[test]
    fn merge_is_transitive_through_chain() {
        // a-b が近く b-c が近い → 3 枚で 1 領域（fixpoint）。
        let regions = merge_image_regions(
            &[
                b(100.0, 500.0, 100.0, 50.0),
                b(205.0, 500.0, 100.0, 50.0),
                b(310.0, 500.0, 100.0, 50.0),
            ],
            595.0,
            842.0,
        );
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0], b(100.0, 500.0, 310.0, 50.0));
    }

    // ---- effective_page_box_origin ----

    #[test]
    fn box_origin_is_the_crop_origin_when_crop_is_inside_media() {
        // 典型: MediaBox は原点ゼロ、CropBox が内側に寄っている。
        assert_eq!(
            effective_page_box_origin(Some((20.0, 30.0)), Some((0.0, 0.0))),
            (20.0, 30.0)
        );
    }

    #[test]
    fn box_origin_follows_media_when_crop_starts_at_zero() {
        // 実測 2 版（382 頁 + 10 頁）: CropBox は (0,0) 始まりだが MediaBox の原点が非ゼロ。
        // pdfium のページ寸法は交差の寸法なので、原点も交差＝MediaBox 側を採る。
        assert_eq!(
            effective_page_box_origin(Some((0.0, 0.0)), Some((77.811, 87.931))),
            (77.811, 87.931)
        );
    }

    #[test]
    fn box_origin_falls_back_to_the_single_box_or_zero() {
        assert_eq!(
            effective_page_box_origin(None, Some((10.0, -51.0))),
            (10.0, -51.0)
        );
        assert_eq!(
            effective_page_box_origin(Some((-4.15, -8.42)), None),
            (-4.15, -8.42)
        );
        // 両方取れない頁は pdfium が US Letter を原点ゼロで代替する（実測 64 頁）。
        assert_eq!(effective_page_box_origin(None, None), (0.0, 0.0));
    }

    // ---- clamp_rect_to_page_box ----

    #[test]
    fn clamp_keeps_rect_fully_inside_the_page_box() {
        let r = b(100.0, 400.0, 200.0, 150.0);
        assert_eq!(clamp_rect_to_page_box(r, 0.0, 0.0, 595.0, 842.0), Some(r));
    }

    #[test]
    fn clamp_trims_overhang_on_a_zero_origin_page() {
        // 原点ゼロのページでの挙動は据え置き（大半の PDF がこちら）。
        assert_eq!(
            clamp_rect_to_page_box(b(-20.0, 400.0, 200.0, 150.0), 0.0, 0.0, 595.0, 842.0),
            Some(b(0.0, 400.0, 180.0, 150.0))
        );
        assert_eq!(
            clamp_rect_to_page_box(b(500.0, 400.0, 150.0, 100.0), 0.0, 0.0, 595.0, 842.0),
            Some(b(500.0, 400.0, 95.0, 100.0))
        );
    }

    #[test]
    fn clamp_uses_box_origin_for_right_and_top_edges() {
        // CropBox [20 30 615 872]（原点 (20,30)・寸法 595x842）の雑誌 PDF。
        // 有効範囲は x∈[20,615] / y∈[30,872] であって [0,595] / [0,842] ではない。
        // 右端いっぱいの図（x∈[400,615]）。
        assert_eq!(
            clamp_rect_to_page_box(b(400.0, 100.0, 215.0, 300.0), 20.0, 30.0, 595.0, 842.0),
            Some(b(400.0, 100.0, 215.0, 300.0)),
            "原点を無視すると右 20pt が切り落とされる"
        );
        // 上端いっぱいの図（y∈[600,872]）。
        assert_eq!(
            clamp_rect_to_page_box(b(100.0, 600.0, 200.0, 272.0), 20.0, 30.0, 595.0, 842.0),
            Some(b(100.0, 600.0, 200.0, 272.0)),
            "原点を無視すると上 30pt が切り落とされる"
        );
    }

    #[test]
    fn clamp_trims_at_the_box_left_and_bottom_edges() {
        // 同じ CropBox [20 30 615 872]。左端・下端を跨ぐ矩形は box の内側で切る
        // （原点を落とすと裁ち落とし領域まで crop に入る）。
        assert_eq!(
            clamp_rect_to_page_box(b(0.0, 100.0, 300.0, 200.0), 20.0, 30.0, 595.0, 842.0),
            Some(b(20.0, 100.0, 280.0, 200.0))
        );
        assert_eq!(
            clamp_rect_to_page_box(b(100.0, 0.0, 200.0, 400.0), 20.0, 30.0, 595.0, 842.0),
            Some(b(100.0, 30.0, 200.0, 370.0))
        );
    }

    #[test]
    fn clamp_handles_a_negative_box_origin() {
        // MediaBox が `[0 -51 495.4 688.2]` の実データ（att46・deutsch1962 系）。
        // 原点を 0 で下限クリップすると、y<0 の帯にある可視の画像が高さ 0 に潰れて消える。
        assert_bbox_near(
            clamp_rect_to_page_box(b(458.4, -37.0, 25.3, 33.5), 0.0, -51.0, 495.4, 739.2),
            b(458.4, -37.0, 25.3, 33.5),
        );
        // 下端 -51 より下は box の外なので落ちる。
        assert_eq!(
            clamp_rect_to_page_box(b(458.4, -90.0, 25.3, 33.5), 0.0, -51.0, 495.4, 739.2),
            None
        );
        // 左端も負になりうる（att41 の CropBox `[-4.1494 -8.41611 480.534 688.849]`）。
        assert_bbox_near(
            clamp_rect_to_page_box(b(-3.0, 100.0, 103.0, 50.0), -4.15, -8.42, 484.68, 697.27),
            b(-3.0, 100.0, 103.0, 50.0),
        );
    }

    #[test]
    fn clamp_keeps_a_figure_that_the_zero_origin_range_would_delete() {
        // 原点が大きい PDF（CropBox の下端 300pt）では、ページ上半分の図が
        // 「[0,page_h] の外」と判定されて面積 0 に潰れ、図そのものが消えていた。
        assert_eq!(
            clamp_rect_to_page_box(b(100.0, 700.0, 200.0, 90.0), 0.0, 300.0, 595.0, 500.0),
            Some(b(100.0, 700.0, 200.0, 90.0))
        );
    }

    #[test]
    fn clamp_drops_a_rect_that_lies_outside_the_page_box() {
        // box の下（トンボ・裁ち落とし領域）にある画像。原点を無視すると素通りして
        // 「ページに表示されない内容の crop」を作ってしまう。
        assert_eq!(
            clamp_rect_to_page_box(b(100.0, 100.0, 200.0, 100.0), 0.0, 300.0, 595.0, 500.0),
            None
        );
    }

    #[test]
    fn clamp_drops_rects_trimmed_below_the_area_ratio() {
        // 元面積の半分未満まで削られたら捨てる（変換異常の兆候・誤配置 crop 回避）。
        assert_eq!(
            clamp_rect_to_page_box(b(500.0, 400.0, 300.0, 100.0), 0.0, 0.0, 595.0, 842.0),
            None
        );
        // 半分を超えて残るなら採る。
        assert_eq!(
            clamp_rect_to_page_box(b(400.0, 400.0, 300.0, 100.0), 0.0, 0.0, 595.0, 842.0),
            Some(b(400.0, 400.0, 195.0, 100.0))
        );
    }

    #[test]
    fn clamp_rejects_degenerate_input() {
        assert_eq!(
            clamp_rect_to_page_box(b(100.0, 100.0, 0.0, 50.0), 0.0, 0.0, 595.0, 842.0),
            None
        );
        assert_eq!(
            clamp_rect_to_page_box(b(100.0, 100.0, 50.0, -10.0), 0.0, 0.0, 595.0, 842.0),
            None
        );
        assert_eq!(
            clamp_rect_to_page_box(b(100.0, 100.0, 50.0, 50.0), 0.0, 0.0, 0.0, 842.0),
            None
        );
    }

    // ---- region_to_pixel_rect ----

    #[test]
    fn converts_bottom_left_pt_to_top_left_px() {
        // 595x842pt のページを幅 1190px（scale 2.0）でレンダリングした場合。
        // 左下 (100, 100) 幅 200 高さ 50 → 上端 pt = 150 → px y = (842-150)*2 = 1384。
        let r = region_to_pixel_rect(b(100.0, 100.0, 200.0, 50.0), 0.0, 0.0, 595.0, 842.0, 1190, 1684);
        assert_eq!(r, Some((200, 1384, 400, 100)));
    }

    #[test]
    fn compensates_nonzero_page_box_origin() {
        // CropBox [20 30 615 872]（原点 (20,30)・サイズ 595x842）の雑誌 PDF。
        // user space の (120, 130) は box 内では (100, 100) に相当する。
        let r = region_to_pixel_rect(
            b(120.0, 130.0, 200.0, 50.0),
            20.0,
            30.0,
            595.0,
            842.0,
            1190,
            1684,
        );
        assert_eq!(r, Some((200, 1384, 400, 100)));
    }

    #[test]
    fn clamps_to_image_and_rejects_degenerate() {
        // ページ外へはみ出す矩形はクランプされる。
        let r = region_to_pixel_rect(b(-50.0, 800.0, 100.0, 100.0), 0.0, 0.0, 595.0, 842.0, 595, 842)
            .unwrap();
        assert_eq!((r.0, r.1), (0, 0));
        assert_eq!(r.2, 50); // x∈[-50,50] → [0,50]
        // 完全にページ外 → クランプで潰れて None。
        assert_eq!(
            region_to_pixel_rect(b(700.0, 100.0, 50.0, 50.0), 0.0, 0.0, 595.0, 842.0, 595, 842),
            None
        );
        // ゼロサイズ画像 → None。
        assert_eq!(
            region_to_pixel_rect(b(10.0, 10.0, 50.0, 50.0), 0.0, 0.0, 595.0, 842.0, 0, 0),
            None
        );
    }

    // ---- pair_captions ----

    #[test]
    fn caption_below_figure_pairs() {
        let figures = vec![b(100.0, 400.0, 300.0, 200.0)];
        let captions = vec![b(100.0, 360.0, 300.0, 24.0)]; // 図の 16pt 下
        assert_eq!(pair_captions(&figures, &captions), vec![(0, 0)]);
    }

    #[test]
    fn caption_above_figure_pairs() {
        let figures = vec![b(100.0, 300.0, 300.0, 200.0)];
        let captions = vec![b(100.0, 510.0, 300.0, 24.0)]; // 図の 10pt 上
        assert_eq!(pair_captions(&figures, &captions), vec![(0, 0)]);
    }

    #[test]
    fn distant_or_nonoverlapping_captions_do_not_pair() {
        let figures = vec![b(100.0, 400.0, 300.0, 200.0)];
        // 垂直に遠い（100pt 下）。
        assert!(pair_captions(&figures, &[b(100.0, 276.0, 300.0, 24.0)]).is_empty());
        // 水平に重ならない（別カラム）。
        assert!(pair_captions(&figures, &[b(450.0, 360.0, 140.0, 24.0)]).is_empty());
    }

    #[test]
    fn ambiguous_caption_resolved_by_mutual_nearest() {
        // 2 図が縦に並び、caption は下の図のすぐ下。
        let figures = vec![b(100.0, 600.0, 300.0, 150.0), b(100.0, 350.0, 300.0, 150.0)];
        let captions = vec![b(100.0, 310.0, 300.0, 24.0)];
        assert_eq!(pair_captions(&figures, &captions), vec![(1, 0)]);
    }

    #[test]
    fn two_figures_two_captions_pair_independently() {
        let figures = vec![b(100.0, 600.0, 300.0, 120.0), b(100.0, 300.0, 300.0, 120.0)];
        let captions = vec![b(100.0, 560.0, 300.0, 24.0), b(100.0, 260.0, 300.0, 24.0)];
        let mut pairs = pair_captions(&figures, &captions);
        pairs.sort();
        assert_eq!(pairs, vec![(0, 0), (1, 1)]);
    }

    #[test]
    fn overlapping_caption_is_tolerated_as_distance_zero() {
        // マージ後の図領域が caption に 4pt 食い込むケース。
        let figures = vec![b(100.0, 400.0, 300.0, 200.0)];
        let captions = vec![b(100.0, 380.0, 300.0, 24.0)]; // 上端 404 > 図下端 400
        assert_eq!(pair_captions(&figures, &captions), vec![(0, 0)]);
    }
}
