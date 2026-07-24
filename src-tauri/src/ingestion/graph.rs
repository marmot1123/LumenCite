//! 参照グラフの抽出（Phase 6a・DB 非依存の純関数）。
//!
//! Phase 5 までに永続化されたノードの軽量ビュー（`GraphNode`）から、ノード間の参照
//! （`node_relations`）を解決する。原資料に生で残る参照を突き合わせる:
//!
//! - **TeX**（`RefStrategy::Tex`・origin=tex_source・高信頼 0.9）: 段落等の `plain_text` に原文の
//!   まま残る `\ref`/`\eqref`/`\cite` を、`\label` 名（payload の labels）/ `\bibitem` の cite key
//!   と照合する。
//! - **PDF**（`RefStrategy::Pdf`・origin=layout_model・中信頼 0.6）: `plain_text` 中の "Theorem 2.3"
//!   / "Eq. (2.1)" を定理番号 / 数式番号と照合する（PDF は `\label` を復元できないため番号一致）。
//! - **proof → theorem**（`proves`）: PDF は "Proof of Theorem 2.3" の番号一致、無ければ読み順で
//!   直前の定理系ノード。TeX は `\ref` 先が定理系ならそれ、無ければ読み順の直前。
//!
//! **誤検出より欠損**（roadmap §16）: 解決できない参照（ターゲット不在・曖昧）は辺を張らない。
//! 自己参照（定理見出し "Theorem 2.3." がその定理自身を指す等）も張らない。

use crate::document_ir::{NodeKind, Origin, RelationType};
use std::collections::{HashMap, HashSet};

/// 参照解決に必要なノードの軽量ビュー（DB 非依存）。PDF は block ノード / TeX は block ノードから
/// 作り、骨格（document/page/line）は含めない。
#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: i64,
    pub kind: NodeKind,
    /// 読み順の単調増加インデックス（PDF は (page, block) を平坦化した通し番号 / TeX は block 順）。
    /// proof → 直前の定理の解決に使う。
    pub reading_index: i64,
    pub plain_text: String,
    /// `\label{..}` の対象名（TeX のみ・payload.labels）。参照解決のターゲット。
    pub labels: Vec<String>,
    /// 数式番号 "(2.1)"（display_math のみ・PDF/TeX）。PDF の "Eq. (2.1)" 参照のターゲット。
    pub equation_label: Option<String>,
    /// 定理番号 "2.3"（定理系・PDF のみ）。PDF の "Theorem 2.3" 参照のターゲット。
    pub theorem_number: Option<String>,
    /// `\bibitem{key}` の cite key（bibliography_entry・TeX のみ）。`\cite` のターゲット。
    pub cite_key: Option<String>,
}

/// 解決した参照辺（`node_relations` に 1 行になる）。
#[derive(Debug, Clone, PartialEq)]
pub struct RelationEdge {
    pub from_node_id: i64,
    pub relation_type: RelationType,
    pub to_node_id: i64,
    pub confidence: f64,
    pub origin: Origin,
    pub metadata_json: Option<String>,
}

/// 参照の見つけ方（原資料の種類で変わる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefStrategy {
    /// TeX: 本文の `\ref`/`\eqref`/`\cite` を label/cite_key と突き合わせる。
    Tex,
    /// PDF: 本文の "Theorem 2.3"/"Eq. (2.1)" を定理番号/数式番号と突き合わせる。
    Pdf,
}

// TeX の参照系マクロ。`\eqref` は数式専用なので分ける。
const EQ_REF_MACROS: &[&str] = &["eqref"];
const GENERIC_REF_MACROS: &[&str] = &[
    "ref", "autoref", "cref", "Cref", "vref", "Vref", "labelcref", "nameref",
];
const CITE_MACROS: &[&str] = &[
    "cite",
    "citep",
    "citet",
    "citealp",
    "citealt",
    "citeauthor",
    "citeyear",
    "citenum",
    "Citep",
    "Citet",
    "Citealp",
    "Citealt",
    "parencite",
    "Parencite",
    "textcite",
    "Textcite",
    "autocite",
    "Autocite",
    "footcite",
    "smartcite",
];

// PDF 本文の参照キーワード（大文字始まりのみ・plural は保守的に拾わない）。
const PDF_THM_KEYWORDS: &[&str] = &[
    "Theorem",
    "Lemma",
    "Proposition",
    "Corollary",
    "Definition",
    "Remark",
    "Example",
];
// 数式参照は "Equation" を "Eq" より先に（前方一致の取りこぼしを防ぐ）。
const PDF_EQ_KEYWORDS: &[&str] = &["Equation", "Eqs", "Eq"];

