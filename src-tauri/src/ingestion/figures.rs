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
/// XObjectForm 内 Image の座標解釈（[`calibrate_form_child_space`]）が要求する最低の面積包含率。
/// これを両解釈とも下回った form は画像ごと捨てる（誤配置 crop より欠損）。
pub const FORM_CONTAINMENT_MIN: f64 = 0.9;
/// 入れ子 XObjectForm を辿る最大の深さ（実ライブラリの実測は 2）。
pub const MAX_FORM_DEPTH: usize = 8;
/// 1 つの XObjectForm の下で走査する子オブジェクト数の上限（Phase 8d-2）。
/// 8d-8 は「Image を持たない巨大な form 木は最後まで辿る」という上限なしの走査を残していた。
/// ベクター図では form の中身が数千の path になりうるので、ここで打ち切る
/// （打ち切ると「Image は無い」と誤って結論しうるが、その form を図にしない側に倒れる）。
pub const MAX_FORM_CHILDREN_SCANNED: usize = 4096;

// ---- Phase 8d-2: ベクター図（tikz/pgf）の領域検出 ----

/// 1 ページの生 path オブジェクト数の上限（Phase 8d-2）。超えるページはベクター検出を
/// スキップする。**[`MAX_RAW_RECTS_PER_PAGE`] とは別の定数**にしてある ── あちらは
/// 「そのページの図領域を丸ごと捨てる」判定にも使われており（`should_scan_forms`）、
/// path の本数を混ぜると 8a/8d-8 の出力が動いてしまう。
///
/// 512 なのは fixpoint マージが最悪 O(n^3) だから（n=256 で ~5.6e6 回・n=512 で ~4.5e7 回の
/// `gap_reachable`）。実ライブラリには 1 ページ 5,874 本の版（vid222）が実在し、そこは捨てる。
pub const MAX_RAW_PATHS_PER_PAGE: usize = 512;
/// ベクタークラスタを畳むときのギャップ（pt）。ラスタ（[`MERGE_GAP_PT`]）と同値だが、
/// **意味が違う**ので別定数にする ── ラスタは「1 矩形 = 図の一部」、path は「1 矩形 = 線 1 本」。
pub const VECTOR_MERGE_GAP_PT: f64 = 12.0;
/// ベクタークラスタとして採る最小の短辺（pt）。罫線・分数線・下線が単体で図になるのを防ぐ。
/// **マージの前ではなく後に掛ける**（[`MIN_DIM_PT`] と逆）── ベクター図を構成する線は
/// 1 本ずつ見れば必ず細く、前段で落とすと図が丸ごと消える。
///
/// ラスタ側の [`MIN_DIM_PT`](16pt) より厳しくしてある。実測（生存 138 版）で 16pt に緩めても
/// 増える領域はコーパス全体で **5 件だけ**で、しかもそのうち少なくとも 1 件は図そのものではなく
/// **図の断片**（他の要素が [`VECTOR_MERGE_GAP_PT`] より遠くて畳まれなかったもの）だった。
/// 得るものが小さく質も低いので、`24` を採る（設計 §16「誤検出より欠損」）。
pub const VECTOR_MIN_DIM_PT: f64 = 24.0;
/// クラスタの面積のうち本文ブロックが占めてよい上限。これを超えたら「本文段を図と誤認した」
/// とみなして捨てる（分数線・脚注罫・段落罫が本文全体を 1 クラスタに畳む形が実データにある）。
pub const VECTOR_MAX_PROSE_COVER: f64 = 0.35;
/// ページ幅（高さ）のこの割合以上に伸びた**細い**矩形は組版の罫線とみなしてクラスタから外す。
pub const VECTOR_RULE_SPAN_RATIO: f64 = 0.6;
/// 上の判定で「細い」とみなす最大の厚み（pt）。図の枠線は太さではなく**面**を持つので当たらない。
pub const VECTOR_RULE_MAX_THICKNESS_PT: f64 = 3.0;
/// 既存のラスタ図領域とこれ以上重なったベクタークラスタは捨てる。
/// **union はしない**（§2.6-2）── ラスタ図に隣接する軸・枠線を畳み込むと既存 figure の bbox が
/// 動き、8c の alt text carry（crop の sha256 キー）が壊れて再課金になる。
pub const VECTOR_RASTER_OVERLAP_MAX: f64 = 0.2;
/// RGB 各成分がこの値以上なら「紙と見分けがつかない白」とみなす（[`path_has_visible_ink`]）。
pub const WHITE_INK_MIN: u8 = 250;
/// ラスタ図領域（埋込画像の bbox・Phase 8a/8d-8）の推定信頼度。
pub const RASTER_REGION_CONFIDENCE: f64 = 0.6;
/// ベクター図領域（path クラスタ・Phase 8d-2）の推定信頼度。ラスタより 1 段低い ──
/// bbox が「そこに画像がある」という事実ではなく、線の寄せ集めからの推定だから。
/// **caption と結ばれたものしか作らない**ので、これ以上の段は設けていない。
pub const VECTOR_REGION_CONFIDENCE: f64 = 0.5;

impl RegionSource {
    /// 図ノード（と `caption_of` 辺）に載せる推定信頼度。
    pub fn confidence(self) -> f64 {
        match self {
            RegionSource::Raster => RASTER_REGION_CONFIDENCE,
            RegionSource::Vector => VECTOR_REGION_CONFIDENCE,
        }
    }
}

/// path オブジェクトの色（RGBA・各 0–255）。
pub type Rgba = (u8, u8, u8, u8);

/// この色は紙の上でインクとして見えるか（不透明で、かつ白に極めて近くない）。
fn is_ink(c: Rgba) -> bool {
    c.3 > 0 && !(c.0 >= WHITE_INK_MIN && c.1 >= WHITE_INK_MIN && c.2 >= WHITE_INK_MIN)
}

/// この path はベクター図の手掛かりになるか（塗りか線のどちらかが**見える色**を持つか）。
///
/// **白抜きは実データにある。** vid148 p15 には線幅 113pt / 138pt の**純白ストローク**
/// （`stroke_color = (255,255,255,255)`）が 4 本あり、線幅ぶん膨らんだその bbox は
/// 532 × 276pt ＝ ページを斜めに跨ぐ。組版が版面を消すために引いた見えない線で、
/// これを残すと 2 つの図と caption と本文が 1 つのクラスタに畳まれる（実測でそうなった）。
///
/// 「白は必ず不可視」ではない（色地の上の白抜き文字・白い図形はありうる）が、
/// 論文 PDF では紙が白なので**欠損側に倒す**（設計 §16）。塗りモードが `None` のときは
/// 塗り色を見ない ── 実データでは塗らない path も黒い塗り色を持っている。
pub fn path_has_visible_ink(stroked: bool, stroke: Rgba, filled: bool, fill: Rgba) -> bool {
    (stroked && is_ink(stroke)) || (filled && is_ink(fill))
}

/// 2 矩形の交差（重ならなければ `None`）。path の bbox にクリップパスを掛けるのに使う。
///
/// pdfium の `bounds()` は**クリップパスを考慮しない**ので、tikz/pgfplots が
/// 「巨大なパスを図の枠で切り出す」形にしていると生の巨大矩形が返る。
/// 実データ（vid238 p6）には 96.9 × 1,845.9pt（ページ高の 2 倍以上）の path があり、
/// クリップを掛けると 96.9 × 124.6pt の図の中身になる。
pub fn intersect_rect(a: BBox, b: BBox) -> Option<BBox> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    (x1 > x0 && y1 > y0).then(|| BBox::new(x0, y0, x1 - x0, y1 - y0))
}

/// 図領域の由来（Phase 8d-2）。crop のファイル名・confidence・caption ペアリングの段が
/// これで分かれる。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionSource {
    /// 埋込画像（トップレベル Image + XObjectForm 内 Image）由来。Phase 8a / 8d-8。
    Raster,
    /// ベクター path のクラスタ由来。Phase 8d-2。
    Vector,
}

/// 1 ページの図領域 1 個（由来つき）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FigureRegion {
    pub bbox: BBox,
    pub source: RegionSource,
}

/// PDF のアフィン変換行列 `[a, b, c, d, e, f]`。点の変換は
/// `x' = a·x + c·y + e` / `y' = b·x + d·y + f`（PDF Reference 1.7 §4.2.3）。
pub type Affine = [f64; 6];

