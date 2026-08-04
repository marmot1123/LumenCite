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
    /// 数式番号（"(2.1)" 等）。display_math のみ・検出できたとき。
    pub equation_label: Option<String>,
    /// 定理番号（"2.3" / "A.1" 等）。定理系ノードのみ・行頭テキストから検出できたとき（Phase 5）。
    pub theorem_number: Option<String>,
    /// 定理・証明の付記名（"Theorem 1 (Zorn)." の "Zorn"）。定理系ノードのみ（Phase 5）。
    pub note: Option<String>,
    /// caption のラベル語（"Figure" / "Fig" / "Table" / "Algorithm" / "Listing"・正規化済み）。
    /// caption ノードのみ（Phase 8a）。図領域ペアリングで Algorithm/Listing を除外する鍵。
    pub caption_label: Option<String>,
    /// caption の番号（"Figure 2:" → "2"・"A.1" 形も）。caption ノードのみ・検出できたとき（Phase 8a）。
    pub caption_number: Option<String>,
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
    recognize_blocks(&page.blocks, page.height_pt, page.box_bottom, state)
}

/// [`recognize_page`] の本体。`ExtractedPage` を組む**前**に呼べるように、実際に読む 3 つの値
/// （セグメント列・ページ box の高さ・box 下端）だけを取る形にしてある。
///
/// Phase 8d-2 が `pdf::extract_document` のページループ内で図 caption の位置を知る必要があり、
/// そこではまだ `image_regions` が決まっていない（＝ `ExtractedPage` を組めない）ため。
/// **同じページ列を同じ順で、それぞれ新しい [`RecognizerState`] から回す限り、抽出側の pass と
/// build 側の pass（`ingestion::insert_pdf_version_tx`）は同じ結果になる** ── `state` の遷移が
/// ページ列の畳み込みで決まり、他に副作用が無いため。
///
/// **テキスト抽出に失敗したページで両者が食い違わないことも保証されている。** 抽出側は
/// そのページで本関数を呼ばずに次へ進むが、build 側は `blocks` が空のまま呼ぶ ── そのとき
/// `group_lines` が空を返して**`state` に触れずに**早期 return するので、`state` の遷移は同じ。
/// この性質に依存しているので、空入力で `state` を触る変更を入れてはいけない。
pub fn recognize_blocks(
    blocks: &[crate::ingestion::pdf::ExtractedBlock],
    page_height_pt: f64,
    box_bottom: f64,
    state: &mut RecognizerState,
) -> Vec<StructuredBlock> {
    let lines = group_lines(blocks);
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
        if let Some(block) = classify_block(lines, page_median_h, page_height_pt, box_bottom, state)
        {
            out.push(block);
        }
    }
    out
}

// ---- 行のグルーピング（セグメント → 行） ----