const TEX_CONF: f64 = 0.9;
const PDF_REF_CONF: f64 = 0.6;
const PDF_PROVES_NUM_CONF: f64 = 0.7;

/// ノード集合からノード間参照辺を解決する。
pub fn resolve_relations(nodes: &[GraphNode], strategy: RefStrategy) -> Vec<RelationEdge> {
    // ── ターゲット索引 ──
    let mut label_to_node: HashMap<&str, (i64, NodeKind)> = HashMap::new();
    // PDF 定理番号は種別ごとに採番されうる（同じ論文に Theorem 3.1 と Definition 3.1 が併存する）。
    // 参照キーワード（"Definition 3.1"）の種別も鍵に含め、番号だけの誤照合を避ける。
    let mut theorem_num_to_node: HashMap<(&str, &str), i64> = HashMap::new();
    let mut equation_num_to_node: HashMap<String, i64> = HashMap::new();
    let mut cite_to_node: HashMap<&str, i64> = HashMap::new();
    for n in nodes {
        for l in &n.labels {
            label_to_node.entry(l.as_str()).or_insert((n.id, n.kind));
        }
        if is_theorem_family(n.kind) {
            if let Some(num) = &n.theorem_number {
                theorem_num_to_node
                    .entry((n.kind.as_str(), num.as_str()))
                    .or_insert(n.id);
            }
        }
        if let Some(lbl) = &n.equation_label {
            if let Some(k) = equation_key(lbl) {
                equation_num_to_node.entry(k).or_insert(n.id);
            }
        }
        if n.kind == NodeKind::BibliographyEntry {
            if let Some(ck) = &n.cite_key {
                cite_to_node.entry(ck.as_str()).or_insert(n.id);
            }
        }
    }

    // proof → 直前の定理系ノードの解決用（読み順で昇順）。
    let mut thm_in_order: Vec<(i64, i64)> = nodes
        .iter()
        .filter(|n| is_theorem_family(n.kind))
        .map(|n| (n.reading_index, n.id))
        .collect();
    thm_in_order.sort_by_key(|&(ri, _)| ri);

    let mut edges: Vec<RelationEdge> = Vec::new();
    let mut seen: HashSet<(i64, &'static str, i64)> = HashSet::new();

    for n in nodes {
        match strategy {
            RefStrategy::Tex => {
                // \eqref{..} → 数式（label 経由）。
                for (name, arg) in find_macros(&n.plain_text, EQ_REF_MACROS) {
                    for key in split_keys(&arg) {
                        if let Some(&(to, _kind)) = label_to_node.get(key.as_str()) {
                            push_edge(
                                &mut edges,
                                &mut seen,
                                n.id,
                                RelationType::RefersToEquation,
                                to,
                                TEX_CONF,
                                Origin::TexSource,
                                meta_ref(&name, &key),
                            );
                        }
                    }
                }
                // \ref/\autoref/\cref/.. → 参照先の種別に応じて張り分け。
                for (name, arg) in find_macros(&n.plain_text, GENERIC_REF_MACROS) {
                    for key in split_keys(&arg) {
                        if let Some(&(to, kind)) = label_to_node.get(key.as_str()) {
                            push_edge(
                                &mut edges,
                                &mut seen,
                                n.id,
                                relation_type_for_target(kind),
                                to,
                                TEX_CONF,
                                Origin::TexSource,
                                meta_ref(&name, &key),
                            );
                        }
                    }
                }
                // \cite/.. → 参考文献エントリ。
                for (name, arg) in find_macros(&n.plain_text, CITE_MACROS) {
                    for key in split_keys(&arg) {
                        if let Some(&to) = cite_to_node.get(key.as_str()) {
                            push_edge(
                                &mut edges,
                                &mut seen,
                                n.id,
                                RelationType::Cites,
                                to,
                                TEX_CONF,
                                Origin::TexSource,
                                meta_cite(&name, &key),
                            );
                        }
                    }
                }
            }
            RefStrategy::Pdf => {
                for pref in find_pdf_refs(&n.plain_text) {
                    match pref.category {
                        RefCategory::Theorem => {
                            // 参照キーワード（"Definition"）の種別で対象を絞る（番号衝突回避）。
                            if let Some(kind) = keyword_to_kind(&pref.keyword) {
                                if let Some(&to) =
                                    theorem_num_to_node.get(&(kind.as_str(), pref.number.as_str()))
                                {
                                    push_edge(
                                        &mut edges,
                                        &mut seen,
                                        n.id,
                                        RelationType::RefersToTheorem,
                                        to,
                                        PDF_REF_CONF,
                                        Origin::LayoutModel,
                                        meta_pdf(&pref.raw, &pref.number),
                                    );
                                }
                            }
                        }
                        RefCategory::Equation => {
                            if let Some(&to) = equation_num_to_node.get(&pref.number) {
                                push_edge(
                                    &mut edges,
                                    &mut seen,
                                    n.id,
                                    RelationType::RefersToEquation,
                                    to,
                                    PDF_REF_CONF,
                                    Origin::LayoutModel,
                                    meta_pdf(&pref.raw, &pref.number),
                                );
                            }
                        }
                    }
                }
            }
        }

        // proof → theorem（proves）。
        if n.kind == NodeKind::Proof {
            resolve_proves(
                &mut edges,
                &mut seen,
                n,
                strategy,
                &label_to_node,
                &theorem_num_to_node,
                &thm_in_order,
            );
        }
    }

    edges
}

/// proof ノード n の証明対象（proves）を 1 本張る。
#[allow(clippy::too_many_arguments)]
fn resolve_proves(
    edges: &mut Vec<RelationEdge>,
    seen: &mut HashSet<(i64, &'static str, i64)>,
    n: &GraphNode,
    strategy: RefStrategy,
    label_to_node: &HashMap<&str, (i64, NodeKind)>,
    theorem_num_to_node: &HashMap<(&str, &str), i64>,
    thm_in_order: &[(i64, i64)],
) {
    // 1) 明示的な参照優先。
    let explicit: Option<(i64, f64, Option<String>)> = match strategy {
        // PDF: "Proof of Theorem 2.3" の種別 + 番号一致。
        RefStrategy::Pdf => proof_of_number(&n.plain_text).and_then(|(kw, num)| {
            let kind = keyword_to_kind(&kw)?;
            theorem_num_to_node
                .get(&(kind.as_str(), num.as_str()))
                .map(|&to| (to, PDF_PROVES_NUM_CONF, meta_proves_number(&num)))
        }),
        // TeX: 本文の \ref 先が定理系ならそれ（"Proof of Theorem~\ref{thm:x}"）。
        RefStrategy::Tex => {
            let mut found = None;
            'outer: for (_, arg) in find_macros(&n.plain_text, &all_ref_macros()) {
                for key in split_keys(&arg) {
                    if let Some(&(to, kind)) = label_to_node.get(key.as_str()) {
                        if is_theorem_family(kind) {
                            found = Some((to, TEX_CONF, meta_proves_ref(&key)));
                            break 'outer;
                        }
                    }
                }
            }
            found
        }
    };

    // 2) 無ければ読み順で直前の定理系ノード（昇順なので後ろから最初に見つかるものが直前）。
    let resolved = explicit.or_else(|| {
        thm_in_order
            .iter()
            .rfind(|&&(ri, _)| ri < n.reading_index)
            .map(|&(_, to)| {
                let conf = if strategy == RefStrategy::Tex {
                    TEX_CONF
                } else {
                    PDF_REF_CONF
                };
                (to, conf, meta_proves_adjacency())
            })
    });

    if let Some((to, conf, meta)) = resolved {
        let origin = match strategy {
            RefStrategy::Tex => Origin::TexSource,
            RefStrategy::Pdf => Origin::LayoutModel,
        };
        push_edge(edges, seen, n.id, RelationType::Proves, to, conf, origin, meta);
    }
}

#[allow(clippy::too_many_arguments)]
fn push_edge(
    edges: &mut Vec<RelationEdge>,
    seen: &mut HashSet<(i64, &'static str, i64)>,
    from: i64,
    relation_type: RelationType,
    to: i64,
    confidence: f64,
    origin: Origin,
    metadata_json: Option<String>,
) {
    if from == to {
        return; // 自己参照は張らない（定理見出しが自分自身を指す等）。
    }
    if !seen.insert((from, relation_type.as_str(), to)) {
        return; // (from, type, to) の重複は 1 本に。
    }
    edges.push(RelationEdge {
        from_node_id: from,
        relation_type,
        to_node_id: to,
        confidence,
        origin,
        metadata_json,
    });
}

fn is_theorem_family(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Theorem
            | NodeKind::Lemma
            | NodeKind::Proposition
            | NodeKind::Corollary
            | NodeKind::Definition
            | NodeKind::Remark
            | NodeKind::Example
    )
}

/// PDF 参照キーワード（"Theorem" 等）→ 対象ノード種別。番号照合を種別ごとに絞るのに使う。
fn keyword_to_kind(keyword: &str) -> Option<NodeKind> {
    match keyword {
        "Theorem" => Some(NodeKind::Theorem),
        "Lemma" => Some(NodeKind::Lemma),
        "Proposition" => Some(NodeKind::Proposition),
        "Corollary" => Some(NodeKind::Corollary),
        "Definition" => Some(NodeKind::Definition),
        "Remark" => Some(NodeKind::Remark),
        "Example" => Some(NodeKind::Example),
        _ => None,
    }
}

/// `\ref` 先ノードの種別 → relation_type。
fn relation_type_for_target(kind: NodeKind) -> RelationType {
    match kind {
        NodeKind::DisplayMath | NodeKind::InlineMath | NodeKind::EquationGroup => {
            RelationType::RefersToEquation
        }
        NodeKind::FigureCaption => RelationType::RefersToFigure,
        // Table = Phase 8b の table ノード（caption なし環境では \label が table 側に付く）。
        NodeKind::TableCaption | NodeKind::Table => RelationType::RefersToTable,
        k if is_theorem_family(k) => RelationType::RefersToTheorem,
        NodeKind::Section | NodeKind::Subsection => RelationType::RefersToSection,
        _ => RelationType::RefersTo,
    }
}

fn all_ref_macros() -> Vec<&'static str> {
    let mut v = Vec::new();
    v.extend_from_slice(EQ_REF_MACROS);
    v.extend_from_slice(GENERIC_REF_MACROS);
    v
}

