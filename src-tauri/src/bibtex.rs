use std::collections::{HashMap, HashSet};

use nom_bibtex::Bibtex;
use sqlx::{Row, SqlitePool};

use crate::db::entries::{create_entry, get_entry};
use crate::models::{AuthorInput, EntryDetail, EntryInput, ImportResult};

pub async fn import_bibtex(pool: &SqlitePool, content: &str) -> Result<ImportResult, String> {
    let bib = Bibtex::parse(content).map_err(|e| format!("BibTeX 解析エラー: {:?}", e))?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut errors: Vec<String> = Vec::new();

    // 既存の固定 cite key を予約集合に読み込み、インポート中に確定したキーも逐次追加して、
    // .bib 内・DB 既存ともに衝突しないよう接尾辞 a/b/c で一意化する。
    let mut used: HashSet<String> = sqlx::query_scalar::<_, String>(
        "SELECT citation_key FROM entries WHERE citation_key IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?
    .into_iter()
    .collect();

    for entry in bib.bibliographies() {
        match bibliography_to_input(entry) {
            Some(mut input) => {
                // 元 .bib のキーをサニタイズして保持。衝突したら一意化する。
                if let Some(base) =
                    crate::db::entries::sanitize_citation_key(entry.citation_key())
                {
                    input.citation_key = Some(dedup_key(&base, &mut used));
                }
                match create_entry(pool, &input).await {
                    Ok(_) => imported += 1,
                    Err(e) => {
                        skipped += 1;
                        errors.push(format!("{}: {}", entry.citation_key(), e));
                    }
                }
            }
            None => {
                skipped += 1;
            }
        }
    }

    Ok(ImportResult { imported, skipped, errors })
}

fn bibliography_to_input(entry: &nom_bibtex::Bibliography) -> Option<EntryInput> {
    let tags = entry.tags();

    let title = tags.get("title")?.trim().to_string();
    if title.is_empty() {
        return None;
    }

    let entry_type = map_entry_type(entry.entry_type());
    let authors: Vec<AuthorInput> = tags
        .get("author")
        .map(|a| parse_authors(a))
        .unwrap_or_default();

    let year = tags
        .get("year")
        .and_then(|y| y.trim().parse::<i64>().ok());

    let doi      = non_empty(tags.get("doi"));
    let arxiv_id = non_empty(tags.get("eprint")).or_else(|| non_empty(tags.get("arxivid")));
    let isbn     = non_empty(tags.get("isbn"));
    let url      = non_empty(tags.get("url"));
    let abstract_ = non_empty(tags.get("abstract"));
    let notes    = non_empty(tags.get("note").or_else(|| tags.get("annote")));

    // Fields stored as extra_fields (journal, booktitle, etc.)
    let extra_keys = [
        "journal", "booktitle", "publisher", "volume", "number",
        "pages", "address", "school", "institution", "edition",
        "series", "chapter", "month", "organization", "howpublished",
    ];
    let known_keys = [
        "title", "author", "year", "doi", "eprint", "arxivid",
        "isbn", "url", "abstract", "note", "annote",
    ];
    let mut extra_fields: HashMap<String, String> = HashMap::new();
    for key in &extra_keys {
        if let Some(val) = non_empty(tags.get(*key)) {
            extra_fields.insert(key.to_string(), val);
        }
    }
    // Any completely unknown fields also go into extra_fields
    for (key, val) in tags {
        if !known_keys.contains(&key.as_str())
            && !extra_keys.contains(&key.as_str())
            && !val.trim().is_empty()
        {
            extra_fields.insert(key.clone(), val.trim().to_string());
        }
    }

    Some(EntryInput {
        title,
        entry_type,
        authors: if authors.is_empty() { None } else { Some(authors) },
        year,
        doi,
        arxiv_id,
        isbn,
        url,
        abstract_,
        notes,
        extra_fields,
        ..Default::default()
    })
}

fn map_entry_type(bib_type: &str) -> String {
    match bib_type.to_lowercase().as_str() {
        "article" | "periodical" | "suppperiodical" => "article",
        "book" | "booklet" | "collection" | "mvbook" | "mvcollection"
        | "reference" | "mvreference" => "book",
        "incollection" | "inbook" | "suppbook" | "suppcollection"
        | "inreference" => "bookSection",
        "inproceedings" | "conference" | "proceedings" | "mvproceedings" => "inproceedings",
        "phdthesis" | "mastersthesis" | "thesis" => "thesis",
        "techreport" | "report" => "report",
        "unpublished" => "manuscript",
        "patent" => "patent",
        "standard" => "standard",
        "dataset" => "dataset",
        "software" => "computerProgram",
        "online" | "electronic" | "www" | "webpage" => "webpage",
        _ => "misc",
    }
    .to_string()
}

/// BibTeX の `author = {...}` 値を `AuthorInput` のリストに分解する。
///
/// v0.3.0 で `is_organization` 検出と family/given 分解を追加。
/// 区切り " and " は **波括弧の外側でのみ** 効くようにし、団体名内の "and"
/// （例: `{Smith and Jones Inc}`）を保護する。各トークンに対し:
///
/// - `{...}` で完全に囲まれていれば団体著者として `is_organization=true`、
///   内側のテキストを `name` に格納（CSL の literal 相当）
/// - "Last, First" 形式ならカンマで分割し `family_name`/`given_name` を埋めつつ
///   `name` は "First Last" の表示順で組み立てる
/// - "First Last" 形式は分割せず `name` のみに入れる（誤判定を避ける）
fn parse_authors(author_str: &str) -> Vec<AuthorInput> {
    split_authors(author_str)
        .into_iter()
        .filter_map(|raw| {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }

            // 団体著者リテラル: `{...}` で完全に囲まれている
            if raw.starts_with('{') && raw.ends_with('}') {
                let inner = raw[1..raw.len() - 1].trim();
                if inner.is_empty() {
                    return None;
                }
                return Some(AuthorInput {
                    name: inner.to_string(),
                    is_organization: true,
                    ..Default::default()
                });
            }

            // 個人著者
            if let Some(comma) = raw.find(',') {
                let last = raw[..comma].trim();
                let first = raw[comma + 1..].trim();
                if first.is_empty() {
                    Some(AuthorInput {
                        name: last.to_string(),
                        family_name: Some(last.to_string()),
                        ..Default::default()
                    })
                } else {
                    Some(AuthorInput {
                        name: format!("{} {}", first, last),
                        given_name: Some(first.to_string()),
                        family_name: Some(last.to_string()),
                        ..Default::default()
                    })
                }
            } else {
                Some(AuthorInput {
                    name: raw.to_string(),
                    ..Default::default()
                })
            }
        })
        .collect()
}

