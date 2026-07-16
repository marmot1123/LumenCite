//! 論理構造の認識（Phase 2）。pdfium が返す細粒度のテキストセグメント列を、
//! **行 → ブロック（段落・見出し・caption 等）** にまとめ、ヒューリスティックで型付けする。
//!
//! この module は pdfium にも sqlx にも依存しない**純関数**で、合成入力で CI テストできる
//! （native lib 不要）。入力は Phase 1 の `ExtractedPage`（セグメント + PDF 座標）。
//!
//! 設計思想（`docs/LCIR_design_overview.md`）:
//! - **認識に確信が持てないブロックは、誤った型を確定せず `unknown_block` + 低信頼度で残す。**
//! - 各ブロックに `confidence`（0–1）を付け、`origin` は build 側で `layout_model`（推定）にする。
//! - 完全な論理構造復元は非目標。確実な範囲（番号付き節・caption・abstract・参考文献・段落）を
//!   高信頼度で出し、残り（footnote/list/citation/code_block 等）は後続で漸進的に改善する。

use crate::document_ir::{BBox, NodeKind};
use crate::ingestion::pdf::ExtractedPage;

/// 同一行判定: 2 セグメントの縦区間がこの割合以上重なれば同じ行とみなす。
const LINE_VOVERLAP_RATIO: f64 = 0.4;
/// 行内でセグメント間に半角空白を挿入する水平ギャップの閾値（行高に対する割合）。
const SPACE_GAP_RATIO: f64 = 0.2;
/// 段落分割: 行間ギャップが「中央値 × この倍率」を超えたら新しいブロックにする。
const PARA_GAP_RATIO: f64 = 1.6;
/// 見出し判定: ブロックの字高が「ページ本文中央値 × この倍率」を超えたら見出し候補。
const HEADING_HEIGHT_RATIO: f64 = 1.15;

/// 認識した論理ブロック（段落・見出し・caption 等）。build 側が `document_nodes` に落とす。
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredBlock {
    pub kind: NodeKind,
    /// ブロック全体のテキスト（行を連結・空白正規化済み）。node-FTS の索引元。
    pub text: String,
    /// ブロックの統合バウンディング（PDF user space・左下原点・pt）。
    pub bbox: BBox,
    /// 型付けの信頼度（0–1）。原文由来ではなく layout 推定なので必ず持たせる。
    pub confidence: f64,
    /// 見出しの階層（section=1 / subsection=2 …）。見出し以外は None。
    pub heading_level: Option<i64>,
    /// 節番号（"3.2" 等）。番号付き見出しのみ。
    pub section_number: Option<String>,
    /// 構成する行（読み順）。各行は node_kind=line の子ノードになる。
    pub lines: Vec<StructuredLine>,
}

/// ブロックを構成する 1 行（セグメントをベースラインでまとめたもの）。
#[derive(Debug, Clone, PartialEq)]
pub struct StructuredLine {
    pub text: String,
    pub bbox: BBox,
    /// 先頭セグメントの読み順（安定ソート・provenance 用）。
    pub reading_order: i64,
}

/// 文書横断で保持する認識状態。ページをまたいで abstract/参考文献モードを継続する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecognizerState {
    mode: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Body,
    Abstract,
    Bibliography,
}

impl Default for RecognizerState {
    fn default() -> Self {
        Self { mode: Mode::Body }
    }
}

impl RecognizerState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// 1 ページのセグメント列を論理ブロックに構造化する。`state` は文書横断で使い回す。
pub fn recognize_page(page: &ExtractedPage, state: &mut RecognizerState) -> Vec<StructuredBlock> {
    let lines = group_lines(page);
    if lines.is_empty() {
        return Vec::new();
    }
    let line_groups = group_blocks(lines);

    // ページ本文の代表字高（見出しを相対的に見分ける基準）。全行の高さの中央値。
    let mut heights: Vec<f64> = line_groups
        .iter()
        .flat_map(|g| g.iter().map(|l| l.bbox.height))
        .collect();
    let page_median_h = median(&mut heights);

    let mut out = Vec::with_capacity(line_groups.len());
    for lines in line_groups {
        if let Some(block) = classify_block(lines, page_median_h, page.height_pt, state) {
            out.push(block);
        }
    }
    out
}