// ── metadata_json 生成 ──

fn meta_ref(macro_name: &str, key: &str) -> Option<String> {
    Some(serde_json::json!({ "ref": format!("\\{macro_name}{{{key}}}"), "key": key }).to_string())
}
fn meta_cite(macro_name: &str, key: &str) -> Option<String> {
    Some(
        serde_json::json!({ "cite": format!("\\{macro_name}{{{key}}}"), "cite_key": key })
            .to_string(),
    )
}
fn meta_pdf(raw: &str, number: &str) -> Option<String> {
    Some(serde_json::json!({ "ref": raw, "number": number }).to_string())
}
fn meta_proves_number(number: &str) -> Option<String> {
    Some(serde_json::json!({ "by": "number", "number": number }).to_string())
}
fn meta_proves_ref(key: &str) -> Option<String> {
    Some(serde_json::json!({ "by": "ref", "key": key }).to_string())
}
fn meta_proves_adjacency() -> Option<String> {
    Some(serde_json::json!({ "by": "adjacency" }).to_string())
}

// ── TeX マクロ字句スキャン（plain_text は `\ref{..}` 等を原文のまま持つ・Phase 4） ──

/// `\<name>{arg}`（任意の `*`・`[..]` 光学引数を読み飛ばす）を集め、(name, arg) を返す。
fn find_macros(text: &str, names: &[&str]) -> Vec<(String, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' {
            i += 1;
            continue;
        }
        let name_start = i + 1;
        let mut j = name_start;
        while j < b.len() && b[j].is_ascii_alphabetic() {
            j += 1;
        }
        if j == name_start {
            // 制御記号（\{, \%, \\ など）。1 文字読み飛ばす。
            i = name_start + 1;
            continue;
        }
        let name = &text[name_start..j];
        if !names.contains(&name) {
            i = j;
            continue;
        }
        // `*` と光学引数 [..] を読み飛ばす。
        let mut k = j;
        if k < b.len() && b[k] == b'*' {
            k += 1;
        }
        loop {
            let ws = skip_spaces(b, k);
            if ws < b.len() && b[ws] == b'[' {
                match matching_bracket(b, ws) {
                    Some(end) => {
                        k = end + 1;
                        continue;
                    }
                    None => break,
                }
            }
            k = ws;
            break;
        }
        let k2 = skip_spaces(b, k);
        if k2 < b.len() && b[k2] == b'{' {
            if let Some((content, end)) = read_brace_group(text, k2) {
                out.push((name.to_string(), content));
                i = end;
                continue;
            }
        }
        i = j;
    }
    out
}

