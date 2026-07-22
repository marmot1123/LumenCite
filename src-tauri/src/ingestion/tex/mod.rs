//! Phase 4: TeX 構造認識。arXiv TeX ソースを型付きブロック列（`TexBlock`）へ変換する。
//!
//! pdfium にも sqlx にも依存しない純関数群で、CI で完全にテストできる。
//! パイプライン: コンテナ展開（`source`）→ main 検出 → コメント除去 + `\input` 解決 →
//! 本文走査でブロック化。**原文由来**（`origin='tex_source'`）なので、本文テキストは
//! LaTeX コマンドを温存し（ユーザーは生 LaTeX を直読する）、display 数式は原文スニペットを
//! そのまま保持する（`semantic_status='source_provided'`）。
//!
//! ## 字句規則（全走査で共通）
//!
//! `\[` `\]` `$` `$$` `%` `{` `}` は、**直前の連続バックスラッシュが偶数個**の位置でだけ
//! トークンと認識する。これにより `\\[4pt]`（改行 + 間隔）を display 数式 `\[` と誤認せず、
//! `\%` は文字として保護され、`a\\%` の `%` はコメント開始になる。
//!
//! ## 認識しない・確定しないもの（roadmap「欠損を許容」）
//!
//! - LaTeX 数式番号の完全エミュレーション（`\tag{X}` のみ label 化。誤番号より欠番）。
//! - インライン数式の独立ノード化（本文に生 LaTeX のまま残す）。
//! - 任意マクロ展開（preamble の自明な `\be`→`\begin{equation}` 型エイリアスのみ対応）。

pub mod source;

use crate::document_ir::NodeKind;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 解決済みソースの上限（`\input` スプライスの増幅ガード）。
const MAX_RESOLVED_BYTES: usize = 64 * 1024 * 1024;
/// 段落アキュムレータの強制 flush 上限（brace 不均衡なソースで肥大しないように）。
const MAX_PARAGRAPH_CHARS: usize = 40_000;

/// display 数式として認識する環境（`*` 付きは別名で列挙）。
const DISPLAY_ENVS: &[&str] = &[
    "equation", "equation*", "align", "align*", "alignat", "alignat*", "flalign", "flalign*",
    "gather", "gather*", "multline", "multline*", "eqnarray", "eqnarray*", "displaymath",
    "dmath", "dmath*",
];
/// 中身を字句解釈しない環境（→ `code_block`）。
const VERBATIM_ENVS: &[&str] = &[
    "verbatim", "verbatim*", "lstlisting", "Verbatim", "BVerbatim", "LVerbatim", "minted",
    "alltt",
];
/// 中身を丸ごと捨てる環境（機械ノイズになるだけのもの）。
const DROP_ENVS: &[&str] = &[
    "tikzpicture", "pgfpicture", "picture", "tabular", "tabular*", "tabularx", "tabu",
];
/// `\caption` だけ取り出して残りは捨てる環境（図・表）。
const FIGURE_ENVS: &[&str] = &["figure", "figure*", "wrapfigure", "sidewaysfigure"];
const TABLE_ENVS: &[&str] = &["table", "table*", "sidewaystable", "longtable"];
/// マーカーだけ剥がして中身を通常解析する環境。
const TRANSPARENT_ENVS: &[&str] = &[
    "center", "flushleft", "flushright", "quote", "quotation", "verse", "widetext",
    "subequations", "acknowledgments", "acknowledgements", "minipage", "sloppypar",
    "singlespace", "doublespace", "spacing", "appendices", "appendix",
];
const LIST_ENVS: &[&str] = &["itemize", "enumerate", "description"];

/// 認識済みブロック 1 個。`ingestion::mod` が `document_nodes`（+ `math_expressions`）へ落とす。
#[derive(Debug)]
pub struct TexBlock {
    pub kind: NodeKind,
    /// 読み出し用テキスト（LaTeX コマンド温存・空白正規化済み）。
    pub text: String,
    /// display_math のみ: 原文スニペット（`\begin{..}..\end{..}` / `\[..\]` / `$$..$$` 丸ごと）。
    pub latex: Option<String>,
    /// display_math のみ: `\tag{X}` → `"(X)"`。
    pub equation_label: Option<String>,
    /// `\label{..}` 名（数式・見出し）。
    pub labels: Vec<String>,
    /// 見出しのみ: カウンタ再現の節番号（`"2"` / `"2.1"` / appendix は `"A"`）。
    pub section_number: Option<String>,
    /// 見出しのみ: 1=section / 2=subsection / 3=subsubsection / 4=paragraph。
    pub heading_level: Option<i64>,
    /// bibliography_entry のみ: `\bibitem{key}` の key。
    pub cite_key: Option<String>,
    /// 定理系のみ: `\begin{theorem}[note]` の付記名（Phase 5）。
    pub note: Option<String>,
    pub confidence: f64,
}

/// TeX ソース 1 件分の抽出結果。
#[derive(Debug)]
pub struct ExtractedTexDocument {
    pub blocks: Vec<TexBlock>,
    pub main_file: String,
    pub source_file_count: usize,
    pub warnings: Vec<String>,
}

/// 添付ファイル（gzip/tar/生 .tex）→ 構造認識まで。同期・CPU 依存なので `spawn_blocking` 下で。
pub fn extract_document(path: &Path) -> Result<ExtractedTexDocument, String> {
    let files = source::load_tex_source(path)?;
    extract_from_files(files)
}

/// 展開済みファイル群からの抽出（テスト・分割の要）。
pub fn extract_from_files(src: source::TexSourceFiles) -> Result<ExtractedTexDocument, String> {
    let mut warnings = src.warnings;
    let stripped: BTreeMap<String, String> = src
        .files
        .iter()
        .map(|(k, v)| (k.clone(), strip_comments(v)))
        .collect();

    let main = find_main_file(&stripped, &mut warnings)?;
    let mut visited = BTreeSet::new();
    let mut resolved = String::new();
    resolve_file(&main, &stripped, &mut visited, &mut resolved, &mut warnings)?;

    let mut parser = Parser::new(&resolved);
    parser.run();
    warnings.extend(parser.warnings);

    Ok(ExtractedTexDocument {
        blocks: parser.out,
        main_file: main,
        source_file_count: src.files.len(),
        warnings,
    })
}

// ─── 字句ヘルパ（バックスラッシュ偶奇パリティ） ─────────────────────────────

/// `i` の直前に連続するバックスラッシュの個数。
fn backslash_run_before(s: &str, i: usize) -> usize {
    s.as_bytes()[..i].iter().rev().take_while(|&&b| b == b'\\').count()
}

/// `i` のバイトがエスケープされていない（= 直前のバックスラッシュ連が偶数個）か。
fn unescaped(s: &str, i: usize) -> bool {
    backslash_run_before(s, i).is_multiple_of(2)
}

/// `\` の直後（`i`）から control word（英字列）を読む。非英字なら空。
fn read_control_word(s: &str, i: usize) -> (&str, usize) {
    let bytes = s.as_bytes();
    let mut j = i;
    while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
        j += 1;
    }
    (&s[i..j], j)
}