// ---- 行のグルーピング（セグメント → 行） ----

fn group_lines(page: &ExtractedPage) -> Vec<StructuredLine> {
    let mut lines: Vec<StructuredLine> = Vec::new();
    // 現在の行に積んでいるセグメント（bbox, text, reading_order）。
    let mut cur: Vec<&crate::ingestion::pdf::ExtractedBlock> = Vec::new();

    for seg in &page.blocks {
        if seg.text.trim().is_empty() {
            continue;
        }
        match cur.last() {
            Some(_) if same_line(&cur, &seg.bbox) => cur.push(seg),
            Some(_) => {
                lines.push(flush_line(&cur));
                cur.clear();
                cur.push(seg);
            }
            None => cur.push(seg),
        }
    }
    if !cur.is_empty() {
        lines.push(flush_line(&cur));
    }
    lines
}

/// 次のセグメントが現在の行と同じベースラインか（縦区間の重なり割合で判定）。
fn same_line(cur: &[&crate::ingestion::pdf::ExtractedBlock], next: &BBox) -> bool {
    // 現在行の縦区間 = メンバ全体の union。
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for s in cur {
        lo = lo.min(s.bbox.y);
        hi = hi.max(s.bbox.y + s.bbox.height);
    }
    let (nlo, nhi) = (next.y, next.y + next.height);
    let overlap = hi.min(nhi) - lo.max(nlo);
    if overlap <= 0.0 {
        return false;
    }
    let min_h = (hi - lo).min(nhi - nlo);
    min_h > 0.0 && overlap >= LINE_VOVERLAP_RATIO * min_h
}

/// 行内セグメントを 1 行（テキスト連結 + union bbox）にまとめる。水平ギャップに空白を補う。
fn flush_line(segs: &[&crate::ingestion::pdf::ExtractedBlock]) -> StructuredLine {
    let reading_order = segs.iter().map(|s| s.reading_order).min().unwrap_or(0);
    let mut bbox = segs[0].bbox;
    let mut text = String::new();
    for (i, s) in segs.iter().enumerate() {
        if i > 0 {
            let prev = segs[i - 1];
            let gap = s.bbox.x - (prev.bbox.x + prev.bbox.width);
            let h = prev.bbox.height.max(s.bbox.height);
            let boundary_ws = text.ends_with(char::is_whitespace)
                || s.text.starts_with(char::is_whitespace);
            if !boundary_ws && gap > SPACE_GAP_RATIO * h {
                text.push(' ');
            }
            bbox = union_bbox(bbox, s.bbox);
        }
        text.push_str(&s.text);
    }
    StructuredLine {
        text: normalize_ws(&text),
        bbox,
        reading_order,
    }
}

// ---- ブロックのグルーピング（行 → 段落/見出し） ----

/// 行を縦ギャップでブロックに分割する。段落間の空きや段組み境界で切る。
fn group_blocks(lines: Vec<StructuredLine>) -> Vec<Vec<StructuredLine>> {
    if lines.len() <= 1 {
        return if lines.is_empty() {
            Vec::new()
        } else {
            vec![lines]
        };
    }

    // 連続行の縦ギャップ（正の値のみ）の中央値を「行送り」の基準にする。
    let mut gaps: Vec<f64> = Vec::new();
    for w in lines.windows(2) {
        let g = line_gap(&w[0], &w[1]);
        if g > 0.0 {
            gaps.push(g);
        }
    }
    let median_gap = median(&mut gaps);

    let mut blocks: Vec<Vec<StructuredLine>> = Vec::new();
    let mut cur: Vec<StructuredLine> = Vec::new();
    for line in lines {
        if let Some(prev) = cur.last() {
            let g = line_gap(prev, &line);
            // 段落間の空き / 段組み・領域境界（負ギャップ）で新ブロック。
            let split = g < 0.0 || (median_gap > 0.0 && g > PARA_GAP_RATIO * median_gap);
            if split {
                blocks.push(std::mem::take(&mut cur));
            }
        }
        cur.push(line);
    }
    if !cur.is_empty() {
        blocks.push(cur);
    }
    blocks
}