/// `" and "` で著者を分割する。ただし波括弧の中の "and" は無視する
/// （`{Smith and Jones Inc}` を 1 トークン扱いする）。
fn split_authors(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
            }
            b' ' if depth == 0 && bytes.get(i..i + 5) == Some(b" and ") => {
                parts.push(&s[start..i]);
                start = i + 5;
                i += 5;
            }
            _ => {
                // UTF-8 連続バイトを安全に飛ばす
                i += utf8_char_len(bytes[i]);
            }
        }
    }
    parts.push(&s[start..]);
    parts
}

fn utf8_char_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xC0 {
        1 // 連続バイト単独は不正だがループを進めるため 1 にする
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    }
}

fn non_empty(opt: Option<&String>) -> Option<String> {
    opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

// ── Export ────────────────────────────────────────────────────────────────────

pub async fn export_bibtex(
    pool: &SqlitePool,
    entry_ids: Option<Vec<i64>>,
) -> Result<String, String> {
    let ids: Vec<i64> = match entry_ids {
        Some(ids) => ids,
        None => sqlx::query(
            "SELECT id FROM entries
             WHERE deleted_at IS NULL
             ORDER BY created_at DESC",
        )
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|r| r.get::<i64, _>("id"))
        .collect(),
    };

    let mut entries = Vec::with_capacity(ids.len());
    for id in ids {
        entries.push(get_entry(pool, id).await.map_err(|e| e.to_string())?);
    }

    let keys = assign_keys(&entries);
    let parts: Vec<String> = entries
        .iter()
        .zip(keys.iter())
        .map(|(entry, key)| entry_to_bibtex(entry, key))
        .collect();

    Ok(parts.join("\n"))
}

