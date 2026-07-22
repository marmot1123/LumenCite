//! 記号定義の抽出（Phase 6b・DB 非依存の純関数）。
//!
//! TeX 本文の定義文（"let $U$ be ...", "define $H$ as ...", "denote by $\mathcal{H}$ ...",
//! "$U := ...$"）から**インライン数式 `$...$` / `\(...\)`** を記号として取り出し、周辺の散文から
//! 説明を付ける。**PDF は対象外**（インライン数式が区切り無しで潰れ、記号を確実に切り出せない）。
//!
//! surface_form/description は TeX 本文の verbatim だが、「この文がこの記号を定義している」対応づけ
//! はヒューリスティック推定なので `confidence` で区別する（roadmap §16「誤検出より欠損」＝強い
//! トリガ + インライン数式が揃ったときだけ拾う）。出現（`symbol_occurrences`）は保守的に、
//! **定義済み記号が display 数式に表層一致した箇所だけ**を記録する。

use crate::document_ir::{NodeKind, SymbolType};
use std::collections::HashSet;

/// 記号抽出に使うノードの軽量ビュー（TeX block・plain_text は `$...$` を原文温存）。
#[derive(Debug, Clone)]
pub struct SymbolNode {
    pub id: i64,
    pub kind: NodeKind,
    pub reading_index: i64,
    pub plain_text: String,
    /// display 数式の LaTeX（出現照合に使う）。display_math のみ。
    pub latex: Option<String>,
}

/// 抽出した記号定義（`symbols` の 1 行になる）。origin は呼び出し側が tex_source を付ける。
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedSymbol {
    pub surface_form: String,
    pub normalized_form: Option<String>,
    pub description: Option<String>,
    pub symbol_type: Option<SymbolType>,
    pub defined_at_node_id: i64,
    pub scope_node_id: Option<i64>,
    pub confidence: f64,
}

/// 抽出した記号出現（`symbol_occurrences` の 1 行になる）。`symbol_index` は返り値 symbols の位置。
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedOccurrence {
    pub symbol_index: usize,
    pub node_id: i64,
    pub surface_form: String,
    pub confidence: f64,
}

const OCCURRENCE_CONF: f64 = 0.5;

/// ノード集合から記号定義と出現を抽出する。
pub fn extract_symbols(nodes: &[SymbolNode]) -> (Vec<ExtractedSymbol>, Vec<ExtractedOccurrence>) {
    let mut ordered: Vec<&SymbolNode> = nodes.iter().collect();
    ordered.sort_by_key(|n| n.reading_index);

    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let mut seen: HashSet<(String, i64)> = HashSet::new();
    let mut current_section: Option<i64> = None;

    for n in &ordered {
        if matches!(n.kind, NodeKind::Section | NodeKind::Subsection) {
            current_section = Some(n.id);
        }
        if !is_prose_kind(n.kind) {
            continue;
        }
        for (a, b, content) in find_inline_math(&n.plain_text) {
            let before = &n.plain_text[..a];
            let after = &n.plain_text[b..];
            if let Some((surface, description, confidence)) =
                classify_definition(before, after, &content)
            {
                // 同じ節内で同一表層の再定義（"let $G$ be a graph" を各命題で繰り返す等）は 1 個に。
                if !seen.insert((surface.clone(), current_section.unwrap_or(-1))) {
                    continue;
                }
                let normalized = normalize_symbol(&surface);
                let symbol_type = description.as_deref().and_then(infer_symbol_type);
                symbols.push(ExtractedSymbol {
                    normalized_form: (normalized != surface).then_some(normalized),
                    symbol_type,
                    surface_form: surface,
                    description,
                    defined_at_node_id: n.id,
                    scope_node_id: current_section,
                    confidence,
                });
            }
        }
    }

    // 出現: display 数式ごとに、定義済み記号の表層一致だけを記録（保守的）。
    let mut occurrences: Vec<ExtractedOccurrence> = Vec::new();
    let mut occ_seen: HashSet<(usize, i64)> = HashSet::new();
    for n in &ordered {
        if n.kind != NodeKind::DisplayMath {
            continue;
        }
        let hay = n.latex.as_deref().unwrap_or(&n.plain_text);
        for (si, sym) in symbols.iter().enumerate() {
            if symbol_occurs(hay, &sym.surface_form) && occ_seen.insert((si, n.id)) {
                occurrences.push(ExtractedOccurrence {
                    symbol_index: si,
                    node_id: n.id,
                    surface_form: sym.surface_form.clone(),
                    confidence: OCCURRENCE_CONF,
                });
            }
        }
    }

    (symbols, occurrences)
}