/// 読み順で上下する 2 行の縦ギャップ。上の行 a の下端と下の行 b の上端の差。
/// 段組み境界で b がページ上部へ飛ぶと負になる。
fn line_gap(a: &StructuredLine, b: &StructuredLine) -> f64 {
    a.bbox.y - (b.bbox.y + b.bbox.height)
}

// ---- 分類 ----

fn classify_block(
    lines: Vec<StructuredLine>,
    page_median_h: f64,
    page_height: f64,
    state: &mut RecognizerState,
) -> Option<StructuredBlock> {
    let text = normalize_ws(
        &lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    );
    if text.is_empty() {
        return None;
    }
    let first = lines.first().map(|l| l.text.as_str()).unwrap_or("");
    let mut block_heights: Vec<f64> = lines.iter().map(|l| l.bbox.height).collect();
    let block_median_h = median(&mut block_heights);
    let bbox = lines
        .iter()
        .map(|l| l.bbox)
        .reduce(union_bbox)
        .unwrap_or(BBox::new(0.0, 0.0, 0.0, 0.0));
    let word_count = text.split_whitespace().count();

    let mk = |kind, confidence, heading_level, section_number, lines: Vec<StructuredLine>| {
        Some(StructuredBlock {
            kind,
            text: text.clone(),
            bbox,
            confidence,
            heading_level,
            section_number,
            lines,
        })
    };

    // 1. 見出し（参考文献モードでは番号付き見出しを無効化 = "1. Author…" の誤検出回避）。
    if let Some(h) = detect_heading(
        first,
        lines.len(),
        word_count,
        block_median_h,
        page_median_h,
        matches!(state.mode, Mode::Bibliography),
    ) {
        state.mode = match h.keyword {
            Some("abstract") => Mode::Abstract,
            Some("references") | Some("bibliography") => Mode::Bibliography,
            _ => Mode::Body,
        };
        return mk(h.kind, h.confidence, h.level, h.section_number, lines);
    }

    // 2. caption（参考文献モードでは "Figure" は稀なのでスキップ）。
    if !matches!(state.mode, Mode::Bibliography) {
        if let Some(cap_kind) = detect_caption(first) {
            return mk(cap_kind, 0.75, None, None, lines);
        }
    }

    // 3. モードに応じた本文分類。
    match state.mode {
        Mode::Abstract => mk(NodeKind::Abstract, 0.7, None, None, lines),
        Mode::Bibliography => mk(NodeKind::BibliographyEntry, 0.5, None, None, lines),
        Mode::Body => {
            // ページ上下の極端なマージンにある短い 1 行は、ランニングヘッダ/フッタ/ページ番号の
            // 可能性が高い。段落と確定せず unknown_block に降格する（誤った型より欠損を許容）。
            let in_margin = page_height > 1.0
                && lines.len() == 1
                && word_count <= 8
                && (bbox.y > page_height * 0.90 || bbox.y + bbox.height < page_height * 0.10);
            if looks_like_prose(&text) && !in_margin {
                mk(NodeKind::Paragraph, 0.6, None, None, lines)
            } else {
                // ページ番号・欄外見出し・孤立記号など、文でも既知構造でもない断片。
                mk(NodeKind::UnknownBlock, 0.3, None, None, lines)
            }
        }
    }
}

struct HeadingHit {
    kind: NodeKind,
    level: Option<i64>,
    section_number: Option<String>,
    keyword: Option<&'static str>,
    confidence: f64,
}

