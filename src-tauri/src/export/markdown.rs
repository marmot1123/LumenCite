//! Phase 9a: `LcirDocument` → Markdown の決定的レンダラ（純関数・pdfium 非依存）。
//!
//! 原則は **verbatim 温存**: 本文・数式の生 LaTeX は一切書き換えない（エスケープもしない —
//! Markdown 都合の加工は LaTeX を壊す）。ヒューリスティックを含まないので「誤検出より欠損」を
//! 構造的に満たす。未知の `node_kind` は plain_text の段落に degrade し、Phase 7/8 の
//! ノード型追加でレンダラが壊れない。
//!
//! 品質は由来に依存する（TeX 版 = 原文 LaTeX / PDF 版 = surface-only の Unicode 線形）。
//! surface-only の数式には `$$` を**付けない** — 生 LaTeX でないものを数式と偽らない。
//! 由来はフロントマターの `lcir_source` で常に区別できる（roadmap §16）。

use std::collections::HashMap;

use crate::document_ir::{LcirDocument, LcirNode};

/// YAML フロントマターに載せるエントリ書誌情報。呼び出し側（Tauri コマンド / CLI）が
/// `EntryDetail` から組む。`None` ならフロントマターごと省略。
#[derive(Debug, Clone, Default)]
pub struct MarkdownHeader {
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<i64>,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
    pub citation_key: Option<String>,
}

/// `LcirDocument` を Markdown 文字列に描画する。
pub fn render_markdown(doc: &LcirDocument, header: Option<&MarkdownHeader>) -> String {
    let mut out = String::new();
    if let Some(h) = header {
        push_frontmatter(&mut out, h, doc);
    }

    // parent_id → 子ノード（ordinal 順）。nodes は DB 読み順だが並びは ordinal で保証する。
    let mut children: HashMap<Option<i64>, Vec<&LcirNode>> = HashMap::new();
    for n in &doc.nodes {
        children.entry(n.parent_id).or_default().push(n);
    }
    for v in children.values_mut() {
        v.sort_by_key(|n| n.ordinal);
    }

    for root in children.get(&None).cloned().unwrap_or_default() {
        render_node(root, &children, &mut out);
    }

    let trimmed = out.trim_end();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut s = trimmed.to_string();
    s.push('\n');
    s
}

fn render_node(
    n: &LcirNode,
    children: &HashMap<Option<i64>, Vec<&LcirNode>>,
    out: &mut String,
) {
    match n.kind.as_str() {
        // 骨格: 自分は描画せず子へ。`page` の plain_text はページ全文（＝ブロックと重複）
        // なので出さない。
        "document" | "page" => render_children(n, children, out),
        // `line` は親ブロックの plain_text と重複する。
        "line" => {}
        "front_matter" => {
            if let Some(t) = text(n) {
                push_block(out, &format!("# {t}"));
            }
        }
        "abstract" => {
            if let Some(t) = text(n) {
                push_block(out, "## Abstract");
                push_block(out, &t);
            }
        }
        "section" => push_heading(n, 2, out),
        "subsection" => push_heading(n, 3, out),
        "heading" => {
            // TeX の subsubsection 以下は heading + `heading_level`（1 = section 相当）。
            let level = payload_i64(n, "heading_level").unwrap_or(1).clamp(1, 5) as usize + 1;
            push_heading(n, level, out);
        }
        "display_math" => push_display_math(n, out),
        "definition" | "theorem" | "lemma" | "proposition" | "corollary" | "remark"
        | "example" => push_theorem_like(n, out),
        "proof" => {
            let body = text(n).unwrap_or_default();
            let full = if body.is_empty() {
                "*Proof.*".to_string()
            } else {
                format!("*Proof.* {body}")
            };
            push_block(out, &prefix_lines("> ", &full));
        }
        "figure_caption" | "table_caption" => {
            if let Some(t) = text(n) {
                push_block(out, &format!("*{t}*"));
            }
        }
        "list" => {
            let mut items = String::new();
            for c in children.get(&Some(n.id)).cloned().unwrap_or_default() {
                if let Some(t) = text(c) {
                    items.push_str(&format!("- {t}\n"));
                }
            }
            if !items.is_empty() {
                push_block(out, items.trim_end());
            }
        }
        // list 側で描画済み（list 直下以外に現れた場合は未知扱いに落とさず無視する）。
        "list_item" => {}
        "code_block" => {
            if let Some(t) = text(n) {
                push_block(out, &format!("```\n{t}\n```"));
            }
        }
        "bibliography" => {
            push_block(out, "## References");
            render_children(n, children, out);
        }
        "bibliography_entry" => {
            if let Some(t) = text(n) {
                match payload_str(n, "cite_key") {
                    Some(k) => push_block(out, &format!("- \\[{k}\\] {t}")),
                    None => push_block(out, &format!("- {t}")),
                }
            }
        }
        // paragraph / text_block / unknown_block / citation / footnote と、将来の未知型
        // （figure / table / inline_math / equation_group …）: plain_text の段落に degrade。
        // テキストが無ければ子に降りる（構造だけのコンテナを黙って捨てない）。
        _ => match text(n) {
            Some(t) => push_block(out, &t),
            None => render_children(n, children, out),
        },
    }
}