/// 恒等変換。
pub const AFFINE_IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// XObjectForm の子オブジェクトの `bounds()` がどの座標空間で返るかの解釈。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormChildSpace {
    /// すでにページ空間（form の行列を当ててはいけない）。
    PageSpace,
    /// form のコンテンツ空間（ページ空間へ移すには合成行列を当てる）。
    FormLocal,
}

/// `inner` を先に、`outer` を後に適用する合成（行ベクトル規約なので行列積は `inner × outer`）。
///
/// 入れ子の XObjectForm で使う。内側 form の子は「内側 form のコンテンツ空間」に居るので、
/// 内側 form の行列 → 外側 form の行列 の順に当てるとページ空間になる。
pub fn compose_affine(inner: Affine, outer: Affine) -> Affine {
    let [ia, ib, ic, id, ie, if_] = inner;
    let [oa, ob, oc, od, oe, of_] = outer;
    [
        ia * oa + ib * oc,
        ia * ob + ib * od,
        ic * oa + id * oc,
        ic * ob + id * od,
        ie * oa + if_ * oc + oe,
        ie * ob + if_ * od + of_,
    ]
}

/// 軸並行 bbox に行列を当て、変換後の 4 隅の**外接**矩形を返す
/// （回転・斜行を含む行列では変換後が軸並行にならないため）。
pub fn transform_bbox(rect: BBox, m: Affine) -> BBox {
    let [a, b, c, d, e, f] = m;
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
    let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for (x, y) in corners {
        let tx = a * x + c * y + e;
        let ty = b * x + d * y + f;
        min_x = min_x.min(tx);
        min_y = min_y.min(ty);
        max_x = max_x.max(tx);
        max_y = max_y.max(ty);
    }
    BBox::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

/// `rect` の面積のうち `container` に入っている割合（0.0..=1.0）。面積ゼロの矩形は 0.0。
pub fn containment_ratio(rect: BBox, container: BBox) -> f64 {
    let area = rect.width.max(0.0) * rect.height.max(0.0);
    if area <= 0.0 {
        return 0.0;
    }
    intersection_area(rect, container) / area
}

fn intersection_area(a: BBox, b: BBox) -> f64 {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.width).min(b.x + b.width);
    let y1 = (a.y + a.height).min(b.y + b.height);
    (x1 - x0).max(0.0) * (y1 - y0).max(0.0)
}

/// Phase 8d-8 の**自己校正**: XObjectForm 内の Image の bbox がどの座標空間で返るかを、
/// 仮説で決めずに測って選ぶ。
///
/// `candidates` は子ごとの `(そのままの矩形, 合成行列を当てた矩形)`。両解釈について
/// 「form 自身の bounds に入っている面積 / 候補矩形の総面積」を出し、高い方を採る。
/// **どちらも [`FORM_CONTAINMENT_MIN`] 未満ならその form の画像は捨てる**（`None`）──
/// 座標空間の推定が外れている証拠なので、誤配置 crop を作るより欠損させる（§16）。
///
/// 面積で重み付けするのは、子が 1 枚でも複数枚でも同じ尺度にするため（多数決だと
/// 小さいアイコン多数が大きな図 1 枚を押し切る）。同率なら [`FormChildSpace::FormLocal`] を採る
/// ── pdfium は入れ子オブジェクトの bounds を「それが属するコンテンツストリームの空間」で
/// 返すので、そちらがデータモデルの含意であり、`PageSpace` は
/// 「合成行列を当てると form の外へ出てしまう」と実測できたときだけ勝たせる。
///
/// **片方の解釈が「測れない」ときは、それを他方の証拠にしない。** どちらかの候補矩形の総面積が
/// 0（合成行列が特異で潰れた・生 bbox が退化）なら比較が成立しないので `None` を返す。
/// ここを「面積 0 → 包含率 0」に落とすと、`FormLocal` 側が測れないだけの form で
/// `PageSpace` が 1.0 を取って勝ってしまい、form ローカル座標の生矩形がそのまま
/// ページ座標として採用される（＝この関数が防ぐはずの誤配置 crop を自分で作る）。
///
/// 実測（生存 138 版 7,345 頁・form 内 Image 109 枚・form 43 個）: 結論は
/// **`FormLocal` 43 個 / `PageSpace` 0 個 / 棄却 0 個**。子 1 枚単位で見ると、合成行列でだけ
/// form に収まるのが 96 枚・どちらの解釈でも収まるのが 13 枚・どちらでも収まらないのが 0 枚。
pub fn calibrate_form_child_space(
    candidates: &[(BBox, BBox)],
    form_bounds: BBox,
) -> Option<FormChildSpace> {
    let (mut area_page, mut inter_page) = (0.0f64, 0.0f64);
    let (mut area_local, mut inter_local) = (0.0f64, 0.0f64);
    for (as_page, as_local) in candidates {
        area_page += as_page.width.max(0.0) * as_page.height.max(0.0);
        inter_page += intersection_area(*as_page, form_bounds);
        area_local += as_local.width.max(0.0) * as_local.height.max(0.0);
        inter_local += intersection_area(*as_local, form_bounds);
    }
    // 片方でも面積が測れなければ比較そのものが成立しない（NaN / 無限大もここで落とす）。
    let measurable = |area: f64| area.is_finite() && area > 0.0;
    if !measurable(area_page) || !measurable(area_local) {
        return None;
    }
    let (r_page, r_local) = (inter_page / area_page, inter_local / area_local);
    if r_local >= r_page && r_local >= FORM_CONTAINMENT_MIN {
        Some(FormChildSpace::FormLocal)
    } else if r_page >= FORM_CONTAINMENT_MIN {
        Some(FormChildSpace::PageSpace)
    } else {
        None
    }
}

/// そのページで XObjectForm の中まで辿ってよいか（Phase 8d-8）。
///
/// **どちらの条件も「ページごと図領域を捨てる」既存の判定と同じもの**で、そういうページでは
/// form を辿っても結果が捨てられるだけ。ここで先に止めることで、**回転ページと画像過多ページの
/// 出力・warning が Phase 8a 当時から 1 ビットも変わらない**ことを保証する
/// （`extract_page_image_regions` は form 由来の矩形をこの述語が真のときしか足さない）。
pub fn should_scan_forms(rotation_deg: f64, raw_image_count: usize) -> bool {
    rotation_deg == 0.0 && raw_image_count <= MAX_RAW_RECTS_PER_PAGE
}

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
/// ページ境界 box（`CropBox ∩ MediaBox`）の**寸法**でしかない。したがって原点が非ゼロの PDF
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

    fixpoint_merge(&mut regions, MERGE_GAP_PT);

    // 面積上位 MAX_REGIONS_PER_PAGE 件に制限してから、読み順（上→下・左→右）に並べる。
    keep_largest(&mut regions, MAX_REGIONS_PER_PAGE);
    sort_reading_order(&mut regions);
    regions
}

