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