fn render_children(
    n: &LcirNode,
    children: &HashMap<Option<i64>, Vec<&LcirNode>>,
    out: &mut String,
) {
    for c in children.get(&Some(n.id)).cloned().unwrap_or_default() {
        render_node(c, children, out);
    }
}

/// 見出し。TeX/PDF とも `plain_text` は節番号込みなので、payload の `section_number` は
/// 「まだ含まれていないときだけ」前置する（二重付与しない）。
fn push_heading(n: &LcirNode, level: usize, out: &mut String) {
    let Some(mut t) = text(n) else { return };
    if let Some(num) = payload_str(n, "section_number") {
        if !t.starts_with(&num) {
            t = format!("{num} {t}");
        }
    }
    push_block(out, &format!("{} {}", "#".repeat(level.clamp(1, 6)), t));
}

/// display 数式。原文 LaTeX（TeX 版）があれば `$$..$$` に正規化して出す。
/// 原文スニペットは区切り込み verbatim（`\begin{equation}..\end{equation}` / `\[..\]` /
/// `$$..$$`）なので、二重区切りにならないよう形ごとに扱う。`\tag`/`\label` は原文のまま。
fn push_display_math(n: &LcirNode, out: &mut String) {
    let math = n.math.as_ref();
    if let Some(l) = math.and_then(|m| m.latex.as_deref()) {
        let l = l.trim();
        let block = if l.starts_with("$$") && l.ends_with("$$") && l.len() >= 4 {
            l.to_string()
        } else if let Some(inner) = l.strip_prefix("\\[").and_then(|s| s.strip_suffix("\\]")) {
            format!("$$\n{}\n$$", inner.trim())
        } else {
            format!("$$\n{l}\n$$")
        };
        push_block(out, &block);
        return;
    }
    // surface-only（PDF 由来）: 生 LaTeX でないので `$$` を付けず、そのまま段落に出す。
    let t = math
        .and_then(|m| m.normalized_text.clone())
        .or_else(|| text(n));
    if let Some(mut t) = t {
        if let Some(lbl) = math.and_then(|m| m.equation_label.as_deref()) {
            if !t.contains(lbl) {
                t.push_str("  ");
                t.push_str(lbl);
            }
        }
        push_block(out, &t);
    }
}

/// 定理系ノード（Phase 5）。`> **Theorem 2.3** (Note). 本文` の blockquote。
fn push_theorem_like(n: &LcirNode, out: &mut String) {
    let mut head = format!("**{}", capitalize(&n.kind));
    if let Some(num) = payload_str(n, "theorem_number") {
        head.push(' ');
        head.push_str(&num);
    }
    head.push_str("**");
    if let Some(note) = payload_str(n, "note") {
        head.push_str(&format!(" ({note})"));
    }
    head.push('.');
    let full = match text(n) {
        Some(body) => format!("{head} {body}"),
        None => head,
    };
    push_block(out, &prefix_lines("> ", &full));
}

