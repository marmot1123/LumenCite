//! Phase 8b: `tabular` 環境のセル構造化（純関数・DB/pdfium 非依存・CI テスト可能）。
//!
//! `tabular` / `tabular*` / `tabularx` の本文を行 × セルの grid にする。字句規則は親モジュール
//! （バックスラッシュ偶奇パリティ・brace-aware group）を共有する。**誤検出より欠損**:
//! 確信の持てない表は `Err(理由)` で丸ごと諦め（呼び出し側が warning 化）、従来どおり
//! 本体破棄に degrade する。セル内容は段落と同じ方針で LaTeX を温存する（`$..$` verbatim・
//! 装飾コマンドは剥がない・`\label` のみ除去）。
//!
//! ## 意図的な非対応（欠損許容）
//!
//! - `longtable`（`\endhead` 等の独自プロトコル）・`tabu`（deprecated・独自 spec）・
//!   `array`（数式レイアウトであり表データでない）
//! - ネスト環境入りセル（外側 grid を壊すため表ごと skip）
//! - `\multirow` の grid 再解釈（下行の空セルはそのまま。`rowspan` は原文リテラルの記録のみ）
//! - 単位（siunitx `S` 列は未検証 spec 扱い）・表脚注（`\tnote` 等はセル text に verbatim 残留）

use super::{
    collapse_ws, find_unescaped, read_control_word, read_group, skip_optional,
    skip_optional_inline, skip_ws, strip_labels, unescaped,
};

/// 行数の暴走ガード。
const MAX_ROWS: usize = 512;
/// 列数の暴走ガード（spec 展開・実セル数の両方に適用）。
const MAX_COLS: usize = 64;
/// spec の `*{n}{..}` 入れ子深度上限。
const MAX_SPEC_DEPTH: usize = 4;
/// これを超えるスニペット（バイト）は表ごと skip（機械生成の巨大表・暴走ガード）。
pub(super) const MAX_TABULAR_SNIPPET_BYTES: usize = 100_000;
/// 原文スニペットの保存上限（バイト）。超えたら `latex_source` のみ省略し表は保持。
pub(super) const MAX_LATEX_SOURCE_BYTES: usize = 40_000;

/// セル構造化済みの表 1 個。`TexBlock.table` に載り payload_json へ serialize される。
#[derive(Debug, Clone, PartialEq)]
pub struct TexTable {
    /// 列仕様の原文 verbatim（例 `l|cc p{2cm}`）。下流は再パースしない（`alignments` を見る）。
    pub column_spec: String,
    /// 実効列数（spec 検証済みなら spec 由来・未検証なら行の最大実効列数）。
    pub n_columns: i64,
    /// 列型レターの配列（`l`/`c`/`r`/`p`/`m`/`b`/`X`・spec 検証済みのときだけ Some・長さ = n_columns）。
    pub alignments: Option<Vec<String>>,
    pub rows: Vec<TexTableRow>,
    /// 環境丸ごとの原文スニペット（display 数式の `source_provided` と同思想。40k 超は None）。
    pub latex_source: Option<String>,
}