/// 空白（改行含む）をスキップ。
fn skip_ws(s: &str, mut i: usize) -> usize {
    let bytes = s.as_bytes();
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// `s[i]` が `{` のとき、対応する `}` までの中身と次位置を返す（brace はパリティ判定・入れ子対応）。
fn read_group(s: &str, i: usize) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    if i >= bytes.len() || bytes[i] != b'{' {
        return None;
    }
    let mut depth = 0i64;
    let mut j = i;
    while j < bytes.len() {
        match bytes[j] {
            b'{' if unescaped(s, j) => depth += 1,
            b'}' if unescaped(s, j) => {
                depth -= 1;
                if depth == 0 {
                    return Some((&s[i + 1..j], j + 1));
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// 空白を飛ばして光学引数 `[..]` を 1 個消費する（`{..}` 内の `]` では閉じない・改行可）。
/// 無ければ位置は進めない。revtex の `\bibitem[{\citenamefont{..}}]{key}` 対応。
fn skip_optional(s: &str, i: usize) -> usize {
    let j = skip_ws(s, i);
    let bytes = s.as_bytes();
    if j >= bytes.len() || bytes[j] != b'[' {
        return i;
    }
    let mut brace = 0i64;
    let mut k = j + 1;
    while k < bytes.len() {
        match bytes[k] {
            b'{' if unescaped(s, k) => brace += 1,
            b'}' if unescaped(s, k) => brace -= 1,
            b']' if unescaped(s, k) && brace <= 0 => return k + 1,
            _ => {}
        }
        k += 1;
    }
    i
}

/// 空白 + 光学引数を飛ばして必須の `{..}` を読む共有引数リーダ。
fn read_command_arg(s: &str, i: usize) -> Option<(&str, usize)> {
    let j = skip_optional(s, i);
    let j = skip_ws(s, j);
    read_group(s, j)
}

/// `\\` / `\item` 向けの光学引数スキップ: **空行（段落区切り）をまたがない**。
/// TeX の \@ifnextchar は \par で先読みを止めるため、`\\` の次段落が `[1] ...` で始まる
/// 場合にそれを寸法引数として食べてはいけない。
fn skip_optional_inline(s: &str, i: usize) -> usize {
    let j = skip_ws(s, i);
    if s[i..j].bytes().filter(|&b| b == b'\n').count() >= 2 {
        return i;
    }
    skip_optional(s, i)
}

/// `from` 以降で最初にパリティ条件を満たす `pat`（`\]` や `$$` 等）の開始位置。
fn find_unescaped(s: &str, from: usize, pat: &str) -> Option<usize> {
    let mut i = from;
    while let Some(rel) = s[i..].find(pat) {
        let pos = i + rel;
        if unescaped(s, pos) {
            return Some(pos);
        }
        i = pos + 1;
    }
    None
}

// ─── コメント除去 ────────────────────────────────────────────────────────────

/// 行内で `\url{..}` / `\href{..}` / `\path{..}` / `\nolinkurl{..}` / `\verb<d>..<d>` が占める
/// バイト範囲（`%` をコメントとみなさない保護区間）。
fn protected_ranges(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = line.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' && unescaped(line, i) {
            let (word, after) = read_control_word(line, i + 1);
            match word {
                "url" | "path" | "nolinkurl" | "href" => {
                    let j = skip_ws(line, after);
                    if let Some((_, end)) = read_group(line, j) {
                        out.push((i, end));
                        i = end;
                        continue;
                    }
                }
                "verb" => {
                    // \verb* も可。直後の 1 文字（マルチバイト可）が区切り。
                    let mut j = after;
                    if bytes.get(j) == Some(&b'*') {
                        j += 1;
                    }
                    if let Some(delim) = line[j..].chars().next() {
                        let dstart = j + delim.len_utf8();
                        if let Some(rel) = line[dstart..].find(delim) {
                            let end = dstart + rel + delim.len_utf8();
                            out.push((i, end));
                            i = end;
                            continue;
                        }
                    }
                }
                _ => {}
            }
            i += 1 + word.len().max(1);
            continue;
        }
        i += 1;
    }
    out
}

/// コメント除去（verbatim 環境内は不変）。
///
/// - `%` はパリティ条件（直前のバックスラッシュ連が偶数）かつ保護区間外のときだけコメント開始。
/// - コメントを取り除いた結果**行全体が空**になるなら行ごと削除する（コメント専用行が
///   空行 = 段落区切りを偽造しないため）。行の途中からのコメントは改行を残す
///   （TeX の行末 `%` による連結までは再現しない — 既知の妥協）。
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_verbatim: Option<String> = None;
    for line in text.lines() {
        if let Some(env) = &in_verbatim {
            out.push_str(line);
            out.push('\n');
            if line.contains(&format!("\\end{{{env}}}")) {
                in_verbatim = None;
            }
            continue;
        }

        let protected = protected_ranges(line);
        let mut cut: Option<usize> = None;
        for (i, b) in line.bytes().enumerate() {
            if b == b'%'
                && unescaped(line, i)
                && !protected.iter().any(|&(s, e)| i >= s && i < e)
            {
                cut = Some(i);
                break;
            }
        }

        // verbatim 開始は**コメント開始より前**にあるときだけ本物（`% TODO \begin{verbatim}` の
        // ようなコメント内マーカーで verbatim モードに入ると、以降の本文が丸ごと素通し・
        // 最悪 \end 不在で文書後半が消えるため）。
        let effective_end = cut.unwrap_or(line.len());
        if let Some(env) = VERBATIM_ENVS.iter().find(|e| {
            line.find(&format!("\\begin{{{e}}}"))
                .is_some_and(|p| p < effective_end && unescaped(line, p))
        }) {
            // 同一行で閉じていなければ verbatim モードへ。開始行は % も内容なので行全体を残す。
            if !line.contains(&format!("\\end{{{env}}}")) {
                in_verbatim = Some(env.to_string());
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }

        match cut {
            Some(p) => {
                let kept = &line[..p];
                if kept.trim().is_empty() {
                    // コメント専用行 → 行ごと削除（空行を作らない）。
                    continue;
                }
                out.push_str(kept);
                out.push('\n');
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

// ─── main ファイル検出・\input 解決 ──────────────────────────────────────────

/// `\documentclass` の最初のクラス名（無ければ None）。
fn documentclass_of(stripped: &str) -> Option<String> {
    let mut i = 0;
    while let Some(rel) = stripped[i..].find("\\documentclass") {
        let pos = i + rel;
        if unescaped(stripped, pos) {
            let after = pos + "\\documentclass".len();
            if let Some((class, _)) = read_command_arg(stripped, after) {
                return Some(class.trim().to_string());
            }
            return None;
        }
        i = pos + 1;
    }
    None
}

/// 他ファイルから `\input`/`\include`/`\subfile`/`\import` されるファイル名（正規化済み）を集める。
fn referenced_files(stripped: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for content in stripped.values() {
        scan_input_commands(content, |target| {
            for cand in candidate_names(target) {
                if stripped.contains_key(&cand) {
                    refs.insert(cand);
                }
            }
        });
    }
    refs
}

/// `\input`/`\include`/`\subfile`/`\import` の参照先を列挙して `f` に渡す（解決はしない）。
fn scan_input_commands(s: &str, mut f: impl FnMut(&str)) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && unescaped(s, i) {
            let (word, after) = read_control_word(s, i + 1);
            match word {
                "input" | "include" | "subfile" => {
                    let j = skip_ws(s, after);
                    if let Some((name, end)) = read_group(s, j) {
                        f(name.trim());
                        i = end;
                        continue;
                    }
                    // plain TeX の braceless 形式: `\input macros.tex `
                    if word == "input" {
                        if let Some((name, end)) = read_braceless_name(s, j) {
                            f(name);
                            i = end;
                            continue;
                        }
                    }
                }
                "import" => {
                    let j = skip_ws(s, after);
                    if let Some((dir, k)) = read_group(s, j) {
                        let k = skip_ws(s, k);
                        if let Some((file, end)) = read_group(s, k) {
                            let joined = format!("{}{}", dir.trim(), file.trim());
                            f(&joined);
                            i = end;
                            continue;
                        }
                    }
                }
                _ => {}
            }
            i = after.max(i + 1);
            continue;
        }
        i += 1;
    }
}

/// braceless `\input name` のファイル名（空白・`\`・`{` で終端）。
fn read_braceless_name(s: &str, i: usize) -> Option<(&str, usize)> {
    let bytes = s.as_bytes();
    let mut j = i;
    while j < bytes.len()
        && !bytes[j].is_ascii_whitespace()
        && bytes[j] != b'\\'
        && bytes[j] != b'{'
        && bytes[j] != b'}'
        && bytes[j] != b'%'
    {
        j += 1;
    }
    if j > i {
        Some((&s[i..j], j))
    } else {
        None
    }
}

/// 参照名 → 実ファイル名の候補（`.tex`/`.ltx` 補完込み・正規化）。
fn candidate_names(target: &str) -> Vec<String> {
    let base = source::normalize_path(target);
    let mut v = vec![base.clone()];
    let has_ext = base.rsplit('/').next().is_some_and(|f| f.contains('.'));
    if !has_ext {
        v.push(format!("{base}.tex"));
        v.push(format!("{base}.ltx"));
    }
    v
}

/// main ファイルを選ぶ。コメント除去済みテキストで判定する。
///
/// 1. `\documentclass`/`\documentstyle` を持つ `.tex`/`.ltx`（standalone/subfiles クラスは除外）。
/// 2. 「他ファイルから参照されていない」→「`\begin{document}` を持つ」→「`\title`/abstract を
///    持つ」→「サイズ大」の順で優先し、最後はパス辞書順。
/// 3. 候補ゼロなら plain TeX 救済: TeX らしさスニッフを通る最大のファイル + warning
///    （旧 hep-th の harvmac/plain TeX 対応）。
fn find_main_file(
    stripped: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<String, String> {
    let refs = referenced_files(stripped);
    let tex_files: Vec<&String> = stripped
        .keys()
        .filter(|k| {
            let l = k.to_ascii_lowercase();
            l.ends_with(".tex") || l.ends_with(".ltx")
        })
        .collect();

    let mut candidates: Vec<(&String, &String)> = Vec::new();
    for name in &tex_files {
        let content = &stripped[*name];
        let has_class = documentclass_of(content).is_some();
        let has_style = find_unescaped(content, 0, "\\documentstyle").is_some();
        if !(has_class || has_style) {
            continue;
        }
        if let Some(class) = documentclass_of(content) {
            if class == "standalone" || class == "subfiles" {
                continue;
            }
        }
        candidates.push((name, content));
    }

    if !candidates.is_empty() {
        candidates.sort_by(|(an, ac), (bn, bc)| {
            let score = |name: &str, c: &str| {
                (
                    !refs.contains(name),
                    find_unescaped(c, 0, "\\begin{document}").is_some(),
                    find_unescaped(c, 0, "\\title").is_some()
                        || find_unescaped(c, 0, "\\begin{abstract}").is_some()
                        || find_unescaped(c, 0, "\\abstract{").is_some(),
                    c.len(),
                )
            };
            score(bn, bc)
                .cmp(&score(an, ac))
                .then_with(|| an.cmp(bn))
        });
        return Ok(candidates[0].0.clone());
    }

    // plain TeX 救済（\documentclass 無し）。
    let sniff = |c: &str| {
        ["\\section", "\\input", "$$", "\\halign", "\\centerline", "\\title", "\\bye", "\\chapter"]
            .iter()
            .any(|m| c.contains(m))
    };
    let mut fallback: Vec<&String> = tex_files
        .iter()
        .filter(|k| !refs.contains(k.as_str()) && sniff(&stripped[**k]))
        .copied()
        .collect();
    fallback.sort_by(|a, b| {
        stripped[*b]
            .len()
            .cmp(&stripped[*a].len())
            .then_with(|| a.cmp(b))
    });
    if let Some(name) = fallback.first() {
        warnings.push(format!(
            "no \\documentclass found; treating '{name}' as a plain-TeX body"
        ));
        return Ok((*name).clone());
    }
    Err("no usable TeX main file found in the source".to_string())
}

/// `name` のコメント除去済み本文を `out` へ書き出しつつ、`\input` 系と `\bibliography` を
/// 再帰スプライスする。include-once（同一ファイルは 1 回だけ）+ 総量上限で増幅を抑える。
fn resolve_file(
    name: &str,
    stripped: &BTreeMap<String, String>,
    visited: &mut BTreeSet<String>,
    out: &mut String,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    if !visited.insert(name.to_string()) {
        warnings.push(format!("skipped repeated \\input of '{name}'"));
        return Ok(());
    }
    let s = &stripped[name];
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut in_verbatim: Option<String> = None;

    while i < bytes.len() {
        if out.len() > MAX_RESOLVED_BYTES {
            return Err("resolved TeX source exceeds the size limit".to_string());
        }
        // verbatim 内は素通し（\input を解釈しない）。
        if let Some(env) = &in_verbatim {
            let end_marker = format!("\\end{{{env}}}");
            if let Some(rel) = s[i..].find(&end_marker) {
                let end = i + rel + end_marker.len();
                out.push_str(&s[i..end]);
                i = end;
                in_verbatim = None;
            } else {
                out.push_str(&s[i..]);
                break;
            }
            continue;
        }
        if bytes[i] == b'\\' && unescaped(s, i) {
            let (word, after) = read_control_word(s, i + 1);
            if word == "begin" {
                if let Some((env, end)) = read_command_arg(s, after) {
                    let env = env.trim().to_string();
                    out.push_str(&s[i..end]);
                    i = end;
                    if VERBATIM_ENVS.contains(&env.as_str()) {
                        in_verbatim = Some(env);
                    }
                    continue;
                }
            }
            let mut splice = |target: &str, out: &mut String, warnings: &mut Vec<String>| {
                let mut found = None;
                for cand in candidate_names(target) {
                    if stripped.contains_key(&cand) {
                        found = Some(cand);
                        break;
                    }
                }
                match found {
                    Some(child) => {
                        // 自前の \documentclass を持つファイル（standalone 図等）は差し込まない。
                        if documentclass_of(&stripped[&child]).is_some()
                            || find_unescaped(&stripped[&child], 0, "\\documentstyle").is_some()
                        {
                            warnings.push(format!(
                                "skipped \\input of '{child}' (it has its own \\documentclass)"
                            ));
                            Ok(())
                        } else {
                            resolve_file(&child, stripped, visited, out, warnings)
                        }
                    }
                    None => {
                        warnings.push(format!("unresolved \\input '{target}'"));
                        Ok(())
                    }
                }
            };
            match word {
                "input" | "include" | "subfile" => {
                    let j = skip_ws(s, after);
                    if let Some((target, end)) = read_group(s, j) {
                        splice(target.trim(), out, warnings)?;
                        i = end;
                        continue;
                    }
                    if word == "input" {
                        if let Some((target, end)) = read_braceless_name(s, j) {
                            let target = target.to_string();
                            splice(&target, out, warnings)?;
                            i = end;
                            continue;
                        }
                    }
                }
                "import" => {
                    let j = skip_ws(s, after);
                    if let Some((dir, k)) = read_group(s, j) {
                        let k2 = skip_ws(s, k);
                        if let Some((file, end)) = read_group(s, k2) {
                            let joined = format!("{}{}", dir.trim(), file.trim());
                            splice(&joined, out, warnings)?;
                            i = end;
                            continue;
                        }
                    }
                }
                "bibliography" => {
                    let j = skip_ws(s, after);
                    if let Some((_, end)) = read_group(s, j) {
                        match pick_bbl(name, stripped) {
                            Some(bbl) => resolve_file(&bbl, stripped, visited, out, warnings)?,
                            None => warnings.push(
                                "\\bibliography used but no .bbl found in the source (references omitted)"
                                    .to_string(),
                            ),
                        }
                        i = end;
                        continue;
                    }
                }
                _ => {}
            }
            // 上のどれでもない: `\` + 続きをそのまま出力（word 分まとめて進める）。
            // control symbol（`\` + 非英字 1 文字）はマルチバイト文字がありうるので char 長で進める。
            let end = if word.is_empty() {
                i + 1 + s[i + 1..].chars().next().map_or(0, |c| c.len_utf8())
            } else {
                after
            };
            out.push_str(&s[i..end]);
            i = end;
            continue;
        }
        // 1 文字コピー（UTF-8 境界を守る）。
        let ch_len = s[i..].chars().next().map_or(1, |c| c.len_utf8());
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    Ok(())
}

/// `\bibliography{..}` に対応する `.bbl` を選ぶ: main と同 stem → 唯一の .bbl → なし。
fn pick_bbl(main: &str, stripped: &BTreeMap<String, String>) -> Option<String> {
    let stem = main.strip_suffix(".tex").or_else(|| main.strip_suffix(".ltx")).unwrap_or(main);
    let preferred = format!("{stem}.bbl");
    if stripped.contains_key(&preferred) {
        return Some(preferred);
    }
    let bbls: Vec<&String> = stripped
        .keys()
        .filter(|k| k.to_ascii_lowercase().ends_with(".bbl"))
        .collect();
    if bbls.len() == 1 {
        return Some(bbls[0].clone());
    }
    bbls.first().map(|s| (*s).clone())
}

// ─── 本文走査 ────────────────────────────────────────────────────────────────

/// 節番号カウンタ。`\appendix` 後は第 1 レベルが A, B, … になる。
#[derive(Default)]
struct SectionCounters {
    sec: i64,
    sub: i64,
    subsub: i64,
    appendix: bool,
}

impl SectionCounters {
    fn top_label(&self) -> Option<String> {
        if self.appendix {
            // A..Z を超えたら誤番号を出すより欠番にする。
            if (1..=26).contains(&self.sec) {
                Some(((b'A' + (self.sec - 1) as u8) as char).to_string())
            } else {
                None
            }
        } else {
            Some(self.sec.to_string())
        }
    }
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
    out: Vec<TexBlock>,
    para: String,
    para_depth: i64,
    counters: SectionCounters,
    warnings: Vec<String>,
    title_emitted: bool,
    unknown_envs: BTreeSet<String>,
    /// preamble の自明マクロ（`\be` → `equation` 等）。
    begin_aliases: BTreeMap<String, String>,
    end_aliases: BTreeMap<String, String>,
    /// 定理系環境名 → ノード種別（Phase 5）。標準名 + `\newtheorem` で宣言された独自名。
    theorem_envs: BTreeMap<String, NodeKind>,
}

impl<'a> Parser<'a> {
    fn new(resolved: &'a str) -> Self {
        Parser {
            s: resolved,
            i: 0,
            out: Vec::new(),
            para: String::new(),
            para_depth: 0,
            counters: SectionCounters::default(),
            warnings: Vec::new(),
            title_emitted: false,
            unknown_envs: BTreeSet::new(),
            begin_aliases: BTreeMap::new(),
            end_aliases: BTreeMap::new(),
            theorem_envs: default_theorem_envs(),
        }
    }

    fn run(&mut self) {
        // preamble / 本文の分割。無ければ全体を本文として扱う（plain TeX / 断片）。
        let body_start = match find_unescaped(self.s, 0, "\\begin{document}") {
            Some(pos) => {
                let preamble = &self.s[..pos];
                self.collect_aliases(preamble);
                self.scan_front_matter(preamble);
                pos + "\\begin{document}".len()
            }
            None => {
                self.warnings
                    .push("no \\begin{document}; parsing the whole file as body".to_string());
                0
            }
        };
        self.i = body_start;
        self.scan_body();
        self.flush_paragraph();
        if !self.unknown_envs.is_empty() {
            let names: Vec<&str> = self.unknown_envs.iter().map(|s| s.as_str()).collect();
            self.warnings.push(format!(
                "unrecognized environments treated as transparent: {}",
                names.join(", ")
            ));
        }
    }

    /// preamble の `\title` / `\abstract{..}`（jheppub コマンド形）を front_matter/abstract に。
    fn scan_front_matter(&mut self, preamble: &str) {
        if let Some(pos) = find_control_word(preamble, 0, "title") {
            if let Some((title, _)) = read_command_arg(preamble, pos) {
                self.emit_front_matter(title);
            }
        }
        if let Some(pos) = find_control_word(preamble, 0, "abstract") {
            let j = skip_ws(preamble, pos);
            if let Some((text, _)) = read_group(preamble, j) {
                self.push_block(TexBlock {
                    kind: NodeKind::Abstract,
                    text: collapse_ws(text),
                    latex: None,
                    equation_label: None,
                    labels: Vec::new(),
                    section_number: None,
                    heading_level: None,
                    cite_key: None,
                    note: None,
                    confidence: 0.95,
                });
            }
        }
    }

    /// preamble の自明な数式エイリアス（`\def\be{\begin{equation}}` /
    /// `\newcommand{\be}{\begin{equation}}`）を回収する。パラメータ付き定義は対象外。
    fn collect_aliases(&mut self, preamble: &str) {
        let bytes = preamble.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && unescaped(preamble, i) {
                let (word, after) = read_control_word(preamble, i + 1);
                // `\newtheorem{env}{Display}`（Phase 5）: 環境名を表示名からノード種別へ対応づける。
                if word == "newtheorem" {
                    i = self.collect_newtheorem(preamble, after);
                    continue;
                }
                let name_and_body = match word {
                    "def" => {
                        // \def\be{...}
                        let j = skip_ws(preamble, after);
                        if j < bytes.len() && bytes[j] == b'\\' {
                            let (name, k) = read_control_word(preamble, j + 1);
                            let k = skip_ws(preamble, k);
                            read_group(preamble, k).map(|(body, end)| (name.to_string(), body, end))
                        } else {
                            None
                        }
                    }
                    "newcommand" | "renewcommand" | "providecommand" => {
                        // \newcommand{\be}{...} / \newcommand\be{...}（[n] 付きは対象外）
                        let j = skip_ws(preamble, after);
                        let (name, k) = if j < bytes.len() && bytes[j] == b'{' {
                            match read_group(preamble, j) {
                                Some((g, k)) => {
                                    let g = g.trim();
                                    match g.strip_prefix('\\') {
                                        Some(n) if n.chars().all(|c| c.is_ascii_alphabetic()) => {
                                            (n.to_string(), k)
                                        }
                                        _ => {
                                            i = k;
                                            continue;
                                        }
                                    }
                                }
                                None => {
                                    i = after;
                                    continue;
                                }
                            }
                        } else if j < bytes.len() && bytes[j] == b'\\' {
                            let (n, k) = read_control_word(preamble, j + 1);
                            (n.to_string(), k)
                        } else {
                            i = after;
                            continue;
                        };
                        let k2 = skip_ws(preamble, k);
                        if k2 < bytes.len() && bytes[k2] == b'[' {
                            // パラメータ付きは自明エイリアスでない。
                            i = k2;
                            continue;
                        }
                        read_group(preamble, k2).map(|(body, end)| (name, body, end))
                    }
                    _ => None,
                };
                if let Some((name, body, end)) = name_and_body {
                    let body = body.trim();
                    if let Some(env) = body
                        .strip_prefix("\\begin{")
                        .and_then(|r| r.strip_suffix('}'))
                    {
                        if DISPLAY_ENVS.contains(&env) {
                            self.begin_aliases.insert(name.clone(), env.to_string());
                        }
                    } else if let Some(env) =
                        body.strip_prefix("\\end{").and_then(|r| r.strip_suffix('}'))
                    {
                        if DISPLAY_ENVS.contains(&env) {
                            self.end_aliases.insert(name.clone(), env.to_string());
                        }
                    }
                    i = end;
                    continue;
                }
                i = after.max(i + 1);
                continue;
            }
            i += 1;
        }
    }

    /// `\newtheorem` の 1 宣言を解析して `theorem_envs` に登録する（`after` = `\newtheorem` の直後）。
    /// 対応形: `\newtheorem{env}{Display}` / `\newtheorem{env}[shared]{Display}` /
    /// `\newtheorem{env}{Display}[within]` / `\newtheorem*{env}{Display}`。返り値は次の走査位置。
    fn collect_newtheorem(&mut self, s: &str, after: usize) -> usize {
        let mut j = after;
        if s.as_bytes().get(j) == Some(&b'*') {
            j += 1;
        }
        let j = skip_ws(s, j);
        let Some((env, k)) = read_group(s, j) else {
            return after.max(j);
        };
        // 表示名は「env の次の必須 `{..}`」。共有カウンタ `[shared]` を挟む形にも対応する。
        let k = skip_optional(s, k);
        let k = skip_ws(s, k);
        let Some((display, end)) = read_group(s, k) else {
            return k;
        };
        if let Some(kind) = theorem_kind_from_title(display) {
            self.theorem_envs.insert(env.trim().to_string(), kind);
        }
        end
    }

    // ── 出力ヘルパ ──

    fn push_block(&mut self, b: TexBlock) {
        self.out.push(b);
    }

    fn emit_front_matter(&mut self, title_raw: &str) {
        if self.title_emitted {
            return;
        }
        let title = collapse_ws(&strip_command(title_raw, "thanks"));
        if title.is_empty() {
            return;
        }
        self.title_emitted = true;
        self.push_block(TexBlock {
            kind: NodeKind::FrontMatter,
            text: title,
            latex: None,
            equation_label: None,
            labels: Vec::new(),
            section_number: None,
            heading_level: None,
            cite_key: None,
            note: None,
            confidence: 0.9,
        });
    }

    fn flush_paragraph(&mut self) {
        let text = collapse_ws(&self.para);
        self.para.clear();
        self.para_depth = 0;
        if text.is_empty() {
            return;
        }
        self.push_block(TexBlock {
            kind: NodeKind::Paragraph,
            text,
            latex: None,
            equation_label: None,
            labels: Vec::new(),
            section_number: None,
            heading_level: None,
            cite_key: None,
            note: None,
            confidence: 0.9,
        });
    }

    fn push_para(&mut self, piece: &str) {
        if self.para.len() > MAX_PARAGRAPH_CHARS {
            self.warnings
                .push("oversized paragraph force-flushed (unbalanced braces?)".to_string());
            self.flush_paragraph();
        }
        self.para.push_str(piece);
    }

    // ── 本文スキャナ ──

    fn scan_body(&mut self) {
        while self.i < self.s.len() {
            let b = self.s.as_bytes()[self.i];
            match b {
                b'\\' => self.on_backslash(),
                b'$' => self.on_dollar(),
                b'{' => {
                    self.para_depth += 1;
                    self.push_para("{");
                    self.i += 1;
                }
                b'}' => {
                    self.para_depth = (self.para_depth - 1).max(0);
                    self.push_para("}");
                    self.i += 1;
                }
                b'\n' => {
                    // 空行（次の非空白まで進んで改行を 2 つ以上跨ぐ）なら段落区切り。
                    let mut j = self.i + 1;
                    let mut newlines = 1;
                    while j < self.s.len() {
                        let c = self.s.as_bytes()[j];
                        if c == b'\n' {
                            newlines += 1;
                            j += 1;
                        } else if c.is_ascii_whitespace() {
                            j += 1;
                        } else {
                            break;
                        }
                    }
                    if newlines >= 2 && self.para_depth == 0 {
                        self.flush_paragraph();
                    } else {
                        self.push_para(" ");
                    }
                    self.i = j;
                }
                b'~' => {
                    self.push_para(" ");
                    self.i += 1;
                }
                _ => {
                    let ch_len = self.s[self.i..].chars().next().map_or(1, |c| c.len_utf8());
                    let piece = &self.s[self.i..self.i + ch_len];
                    self.i += ch_len;
                    self.push_para(piece);
                }
            }
        }
    }

    /// `\` から始まるトークンの処理。
    fn on_backslash(&mut self) {
        let s = self.s;
        let (word, after) = read_control_word(s, self.i + 1);
        if word.is_empty() {
            // control symbol。
            let sym = s.as_bytes().get(self.i + 1).copied();
            match sym {
                Some(b'\\') => {
                    // 改行 `\\`（+ 任意の `[2pt]`）→ 空白。display 数式 `\[` と区別する要。
                    // 空行をまたいだ `[..]` は引数ではない（次段落の本文）。
                    let j = skip_optional_inline(s, self.i + 2);
                    self.push_para(" ");
                    self.i = j.max(self.i + 2);
                }
                Some(b'[') => {
                    // display 数式 \[ .. \]
                    self.flush_paragraph();
                    let start = self.i;
                    let content_start = self.i + 2;
                    match find_unescaped(s, content_start, "\\]") {
                        Some(end) => {
                            let snippet = &s[start..end + 2];
                            let inner = &s[content_start..end];
                            self.emit_math(snippet, inner);
                            self.i = end + 2;
                        }
                        None => {
                            self.warnings.push("unterminated \\[ display math".to_string());
                            self.i = content_start;
                        }
                    }
                }
                Some(b'(') => {
                    // インライン数式 \( .. \) は段落テキストにそのまま残す。
                    let end = find_unescaped(s, self.i + 2, "\\)")
                        .map(|e| e + 2)
                        .unwrap_or(s.len());
                    let piece = &s[self.i..end];
                    self.i = end;
                    self.push_para(piece);
                }
                Some(_) => {
                    // エスケープ文字（\% \& \_ \$ など）は原文のまま段落へ。
                    let ch_len = s[self.i + 1..].chars().next().map_or(1, |c| c.len_utf8());
                    let piece = &s[self.i..self.i + 1 + ch_len];
                    self.i += 1 + ch_len;
                    self.push_para(piece);
                }
                None => {
                    self.push_para("\\");
                    self.i += 1;
                }
            }
            return;
        }

        // preamble 由来の数式エイリアス（\be .. \ee）。
        if let Some(env) = self.begin_aliases.get(word).cloned() {
            let alias_start = self.i;
            self.flush_paragraph();
            self.consume_alias_math(word, &env, alias_start, after);
            return;
        }

        match word {
            "begin" => {
                // `\begin {equation}` のような空白入りでも snippet を正確に切り出せるよう、
                // マーカー開始位置（`\` の位置）を渡す（文字列 rfind での再構成はしない）。
                let marker_start = self.i;
                if let Some((env, end)) = read_command_arg(s, after) {
                    let env = env.trim().to_string();
                    self.i = end;
                    self.on_env(&env, marker_start);
                } else {
                    self.push_para("\\begin");
                    self.i = after;
                }
            }
            "end" => {
                if let Some((env, end)) = read_command_arg(s, after) {
                    let env = env.trim();
                    self.i = end;
                    if env == "document" {
                        self.flush_paragraph();
                        self.i = s.len();
                    } else {
                        // 透過/未知環境の閉じ。ブロック境界として flush だけする。
                        self.flush_paragraph();
                    }
                } else {
                    self.push_para("\\end");
                    self.i = after;
                }
            }
            "section" | "subsection" | "subsubsection" | "chapter" => {
                self.on_heading(word.to_string(), after);
            }
            "paragraph" | "subparagraph" => {
                self.on_heading(word.to_string(), after);
            }
            "title" => {
                if let Some((t, end)) = read_command_arg(s, after) {
                    self.flush_paragraph();
                    let t = t.to_string();
                    self.emit_front_matter(&t);
                    self.i = end;
                } else {
                    self.i = after;
                }
            }
            "abstract" => {
                // jheppub 等のコマンド形 \abstract{..}（環境形は on_env が処理）。
                let j = skip_ws(s, after);
                if let Some((text, end)) = read_group(s, j) {
                    self.flush_paragraph();
                    let text = collapse_ws(text);
                    self.push_block(TexBlock {
                        kind: NodeKind::Abstract,
                        text,
                        latex: None,
                        equation_label: None,
                        labels: Vec::new(),
                        section_number: None,
                        heading_level: None,
                        cite_key: None,
                        note: None,
                        confidence: 0.95,
                    });
                    self.i = end;
                } else {
                    self.push_para("\\abstract");
                    self.i = after;
                }
            }
            "appendix" => {
                self.flush_paragraph();
                self.counters.appendix = true;
                self.counters.sec = 0;
                self.counters.sub = 0;
                self.counters.subsub = 0;
                // revtex の \appendix* も同じ扱い（番号は付かない方向に倒れる）。
                let j = self.i.max(after);
                self.i = if s.as_bytes().get(after) == Some(&b'*') { after + 1 } else { j };
            }
            "par" => {
                if self.para_depth == 0 {
                    self.flush_paragraph();
                } else {
                    self.push_para(" ");
                }
                self.i = after;
            }
            "item" => {
                // リスト環境外の迷子 \item（描画上は箇条書き）。空行越しの `[..]` は食べない。
                let j = skip_optional_inline(s, after);
                self.push_para(" • ");
                self.i = j;
            }
            "verb" => {
                // \verb<d>..<d> は中身を字句解釈せず原文のまま段落へ（`$` や `%` を含みうる —
                // 放置すると on_dollar が対応を崩し、以降の構造を丸ごと飲み込む）。
                let mut j = after;
                if s.as_bytes().get(j) == Some(&b'*') {
                    j += 1;
                }
                match s[j..].chars().next() {
                    Some(delim) if delim != '\n' => {
                        let dstart = j + delim.len_utf8();
                        match s[dstart..].find(delim) {
                            Some(rel) => {
                                let end = dstart + rel + delim.len_utf8();
                                let piece = &s[self.i..end];
                                self.i = end;
                                self.push_para(piece);
                            }
                            None => {
                                let piece = &s[self.i..j];
                                self.i = j;
                                self.push_para(piece);
                            }
                        }
                    }
                    _ => {
                        self.push_para("\\verb");
                        self.i = j;
                    }
                }
            }
            "url" | "path" | "nolinkurl" => {
                // URL は `%`/`$`/`~`/`#` を含みうるので引数ごと原文のまま段落へ。
                let j = skip_ws(s, after);
                match read_group(s, j) {
                    Some((_, end)) => {
                        let piece = &s[self.i..end];
                        self.i = end;
                        self.push_para(piece);
                    }
                    None => {
                        let piece = &s[self.i..after];
                        self.i = after;
                        self.push_para(piece);
                    }
                }
            }
            "href" => {
                // \href{url}{text}: 両群を原文のまま段落へ。
                let j = skip_ws(s, after);
                let end = read_group(s, j).map(|(_, k)| {
                    let k2 = skip_ws(s, k);
                    read_group(s, k2).map(|(_, e)| e).unwrap_or(k)
                });
                match end {
                    Some(end) => {
                        let piece = &s[self.i..end];
                        self.i = end;
                        self.push_para(piece);
                    }
                    None => {
                        let piece = &s[self.i..after];
                        self.i = after;
                        self.push_para(piece);
                    }
                }
            }
            "label" => {
                // 段落中の \label は本文ノイズになるので落とす（数式・見出しは個別に回収）。
                let j = skip_ws(s, after);
                self.i = read_group(s, j).map(|(_, e)| e).unwrap_or(after);
            }
            // 前付け・整形系: 引数ごと読み飛ばす。
            "maketitle" | "tableofcontents" | "newpage" | "clearpage" | "cleardoublepage"
            | "bigskip" | "medskip" | "smallskip" | "noindent" | "indent" | "centering"
            | "raggedright" | "raggedbottom" | "onecolumngrid" | "twocolumngrid" | "linebreak"
            | "pagebreak" | "sloppy" | "printbibliography" => {
                if word == "printbibliography" {
                    self.warnings.push(
                        "biblatex \\printbibliography is not expanded (references omitted)"
                            .to_string(),
                    );
                }
                self.i = after;
            }
            "author" | "affiliation" | "altaffiliation" | "address" | "email" | "date"
            | "thanks" | "preprint" | "pacs" | "keywords" | "bibliographystyle" | "vspace"
            | "hspace" | "collaboration" => {
                // 任意の * と光学引数 + 必須引数 1 個を消費。
                let mut j = after;
                if s.as_bytes().get(j) == Some(&b'*') {
                    j += 1;
                }
                let j2 = skip_optional(s, j);
                let j3 = skip_ws(s, j2);
                self.i = read_group(s, j3).map(|(_, e)| e).unwrap_or(j2.max(j));
            }
            "setcounter" | "renewcommand" | "newcommand" | "providecommand" | "numberwithin" => {
                // 本文中の定義系: {..}{..}（+ 光学引数）を消費。
                let mut j = skip_ws(s, after);
                if s.as_bytes().get(j) == Some(&b'*') {
                    j = skip_ws(s, j + 1);
                }
                for _ in 0..2 {
                    if j < s.len() && s.as_bytes()[j] == b'\\' {
                        let (_, k) = read_control_word(s, j + 1);
                        j = k;
                    } else if let Some((_, k)) = read_group(s, j) {
                        j = k;
                    }
                    j = skip_optional(s, j);
                    j = skip_ws(s, j);
                }
                self.i = j;
            }
            "input" | "include" | "subfile" | "import" | "bibliography" => {
                // 解決フェーズで残ったもの（未解決）は引数ごと読み飛ばす。
                let j = skip_ws(s, after);
                self.i = read_group(s, j)
                    .map(|(_, e)| e)
                    .or_else(|| read_braceless_name(s, j).map(|(_, e)| e))
                    .unwrap_or(after);
            }
            _ => {
                // 未知コマンドは原文のまま段落へ（\cite/\ref/\emph などを含む）。
                let piece = &s[self.i..after];
                self.i = after;
                self.push_para(piece);
            }
        }
    }

    /// `$` / `$$` の処理。`$$..$$` は display、`$..$` は段落内に温存。
    fn on_dollar(&mut self) {
        let s = self.s;
        if s[self.i + 1..].starts_with('$') {
            self.flush_paragraph();
            let start = self.i;
            let content_start = self.i + 2;
            match find_unescaped(s, content_start, "$$") {
                Some(end) => {
                    let snippet = &s[start..end + 2];
                    let inner = &s[content_start..end];
                    self.emit_math(snippet, inner);
                    self.i = end + 2;
                }
                None => {
                    self.warnings.push("unterminated $$ display math".to_string());
                    self.i = content_start;
                }
            }
            return;
        }
        // インライン $..$: 閉じまで原文のまま段落へ。
        match find_unescaped(s, self.i + 1, "$") {
            Some(end) => {
                let piece = &s[self.i..end + 1];
                self.i = end + 1;
                self.push_para(piece);
            }
            None => {
                self.warnings.push("unterminated inline $ math".to_string());
                self.push_para("$");
                self.i += 1;
            }
        }
    }

    /// 見出しコマンド（\section 等）。共有引数リーダで `*` / `[short]` / `{title}` を読む。
    fn on_heading(&mut self, cmd: String, after: usize) {
        let s = self.s;
        let mut j = after;
        let starred = s.as_bytes().get(j) == Some(&b'*');
        if starred {
            j += 1;
        }
        let Some((title, end)) = read_command_arg(s, j) else {
            self.push_para(&format!("\\{cmd}"));
            self.i = after;
            return;
        };
        self.flush_paragraph();
        let title_clean = collapse_ws(title);

        let (kind, level, number) = match cmd.as_str() {
            "section" => {
                let number = if starred {
                    None
                } else {
                    self.counters.sec += 1;
                    self.counters.sub = 0;
                    self.counters.subsub = 0;
                    self.counters.top_label()
                };
                (NodeKind::Section, 1, number)
            }
            "subsection" => {
                let number = if starred {
                    None
                } else {
                    self.counters.sub += 1;
                    self.counters.subsub = 0;
                    self.counters
                        .top_label()
                        .map(|t| format!("{t}.{}", self.counters.sub))
                };
                (NodeKind::Subsection, 2, number)
            }
            "subsubsection" => {
                let number = if starred {
                    None
                } else {
                    self.counters.subsub += 1;
                    self.counters
                        .top_label()
                        .map(|t| format!("{t}.{}.{}", self.counters.sub, self.counters.subsub))
                };
                (NodeKind::Heading, 3, number)
            }
            // \chapter は article 系では稀。番号エミュレーションはせず見出しとして残す。
            "chapter" => (NodeKind::Section, 1, None),
            _ => (NodeKind::Heading, 4, None), // paragraph / subparagraph
        };

        // 見出し直後の \label を回収する。
        let mut labels = Vec::new();
        let mut k = end;
        loop {
            let k2 = skip_ws(s, k);
            if s[k2..].starts_with("\\label") {
                let k3 = skip_ws(s, k2 + "\\label".len());
                match read_group(s, k3) {
                    Some((name, k4)) => {
                        labels.push(name.trim().to_string());
                        k = k4;
                    }
                    None => break,
                }
            } else {
                break;
            }
        }
        self.i = k;

        let text = match &number {
            Some(n) => format!("{n} {title_clean}"),
            None => title_clean,
        };
        self.push_block(TexBlock {
            kind,
            text,
            latex: None,
            equation_label: None,
            labels,
            section_number: number,
            heading_level: Some(level),
            cite_key: None,
            note: None,
            confidence: 0.95,
        });
    }

    /// `\begin{env}` の処理（display 数式・図表・リスト・verbatim・参考文献・透過・破棄）。
    /// `marker_start` = `\begin` の `\` の位置（display 数式の snippet 切り出しに使う）。
    fn on_env(&mut self, env: &str, marker_start: usize) {
        let s = self.s;
        if DISPLAY_ENVS.contains(&env) {
            self.flush_paragraph();
            let mut content_start = self.i; // self.i は既に \begin{env} の直後
            // alignat 系は列数の必須引数 {n} を持つ。
            if env.starts_with("alignat") {
                let j = skip_ws(s, content_start);
                if let Some((_, e)) = read_group(s, j) {
                    content_start = e;
                }
            }
            match self.find_env_end(content_start, env) {
                Some((content_end, after_end)) => {
                    let snippet = &s[marker_start..after_end];
                    let inner = &s[content_start..content_end];
                    self.emit_math(snippet, inner);
                    self.i = after_end;
                }
                None => {
                    self.warnings
                        .push(format!("unterminated math environment '{env}'"));
                }
            }
            return;
        }
        if env == "abstract" {
            self.flush_paragraph();
            if let Some((content_end, after_end)) = self.find_env_end(self.i, env) {
                let text = collapse_ws(&s[self.i..content_end]);
                self.push_block(TexBlock {
                    kind: NodeKind::Abstract,
                    text,
                    latex: None,
                    equation_label: None,
                    labels: Vec::new(),
                    section_number: None,
                    heading_level: None,
                    cite_key: None,
                    note: None,
                    confidence: 0.95,
                });
                self.i = after_end;
            }
            return;
        }
        if FIGURE_ENVS.contains(&env) || TABLE_ENVS.contains(&env) {
            self.flush_paragraph();
            let is_table = TABLE_ENVS.contains(&env);
            if let Some((content_end, after_end)) = self.find_env_end(self.i, env) {
                let content = &s[self.i..content_end];
                if let Some(pos) = find_control_word(content, 0, "caption") {
                    if let Some((cap, _)) = read_command_arg(content, pos) {
                        self.push_block(TexBlock {
                            kind: if is_table {
                                NodeKind::TableCaption
                            } else {
                                NodeKind::FigureCaption
                            },
                            text: collapse_ws(cap),
                            latex: None,
                            equation_label: None,
                            labels: collect_labels(content),
                            section_number: None,
                            heading_level: None,
                            cite_key: None,
                            note: None,
                            confidence: 0.95,
                        });
                    }
                }
                self.i = after_end;
            }
            return;
        }
        if LIST_ENVS.contains(&env) {
            self.flush_paragraph();
            if let Some((content_end, after_end)) = self.find_env_end(self.i, env) {
                let content = &s[self.i..content_end];
                let items = split_items(content);
                if !items.is_empty() {
                    let text = items
                        .iter()
                        .map(|t| format!("• {t}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.push_block(TexBlock {
                        kind: NodeKind::List,
                        text,
                        latex: None,
                        equation_label: None,
                        labels: Vec::new(),
                        section_number: None,
                        heading_level: None,
                        cite_key: None,
                        note: None,
                        confidence: 0.9,
                    });
                }
                self.i = after_end;
            }
            return;
        }
        if VERBATIM_ENVS.contains(&env) {
            self.flush_paragraph();
            // verbatim 内は字句解釈しない: リテラルの \end{env} を探す。
            let end_marker = format!("\\end{{{env}}}");
            match s[self.i..].find(&end_marker) {
                Some(rel) => {
                    let content = &s[self.i..self.i + rel];
                    self.push_block(TexBlock {
                        kind: NodeKind::CodeBlock,
                        text: content.trim_matches('\n').to_string(),
                        latex: None,
                        equation_label: None,
                        labels: Vec::new(),
                        section_number: None,
                        heading_level: None,
                        cite_key: None,
                        note: None,
                        confidence: 0.95,
                    });
                    self.i = self.i + rel + end_marker.len();
                }
                None => {
                    self.warnings
                        .push(format!("unterminated verbatim environment '{env}'"));
                    self.i = s.len();
                }
            }
            return;
        }
        if env == "thebibliography" {
            self.flush_paragraph();
            // 必須の {widest} 引数を消費。
            let j = skip_ws(s, self.i);
            if let Some((_, e)) = read_group(s, j) {
                self.i = e;
            }
            if let Some((content_end, after_end)) = self.find_env_end(self.i, env) {
                let content = &s[self.i..content_end];
                self.push_block(TexBlock {
                    kind: NodeKind::Bibliography,
                    text: "References".to_string(),
                    latex: None,
                    equation_label: None,
                    labels: Vec::new(),
                    section_number: None,
                    heading_level: None,
                    cite_key: None,
                    note: None,
                    confidence: 0.95,
                });
                for (key, text) in split_bibitems(content) {
                    self.push_block(TexBlock {
                        kind: NodeKind::BibliographyEntry,
                        text,
                        latex: None,
                        equation_label: None,
                        labels: Vec::new(),
                        section_number: None,
                        heading_level: None,
                        cite_key: Some(key),
                        note: None,
                        confidence: 0.9,
                    });
                }
                self.i = after_end;
            }
            return;
        }
        if DROP_ENVS.contains(&env) {
            self.flush_paragraph();
            if let Some((_, after_end)) = self.find_env_end(self.i, env) {
                self.i = after_end;
            } else {
                self.warnings.push(format!("unterminated environment '{env}'"));
                self.i = s.len();
            }
            return;
        }
        if TRANSPARENT_ENVS.contains(&env) {
            self.flush_paragraph();
            if env == "appendices" || env == "appendix" {
                self.counters.appendix = true;
                self.counters.sec = 0;
                self.counters.sub = 0;
                self.counters.subsub = 0;
            }
            if env == "minipage" {
                // [pos] + {width} を消費してから中身を通常解析する。
                let j = skip_optional(s, self.i);
                let j2 = skip_ws(s, j);
                if let Some((_, e)) = read_group(s, j2) {
                    self.i = e;
                }
            }
            return; // 中身は通常フローで解析（\end{env} は on_backslash 側で flush のみ）。
        }
        // 定理・定義・証明（Phase 5）。環境名 → 種別。原文由来なので高信頼。本文は 1 ブロックに
        // まとめ（`\label` を除き collapse）、`[note]` と `\label` を捕捉する。内側の display 数式は
        // 生 LaTeX のまま本文に残る（別ノード化はしない — TeX の統計と統一）。
        if let Some(kind) = self.theorem_envs.get(env).copied() {
            self.flush_paragraph();
            // 任意の付記名 `\begin{theorem}[Pythagoras]` を消費して捕捉する。
            let (note, after_opt) = read_optional_arg(s, self.i);
            self.i = after_opt;
            match self.find_env_end(self.i, env) {
                Some((content_end, after_end)) => {
                    let inner = &s[self.i..content_end];
                    let labels = collect_labels(inner);
                    let text = collapse_ws(&strip_labels(inner));
                    self.push_block(TexBlock {
                        kind,
                        text,
                        latex: None,
                        equation_label: None,
                        labels,
                        section_number: None,
                        heading_level: None,
                        cite_key: None,
                        note,
                        confidence: 0.95,
                    });
                    self.i = after_end;
                }
                None => {
                    self.warnings
                        .push(format!("unterminated theorem-like environment '{env}'"));
                    self.i = s.len();
                }
            }
            return;
        }
        if env == "document" {
            return;
        }
        // 未知環境: 透過扱い（マーカーは剥がし、中身は解析する）。名前は警告にまとめる。
        self.flush_paragraph();
        self.unknown_envs.insert(env.to_string());
    }

    /// `from` から対応する `\end{env}` を探す（同名の入れ子を数える）。
    /// 返り値は (中身の終端, `\end{env}` の直後)。既知の限界: `\end {env}` のような
    /// 空白入りの終端は見つけられず unterminated 警告になる（誤対応より欠損を選ぶ）。
    fn find_env_end(&self, from: usize, env: &str) -> Option<(usize, usize)> {
        let s = self.s;
        let begin_marker = format!("\\begin{{{env}}}");
        let end_marker = format!("\\end{{{env}}}");
        let mut depth = 1i64;
        let mut i = from;
        loop {
            let next_end = find_unescaped(s, i, &end_marker)?;
            let next_begin = find_unescaped(s, i, &begin_marker);
            if let Some(nb) = next_begin {
                if nb < next_end {
                    depth += 1;
                    i = nb + begin_marker.len();
                    continue;
                }
            }
            depth -= 1;
            if depth == 0 {
                return Some((next_end, next_end + end_marker.len()));
            }
            i = next_end + end_marker.len();
        }
    }

    /// `\be .. \ee` 型エイリアスの display 数式。終端はエイリアス（`\ee`）か本物の
    /// `\end{env}` の先に来た方。`alias_start` = `\be` の `\` の位置。
    fn consume_alias_math(&mut self, begin_alias: &str, env: &str, alias_start: usize, content_start: usize) {
        let s = self.s;
        let end_names: Vec<String> = self
            .end_aliases
            .iter()
            .filter(|(_, e)| e.as_str() == env)
            .map(|(n, _)| format!("\\{n}"))
            .collect();
        let literal_end = format!("\\end{{{env}}}");

        let mut best: Option<(usize, usize)> = None; // (終端開始, 終端直後)
        for pat in end_names.iter().chain(std::iter::once(&literal_end)) {
            let mut from = content_start;
            while let Some(pos) = find_unescaped(s, from, pat) {
                // control word 境界を確認（\ee が \eeq の頭に一致しないように）。
                let after = pos + pat.len();
                let boundary_ok = pat.starts_with("\\end{")
                    || !s.as_bytes().get(after).is_some_and(|b| b.is_ascii_alphabetic());
                if boundary_ok {
                    if best.is_none_or(|(b, _)| pos < b) {
                        best = Some((pos, after));
                    }
                    break;
                }
                from = pos + 1;
            }
        }
        match best {
            Some((end_start, after_end)) => {
                let snippet = &s[alias_start..after_end];
                let inner = &s[content_start..end_start];
                self.emit_math(snippet, inner);
                self.i = after_end;
            }
            None => {
                self.warnings
                    .push(format!("unterminated math alias '\\{begin_alias}'"));
                self.i = content_start;
            }
        }
    }

    /// display 数式ブロックを作る。latex = 原文スニペット（verbatim）。
    fn emit_math(&mut self, snippet: &str, inner: &str) {
        let labels = collect_labels(inner);
        let equation_label = extract_tag(inner);
        let normalized = normalize_math_text(inner);
        let text = if normalized.is_empty() {
            collapse_ws(snippet)
        } else {
            normalized
        };
        self.push_block(TexBlock {
            kind: NodeKind::DisplayMath,
            text,
            latex: Some(snippet.trim().to_string()),
            equation_label,
            labels,
            section_number: None,
            heading_level: None,
            cite_key: None,
            note: None,
            confidence: 0.98,
        });
    }
}

/// `from` 以降で control word `\word`（直後が英字でない）を探す。
fn find_control_word(s: &str, from: usize, word: &str) -> Option<usize> {
    let pat = format!("\\{word}");
    let mut i = from;
    while let Some(rel) = s[i..].find(&pat) {
        let pos = i + rel;
        let after = pos + pat.len();
        if unescaped(s, pos)
            && !s.as_bytes().get(after).is_some_and(|b| b.is_ascii_alphabetic())
        {
            return Some(after);
        }
        i = pos + 1;
    }
    None
}

/// 定理系環境名の既定マップ（Phase 5）。amsthm が予約する `proof` と、クラス/パッケージが
/// 予約しうる標準英名。独自名・略記（`thm`/`lem` 等）は preamble の `\newtheorem` で足す。
fn default_theorem_envs() -> BTreeMap<String, NodeKind> {
    [
        ("theorem", NodeKind::Theorem),
        ("lemma", NodeKind::Lemma),
        ("proposition", NodeKind::Proposition),
        ("corollary", NodeKind::Corollary),
        ("definition", NodeKind::Definition),
        ("remark", NodeKind::Remark),
        ("example", NodeKind::Example),
        ("proof", NodeKind::Proof),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// `\newtheorem{env}{Display}` の表示名からノード種別を推定する（"Main Theorem" → Theorem）。
/// 8 種のどれにも一致しなければ None（未知の定理様環境は従来どおり透過扱いになる）。
fn theorem_kind_from_title(title: &str) -> Option<NodeKind> {
    let t = title.to_ascii_lowercase();
    // 具体的な語を先に見る（"proposition" は "proof" と衝突しない）。
    for (kw, kind) in [
        ("theorem", NodeKind::Theorem),
        ("lemma", NodeKind::Lemma),
        ("proposition", NodeKind::Proposition),
        ("corollary", NodeKind::Corollary),
        ("definition", NodeKind::Definition),
        ("remark", NodeKind::Remark),
        ("example", NodeKind::Example),
        ("proof", NodeKind::Proof),
    ] {
        if t.contains(kw) {
            return Some(kind);
        }
    }
    None
}

/// `s[i]` 以降の任意 `[..]` を 1 個読んで中身を返す（brace-aware・改行可）。無ければ (None, i)。
fn read_optional_arg(s: &str, i: usize) -> (Option<String>, usize) {
    let j = skip_ws(s, i);
    let bytes = s.as_bytes();
    if j >= bytes.len() || bytes[j] != b'[' {
        return (None, i);
    }
    let mut brace = 0i64;
    let mut k = j + 1;
    while k < bytes.len() {
        match bytes[k] {
            b'{' if unescaped(s, k) => brace += 1,
            b'}' if unescaped(s, k) => brace -= 1,
            b']' if unescaped(s, k) && brace <= 0 => {
                let inner = collapse_ws(&s[j + 1..k]);
                return (if inner.is_empty() { None } else { Some(inner) }, k + 1);
            }
            _ => {}
        }
        k += 1;
    }
    (None, i)
}

/// `\label{..}` を除去する（本文ノイズ）。他のコマンドは温存する。
fn strip_labels(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'\\' && unescaped(s, i) {
            let (word, after) = read_control_word(s, i + 1);
            if word == "label" {
                let j = skip_ws(s, after);
                i = read_group(s, j).map(|(_, e)| e).unwrap_or(after);
                continue;
            }
            // control symbol（`\%` 等）はマルチバイトを割らないよう char 長で進める。
            let end = if word.is_empty() {
                i + 1 + s[i + 1..].chars().next().map_or(0, |c| c.len_utf8())
            } else {
                after
            };
            out.push_str(&s[i..end]);
            i = end;
            continue;
        }
        let ch_len = s[i..].chars().next().map_or(1, |c| c.len_utf8());
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// テキスト中の `\label{..}` 名を全て集める。
fn collect_labels(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(after) = find_control_word(s, i, "label") {
        let j = skip_ws(s, after);
        match read_group(s, j) {
            Some((name, end)) => {
                out.push(name.trim().to_string());
                i = end;
            }
            None => i = after,
        }
    }
    out
}

/// `\tag{X}` / `\tag*{X}` → `"(X)"`。
fn extract_tag(s: &str) -> Option<String> {
    let after = find_control_word(s, 0, "tag")?;
    let mut j = after;
    if s.as_bytes().get(j) == Some(&b'*') {
        j += 1;
    }
    let j = skip_ws(s, j);
    let (inner, _) = read_group(s, j)?;
    let inner = collapse_ws(inner);
    if inner.is_empty() {
        None
    } else {
        Some(format!("({inner})"))
    }
}

/// 数式の検索/表示用の線形文字列: `\label`/`\nonumber`/`\notag` を除き空白を正規化。
fn normalize_math_text(s: &str) -> String {
    let mut t = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'\\' && unescaped(s, i) {
            let (word, after) = read_control_word(s, i + 1);
            match word {
                "label" => {
                    let j = skip_ws(s, after);
                    i = read_group(s, j).map(|(_, e)| e).unwrap_or(after);
                    continue;
                }
                "nonumber" | "notag" => {
                    i = after;
                    continue;
                }
                _ => {
                    // マルチバイトの control symbol（`\é` 等）で char 境界を割らない。
                    let end = if word.is_empty() {
                        i + 1 + s[i + 1..].chars().next().map_or(0, |c| c.len_utf8())
                    } else {
                        after
                    };
                    t.push_str(&s[i..end]);
                    i = end;
                    continue;
                }
            }
        }
        let ch_len = s[i..].chars().next().map_or(1, |c| c.len_utf8());
        t.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    collapse_ws(&t)
}

/// 空白正規化（改行含む連続空白 → 1 個のスペース・前後 trim）。
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `\cmd{..}` を（引数ごと）取り除く。タイトルの `\thanks{..}` 除去用。
fn strip_command(s: &str, cmd: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if let Some(after) = find_control_word(s, i, cmd) {
            let start = after - cmd.len() - 1;
            if start >= i {
                out.push_str(&s[i..start]);
                let j = skip_ws(s, after);
                i = read_group(s, j).map(|(_, e)| e).unwrap_or(after);
                continue;
            }
        }
        out.push_str(&s[i..]);
        break;
    }
    out
}

/// リスト環境の中身を `\item` 単位に割る（先頭の前置きは捨てる・光学引数は除去）。
fn split_items(content: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut positions = Vec::new();
    let mut i = 0;
    while let Some(after) = find_control_word(content, i, "item") {
        positions.push((after - "item".len() - 1, after));
        i = after;
    }
    for (idx, &(_, after)) in positions.iter().enumerate() {
        let start = skip_optional(content, after);
        let end = positions
            .get(idx + 1)
            .map(|&(s, _)| s)
            .unwrap_or(content.len());
        let text = collapse_ws(&content[start..end]);
        if !text.is_empty() {
            items.push(text);
        }
    }
    items
}

/// `thebibliography` の中身を `\bibitem[..]{key}` 単位に割り、(key, 本文) を返す。
fn split_bibitems(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut starts = Vec::new();
    let mut i = 0;
    while let Some(after) = find_control_word(content, i, "bibitem") {
        starts.push((after - "bibitem".len() - 1, after));
        i = after;
    }
    for (idx, &(_, after)) in starts.iter().enumerate() {
        let entry_end = starts
            .get(idx + 1)
            .map(|&(s, _)| s)
            .unwrap_or(content.len());
        // revtex の `\bibitem[{..}]{key}`: 光学引数は brace-aware に消費する。
        let j = skip_optional(content, after);
        let j = skip_ws(content, j);
        let Some((key, body_start)) = read_group(content, j) else {
            continue;
        };
        let text = collapse_ws(&content[body_start..entry_end]);
        out.push((key.trim().to_string(), text));
    }
    out
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// ファイル群から抽出する（コンテナを介さないテスト入口）。
    fn extract(files: &[(&str, &str)]) -> ExtractedTexDocument {
        let map: BTreeMap<String, String> = files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        extract_from_files(source::TexSourceFiles {
            files: map,
            warnings: Vec::new(),
        })
        .unwrap()
    }

    fn kinds(doc: &ExtractedTexDocument) -> Vec<&'static str> {
        doc.blocks.iter().map(|b| b.kind.as_str()).collect()
    }

    fn find(doc: &ExtractedTexDocument, kind: NodeKind) -> Vec<&TexBlock> {
        doc.blocks.iter().filter(|b| b.kind == kind).collect()
    }

    // ── 字句規則（パリティ） ──

    #[test]
    fn linebreak_with_dimension_is_not_display_math() {
        // blocker: `\\[4pt]` は改行 + 間隔。display 数式 `\[` と誤認して以降を飲み込まない。
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             first line \\\\[4pt] second line\n\n\
             next paragraph\n\
             \\end{document}",
        )]);
        assert_eq!(kinds(&doc), vec!["paragraph", "paragraph"]);
        assert!(doc.blocks[0].text.contains("first line"));
        assert!(doc.blocks[0].text.contains("second line"));
    }

    #[test]
    fn escaped_percent_is_kept_and_double_backslash_percent_is_comment() {
        let stripped = strip_comments("a 50\\% level\nb \\\\% trailing comment\nc % gone\n");
        assert!(stripped.contains("50\\%"), "{stripped}");
        assert!(!stripped.contains("trailing"), "{stripped}");
        assert!(!stripped.contains("gone"), "{stripped}");
        assert!(stripped.contains("c \n") || stripped.contains("c\n"), "{stripped}");
    }

    #[test]
    fn comment_only_line_does_not_split_paragraph() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             one sentence here\n\
             % just a comment\n\
             and it continues\n\
             \\end{document}",
        )]);
        let paras = find(&doc, NodeKind::Paragraph);
        assert_eq!(paras.len(), 1, "{:?}", kinds(&doc));
        assert!(paras[0].text.contains("one sentence here and it continues"));
    }

    #[test]
    fn url_with_percent_survives_comment_stripping() {
        let stripped = strip_comments("see \\url{http://x.org/a%20b} now\nand \\verb|x%y| too\n");
        assert!(stripped.contains("a%20b"), "{stripped}");
        assert!(stripped.contains("x%y"), "{stripped}");
    }

    // ── display 数式 ──

    #[test]
    fn equation_env_yields_raw_latex_with_label_and_tag() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{equation}\\label{eq:mass}\n  E = m c^2 \\tag{1'}\n\\end{equation}\n\
             \\end{document}",
        )]);
        let math = find(&doc, NodeKind::DisplayMath);
        assert_eq!(math.len(), 1);
        let m = math[0];
        let latex = m.latex.as_deref().unwrap();
        assert!(latex.starts_with("\\begin{equation}"), "{latex}");
        assert!(latex.ends_with("\\end{equation}"), "{latex}");
        assert!(latex.contains("\\label{eq:mass}"), "raw snippet keeps label");
        assert_eq!(m.labels, vec!["eq:mass"]);
        assert_eq!(m.equation_label.as_deref(), Some("(1')"));
        assert!(!m.text.contains("\\label"), "normalized text drops label: {}", m.text);
        assert!(m.text.contains("E = m c^2"));
        assert!((m.confidence - 0.98).abs() < 1e-9);
    }

    #[test]
    fn bracket_and_dollar_display_math_forms() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\[ a^2 + b^2 = c^2 \\]\n\
             $$ x = y $$\n\
             inline $z=1$ stays in prose\n\
             \\end{document}",
        )]);
        let math = find(&doc, NodeKind::DisplayMath);
        assert_eq!(math.len(), 2);
        assert_eq!(math[0].latex.as_deref(), Some("\\[ a^2 + b^2 = c^2 \\]"));
        assert_eq!(math[1].latex.as_deref(), Some("$$ x = y $$"));
        let paras = find(&doc, NodeKind::Paragraph);
        assert_eq!(paras.len(), 1);
        assert!(paras[0].text.contains("$z=1$"), "inline math kept raw: {}", paras[0].text);
    }

    #[test]
    fn align_with_nested_env_and_alignat_argument() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{align}\n a &= \\begin{cases} 1 & x>0 \\\\ 0 & x\\le 0 \\end{cases}\n\\end{align}\n\
             \\begin{alignat}{2}\n x &= 1 &\\quad y &= 2\n\\end{alignat}\n\
             \\end{document}",
        )]);
        let math = find(&doc, NodeKind::DisplayMath);
        assert_eq!(math.len(), 2);
        assert!(math[0].latex.as_deref().unwrap().contains("\\begin{cases}"));
        // alignat の {2} は normalized からは見えない（引数として消費）。
        assert!(math[1].text.starts_with("x"), "{}", math[1].text);
    }

    #[test]
    fn preamble_macro_aliases_be_ee_are_recognized() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\n\
             \\def\\be{\\begin{equation}}\n\\def\\ee{\\end{equation}}\n\
             \\newcommand{\\bea}{\\begin{eqnarray}}\n\\newcommand{\\eea}{\\end{eqnarray}}\n\
             \\begin{document}\n\
             \\be E = mc^2 \\ee\n\
             \\bea a &=& b \\eea\n\
             \\end{document}",
        )]);
        let math = find(&doc, NodeKind::DisplayMath);
        assert_eq!(math.len(), 2, "{:?}", kinds(&doc));
        assert_eq!(math[0].latex.as_deref(), Some("\\be E = mc^2 \\ee"));
        assert!(math[1].latex.as_deref().unwrap().starts_with("\\bea"));
    }

    #[test]
    fn subequations_is_transparent_wrapper() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{subequations}\n\\begin{align} a &= b \\end{align}\n\\end{subequations}\n\
             \\end{document}",
        )]);
        assert_eq!(find(&doc, NodeKind::DisplayMath).len(), 1);
        // subequations のマーカーが段落として漏れない。
        assert!(find(&doc, NodeKind::Paragraph).is_empty(), "{:?}", kinds(&doc));
    }

    // ── 見出し・番号 ──

    #[test]
    fn section_counters_and_appendix_lettering() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\section{Intro}\\label{sec:intro}\n\
             \\subsection{Background}\n\
             \\subsubsection{Detail}\n\
             \\section*{Unnumbered}\n\
             \\section{Model}\n\
             \\appendix\n\
             \\section{Extra}\n\
             \\subsection{More}\n\
             \\section{Second}\n\
             \\end{document}",
        )]);
        let heads: Vec<(&str, Option<&str>, &str)> = doc
            .blocks
            .iter()
            .filter(|b| b.heading_level.is_some())
            .map(|b| (b.kind.as_str(), b.section_number.as_deref(), b.text.as_str()))
            .collect();
        assert_eq!(
            heads,
            vec![
                ("section", Some("1"), "1 Intro"),
                ("subsection", Some("1.1"), "1.1 Background"),
                ("heading", Some("1.1.1"), "1.1.1 Detail"),
                ("section", None, "Unnumbered"),
                ("section", Some("2"), "2 Model"),
                ("section", Some("A"), "A Extra"),
                ("subsection", Some("A.1"), "A.1 More"),
                ("section", Some("B"), "B Second"),
            ]
        );
        let intro = &doc.blocks.iter().find(|b| b.text == "1 Intro").unwrap();
        assert_eq!(intro.labels, vec!["sec:intro"]);
    }

    #[test]
    fn section_optional_short_title_is_skipped() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\section[short]{The Long Title}\n\
             \\end{document}",
        )]);
        let heads = find(&doc, NodeKind::Section);
        assert_eq!(heads[0].text, "1 The Long Title");
    }

    // ── front matter（revtex / jheppub） ──

    #[test]
    fn revtex_title_and_abstract_after_begin_document() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass[aps,prl]{revtex4-2}\\begin{document}\n\
             \\title{Observation of Something\\thanks{grant}}\n\
             \\author{A. Author}\\affiliation{Some University}\n\
             \\begin{abstract}\nWe observe something interesting.\n\\end{abstract}\n\
             \\maketitle\n\
             \\section{Introduction}\nBody text.\n\
             \\end{document}",
        )]);
        let fm = find(&doc, NodeKind::FrontMatter);
        assert_eq!(fm.len(), 1);
        assert_eq!(fm[0].text, "Observation of Something");
        let abs = find(&doc, NodeKind::Abstract);
        assert_eq!(abs.len(), 1);
        assert!(abs[0].text.contains("something interesting"));
        // author/affiliation は段落として漏れない。
        assert!(!doc.blocks.iter().any(|b| b.text.contains("Some University")));
    }

    #[test]
    fn jheppub_abstract_command_form_in_preamble() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\n\
             \\title{JHEP Style Paper}\n\
             \\abstract{Command-form abstract text.}\n\
             \\begin{document}\nBody.\n\\end{document}",
        )]);
        let fm = find(&doc, NodeKind::FrontMatter);
        assert_eq!(fm[0].text, "JHEP Style Paper");
        let abs = find(&doc, NodeKind::Abstract);
        assert_eq!(abs.len(), 1);
        assert!(abs[0].text.contains("Command-form"));
    }

    // ── 図表・リスト・verbatim・未知環境 ──

    #[test]
    fn figure_caption_extracted_body_dropped() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{figure}[htbp]\n\
             \\begin{tikzpicture}\\draw (0,0) -- (1,1);\\end{tikzpicture}\n\
             \\caption[short]{The spectrum of $H$.}\\label{fig:spec}\n\
             \\end{figure}\n\
             \\begin{table}\n\\begin{tabular}{cc} a & b \\\\ c & d \\end{tabular}\n\
             \\caption{Data table.}\n\\end{table}\n\
             \\end{document}",
        )]);
        let figs = find(&doc, NodeKind::FigureCaption);
        assert_eq!(figs.len(), 1);
        assert_eq!(figs[0].text, "The spectrum of $H$.");
        assert_eq!(figs[0].labels, vec!["fig:spec"]);
        let tabs = find(&doc, NodeKind::TableCaption);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].text, "Data table.");
        // tikz/tabular の中身が段落に漏れない。
        assert!(!doc.blocks.iter().any(|b| b.text.contains("\\draw")));
        assert!(!doc.blocks.iter().any(|b| b.text.contains("a & b")));
    }

    #[test]
    fn itemize_becomes_single_list_block() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{itemize}\n\\item first point\n\\item[*] second point\n\\end{itemize}\n\
             \\end{document}",
        )]);
        let lists = find(&doc, NodeKind::List);
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].text, "• first point\n• second point");
    }

    #[test]
    fn verbatim_is_opaque_code_block() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{verbatim}\n\\begin{equation} not math % not comment\n\\end{verbatim}\n\
             \\end{document}",
        )]);
        let code = find(&doc, NodeKind::CodeBlock);
        assert_eq!(code.len(), 1);
        assert!(code[0].text.contains("\\begin{equation} not math % not comment"));
        assert!(find(&doc, NodeKind::DisplayMath).is_empty());
    }

    #[test]
    fn unknown_env_is_transparent_with_warning_and_widetext_passes_through() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{mysterybox}\nEvery finite group is finite.\n\\end{mysterybox}\n\
             \\begin{widetext}\n\\begin{equation} w = 1 \\end{equation}\n\\end{widetext}\n\
             \\end{document}",
        )]);
        let paras = find(&doc, NodeKind::Paragraph);
        assert!(paras.iter().any(|p| p.text.contains("Every finite group")));
        assert_eq!(find(&doc, NodeKind::DisplayMath).len(), 1);
        assert!(doc
            .warnings
            .iter()
            .any(|w| w.contains("unrecognized environments") && w.contains("mysterybox")));
        assert!(!doc.blocks.iter().any(|b| b.text.contains("\\begin{mysterybox}")));
    }

    // ── 定理・定義・証明（Phase 5） ──

    #[test]
    fn standard_theorem_and_proof_environments_become_typed_nodes() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{theorem}\\label{thm:main}\nEvery bounded sequence has a convergent subsequence.\n\\end{theorem}\n\
             \\begin{proof}\nConsider a monotone subsequence; it converges.\n\\end{proof}\n\
             \\begin{definition}\nA set is compact if every cover has a finite subcover.\n\\end{definition}\n\
             \\end{document}",
        )]);
        let thm = find(&doc, NodeKind::Theorem);
        assert_eq!(thm.len(), 1);
        assert!(thm[0].text.contains("bounded sequence"));
        assert_eq!(thm[0].labels, vec!["thm:main"]);
        // \label は本文テキストからは除かれる。
        assert!(!thm[0].text.contains("\\label"), "{}", thm[0].text);
        assert!((thm[0].confidence - 0.95).abs() < 1e-9);
        assert_eq!(find(&doc, NodeKind::Proof).len(), 1);
        assert_eq!(find(&doc, NodeKind::Definition).len(), 1);
    }

    #[test]
    fn newtheorem_custom_names_map_to_kinds() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\n\
             \\newtheorem{thm}{Theorem}\n\
             \\newtheorem{lem}[thm]{Lemma}\n\
             \\newtheorem{prop}{Proposition}[section]\n\
             \\newtheorem*{rmk}{Remark}\n\
             \\begin{document}\n\
             \\begin{thm}\nStatement of the theorem.\n\\end{thm}\n\
             \\begin{lem}\nStatement of the lemma.\n\\end{lem}\n\
             \\begin{prop}\nStatement of the proposition.\n\\end{prop}\n\
             \\begin{rmk}\nA passing remark.\n\\end{rmk}\n\
             \\end{document}",
        )]);
        assert_eq!(find(&doc, NodeKind::Theorem).len(), 1, "{:?}", kinds(&doc));
        assert_eq!(find(&doc, NodeKind::Lemma).len(), 1);
        assert_eq!(find(&doc, NodeKind::Proposition).len(), 1);
        assert_eq!(find(&doc, NodeKind::Remark).len(), 1);
        // 独自環境名が段落に漏れない。
        assert!(!doc.blocks.iter().any(|b| b.text.contains("\\begin{thm}")));
    }

    #[test]
    fn theorem_optional_note_is_captured() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{theorem}[Bolzano--Weierstrass]\nEvery bounded sequence ...\n\\end{theorem}\n\
             \\end{document}",
        )]);
        let thm = find(&doc, NodeKind::Theorem);
        assert_eq!(thm.len(), 1);
        assert_eq!(thm[0].note.as_deref(), Some("Bolzano--Weierstrass"));
        // note は本文テキストには含めない。
        assert!(!thm[0].text.contains("Bolzano"), "{}", thm[0].text);
        assert!(thm[0].text.contains("bounded sequence"));
    }

    #[test]
    fn theorem_body_keeps_inner_display_math_as_raw_latex() {
        // 定理内の display 数式は独立ノード化せず、生 LaTeX のまま本文に残す（flat 統計）。
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{theorem}\nWe have \\begin{equation} E = mc^2 \\end{equation} for all bodies.\n\\end{theorem}\n\
             \\end{document}",
        )]);
        let thm = find(&doc, NodeKind::Theorem);
        assert_eq!(thm.len(), 1);
        assert!(thm[0].text.contains("E = mc^2"), "{}", thm[0].text);
        // 定理の内側は別 display_math ノードにしない。
        assert!(find(&doc, NodeKind::DisplayMath).is_empty(), "{:?}", kinds(&doc));
    }

    // ── 参考文献 ──

    #[test]
    fn thebibliography_with_revtex_optional_args() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{thebibliography}{99}\n\
             \\bibitem{plain} A. Author, Journal 1 (2020).\n\
             \\bibitem[{\\citenamefont{Abbott et al.}(2016)}]{LIGO} B. P. Abbott et al.\n\
             \\end{thebibliography}\n\\end{document}",
        )]);
        let bib = find(&doc, NodeKind::Bibliography);
        assert_eq!(bib.len(), 1);
        let entries = find(&doc, NodeKind::BibliographyEntry);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].cite_key.as_deref(), Some("plain"));
        assert!(entries[0].text.contains("Journal 1 (2020)"));
        assert_eq!(entries[1].cite_key.as_deref(), Some("LIGO"));
        assert!(entries[1].text.contains("Abbott"));
        // {99} が本文に漏れない。
        assert!(!doc.blocks.iter().any(|b| b.text.trim() == "99"));
    }

    #[test]
    fn bibliography_command_splices_bbl_file() {
        let doc = extract(&[
            (
                "main.tex",
                "\\documentclass{article}\\begin{document}\nText.\n\
                 \\bibliographystyle{plain}\n\\bibliography{refs}\n\\end{document}",
            ),
            (
                "main.bbl",
                "\\begin{thebibliography}{9}\n\\bibitem{k1} Some Reference.\n\\end{thebibliography}\n",
            ),
        ]);
        let entries = find(&doc, NodeKind::BibliographyEntry);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cite_key.as_deref(), Some("k1"));
    }

    #[test]
    fn missing_bbl_warns() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\\bibliography{refs}\n\\end{document}",
        )]);
        assert!(doc.warnings.iter().any(|w| w.contains("no .bbl")), "{:?}", doc.warnings);
    }

    // ── \input 解決・main 検出 ──

    #[test]
    fn input_include_and_braceless_forms_are_spliced() {
        let doc = extract(&[
            (
                "main.tex",
                "\\documentclass{article}\\begin{document}\n\
                 \\input{sections/intro}\n\\include{model}\n\\input macros.tex\n\
                 \\end{document}",
            ),
            ("sections/intro.tex", "\\section{Intro}\nIntro text.\n"),
            ("model.tex", "\\section{Model}\nModel text.\n"),
            ("macros.tex", "Extra prose.\n"),
        ]);
        let secs = find(&doc, NodeKind::Section);
        assert_eq!(secs.len(), 2);
        assert!(doc.blocks.iter().any(|b| b.text.contains("Extra prose")));
    }

    #[test]
    fn input_cycles_and_missing_files_warn_not_hang() {
        let doc = extract(&[
            (
                "main.tex",
                "\\documentclass{article}\\begin{document}\n\\input{a}\n\\input{missing}\n\\end{document}",
            ),
            ("a.tex", "A text \\input{a}\n"),
        ]);
        assert!(doc.blocks.iter().any(|b| b.text.contains("A text")));
        assert!(doc.warnings.iter().any(|w| w.contains("repeated")), "{:?}", doc.warnings);
        assert!(doc.warnings.iter().any(|w| w.contains("unresolved")), "{:?}", doc.warnings);
    }

    #[test]
    fn input_inside_verbatim_is_not_spliced() {
        let doc = extract(&[
            (
                "main.tex",
                "\\documentclass{article}\\begin{document}\n\
                 \\begin{verbatim}\n\\input{a}\n\\end{verbatim}\n\\end{document}",
            ),
            ("a.tex", "SHOULD NOT APPEAR"),
        ]);
        let code = find(&doc, NodeKind::CodeBlock);
        assert!(code[0].text.contains("\\input{a}"));
        assert!(!doc.blocks.iter().any(|b| b.text.contains("SHOULD NOT APPEAR")));
    }

    #[test]
    fn main_detection_skips_standalone_and_referenced_files() {
        // fig1.tex は standalone クラス + 辞書順で先に来るが、main を選ぶこと。
        let doc = extract(&[
            (
                "fig1.tex",
                "\\documentclass{standalone}\\begin{document}tikz\\end{document}",
            ),
            (
                "zz-paper.tex",
                "\\documentclass{article}\\title{Real Paper}\\begin{document}\n\
                 \\input{fig1}\nBody.\n\\end{document}",
            ),
        ]);
        assert_eq!(doc.main_file, "zz-paper.tex");
        // standalone の fig1 はスプライスもされない（自前の documentclass を持つため）。
        assert!(doc.warnings.iter().any(|w| w.contains("fig1")), "{:?}", doc.warnings);
        assert!(!doc.blocks.iter().any(|b| b.text.contains("tikz")));
    }

    #[test]
    fn commented_documentclass_does_not_make_a_candidate() {
        let doc = extract(&[
            ("notes.tex", "% \\documentclass{article}\nJust notes, no class.\n"),
            (
                "real.tex",
                "\\documentclass{article}\\begin{document}Real body\\end{document}",
            ),
        ]);
        assert_eq!(doc.main_file, "real.tex");
    }

    #[test]
    fn plain_tex_fallback_with_documentstyle_and_dollars() {
        let doc = extract(&[(
            "old.tex",
            "\\documentstyle{article}\n\\section{Old Style}\n$$ x=1 $$\n\\end{document}",
        )]);
        assert_eq!(doc.main_file, "old.tex");
        assert_eq!(find(&doc, NodeKind::DisplayMath).len(), 1);
        // \begin{document} が無いので警告付きで全体を本文として扱う。
        assert!(doc.warnings.iter().any(|w| w.contains("no \\begin{document}")));
    }

    #[test]
    fn no_tex_content_errors() {
        let map: BTreeMap<String, String> =
            [("readme.tex".to_string(), "hello world".to_string())].into();
        let err = extract_from_files(source::TexSourceFiles {
            files: map,
            warnings: Vec::new(),
        })
        .unwrap_err();
        assert!(err.contains("no usable TeX main file"), "{err}");
    }

    // ── 段落規則 ──

    #[test]
    fn blank_line_inside_braces_does_not_split() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             before \\footnote{first part\n\n  second part} after\n\
             \\end{document}",
        )]);
        let paras = find(&doc, NodeKind::Paragraph);
        assert_eq!(paras.len(), 1, "{:?}", kinds(&doc));
        assert!(paras[0].text.contains("before"));
        assert!(paras[0].text.contains("after"));
    }

    /// レビュー回帰: `\` + マルチバイト文字（latin-1 由来の `\à` 等）で char 境界 panic しない。
    #[test]
    fn backslash_before_multibyte_char_does_not_panic() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             voil\\à un café — and math \\begin{equation} a \\± b \\end{equation}\n\
             \\end{document}",
        )]);
        assert!(doc.blocks.iter().any(|b| b.text.contains("voil")));
        let math = find(&doc, NodeKind::DisplayMath);
        assert_eq!(math.len(), 1);
        assert!(math[0].text.contains('±'), "{}", math[0].text);
    }

    /// レビュー回帰: コメント内の `\begin{verbatim}` で verbatim モードに入り、
    /// 以降の本文が消えてはいけない。
    #[test]
    fn commented_verbatim_marker_does_not_swallow_document() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             Intro text.\n\
             % TODO add \\begin{verbatim} sample here\n\
             \\section{Results}\nWe find things.\n\
             \\end{document}",
        )]);
        let secs = find(&doc, NodeKind::Section);
        assert_eq!(secs.len(), 1, "{:?}", kinds(&doc));
        assert!(doc.blocks.iter().any(|b| b.text.contains("We find things")));
        assert!(!doc.blocks.iter().any(|b| b.text.contains("TODO")));
    }

    /// レビュー回帰: `\begin {equation}`（空白入り）でも snippet が正確に切り出され、
    /// 先行する同名環境へ誤アンカーしない。
    #[test]
    fn spaced_begin_yields_exact_snippet() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             \\begin{equation} E_1 = 1 \\end{equation}\n\
             Prose between the two equations.\n\
             \\begin {equation} E_2 = 2 \\end{equation}\n\
             \\end{document}",
        )]);
        let math = find(&doc, NodeKind::DisplayMath);
        assert_eq!(math.len(), 2);
        let second = math[1].latex.as_deref().unwrap();
        assert!(second.starts_with("\\begin {equation}"), "{second}");
        assert!(!second.contains("E_1"), "先行数式や散文を含まない: {second}");
        assert!(!second.contains("Prose"), "{second}");
    }

    /// レビュー回帰: `\verb|$|` / `\url{...$...}` の中身が字句解釈されず、
    /// 以降の `$` 対応や構造認識が壊れない。
    #[test]
    fn verb_and_url_content_is_opaque_to_the_lexer() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             Use \\verb|$| to enter math and see \\url{http://x.org/~user}.\n\n\
             \\section{Results}\nWe find $x=1$ here.\n\
             \\end{document}",
        )]);
        let secs = find(&doc, NodeKind::Section);
        assert_eq!(secs.len(), 1, "{:?}", kinds(&doc));
        let paras = find(&doc, NodeKind::Paragraph);
        assert!(paras[0].text.contains("\\verb|$|"), "{}", paras[0].text);
        assert!(paras[0].text.contains("x.org/~user"), "~ は URL 内で保持: {}", paras[0].text);
        assert!(paras.iter().any(|p| p.text.contains("$x=1$")));
    }

    /// レビュー回帰: `\\` の光学引数探索は空行（段落区切り）をまたがない。
    #[test]
    fn linebreak_optional_arg_does_not_cross_paragraph_break() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             He said hello \\\\\n\n\
             [1] Bracketed paragraph start.\n\
             \\end{document}",
        )]);
        let paras = find(&doc, NodeKind::Paragraph);
        assert_eq!(paras.len(), 2, "{:?}", kinds(&doc));
        assert!(paras[1].text.contains("[1] Bracketed"), "{}", paras[1].text);
    }

    #[test]
    fn end_document_stops_parsing() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\nkeep\n\\end{document}\nDROPPED TRAILER",
        )]);
        assert!(!doc.blocks.iter().any(|b| b.text.contains("DROPPED")));
    }

    /// 手動スモーク: 実 arXiv e-print（gzip/tar）を丸ごと解析し、ブロック統計と数式の
    /// LaTeX 例を印字する。ネットワーク不要（ファイルは事前に取得しておく）。
    /// 例:
    /// `curl -sL https://arxiv.org/e-print/1706.03762 -o /tmp/eprint.gz && \
    ///  LCIR_SMOKE_TEX=/tmp/eprint.gz cargo test --lib tex_extract_real_source -- --ignored --nocapture`
    #[test]
    #[ignore = "manual smoke test; needs LCIR_SMOKE_TEX=<path to e-print .gz>"]
    fn tex_extract_real_source() {
        let Ok(path) = std::env::var("LCIR_SMOKE_TEX") else {
            eprintln!("skip: set LCIR_SMOKE_TEX=<path to arXiv e-print .gz>");
            return;
        };
        let doc = extract_document(std::path::Path::new(&path)).expect("extract");
        let count = |k: NodeKind| doc.blocks.iter().filter(|b| b.kind == k).count();
        eprintln!(
            "main={} files={} blocks={}\n  front_matter={} abstract={} section={} subsection={} \
             heading={} paragraph={} display_math={} figure_caption={} table_caption={} list={} \
             code_block={} bibliography_entry={}\n  warnings={:?}",
            doc.main_file,
            doc.source_file_count,
            doc.blocks.len(),
            count(NodeKind::FrontMatter),
            count(NodeKind::Abstract),
            count(NodeKind::Section),
            count(NodeKind::Subsection),
            count(NodeKind::Heading),
            count(NodeKind::Paragraph),
            count(NodeKind::DisplayMath),
            count(NodeKind::FigureCaption),
            count(NodeKind::TableCaption),
            count(NodeKind::List),
            count(NodeKind::CodeBlock),
            count(NodeKind::BibliographyEntry),
            doc.warnings,
        );
        // Phase 5: 定理系ノードの内訳。
        eprintln!(
            "  [phase5] theorem={} lemma={} proposition={} corollary={} definition={} \
             remark={} example={} proof={}",
            count(NodeKind::Theorem),
            count(NodeKind::Lemma),
            count(NodeKind::Proposition),
            count(NodeKind::Corollary),
            count(NodeKind::Definition),
            count(NodeKind::Remark),
            count(NodeKind::Example),
            count(NodeKind::Proof),
        );
        for b in doc.blocks.iter().filter(|b| b.heading_level.is_some()).take(12) {
            eprintln!("  [{}] {:?} {}", b.kind.as_str(), b.section_number, b.text);
        }
        for b in doc.blocks.iter().filter(|b| b.kind == NodeKind::DisplayMath).take(4) {
            eprintln!(
                "  [math] label={:?} latex={}",
                b.equation_label,
                b.latex.as_deref().unwrap_or("").chars().take(90).collect::<String>()
            );
        }
        // 定理・証明のサンプル（note/label と本文冒頭）。
        for b in doc
            .blocks
            .iter()
            .filter(|b| {
                matches!(
                    b.kind,
                    NodeKind::Theorem
                        | NodeKind::Lemma
                        | NodeKind::Proposition
                        | NodeKind::Corollary
                        | NodeKind::Definition
                        | NodeKind::Remark
                        | NodeKind::Example
                        | NodeKind::Proof
                )
            })
            .take(10)
        {
            eprintln!(
                "  [{}] note={:?} labels={:?} {}",
                b.kind.as_str(),
                b.note,
                b.labels,
                b.text.chars().take(80).collect::<String>()
            );
        }
        // Phase 6a: 参照グラフ（\ref/\eqref/\cite・proof→theorem）を純関数で解決して type 別に集計。
        use crate::ingestion::graph::{resolve_relations, GraphNode, RefStrategy};
        let graph_nodes: Vec<GraphNode> = doc
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| GraphNode {
                id: i as i64,
                kind: b.kind,
                reading_index: i as i64,
                plain_text: b.text.clone(),
                labels: b.labels.clone(),
                equation_label: b.equation_label.clone(),
                theorem_number: None,
                cite_key: b.cite_key.clone(),
            })
            .collect();
        let edges = resolve_relations(&graph_nodes, RefStrategy::Tex);
        let mut by_type: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
        for e in &edges {
            *by_type.entry(e.relation_type.as_str()).or_insert(0) += 1;
        }
        eprintln!("  [phase6a] node_relations = {} | {by_type:?}", edges.len());
        for e in edges.iter().take(8) {
            eprintln!(
                "    [{}] {}→{} conf={} meta={:?}",
                e.relation_type.as_str(),
                e.from_node_id,
                e.to_node_id,
                e.confidence,
                e.metadata_json
            );
        }

        assert!(doc.blocks.iter().any(|b| b.kind == NodeKind::Section));
        assert!(doc.blocks.iter().any(|b| b.kind == NodeKind::Paragraph));
    }

    #[test]
    fn cite_and_inline_commands_stay_verbatim_in_paragraphs() {
        let doc = extract(&[(
            "main.tex",
            "\\documentclass{article}\\begin{document}\n\
             As shown in \\cite{smith2020}, the value $\\alpha$ grows~fast (see \\ref{sec:x}).\n\
             \\end{document}",
        )]);
        let p = &find(&doc, NodeKind::Paragraph)[0];
        assert!(p.text.contains("\\cite{smith2020}"), "{}", p.text);
        assert!(p.text.contains("$\\alpha$"));
        assert!(p.text.contains("\\ref{sec:x}"));
        assert!(p.text.contains("grows fast"), "~ → space: {}", p.text);
    }
}