fn push_frontmatter(out: &mut String, h: &MarkdownHeader, doc: &LcirDocument) {
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_str(&h.title)));
    if !h.authors.is_empty() {
        out.push_str("authors:\n");
        for a in &h.authors {
            out.push_str(&format!("  - {}\n", yaml_str(a)));
        }
    }
    if let Some(y) = h.year {
        out.push_str(&format!("year: {y}\n"));
    }
    if let Some(d) = &h.doi {
        out.push_str(&format!("doi: {}\n", yaml_str(d)));
    }
    if let Some(a) = &h.arxiv_id {
        out.push_str(&format!("arxiv: {}\n", yaml_str(a)));
    }
    if let Some(k) = &h.citation_key {
        out.push_str(&format!("citation_key: {}\n", yaml_str(k)));
    }
    // 由来の明示（roadmap §16「AI 推定と原文由来の区別」）: 抽出器名 + 版。
    out.push_str(&format!(
        "lcir_source: {}\n",
        yaml_str(&format!(
            "{} {}",
            doc.source.extractor_name, doc.source.extractor_version
        ))
    ));
    out.push_str(&format!(
        "lcir_schema_version: {}\n",
        yaml_str(&doc.schema_version)
    ));
    out.push_str("---\n\n");
}

/// YAML の double-quoted scalar（`\` と `"` のみエスケープ。LaTeX 混じりの題名に耐える）。
fn yaml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn text(n: &LcirNode) -> Option<String> {
    let t = n.plain_text.as_deref()?.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn payload_str(n: &LcirNode, key: &str) -> Option<String> {
    n.payload
        .as_ref()?
        .get(key)?
        .as_str()
        .map(|s| s.to_string())
}

fn payload_i64(n: &LcirNode, key: &str) -> Option<i64> {
    n.payload.as_ref()?.get(key)?.as_i64()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn prefix_lines(prefix: &str, s: &str) -> String {
    s.lines()
        .map(|l| format!("{prefix}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn push_block(out: &mut String, block: &str) {
    out.push_str(block);
    out.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_ir::{BBox, LcirFragment, LcirMath, LcirSource};

    fn node(id: i64, parent: Option<i64>, ordinal: i64, kind: &str, text: Option<&str>) -> LcirNode {
        LcirNode {
            id,
            kind: kind.to_string(),
            ordinal,
            parent_id: parent,
            plain_text: text.map(|s| s.to_string()),
            origin: None,
            confidence: None,
            payload: None,
            math: None,
            source_fragments: Vec::new(),
        }
    }

    fn doc(nodes: Vec<LcirNode>) -> LcirDocument {
        LcirDocument {
            schema: "https://lumencite.dev/schema/document-ir/0.1".to_string(),
            schema_version: "0.1.0".to_string(),
            version_id: 1,
            content_key: "k".to_string(),
            source: LcirSource {
                sha256: "s".to_string(),
                mime_type: "application/gzip".to_string(),
                extractor_name: "lumencite-tex".to_string(),
                extractor_version: "0.4.0".to_string(),
            },
            coordinate_space: None,
            nodes,
            relations: Vec::new(),
            symbols: Vec::new(),
        }
    }

    #[test]
    fn renders_tex_style_structure() {
        let mut sec = node(3, Some(1), 1, "section", Some("1 Introduction"));
        sec.payload = serde_json::json!({"section_number": "1", "heading_level": 1}).into();
        let mut math = node(5, Some(1), 3, "display_math", None);
        math.math = Some(LcirMath {
            display_mode: "display".to_string(),
            equation_label: None,
            latex: Some("\\begin{equation}\\label{eq:e}E=mc^2\\end{equation}".to_string()),
            presentation_mathml: None,
            content_mathml: None,
            openmath: None,
            normalized_text: Some("E = m c^2".to_string()),
            semantic_status: "source_provided".to_string(),
            confidence: Some(0.98),
            origin: Some("tex_source".to_string()),
        });
        let mut thm = node(6, Some(1), 4, "theorem", Some("Every $E$ is conserved."));
        thm.payload = serde_json::json!({"theorem_number": "2.3", "note": "Noether"}).into();
        let proof = node(7, Some(1), 5, "proof", Some("Trivial. $\\square$"));
        let bib = node(8, Some(1), 6, "bibliography", None);
        let mut be = node(9, Some(8), 0, "bibliography_entry", Some("A. Author, Title."));
        be.payload = serde_json::json!({"cite_key": "author2020"}).into();

        let d = doc(vec![
            node(1, None, 0, "document", None),
            node(2, Some(1), 0, "front_matter", Some("Tex Paper")),
            sec,
            node(4, Some(1), 2, "paragraph", Some("Let $z=1$ be inline.")),
            math,
            thm,
            proof,
            bib,
            be,
        ]);
        let md = render_markdown(&d, None);

        assert!(md.starts_with("# Tex Paper\n"), "{md}");
        assert!(md.contains("\n## 1 Introduction\n"), "節番号は二重付与しない: {md}");
        assert!(md.contains("Let $z=1$ be inline."), "{md}");
        assert!(
            md.contains("$$\n\\begin{equation}\\label{eq:e}E=mc^2\\end{equation}\n$$"),
            "原文 LaTeX を $$ で包む: {md}"
        );
        assert!(
            md.contains("> **Theorem 2.3** (Noether). Every $E$ is conserved."),
            "{md}"
        );
        assert!(md.contains("> *Proof.* Trivial. $\\square$"), "{md}");
        assert!(md.contains("## References"), "{md}");
        assert!(md.contains("- \\[author2020\\] A. Author, Title."), "{md}");
        assert!(md.ends_with('\n') && !md.ends_with("\n\n"), "末尾は単一改行: {md:?}");
    }

    #[test]
    fn math_delimiters_are_not_doubled() {
        let mut bracket = node(2, Some(1), 0, "display_math", None);
        bracket.math = Some(LcirMath {
            display_mode: "display".to_string(),
            equation_label: None,
            latex: Some("\\[ a^2 + b^2 = c^2 \\]".to_string()),
            presentation_mathml: None,
            content_mathml: None,
            openmath: None,
            normalized_text: None,
            semantic_status: "source_provided".to_string(),
            confidence: None,
            origin: None,
        });
        let mut dollars = node(3, Some(1), 1, "display_math", None);
        dollars.math = Some(LcirMath {
            display_mode: "display".to_string(),
            equation_label: None,
            latex: Some("$$ x = y $$".to_string()),
            presentation_mathml: None,
            content_mathml: None,
            openmath: None,
            normalized_text: None,
            semantic_status: "source_provided".to_string(),
            confidence: None,
            origin: None,
        });
        let d = doc(vec![node(1, None, 0, "document", None), bracket, dollars]);
        let md = render_markdown(&d, None);
        assert!(md.contains("$$\na^2 + b^2 = c^2\n$$"), "\\[..\\] は剥がして包む: {md}");
        assert!(md.contains("$$ x = y $$"), "$$..$$ はそのまま: {md}");
        assert!(!md.contains("$$$"), "二重区切りを作らない: {md}");
    }

    #[test]
    fn pdf_surface_math_gets_no_dollars_and_page_text_is_skipped() {
        let mut d = doc(Vec::new());
        d.source.extractor_name = "lumencite-pdfium".to_string();
        d.coordinate_space = Some(crate::document_ir::CoordinateSpace::default());

        let mut page = node(2, Some(1), 0, "page", Some("FULL PAGE TEXT DUPLICATE"));
        page.source_fragments = vec![LcirFragment {
            page: 1,
            bbox: BBox::new(0.0, 0.0, 595.0, 842.0),
            fragment_type: Some("page".to_string()),
        }];
        let block = node(3, Some(2), 0, "paragraph", Some("A paragraph."));
        let line = node(4, Some(3), 0, "line", Some("A paragraph."));
        let mut math = node(5, Some(2), 1, "display_math", Some("E = m c 2 (2.1)"));
        math.math = Some(LcirMath {
            display_mode: "display".to_string(),
            equation_label: Some("(2.1)".to_string()),
            latex: None,
            presentation_mathml: None,
            content_mathml: None,
            openmath: None,
            normalized_text: Some("E = m c^2".to_string()),
            semantic_status: "surface_only".to_string(),
            confidence: None,
            origin: Some("pdf_text_layer".to_string()),
        });
        d.nodes = vec![node(1, None, 0, "document", None), page, block, line, math];

        let md = render_markdown(&d, None);
        assert!(!md.contains("FULL PAGE TEXT DUPLICATE"), "page 全文は出さない: {md}");
        assert_eq!(md.matches("A paragraph.").count(), 1, "line と重複させない: {md}");
        assert!(!md.contains("$$"), "surface-only に $$ を付けない: {md}");
        assert!(md.contains("E = m c^2  (2.1)"), "数式番号を添える: {md}");
    }

    #[test]
    fn unknown_kinds_degrade_to_paragraph_and_lists_render() {
        let future = node(2, Some(1), 0, "figure", Some("Figure body text (Phase 8)."));
        let list = node(3, Some(1), 1, "list", None);
        let li1 = node(4, Some(3), 0, "list_item", Some("first"));
        let li2 = node(5, Some(3), 1, "list_item", Some("second"));
        let code = node(6, Some(1), 2, "code_block", Some("let x = 1;"));
        let cap = node(7, Some(1), 3, "figure_caption", Some("Figure 1: caption"));
        let d = doc(vec![node(1, None, 0, "document", None), future, list, li1, li2, code, cap]);
        let md = render_markdown(&d, None);
        assert!(md.contains("Figure body text (Phase 8)."), "未知型は段落に degrade: {md}");
        assert!(md.contains("- first\n- second"), "{md}");
        assert!(md.contains("```\nlet x = 1;\n```"), "{md}");
        assert!(md.contains("*Figure 1: caption*"), "{md}");
    }

    #[test]
    fn frontmatter_escapes_yaml_and_records_provenance() {
        let d = doc(vec![
            node(1, None, 0, "document", None),
            node(2, Some(1), 0, "paragraph", Some("Body.")),
        ]);
        let header = MarkdownHeader {
            title: "On \"quoted\" $\\tau$-periodic walks".to_string(),
            authors: vec!["Alice A.".to_string(), "Bob B.".to_string()],
            year: Some(2026),
            doi: Some("10.1000/xyz".to_string()),
            arxiv_id: Some("2607.14797".to_string()),
            citation_key: Some("alice2026".to_string()),
        };
        let md = render_markdown(&d, Some(&header));
        assert!(md.starts_with("---\n"), "{md}");
        assert!(
            md.contains(r#"title: "On \"quoted\" $\\tau$-periodic walks""#),
            "YAML エスケープ: {md}"
        );
        assert!(md.contains("  - \"Alice A.\""), "{md}");
        assert!(md.contains("year: 2026"), "{md}");
        assert!(md.contains("citation_key: \"alice2026\""), "{md}");
        assert!(md.contains("lcir_source: \"lumencite-tex 0.4.0\""), "{md}");
        assert!(md.contains("lcir_schema_version: \"0.1.0\""), "{md}");
        assert!(md.contains("---\n\nBody.\n"), "{md}");
    }

    #[test]
    fn empty_document_renders_empty_string() {
        let d = doc(vec![node(1, None, 0, "document", None)]);
        assert_eq!(render_markdown(&d, None), "");
    }
}