impl TexTable {
    /// spec がホワイトリストで完全パースできたか（confidence 0.9 / 0.8 の分岐）。
    pub fn spec_verified(&self) -> bool {
        self.alignments.is_some()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TexTableRow {
    pub cells: Vec<TexTableCell>,
    /// この行の直前に**全幅**罫線（`\hline`/`\toprule`/`\midrule`/`\bottomrule`/`\specialrule`）が
    /// あったという事実の記録。部分罫線（`\cline`/`\cmidrule`）は消費するが立てない
    /// （部分的事実を全列に昇格しない）。ヘッダ推定はしない。
    pub rule_above: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TexTableCell {
    /// セル本文（LaTeX 温存・`\label` 除去・空白正規化済み。空セルは空文字列で位置保持）。
    pub text: String,
    /// `\multicolumn{n}{spec}{..}` の n（n > 1 のときのみ Some）。
    pub colspan: Option<i64>,
    /// セル全体が `\multirow[..]{n}[..]{..}[..]{..}` 形だったときの n（n > 1 のみ・情報の記録。
    /// grid の再解釈はしない — 下行の空セルは原文どおり残る）。
    pub rowspan: Option<i64>,
}

/// `\begin{env}` の直後（引数の手前）から `\end{env}` の手前までを grid にする。
/// `Err(理由)` = 表ごと skip（呼び出し側で warning 化・従来の本体破棄に degrade）。
pub(super) fn parse_tabular(env: &str, inner: &str) -> Result<TexTable, String> {
    // [pos] と、tabular*/tabularx の {width} を消費して {spec} を読む。
    let mut i = skip_optional(inner, 0);
    if env == "tabular*" || env == "tabularx" {
        let j = skip_ws(inner, i);
        let Some((_, e)) = read_group(inner, j) else {
            return Err(format!("{env}: width argument not found"));
        };
        // 星付き形の [pos] は width の**後**に来る（`\begin{tabular*}{width}[pos]{spec}`）。
        i = skip_optional(inner, e);
    }
    let j = skip_ws(inner, i);
    let Some((spec, after_spec)) = read_group(inner, j) else {
        return Err(format!("{env}: column spec not found"));
    };
    let column_spec = spec.trim().to_string();
    let alignments = parse_column_spec(spec);
    let body = &inner[after_spec..];

    let raw_rows = split_rows(body)?;
    if raw_rows.len() > MAX_ROWS {
        return Err(format!("{env}: more than {MAX_ROWS} rows"));
    }

    let mut rows: Vec<TexTableRow> = Vec::new();
    // 罫線だけの空セグメント（`a & b \\ \hline \\ c & d` の中間など）を drop するとき、
    // その全幅罫線の事実は**次の実在行**に引き継ぐ（捨てると罫線が消える）。
    let mut pending_rule = false;
    for raw in &raw_rows {
        let (mut row, dropped) = process_row(raw)?;
        if dropped {
            pending_rule = pending_rule || row.rule_above;
            continue;
        }
        row.rule_above = row.rule_above || pending_rule;
        pending_rule = false;
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(format!("{env}: no rows recognized"));
    }

    // 実効列数 = セル数 + Σ(colspan−1)。spec 検証済みで期待超過ならパーサの取りこぼしの
    // 強いシグナルなので skip（不足は LaTeX の正規の末尾省略なので保持）。
    let mut max_effective = 0i64;
    for r in &rows {
        let eff: i64 = r.cells.iter().map(|c| c.colspan.unwrap_or(1)).sum();
        if eff as usize > MAX_COLS {
            return Err(format!("{env}: more than {MAX_COLS} columns"));
        }
        if let Some(a) = &alignments {
            if eff > a.len() as i64 {
                return Err(format!(
                    "{env}: row has {eff} effective columns but spec declares {}",
                    a.len()
                ));
            }
        }
        max_effective = max_effective.max(eff);
    }

    let n_columns = match &alignments {
        Some(a) => a.len() as i64,
        None => max_effective,
    };
    Ok(TexTable {
        column_spec,
        n_columns,
        alignments: alignments
            .map(|a| a.into_iter().map(|c| c.to_string()).collect()),
        rows,
        latex_source: None, // 呼び出し側がスニペットを知っているので後付け
    })
}

/// 列仕様のホワイトリストパース。列型レター列を返す。
/// ホワイトリスト外の文字（siunitx `S`・dcolumn `D` 等）を含む spec は None（未検証）。
/// 誤った期待列数を確定するよりは「列数不明」に落とす。
fn parse_column_spec(spec: &str) -> Option<Vec<char>> {
    fn walk(spec: &str, cols: &mut Vec<char>, depth: usize) -> Option<()> {
        if depth > MAX_SPEC_DEPTH {
            return None;
        }
        let bytes = spec.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if cols.len() > MAX_COLS {
                return None;
            }
            match bytes[i] {
                b'l' | b'c' | b'r' | b'X' => {
                    cols.push(bytes[i] as char);
                    i += 1;
                }
                b'p' | b'm' | b'b' => {
                    let j = skip_ws(spec, i + 1);
                    let (_, e) = read_group(spec, j)?;
                    cols.push(bytes[i] as char);
                    i = e;
                }
                b'|' => i += 1,
                b'@' | b'!' | b'>' | b'<' => {
                    let j = skip_ws(spec, i + 1);
                    let (_, e) = read_group(spec, j)?;
                    i = e;
                }
                b'*' => {
                    let j = skip_ws(spec, i + 1);
                    let (n_str, e) = read_group(spec, j)?;
                    let n: usize = n_str.trim().parse().ok()?;
                    // 64 回超の繰り返しは列 64 制限を満たしえない（空 sub の無限繰り返しも遮断）。
                    if n > MAX_COLS {
                        return None;
                    }
                    let j2 = skip_ws(spec, e);
                    let (sub, e2) = read_group(spec, j2)?;
                    for _ in 0..n {
                        walk(sub, cols, depth + 1)?;
                        if cols.len() > MAX_COLS {
                            return None;
                        }
                    }
                    i = e2;
                }
                b if b.is_ascii_whitespace() => i += 1,
                _ => return None,
            }
        }
        Some(())
    }
    let mut cols = Vec::new();
    walk(spec, &mut cols, 0)?;
    if cols.is_empty() || cols.len() > MAX_COLS {
        None
    } else {
        Some(cols)
    }
}

/// 本文を行 × 生セル文字列に分割する。`&`/`\\` は brace depth 0 かつ opaque 区間
/// （`$..$`/`$$..$$`/`\(..\)`/`\[..\]`/`\verb`/`\url` 系引数）の外でだけ構造と見なす。
/// ネスト環境（opaque 外の `\begin{`）・brace 不均衡・未終端 opaque は Err（表ごと skip）。
fn split_rows(body: &str) -> Result<Vec<Vec<String>>, String> {
    let bytes = body.as_bytes();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut cell_start = 0usize;
    let mut brace = 0i64;
    let mut i = 0usize;

    let mut close_row =
        |cur: &mut Vec<String>, cell: &str| {
            cur.push(cell.to_string());
            rows.push(std::mem::take(cur));
        };

    while i < bytes.len() {
        match bytes[i] {
            b'\\' if unescaped(body, i) => {
                // `\\` = 行区切り（brace 内は改行として無視・grid に関与しない）。
                if bytes.get(i + 1) == Some(&b'\\') {
                    if brace == 0 {
                        close_row(&mut cur, &body[cell_start..i]);
                        let mut j = i + 2;
                        if bytes.get(j) == Some(&b'*') {
                            j += 1;
                        }
                        j = skip_optional_inline(body, j);
                        i = j;
                        cell_start = i;
                    } else {
                        i += 2;
                    }
                    continue;
                }
                let (word, after) = read_control_word(body, i + 1);
                if word.is_empty() {
                    match bytes.get(i + 1) {
                        // インライン数式 \( .. \) と display \[ .. \] は opaque。
                        Some(b'(') => match find_unescaped(body, i + 2, "\\)") {
                            Some(e) => i = e + 2,
                            None => return Err("unterminated \\( math in tabular".into()),
                        },
                        Some(b'[') => match find_unescaped(body, i + 2, "\\]") {
                            Some(e) => i = e + 2,
                            None => return Err("unterminated \\[ math in tabular".into()),
                        },
                        // \& \$ \% \_ 等のエスケープ文字はそのままセル本文。
                        Some(_) => {
                            let ch_len =
                                body[i + 1..].chars().next().map_or(1, |c| c.len_utf8());
                            i += 1 + ch_len;
                        }
                        None => i += 1,
                    }
                    continue;
                }
                match word {
                    // ネスト環境はセル分割を必ず壊す — 誤構造より欠損（表ごと skip）。
                    "begin" => {
                        return Err("nested environment in tabular; cells not structured".into())
                    }
                    // \verb<d>..<d> は opaque（`&`/`$` を含みうる）。
                    "verb" => {
                        let mut j = after;
                        if bytes.get(j) == Some(&b'*') {
                            j += 1;
                        }
                        match body[j..].chars().next() {
                            Some(delim) if delim != '\n' => {
                                let dstart = j + delim.len_utf8();
                                match body[dstart..].find(delim) {
                                    Some(rel) => i = dstart + rel + delim.len_utf8(),
                                    None => return Err("unterminated \\verb in tabular".into()),
                                }
                            }
                            _ => i = j,
                        }
                    }
                    // URL 系の引数は `$`/`%`/`&` を含みうるので opaque に消費。
                    // url パッケージは `\url|...|` の任意デリミタ形も許す（brace が無ければ
                    // \verb と同じ規則で読む）。
                    "url" | "path" | "nolinkurl" | "href" => {
                        let j = skip_ws(body, after);
                        let mut e = match read_group(body, j) {
                            Some((_, e)) => e,
                            None => match body[after..].chars().next() {
                                Some(delim)
                                    if word != "href"
                                        && delim != '\n'
                                        && !delim.is_ascii_whitespace() =>
                                {
                                    let dstart = after + delim.len_utf8();
                                    match body[dstart..].find(delim) {
                                        Some(rel) => dstart + rel + delim.len_utf8(),
                                        None => {
                                            return Err(format!(
                                                "unterminated \\{word} in tabular"
                                            ))
                                        }
                                    }
                                }
                                _ => after,
                            },
                        };
                        if word == "href" {
                            let j2 = skip_ws(body, e);
                            if let Some((_, e2)) = read_group(body, j2) {
                                e = e2;
                            }
                        }
                        i = e;
                    }
                    "tabularnewline" => {
                        if brace == 0 {
                            close_row(&mut cur, &body[cell_start..i]);
                            i = skip_optional_inline(body, after);
                            cell_start = i;
                        } else {
                            i = after;
                        }
                    }
                    _ => i = after,
                }
            }
            b'$' if unescaped(body, i) => {
                // `$..$` / `$$..$$` は opaque（`\$` はパリティで文字扱い）。
                if bytes.get(i + 1) == Some(&b'$') {
                    match find_unescaped(body, i + 2, "$$") {
                        Some(e) => i = e + 2,
                        None => return Err("unterminated $$ math in tabular".into()),
                    }
                } else {
                    match find_unescaped(body, i + 1, "$") {
                        Some(e) => i = e + 1,
                        None => return Err("unterminated $ math in tabular".into()),
                    }
                }
            }
            b'{' if unescaped(body, i) => {
                brace += 1;
                i += 1;
            }
            b'}' if unescaped(body, i) => {
                brace -= 1;
                if brace < 0 {
                    return Err("unbalanced braces in tabular".into());
                }
                i += 1;
            }
            b'&' if unescaped(body, i) && brace == 0 => {
                cur.push(body[cell_start..i].to_string());
                i += 1;
                cell_start = i;
            }
            _ => i += 1,
        }
    }
    if brace != 0 {
        return Err("unbalanced braces in tabular".into());
    }
    // 最終セグメント（`\\` なしで \end に到達する行）。罫線と空白だけなら行にしない。
    let tail = &body[cell_start..];
    if !cur.is_empty() || !strip_leading_rules(tail).0.trim().is_empty() {
        close_row(&mut cur, tail);
    }
    Ok(rows)
}

/// 行の生セル列 → セル構造。行頭の罫線群を消費して `rule_above` を記録する。
/// 戻り値 (row, dropped): dropped = 空行（`\\` 直後に \end 等・grid に寄与しない）。
fn process_row(raw_cells: &[String]) -> Result<(TexTableRow, bool), String> {
    let mut cells: Vec<TexTableCell> = Vec::new();
    let mut rule_above = false;
    for (ci, raw) in raw_cells.iter().enumerate() {
        let piece = if ci == 0 {
            let (rest, full_rule) = strip_leading_rules(raw);
            rule_above = full_rule;
            rest
        } else {
            raw.clone()
        };
        cells.push(process_cell(&piece)?);
    }
    // `\\` と `\end` の間の空セグメント等: セル 1 個・空・colspan 無しの行は grid に寄与しない。
    let dropped = cells.len() == 1
        && cells[0].text.is_empty()
        && cells[0].colspan.is_none()
        && cells[0].rowspan.is_none();
    Ok((TexTableRow { cells, rule_above }, dropped))
}

/// 行頭の罫線・スペーシングコマンド群を消費する。
/// 戻り値 (残り, 全幅罫線があったか)。`\cline`/`\cmidrule`/`\noalign` は消費するだけで
/// 全幅罫線には数えない（部分的事実を全列に昇格しない）。
fn strip_leading_rules(s: &str) -> (String, bool) {
    let mut i = 0usize;
    let mut full = false;
    loop {
        let j = skip_ws(s, i);
        let bytes = s.as_bytes();
        if bytes.get(j) != Some(&b'\\') || !unescaped(s, j) {
            i = j;
            break;
        }
        let (word, after) = read_control_word(s, j + 1);
        let next = match word {
            "hline" => {
                full = true;
                after
            }
            "toprule" | "midrule" | "bottomrule" => {
                full = true;
                skip_optional(s, after)
            }
            "specialrule" => {
                full = true;
                let mut k = after;
                for _ in 0..3 {
                    let k2 = skip_ws(s, k);
                    match read_group(s, k2) {
                        Some((_, e)) => k = e,
                        None => break,
                    }
                }
                k
            }
            "cline" => {
                let k = skip_ws(s, after);
                match read_group(s, k) {
                    Some((_, e)) => e,
                    None => break,
                }
            }
            "cmidrule" => {
                // \cmidrule[wd](trim){i-j} — 光学引数と (lr) トリム記法も消費。
                let mut k = skip_optional(s, after);
                k = skip_paren_group(s, k);
                let k2 = skip_ws(s, k);
                match read_group(s, k2) {
                    Some((_, e)) => e,
                    None => break,
                }
            }
            "noalign" => {
                let k = skip_ws(s, after);
                match read_group(s, k) {
                    Some((_, e)) => e,
                    None => break,
                }
            }
            // colortbl の行頭色指定。消費しないと直後の \multicolumn 検出を隠す。
            // 罫線ではないので rule_above は立てない。
            "rowcolor" => {
                let k = skip_optional(s, after);
                let k2 = skip_ws(s, k);
                match read_group(s, k2) {
                    Some((_, e)) => e,
                    None => break,
                }
            }
            _ => break,
        };
        i = next;
    }
    (s[i..].to_string(), full)
}

/// `(lr)` のような括弧グループを 1 個消費する（無ければそのまま）。
fn skip_paren_group(s: &str, i: usize) -> usize {
    let j = skip_ws(s, i);
    if s.as_bytes().get(j) != Some(&b'(') {
        return i;
    }
    match s[j + 1..].find(')') {
        Some(rel) => j + 1 + rel + 1,
        None => i,
    }
}

/// セル 1 個: `\multicolumn`（colspan）→ content 内の whole-cell `\multirow`（rowspan）→
/// `\label` 除去 + 空白正規化。`\multicolumn{n}` の n が整数リテラル（1..=64）でなければ
/// 列整合が検証不能になるので Err（表ごと skip）。
fn process_cell(raw: &str) -> Result<TexTableCell, String> {
    let trimmed = raw.trim();
    let mut colspan: Option<i64> = None;
    let mut rowspan: Option<i64> = None;

    let mut content: String = match leading_control_word(trimmed) {
        Some(("multicolumn", after)) => match parse_multicolumn(trimmed, after) {
            Some((n, inner)) => {
                if !(1..=MAX_COLS as i64).contains(&n) {
                    return Err("\\multicolumn with out-of-range span".into());
                }
                if n > 1 {
                    colspan = Some(n);
                }
                inner
            }
            None => return Err("\\multicolumn with non-literal span".into()),
        },
        _ => trimmed.to_string(),
    };
    // \multicolumn の content にも whole-cell \multirow 判定を再適用（縦横結合ヘッダの定番形）。
    // 非整数 n・非 whole-cell 形は原文温存（rowspan は情報にすぎず grid を壊さない）。
    let snapshot = content.trim().to_string();
    if let Some(("multirow", after)) = leading_control_word(&snapshot) {
        if let Some((n, inner)) = parse_multirow(&snapshot, after) {
            if n > 1 {
                rowspan = Some(n);
            }
            content = inner;
        }
    }
    Ok(TexTableCell {
        text: collapse_ws(&strip_labels(&content)),
        colspan,
        rowspan,
    })
}

/// 文字列先頭（trim 済み前提）の control word。`(word, `\` と word の直後の位置)`。
fn leading_control_word(s: &str) -> Option<(&str, usize)> {
    if !s.starts_with('\\') {
        return None;
    }
    let (word, after) = read_control_word(s, 1);
    if word.is_empty() {
        None
    } else {
        Some((word, after))
    }
}

/// `\multicolumn{n}{spec}{content}`。content の後にトークンが残る形
/// （`\multicolumn{1}{c}{Mass}\tnote{a}` 等）は合法 LaTeX なので、残りを本文に連結して
/// colspan を保つ（表ごと skip すると threeparttable 系の表が全滅する）。
fn parse_multicolumn(s: &str, after_word: usize) -> Option<(i64, String)> {
    let j = skip_ws(s, after_word);
    let (n_str, e1) = read_group(s, j)?;
    let n: i64 = n_str.trim().parse().ok()?;
    let j2 = skip_ws(s, e1);
    let (_spec, e2) = read_group(s, j2)?;
    let j3 = skip_ws(s, e2);
    let (content, e3) = read_group(s, j3)?;
    let trailing = s[e3..].trim();
    if trailing.is_empty() {
        Some((n, content.to_string()))
    } else {
        Some((n, format!("{content}{trailing}")))
    }
}

/// `\multirow[vpos]{nrows}[bigstruts]{width}[vmove]{content}`（セル全体のみ）。
fn parse_multirow(s: &str, after_word: usize) -> Option<(i64, String)> {
    let j = skip_optional(s, after_word);
    let j1 = skip_ws(s, j);
    let (n_str, e1) = read_group(s, j1)?;
    let n: i64 = n_str.trim().parse().ok()?;
    let k = skip_optional(s, e1);
    let k1 = skip_ws(s, k);
    let (_width, e2) = read_group(s, k1)?;
    let k2 = skip_optional(s, e2);
    let k3 = skip_ws(s, k2);
    let (content, e3) = read_group(s, k3)?;
    if !s[e3..].trim().is_empty() {
        return None;
    }
    Some((n, content.to_string()))
}

/// 読み出し用の可読テキスト（セルを ` | `・行を改行で結合。LaTeX 温存）。
pub(super) fn table_plain_text(table: &TexTable) -> String {
    table
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(env: &str, inner: &str) -> Result<TexTable, String> {
        parse_tabular(env, inner)
    }

    fn cell_texts(t: &TexTable) -> Vec<Vec<&str>> {
        t.rows
            .iter()
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect())
            .collect()
    }

    // ── 基本形 ──

    #[test]
    fn basic_two_by_two() {
        let t = parse("tabular", "{cc} a & b \\\\ c & d ").expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"], vec!["c", "d"]]);
        assert_eq!(t.n_columns, 2);
        assert!(t.spec_verified());
        assert_eq!(
            t.alignments.as_deref(),
            Some(&["c".to_string(), "c".to_string()][..])
        );
        assert_eq!(t.column_spec, "cc");
    }

    #[test]
    fn plain_text_joins_cells_and_rows() {
        let t = parse("tabular", "{cc} a & b \\\\ c & d ").expect("parse");
        assert_eq!(table_plain_text(&t), "a | b\nc | d");
    }

    #[test]
    fn trailing_row_without_linebreak_and_empty_tail() {
        // 最終行は `\\` なしで終わるのが普通。`\\` + 罫線だけの末尾は行にしない。
        let t = parse("tabular", "{ll} a & b \\\\ c & d \\\\ \\hline ").expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn empty_cells_keep_position() {
        let t = parse("tabular", "{ccc} a & & c \\\\ & b & ").expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["a", "", "c"], vec!["", "b", ""]]);
    }