fn group_lines(blocks: &[crate::ingestion::pdf::ExtractedBlock]) -> Vec<StructuredLine> {
    let mut lines: Vec<StructuredLine> = Vec::new();
    // 現在の行に積んでいるセグメント（bbox, text, reading_order）。
    let mut cur: Vec<&crate::ingestion::pdf::ExtractedBlock> = Vec::new();

    for seg in blocks {
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
    box_bottom: f64,
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
            equation_label: None,
            theorem_number: None,
            note: None,
            caption_label: None,
            caption_number: None,
            lines,
        })
    };

    let in_bibliography = matches!(state.mode, Mode::Bibliography);

    // 1. caption（参考文献モードでは "Figure" は稀なのでスキップ）。行頭ラベルが最優先。
    //    ラベル語と番号は payload に載せる（Phase 8a: 図領域ペアリング・図番号参照の鍵）。
    if !in_bibliography {
        if let Some(cap) = detect_caption(first) {
            return Some(StructuredBlock {
                kind: cap.kind,
                text: text.clone(),
                bbox,
                confidence: 0.75,
                heading_level: None,
                section_number: None,
                equation_label: None,
                theorem_number: None,
                note: None,
                caption_label: Some(cap.label.to_string()),
                caption_number: cap.number,
                lines,
            });
        }
    }

    // 2. 定理・定義・証明（Phase 5・PDF 由来のヒューリスティック）。行頭キーワード + 番号で判定し、
    //    数式・見出し検出より先に見る（"Theorem 2.3." の '.' や記号で誤分類させない）。確信は
    //    中程度で、番号・note を payload に載せる（呼び出し側で origin=layout_model）。
    if !in_bibliography {
        if let Some(th) = detect_theorem(first) {
            return Some(StructuredBlock {
                kind: th.kind,
                text: text.clone(),
                bbox,
                confidence: th.confidence,
                heading_level: None,
                section_number: None,
                equation_label: None,
                theorem_number: th.number,
                note: th.note,
                caption_label: None,
                caption_number: None,
                lines,
            });
        }
    }

    // 3. display 数式。見出しより先に見て、大フォントの数式を見出しに誤判定させない
    //    （Phase 3。意味は取らず表層のみ＝呼び出し側で semantic_status='surface_only'）。
    if !in_bibliography {
        if let Some((confidence, equation_label)) = detect_display_math(&text, lines.len()) {
            return Some(StructuredBlock {
                kind: NodeKind::DisplayMath,
                text: text.clone(),
                bbox,
                confidence,
                heading_level: None,
                section_number: None,
                equation_label,
                theorem_number: None,
                note: None,
                caption_label: None,
                caption_number: None,
                lines,
            });
        }
    }

    // 4. 見出し（参考文献モードでは番号付き見出しを無効化 = "1. Author…" の誤検出回避）。
    if let Some(h) = detect_heading(
        first,
        lines.len(),
        word_count,
        block_median_h,
        page_median_h,
        in_bibliography,
    ) {
        state.mode = match h.keyword {
            Some("abstract") => Mode::Abstract,
            Some("references") | Some("bibliography") => Mode::Bibliography,
            _ => Mode::Body,
        };
        return mk(h.kind, h.confidence, h.level, h.section_number, lines);
    }

    // 5. モードに応じた本文分類。
    match state.mode {
        Mode::Abstract => mk(NodeKind::Abstract, 0.7, None, None, lines),
        Mode::Bibliography => mk(NodeKind::BibliographyEntry, 0.5, None, None, lines),
        Mode::Body => {
            // ページ上下の極端なマージンにある短い 1 行は、ランニングヘッダ/フッタ/ページ番号の
            // 可能性が高い。段落と確定せず unknown_block に降格する（誤った型より欠損を許容）。
            //
            // **帯はページ境界 box の原点から測る**（debt-18）。`bbox.y` は絶対 user space
            // （MediaBox 基準）で、`page_height` は box の**高さ**でしかないので、原点が非ゼロの
            // PDF で `box_bottom` を落とすと帯が下へずれる ── 本文の短い行が上端帯に入って
            // 降格し（実測 vid 149 で誤った帯 1,179 行 vs 正しい帯 272 行）、逆に box 下端すぐ上に
            // ある本物の走り柱は下端帯に届かず段落として残る。
            let in_margin = page_height > 1.0
                && lines.len() == 1
                && word_count <= 8
                && (bbox.y > box_bottom + page_height * 0.90
                    || bbox.y + bbox.height < box_bottom + page_height * 0.10);
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

/// caption 検出の結果（Phase 8a でラベル語・番号を追加）。
struct CaptionHit {
    kind: NodeKind,
    /// 正規化したラベル語（"FIGURE 2" でも "Figure"）。
    label: &'static str,
    /// caption 番号（"2" / "A.1"）。取れないときは None（検出自体は従来どおり成立する）。
    number: Option<String>,
}

/// 行頭が "Figure 1" / "Table 2:" / "Fig. 3" / "TABLE III." のような caption ラベルか。
fn detect_caption(first: &str) -> Option<CaptionHit> {
    let f = first.trim_start();
    let lower = f.to_ascii_lowercase();
    let (label_len, kind, label) = if lower.starts_with("figure") {
        (6, NodeKind::FigureCaption, "Figure")
    } else if lower.starts_with("fig.") {
        (4, NodeKind::FigureCaption, "Fig")
    } else if lower.starts_with("fig ") {
        (3, NodeKind::FigureCaption, "Fig")
    } else if lower.starts_with("table") {
        (5, NodeKind::TableCaption, "Table")
    } else if lower.starts_with("algorithm") {
        (9, NodeKind::FigureCaption, "Algorithm")
    } else if lower.starts_with("listing") {
        (7, NodeKind::FigureCaption, "Listing")
    } else {
        return None;
    };
    // ラベル語はここまでの照合で ASCII と確定しているのでバイト境界で切ってよい。
    let (label_text, rest) = f.split_at(label_len);
    let rest = rest.trim_start();
    // ラベル直後の数文字以内に番号（数字）があること（"Figures show…" の誤検出回避）。
    let has_digit = f[label_len..].chars().take(6).any(|c| c.is_ascii_digit());
    // 算用数字が無くても、全大文字ラベル + ローマ数字 + 終端記号なら caption（debt-12）。
    let roman = roman_caption_number(label_text, rest);
    if !has_digit && roman.is_none() {
        return None;
    }
    // 番号は算用数字を優先し、読めないときだけローマ数字で埋める（"TABLE II. 3 …" 形）。
    let number = parse_theorem_number(rest).or(roman);
    Some(CaptionHit { kind, label, number })
}

/// 全大文字ラベルに続くローマ数字の caption 番号（"TABLE III." → `"III"`）。debt-12。
///
/// 算用数字に直さずローマ数字のまま返す。参照側（`graph::take_ref_number`）は ASCII 数字しか
/// 読まないので照合には使われず、`node_kind` の正しさのためだけに取る（§8.1）。
///
/// ガードは 3 つで、いずれも「誤検出より欠損」側に倒してある。
///
/// - **ラベルが全大文字**であること。本文の文末が偶然この形になる
///   （"… as shown in Table III. Since the extended Hermitian system H …"）ため、終端記号だけでは
///   本文と分離できない（実ライブラリ v171 に実例）。実測では全大文字を要求すると
///   利得 48 ブロック / 12 版に対しこの型の混入が 0 になる。
/// - **直後に終端記号**（`.` / `:`）。"TABLE XIV shows the equivalence …" のような本文参照を弾く。
/// - **標準形のローマ数字**であること。ローマ数字の文字だけでできた英単語（"DIM"）や
///   非標準表記（"IIII"）を弾く。
fn roman_caption_number(label: &str, rest: &str) -> Option<String> {
    if !label
        .chars()
        .all(|c| !c.is_ascii_alphabetic() || c.is_ascii_uppercase())
    {
        return None;
    }
    let run: String = rest
        .chars()
        .take_while(|c| roman_digit(*c).is_some())
        .collect();
    if !is_canonical_roman(&run) {
        return None;
    }
    // run は ASCII なのでバイト長 = 文字数。
    let after = &rest[run.len()..];
    // 章番号形（"FIGURE II.2" = 第 II 章の図 2）。ローマ数字の部分だけを番号と名乗ると
    // 章番号を図番号と偽ることになる（欠損より悪い）ので、続く算用数字の枝番も取り込む。
    // この形は算用数字を含むため**そもそも従来から caption**（`has_digit` が真）で、
    // ここで変わるのは番号だけ ＝ 分類の誤検出リスクは増えない。
    // `parse_theorem_number` の付録形は大文字 1 字しか見ないので "I.1" は拾えるが "II.2" は
    // 拾えず、直さないと同じ文書の中で 1 文字の章だけ正しい非対称になる。
    if let Some(tail) = compound_arabic_tail(after) {
        return Some(format!("{run}{tail}"));
    }
    if !after.starts_with(['.', ':']) {
        return None;
    }
    Some(run)
}

/// ローマ数字の章番号に続く算用数字の枝番（"II" の後ろの ".2" / ".2.1"）。
/// **`.` の直後が数字のときだけ**採る ── "TABLE III. 4 experiments" のように終端記号のあとに
/// 空白を置いて文が続く形（この `.` は番号の区切りではない）と区別するため。
fn compound_arabic_tail(after: &str) -> Option<String> {
    let mut chars = after.chars();
    if chars.next()? != '.' || !chars.next()?.is_ascii_digit() {
        return None;
    }
    let tail: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    Some(tail.trim_end_matches('.').to_string())
}

/// ローマ数字 1 文字の値（大文字のみ）。
fn roman_digit(c: char) -> Option<u32> {
    Some(match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        'M' => 1000,
        _ => return None,
    })
}

/// 減算記法を含む**標準形**の 1–3999 のローマ数字か。値に読んでから正準表記へ描き直し、
/// 入力と一致するかで判定する（"IIII" や "DIM" のような非標準表記・英単語を弾くため）。
fn is_canonical_roman(s: &str) -> bool {
    // 1–3999 の正準表記は最長 15 文字（3888 = "MMMDCCCLXXXVIII"）。"MMMCMXCIX" は最大値の
    // 表記であって最長ではない。長さで先に切って加算側の溢れも同時に防ぐ
    // （15 文字 × 最大 1000 = 15,000 で u32 に収まる）。
    if s.is_empty() || s.len() > 15 {
        return false;
    }
    let (mut total, mut prev) = (0u32, 0u32);
    for c in s.chars().rev() {
        let Some(v) = roman_digit(c) else {
            return false;
        };
        // 右から見て直前（右隣）より小さい文字は減算記法。減算が積み上がると総和は負になる
        // （"IIIIIIV" = V の後ろに I が 6 つ）ので checked で弾く。非標準表記なのでどのみち
        // 正準化判定で落ちるが、**debug ビルドでは overflow で panic する**（OCR 崩れの
        // 実テキストから届きうる経路なので、値ではなく算術で止める）。
        if v < prev {
            let Some(t) = total.checked_sub(v) else {
                return false;
            };
            total = t;
        } else {
            total += v;
            prev = v;
        }
    }
    total <= 3999 && canonical_roman_of(total) == s
}

/// 1–3999 の値の正準ローマ数字表記。
fn canonical_roman_of(mut n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (v, s) in TABLE {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// caption のラベル語が「図（figure）の caption」か。`detect_caption` は Algorithm / Listing も
/// `figure_caption` にするので、図領域ペアリング（Phase 8a）と図番号参照（Phase 8d-7）は
/// この述語で絞る。ラベル語のリテラルを 1 箇所に集約し、両者が同じ集合を見ることを保証する。
pub fn is_figure_caption_label(label: Option<&str>) -> bool {
    matches!(label, Some("Figure") | Some("Fig"))
}

/// 同上・表（table）の caption か。`detect_caption` が `TableCaption` にするラベル語は
/// "Table" だけなので現状は kind と等価だが、参照側（Phase 8d-7）と対で持たせて
/// ラベル語の追加時に両方が同時に効くようにする。
pub fn is_table_caption_label(label: Option<&str>) -> bool {
    matches!(label, Some("Table"))
}

/// 「この矩形は本文（散文・数式・参考文献）である」と読める block 級ノード種別か（Phase 8d-2）。
///
/// ベクター図の誤検出ガードに使う ── path クラスタが本文をどれだけ覆っているかを測り、
/// 覆いすぎているものは「本文段を図と誤認した」として捨てる。
///
/// **`unknown_block` は本文に数えない。** 図の中の記号・数式断片は `looks_like_prose`
/// （英字 3 文字以上）を通らず `unknown_block` に落ちるので、数えると図そのものを弾いてしまう。
/// **ただしこれで図内テキストを全部除けるわけではない** ── "number of papers" のような
/// 軸ラベル・凡例は英字 3 文字以上なので `paragraph` になる。ガードが面積比
/// （`figures::VECTOR_MAX_PROSE_COVER`）なのはそのためで、図の中の小さな注記が数個あっても
/// 面積では効かない。**逆に、注記が図面積の 35% を超えるほど密な小さい図は落ちる**（既知の限界）。
/// **`figure_caption` も数えない。** caption は図に隣接し、`pair_captions` は多少の重なりを
/// 許容している（マージ後の領域が caption に食い込む形）ので、数えると正しいペアを捨てる。
/// 一方 **`display_math` は数える** ── 分数線・根号は path なので、数式ブロックは
/// ベクタークラスタの最大の誤検出源になる。
pub fn is_prose_block_kind(kind: NodeKind) -> bool {
    !matches!(
        kind,
        NodeKind::UnknownBlock | NodeKind::FigureCaption | NodeKind::Figure | NodeKind::Line
    )
}

/// 定理系ブロックの検出結果（Phase 5・PDF ヒューリスティック）。
struct TheoremHit {
    kind: NodeKind,
    number: Option<String>,
    note: Option<String>,
    confidence: f64,
}

/// 行頭が定理系キーワード（"Theorem 2.3." / "Proof." / "Definition (Name)." 等）で始まるか。
///
/// PDF レイアウト由来なので確信は中程度。参照文中の "Theorem 2 shows …"（キーワード + 番号の後が
/// 終端記号でない）は棄却し、誤検出より欠損を選ぶ。number（"2.3"）と note（丸括弧名）を取り出す。
fn detect_theorem(first: &str) -> Option<TheoremHit> {
    let f = first.trim_start();
    let word: String = f.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if word.is_empty() {
        return None;
    }
    let kind = match word.to_ascii_lowercase().as_str() {
        "theorem" => NodeKind::Theorem,
        "lemma" => NodeKind::Lemma,
        "proposition" => NodeKind::Proposition,
        "corollary" => NodeKind::Corollary,
        "definition" => NodeKind::Definition,
        "remark" => NodeKind::Remark,
        "example" => NodeKind::Example,
        "proof" => NodeKind::Proof,
        _ => return None,
    };
    let rest = f[word.len()..].trim_start();

    // proof は番号を持たないことが多い（"Proof." / "Proof of Theorem 2.3." / "Proof:"）。
    if kind == NodeKind::Proof {
        let lower = rest.to_ascii_lowercase();
        let ok = rest.is_empty()
            || rest.starts_with(['.', ':', '(', '—', '–'])
            || lower == "of"
            || lower.starts_with("of ");
        return ok.then_some(TheoremHit {
            kind,
            number: None,
            note: note_before_period(rest),
            confidence: 0.6,
        });
    }

    // それ以外は「番号（任意）+ 終端記号」で定理見出しとみなす。
    let number = parse_theorem_number(rest);
    let after_num = match &number {
        Some(n) => rest[n.len()..].trim_start(),
        None => rest,
    };
    let terminated = after_num.starts_with(['.', ':', '(', '—', '–']);
    if !terminated {
        return None;
    }
    // 番号 + 終端は確信高め、番号なし（"Definition."）は中程度。
    let confidence = if number.is_some() { 0.7 } else { 0.6 };
    Some(TheoremHit {
        kind,
        number,
        note: note_before_period(after_num),
        confidence,
    })
}

/// 行頭の定理番号（"2" / "2.3" / 付録 "A.1"）を取り出す。数字を含まなければ None。
fn parse_theorem_number(s: &str) -> Option<String> {
    let b = s.as_bytes();
    // 付録形 "A.1"（大文字 + '.' + 数字）。
    let appendix = b.len() >= 3
        && b[0].is_ascii_uppercase()
        && b[1] == b'.'
        && b[2].is_ascii_digit();
    let prefix: String = if appendix {
        std::iter::once(b[0] as char)
            .chain(
                s[1..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.'),
            )
            .collect()
    } else {
        s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect()
    };
    let trimmed = prefix.trim_end_matches('.');
    if trimmed.is_empty() || !trimmed.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.to_string())
}

/// 最初の '.'/':' より前に現れる "(..)" の中身を付記名として取り出す（"(Zorn). Statement" → "Zorn"）。
/// 文中の "(x, y)" を誤って拾わないよう、括弧は先頭の終端記号より前にある場合だけ採用する。
fn note_before_period(s: &str) -> Option<String> {
    let paren = s.find('(')?;
    let terminator = s
        .find(['.', ':'])
        .unwrap_or(usize::MAX);
    if paren > terminator {
        return None;
    }
    let close = s[paren + 1..].find(')')?;
    let inner = s[paren + 1..paren + 1 + close].trim();
    (!inner.is_empty()).then(|| inner.to_string())
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

/// 強い数式シグナル文字（関係・演算子・量化子・集合・矢印・黒板太字 等）。
/// **ASCII ルックアライク（'-' ハイフン, 'x', '*')は含めない**。pdfium は数式のマイナスを
/// U+2212 '−'、乗算を '·'/'×' で出すので、散文のハイフンや変数 x と区別できる。'=' '<' '>' '+' は
/// 学術散文では稀なので含める（isolation + alpha_ratio ガードと併せて誤検出を抑える）。
const MATH_STRONG: &[char] = &[
    '=', '≠', '≈', '≃', '≅', '≡', '≤', '≥', '≪', '≫', '<', '>', '≺', '≻',
    '+', '−', '±', '∓', '×', '÷', '·', '⋅', '∗', '∘', '⊗', '⊕', '⊙', '⊘',
    '∑', '∏', '∫', '∬', '∮', '√', '∛', '∞', '∂', '∇', '∝', '∆', '∈', '∉',
    '∋', '⊂', '⊆', '⊃', '⊇', '⊄', '∪', '∩', '∖', '∅', '∀', '∃', '∄', '∴', '∵',
    '→', '↦', '↔', '⇒', '⇔', '⟨', '⟩', '‖', '⌊', '⌋', '⌈', '⌉', '∧', '∨', '¬',
    'ℝ', 'ℂ', 'ℤ', 'ℕ', 'ℚ', 'ℍ', 'ℋ', 'ℓ', '℘', '′', '″', '⊤', '⊥', '⊢', '⊨',
];

fn strong_math_count(t: &str) -> usize {
    t.chars().filter(|c| MATH_STRONG.contains(c)).count()
}

/// 独立した display 数式か（Phase 3・表層のみ）。ブロックが短く、数式記号を持ち、散文優位でない
/// ときに `(信頼度, 数式番号)` を返す。演算子の無い（記号が飛んだ）式は拾えない＝欠損を許容。
fn detect_display_math(text: &str, line_count: usize) -> Option<(f64, Option<String>)> {
    if line_count > 3 {
        return None;
    }
    let strong = strong_math_count(text);
    if strong == 0 {
        return None;
    }
    let ratio = alpha_ratio(text);
    // 散文優位（英字が 7 割以上）は、記号がいくつ混じっても数式にしない。
    if ratio >= 0.7 {
        return None;
    }
    let label = extract_equation_label(text);
    // 記号 2 個以上 / 英字が半分未満 / 数式番号つき、のいずれかで数式とみなす。
    if strong >= 2 || ratio < 0.6 || label.is_some() {
        let confidence = (0.5 + 0.05 * strong as f64).min(0.75);
        Some((confidence, label))
    } else {
        None
    }
}

/// 行末の数式番号 "(2)" / "(2.1)" / "(A.1)" を取り出す。式の一部の "(U0U0)" 等は弾く。
fn extract_equation_label(text: &str) -> Option<String> {
    let t = text.trim_end();
    if !t.ends_with(')') {
        return None;
    }
    let open = t.rfind('(')?;
    let inner = &t[open + 1..t.len() - 1];
    if inner.is_empty() || inner.chars().count() > 10 {
        return None;
    }
    // 純数値（"2" / "2.1"）または付録式（"A.1" = 大文字 + '.' + 数字）だけを数式番号とみなす。
    let pure_numeric = inner.chars().all(|c| c.is_ascii_digit() || c == '.')
        && inner.chars().any(|c| c.is_ascii_digit());
    let bytes = inner.as_bytes();
    let appendix = inner.len() >= 3
        && bytes[0].is_ascii_uppercase()
        && bytes[1] == b'.'
        && bytes[2].is_ascii_digit();
    if pure_numeric || appendix {
        Some(format!("({inner})"))
    } else {
        None
    }
}

// ---- 小物ユーティリティ ----

fn union_bbox(a: BBox, b: BBox) -> BBox {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let top = (a.y + a.height).max(b.y + b.height);
    BBox::new(x, y, right - x, top - y)
}

/// 空白を 1 個に正規化しつつ、非空白の制御文字を落とす。pdfium はマップできない数式グリフを
/// C0 制御文字（\u{2} 等）で吐くことがあり、そのままだと検索や表層文字列を汚すため除去する。
fn normalize_ws(s: &str) -> String {
    s.split_whitespace()
        .map(|w| w.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// page ノードの `plain_text` 用の軽量クリーナー（debt-22）。
///
/// **[`normalize_ws`] は使えない。** あちらは空白を 1 個に潰すので、page に掛けると
/// `get_fulltext` が返す `"[page N]\n{content}"` が 1 行の塊になり、チャット LLM / MCP に
/// 渡す本文の可読性を落とす（block / line 側は 1 ブロック = 1 行なので潰してよい）。
///
/// ここで落とすのは**紙に出ないのに検索と LLM 入力を汚すものだけ**:
///
/// 1. `\n` / `\t` 以外の **C0 制御文字**（U+0000..U+001F）。pdfium はマップできない
///    数式グリフを `\u{2}` 等で吐き、それが**語の内側に刺さる**（実データに `"consis\u{2}tent"`）。
///    FTS5 の trigram 索引では語が割れて検索から落ちる。
/// 2. `\r\n` / 単独 `\r` → `\n`（改行コードの揺れ。実 DB では非空 5,803 ページ中
///    **5,786 ページ**に `\r` がある）。
///
/// **DEL(U+007F) と C1 は落とさない** ── debt-22 の実測（C0 のみ）と母集団を一致させ、
/// 「直した後は 0 件」を同じ定義で検算できるようにするため。
pub fn clean_page_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next(); // \r\n を 1 個の \n にする
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(c),
            c if (c as u32) < 0x20 => {} // 残りの C0 は落とす
            c => out.push(c),
        }
    }
    out
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

    #[test]
    fn prose_block_kinds_exclude_what_a_figure_is_made_of() {
        // 8d-2 の本文被覆ガードの入力。**除外集合が緩むと図が自分の中身で弾かれる**。
        assert!(!is_prose_block_kind(NodeKind::UnknownBlock), "図中の記号・数式断片");
        assert!(!is_prose_block_kind(NodeKind::FigureCaption), "図に隣接し重なりも許容している");
        assert!(!is_prose_block_kind(NodeKind::Figure));
        assert!(!is_prose_block_kind(NodeKind::Line));
        // **`display_math` は本文に数える** ── 分数線・根号は path なので、数式ブロックは
        // ベクタークラスタの最大の誤検出源。
        assert!(is_prose_block_kind(NodeKind::DisplayMath));
        assert!(is_prose_block_kind(NodeKind::Paragraph));
        assert!(is_prose_block_kind(NodeKind::BibliographyEntry));
        assert!(is_prose_block_kind(NodeKind::Section));
        assert!(is_prose_block_kind(NodeKind::Theorem));
        assert!(is_prose_block_kind(NodeKind::TableCaption));
    }

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
        page_with_box(segs, 0.0, 0.0, 595.0, 842.0)
    }

    /// 原点が非ゼロのページ境界 box を持つページ（雑誌・紀要の PDF）。
    fn page_with_box(
        segs: Vec<ExtractedBlock>,
        box_left: f64,
        box_bottom: f64,
        width_pt: f64,
        height_pt: f64,
    ) -> ExtractedPage {
        ExtractedPage {
            page_number: 1,
            width_pt,
            height_pt,
            box_left,
            box_bottom,
            rotation_deg: 0.0,
            plain_text: String::new(),
            blocks: segs,
            image_regions: Vec::new(),
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
        let lines = group_lines(&p.blocks);
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
        let lines = group_lines(&p.blocks);
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
        let lines = group_lines(&p.blocks);
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
        // Phase 8a: ラベル語と番号を payload 用に捕捉する。
        assert_eq!(blocks[1].caption_label.as_deref(), Some("Figure"));
        assert_eq!(blocks[1].caption_number.as_deref(), Some("1"));
        assert_eq!(blocks[2].caption_label.as_deref(), Some("Table"));
        assert_eq!(blocks[2].caption_number.as_deref(), Some("2"));
    }

    /// Phase 8a の図領域ペアリングと Phase 8d-7 の図番号参照は同じラベル述語で絞る。
    /// Algorithm / Listing は `figure_caption` だが図番号ではないので両方から外れること。
    #[test]
    fn figure_caption_label_predicates_exclude_algorithm_and_listing() {
        assert!(is_figure_caption_label(Some("Figure")));
        assert!(is_figure_caption_label(Some("Fig")));
        assert!(!is_figure_caption_label(Some("Algorithm")));
        assert!(!is_figure_caption_label(Some("Listing")));
        assert!(!is_figure_caption_label(Some("Table")));
        assert!(!is_figure_caption_label(None));

        assert!(is_table_caption_label(Some("Table")));
        assert!(!is_table_caption_label(Some("Figure")));
        assert!(!is_table_caption_label(None));
    }

    #[test]
    fn caption_label_variants_and_appendix_numbers() {
        let p = build_page(&[
            ("some earlier body sentence appears here", 10.0, 0.0),
            ("and it continues onto a second line", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
            ("plus a fourth line keeping gaps small", 10.0, G),
            ("and a fifth line to hold the median down", 10.0, G),
            ("Fig. 3a shows the apparatus in detail", 10.0, H),
            ("Algorithm 2: Greedy matching procedure", 10.0, H),
            ("Table A.1: Supplementary hyperparameters", 10.0, H),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[1].kind, NodeKind::FigureCaption);
        assert_eq!(blocks[1].caption_label.as_deref(), Some("Fig"));
        assert_eq!(blocks[1].caption_number.as_deref(), Some("3"));
        // Algorithm/Listing は FigureCaption だがラベル語で区別できる（図領域ペアリングから除外する鍵）。
        assert_eq!(blocks[2].kind, NodeKind::FigureCaption);
        assert_eq!(blocks[2].caption_label.as_deref(), Some("Algorithm"));
        assert_eq!(blocks[2].caption_number.as_deref(), Some("2"));
        // 付録番号 "A.1" も取れる。
        assert_eq!(blocks[3].kind, NodeKind::TableCaption);
        assert_eq!(blocks[3].caption_number.as_deref(), Some("A.1"));
    }

    /// debt-12: 全大文字ラベル + ローマ数字の caption（"TABLE III." 形）。番号は算用数字に
    /// 直さずローマ数字のまま payload に載せる（参照側は ASCII 数字しか読まないので照合には
    /// 使われない・§8.1）。
    #[test]
    fn all_caps_roman_captions_are_detected() {
        let p = build_page(&[
            ("some earlier body sentence appears here", 10.0, 0.0),
            ("and it continues onto a second line", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
            ("plus a fourth line keeping gaps small", 10.0, G),
            ("and a fifth line to hold the median down", 10.0, G),
            ("TABLE III. Limit distributions and 1D limits", 10.0, H),
            ("TABLE VIII: Difference between the two cases", 10.0, H),
            ("FIG. IV. Signs of the two coefficients here", 10.0, H),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[1].kind, NodeKind::TableCaption);
        assert_eq!(blocks[1].caption_label.as_deref(), Some("Table"));
        assert_eq!(blocks[1].caption_number.as_deref(), Some("III"));
        // 終端記号は ':' でもよい（"Table 2:" と同じ扱い）。
        assert_eq!(blocks[2].kind, NodeKind::TableCaption);
        assert_eq!(blocks[2].caption_number.as_deref(), Some("VIII"));
        // 図側も同じ規則（実ライブラリでは表側が大半だが規則は共通）。
        assert_eq!(blocks[3].kind, NodeKind::FigureCaption);
        assert_eq!(blocks[3].caption_label.as_deref(), Some("Fig"));
        assert_eq!(blocks[3].caption_number.as_deref(), Some("IV"));
    }

    /// ラベルが全大文字でなければローマ数字 caption にしない。本文の文末が偶然この形になる
    /// （"… as shown in Table III. Since the extended Hermitian system H …"）ためで、実ライブラリに
    /// 実例がある（v171）。**終端記号だけでは本文と分離できない**（§8.1 の初版はここを見落としていた）。
    #[test]
    fn mixed_case_roman_reference_is_not_a_caption() {
        let p = build_page(&[
            ("Table III. Since the extended Hermitian system", 10.0, 0.0),
            ("has the same spectrum we can classify the model", 10.0, G),
            ("and the argument carries over without change", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Paragraph);
        assert_eq!(blocks[0].caption_label, None);
    }

    /// 終端記号のないローマ数字は本文の参照（"TABLE XIV shows the equivalence …"）。
    /// 終端記号は '.' と ':' だけ — 読点で続く形（"TABLE VI, which lists …"）は本文なので通さない。
    #[test]
    fn roman_without_terminator_is_not_a_caption() {
        for first in [
            "TABLE XIV shows the equivalence between them",
            "TABLE VI, which lists the remaining symmetries",
        ] {
            let p = build_page(&[
                (first, 10.0, 0.0),
                ("as an additional symmetry of the same class", 10.0, G),
                ("which we use throughout the rest of the text", 10.0, G),
            ]);
            let blocks = recognize(&p);
            assert_eq!(blocks[0].kind, NodeKind::Paragraph, "誤検出: {first}");
            assert_eq!(blocks[0].caption_label, None, "誤検出: {first}");
        }
    }

    /// ローマ数字は標準形だけを受ける。ローマ数字の文字だけでできた英単語（"DIM"）や
    /// 非標準表記（"IIII"）を弾くための正準化判定。
    #[test]
    fn non_canonical_roman_is_not_a_caption() {
        for first in [
            "FIGURE DIM. of the reconstructed lattice sites",
            "TABLE IIII. of the coefficients used in the fit",
            "TABLE OF CONTENTS. listing every chapter here",
        ] {
            let p = build_page(&[
                (first, 10.0, 0.0),
                ("with a second line to make it a block", 10.0, G),
                ("and a third line to anchor the median", 10.0, G),
            ]);
            let blocks = recognize(&p);
            assert_eq!(blocks[0].caption_label, None, "誤検出: {first}");
        }
    }

    /// 章番号つきの図表番号（"FIGURE II.2" = 第 II 章の図 2）。ラベル直後 6 文字に数字があるので
    /// **従来から caption** で、変わるのは番号だけ。ローマ数字の部分だけを番号と名乗ると
    /// 章番号を図番号と偽ることになる（欠損より悪い）。`parse_theorem_number` の付録形は
    /// 大文字 1 字しか見ないので "I.1" は拾えるが "II.2" は拾えない ── 同じ文書の中で
    /// 1 文字の章だけ正しい、という非対称を作らないこと。実ライブラリ v250（node 2525439）に実在。
    #[test]
    fn compound_roman_chapter_number_is_not_truncated() {
        let p = build_page(&[
            ("FIGURE II.2 Gram-Schmidt orthogonalization", 10.0, 0.0),
            ("with a second line to make it a block", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::FigureCaption);
        assert_eq!(blocks[0].caption_number.as_deref(), Some("II.2"));

        // 1 文字の章番号は従来どおり付録形の経路で取れる（こちらは変わらない）。
        let p = build_page(&[
            ("FIGURE I.1 The unit ball in three norms", 10.0, 0.0),
            ("with a second line to make it a block", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].caption_number.as_deref(), Some("I.1"));

        // 枝番として採るのは「'.' の直後が数字」のときだけ。OCR 崩れの二重ドットは
        // 枝番と見なさずローマ数字の部分だけを採る（壊れた番号を組み立てない）。
        let p = build_page(&[
            ("TABLE III..2 of the measured coefficients", 10.0, 0.0),
            ("with a second line to make it a block", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].caption_number.as_deref(), Some("III"));
    }

    /// ローマ数字の文字が長く続く列（OCR 崩れの "TABLE IIIIIIV." 等）で、減算記法の累積により
    /// 内部の総和が負になる。**debug ビルドでは overflow で panic する**（`cargo test` も
    /// `pnpm tauri dev` も debug なので、崩れた PDF 1 本で取り込みが落ちる経路になる）。
    #[test]
    fn long_roman_letter_runs_do_not_panic() {
        for first in [
            "TABLE IIIIIIV. of the measured coefficients",
            "TABLE IIIIIIDDM. of the measured coefficients",
            "FIGURE IIIIIIIIV: of the measured coefficients",
        ] {
            let p = build_page(&[
                (first, 10.0, 0.0),
                ("with a second line to make it a block", 10.0, G),
                ("and a third line to anchor the median", 10.0, G),
            ]);
            let blocks = recognize(&p);
            assert_eq!(blocks[0].caption_label, None, "誤検出: {first}");
        }
    }

    /// 長さガードは「1–3999 の正準表記の最長 = 15 文字（3888 = `MMMDCCCLXXXVIII`）」に合わせる。
    /// 9 文字で切ると正準表記 736 通りを弾く。表番号の実用域（I–XL は 7 文字以内）では実害が
    /// 出ないが、算術の都合で置いた定数が受理集合の定義に化けるのを避ける。
    #[test]
    fn canonical_roman_longer_than_nine_chars_is_accepted() {
        let p = build_page(&[
            ("some earlier body sentence appears here", 10.0, 0.0),
            ("and it continues onto a second line", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
            ("TABLE DCCCLXXXVIII. Coefficients of the fit", 10.0, H),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[1].kind, NodeKind::TableCaption);
        assert_eq!(blocks[1].caption_number.as_deref(), Some("DCCCLXXXVIII"));
    }

    /// 算用数字の経路は従来どおり（全大文字ラベルでも変わらない）。加えて、ラベル直後 6 文字に
    /// たまたま数字がある場合でも番号はローマ数字から埋める。
    #[test]
    fn arabic_caption_path_is_unchanged_and_roman_fills_the_number() {
        let p = build_page(&[
            ("some earlier body sentence appears here", 10.0, 0.0),
            ("and it continues onto a second line", 10.0, G),
            ("and a third line to anchor the median", 10.0, G),
            ("FIG. 4. The apparatus used in the experiment", 10.0, H),
            ("TABLE II. 3 configurations of the lattice", 10.0, H),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[1].kind, NodeKind::FigureCaption);
        assert_eq!(blocks[1].caption_number.as_deref(), Some("4"));
        // "II. 3" はラベル直後 6 文字に数字があるので従来経路で caption にはなるが、
        // 番号は算用数字として読めない。ローマ数字で埋める。
        assert_eq!(blocks[2].kind, NodeKind::TableCaption);
        assert_eq!(blocks[2].caption_number.as_deref(), Some("II"));
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

    /// **空入力のページは `state` に触れない**（ゲート ②a の変異 S7）。
    ///
    /// これは doc コメントが明記している不変量で、しかも 8d-2 の 2 pass 同値性が丸ごと
    /// これに乗っている ── 抽出側はテキスト抽出に失敗したページで `recognize_blocks` を
    /// **呼ばずに**次へ進み、build 側は `blocks` が空のまま**呼ぶ**。空入力で state が
    /// 動くと、テキスト抽出に失敗したページを 1 枚挟んだだけで両者の分類が食い違う。
    /// 変異（空の早期 return で state をリセット）は 1,051 本すべて緑のまま通っていた。
    #[test]
    fn an_empty_page_does_not_reset_the_recognizer_state() {
        // 行間の中央値で段落を割るので、2 行だけだと 1 ブロックに畳まれて
        // "References" が見出しにならない。既存テストと同じ 4 行の形にする。
        let refs = build_page(&[
            ("References", 12.0, 0.0),
            ("1. Smith, J. and Doe, A. Foo Bar. 2020", 10.0, H),
            ("2. Lee, C. and Kim, D. Baz Qux. 2021", 10.0, G),
            ("3. Park, E. Quux Corge Grault. 2022", 10.0, G),
        ]);
        let after = build_page(&[("4. Adams, F. Garply Waldo. 2023", 10.0, 0.0)]);

        // (a) 空ページを挟んで回す（= build 側の pass）。
        let mut with_empty = RecognizerState::new();
        assert_eq!(recognize_page(&refs, &mut with_empty)[0].kind, NodeKind::Heading);
        let empty = page(Vec::new());
        assert!(recognize_page(&empty, &mut with_empty).is_empty());
        let a = recognize_page(&after, &mut with_empty);

        // (b) 空ページを飛ばして回す（= 抽出側の pass）。
        let mut skipping = RecognizerState::new();
        recognize_page(&refs, &mut skipping);
        let b = recognize_page(&after, &mut skipping);

        assert_eq!(
            a[0].kind,
            NodeKind::BibliographyEntry,
            "空ページで参考文献モードが失われている（失われると番号付き節に誤検出される）"
        );
        assert_eq!(
            a.iter().map(|x| x.kind).collect::<Vec<_>>(),
            b.iter().map(|x| x.kind).collect::<Vec<_>>(),
            "空ページを挟むかどうかで分類が変わってはいけない"
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

    /// 実データ vid 149（att6）の形。`CropBox ∩ MediaBox` の原点 (77.811, 87.931)・
    /// 寸法 439.455 x 666.283pt。box の上端は 87.931 + 666.283 = 754.214。
    fn nonzero_box_page(segs: Vec<ExtractedBlock>) -> ExtractedPage {
        page_with_box(segs, 77.811, 87.931, 439.455, 666.283)
    }

    #[test]
    fn margin_band_is_measured_from_the_page_box_origin() {
        // 絶対 y=640 は box の中では上から 15% ほどの**本文域**（上端帯は 87.931+599.65=687.58 超）。
        // 原点を落とすと「上端 90%（=599.65）超」と誤判定して段落を unknown_block に降格する。
        let p = nonzero_box_page(vec![seg(
            "This holds for all n",
            100.0,
            640.0,
            200.0,
            10.0,
            0,
        )]);
        assert_eq!(recognize(&p)[0].kind, NodeKind::Paragraph);
    }

    #[test]
    fn running_footer_just_above_the_box_bottom_is_demoted() {
        // 逆向きの実害: box の下端 87.931 のすぐ上（絶対 y=100）にある短い 1 行は本物の走り柱。
        // 原点を落とすと下端帯（=66.63pt 未満）に届かず、段落として残ってしまう。
        let p = nonzero_box_page(vec![seg("104 A. Suzuki", 100.0, 100.0, 120.0, 10.0, 0)]);
        assert_eq!(recognize(&p)[0].kind, NodeKind::UnknownBlock);
    }

    #[test]
    fn margin_band_handles_a_negative_box_origin() {
        // 実データ vid 183（att41）の形: CropBox `[-4.1494 -8.41611 480.534 688.849]` の頁があり
        // box 原点は負。原点を 0 で下限クリップすると帯が上へずれ、走り柱を取り逃がす。
        // 上端帯は -8.416 + 697.265*0.90 = 619.12pt 超（クリップすると 627.54pt 超になる）。
        let head = page_with_box(
            vec![seg("104 A. Suzuki", 100.0, 623.0, 120.0, 10.0, 0)],
            -4.149,
            -8.416,
            484.683,
            697.265,
        );
        assert_eq!(recognize(&head)[0].kind, NodeKind::UnknownBlock);
        // 下端帯（-8.416 + 69.73 = 61.31pt 未満）も同様に box 原点から測る。
        let foot = page_with_box(
            vec![seg("Random walks", 100.0, 0.0, 120.0, 10.0, 0)],
            -4.149,
            -8.416,
            484.683,
            697.265,
        );
        assert_eq!(recognize(&foot)[0].kind, NodeKind::UnknownBlock);
    }

    #[test]
    fn block_straddling_the_band_edge_is_not_demoted() {
        // 帯は「ブロックが丸ごと帯の中にある」ことを要求する ── 上端帯は**下端** `y` が、
        // 下端帯は**上端** `y+h` が境界を越えていること。跨いでいるだけの行は走り柱ではない。
        // 原点ゼロ・842pt のページなので上端帯は 757.8pt 超、下端帯は 84.2pt 未満。
        let straddle_top = page(vec![seg("104 A. Suzuki", 72.0, 750.0, 120.0, 20.0, 0)]);
        assert_eq!(recognize(&straddle_top)[0].kind, NodeKind::Paragraph);
        let inside_top = page(vec![seg("104 A. Suzuki", 72.0, 760.0, 120.0, 20.0, 0)]);
        assert_eq!(recognize(&inside_top)[0].kind, NodeKind::UnknownBlock);

        let straddle_bottom = page(vec![seg("104 A. Suzuki", 72.0, 80.0, 120.0, 20.0, 0)]);
        assert_eq!(recognize(&straddle_bottom)[0].kind, NodeKind::Paragraph);
        let inside_bottom = page(vec![seg("104 A. Suzuki", 72.0, 60.0, 120.0, 20.0, 0)]);
        assert_eq!(recognize(&inside_bottom)[0].kind, NodeKind::UnknownBlock);
    }

    #[test]
    fn long_line_in_the_margin_band_stays_a_paragraph() {
        // 帯の中でも 8 語を超える 1 行は走り柱ではない（既存ガードの固定）。
        let p = page(vec![seg(
            "The quick brown fox jumps over the lazy dog today",
            72.0,
            795.0,
            300.0,
            10.0,
            0,
        )]);
        assert_eq!(recognize(&p)[0].kind, NodeKind::Paragraph);
    }

    #[test]
    fn multi_line_block_in_the_margin_band_stays_a_paragraph() {
        // 帯の中でも 2 行以上のブロックは走り柱ではない（既存ガードの固定）。
        let p = page(vec![
            seg("short text here", 72.0, 795.0, 200.0, 10.0, 0),
            seg("second line here", 72.0, 783.0, 200.0, 10.0, 1),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks.len(), 1, "2 行が 1 ブロックに束ねられる前提: {blocks:?}");
        assert_eq!(blocks[0].kind, NodeKind::Paragraph);
    }

    #[test]
    fn margin_band_is_unchanged_on_a_zero_origin_page() {
        // 原点ゼロのページ（大半の PDF）では従来と同じ判定であること。
        let top = page(vec![seg("104 A. Suzuki", 72.0, 795.0, 120.0, 10.0, 0)]);
        assert_eq!(recognize(&top)[0].kind, NodeKind::UnknownBlock);
        let body = page(vec![seg("This holds for all n", 72.0, 400.0, 200.0, 10.0, 0)]);
        assert_eq!(recognize(&body)[0].kind, NodeKind::Paragraph);
    }

    #[test]
    fn detect_display_math_catches_symbol_heavy_lines() {
        // 演算子・集合記号が複数 → 数式。
        assert!(detect_display_math("−ik·x ψ(x), ψ ∈ H", 1).is_some());
        assert!(detect_display_math("|x| ψ(x) 2C2 < ∞", 1).is_some());
        // 記号は少ないが英字が半分未満 → 数式。
        assert!(detect_display_math("U − t U 0tU 0 = S2 C2", 1).is_some());
    }

    #[test]
    fn detect_display_math_rejects_prose_and_symbolless() {
        // 散文（英字 7 割以上）は記号が混じっても数式にしない。
        assert!(detect_display_math("The value of x = y holds in this case", 1).is_none());
        // 演算子が飛んで英字だけになった式は拾えない（欠損を許容）。
        assert!(detect_display_math("λj (k)t U", 1).is_none());
        // 長すぎるブロック（4 行以上）は display 数式ではない。
        assert!(detect_display_math("a = b ∈ C", 4).is_none());
    }

    #[test]
    fn equation_label_only_matches_real_numbers() {
        assert_eq!(extract_equation_label("U = S2 C2 (2.1)"), Some("(2.1)".to_string()));
        assert_eq!(extract_equation_label("x + y (12)"), Some("(12)".to_string()));
        assert_eq!(extract_equation_label("f(x) (A.1)"), Some("(A.1)".to_string()));
        // 式の一部の丸括弧は数式番号ではない。
        assert_eq!(extract_equation_label("U − t U ac(U0U0)"), None);
        assert_eq!(extract_equation_label("g(x, y)"), None);
    }

    #[test]
    fn display_math_block_is_recognized_with_label() {
        let p = build_page(&[
            ("some body text before the equation here", 10.0, 0.0),
            ("more body text on the second line now", 10.0, G),
            ("and a third body line to anchor median", 10.0, G),
            ("U − t U 0 = S2 C2 S1 C1 (2.1)", 10.0, H),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Paragraph);
        assert_eq!(blocks[1].kind, NodeKind::DisplayMath);
        assert_eq!(blocks[1].equation_label.as_deref(), Some("(2.1)"));
        assert!(blocks[1].confidence >= 0.5);
    }

    #[test]
    fn detect_theorem_matches_headers_and_rejects_references() {
        // キーワード + 番号 + 終端記号 → 定理系（number/note を取り出す）。
        let t = detect_theorem("Theorem 2.3. Let X be a compact set").unwrap();
        assert_eq!(t.kind, NodeKind::Theorem);
        assert_eq!(t.number.as_deref(), Some("2.3"));
        assert!(t.confidence >= 0.7);
        // 付記名（丸括弧）を取り出す。
        let t = detect_theorem("Theorem 1 (Zorn's Lemma). Every poset ...").unwrap();
        assert_eq!(t.number.as_deref(), Some("1"));
        assert_eq!(t.note.as_deref(), Some("Zorn's Lemma"));
        // 付録番号 "A.1"。
        assert_eq!(
            detect_theorem("Lemma A.1. The map is continuous").unwrap().kind,
            NodeKind::Lemma
        );
        // proof は番号なしでも "." / "of" で認識。
        assert_eq!(detect_theorem("Proof.").unwrap().kind, NodeKind::Proof);
        assert_eq!(
            detect_theorem("Proof of Theorem 2.3. We first ...").unwrap().kind,
            NodeKind::Proof
        );
        // 参照文中の "Theorem 2 shows …" は終端記号が続かないので棄却（誤検出より欠損）。
        assert!(detect_theorem("Theorem 2 shows that the bound holds").is_none());
        // 定理でない散文の行頭語。
        assert!(detect_theorem("Example usage is shown in the appendix").is_none());
        assert!(detect_theorem("Remarkably, the result also holds here").is_none());
        // 文中の "(x, y)" を note と誤認しない（終端記号が括弧より前）。
        let t = detect_theorem("Definition 4.1. Let (x, y) denote a pair").unwrap();
        assert_eq!(t.kind, NodeKind::Definition);
        assert_eq!(t.note, None);
    }

    #[test]
    fn theorem_block_is_classified_with_number_payload() {
        let p = build_page(&[
            ("some body sentence to anchor the median here", 10.0, 0.0),
            ("and a second body line of running prose now", 10.0, G),
            ("and a third body line to anchor the median", 10.0, G),
            ("Theorem 2.3. Every bounded sequence has a limit", 10.0, H),
            ("Proof. Consider the monotone subsequence and", 10.0, H),
        ]);
        let blocks = recognize(&p);
        let thm = blocks.iter().find(|b| b.kind == NodeKind::Theorem).unwrap();
        assert_eq!(thm.theorem_number.as_deref(), Some("2.3"));
        assert!(thm.confidence >= 0.7);
        assert!(blocks.iter().any(|b| b.kind == NodeKind::Proof));
    }

    #[test]
    fn theorem_keyword_in_bibliography_mode_is_not_detected() {
        // 参考文献モードでは定理検出を行わない: "Theorem 1. …" で始まる行も書誌項目にする
        // （本文モードなら Theorem になる行を、mode ガードが抑える対比）。
        let p = build_page(&[
            ("References", 12.0, 0.0),
            ("Theorem 1. A. Author, A Title. Journal 2020", 10.0, H),
            ("Lemma 2. B. Author, B Title. Journal 2021", 10.0, G),
            ("Proof 3. C. Author, C Title. Journal 2022", 10.0, G),
        ]);
        let blocks = recognize(&p);
        assert_eq!(blocks[0].kind, NodeKind::Heading); // "References"
        assert_eq!(blocks[1].kind, NodeKind::BibliographyEntry);
        assert!(!blocks
            .iter()
            .any(|b| matches!(b.kind, NodeKind::Theorem | NodeKind::Lemma | NodeKind::Proof)));
    }

    /// debt-22: page の `plain_text` から C0 制御文字を落とす。**改行は保つ**。
    #[test]
    fn clean_page_text_drops_c0_but_keeps_line_structure() {
        // pdfium がマップできない数式グリフを語の内側に吐く形（実データ）。
        // trigram 索引ではここで語が割れて検索から落ちる。
        assert_eq!(clean_page_text("consis\u{2}tent"), "consistent");
        assert_eq!(clean_page_text("ψ(x)\u{2} = 2C2\u{15}"), "ψ(x) = 2C2");
        // 改行とタブは残す（`normalize_ws` との違いがここ）。
        assert_eq!(
            clean_page_text("first line\nsecond\tcolumn\n\nthird"),
            "first line\nsecond\tcolumn\n\nthird"
        );
        // 改行コードの揺れは \n に寄せる（\r\n が 2 個の改行にならないこと）。
        assert_eq!(clean_page_text("a\r\nb\rc\nd"), "a\nb\nc\nd");
        // 垂直タブ・改ページも C0 なので落ちる。
        assert_eq!(clean_page_text("a\u{b}b\u{c}c"), "abc");
        assert_eq!(clean_page_text("a\u{0}b"), "ab");
        // **DEL と C1 は落とさない**（debt-22 の実測が C0 のみを数えているので母集団を揃える）。
        assert_eq!(clean_page_text("a\u{7f}b\u{85}c"), "a\u{7f}b\u{85}c");
        // 制御文字だけのページは空になる（呼び出し側が `None` にする）。
        assert!(clean_page_text("\u{2}\u{15}\u{c}").is_empty());
        // 変化の無い入力は素通り。
        assert_eq!(clean_page_text("plain ascii text"), "plain ascii text");
    }

    /// debt-22 の実データ計測（手動）。実 DB から吸い出した page の `plain_text` に
    /// **本番の [`clean_page_text`] をそのまま**流し、効果と副作用を数える
    /// （プローブ側に第 2 の実装を作らない）。
    ///
    /// 入力は 1 行 1 ページの hex ダンプ:
    /// ```sh
    /// sqlite3 "file:$DB?mode=ro" "SELECT hex(CAST(dn.plain_text AS BLOB))
    ///   FROM document_nodes dn JOIN document_versions dv ON dv.id = dn.document_version_id
    ///   WHERE dv.extractor_name='lumencite-pdfium'
    ///     AND dv.extraction_status IN ('completed','completed_with_warnings')
    ///     AND dn.node_kind='page' AND TRIM(dn.plain_text) != '';" > pages.hex
    /// LCIR_PAGE_HEX=pages.hex cargo test --lib clean_page_text_corpus -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "manual measurement; needs LCIR_PAGE_HEX dumped from the live DB"]
    fn clean_page_text_corpus() {
        let Ok(path) = std::env::var("LCIR_PAGE_HEX") else {
            eprintln!("skip: set LCIR_PAGE_HEX=/path/to/pages.hex");
            return;
        };
        eprintln!("C0_BEGIN"); // libtest は 1 行目を食う
        // **債務の定義に揃える**: C0 = U+0000..U+001F から `\t` `\n` `\r` を除いたもの。
        // `\r` を混ぜると「汚染率」が 5,786 / 5,803 に見えて 78.8% という実測とずれる
        // （`\r` は改行コードの揺れであって、語を割る汚染とは別の話）。
        let strict_c0 = |c: char| (c as u32) < 0x20 && c != '\n' && c != '\t' && c != '\r';
        let has_c0 = |s: &str| s.chars().any(&strict_c0);
        let (mut pages, mut before, mut after, mut changed, mut cr, mut emptied) =
            (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
        let (mut removed_c0, mut removed_cr, mut examples) = (0usize, 0usize, Vec::new());
        for line in std::fs::read_to_string(&path).expect("hex dump").lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let bytes: Vec<u8> = (0..line.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&line[i..i + 2], 16).expect("hex"))
                .collect();
            let src = String::from_utf8_lossy(&bytes).into_owned();
            pages += 1;
            // `\r` は「揺れ」なので C0 の汚染率とは別に数える（実測 5,786 / 5,803 ページ）。
            if src.contains('\r') {
                cr += 1;
            }
            if has_c0(&src) {
                before += 1;
            }
            let out = clean_page_text(&src);
            if has_c0(&out) {
                after += 1;
            }
            removed_c0 += src.chars().filter(|c| strict_c0(*c)).count();
            removed_cr += src.chars().filter(|c| *c == '\r').count();
            if out != src {
                changed += 1;
                if examples.len() < 3 {
                    let at = src.char_indices().find(|(_, c)| strict_c0(*c)).map(|(i, _)| i);
                    if let Some(i) = at {
                        let lo = src[..i].char_indices().rev().nth(20).map_or(0, |(j, _)| j);
                        let hi = src[i..].char_indices().nth(20).map_or(src.len(), |(j, _)| i + j);
                        examples.push(format!("{:?}", &src[lo..hi]));
                    }
                }
            }
            if out.trim().is_empty() && !src.trim().is_empty() {
                emptied += 1;
            }
        }
        eprintln!(
            "C0\tpages={pages}\tc0_before={before}\tc0_after={after}\tchanged={changed}\t\
             cr_pages={cr}\tremoved_c0={removed_c0}\tremoved_cr={removed_cr}\temptied={emptied}"
        );
        for e in &examples {
            eprintln!("C0_SAMPLE\t{e}");
        }
        assert_eq!(after, 0, "掃除の後に C0 が残っている");
    }

    /// `normalize_ws` を page に流用してはいけない理由を固定する（改行が潰れる）。
    #[test]
    fn normalize_ws_would_flatten_a_page_but_clean_page_text_does_not() {
        let page = "Introduction\n\nWe study quantum walks.";
        assert_eq!(normalize_ws(page), "Introduction We study quantum walks.");
        assert_eq!(clean_page_text(page), page);
    }

    #[test]
    fn normalize_ws_strips_control_glyphs() {
        // pdfium が吐く制御文字（\u{2} 等）は落として空白正規化する。
        let p = page(vec![seg("ψ(x)\u{2} = 2C2\u{15}", 72.0, 400.0, 120.0, 10.0, 0)]);
        let lines = group_lines(&p.blocks);
        assert_eq!(lines[0].text, "ψ(x) = 2C2");
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