fn is_prose_kind(k: NodeKind) -> bool {
    matches!(
        k,
        NodeKind::Paragraph
            | NodeKind::Abstract
            | NodeKind::Definition
            | NodeKind::Theorem
            | NodeKind::Lemma
            | NodeKind::Proposition
            | NodeKind::Corollary
            | NodeKind::Remark
            | NodeKind::Example
            | NodeKind::ListItem
            | NodeKind::Footnote
    )
}

// ── 定義文の分類 ──

/// インライン数式スパンの前後の文脈から定義を分類し、(surface, description, confidence) を返す。
/// surface は content の**先頭記号**（`=`/`:=` があれば LHS だけ）にそろえる。
fn classify_definition(
    before: &str,
    after: &str,
    content: &str,
) -> Option<(String, Option<String>, f64)> {
    let btl = before.trim_end().to_lowercase();
    let (surface, has_eq, is_coloneq) = leading_symbol(content);
    if !is_symbol_like(&surface) {
        return None;
    }

    // := は無条件で定義。= は明示トリガがあるときだけ（"we have $x=y$" 等の派生式を除外）。
    if is_coloneq {
        return Some((surface, None, 0.6));
    }
    if has_eq {
        if ends_with_any_word(&btl, &["let", "set", "define", "put", "denote", "where"]) {
            return Some((surface, None, 0.55));
        }
        return None; // トリガ無しの = は定義ではない
    }

    // (B) surface が トリガの後（let/define/denote by/write/call $X$ ...）。
    if ends_with_word(&btl, "let") {
        let desc = strip_link(after, &["be", "denote", "denotes"]).and_then(clean_description);
        return Some((surface, desc, 0.6));
    }
    if ends_with_word(&btl, "define") {
        let desc = strip_link(after, &["as", "to be", "by"])
            .and_then(clean_description)
            .or_else(|| clean_description(after));
        return Some((surface, desc, 0.6));
    }
    if btl.ends_with("denote by") || btl.ends_with("denoted by") {
        return Some((surface, clean_description(after), 0.6));
    }
    if ends_with_word(&btl, "write") {
        let desc = strip_link(after, &["for", "as"]).and_then(clean_description);
        return Some((surface, desc, 0.55));
    }
    if ends_with_word(&btl, "call") {
        return Some((surface, clean_description(after), 0.5));
    }

    // (C) surface が トリガの前（$X$ denotes / is defined / is called / stands for ...）。
    let a = after.trim_start();
    let al = a.to_lowercase();
    if starts_with_word(&al, "denotes") {
        return Some((surface, clean_description(&a["denotes".len()..]), 0.6));
    }
    if al.starts_with("is defined") {
        let rest = a["is defined".len()..].trim_start();
        let rest = strip_link(rest, &["as", "by", "to be"]).unwrap_or(rest);
        return Some((surface, clean_description(rest), 0.6));
    }
    if al.starts_with("is called") {
        return Some((surface, clean_description(&a["is called".len()..]), 0.55));
    }
    if al.starts_with("stands for") {
        return Some((surface, clean_description(&a["stands for".len()..]), 0.55));
    }

    None
}

/// content の先頭記号を返す。トップレベルに `:=`/`=` があれば LHS だけを取り、(surface, has_eq,
/// is_coloneq) を返す（"U_\beta = U_\beta(G,a)" → surface "U_\beta"）。
fn leading_symbol(content: &str) -> (String, bool, bool) {
    if let Some(p) = find_top_level(content, ":=") {
        return (content[..p].trim().to_string(), true, true);
    }
    if let Some(p) = find_top_level_eq(content) {
        return (content[..p].trim().to_string(), true, false);
    }
    (content.trim().to_string(), false, false)
}