fn detect_heading(
    first: &str,
    line_count: usize,
    word_count: usize,
    block_median_h: f64,
    page_median_h: f64,
    in_bibliography: bool,
) -> Option<HeadingHit> {
    // 見出しは短い（1–2 行）。
    if line_count > 2 {
        return None;
    }

    // 番号付き節（"3 Method" / "3.2 Details"）。参考文献モードでは無効。
    if !in_bibliography && word_count <= 14 {
        if let Some((number, level)) = parse_section_number(first) {
            // 単一レベルで 100 以上の番号はページ番号/年（"104 A. Suzuki" / "2020 …"）の可能性が
            // 高く、節番号としてはまず現れない。誤って section にせず素通りさせる。
            let looks_like_page_number =
                level == 1 && number.parse::<u32>().is_ok_and(|n| n >= 100);
            if !looks_like_page_number {
                let kind = if level >= 2 {
                    NodeKind::Subsection
                } else {
                    NodeKind::Section
                };
                return Some(HeadingHit {
                    kind,
                    level: Some(level),
                    section_number: Some(number),
                    keyword: None,
                    confidence: 0.75,
                });
            }
        }
    }

    // 既知キーワード見出し（"Abstract" / "Introduction" / "References" …）。
    if let Some(kw) = heading_keyword(first) {
        if word_count <= 6 {
            return Some(HeadingHit {
                kind: NodeKind::Heading,
                level: Some(1),
                section_number: None,
                keyword: Some(kw),
                confidence: 0.7,
            });
        }
    }

    // 字の大きさ（番号もキーワードも無いが本文より大きい短い 1 行）。参考文献モードでは無効。
    // 文字が主体の行に限る（純数字のページ番号 "123" や、記号主体の display 数式 "U−tU…" を
    // 大フォントで見出しにしない。数式の本格認識は Phase 3）。
    if !in_bibliography
        && line_count == 1
        && word_count <= 8
        && looks_like_prose(first)
        && alpha_ratio(first) >= 0.6
        && page_median_h > 0.0
        && block_median_h > page_median_h * HEADING_HEIGHT_RATIO
    {
        return Some(HeadingHit {
            kind: NodeKind::Heading,
            level: None,
            section_number: None,
            keyword: None,
            confidence: 0.55,
        });
    }

    None
}

/// 行頭の "N" / "N.M" / "N.M.K" 節番号を取り出す。`(番号, 階層)`。見出しでなければ None。
fn parse_section_number(s: &str) -> Option<(String, i64)> {
    let s = s.trim_start();
    let prefix: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if !prefix.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    let rest = &s[prefix.len()..];
    let rest_trim = rest.trim_start();
    // 番号とタイトルの間に空白が必要（"3.14pi" のような値を弾く）。
    if rest == rest_trim {
        return None;
    }
    // タイトルが英字で始まること（"3. 2020" のような数字続きを弾く）。
    match rest_trim.chars().next() {
        Some(c) if c.is_alphabetic() => {}
        _ => return None,
    }
    let number = prefix.trim_end_matches('.').to_string();
    if number.is_empty() {
        return None;
    }
    let level = number.split('.').filter(|p| !p.is_empty()).count() as i64;
    Some((number, level))
}

/// 既知の節見出しキーワード（小文字・末尾の ':' '.' を除いた完全一致）。
const HEADING_KEYWORDS: &[&str] = &[
    "abstract",
    "introduction",
    "related work",
    "background",
    "motivation",
    "preliminaries",
    "notation",
    "method",
    "methods",
    "methodology",
    "approach",
    "materials and methods",
    "experiments",
    "experimental setup",
    "experimental results",
    "results",
    "results and discussion",
    "evaluation",
    "analysis",
    "discussion",
    "conclusion",
    "conclusions",
    "concluding remarks",
    "future work",
    "limitations",
    "acknowledgment",
    "acknowledgments",
    "acknowledgement",
    "acknowledgements",
    "references",
    "bibliography",
    "appendix",
    "appendices",
    "supplementary material",
];

fn heading_keyword(first: &str) -> Option<&'static str> {
    let norm = first.trim().trim_end_matches([':', '.']).trim();
    let lower = norm.to_ascii_lowercase();
    HEADING_KEYWORDS.iter().copied().find(|&k| lower == k)
}