fn skip_spaces(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

fn matching_bracket(b: &[u8], open: usize) -> Option<usize> {
    let mut i = open + 1;
    while i < b.len() {
        if b[i] == b']' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// `{` から始まるグループを（ネスト対応で）読み、(中身, 閉じ `}` の次の index) を返す。
fn read_brace_group(text: &str, open: usize) -> Option<(String, usize)> {
    let b = text.as_bytes();
    if b.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((text[open + 1..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_keys(arg: &str) -> Vec<String> {
    arg.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

// ── PDF 本文の参照スキャン（"Theorem 2.3" / "Eq. (2.1)"） ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefCategory {
    Theorem,
    Equation,
}

#[derive(Debug, Clone)]
struct PdfRef {
    category: RefCategory,
    /// 定理系のときの参照キーワード（"Theorem"/"Definition"/…）。数式は空。
    keyword: String,
    number: String,
    raw: String,
}

fn find_pdf_refs(text: &str) -> Vec<PdfRef> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let boundary = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        if !boundary || !b[i].is_ascii_uppercase() {
            i += 1;
            continue;
        }
        // 定理系キーワード + 番号。
        if let Some((kw, after)) = match_keyword(b, i, PDF_THM_KEYWORDS) {
            let ws = skip_spaces(b, after);
            if ws > after {
                if let Some((num, end)) = take_ref_number(text, ws) {
                    out.push(PdfRef {
                        category: RefCategory::Theorem,
                        keyword: kw.to_string(),
                        number: num,
                        raw: text[i..end].to_string(),
                    });
                    i = end;
                    continue;
                }
            }
        }
        // 数式キーワード + （任意の '.' と '('）番号。
        if let Some((_, after)) = match_keyword(b, i, PDF_EQ_KEYWORDS) {
            let mut k = after;
            if b.get(k) == Some(&b'.') {
                k += 1;
            }
            k = skip_spaces(b, k);
            let paren = b.get(k) == Some(&b'(');
            if paren {
                k += 1;
            }
            if let Some((num, end)) = take_ref_number(text, k) {
                let raw_end = if paren && b.get(end) == Some(&b')') {
                    end + 1
                } else {
                    end
                };
                out.push(PdfRef {
                    category: RefCategory::Equation,
                    keyword: String::new(),
                    number: num,
                    raw: text[i..raw_end].to_string(),
                });
                i = raw_end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// text の位置 i でキーワードのどれかが完全語（後続が非英字）で一致するか。
fn match_keyword<'a>(b: &[u8], i: usize, keywords: &[&'a str]) -> Option<(&'a str, usize)> {
    for &kw in keywords {
        let end = i + kw.len();
        if end <= b.len() && &b[i..end] == kw.as_bytes() {
            let trailing_ok = end == b.len() || !b[end].is_ascii_alphabetic();
            if trailing_ok {
                return Some((kw, end));
            }
        }
    }
    None
}

/// 先頭から参照番号（"2" / "2.3" / 付録 "A.1"）を取り、(number, end_index) を返す。
fn take_ref_number(text: &str, i: usize) -> Option<(String, usize)> {
    let b = text.as_bytes();
    if i >= b.len() {
        return None;
    }
    let appendix = i + 2 < b.len()
        && b[i].is_ascii_uppercase()
        && b[i + 1] == b'.'
        && b[i + 2].is_ascii_digit();
    let mut j = i;
    if appendix {
        j = i + 1;
    } else if !b[i].is_ascii_digit() {
        return None;
    }
    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.') {
        j += 1;
    }
    let mut end = j;
    while end > i && b[end - 1] == b'.' {
        end -= 1;
    }
    let num = &text[i..end];
    if num.is_empty() || !num.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((num.to_string(), end))
}

/// 数式番号ラベル "(2.1)" → 内側 "2.1"（PDF 参照との照合キー）。
fn equation_key(label: &str) -> Option<String> {
    let t = label.trim();
    let inner = t
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(t)
        .trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

/// "Proof of Theorem 2.3" の (キーワード, 番号) を取り出す（PDF proof の proves 解決）。
fn proof_of_number(text: &str) -> Option<(String, String)> {
    let t = text.trim_start();
    let b = t.as_bytes();
    if b.len() < 9 || !b[..9].eq_ignore_ascii_case(b"proof of ") {
        return None;
    }
    for pref in find_pdf_refs(&t[9..]) {
        if pref.category == RefCategory::Theorem {
            return Some((pref.keyword, pref.number));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i64, kind: NodeKind, reading_index: i64, text: &str) -> GraphNode {
        GraphNode {
            id,
            kind,
            reading_index,
            plain_text: text.to_string(),
            labels: vec![],
            equation_label: None,
            theorem_number: None,
            cite_key: None,
        }
    }

    fn find(edges: &[RelationEdge], from: i64, ty: RelationType, to: i64) -> Option<&RelationEdge> {
        edges
            .iter()
            .find(|e| e.from_node_id == from && e.relation_type == ty && e.to_node_id == to)
    }

    // ── TeX: \ref / \eqref / \cite ──

    #[test]
    fn tex_ref_resolves_to_labeled_theorem() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem statement.");
        thm.labels = vec!["thm:main".to_string()];
        let para = node(3, NodeKind::Paragraph, 2, "As shown in \\ref{thm:main}, we win.");
        let edges = resolve_relations(&[thm, para], RefStrategy::Tex);
        let e = find(&edges, 3, RelationType::RefersToTheorem, 2).expect("ref→theorem");
        assert_eq!(e.origin, Origin::TexSource);
        assert!((e.confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn tex_eqref_resolves_to_equation() {
        let mut eq = node(2, NodeKind::DisplayMath, 1, "E=mc^2");
        eq.labels = vec!["eq:e".to_string()];
        eq.equation_label = Some("(1)".to_string());
        let para = node(3, NodeKind::Paragraph, 2, "Combine \\eqref{eq:e} with the above.");
        let edges = resolve_relations(&[eq, para], RefStrategy::Tex);
        assert!(find(&edges, 3, RelationType::RefersToEquation, 2).is_some());
    }

    #[test]
    fn tex_ref_to_section_and_figure_use_target_kind() {
        let mut sec = node(2, NodeKind::Section, 1, "Introduction");
        sec.labels = vec!["sec:intro".to_string()];
        let mut fig = node(3, NodeKind::FigureCaption, 2, "Figure 1: architecture");
        fig.labels = vec!["fig:arch".to_string()];
        let para = node(4, NodeKind::Paragraph, 3, "See \\ref{sec:intro} and \\ref{fig:arch}.");
        let edges = resolve_relations(&[sec, fig, para], RefStrategy::Tex);
        assert!(find(&edges, 4, RelationType::RefersToSection, 2).is_some());
        assert!(find(&edges, 4, RelationType::RefersToFigure, 3).is_some());
    }

    #[test]
    fn tex_ref_to_table_node_uses_refers_to_table() {
        // Phase 8b: caption なし環境では \label が table ノード側に付く。
        let mut tab = node(2, NodeKind::Table, 1, "a | b\nc | d");
        tab.labels = vec!["tab:bare".to_string()];
        let para = node(3, NodeKind::Paragraph, 2, "See \\ref{tab:bare}.");
        let edges = resolve_relations(&[tab, para], RefStrategy::Tex);
        assert!(find(&edges, 3, RelationType::RefersToTable, 2).is_some());
    }

    #[test]
    fn tex_cite_inside_table_cell_creates_edge_from_table_node() {
        // Phase 8b: セル内の \cite は table ノードを出典とする cites 辺になる（原文由来）。
        let mut bib = node(2, NodeKind::BibliographyEntry, 1, "A. Author, Paper.");
        bib.cite_key = Some("author2020".to_string());
        let tab = node(3, NodeKind::Table, 2, "method | \\cite{author2020}");
        let edges = resolve_relations(&[bib, tab], RefStrategy::Tex);
        assert!(find(&edges, 3, RelationType::Cites, 2).is_some());
    }

    #[test]
    fn tex_cite_resolves_to_bibitem_and_splits_multiple() {
        let mut b1 = node(2, NodeKind::BibliographyEntry, 10, "Smith 2020");
        b1.cite_key = Some("smith2020".to_string());
        let mut b2 = node(3, NodeKind::BibliographyEntry, 11, "Jones 2019");
        b2.cite_key = Some("jones2019".to_string());
        let para = node(4, NodeKind::Paragraph, 1, "Prior work \\cite{smith2020, jones2019} shows.");
        let edges = resolve_relations(&[b1, b2, para], RefStrategy::Tex);
        assert!(find(&edges, 4, RelationType::Cites, 2).is_some());
        assert!(find(&edges, 4, RelationType::Cites, 3).is_some());
    }

    #[test]
    fn tex_cite_with_optional_arg_and_star_is_parsed() {
        let mut b1 = node(2, NodeKind::BibliographyEntry, 10, "Smith 2020");
        b1.cite_key = Some("smith2020".to_string());
        let para = node(3, NodeKind::Paragraph, 1, "See \\citep[p.~3]{smith2020} and \\cite*{smith2020}.");
        let edges = resolve_relations(&[b1, para], RefStrategy::Tex);
        assert_eq!(
            edges
                .iter()
                .filter(|e| e.relation_type == RelationType::Cites)
                .count(),
            1,
            "同一 (from,cites,to) は 1 本に dedupe"
        );
    }

    #[test]
    fn tex_unresolved_ref_produces_no_edge() {
        let para = node(3, NodeKind::Paragraph, 1, "See \\ref{nowhere} and \\cite{ghost}.");
        let edges = resolve_relations(&[para], RefStrategy::Tex);
        assert!(edges.is_empty(), "ターゲット不在の参照は張らない");
    }

    #[test]
    fn tex_proof_proves_preceding_theorem_by_adjacency() {
        let thm = node(2, NodeKind::Theorem, 1, "Every bounded sequence has a limit point.");
        let proof = node(3, NodeKind::Proof, 2, "Straightforward.");
        let edges = resolve_relations(&[thm, proof], RefStrategy::Tex);
        let e = find(&edges, 3, RelationType::Proves, 2).expect("proof proves theorem");
        assert_eq!(e.origin, Origin::TexSource);
    }

    #[test]
    fn tex_proof_proves_referenced_theorem_over_adjacency() {
        // 直前は lemma(4) だが、証明本文は thm:main(=2) を \ref する → thm:main を証明。
        let mut thm = node(2, NodeKind::Theorem, 1, "Main result.");
        thm.labels = vec!["thm:main".to_string()];
        let lemma = node(4, NodeKind::Lemma, 2, "Helper.");
        let proof = node(5, NodeKind::Proof, 3, "Proof of Theorem~\\ref{thm:main}. Done.");
        let edges = resolve_relations(&[thm, lemma, proof], RefStrategy::Tex);
        assert!(
            find(&edges, 5, RelationType::Proves, 2).is_some(),
            "\\ref 先の定理を優先して proves"
        );
        assert!(
            find(&edges, 5, RelationType::Proves, 4).is_none(),
            "直前 lemma への adjacency proves は張らない"
        );
    }

    // ── PDF: 番号一致 ──

    #[test]
    fn pdf_theorem_ref_resolves_by_number() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem 2.3. Bounded implies compact.");
        thm.theorem_number = Some("2.3".to_string());
        let para = node(3, NodeKind::Paragraph, 2, "The claim follows by Theorem 2.3 above.");
        let edges = resolve_relations(&[thm, para], RefStrategy::Pdf);
        let e = find(&edges, 3, RelationType::RefersToTheorem, 2).expect("pdf theorem ref");
        assert_eq!(e.origin, Origin::LayoutModel);
        assert!((e.confidence - 0.6).abs() < 1e-9);
    }

    #[test]
    fn pdf_theorem_header_does_not_self_reference() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem 2.3. Bounded implies compact.");
        thm.theorem_number = Some("2.3".to_string());
        let edges = resolve_relations(&[thm], RefStrategy::Pdf);
        assert!(
            find(&edges, 2, RelationType::RefersToTheorem, 2).is_none(),
            "見出しの自己参照は張らない"
        );
    }

    #[test]
    fn pdf_theorem_ref_is_kind_aware_for_per_kind_counters() {
        // 論文が種別ごとに採番: Theorem 3.1 と Definition 3.1 が併存。
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem 3.1. First.");
        thm.theorem_number = Some("3.1".to_string());
        let mut def = node(3, NodeKind::Definition, 2, "Definition 3.1. A gadget is...");
        def.theorem_number = Some("3.1".to_string());
        let para = node(4, NodeKind::Paragraph, 3, "Recall Definition 3.1 for the setup.");
        let edges = resolve_relations(&[thm, def, para], RefStrategy::Pdf);
        assert!(
            find(&edges, 4, RelationType::RefersToTheorem, 3).is_some(),
            "\"Definition 3.1\" は Definition ノードに解決"
        );
        assert!(
            find(&edges, 4, RelationType::RefersToTheorem, 2).is_none(),
            "同番号の Theorem 3.1 には誤解決しない"
        );
    }

    #[test]
    fn pdf_equation_ref_resolves_by_number() {
        let mut eq = node(2, NodeKind::DisplayMath, 1, "x = y + z");
        eq.equation_label = Some("(2.1)".to_string());
        let para = node(3, NodeKind::Paragraph, 2, "Substituting into Eq. (2.1) gives the result.");
        let edges = resolve_relations(&[eq, para], RefStrategy::Pdf);
        assert!(find(&edges, 3, RelationType::RefersToEquation, 2).is_some());
    }

    #[test]
    fn pdf_plural_and_lowercase_are_not_matched() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem 2.3.");
        thm.theorem_number = Some("2.3".to_string());
        // "Theorems" (plural) と小文字 "theorem" は保守的に拾わない。
        let para = node(3, NodeKind::Paragraph, 2, "See Theorems 2.3 and by theorem 2.3.");
        let edges = resolve_relations(&[thm, para], RefStrategy::Pdf);
        assert!(edges.is_empty(), "plural/lowercase は誤検出を避けて拾わない");
    }

    #[test]
    fn pdf_proof_of_theorem_proves_by_number() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem 2.3. Statement.");
        thm.theorem_number = Some("2.3".to_string());
        let other = node(4, NodeKind::Lemma, 5, "Lemma 5. Helper.");
        // proof は lemma(reading 5) の直後(6) だが "Proof of Theorem 2.3" なので thm を証明。
        let proof = node(5, NodeKind::Proof, 6, "Proof of Theorem 2.3. Follows from Lemma 5.");
        let edges = resolve_relations(&[thm, other, proof], RefStrategy::Pdf);
        let e = find(&edges, 5, RelationType::Proves, 2).expect("proves by number");
        assert!((e.confidence - 0.7).abs() < 1e-9);
        assert!(
            find(&edges, 5, RelationType::Proves, 4).is_none(),
            "直前 lemma への adjacency proves にはしない"
        );
    }

    #[test]
    fn pdf_bare_proof_proves_preceding_theorem() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem 1. Statement.");
        thm.theorem_number = Some("1".to_string());
        let proof = node(3, NodeKind::Proof, 2, "Proof. Immediate from the definition.");
        let edges = resolve_relations(&[thm, proof], RefStrategy::Pdf);
        let e = find(&edges, 3, RelationType::Proves, 2).expect("adjacency proves");
        assert!((e.confidence - 0.6).abs() < 1e-9);
    }

    #[test]
    fn appendix_theorem_number_resolves() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem A.1. Appendix result.");
        thm.theorem_number = Some("A.1".to_string());
        let para = node(3, NodeKind::Paragraph, 2, "By Theorem A.1 the bound holds.");
        let edges = resolve_relations(&[thm, para], RefStrategy::Pdf);
        assert!(find(&edges, 3, RelationType::RefersToTheorem, 2).is_some());
    }

    #[test]
    fn no_theorem_before_proof_yields_no_proves() {
        let proof = node(3, NodeKind::Proof, 2, "Proof. Nothing precedes.");
        let edges = resolve_relations(&[proof], RefStrategy::Pdf);
        assert!(edges.is_empty(), "直前に定理が無ければ proves を張らない");
    }

    #[test]
    fn multibyte_text_does_not_panic() {
        // 日本語混じり + マクロ。バイト走査が UTF-8 境界を割らないこと。
        let mut thm = node(2, NodeKind::Theorem, 1, "定理。");
        thm.labels = vec!["thm:α".to_string()];
        let para = node(3, NodeKind::Paragraph, 2, "本文中で \\ref{thm:α} を参照する（日本語）。");
        let edges = resolve_relations(&[thm, para], RefStrategy::Tex);
        assert!(find(&edges, 3, RelationType::RefersToTheorem, 2).is_some());
    }

    #[test]
    fn metadata_is_populated() {
        let mut thm = node(2, NodeKind::Theorem, 1, "Theorem statement.");
        thm.labels = vec!["thm:main".to_string()];
        let para = node(3, NodeKind::Paragraph, 2, "See \\ref{thm:main}.");
        let edges = resolve_relations(&[thm, para], RefStrategy::Tex);
        let e = find(&edges, 3, RelationType::RefersToTheorem, 2).unwrap();
        let meta: serde_json::Value =
            serde_json::from_str(e.metadata_json.as_ref().unwrap()).unwrap();
        assert_eq!(meta["key"], "thm:main");
    }
}