/// ギャップ `gap` 以内で接する矩形を union に畳む（fixpoint）。
/// 1 マージで要素が 1 個減るので必ず停止する。最悪 O(n^3) なので、呼び出し側が入力数を
/// 必ず上限する（[`MAX_RAW_RECTS_PER_PAGE`] / [`MAX_RAW_PATHS_PER_PAGE`]）。
fn fixpoint_merge(regions: &mut Vec<BBox>, gap: f64) {
    loop {
        let mut merged_any = false;
        'outer: for i in 0..regions.len() {
            for j in (i + 1)..regions.len() {
                if gap_reachable(&regions[i], &regions[j], gap) {
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
}

/// 面積上位 `max` 件に切り詰める（`max` 以下なら並びを一切触らない）。
fn keep_largest(regions: &mut Vec<BBox>, max: usize) {
    if regions.len() <= max {
        return;
    }
    regions.sort_by(|a, b| {
        (b.width * b.height)
            .partial_cmp(&(a.width * a.height))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    regions.truncate(max);
}

/// 読み順（上→下・左→右）に並べる。
fn sort_reading_order(regions: &mut [BBox]) {
    regions.sort_by(|a, b| {
        let top_a = a.y + a.height;
        let top_b = b.y + b.height;
        top_b
            .partial_cmp(&top_a)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
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

// ---- Phase 8d-2: ベクター図領域の検出と受理 ----

/// 矩形列の外接矩形（空なら `None`）。
pub fn bbox_hull(rects: &[BBox]) -> Option<BBox> {
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for r in rects {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.width);
        y1 = y1.max(r.y + r.height);
    }
    (x1 > x0 && y1 > y0).then(|| BBox::new(x0, y0, x1 - x0, y1 - y0))
}

/// その XObjectForm を「中身がベクターだけ」とみなして、**見える子の外接矩形**を
/// 図領域の候補にしてよいか（Phase 8d-2）。
///
/// **Image を子孫に持つ form は対象外。** 8d-8 がその画像を既にラスタ矩形として拾っているので、
/// form 全体も足すと**同じ図を二重に数える**（`fig-` と `vec-` の crop が 2 枚出て
/// Vision の課金も 2 回になる）。見える path が 1 つも無い form も対象外
/// （白抜きだけ・テキストだけの form）。
pub fn form_is_vector_only(image_children: usize, visible_path_children: usize) -> bool {
    image_children == 0 && visible_path_children > 0
}

/// そのページで path オブジェクトを走査してよいか（Phase 8d-2）。
///
/// [`should_scan_forms`] とは**別の述語**にしてある。あちらは「回転ページ・画像過多ページの
/// 出力が 8a 当時から 1 ビットも変わらない」を保証する門で、そこへ path の本数を混ぜると
/// その保証が壊れる（画像 3 枚 + path 600 本のページが新たに丸ごと skip されうる）。
pub fn should_scan_vector_paths(rotation_deg: f64, raw_path_count: usize) -> bool {
    rotation_deg == 0.0 && raw_path_count <= MAX_RAW_PATHS_PER_PAGE
}

/// path 矩形群をベクター図領域の候補（クラスタ）に畳む。
///
/// [`merge_image_regions`] と**フィルタの順序が逆**なのが要点。埋込画像は「1 矩形 = 図の一部」
/// なので小さすぎる矩形をマージ前に落としてよいが、ベクター図は「1 矩形 = 線 1 本」なので
/// マージ前に短辺で落とすと図を構成する線が全部消えて図そのものが無くなる
/// （実データ: vid181 p15 の図は 0.80pt 厚の直線 4 本だけでできている）。
/// したがって短辺フィルタ（[`VECTOR_MIN_DIM_PT`]）は**マージ後のクラスタ**に掛ける。
///
/// マージ前に落とすのはページ面積の [`MAX_PAGE_AREA_RATIO`] を超える矩形だけ ── これは
/// ページ枠・背景と、**クリップで小さく見せている巨大パス**（pdfium の `bounds()` は
/// クリップを考慮しないので生の巨大矩形が返る）を除くためで、残すと必ず全面クラスタになる。
pub fn cluster_vector_rects(rects: &[BBox], page_w: f64, page_h: f64) -> Vec<BBox> {
    let page_area = (page_w * page_h).max(1.0);
    let mut clusters: Vec<BBox> = rects
        .iter()
        .copied()
        .filter(|r| r.width > 0.0 || r.height > 0.0)
        .filter(|r| (r.width * r.height) / page_area <= MAX_PAGE_AREA_RATIO)
        .filter(|r| !is_page_rule(*r, page_w, page_h))
        .collect();
    fixpoint_merge(&mut clusters, VECTOR_MERGE_GAP_PT);
    clusters.retain(|c| {
        c.width.min(c.height) >= VECTOR_MIN_DIM_PT
            && (c.width * c.height) / page_area <= MAX_PAGE_AREA_RATIO
    });
    sort_reading_order(&mut clusters);
    clusters
}

/// ページを横断する**細い**矩形か（ヘッダ/フッタ罫・段組の境界罫・表の横罫）。
///
/// クラスタから外すのは「見た目に効く」からではなく、**マージの橋になる**からである。
/// ページ幅いっぱいの罫線は左段と右段の両方に [`VECTOR_MERGE_GAP_PT`] 以内で接するので、
/// 1 本残るだけで無関係な 2 つの図と caption と本文が 1 クラスタに畳まれる
/// （実データ: vid146 p6 では欄外 caption まで crop に入っていた）。
///
/// 図の枠線は当たらない ── 枠は 4 辺を持つ 1 個の矩形として返るので短辺が
/// [`VECTOR_RULE_MAX_THICKNESS_PT`] を超える。当たるのは「線 1 本」の path だけ。
fn is_page_rule(r: BBox, page_w: f64, page_h: f64) -> bool {
    (r.height < VECTOR_RULE_MAX_THICKNESS_PT && r.width >= VECTOR_RULE_SPAN_RATIO * page_w)
        || (r.width < VECTOR_RULE_MAX_THICKNESS_PT && r.height >= VECTOR_RULE_SPAN_RATIO * page_h)
}

/// `region` の面積のうち `others` に覆われている割合（0.0..=1.0）。
///
/// `others` どうしの重なりは二重に数えるので**過大評価**になる（上限 1.0 で頭打ち）。
/// 使い道が 2 つとも「覆われすぎているものを捨てる」＝**欠損側に倒す**判定なので、
/// 過大評価は安全側に効く（構造ブロックどうしはほぼ重ならないので実害も小さい）。
pub fn coverage_ratio(region: BBox, others: &[BBox]) -> f64 {
    let a = region.width.max(0.0) * region.height.max(0.0);
    if a <= 0.0 {
        return 1.0; // 面積ゼロの領域は「全部覆われている」扱い＝捨てる側
    }
    let covered: f64 = others.iter().map(|o| intersection_area(region, *o)).sum();
    (covered / a).min(1.0)
}

/// `captions` のうち `figures` と[`pair_captions`]で結ばれなかったもののインデックス。
///
/// 8d-2 のゲートに使う ── **ラスタ図と結ばれた caption が 1 つでも残っていれば
/// そのページで path を走査する意味がある**、の判定と、ベクター領域を結ぶ相手の集合。
pub fn unpaired_caption_indices(figures: &[BBox], captions: &[BBox]) -> Vec<usize> {
    let paired: std::collections::HashSet<usize> = pair_captions(figures, captions)
        .into_iter()
        .map(|(_, ci)| ci)
        .collect();
    (0..captions.len()).filter(|i| !paired.contains(i)).collect()
}

/// 2 段の caption ペアリング（Phase 8d-2）。
///
/// **ラスタ図を先に確定させ、ベクター領域は余った caption とだけ結ぶ。** 戻り値は
/// `(raster ++ vector` の連結順のインデックス, captions のインデックス)。
///
/// 1 段でまとめて [`pair_captions`] に掛けてはいけない。相互最近はページ全体の図集合に対する
/// **大域的な**計算なので、ラスタ図と一切重ならないベクター領域を 1 個足しただけで、
/// 既存のラスタ図が持っていた `caption_of` 辺を奪って消しうる（[`VECTOR_RASTER_OVERLAP_MAX`]
/// の重なり判定ではこれを防げない ── 重なっていなくても「より近い」だけで奪える）。
/// 2 段にすると、ベクター領域を足しても**ラスタ側のペアは定義から不変**になる。
pub fn pair_captions_two_stage(
    raster: &[BBox],
    vector: &[BBox],
    captions: &[BBox],
) -> Vec<(usize, usize)> {
    let mut pairs = pair_captions(raster, captions);
    if vector.is_empty() {
        return pairs;
    }
    let leftover = unpaired_caption_indices(raster, captions);
    let leftover_boxes: Vec<BBox> = leftover.iter().map(|i| captions[*i]).collect();
    for (vi, li) in pair_captions(vector, &leftover_boxes) {
        pairs.push((raster.len() + vi, leftover[li]));
    }
    pairs
}

/// 1 ページの図領域列を組む（Phase 8d-2）。**ラスタ側は [`merge_image_regions`] の出力
/// そのものを順序ごと先頭に置き**、その後ろにベクター領域を足す。
///
/// ベクター領域の受理規則は 4 つ。どれも「誤検出より欠損」（設計 §16）に倒してある:
///
/// 1. クラスタの面積を本文ブロックが [`VECTOR_MAX_PROSE_COVER`] を超えて占めるなら捨てる。
/// 2. 既存のラスタ図領域と [`VECTOR_RASTER_OVERLAP_MAX`] を超えて重なるなら捨てる（union しない）。
/// 3. **ラスタと結ばれなかった caption と相互最近でペアになったものだけ**採る。
///    caption を持たないベクター図は取り逃すが、本文の罫線クラスタと区別する手段が無い。
/// 4. 1 ページの上限 [`MAX_REGIONS_PER_PAGE`] の**残り枠**にだけ入れる（面積上位）。
///    ラスタを押し出さないのは規則ではなく構造 ── ラスタ側の列に一切触らないため。
pub fn compose_figure_regions(
    raster_regions: &[BBox],
    vector_rects: &[BBox],
    prose_blocks: &[BBox],
    captions: &[BBox],
    page_w: f64,
    page_h: f64,
) -> Vec<FigureRegion> {
    let mut out: Vec<FigureRegion> = raster_regions
        .iter()
        .map(|b| FigureRegion {
            bbox: *b,
            source: RegionSource::Raster,
        })
        .collect();
    let budget = MAX_REGIONS_PER_PAGE.saturating_sub(raster_regions.len());
    if vector_rects.is_empty() || budget == 0 {
        return out;
    }
    let leftover = unpaired_caption_indices(raster_regions, captions);
    if leftover.is_empty() {
        return out;
    }
    let mut clusters = cluster_vector_rects(vector_rects, page_w, page_h);
    clusters.retain(|c| {
        coverage_ratio(*c, prose_blocks) <= VECTOR_MAX_PROSE_COVER
            && coverage_ratio(*c, raster_regions) <= VECTOR_RASTER_OVERLAP_MAX
    });
    if clusters.is_empty() {
        return out;
    }
    let leftover_boxes: Vec<BBox> = leftover.iter().map(|i| captions[*i]).collect();
    let paired: std::collections::HashSet<usize> = pair_captions(&clusters, &leftover_boxes)
        .into_iter()
        .map(|(fi, _)| fi)
        .collect();
    let mut accepted: Vec<BBox> = clusters
        .into_iter()
        .enumerate()
        .filter(|(i, _)| paired.contains(i))
        .map(|(_, c)| c)
        .collect();
    keep_largest(&mut accepted, budget);
    sort_reading_order(&mut accepted);
    out.extend(accepted.into_iter().map(|bbox| FigureRegion {
        bbox,
        source: RegionSource::Vector,
    }));
    out
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

    // ---- compose_affine / transform_bbox / containment_ratio（8d-8） ----

    /// 実データ（att134 p1）の form 行列: 0.4489 倍 + (317.014, 450.913) 平行移動。
    const FORM_M: Affine = [0.4489, 0.0, 0.0, 0.4489, 317.014, 450.913];

    #[test]
    fn transform_bbox_applies_the_pdf_point_formula() {
        // 恒等は素通り。
        assert_eq!(
            transform_bbox(b(10.0, 20.0, 30.0, 40.0), AFFINE_IDENTITY),
            b(10.0, 20.0, 30.0, 40.0)
        );
        // 拡大 + 平行移動（実データの form 行列）。0.4489·187.2 + 317.014 = 401.048。
        let got = transform_bbox(b(187.2, 1.1273, 44.9455, 44.9455), FORM_M);
        let d = |a: f64, e: f64| (a - e).abs() < 1e-3;
        assert!(
            d(got.x, 401.048) && d(got.y, 451.419) && d(got.width, 20.176) && d(got.height, 20.176),
            "{got:?}"
        );
    }

    #[test]
    fn transform_bbox_takes_the_hull_under_rotation() {
        // 90° 回転 `[0 1 -1 0 0 0]`: x' = -y / y' = x。b と c を取り違えると符号が反転する。
        let got = transform_bbox(b(1.0, 2.0, 3.0, 4.0), [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        assert_eq!(got, b(-6.0, 1.0, 4.0, 3.0));
    }

    #[test]
    fn compose_affine_applies_inner_first() {
        // 内側 = 2 倍、外側 = (10, 20) 平行移動。点 (1,1) は (2,2) を経て (12,22)。
        let inner = [2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
        let outer = [1.0, 0.0, 0.0, 1.0, 10.0, 20.0];
        let composed = compose_affine(inner, outer);
        let p = transform_bbox(b(1.0, 1.0, 0.0, 0.0), composed);
        assert_eq!((p.x, p.y), (12.0, 22.0));
        // 逆順は別物（合成の順序を入れ替える変異を捕まえる）: (1,1) → (11,21) → (22,42)。
        let swapped = transform_bbox(b(1.0, 1.0, 0.0, 0.0), compose_affine(outer, inner));
        assert_eq!((swapped.x, swapped.y), (22.0, 42.0));
    }

    #[test]
    fn compose_affine_with_identity_is_a_no_op() {
        assert_eq!(compose_affine(FORM_M, AFFINE_IDENTITY), FORM_M);
        assert_eq!(compose_affine(AFFINE_IDENTITY, FORM_M), FORM_M);
    }

    #[test]
    fn containment_ratio_measures_area_not_corners() {
        let container = b(0.0, 0.0, 100.0, 100.0);
        assert_eq!(containment_ratio(b(10.0, 10.0, 20.0, 20.0), container), 1.0);
        // 半分だけ外（x∈[90,110]）。
        assert_eq!(containment_ratio(b(90.0, 10.0, 20.0, 20.0), container), 0.5);
        // 完全に外。
        assert_eq!(containment_ratio(b(200.0, 10.0, 20.0, 20.0), container), 0.0);
        // 面積ゼロは「入っている」と言えない（0 除算も避ける）。
        assert_eq!(containment_ratio(b(10.0, 10.0, 0.0, 20.0), container), 0.0);
    }

    // ---- calibrate_form_child_space（8d-8 の自己校正） ----

    /// 実データ（att134 p1）の form bounds。
    const FORM_BOUNDS: BBox = BBox {
        x: 317.014,
        y: 450.929,
        width: 245.034,
        height: 119.383,
    };

    fn candidate(raw: BBox) -> (BBox, BBox) {
        (raw, transform_bbox(raw, FORM_M))
    }

    #[test]
    fn calibration_picks_form_local_on_real_data() {
        // att134 p1 の 6 枚。生 bbox は form の外（包含率 0.0）、行列を当てると 1.0。
        let candidates: Vec<(BBox, BBox)> = [
            b(187.2, 1.1273, 44.9455, 44.9455),
            b(188.509, 79.2364, 44.9455, 44.9455),
            b(188.509, 181.7818, 44.9455, 44.9455),
            b(349.527, 15.3091, 33.6, 18.1091),
            b(381.382, 236.9818, 10.9091, 19.2),
            b(149.891, 10.9455, 36.0, 22.9091),
        ]
        .into_iter()
        .map(candidate)
        .collect();
        assert_eq!(
            calibrate_form_child_space(&candidates, FORM_BOUNDS),
            Some(FormChildSpace::FormLocal)
        );
    }

    #[test]
    fn calibration_picks_page_space_when_the_matrix_reading_leaves_the_form() {
        // 生 bbox がすでに form の中にあり、行列を当てると外へ出る形。
        let form = b(0.0, 0.0, 200.0, 200.0);
        let m = [1.0, 0.0, 0.0, 1.0, 1000.0, 0.0]; // 右へ 1000pt
        let candidates = vec![(b(50.0, 50.0, 60.0, 60.0), transform_bbox(b(50.0, 50.0, 60.0, 60.0), m))];
        assert_eq!(
            calibrate_form_child_space(&candidates, form),
            Some(FormChildSpace::PageSpace)
        );
    }

    #[test]
    fn calibration_drops_the_form_when_neither_reading_fits() {
        // どちらの解釈でも form の外に大きくはみ出す ＝ 座標空間の推定が外れている。
        let form = b(0.0, 0.0, 200.0, 200.0);
        let raw = b(150.0, 150.0, 200.0, 200.0); // 4 分の 1 しか入らない
        let m = [1.0, 0.0, 0.0, 1.0, 500.0, 500.0];
        assert_eq!(
            calibrate_form_child_space(&[(raw, transform_bbox(raw, m))], form),
            None
        );
    }

    #[test]
    fn calibration_is_area_weighted_not_a_majority_vote() {
        // 小さいアイコン 3 枚は生 bbox が form 内、大きな図 1 枚は行列を当てたときだけ form 内。
        // 面積で重み付けするので大きな図が勝ち、多数決だと負ける。
        let form = b(0.0, 0.0, 400.0, 400.0);
        let m = [1.0, 0.0, 0.0, 1.0, -1000.0, 0.0];
        let mut candidates: Vec<(BBox, BBox)> = (0..3)
            .map(|i| {
                let r = b(10.0 + 20.0 * i as f64, 10.0, 8.0, 8.0);
                (r, transform_bbox(r, m))
            })
            .collect();
        let big = b(1050.0, 50.0, 300.0, 300.0);
        candidates.push((big, transform_bbox(big, m)));
        assert_eq!(
            calibrate_form_child_space(&candidates, form),
            Some(FormChildSpace::FormLocal)
        );
    }

    #[test]
    fn calibration_tolerates_a_child_that_slightly_overflows_the_form() {
        // 行列を当てた矩形が form の縁を 5% ほど食み出す（bounds の丸め・線幅）。閾値 0.9 を通る。
        let form = b(0.0, 0.0, 200.0, 200.0);
        let raw = b(-300.0, 0.0, 100.0, 100.0); // そのままでは form の外（包含率 0）
        let m = [1.0, 0.0, 0.0, 1.0, 405.0, 50.0]; // x∈[105,205] → 5% だけ外
        assert_eq!(
            calibrate_form_child_space(&[(raw, transform_bbox(raw, m))], form),
            Some(FormChildSpace::FormLocal)
        );
    }

    #[test]
    fn calibration_accepts_exactly_the_threshold() {
        // 包含率が**ちょうど** FORM_CONTAINMENT_MIN のとき採る（閾値は閉区間）。
        // 9000/10000 は 0.9 リテラルと同じ double になるので比較は厳密。
        let form = b(0.0, 0.0, 100.0, 90.0);
        let raw = b(-1000.0, 0.0, 100.0, 100.0); // そのままでは form の外
        let m = [1.0, 0.0, 0.0, 1.0, 1000.0, 0.0]; // 当てると 9000/10000 = 0.9 だけ内側
        assert_eq!(containment_ratio(transform_bbox(raw, m), form), FORM_CONTAINMENT_MIN);
        assert_eq!(
            calibrate_form_child_space(&[(raw, transform_bbox(raw, m))], form),
            Some(FormChildSpace::FormLocal)
        );
    }

    #[test]
    fn calibration_rejects_degenerate_input() {
        let form = b(0.0, 0.0, 200.0, 200.0);
        assert_eq!(calibrate_form_child_space(&[], form), None);
        // form の bounds が面積ゼロなら何も入らない。
        assert_eq!(
            calibrate_form_child_space(&[candidate(b(10.0, 10.0, 20.0, 20.0))], b(0.0, 0.0, 0.0, 0.0)),
            None
        );
    }

    #[test]
    fn calibration_drops_the_form_when_one_reading_cannot_be_measured() {
        // 入れ子 form の行列が特異（`0 0 0 0 x y cm` は合法）だと合成後の矩形が線・点に潰れ、
        // FormLocal 側の面積が 0 になる。このとき「生の矩形は form に収まる（1.0）」を根拠に
        // `PageSpace` を選ぶと、form ローカル座標の矩形をページ座標として採ってしまう。
        // 測れない方を反証扱いにしないこと＝ここは `None`（誤配置 crop より欠損）。
        let form = b(0.0, 0.0, 200.0, 200.0);
        let raw = b(50.0, 50.0, 60.0, 60.0); // そのままなら form に完全に収まる
        let singular = [0.0, 0.0, 0.0, 0.0, 10.0, 10.0]; // 全部 1 点に潰す
        assert_eq!(
            calibrate_form_child_space(&[(raw, transform_bbox(raw, singular))], form),
            None
        );
        // 逆向き（生 bbox が退化していて PageSpace 側が測れない）も同じく `None`。
        let degenerate = b(50.0, 50.0, 0.0, 0.0);
        assert_eq!(
            calibrate_form_child_space(&[(degenerate, b(10.0, 10.0, 20.0, 20.0))], form),
            None
        );
    }

    // ---- should_scan_forms（8d-8 のゲート・命題「既存ページの出力を動かさない」） ----

    #[test]
    fn forms_are_scanned_only_on_pages_that_keep_their_figure_regions() {
        // 通常のページは辿る。
        assert!(should_scan_forms(0.0, 0));
        assert!(should_scan_forms(0.0, 10));
        // 回転ページは辿らない（辿っても図領域ごと捨てられる・debt-9）。
        assert!(!should_scan_forms(90.0, 10));
        assert!(!should_scan_forms(270.0, 0));
        // 画像過多ページも辿らない。上限**ちょうど**は捨てられない側なので辿る。
        assert!(should_scan_forms(0.0, MAX_RAW_RECTS_PER_PAGE));
        assert!(!should_scan_forms(0.0, MAX_RAW_RECTS_PER_PAGE + 1));
    }

    #[test]
    fn calibration_falls_back_to_form_local_when_both_readings_fit() {
        // form の行列が恒等なら 2 つの解釈は同一。同率は FormLocal（＝この場合は恒等）に倒す。
        let form = b(0.0, 0.0, 200.0, 200.0);
        let raw = b(50.0, 50.0, 60.0, 60.0);
        assert_eq!(
            calibrate_form_child_space(&[(raw, transform_bbox(raw, AFFINE_IDENTITY))], form),
            Some(FormChildSpace::FormLocal)
        );
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

    // ---- Phase 8d-2: 可視性 / クリップ交差 ----

    const BLACK: Rgba = (0, 0, 0, 255);
    const WHITE: Rgba = (255, 255, 255, 255);

    #[test]
    fn white_only_paths_are_not_ink() {
        // 実データ（vid148 p15）: 線幅 113pt の**純白ストローク**が 532×276pt の bbox を持ち、
        // 2 つの図と caption と本文を 1 クラスタに畳んでいた。
        assert!(!path_has_visible_ink(true, WHITE, false, BLACK));
        // 塗りモードが None なら塗り色は見ない（塗らない path も黒い塗り色を持っている）。
        assert!(path_has_visible_ink(true, BLACK, false, WHITE));
        // 白いストローク + 黒い塗りは見える（塗りがインク）。
        assert!(path_has_visible_ink(true, WHITE, true, BLACK));
        // 線も塗りも無い path は手掛かりにならない。
        assert!(!path_has_visible_ink(false, BLACK, false, BLACK));
    }

    #[test]
    fn transparent_and_near_white_are_not_ink() {
        assert!(!path_has_visible_ink(true, (0, 0, 0, 0), false, BLACK));
        assert!(!path_has_visible_ink(false, BLACK, true, (10, 20, 30, 0)));
        // 閾値ちょうど（250）は白側、1 つ下（249）はインク側。
        let at = (WHITE_INK_MIN, WHITE_INK_MIN, WHITE_INK_MIN, 255);
        let below = (WHITE_INK_MIN - 1, WHITE_INK_MIN, WHITE_INK_MIN, 255);
        assert!(!path_has_visible_ink(true, at, false, BLACK));
        assert!(path_has_visible_ink(true, below, false, BLACK));
        // **リテラルでも固定する** ── 上の 2 本は `WHITE_INK_MIN` を使っているので、定数を
        // 255 に上げる変異と一緒に動いてしまう（純白しか白と見なさなくなっても素通りする）。
        assert!(!path_has_visible_ink(true, (252, 253, 251, 255), false, BLACK));
        assert!(path_has_visible_ink(true, (240, 240, 240, 255), false, BLACK));
    }

    #[test]
    fn intersect_rect_returns_none_when_it_would_be_degenerate() {
        let a = b(0.0, 0.0, 100.0, 100.0);
        assert_eq!(intersect_rect(a, b(50.0, 50.0, 100.0, 100.0)), Some(b(50.0, 50.0, 50.0, 50.0)));
        // 辺で接するだけ（面積ゼロ）は交差なし扱い。
        assert_eq!(intersect_rect(a, b(100.0, 0.0, 10.0, 10.0)), None);
        assert_eq!(intersect_rect(a, b(200.0, 0.0, 10.0, 10.0)), None);
        // 実データ（vid238 p6）: クリップを掛けると図の中身の大きさに戻る。
        let huge = b(330.3, -228.8, 96.9, 1845.9);
        let clip = b(330.66, 452.9, 198.4, 124.6);
        let got = intersect_rect(huge, clip).expect("交差する");
        assert!(got.height < 125.0 && got.width < 97.0, "{got:?}");
    }

    // ---- Phase 8d-2: cluster_vector_rects ----

    /// 実データ（vid181 p15・612x792pt）の図: 0.80pt 厚の直線 4 本でできた格子。
    fn v181_grid() -> Vec<BBox> {
        vec![
            b(250.33, 608.02, 120.35, 0.80),
            b(250.33, 647.87, 120.35, 0.80),
            b(290.18, 568.17, 0.80, 120.35),
            b(330.03, 568.17, 0.80, 120.35),
        ]
    }

    #[test]
    fn thin_lines_cluster_into_a_figure_region() {
        let got = cluster_vector_rects(&v181_grid(), 612.0, 792.0);
        assert_eq!(got.len(), 1);
        assert_bbox_near(Some(got[0]), b(250.33, 568.17, 120.35, 120.35));
    }

    #[test]
    fn the_same_thin_lines_are_erased_by_the_raster_merge() {
        // **これが 8d-2 で `merge_image_regions` を流用できない理由**。短辺フィルタが
        // マージ**前**に掛かるので、図を構成する線が 1 本ずつ落ちて図が丸ごと消える。
        assert!(merge_image_regions(&v181_grid(), 612.0, 792.0).is_empty());
    }

    #[test]
    fn a_lone_rule_is_not_a_figure() {
        // 段落罫・脚注罫・ヘッダ罫は単体では短辺が小さいのでクラスタにならない。
        assert!(cluster_vector_rects(&[b(72.0, 700.0, 450.0, 0.6)], 612.0, 792.0).is_empty());
        // 分数線が 2 本近接しても短辺は増えない。
        assert!(cluster_vector_rects(
            &[b(100.0, 500.0, 20.0, 1.0), b(100.0, 504.0, 20.0, 1.0)],
            612.0,
            792.0
        )
        .is_empty());
    }

    #[test]
    fn page_sized_paths_are_dropped_before_and_after_merging() {
        // マージ前: ページ枠・背景（クリップ前の巨大パスもここで落ちる）。
        assert!(cluster_vector_rects(&[b(0.0, 0.0, 610.0, 790.0)], 612.0, 792.0).is_empty());
        // マージ後: 単体では小さい矩形が畳まれて全面になった場合も落とす
        //（`merge_image_regions` にはこの再判定が無い ── 実データに 98.8% の crop が 1 件ある）。
        let mut tiles = Vec::new();
        for i in 0..8 {
            for j in 0..10 {
                tiles.push(b(5.0 + 76.0 * i as f64, 5.0 + 78.0 * j as f64, 74.0, 76.0));
            }
        }
        assert!(cluster_vector_rects(&tiles, 612.0, 792.0).is_empty());
    }

    #[test]
    fn a_page_wide_rule_does_not_bridge_two_figures() {
        // ヘッダ罫（ページ幅いっぱい・厚さ 0.6pt）と、その 8pt 下の図。罫線を残すと
        // 罫線経由で左右のカラムが 1 クラスタに畳まれる（実データ: vid146 p6）。
        let rects = vec![
            b(50.0, 700.0, 512.0, 0.6),   // ヘッダ罫（612pt 幅の 84%）
            b(250.0, 620.0, 120.0, 70.0), // 右の図
            b(60.0, 620.0, 80.0, 70.0),   // 左の欄外（別物）
        ];
        let got = cluster_vector_rects(&rects, 612.0, 792.0);
        assert_eq!(got.len(), 2, "罫線が橋にならず 2 個に分かれる: {got:?}");
        assert!(got.iter().all(|c| c.width < 200.0));
        // 縦の罫線（段間罫）も同じ。**罫線から 12pt 以内に両方の図を置く**こと ──
        // 離れた配置だと罫線を残しても橋にならず、判定を外す変異が素通りする。
        let vertical = vec![
            b(300.0, 60.0, 0.6, 680.0),
            b(184.0, 400.0, 108.0, 70.0), // 罫線まで 8pt（罫線経由なら繋がる）
            b(309.0, 400.0, 108.0, 70.0), // 同上。図どうしは 17pt 離れているので直接は繋がらない
        ];
        assert_eq!(
            cluster_vector_rects(&vertical, 612.0, 792.0).len(),
            2,
            "段間罫が左右のカラムを繋がない"
        );
    }

    #[test]
    fn a_figure_frame_is_not_mistaken_for_a_rule() {
        // 図の枠は 4 辺を持つ 1 個の矩形なので短辺が厚み判定を超える ＝ 罫線扱いにならない。
        let frame = b(50.0, 300.0, 512.0, 200.0);
        assert_eq!(cluster_vector_rects(&[frame], 612.0, 792.0), vec![frame]);
        // 厚みちょうど（3.0pt）は罫線ではない側（`<` 比較）。ただし単体では短辺が足りず落ちる。
        let thick = b(50.0, 300.0, 512.0, VECTOR_RULE_MAX_THICKNESS_PT);
        assert!(!is_page_rule(thick, 612.0, 792.0));
        // 幅が閾値ちょうど（60%）なら罫線側。
        let at_span = b(0.0, 300.0, VECTOR_RULE_SPAN_RATIO * 612.0, 0.6);
        assert!(is_page_rule(at_span, 612.0, 792.0));
        let below_span = b(0.0, 300.0, VECTOR_RULE_SPAN_RATIO * 612.0 - 1.0, 0.6);
        assert!(!is_page_rule(below_span, 612.0, 792.0));
    }

    #[test]
    fn clusters_come_back_in_reading_order() {
        let rects = vec![
            b(100.0, 100.0, 80.0, 80.0),
            b(100.0, 600.0, 80.0, 80.0),
            b(400.0, 600.0, 80.0, 80.0),
        ];
        let got = cluster_vector_rects(&rects, 612.0, 792.0);
        assert_eq!(got.len(), 3);
        assert_eq!((got[0].x, got[0].y), (100.0, 600.0));
        assert_eq!((got[1].x, got[1].y), (400.0, 600.0));
        assert_eq!((got[2].x, got[2].y), (100.0, 100.0));
    }

    // ---- Phase 8d-2: coverage_ratio ----

    #[test]
    fn coverage_ratio_sums_and_saturates() {
        let r = b(0.0, 0.0, 100.0, 100.0);
        assert_eq!(coverage_ratio(r, &[]), 0.0);
        assert_eq!(coverage_ratio(r, &[b(0.0, 0.0, 50.0, 100.0)]), 0.5);
        // 重なり合う相手は二重に数える（過大評価 = 捨てる側 = 安全側）。上限は 1.0。
        assert_eq!(
            coverage_ratio(r, &[b(0.0, 0.0, 100.0, 100.0), b(0.0, 0.0, 100.0, 100.0)]),
            1.0
        );
        // 面積ゼロの領域は「全部覆われている」＝捨てる側に倒す（0 除算も避ける）。
        assert_eq!(coverage_ratio(b(10.0, 10.0, 0.0, 50.0), &[]), 1.0);
    }

    // ---- Phase 8d-2: 2 段ペアリング ----

    #[test]
    fn unpaired_caption_indices_lists_what_the_raster_pass_left() {
        let fig = vec![b(100.0, 400.0, 300.0, 200.0)];
        let caps = vec![b(100.0, 360.0, 300.0, 24.0), b(100.0, 100.0, 300.0, 24.0)];
        assert_eq!(unpaired_caption_indices(&fig, &caps), vec![1]);
        assert_eq!(unpaired_caption_indices(&[], &caps), vec![0, 1]);
        assert!(unpaired_caption_indices(&fig, &[]).is_empty());
    }

    #[test]
    fn two_stage_pairing_matches_one_stage_when_there_are_no_vectors() {
        let fig = vec![b(100.0, 400.0, 300.0, 200.0), b(100.0, 100.0, 300.0, 120.0)];
        let caps = vec![b(100.0, 360.0, 300.0, 24.0), b(100.0, 60.0, 300.0, 24.0)];
        let mut one = pair_captions(&fig, &caps);
        let mut two = pair_captions_two_stage(&fig, &[], &caps);
        one.sort();
        two.sort();
        assert_eq!(one, two);
    }

    #[test]
    fn a_vector_region_cannot_steal_a_caption_from_a_raster_figure() {
        // ラスタ図の**すぐ下**（4pt）に caption。ベクター領域はその caption の
        // さらに近く（1pt 下）に置く ── 1 段でまとめて相互最近を取ると caption を奪う。
        let raster = vec![b(100.0, 400.0, 300.0, 200.0)];
        let caption = vec![b(100.0, 372.0, 300.0, 24.0)];
        let vector = vec![b(100.0, 340.0, 300.0, 30.0)];
        // 1 段だと奪われることを先に固定する（この前提が崩れたらテストの意味が消える）。
        let mut all = raster.clone();
        all.extend(vector.iter().copied());
        assert_eq!(pair_captions(&all, &caption), vec![(1, 0)], "1 段だと奪われる");
        // 2 段ならラスタ側のペアが残り、ベクターは余りが無いので何も結ばれない。
        assert_eq!(pair_captions_two_stage(&raster, &vector, &caption), vec![(0, 0)]);
    }

    #[test]
    fn two_stage_pairing_indexes_into_the_concatenated_list() {
        let raster = vec![b(100.0, 600.0, 300.0, 120.0)];
        let vector = vec![b(100.0, 300.0, 300.0, 120.0)];
        let caps = vec![b(100.0, 560.0, 300.0, 24.0), b(100.0, 260.0, 300.0, 24.0)];
        let mut got = pair_captions_two_stage(&raster, &vector, &caps);
        got.sort();
        // ベクターは `raster.len() + 0` = 1 番。
        assert_eq!(got, vec![(0, 0), (1, 1)]);
    }

    // ---- Phase 8d-2: compose_figure_regions ----

    /// ラスタ図 1 個 + その caption + 遠くのベクター図 + その caption、というページ。
    fn two_figure_page() -> (Vec<BBox>, Vec<BBox>, Vec<BBox>) {
        let raster_rects = vec![b(100.0, 600.0, 300.0, 120.0)];
        let vector_rects = vec![
            b(100.0, 300.0, 300.0, 1.0),
            b(100.0, 419.0, 300.0, 1.0),
            b(100.0, 300.0, 1.0, 120.0),
        ];
        let caps = vec![b(100.0, 560.0, 300.0, 24.0), b(100.0, 260.0, 300.0, 24.0)];
        (raster_rects, vector_rects, caps)
    }

    #[test]
    fn raster_only_input_is_byte_identical_to_the_shipping_merge() {
        let (raster, _, caps) = two_figure_page();
        let merged = merge_image_regions(&raster, 612.0, 792.0);
        let got = compose_figure_regions(&merged, &[], &[], &caps, 612.0, 792.0);
        assert_eq!(
            got.iter().map(|r| r.bbox).collect::<Vec<_>>(),
            merged,
            "ラスタだけの入力では 8a の領域列そのもの"
        );
        assert!(got.iter().all(|r| r.source == RegionSource::Raster));
    }

    #[test]
    fn vector_clusters_overlapping_a_raster_region_are_dropped_not_unioned() {
        // **§2.6-2 のゲート本体**。ベクター入力は空ではなく、**余った caption も在る**のに、
        // 既存ラスタ図に重なるので 1 個も採られない（union は禁止）。
        //
        // caption を 2 つ置くのが要点 ── 1 つだけだとラスタと結ばれて「余りなし」の早期 return に
        // 落ち、重なり判定を外す変異が素通りする（ゲートが空になる）。
        let merged = merge_image_regions(&[b(100.0, 600.0, 300.0, 120.0)], 612.0, 792.0);
        let caps = vec![
            b(100.0, 570.0, 300.0, 24.0), // ラスタ図に近い → ラスタと結ばれる
            b(100.0, 540.0, 300.0, 24.0), // 余る
        ];
        assert_eq!(unpaired_caption_indices(&merged, &caps), vec![1], "余りが 1 つ在る");
        // 図の枠線（ラスタ図にぴったり重なる）。余った caption とは十分近い。
        let axes = vec![
            b(98.0, 598.0, 304.0, 1.0),
            b(98.0, 721.0, 304.0, 1.0),
            b(98.0, 598.0, 1.0, 124.0),
        ];
        assert_eq!(
            cluster_vector_rects(&axes, 612.0, 792.0).len(),
            1,
            "クラスタ自体は成立している（落ちているのは重なり判定）"
        );
        let got = compose_figure_regions(&merged, &axes, &[], &caps, 612.0, 792.0);
        assert_eq!(got.iter().map(|r| r.bbox).collect::<Vec<_>>(), merged);
    }

    #[test]
    fn a_cluster_that_pairs_with_nothing_is_not_kept() {
        // 余った caption は在るが、クラスタはそこから遠い ＝ ペアが成立しない。
        // **caption を持たないベクター図は取らない**という受理規則の本体。
        let caps = vec![b(100.0, 100.0, 300.0, 24.0)];
        let far = vec![
            b(100.0, 600.0, 300.0, 1.0),
            b(100.0, 719.0, 300.0, 1.0),
            b(100.0, 600.0, 1.0, 120.0),
        ];
        assert_eq!(cluster_vector_rects(&far, 612.0, 792.0).len(), 1);
        assert!(compose_figure_regions(&[], &far, &[], &caps, 612.0, 792.0).is_empty());
    }

    #[test]
    fn a_distant_vector_cluster_is_appended_without_touching_the_raster_list() {
        let (raster, vector, caps) = two_figure_page();
        let merged = merge_image_regions(&raster, 612.0, 792.0);
        let got = compose_figure_regions(&merged, &vector, &[], &caps, 612.0, 792.0);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].bbox, merged[0]);
        assert_eq!(got[0].source, RegionSource::Raster);
        assert_eq!(got[1].source, RegionSource::Vector);
        assert_bbox_near(Some(got[1].bbox), b(100.0, 300.0, 300.0, 120.0));
    }

    #[test]
    fn vector_clusters_without_a_leftover_caption_are_not_kept() {
        // caption が無いページでは 1 個も作らない（caption を持たないベクター図は取り逃す）。
        let (_, vector, _) = two_figure_page();
        assert!(compose_figure_regions(&[], &vector, &[], &[], 612.0, 792.0).is_empty());
        // caption がラスタ図と結ばれ済みなら、余りが無いので走らせない。
        let merged = merge_image_regions(&[b(100.0, 600.0, 300.0, 120.0)], 612.0, 792.0);
        let caps = vec![b(100.0, 560.0, 300.0, 24.0)];
        let got = compose_figure_regions(&merged, &vector, &[], &caps, 612.0, 792.0);
        assert_eq!(got.len(), 1, "余った caption が無いのでベクターは走らない");
    }

    #[test]
    fn a_cluster_that_covers_body_text_is_dropped() {
        // 本文段を丸ごと覆うクラスタは「本文を図と誤認した」として捨てる。
        let (_, vector, caps) = two_figure_page();
        let prose = vec![b(100.0, 300.0, 300.0, 120.0)];
        assert!(compose_figure_regions(&[], &vector, &prose, &caps, 612.0, 792.0).is_empty());
        // 端に少し掛かるだけなら残す（閾値の下側）。
        let edge = vec![b(100.0, 300.0, 300.0, 30.0)];
        assert_eq!(
            compose_figure_regions(&[], &vector, &edge, &caps, 612.0, 792.0).len(),
            1
        );
        // **閾値と 1.0 の間にも点を置く**（半分が本文）。ここが無いと、閾値を 0.95 に
        // 緩める変異が「全面が本文」のケースだけで素通りする。
        let half = vec![b(100.0, 300.0, 300.0, 60.0)];
        let cover = coverage_ratio(cluster_vector_rects(&vector, 612.0, 792.0)[0], &half);
        assert!(
            cover > VECTOR_MAX_PROSE_COVER && cover < 0.95,
            "閾値と 1.0 の間: {cover}"
        );
        assert!(compose_figure_regions(&[], &vector, &half, &caps, 612.0, 792.0).is_empty());
    }

    #[test]
    fn the_page_region_cap_is_spent_on_raster_first() {
        // ラスタが上限を埋めているページではベクターを 1 個も足さない
        //（＝ 面積上位カットで既存のラスタ図が押し出される経路を構造的に塞ぐ）。
        //
        // ベクター側は**別カラム**に置く ── 同じカラムに置くと caption がラスタと結ばれて
        // 「余りなし」の早期 return に落ち、枠の計算を壊す変異が素通りする。
        let mut rects = Vec::new();
        for i in 0..MAX_REGIONS_PER_PAGE {
            rects.push(b(50.0, 40.0 + 90.0 * i as f64, 150.0, 60.0));
        }
        let merged = merge_image_regions(&rects, 612.0, 792.0);
        assert_eq!(merged.len(), MAX_REGIONS_PER_PAGE);
        let caps = vec![b(350.0, 300.0, 250.0, 24.0)];
        assert_eq!(unpaired_caption_indices(&merged, &caps), vec![0], "余りが在る");
        let vector = vec![
            b(350.0, 340.0, 250.0, 1.0),
            b(350.0, 459.0, 250.0, 1.0),
            b(350.0, 340.0, 1.0, 120.0),
        ];
        // 枠が空いていれば採られる形であることを先に固定する。
        let with_room = compose_figure_regions(&merged[..1], &vector, &[], &caps, 612.0, 792.0);
        assert_eq!(with_room.len(), 2, "残り枠があるときは採る");
        let got = compose_figure_regions(&merged, &vector, &[], &caps, 612.0, 792.0);
        assert_eq!(got.iter().map(|r| r.bbox).collect::<Vec<_>>(), merged);
    }

    #[test]
    fn vector_paths_are_scanned_only_on_unrotated_pages_within_the_path_budget() {
        assert!(should_scan_vector_paths(0.0, 0));
        assert!(should_scan_vector_paths(0.0, MAX_RAW_PATHS_PER_PAGE));
        assert!(!should_scan_vector_paths(0.0, MAX_RAW_PATHS_PER_PAGE + 1));
        assert!(!should_scan_vector_paths(90.0, 10));
        // ラスタ側の門とは別の定数を使う（混ぜると 8d-8 の「出力不変」の保証が壊れる）。
        const { assert!(MAX_RAW_PATHS_PER_PAGE > MAX_RAW_RECTS_PER_PAGE) };
    }

    #[test]
    fn bbox_hull_ignores_nothing_and_rejects_degenerate_input() {
        assert_eq!(bbox_hull(&[]), None);
        assert_eq!(
            bbox_hull(&[b(10.0, 10.0, 20.0, 20.0), b(100.0, 5.0, 10.0, 10.0)]),
            Some(b(10.0, 5.0, 100.0, 25.0))
        );
        // 面積ゼロの hull は領域にしない。
        assert_eq!(bbox_hull(&[b(10.0, 10.0, 0.0, 0.0)]), None);
    }

    #[test]
    fn the_visible_hull_is_smaller_than_a_form_bounds_padded_by_white() {
        // 実データ（vid275 p35）の形: form の bounds を**純白の塗り矩形**が単独で決めており、
        // 見えるインクはその内側の一部だけ。form bounds を採ると面積の 69% が図でなくなる。
        let form_bounds = b(39.883, 510.407, 460.800, 331.483);
        let visible = b(111.8, 627.7, 368.1, 129.2);
        // hull は `(x+w) - x` で幅を組み直すので最下位ビットがずれる（他の幾何と同じ）。
        assert_bbox_near(bbox_hull(&[visible]), visible);
        let hull = bbox_hull(&[visible]).expect("hull");
        let form_area = form_bounds.width * form_bounds.height;
        assert!(hull.width * hull.height < 0.35 * form_area, "{hull:?}");
    }

    #[test]
    fn a_form_that_also_holds_an_image_is_not_a_vector_candidate() {
        // Image を持つ form を採ると、8d-8 が既に拾ったのと**同じ図に 2 枚の crop** が出て
        // Vision の課金も 2 回になる。
        assert!(form_is_vector_only(0, 3));
        assert!(!form_is_vector_only(1, 3));
        assert!(!form_is_vector_only(0, 0), "見える path が無い form は手掛かりにならない");
        assert!(!form_is_vector_only(usize::MAX, 3), "子を数え切れなかった form は捨てる");
    }

    #[test]
    fn only_captions_left_over_by_the_raster_pass_can_anchor_a_vector_region() {
        // ラスタ図が既に取っている caption を相手にすると、build 側は 2 段でペアリングするので
        // その caption はラスタ図に行き、**caption を持たない余分な figure + crop + 課金**が生える。
        let merged = merge_image_regions(&[b(100.0, 600.0, 300.0, 120.0)], 612.0, 792.0);
        let caps = vec![b(100.0, 560.0, 300.0, 24.0)];
        assert!(unpaired_caption_indices(&merged, &caps).is_empty(), "余りは無い");
        // その caption のすぐ下（ラスタ図とは重ならない位置）にベクタークラスタを置く。
        let vector = vec![
            b(100.0, 420.0, 300.0, 1.0),
            b(100.0, 539.0, 300.0, 1.0),
            b(100.0, 420.0, 1.0, 120.0),
        ];
        assert_eq!(cluster_vector_rects(&vector, 612.0, 792.0).len(), 1, "クラスタは成立する");
        assert_eq!(
            compose_figure_regions(&merged, &vector, &[], &caps, 612.0, 792.0).len(),
            1,
            "余った caption が無いので採らない"
        );
    }

    #[test]
    fn a_small_overlap_with_a_raster_region_is_tolerated() {
        // 重なり判定の**下側**も固定する（上側だけだと閾値を 0.01 にする変異が素通りする）。
        let merged = merge_image_regions(&[b(100.0, 600.0, 300.0, 120.0)], 612.0, 792.0);
        let caps = vec![
            b(100.0, 570.0, 300.0, 24.0),
            b(100.0, 400.0, 300.0, 24.0),
        ];
        // 図はラスタ図の下。上辺だけが 6pt 重なる（面積比 ≒ 0.05）。
        let vector = vec![
            b(100.0, 440.0, 300.0, 1.0),
            b(100.0, 605.0, 300.0, 1.0),
            b(100.0, 440.0, 1.0, 166.0),
        ];
        let cluster = cluster_vector_rects(&vector, 612.0, 792.0);
        assert_eq!(cluster.len(), 1);
        let overlap = coverage_ratio(cluster[0], &merged);
        assert!(
            overlap > 0.0 && overlap < VECTOR_RASTER_OVERLAP_MAX,
            "少しだけ重なる配置になっている: {overlap}"
        );
        assert_eq!(
            compose_figure_regions(&merged, &vector, &[], &caps, 612.0, 792.0).len(),
            2,
            "少しの重なりでは捨てない"
        );
    }

    #[test]
    fn prose_cover_just_under_the_threshold_is_kept() {
        // 本文被覆の**下側**も固定する（0.95 にする変異を捕まえる）。
        let (_, vector, caps) = two_figure_page();
        let cluster = cluster_vector_rects(&vector, 612.0, 792.0);
        assert_eq!(cluster.len(), 1);
        // クラスタ面積の 30%（閾値 0.35 のすぐ下）を本文が覆う配置。
        let a = cluster[0];
        let prose = vec![b(a.x, a.y, a.width, a.height * 0.30)];
        let cover = coverage_ratio(a, &prose);
        assert!(
            cover < VECTOR_MAX_PROSE_COVER && cover > 0.25,
            "閾値のすぐ下: {cover}"
        );
        assert_eq!(
            compose_figure_regions(&[], &vector, &prose, &caps, 612.0, 792.0).len(),
            1
        );
    }

    #[test]
    fn a_full_page_path_does_not_swallow_the_real_figure_on_the_same_page() {
        // マージ**前**の面積フィルタが無いと、全面の背景 path（クリップで小さく見せている
        // 巨大パスを含む）と図が 1 クラスタに畳まれ、マージ**後**の 0.9 判定でまとめて捨てられて
        // **図が失われる**。全面矩形だけのページでは両者の区別が付かないので、図と同居させる。
        let mut rects = v181_grid();
        rects.push(b(1.0, 1.0, 610.0, 790.0)); // ページ面積の 99.5%
        let got = cluster_vector_rects(&rects, 612.0, 792.0);
        assert_eq!(got.len(), 1, "図だけが残る: {got:?}");
        assert_bbox_near(Some(got[0]), b(250.33, 568.17, 120.35, 120.35));
    }

    #[test]
    fn region_confidence_is_lower_for_vector_than_raster() {
        assert_eq!(RegionSource::Raster.confidence(), 0.6);
        assert!(RegionSource::Vector.confidence() < RegionSource::Raster.confidence());
    }

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