/// 行頭が "Figure 1" / "Table 2:" / "Fig. 3" のような caption ラベルか。
fn detect_caption(first: &str) -> Option<NodeKind> {
    let f = first.trim_start();
    let lower = f.to_ascii_lowercase();
    let (label_len, kind) = if lower.starts_with("figure") {
        (6, NodeKind::FigureCaption)
    } else if lower.starts_with("fig.") {
        (4, NodeKind::FigureCaption)
    } else if lower.starts_with("fig ") {
        (3, NodeKind::FigureCaption)
    } else if lower.starts_with("table") {
        (5, NodeKind::TableCaption)
    } else if lower.starts_with("algorithm") {
        (9, NodeKind::FigureCaption)
    } else if lower.starts_with("listing") {
        (7, NodeKind::FigureCaption)
    } else {
        return None;
    };
    // ラベル直後の数文字以内に番号（数字）があること（"Figures show…" の誤検出回避）。
    let after: String = f[label_len..].chars().take(6).collect();
    if after.chars().any(|c| c.is_ascii_digit()) {
        Some(kind)
    } else {
        None
    }
}

/// 文らしさ（英字が数個以上）。ページ番号 "12" や孤立記号を段落から除くための粗い判定。
fn looks_like_prose(t: &str) -> bool {
    t.chars().filter(|c| c.is_alphabetic()).count() >= 3
}

/// 非空白文字に占める英字の割合（0–1）。数式・記号列（低い）と散文（高い）を粗く分ける。
fn alpha_ratio(t: &str) -> f64 {
    let non_ws = t.chars().filter(|c| !c.is_whitespace()).count();
    if non_ws == 0 {
        return 0.0;
    }
    let alpha = t.chars().filter(|c| c.is_alphabetic()).count();
    alpha as f64 / non_ws as f64
}

// ---- 小物ユーティリティ ----