    #[test]
    fn short_rows_are_kept() {
        // LaTeX は行末セルの省略を許す — 欠損でなく正規の形。
        let t = parse("tabular", "{lcc} Name & Value \\\\ a & 1 & 2 ").expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["Name", "Value"], vec!["a", "1", "2"]]);
        assert_eq!(t.n_columns, 3);
    }

    // ── 列仕様 ──

    #[test]
    fn column_spec_with_pipes_p_and_repeat() {
        let t = parse(
            "tabular",
            "{|l|p{2cm}|*{2}{c}|@{\\hspace{1em}}r|} a & b & c & d & e ",
        )
        .expect("parse");
        assert!(t.spec_verified());
        assert_eq!(
            t.alignments.as_deref(),
            Some(
                &[
                    "l".to_string(),
                    "p".to_string(),
                    "c".to_string(),
                    "c".to_string(),
                    "r".to_string()
                ][..]
            )
        );
        assert_eq!(t.n_columns, 5);
    }

    #[test]
    fn unknown_spec_letter_falls_back_to_row_derived_columns() {
        // siunitx の S 列などは列数を確定しない（誤った期待列数を出さない）。
        let t = parse("tabular", "{lS} a & 1.0 \\\\ b & 2.0 ").expect("parse");
        assert!(!t.spec_verified());
        assert!(t.alignments.is_none());
        assert_eq!(t.n_columns, 2);
    }

    #[test]
    fn huge_star_repeat_is_rejected_before_expansion() {
        let r = parse("tabular", "{*{9999999}{c}} a & b ");
        // 展開途中で 64 列超過 → spec 未検証に落ち、行由来の 2 列で保持される。
        let t = r.expect("kept with unverified spec");
        assert!(!t.spec_verified());
        assert_eq!(t.n_columns, 2);
    }

    #[test]
    fn tabular_star_consumes_width_argument() {
        let t = parse(
            "tabular*",
            "{\\textwidth}{l@{\\extracolsep{\\fill}}r} a & b ",
        )
        .expect("parse");
        assert_eq!(t.n_columns, 2);
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"]]);
    }

    #[test]
    fn tabularx_x_column_counts() {
        let t = parse("tabularx", "{\\textwidth}{lX} a & b ").expect("parse");
        assert!(t.spec_verified());
        assert_eq!(
            t.alignments.as_deref(),
            Some(&["l".to_string(), "X".to_string()][..])
        );
    }

    #[test]
    fn position_optional_argument_is_consumed() {
        let t = parse("tabular", "[t]{cc} a & b ").expect("parse");
        assert_eq!(t.n_columns, 2);
    }

    // ── colspan / rowspan ──

    #[test]
    fn multicolumn_yields_colspan() {
        let t = parse(
            "tabular",
            "{ccc} \\multicolumn{2}{c}{Header} & x \\\\ a & b & c ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells[0].text, "Header");
        assert_eq!(t.rows[0].cells[0].colspan, Some(2));
        assert_eq!(t.rows[0].cells[1].text, "x");
        // 実効列数 = 2 + 1 = 3 で spec と整合。
        assert!(t.spec_verified());
    }

    #[test]
    fn multirow_whole_cell_yields_rowspan() {
        let t = parse(
            "tabular",
            "{cc} \\multirow{2}{*}{X} & a \\\\ & b ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells[0].text, "X");
        assert_eq!(t.rows[0].cells[0].rowspan, Some(2));
        // 下行の空セルは原文どおり残る（grid 再解釈をしない）。
        assert_eq!(t.rows[1].cells[0].text, "");
    }

    #[test]
    fn multirow_with_bigstruts_slot() {
        let t = parse("tabular", "{cc} \\multirow{2}[2]{*}{X} & a ").expect("parse");
        assert_eq!(t.rows[0].cells[0].text, "X");
        assert_eq!(t.rows[0].cells[0].rowspan, Some(2));
    }

    #[test]
    fn multicolumn_wrapping_multirow_combines() {
        let t = parse(
            "tabular",
            "{ccc} \\multicolumn{2}{c}{\\multirow{2}{*}{X}} & a ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells[0].text, "X");
        assert_eq!(t.rows[0].cells[0].colspan, Some(2));
        assert_eq!(t.rows[0].cells[0].rowspan, Some(2));
    }

    #[test]
    fn multicolumn_non_literal_span_skips_table() {
        assert!(parse("tabular", "{cc} \\multicolumn{\\n}{c}{X} & a ").is_err());
    }

    #[test]
    fn multirow_non_literal_span_keeps_cell_verbatim() {
        let t = parse("tabular", "{cc} \\multirow{\\n}{*}{X} & a ").expect("parse");
        assert!(t.rows[0].cells[0].text.contains("\\multirow"));
        assert_eq!(t.rows[0].cells[0].rowspan, None);
    }

    // ── 罫線 ──

    #[test]
    fn booktabs_rules_set_rule_above_and_stay_out_of_cells() {
        let t = parse(
            "tabular",
            "{cc} \\toprule a & b \\\\ \\midrule c & d \\\\ \\bottomrule ",
        )
        .expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"], vec!["c", "d"]]);
        assert!(t.rows[0].rule_above);
        assert!(t.rows[1].rule_above);
    }

    #[test]
    fn double_hline_and_cline() {
        let t = parse(
            "tabular",
            "{cc} \\hline\\hline a & b \\\\ \\cline{1-2} c & d ",
        )
        .expect("parse");
        assert!(t.rows[0].rule_above);
        // \cline は部分罫線 — 消費はするが rule_above には数えない。
        assert!(!t.rows[1].rule_above);
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn cmidrule_with_trim_and_noalign_idiom() {
        // Springer/A&A 系の \noalign{\smallskip}\hline\noalign{\smallskip} イディオム。
        let t = parse(
            "tabular",
            "{cc} a & b \\\\ \\noalign{\\smallskip}\\hline\\noalign{\\smallskip} c & d \\\\ \\cmidrule(lr){1-1} e & f ",
        )
        .expect("parse");
        assert_eq!(
            cell_texts(&t),
            vec![vec!["a", "b"], vec!["c", "d"], vec!["e", "f"]]
        );
        assert!(t.rows[1].rule_above);
        assert!(!t.rows[2].rule_above);
    }

    // ── opaque 区間・エスケープ ──

    #[test]
    fn inline_math_protects_ampersand_and_linebreak() {
        let t = parse(
            "tabular",
            "{cc} $\\begin{smallmatrix} a & b \\\\ c & d \\end{smallmatrix}$ & x ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells.len(), 2);
        assert!(t.rows[0].cells[0].text.contains("smallmatrix"));
        assert_eq!(t.rows[0].cells[1].text, "x");
    }

    #[test]
    fn escaped_ampersand_and_dollar_are_literal() {
        // `\&` は文字・`\$` は opaque を開かない（`\$5 & \$10` は 2 セル）。
        let t = parse("tabular", "{cc} A\\&B & \\$5 \\\\ \\$10 & x ").expect("parse");
        assert_eq!(
            cell_texts(&t),
            vec![vec!["A\\&B", "\\$5"], vec!["\\$10", "x"]]
        );
    }

    #[test]
    fn verb_and_url_are_opaque() {
        let t = parse(
            "tabular",
            "{cc} \\verb|a&b| & \\url{http://ex.com/?a=$q&b=2} \\\\ c & d ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells.len(), 2);
        assert!(t.rows[0].cells[0].text.contains("a&b"));
        assert!(t.rows[0].cells[1].text.contains("ex.com"));
        assert_eq!(cell_texts(&t)[1], vec!["c", "d"]);
    }

    #[test]
    fn linebreak_inside_braces_does_not_split_row() {
        // \makecell{a\\b} 型: brace 内の `\\` は行区切りでない。
        let t = parse("tabular", "{cc} \\makecell{a\\\\b} & x ").expect("parse");
        assert_eq!(t.rows.len(), 1);
        assert!(t.rows[0].cells[0].text.contains("\\makecell{a\\\\b}"));
    }

    #[test]
    fn linebreak_optional_dimension_is_consumed() {
        let t = parse("tabular", "{cc} a & b \\\\[4pt] c & d ").expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn tabularnewline_splits_rows() {
        let t = parse("tabular", "{cc} a & b \\tabularnewline c & d ").expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn label_is_stripped_and_decorations_kept() {
        let t = parse(
            "tabular",
            "{cc} \\textbf{bold} \\label{tab:x} & $\\alpha$ ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells[0].text, "\\textbf{bold}");
        assert_eq!(t.rows[0].cells[1].text, "$\\alpha$");
    }

    // ── skip 条件（誤検出より欠損） ──

    #[test]
    fn nested_environment_skips_table() {
        assert!(parse("tabular", "{cc} \\begin{tabular}{c} x \\end{tabular} & a ").is_err());
        assert!(parse("tabular", "{cc} \\begin{minipage}{3cm} p \\end{minipage} & a ").is_err());
    }

    #[test]
    fn unterminated_math_skips_table() {
        assert!(parse("tabular", "{cc} $x & a ").is_err());
        assert!(parse("tabular", "{cc} $$x & a ").is_err());
    }

    #[test]
    fn unbalanced_braces_skip_table() {
        assert!(parse("tabular", "{cc} a{ & b ").is_err());
        assert!(parse("tabular", "{cc} a} & b ").is_err());
    }

    #[test]
    fn effective_columns_beyond_spec_skip_table() {
        // 取りこぼしの強シグナル: spec 2 列に 3 セルの行。
        assert!(parse("tabular", "{cc} a & b & c ").is_err());
        // \multicolumn 由来の超過も同じ。
        assert!(parse("tabular", "{cc} \\multicolumn{2}{c}{H} & x ").is_err());
    }

    #[test]
    fn empty_tabular_skips() {
        assert!(parse("tabular", "{cc} ").is_err());
        assert!(parse("tabular", "{cc} \\hline ").is_err());
    }

    #[test]
    fn missing_spec_skips() {
        assert!(parse("tabular", " a & b ").is_err());
    }

    // ── レビュー修正の回帰 ──

    #[test]
    fn tabular_star_position_optional_after_width() {
        // 星付き形の [pos] は width の**後**（`{width}[pos]{spec}`）に来る。
        let t = parse("tabular*", "{\\textwidth}[t]{cc} a & b ").expect("parse");
        assert_eq!(t.n_columns, 2);
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"]]);
        assert!(t.spec_verified());
    }

    #[test]
    fn tabularx_position_optional_after_width() {
        let t = parse("tabularx", "{\\linewidth}[b]{lX} a & b ").expect("parse");
        assert!(t.spec_verified());
        assert_eq!(t.n_columns, 2);
    }

    #[test]
    fn multicolumn_trailing_token_keeps_colspan() {
        // `\multicolumn{..}{..}{X}\tnote{a}` は合法 LaTeX — 残りを本文に連結して colspan を保つ
        // （表ごと skip すると threeparttable 系が全滅する）。
        let t = parse(
            "tabular",
            "{ccc} \\multicolumn{2}{c}{Mass}\\tnote{a} & x \\\\ a & b & c ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells[0].colspan, Some(2));
        assert!(t.rows[0].cells[0].text.contains("Mass"));
        assert!(t.rows[0].cells[0].text.contains("\\tnote{a}"));
        assert_eq!(t.rows[0].cells[1].text, "x");
        assert!(t.spec_verified());
    }

    #[test]
    fn multicolumn_huge_span_is_rejected_without_panic() {
        // i64::MAX を 2 セルに置いても overflow panic せず表 skip（範囲検査が sum 前に弾く）。
        let r = parse(
            "tabular",
            "{cc} \\multicolumn{9223372036854775807}{c}{X} & \
             \\multicolumn{9223372036854775807}{c}{Y} ",
        );
        assert!(r.is_err());
    }

    #[test]
    fn rowcolor_at_row_start_is_consumed_and_reveals_multicolumn() {
        // \rowcolor を消費しないと直後の \multicolumn を隠す。罫線ではないので rule_above は立てない。
        let t = parse(
            "tabular",
            "{ccc} \\rowcolor{gray!20}\\multicolumn{2}{c}{H} & x \\\\ a & b & c ",
        )
        .expect("parse");
        assert_eq!(t.rows[0].cells[0].colspan, Some(2));
        assert_eq!(t.rows[0].cells[0].text, "H");
        assert!(!t.rows[0].rule_above);
        assert_eq!(t.rows[0].cells[1].text, "x");
    }

    #[test]
    fn url_pipe_delimiter_form_is_opaque() {
        // url パッケージは brace 無しの `\url|..|` 任意デリミタ形も許す（\verb 規則）。
        let t = parse("tabular", "{cc} \\url|http://x/?a&b| & y \\\\ c & d ").expect("parse");
        assert_eq!(t.rows[0].cells.len(), 2);
        assert!(t.rows[0].cells[0].text.contains("a&b"));
        assert_eq!(t.rows[0].cells[1].text, "y");
        assert_eq!(cell_texts(&t)[1], vec!["c", "d"]);
    }

    #[test]
    fn full_rule_before_empty_segment_carries_to_next_row() {
        // `a & b \\ \hline \\ c & d`: \hline と次の `\\` の間は空セグメント（drop）だが、
        // その全幅罫線の事実は次の実在行に引き継ぐ（捨てると罫線が消える）。
        let t = parse("tabular", "{cc} a & b \\\\ \\hline \\\\ c & d ").expect("parse");
        assert_eq!(cell_texts(&t), vec![vec!["a", "b"], vec!["c", "d"]]);
        assert!(!t.rows[0].rule_above);
        assert!(t.rows[1].rule_above);
    }

}