/// `(ピン留めキー, 自動 base)` の列に、export と同じ規則で最終的な cite key を割り当てる。
/// ピン留め済みキー（`Some`）を先に予約し、自動（`None`）は base から `a`/`b`/`c`… の
/// 接尾辞でファイル内一意化する。返り値は入力と同じ並び。export とプレビュー
/// （`resolve_citation_key`）でこの 1 関数を共有し、両者のキーが必ず一致するようにする。
fn assign_keys_from(items: &[(Option<String>, String)]) -> Vec<String> {
    let mut used: HashSet<String> = items.iter().filter_map(|(k, _)| k.clone()).collect();
    items
        .iter()
        .map(|(pinned, base)| match pinned {
            Some(k) => k.clone(),
            None => dedup_key(base, &mut used),
        })
        .collect()
}

/// 渡された順序の `EntryDetail` 群に export と同じ cite key を割り当てる。
fn assign_keys(entries: &[EntryDetail]) -> Vec<String> {
    let items: Vec<(Option<String>, String)> = entries
        .iter()
        .map(|e| (e.citation_key.clone(), make_citation_key(e)))
        .collect();
    assign_keys_from(&items)
}

#[derive(sqlx::FromRow)]
struct KeyRow {
    id: i64,
    title: String,
    year: Option<i64>,
    citation_key: Option<String>,
    first_author: Option<String>,
}