fn union_bbox(a: BBox, b: BBox) -> BBox {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let top = (a.y + a.height).max(b.y + b.height);
    BBox::new(x, y, right - x, top - y)
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 中央値（空なら 0.0）。呼び出し側の Vec を破壊的にソートする。
fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingestion::pdf::{ExtractedBlock, ExtractedPage};

    /// 段落内の行間ギャップと、ブロック区切りのギャップ（テスト用の代表値）。
    const G: f64 = 4.0; // intra-paragraph
    const H: f64 = 40.0; // block break

    fn seg(text: &str, x: f64, y: f64, w: f64, h: f64, ro: i64) -> ExtractedBlock {
        ExtractedBlock {
            text: text.to_string(),
            bbox: BBox::new(x, y, w, h),
            reading_order: ro,
        }
    }

    fn page(segs: Vec<ExtractedBlock>) -> ExtractedPage {
        ExtractedPage {
            page_number: 1,
            width_pt: 595.0,
            height_pt: 842.0,
            rotation_deg: 0.0,
            plain_text: String::new(),
            blocks: segs,
        }
    }

    /// 1 セグメント = 1 行としてページを縦積みする。`items` は (text, height, gap_before)。
    /// 先頭の gap は無視。`line_gap` がちょうど gap_before になるよう座標を置く。
    fn build_page(items: &[(&str, f64, f64)]) -> ExtractedPage {
        let mut segs = Vec::new();
        let mut prev_bottom = 0.0;
        for (i, (text, h, gap)) in items.iter().enumerate() {
            let top = if i == 0 { 800.0 } else { prev_bottom - gap };
            let bottom = top - h;
            segs.push(seg(text, 72.0, bottom, 300.0, *h, i as i64));
            prev_bottom = bottom;
        }
        page(segs)
    }

    fn recognize(p: &ExtractedPage) -> Vec<StructuredBlock> {
        recognize_page(p, &mut RecognizerState::new())
    }

    #[test]
    fn group_lines_splits_on_baseline_and_inserts_space() {
        // 同じ y の 2 セグメント → 1 行（水平ギャップに空白補完）。下段は別行。
        let p = page(vec![
            seg("Hello", 72.0, 800.0, 30.0, 10.0, 0),
            seg("world", 110.0, 800.0, 30.0, 10.0, 1),
            seg("next", 72.0, 780.0, 25.0, 10.0, 2),
        ]);
        let lines = group_lines(&p);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Hello world");
        assert_eq!(lines[1].text, "next");
        assert_eq!(lines[0].reading_order, 0);
    }

    #[test]
    fn group_lines_joins_touching_segments_without_space() {
        // 水平ギャップが無い（隣接）2 セグメントは空白を挟まず連結。
        let p = page(vec![
            seg("Hel", 72.0, 800.0, 15.0, 10.0, 0),
            seg("lo", 87.0, 800.0, 10.0, 10.0, 1),
        ]);
        let lines = group_lines(&p);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Hello");
    }

    #[test]
    fn group_blocks_splits_on_large_vertical_gap() {
        // 小ギャップの 3 行 + 大ギャップ 1 行 → 2 ブロック。
        let p = build_page(&[
            ("line one of paragraph here now", 10.0, 0.0),
            ("line two of paragraph here now", 10.0, G),
            ("line three of paragraph now ok", 10.0, G),
            ("a separated far away last line", 10.0, H),
        ]);
        let lines = group_lines(&p);
        let blocks = group_blocks(lines);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].len(), 3);
        assert_eq!(blocks[1].len(), 1);
    }

    #[test]
    fn numbered_heading_becomes_section() {
        let p = build_page(&[
            ("3 Method", 12.0, 0.0),
            ("We describe the proposed approach here", 10.0, H),
            ("and give the full training procedure", 10.0, G),
            ("with all hyperparameters listed below", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Section);
        assert_eq!(blocks[0].section_number.as_deref(), Some("3"));
        assert_eq!(blocks[0].heading_level, Some(1));
        assert_eq!(blocks[1].kind, NodeKind::Paragraph);
    }

    #[test]
    fn deep_number_becomes_subsection() {
        let p = build_page(&[
            ("3.2 Details of the Model", 12.0, 0.0),
            ("The model consists of stacked layers", 10.0, H),
            ("each with attention and a feedforward", 10.0, G),
            ("block followed by a normalization step", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Subsection);
        assert_eq!(blocks[0].section_number.as_deref(), Some("3.2"));
        assert_eq!(blocks[0].heading_level, Some(2));
    }

    #[test]
    fn figure_and_table_captions_are_detected() {
        let p = build_page(&[
            ("some earlier body sentence appears here", 10.0, 0.0),
            ("and it continues onto a second line", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
            ("Figure 1: The overall pipeline diagram", 10.0, H),
            ("Table 2: Accuracy across all datasets", 10.0, H),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Paragraph);
        assert_eq!(blocks[1].kind, NodeKind::FigureCaption);
        assert_eq!(blocks[2].kind, NodeKind::TableCaption);
    }

    #[test]
    fn abstract_state_machine_tags_body_then_resets_on_next_heading() {
        let p = build_page(&[
            ("Abstract", 12.0, 0.0),
            ("We present a fast method for the task", 10.0, H),
            ("and we evaluate it on three datasets", 10.0, G),
            ("with strong and consistent results", 10.0, G),
            ("1 Introduction", 12.0, H),
            ("Neural networks are widely used today", 10.0, H),
            ("and their scale keeps growing steadily", 10.0, G),
            ("across many application domains now", 10.0, G),
        ]);
        let blocks = recognize(&p);
        let kinds: Vec<NodeKind> = blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                NodeKind::Heading,   // "Abstract"
                NodeKind::Abstract,  // abstract body
                NodeKind::Section,   // "1 Introduction"
                NodeKind::Paragraph, // intro body (mode reset to Body)
            ]
        );
    }

    #[test]
    fn references_make_bibliography_entries_and_suppress_numbering() {
        let p = build_page(&[
            ("References", 12.0, 0.0),
            ("1. Smith, J. and Doe, A. Foo Bar. 2020", 10.0, H),
            ("2. Lee, C. and Kim, D. Baz Qux. 2021", 10.0, G),
            ("3. Park, E. Quux Corge Grault. 2022", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Heading); // "References"
                                                       // "1. Smith…" must NOT be parsed as a numbered section here.
        assert_eq!(blocks[1].kind, NodeKind::BibliographyEntry);
    }

    #[test]
    fn numbered_reference_line_is_section_in_body_without_references() {
        // 同じ "1. Author…" 行でも、References 見出しが先行しなければ番号付き節に見える
        // （biblio モードだけがこの誤検出を抑える、という対比）。
        let p = build_page(&[
            ("1. Smith, J. and Doe Foo Bar 2020", 10.0, 0.0),
            ("following body text line one here now", 10.0, H),
            ("following body text line two here now", 10.0, G),
            ("following body text line three now ok", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Section);
        assert_eq!(blocks[0].section_number.as_deref(), Some("1"));
    }

    #[test]
    fn page_number_is_unknown_block() {
        let p = page(vec![seg("12", 72.0, 780.0, 20.0, 10.0, 0)]);
        let blocks = recognize(&p);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, NodeKind::UnknownBlock);
    }

    #[test]
    fn font_size_heading_requires_letters() {
        // 大フォントでも純数字（ページ番号）は見出しにしない。
        assert!(detect_heading("123", 1, 1, 20.0, 10.0, false).is_none());
        // 文字があれば大フォント見出しとして拾う。
        assert!(detect_heading("Method", 1, 1, 20.0, 10.0, false).is_some());
    }

    #[test]
    fn font_size_heading_rejects_symbol_heavy_math() {
        // 記号主体の display 数式は大フォントでも見出しにしない（数式は Phase 3）。
        assert!(detect_heading("U − t U 0tU 0 ac(U0U0)", 1, 6, 20.0, 10.0, false).is_none());
        // 文字主体の見出しは通す。
        assert!(detect_heading("Definition of the Model", 1, 4, 20.0, 10.0, false).is_some());
    }

    #[test]
    fn large_single_level_number_is_not_a_section() {
        // "104 A. Suzuki"（ランニングヘッダ）は section にしない。
        assert!(detect_heading("104 A. Suzuki", 1, 3, 10.0, 10.0, false).is_none());
        // 2020（年）も単一レベル ≥100 なので節にしない。
        assert!(detect_heading("2020 was a productive year", 1, 5, 10.0, 10.0, false).is_none());
        // 通常の節番号は拾う。
        let h = detect_heading("3 Method", 1, 2, 10.0, 10.0, false).unwrap();
        assert_eq!(h.kind, NodeKind::Section);
        assert_eq!(h.section_number.as_deref(), Some("3"));
    }

    #[test]
    fn running_header_in_top_margin_becomes_unknown() {
        // ページ上端（page() の height 842 → top 90% = 757.8pt 超）の短い 1 行は unknown へ降格。
        let p = page(vec![seg("104 A. Suzuki", 72.0, 795.0, 120.0, 10.0, 0)]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::UnknownBlock);
    }

    #[test]
    fn plain_body_is_paragraph_with_moderate_confidence() {
        let p = build_page(&[
            ("This is a normal body paragraph that", 10.0, 0.0),
            ("spans a few lines of running prose", 10.0, G),
            ("without any special leading markers", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, NodeKind::Paragraph);
        assert!((blocks[0].confidence - 0.6).abs() < 1e-9);
        // ブロック統合テキストは行を空白でつなぐ。
        assert!(blocks[0].text.starts_with("This is a normal body paragraph"));
        assert_eq!(blocks[0].lines.len(), 3);
    }

    #[test]
    fn empty_page_yields_no_blocks() {
        let p = page(vec![]);
        assert!(recognize(&p).is_empty());
    }

    #[test]
    fn state_persists_across_pages() {
        // ページ 1 で Abstract 見出し、ページ 2 冒頭の本文も Abstract 継続。
        let mut state = RecognizerState::new();
        let p1 = build_page(&[("Abstract", 12.0, 0.0)]);
        let b1 = recognize_page(&p1, &mut state);
        assert_eq!(b1[0].kind, NodeKind::Heading);

        let p2 = build_page(&[
            ("the abstract continues on this page", 10.0, 0.0),
            ("with additional summary sentences here", 10.0, G),
        ]);
        let b2 = recognize_page(&p2, &mut state);
        assert_eq!(b2[0].kind, NodeKind::Abstract);
    }
}