/// トップレベル（brace 深度 0）の部分文字列位置。
fn find_top_level(s: &str, needle: &str) -> Option<usize> {
    let b = s.as_bytes();
    let nb = needle.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i + nb.len() <= b.len() {
        match b[i] {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            _ => {}
        }
        if depth <= 0 && &b[i..i + nb.len()] == nb {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// トップレベルの単一 `=`（`:=`/`==`/`<=`/`>=`/`!=`/`\=` 等の一部でないもの）。
fn find_top_level_eq(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            b'=' if depth <= 0 => {
                let prev = if i > 0 { b[i - 1] } else { b' ' };
                let next = if i + 1 < b.len() { b[i + 1] } else { b' ' };
                if !matches!(prev, b':' | b'<' | b'>' | b'!' | b'\\' | b'=')
                    && next != b'='
                {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// 単一の記号らしいか（短く・トップレベルに空白が無く・英字か `\` を含む）。
fn is_symbol_like(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > 24 {
        return false;
    }
    let mut depth = 0i32;
    let mut has_letter_or_cmd = false;
    for c in s.chars() {
        match c {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            ' ' | '\t' if depth <= 0 => return false, // 深度 0 の空白 = 複数トークン/散文
            'a'..='z' | 'A'..='Z' | '\\' => has_letter_or_cmd = true,
            _ => {}
        }
    }
    has_letter_or_cmd
}

/// 装飾コマンド（\mathcal/\mathbf/\hat/…）を剥いた正規化表層。
fn normalize_symbol(surface: &str) -> String {
    let wrappers = [
        "\\mathbf",
        "\\mathcal",
        "\\mathrm",
        "\\mathbb",
        "\\mathsf",
        "\\mathfrak",
        "\\mathit",
        "\\boldsymbol",
        "\\bm",
        "\\vec",
        "\\hat",
        "\\tilde",
        "\\bar",
        "\\overline",
        "\\underline",
        "\\operatorname",
    ];
    let mut s = surface.trim().to_string();
    loop {
        let mut changed = false;
        for w in wrappers {
            if let Some(rest) = s.strip_prefix(w) {
                let rest = rest.trim_start();
                if let Some(inner) = rest.strip_prefix('{') {
                    if let Some(close) = inner.find('}') {
                        s = format!("{}{}", &inner[..close], &inner[close + 1..]);
                        changed = true;
                        break;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    s.trim().to_string()
}

/// 説明文の推定型（best-effort）。
fn infer_symbol_type(desc: &str) -> Option<SymbolType> {
    let d = desc.to_lowercase();
    for (kw, ty) in [
        ("operator", SymbolType::Operator),
        ("adjacency", SymbolType::Matrix),
        ("matrix", SymbolType::Matrix),
        ("graph", SymbolType::Graph),
        ("group", SymbolType::Group),
        ("space", SymbolType::Space),
        ("vector", SymbolType::Vector),
        ("function", SymbolType::Function),
        ("mapping", SymbolType::Map),
        ("field", SymbolType::Field),
        ("constant", SymbolType::Constant),
    ] {
        if d.contains(kw) {
            return Some(ty);
        }
    }
    None
}

// ── 出現照合 ──

fn symbol_occurs(latex: &str, surface: &str) -> bool {
    let s = surface.trim();
    if s.is_empty() {
        return false;
    }
    if s.starts_with('\\') {
        latex.contains(s) // \gamma 等はリテラル一致
    } else {
        token_match(latex, s) // 英字境界のトークン一致
    }
}

fn token_match(hay: &str, needle: &str) -> bool {
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() {
        return false;
    }
    let mut i = 0;
    while let Some(p) = find_from(hb, i, nb) {
        let before_ok = p == 0 || !hb[p - 1].is_ascii_alphabetic();
        let after = p + nb.len();
        let after_ok = after >= hb.len() || !hb[after].is_ascii_alphabetic();
        if before_ok && after_ok {
            return true;
        }
        i = p + 1;
    }
    false
}

// ── 文字列ヘルパ ──

fn find_inline_math(text: &str) -> Vec<(usize, usize, String)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // \( ... \)
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'(') {
            if let Some(close) = find_from(b, i + 2, b"\\)") {
                out.push((i, close + 2, text[i + 2..close].to_string()));
                i = close + 2;
                continue;
            }
        }
        // $ ... $（$$ でなく・エスケープされていない）
        if b[i] == b'$' && preceding_backslashes_even(b, i) {
            if b.get(i + 1) == Some(&b'$') {
                i += 2;
                continue;
            }
            let mut j = i + 1;
            while j < b.len() {
                if b[j] == b'$' && preceding_backslashes_even(b, j) {
                    break;
                }
                j += 1;
            }
            if j < b.len() {
                out.push((i, j + 1, text[i + 1..j].to_string()));
                i = j + 1;
                continue;
            } else {
                break; // 閉じない
            }
        }
        i += 1;
    }
    out
}

fn preceding_backslashes_even(b: &[u8], pos: usize) -> bool {
    let mut n = 0;
    let mut k = pos;
    while k > 0 && b[k - 1] == b'\\' {
        n += 1;
        k -= 1;
    }
    n % 2 == 0
}

fn find_from(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= hay.len() || needle.len() > hay.len() - from {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn clean_description(after_link: &str) -> Option<String> {
    let s = after_link.trim_start();
    // インライン数式（$..$）は説明に含める（"$\tau$-periodic Grover walk" 等・LaTeX 読者向け）。
    // 文末・display 数式・改行・長さで切る。
    let mut end = s.len();
    for pat in [". ", "\\[", "\n", "; "] {
        if let Some(p) = s.find(pat) {
            if p < end {
                end = p;
            }
        }
    }
    if end > 160 {
        end = 160;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
    }
    let desc = s[..end]
        .trim()
        .trim_end_matches([',', ';', ':', '.', ' '])
        .trim();
    if desc.chars().count() < 3 {
        None
    } else {
        Some(desc.to_string())
    }
}

fn ends_with_word(haystack_lower: &str, word: &str) -> bool {
    if !haystack_lower.ends_with(word) {
        return false;
    }
    let idx = haystack_lower.len() - word.len();
    idx == 0 || !haystack_lower.as_bytes()[idx - 1].is_ascii_alphabetic()
}

fn ends_with_any_word(haystack_lower: &str, words: &[&str]) -> bool {
    words.iter().any(|w| ends_with_word(haystack_lower, w))
}

fn starts_with_word(haystack_lower: &str, word: &str) -> bool {
    if !haystack_lower.starts_with(word) {
        return false;
    }
    let after = word.len();
    after == haystack_lower.len() || !haystack_lower.as_bytes()[after].is_ascii_alphabetic()
}

fn strip_link<'a>(after: &'a str, links: &[&str]) -> Option<&'a str> {
    let a = after.trim_start();
    let al = a.to_lowercase();
    for &link in links {
        if starts_with_word(&al, link) {
            return Some(a[link.len()..].trim_start());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i64, kind: NodeKind, ri: i64, text: &str) -> SymbolNode {
        SymbolNode {
            id,
            kind,
            reading_index: ri,
            plain_text: text.to_string(),
            latex: None,
        }
    }

    fn only(nodes: &[SymbolNode]) -> Vec<ExtractedSymbol> {
        extract_symbols(nodes).0
    }

    #[test]
    fn let_be_extracts_symbol_description_and_type() {
        let syms = only(&[node(
            1,
            NodeKind::Paragraph,
            0,
            "In this paper, let $U$ be the time evolution operator of the walk.",
        )]);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].surface_form, "U");
        assert_eq!(syms[0].description.as_deref(), Some("the time evolution operator of the walk"));
        assert_eq!(syms[0].symbol_type, Some(SymbolType::Operator));
        assert!((syms[0].confidence - 0.6).abs() < 1e-9);
    }

    #[test]
    fn define_as_extracts() {
        let syms = only(&[node(1, NodeKind::Paragraph, 0, "We define $H$ as the Hamiltonian matrix.")]);
        assert_eq!(syms[0].surface_form, "H");
        assert_eq!(syms[0].description.as_deref(), Some("the Hamiltonian matrix"));
        assert_eq!(syms[0].symbol_type, Some(SymbolType::Matrix));
    }

    #[test]
    fn denote_by_with_mathcal_normalizes() {
        let syms = only(&[node(
            1,
            NodeKind::Definition,
            0,
            "We denote by $\\mathcal{H}$ the Hilbert space of states.",
        )]);
        assert_eq!(syms[0].surface_form, "\\mathcal{H}");
        assert_eq!(syms[0].normalized_form.as_deref(), Some("H"));
        assert_eq!(syms[0].description.as_deref(), Some("the Hilbert space of states"));
        assert_eq!(syms[0].symbol_type, Some(SymbolType::Space));
    }

    #[test]
    fn coloneq_equation_is_definition_without_trigger() {
        let syms = only(&[node(1, NodeKind::Paragraph, 0, "Here $U := S_2 C_2 S_1 C_1$ acts on the walk.")]);
        assert_eq!(syms.len(), 1);
        assert_eq!(syms[0].surface_form, "U");
        assert_eq!(syms[0].description, None);
    }

    #[test]
    fn plain_equation_without_trigger_is_not_a_definition() {
        let syms = only(&[node(1, NodeKind::Paragraph, 0, "We then have $x = y + z$ by substitution.")]);
        assert!(syms.is_empty(), "トリガの無い = は派生式であって定義ではない");
    }

    #[test]
    fn set_trigger_with_equation() {
        let syms = only(&[node(1, NodeKind::Paragraph, 0, "Set $M = D - A$ for the graph Laplacian.")]);
        assert_eq!(syms[0].surface_form, "M");
    }

    #[test]
    fn sym_denotes_after_trigger() {
        let syms = only(&[node(1, NodeKind::Paragraph, 0, "Here $\\gamma$ denotes the magnetic phase.")]);
        assert_eq!(syms[0].surface_form, "\\gamma");
        assert_eq!(syms[0].description.as_deref(), Some("the magnetic phase"));
    }

    #[test]
    fn sym_is_defined_as() {
        let syms = only(&[node(1, NodeKind::Paragraph, 0, "The set $V$ is defined as the vertex set.")]);
        assert_eq!(syms[0].surface_form, "V");
        assert_eq!(syms[0].description.as_deref(), Some("the vertex set"));
    }

    #[test]
    fn no_trigger_no_symbol() {
        let syms = only(&[node(1, NodeKind::Paragraph, 0, "For all $x$ in the domain, the map is continuous.")]);
        assert!(syms.is_empty(), "定義トリガが無ければ拾わない（$x$ は束縛変数）");
    }

    #[test]
    fn section_titles_are_not_scanned() {
        let syms = only(&[node(1, NodeKind::Section, 0, "3 Let $G$ be finite")]);
        assert!(syms.is_empty(), "見出しは散文ではないので走査しない");
    }

    #[test]
    fn scope_is_nearest_preceding_section() {
        let nodes = vec![
            node(10, NodeKind::Section, 0, "2 Preliminaries"),
            node(11, NodeKind::Paragraph, 1, "Let $A$ be the adjacency matrix."),
        ];
        let syms = only(&nodes);
        assert_eq!(syms[0].scope_node_id, Some(10));
    }

    #[test]
    fn dedup_same_surface_in_one_node() {
        // 同じノードで同じ表層が二度定義形にマッチしても 1 行。
        let syms = only(&[node(
            1,
            NodeKind::Paragraph,
            0,
            "Let $U$ be the operator; here $U$ denotes the same operator.",
        )]);
        assert_eq!(syms.len(), 1);
    }

    #[test]
    fn occurrences_only_in_display_math_for_defined_symbols() {
        let mut eq = node(2, NodeKind::DisplayMath, 1, "U = S_2 C_2 S_1 C_1");
        eq.latex = Some("U = S_2 C_2 S_1 C_1".to_string());
        let nodes = vec![
            node(1, NodeKind::Paragraph, 0, "Let $U$ be the time evolution operator."),
            eq,
        ];
        let (syms, occs) = extract_symbols(&nodes);
        assert_eq!(syms.len(), 1);
        assert_eq!(occs.len(), 1);
        assert_eq!(occs[0].node_id, 2);
        assert_eq!(occs[0].surface_form, "U");
        assert_eq!(syms[occs[0].symbol_index].surface_form, "U");
    }

    #[test]
    fn occurrence_token_match_respects_letter_boundary() {
        // "U" は "Universe" 中の U には一致しない（英字境界）。
        let mut eq = node(2, NodeKind::DisplayMath, 1, "Universe");
        eq.latex = Some("Universe".to_string());
        let nodes = vec![node(1, NodeKind::Paragraph, 0, "Let $U$ be the operator."), eq];
        let (_syms, occs) = extract_symbols(&nodes);
        assert!(occs.is_empty(), "英字の途中には一致しない");
    }

    #[test]
    fn multibyte_text_does_not_panic() {
        let syms = only(&[node(
            1,
            NodeKind::Paragraph,
            0,
            "ここで $U$ を時間発展作用素とする。let $H$ be the Hamiltonian（日本語混在）.",
        )]);
        // 少なくとも $H$（英語トリガ）は拾える。panic しないことが主眼。
        assert!(syms.iter().any(|s| s.surface_form == "H"));
    }
}