/// 指定エントリが `.bib` 同期（ゴミ箱を除く全エントリ書き出し）で実際に割り当てられる
/// cite key を返す。`export_bibtex(None)` と同じ並び・同じ衝突回避を再現するため、
/// 詳細ビューに「実際に書き出されるキー」を表示・コピーできる。
/// 全 `EntryDetail` を読み込む代わりに base 生成に必要な列だけを取得して軽量化する。
pub async fn resolve_citation_key(pool: &SqlitePool, entry_id: i64) -> Result<String, String> {
    let rows: Vec<KeyRow> = sqlx::query_as(
        "SELECT e.id, e.title, e.year, e.citation_key,
                (SELECT a.name FROM entry_authors ea
                   JOIN authors a ON a.id = ea.author_id
                  WHERE ea.entry_id = e.id ORDER BY ea.position LIMIT 1) AS first_author
         FROM entries e
         WHERE e.deleted_at IS NULL
         ORDER BY e.created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let items: Vec<(Option<String>, String)> = rows
        .iter()
        .map(|r| {
            (
                r.citation_key.clone(),
                make_base_key(r.first_author.as_deref(), &r.title, r.year),
            )
        })
        .collect();
    let keys = assign_keys_from(&items);

    if let Some(key) = rows.iter().zip(keys).find(|(r, _)| r.id == entry_id).map(|(_, k)| k) {
        return Ok(key);
    }

    // ゴミ箱内など同期対象外のエントリ: ピン留めキー、なければ自動 base を返す。
    let entry = get_entry(pool, entry_id).await.map_err(|e| e.to_string())?;
    Ok(match &entry.citation_key {
        Some(k) => k.clone(),
        None => make_citation_key(&entry),
    })
}

/// `base` が未使用ならそのまま、使われていれば `base` + `a`/`b`/`c`… と接尾辞を付けて
/// 未使用のキーを返す。返したキーは `used` に登録する。
fn dedup_key(base: &str, used: &mut HashSet<String>) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    let mut n = 1usize;
    loop {
        let candidate = format!("{}{}", base, suffix_letters(n));
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// 1 → "a", 26 → "z", 27 → "aa" のスプレッドシート列方式で接尾辞を生成する。
fn suffix_letters(mut n: usize) -> String {
    let mut s = Vec::new();
    while n > 0 {
        n -= 1;
        s.push(b'a' + (n % 26) as u8);
        n /= 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

/// 全エントリ（ゴミ箱を除く）の BibTeX を指定パスへ書き出す。
/// 部分書き込みによる壊れたファイルを避けるため、隣に `.tmp` を作って rename する。
pub async fn sync_bibtex(pool: &SqlitePool, path: &std::path::Path) -> Result<(), String> {
    let content = export_bibtex(pool, None).await?;

    let parent = path
        .parent()
        .ok_or_else(|| "同期先パスの親ディレクトリが取得できません".to_string())?;
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| "同期先パスのファイル名が取得できません".to_string())?
        .to_string_lossy();
    let tmp_path = path.with_file_name(format!(".{file_name}.tmp"));

    std::fs::write(&tmp_path, content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, path).map_err(|e| {
        // rename 失敗時は中途半端な tmp を残さない
        let _ = std::fs::remove_file(&tmp_path);
        e.to_string()
    })?;
    Ok(())
}

fn entry_to_bibtex(entry: &EntryDetail, key: &str) -> String {
    // 内部の種別キー → BibTeX(biblatex) のエントリ型。biblatex 前提（@online 等を使う）。
    // 対応する標準型が無いもの（preprint/presentation/standard 等）は @misc に丸める。
    let bib_type = match entry.entry_type.as_str() {
        "article"             => "article",
        "magazineArticle"     => "article",
        "newspaperArticle"    => "article",
        "book"                => "book",
        "bookSection"         => "incollection",
        "encyclopediaArticle" => "incollection",
        "dictionaryEntry"     => "incollection",
        "inproceedings"       => "inproceedings",
        "thesis"              => "phdthesis",
        "report"              => "report",
        "manuscript"          => "unpublished",
        "patent"              => "patent",
        "dataset"             => "dataset",
        "computerProgram"     => "software",
        "webpage"             => "online",
        _                     => "misc",
    };

    let mut fields: Vec<String> = Vec::new();

    fields.push(format!("  title      = {{{}}}", entry.title));

    if !entry.authors.is_empty() {
        let author_str = entry.authors.iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(" and ");
        fields.push(format!("  author     = {{{}}}", author_str));
    }

    if let Some(year) = entry.year {
        fields.push(format!("  year       = {{{}}}", year));
    }
    if let Some(doi) = &entry.doi {
        fields.push(format!("  doi        = {{{}}}", doi));
    }
    if let Some(arxiv_id) = &entry.arxiv_id {
        fields.push(format!("  eprint     = {{{}}}", arxiv_id));
        fields.push(        "  eprinttype = {arXiv}".to_string());
    }
    if let Some(isbn) = &entry.isbn {
        fields.push(format!("  isbn       = {{{}}}", isbn));
    }
    if let Some(url) = &entry.url {
        fields.push(format!("  url        = {{{}}}", url));
    }
    if let Some(abs) = &entry.abstract_ {
        fields.push(format!("  abstract   = {{{}}}", abs));
    }
    if let Some(notes) = &entry.notes {
        fields.push(format!("  note       = {{{}}}", notes));
    }

    let mut extra: Vec<_> = entry.extra_fields.iter().collect();
    extra.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in extra {
        fields.push(format!("  {:<10} = {{{}}}", k, v));
    }

    format!("@{}{{{},\n{}\n}}", bib_type, key, fields.join(",\n"))
}

fn make_citation_key(entry: &EntryDetail) -> String {
    make_base_key(
        entry.authors.first().map(|a| a.name.as_str()),
        &entry.title,
        entry.year,
    )
}

/// 自動 cite key の base（接尾辞なし）を `第一著者の姓 + 年` で生成する。著者がいなければ
/// タイトル先頭語、年がなければ `nd`。英数字と `_` 以外を除去する。
/// `make_citation_key`（EntryDetail 経由）と `resolve_citation_key`（軽量行経由）で共有する。
fn make_base_key(first_author: Option<&str>, title: &str, year: Option<i64>) -> String {
    let author_part = match first_author {
        Some(name) if !name.trim().is_empty() => {
            name.split_whitespace().last().unwrap_or(name).to_lowercase()
        }
        _ => title.split_whitespace().next().unwrap_or("unknown").to_lowercase(),
    };

    let year_part = year.map(|y| y.to_string()).unwrap_or_else(|| "nd".to_string());

    format!("{}{}", author_part, year_part)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_author_last_first() {
        let authors = parse_authors("Einstein, Albert and Bohr, Niels");
        let names: Vec<&str> = authors.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Albert Einstein", "Niels Bohr"]);
        // "Last, First" は family/given を分割できる
        assert_eq!(authors[0].family_name.as_deref(), Some("Einstein"));
        assert_eq!(authors[0].given_name.as_deref(), Some("Albert"));
        assert!(!authors[0].is_organization);
    }

    #[test]
    fn parse_author_first_last() {
        let authors = parse_authors("Alan Turing and John McCarthy");
        let names: Vec<&str> = authors.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["Alan Turing", "John McCarthy"]);
        // "First Last" 形式は分割しない（誤判定回避のため name のみ）
        assert!(authors[0].family_name.is_none());
        assert!(authors[0].given_name.is_none());
    }

    // ── v0.3.0 §8.3 BibTeX 団体著者 ──────────────────────────────────────

    #[test]
    fn parse_author_literal_organization() {
        // `author = {{IEEE}}` の中身（nom-bibtex が outer {} を剥がした後）= "{IEEE}"
        let authors = parse_authors("{IEEE}");
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "IEEE");
        assert!(authors[0].is_organization);
        assert!(authors[0].family_name.is_none());
        assert!(authors[0].given_name.is_none());
    }

    #[test]
    fn parse_author_mixed_organization_and_person() {
        // 団体と個人が混在
        let authors = parse_authors("{IEEE} and Alan Turing and {ACM}");
        assert_eq!(authors.len(), 3);
        assert!(authors[0].is_organization && authors[0].name == "IEEE");
        assert!(!authors[1].is_organization && authors[1].name == "Alan Turing");
        assert!(authors[2].is_organization && authors[2].name == "ACM");
    }

    #[test]
    fn parse_author_literal_with_inner_and_is_one_token() {
        // 波括弧内の "and" は区切りとして扱わない（団体名を保護）
        let authors = parse_authors("{Smith and Jones Inc}");
        assert_eq!(authors.len(), 1);
        assert!(authors[0].is_organization);
        assert_eq!(authors[0].name, "Smith and Jones Inc");
    }

    #[test]
    fn parse_author_empty_literal_is_dropped() {
        let authors = parse_authors("{} and Real Person");
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "Real Person");
    }

    #[test]
    fn map_entry_type_variants() {
        assert_eq!(map_entry_type("article"),       "article");
        assert_eq!(map_entry_type("ARTICLE"),       "article");
        assert_eq!(map_entry_type("inproceedings"), "inproceedings");
        assert_eq!(map_entry_type("conference"),    "inproceedings");
        assert_eq!(map_entry_type("phdthesis"),     "thesis");
        assert_eq!(map_entry_type("online"),        "webpage");
        // v0.4.0: 新種別へのマッピング
        assert_eq!(map_entry_type("incollection"),  "bookSection");
        assert_eq!(map_entry_type("inbook"),        "bookSection");
        assert_eq!(map_entry_type("techreport"),    "report");
        assert_eq!(map_entry_type("report"),        "report");
        assert_eq!(map_entry_type("unpublished"),   "manuscript");
        assert_eq!(map_entry_type("patent"),        "patent");
        assert_eq!(map_entry_type("dataset"),       "dataset");
        assert_eq!(map_entry_type("software"),      "computerProgram");
        assert_eq!(map_entry_type("unknowntype"),   "misc");
    }

    #[test]
    fn suffix_letters_spreadsheet_style() {
        assert_eq!(suffix_letters(1), "a");
        assert_eq!(suffix_letters(2), "b");
        assert_eq!(suffix_letters(26), "z");
        assert_eq!(suffix_letters(27), "aa");
        assert_eq!(suffix_letters(28), "ab");
    }

    #[test]
    fn dedup_key_appends_suffixes() {
        let mut used = HashSet::new();
        assert_eq!(dedup_key("smith2020", &mut used), "smith2020");
        assert_eq!(dedup_key("smith2020", &mut used), "smith2020a");
        assert_eq!(dedup_key("smith2020", &mut used), "smith2020b");
        // 別 base は独立
        assert_eq!(dedup_key("jones2021", &mut used), "jones2021");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_single_article(pool: sqlx::SqlitePool) {
        let bib = r#"
@article{vaswani2017,
  title   = {Attention Is All You Need},
  author  = {Vaswani, Ashish and Shazeer, Noam and Parmar, Niki},
  year    = {2017},
  journal = {Advances in Neural Information Processing Systems},
  doi     = {10.48550/arXiv.1706.03762}
}
"#;
        let result = import_bibtex(&pool, bib).await.unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 0);

        let entries = crate::db::entries::get_entries(&pool, None, None, None).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Attention Is All You Need");
        assert_eq!(entries[0].authors.len(), 3);
        assert_eq!(entries[0].authors[0].name, "Ashish Vaswani");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_marks_literal_authors_as_organization(pool: sqlx::SqlitePool) {
        let bib = r#"
@misc{ieeestd2008,
  title  = {IEEE Standard for Floating-Point Arithmetic},
  author = {{IEEE}},
  year   = {2008}
}
"#;
        let result = import_bibtex(&pool, bib).await.unwrap();
        assert_eq!(result.imported, 1);

        let (count, is_org): (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), MAX(is_organization) FROM authors WHERE name = 'IEEE'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "団体著者 IEEE は 1 行のみ");
        assert_eq!(is_org, 1, "is_organization=1 が立つこと");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_keeps_existing_org_author(pool: sqlx::SqlitePool) {
        // 同じ団体著者を含む 2 つの entry を import しても authors 行は 1 つ。
        // is_organization は両方とも 1 として扱われる。
        let bib = r#"
@misc{a,
  title  = {Standard A},
  author = {{IEEE}},
  year   = {2008}
}
@misc{b,
  title  = {Standard B},
  author = {{IEEE}},
  year   = {2019}
}
"#;
        let result = import_bibtex(&pool, bib).await.unwrap();
        assert_eq!(result.imported, 2);

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE name = 'IEEE'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1, "name 一致で名寄せされ 1 行に集約されること");

        // 両 entry に同じ author_id がぶら下がっていること
        let author_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT ea.author_id
             FROM entry_authors ea
             JOIN authors a ON a.id = ea.author_id
             WHERE a.name = 'IEEE'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(author_ids.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_skips_entry_without_title(pool: sqlx::SqlitePool) {
        let bib = r#"
@article{notitle,
  author = {Someone},
  year   = {2020}
}
"#;
        let result = import_bibtex(&pool, bib).await.unwrap();
        assert_eq!(result.imported, 0);
        assert_eq!(result.skipped, 1);
    }

    // ── export tests ──────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn export_bibtex_single_article(pool: sqlx::SqlitePool) {
        let input = EntryInput {
            title: "Attention Is All You Need".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Ashish Vaswani".to_string(), "Noam Shazeer".to_string()],
            year: Some(2017),
            doi: Some("10.48550/arXiv.1706.03762".to_string()),
            extra_fields: [("journal".to_string(), "NeurIPS".to_string())].into(),
            ..Default::default()
        };
        let entry = create_entry(&pool, &input).await.unwrap();

        let bib = export_bibtex(&pool, Some(vec![entry.id])).await.unwrap();

        assert!(bib.contains("@article{"));
        assert!(bib.contains("Attention Is All You Need"));
        assert!(bib.contains("Ashish Vaswani and Noam Shazeer"));
        assert!(bib.contains("2017"));
        assert!(bib.contains("10.48550/arXiv.1706.03762"));
        assert!(bib.contains("NeurIPS"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_bibtex_type_mapping(pool: sqlx::SqlitePool) {
        let book = create_entry(&pool, &EntryInput {
            title: "CLRS".to_string(), entry_type: "book".to_string(), ..Default::default()
        }).await.unwrap();
        let conf = create_entry(&pool, &EntryInput {
            title: "ResNet".to_string(), entry_type: "inproceedings".to_string(), ..Default::default()
        }).await.unwrap();
        let thesis = create_entry(&pool, &EntryInput {
            title: "My Thesis".to_string(), entry_type: "thesis".to_string(), ..Default::default()
        }).await.unwrap();

        let bib_book   = export_bibtex(&pool, Some(vec![book.id])).await.unwrap();
        let bib_conf   = export_bibtex(&pool, Some(vec![conf.id])).await.unwrap();
        let bib_thesis = export_bibtex(&pool, Some(vec![thesis.id])).await.unwrap();

        assert!(bib_book.starts_with("@book{"));
        assert!(bib_conf.starts_with("@inproceedings{"));
        assert!(bib_thesis.starts_with("@phdthesis{"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_bibtex_all_when_ids_none(pool: sqlx::SqlitePool) {
        create_entry(&pool, &EntryInput { title: "Paper A".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        create_entry(&pool, &EntryInput { title: "Paper B".to_string(), entry_type: "book".to_string(), ..Default::default() }).await.unwrap();

        let bib = export_bibtex(&pool, None).await.unwrap();

        assert!(bib.contains("Paper A"));
        assert!(bib.contains("Paper B"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_bibtex_citation_key_format(pool: sqlx::SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Some Paper".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["John Smith".to_string()],
            year: Some(2023),
            ..Default::default()
        }).await.unwrap();

        let bib = export_bibtex(&pool, Some(vec![entry.id])).await.unwrap();
        assert!(bib.contains("@article{smith2023,"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_preserves_citation_key(pool: sqlx::SqlitePool) {
        let bib = r#"
@article{vaswani2017attention,
  title  = {Attention Is All You Need},
  author = {Vaswani, Ashish},
  year   = {2017}
}
"#;
        import_bibtex(&pool, bib).await.unwrap();

        let entries = crate::db::entries::get_entries(&pool, None, None, None).await.unwrap();
        let detail = crate::db::entries::get_entry(&pool, entries[0].id).await.unwrap();
        assert_eq!(detail.citation_key, Some("vaswani2017attention".to_string()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_dedups_colliding_citation_keys(pool: sqlx::SqlitePool) {
        // 同じ cite key を持つ 2 エントリ（.bib では重複キーは不正だが防御的に処理する）
        let bib = r#"
@article{smith2020,
  title  = {First},
  author = {Smith, A},
  year   = {2020}
}
@article{smith2020,
  title  = {Second},
  author = {Smith, B},
  year   = {2020}
}
"#;
        let result = import_bibtex(&pool, bib).await.unwrap();
        assert_eq!(result.imported, 2);

        let mut keys: Vec<String> = Vec::new();
        for e in crate::db::entries::get_entries(&pool, None, None, None).await.unwrap() {
            let d = crate::db::entries::get_entry(&pool, e.id).await.unwrap();
            keys.push(d.citation_key.unwrap());
        }
        keys.sort();
        assert_eq!(keys, vec!["smith2020".to_string(), "smith2020a".to_string()]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_uses_pinned_citation_key(pool: sqlx::SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Whatever".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["John Smith".to_string()],
            year: Some(2023),
            citation_key: Some("mypinnedkey".to_string()),
            ..Default::default()
        }).await.unwrap();

        let bib = export_bibtex(&pool, Some(vec![entry.id])).await.unwrap();
        // 自動生成の smith2023 ではなくピン留めキーが使われる
        assert!(bib.contains("@article{mypinnedkey,"));
        assert!(!bib.contains("smith2023"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_auto_dedups_same_author_year(pool: sqlx::SqlitePool) {
        let a = create_entry(&pool, &EntryInput {
            title: "Paper One".to_string(), entry_type: "article".to_string(),
            author_names: vec!["John Smith".to_string()], year: Some(2020),
            ..Default::default()
        }).await.unwrap();
        let b = create_entry(&pool, &EntryInput {
            title: "Paper Two".to_string(), entry_type: "article".to_string(),
            author_names: vec!["John Smith".to_string()], year: Some(2020),
            ..Default::default()
        }).await.unwrap();

        let bib = export_bibtex(&pool, Some(vec![a.id, b.id])).await.unwrap();
        assert!(bib.contains("@article{smith2020,"), "1件目は素の smith2020");
        assert!(bib.contains("@article{smith2020a,"), "2件目は接尾辞付き smith2020a");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn resolve_citation_key_matches_export(pool: sqlx::SqlitePool) {
        // 2 件とも自動キーが smith2020 になる → resolve は export と同じ a/b/c を返す
        let first = create_entry(&pool, &EntryInput {
            title: "First".to_string(), entry_type: "article".to_string(),
            author_names: vec!["John Smith".to_string()], year: Some(2020),
            ..Default::default()
        }).await.unwrap();
        let second = create_entry(&pool, &EntryInput {
            title: "Second".to_string(), entry_type: "article".to_string(),
            author_names: vec!["Jane Smith".to_string()], year: Some(2020),
            ..Default::default()
        }).await.unwrap();

        // export_bibtex(None) は created_at DESC（second が先）。resolve も同順序で割り当てる。
        let bib = export_bibtex(&pool, None).await.unwrap();
        let k_first = resolve_citation_key(&pool, first.id).await.unwrap();
        let k_second = resolve_citation_key(&pool, second.id).await.unwrap();

        assert_ne!(k_first, k_second);
        assert!(bib.contains(&format!("@article{{{},", k_first)));
        assert!(bib.contains(&format!("@article{{{},", k_second)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn resolve_citation_key_returns_pinned(pool: sqlx::SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "P".to_string(), entry_type: "article".to_string(),
            author_names: vec!["John Smith".to_string()], year: Some(2020),
            citation_key: Some("pinned2020".to_string()),
            ..Default::default()
        }).await.unwrap();
        assert_eq!(resolve_citation_key(&pool, entry.id).await.unwrap(), "pinned2020");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_auto_key_avoids_pinned_key(pool: sqlx::SqlitePool) {
        // ピン留め "smith2020" と、自動なら smith2020 になる別エントリが衝突しないこと
        let pinned = create_entry(&pool, &EntryInput {
            title: "Pinned".to_string(), entry_type: "article".to_string(),
            author_names: vec!["John Smith".to_string()], year: Some(2020),
            citation_key: Some("smith2020".to_string()),
            ..Default::default()
        }).await.unwrap();
        let auto = create_entry(&pool, &EntryInput {
            title: "Auto".to_string(), entry_type: "article".to_string(),
            author_names: vec!["Jane Smith".to_string()], year: Some(2020),
            ..Default::default()
        }).await.unwrap();

        let bib = export_bibtex(&pool, Some(vec![pinned.id, auto.id])).await.unwrap();
        assert!(bib.contains("@article{smith2020,"));
        assert!(bib.contains("@article{smith2020a,"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_bibtex_excludes_trashed(pool: sqlx::SqlitePool) {
        let kept = create_entry(&pool, &EntryInput { title: "Visible".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        let dropped = create_entry(&pool, &EntryInput { title: "Hidden".to_string(), entry_type: "article".to_string(), ..Default::default() }).await.unwrap();
        crate::db::entries::trash_entry(&pool, dropped.id).await.unwrap();

        let bib = export_bibtex(&pool, None).await.unwrap();
        assert!(bib.contains("Visible"));
        assert!(!bib.contains("Hidden"));
        let _ = kept;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sync_bibtex_writes_to_file(pool: sqlx::SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "Synced Paper".to_string(),
            entry_type: "article".to_string(),
            author_names: vec!["Ada Lovelace".to_string()],
            year: Some(1843),
            ..Default::default()
        }).await.unwrap();

        let dir = std::env::temp_dir().join(format!(
            "lumencite_sync_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("refs.bib");

        sync_bibtex(&pool, &path).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Synced Paper"));
        // tmp file は残らない
        let leftover = dir.join(".refs.bib.tmp");
        assert!(!leftover.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sync_bibtex_overwrites_existing_file(pool: sqlx::SqlitePool) {
        create_entry(&pool, &EntryInput {
            title: "New Content".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();

        let dir = std::env::temp_dir().join(format!(
            "lumencite_sync_overwrite_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("refs.bib");
        std::fs::write(&path, "OLD CONTENT").unwrap();

        sync_bibtex(&pool, &path).await.unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.contains("OLD CONTENT"));
        assert!(content.contains("New Content"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_multiple_entries(pool: sqlx::SqlitePool) {
        let bib = r#"
@article{a1, title = {Paper One}, author = {Smith, Alice}, year = {2021}}
@book{b1, title = {Book One}, author = {Jones, Bob}, year = {2022}}
@inproceedings{c1, title = {Conf Paper}, author = {Lee, Carol}, year = {2023}}
"#;
        let result = import_bibtex(&pool, bib).await.unwrap();
        assert_eq!(result.imported, 3);
        assert_eq!(result.skipped, 0);
    }
}
