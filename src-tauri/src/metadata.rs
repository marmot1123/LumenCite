use std::collections::HashMap;

use crate::models::{AuthorInput, EntryInput};

// ── DOI（CrossRef API）────────────────────────────────────────────────────────

pub async fn fetch_by_doi(doi: &str) -> Result<EntryInput, String> {
    let doi = doi
        .trim()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi:");

    let url = format!("https://api.crossref.org/works/{}", doi);
    let client = reqwest::Client::builder()
        .user_agent("LumenCite/0.1 (mailto:support@lumencite.app)")
        .build()
        .map_err(|e| e.to_string())?;

    let resp: serde_json::Value = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("ネットワークエラー: {}", e))?
        .error_for_status()
        .map_err(|_| "DOI が見つかりませんでした".to_string())?
        .json()
        .await
        .map_err(|e| format!("レスポンス解析エラー: {}", e))?;

    Ok(crossref_to_input(&resp["message"], doi))
}

fn crossref_to_input(msg: &serde_json::Value, doi: &str) -> EntryInput {
    let title = msg["title"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("(タイトルなし)")
        .to_string();

    // v0.3.0: CrossRef の著者配列を AuthorInput に変換し、ORCID / given / family を拾う。
    // `name` フィールド単独（団体名や分離不能な表記）は is_organization=false で literal 扱い。
    let crossref_authors: Vec<AuthorInput> = msg["author"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(crossref_author_to_input)
        .collect();
    // 互換: フロント側がまだ author_names だけ見るルートにも値が残るよう、表示名を併設する。
    let author_names: Vec<String> = crossref_authors.iter().map(|a| a.name.clone()).collect();

    let year = msg["published"]["date-parts"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|d| d.as_array())
        .and_then(|d| d.first())
        .and_then(|y| y.as_i64())
        .or_else(|| {
            msg["published-print"]["date-parts"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|d| d.as_array())
                .and_then(|d| d.first())
                .and_then(|y| y.as_i64())
        });

    let abstract_ = msg["abstract"].as_str().map(strip_html_tags);

    let entry_type = match msg["type"].as_str().unwrap_or("") {
        "journal-article" | "article" => "article",
        "book" | "monograph" | "reference-book" | "edited-book" => "book",
        "proceedings-article" => "inproceedings",
        "dissertation" => "thesis",
        _ => "misc",
    }
    .to_string();

    let url_val = msg["URL"].as_str().map(|s| s.to_string());

    let mut extra_fields: HashMap<String, String> = HashMap::new();

    // container-title は article なら journal、inproceedings なら booktitle
    let container = msg["container-title"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(c) = container {
        let key = if entry_type == "inproceedings" { "booktitle" } else { "journal" };
        extra_fields.insert(key.to_string(), c);
    }

    if let Some(v) = msg["volume"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        extra_fields.insert("volume".to_string(), v.to_string());
    }
    if let Some(v) = msg["issue"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        extra_fields.insert("issue".to_string(), v.to_string());
    }
    if let Some(v) = msg["page"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        extra_fields.insert("pages".to_string(), v.to_string());
    }
    if let Some(v) = msg["publisher"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        extra_fields.insert("publisher".to_string(), v.to_string());
    }
    if let Some(v) = msg["publisher-location"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        extra_fields.insert("address".to_string(), v.to_string());
    }

    EntryInput {
        title,
        year,
        entry_type,
        doi: Some(doi.to_string()),
        url: url_val,
        author_names,
        authors: if crossref_authors.is_empty() { None } else { Some(crossref_authors) },
        abstract_,
        extra_fields,
        ..Default::default()
    }
}

/// CrossRef の author 1 件を `AuthorInput` に変換する。
///
/// CrossRef の author 要素は以下の形を想定:
///   { "given": "Albert", "family": "Einstein",
///     "ORCID": "https://orcid.org/0000-0002-1825-0097",
///     "name": "..." (個人名が given/family に分割できないとき、または団体名) }
///
/// 仕様メモ:
/// - ORCID は URL 形式 (`https://orcid.org/<id>`) で返ることが多い。`<id>` だけ取り出して
///   `AuthorInput.orcid` に詰める（`db/authors.rs::get_or_create_author` 側が両形式を吸収する）。
/// - given/family が両方欠けて `name` のみのケースは個人/団体の判別が付かないため
///   v0.3.0 では **is_organization=false（個人扱い）** で literal name を返す。BibTeX の
///   `{IEEE}` 検出のような明示マーカーが無いため、団体判定は将来の TODO。
fn crossref_author_to_input(a: &serde_json::Value) -> Option<AuthorInput> {
    let given = a["given"].as_str().map(str::trim).unwrap_or("");
    let family = a["family"].as_str().map(str::trim).unwrap_or("");
    let name_only = a["name"].as_str().map(str::trim).unwrap_or("");

    let (name, given_opt, family_opt) = if !family.is_empty() && !given.is_empty() {
        (format!("{} {}", given, family), Some(given.to_string()), Some(family.to_string()))
    } else if !family.is_empty() {
        (family.to_string(), None, Some(family.to_string()))
    } else if !name_only.is_empty() {
        (name_only.to_string(), None, None)
    } else {
        return None;
    };

    let orcid = a["ORCID"]
        .as_str()
        .map(normalize_orcid)
        .filter(|s| !s.is_empty());

    Some(AuthorInput {
        name,
        given_name: given_opt,
        family_name: family_opt,
        orcid,
        ..Default::default()
    })
}

/// ORCID 値を「ハイフン込みの素の ID」に正規化する。
/// `https://orcid.org/0000-...` / `http://orcid.org/0000-...` / 末尾スラッシュ /
/// 余分な空白を吸収。形式チェックはここでは行わず、呼び出し側で必要なら追加検証する。
fn normalize_orcid(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

// ── arXiv API ─────────────────────────────────────────────────────────────────

pub async fn fetch_by_arxiv(arxiv_id: &str) -> Result<EntryInput, String> {
    let id = normalize_arxiv_id(arxiv_id);
    let url = format!("https://export.arxiv.org/api/query?id_list={}", id);

    let text = reqwest::get(&url)
        .await
        .map_err(|e| format!("ネットワークエラー: {}", e))?
        .text()
        .await
        .map_err(|e| format!("レスポンス解析エラー: {}", e))?;

    // Find the <entry> element (skip feed-level tags)
    let entry = {
        let start = text.find("<entry>").ok_or("arXiv: エントリが見つかりません（IDを確認してください）")?;
        let end = text[start..].find("</entry>").ok_or("arXiv: レスポンス解析エラー")?;
        &text[start..start + end + "</entry>".len()]
    };

    let title = extract_first_xml_text(entry, "title")
        .map(|t| t.trim().replace('\n', " "))
        .unwrap_or_else(|| "(タイトルなし)".to_string());

    let author_names = extract_all_xml_text(entry, "name");

    let year = extract_first_xml_text(entry, "published")
        .and_then(|s| s.get(..4).map(|y| y.to_string()))
        .and_then(|y| y.parse::<i64>().ok());

    let abstract_ = extract_first_xml_text(entry, "summary")
        .map(|s| s.trim().replace('\n', " "));

    Ok(EntryInput {
        title,
        year,
        entry_type: "article".to_string(),
        arxiv_id: Some(id),
        author_names,
        abstract_,
        ..Default::default()
    })
}

// ── ISBN（OpenLibrary API）────────────────────────────────────────────────────

pub async fn fetch_by_isbn(isbn: &str) -> Result<EntryInput, String> {
    let isbn = isbn.trim().replace(['-', ' '], "");
    let url = format!(
        "https://openlibrary.org/api/books?bibkeys=ISBN:{}&format=json&jscmd=data",
        isbn
    );

    let resp: serde_json::Value = reqwest::get(&url)
        .await
        .map_err(|e| format!("ネットワークエラー: {}", e))?
        .json()
        .await
        .map_err(|e| format!("レスポンス解析エラー: {}", e))?;

    let key = format!("ISBN:{}", isbn);
    let book = resp
        .get(&key)
        .ok_or_else(|| "ISBN が見つかりませんでした".to_string())?;

    Ok(openlibrary_to_input(book, &isbn))
}

fn openlibrary_to_input(book: &serde_json::Value, isbn: &str) -> EntryInput {
    let title = book["title"]
        .as_str()
        .unwrap_or("(タイトルなし)")
        .to_string();

    let author_names: Vec<String> = book["authors"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|a| a["name"].as_str().map(|s| s.to_string()))
        .collect();

    let year = book["publish_date"].as_str().and_then(|s| {
        s.split_whitespace()
            .find_map(|w| w.parse::<i64>().ok().filter(|&y| y > 1000 && y < 2200))
    });

    let mut extra_fields: HashMap<String, String> = HashMap::new();

    let publisher = book["publishers"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|p| p["name"].as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(p) = publisher {
        extra_fields.insert("publisher".to_string(), p);
    }

    let pages = book["number_of_pages"]
        .as_i64()
        .map(|n| n.to_string())
        .or_else(|| {
            book["pagination"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        });
    if let Some(p) = pages {
        extra_fields.insert("pages".to_string(), p);
    }

    let address = book["publish_places"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|p| p["name"].as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(a) = address {
        extra_fields.insert("address".to_string(), a);
    }

    EntryInput {
        title,
        year,
        entry_type: "book".to_string(),
        isbn: Some(isbn.to_string()),
        author_names,
        extra_fields,
        ..Default::default()
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn strip_html_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

fn normalize_arxiv_id(id: &str) -> String {
    let id = id.trim();
    if let Some(pos) = id.rfind('/') {
        let rest = &id[pos + 1..];
        // strip version suffix like v5
        return rest
            .find('v')
            .map_or(rest, |v| &rest[..v])
            .to_string();
    }
    if let Some(s) = id.strip_prefix("arXiv:").or_else(|| id.strip_prefix("arxiv:")) {
        return s.to_string();
    }
    id.to_string()
}

fn extract_first_xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].to_string())
}

fn extract_all_xml_text(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(s) = xml[pos..].find(&open) {
        let cs = pos + s + open.len();
        if let Some(e) = xml[cs..].find(&close) {
            results.push(xml[cs..cs + e].trim().to_string());
            pos = cs + e + close.len();
        } else {
            break;
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── crossref_to_input ────────────────────────────────────────────────────

    #[test]
    fn crossref_extracts_journal_and_pages_for_article() {
        let msg = json!({
            "type": "journal-article",
            "title": ["Example Title"],
            "container-title": ["Nature"],
            "volume": "612",
            "issue": "7940",
            "page": "150-160",
            "publisher": "Springer Nature",
        });

        let input = crossref_to_input(&msg, "10.1234/test");

        assert_eq!(input.entry_type, "article");
        assert_eq!(input.extra_fields.get("journal").map(String::as_str), Some("Nature"));
        assert_eq!(input.extra_fields.get("volume").map(String::as_str), Some("612"));
        assert_eq!(input.extra_fields.get("issue").map(String::as_str), Some("7940"));
        assert_eq!(input.extra_fields.get("pages").map(String::as_str), Some("150-160"));
        assert_eq!(input.extra_fields.get("publisher").map(String::as_str), Some("Springer Nature"));
    }

    #[test]
    fn crossref_uses_booktitle_for_inproceedings() {
        let msg = json!({
            "type": "proceedings-article",
            "title": ["Conference Paper"],
            "container-title": ["Proceedings of CVPR 2024"],
            "page": "1234-1245",
        });

        let input = crossref_to_input(&msg, "10.1234/conf");

        assert_eq!(input.entry_type, "inproceedings");
        assert_eq!(input.extra_fields.get("booktitle").map(String::as_str), Some("Proceedings of CVPR 2024"));
        assert!(!input.extra_fields.contains_key("journal"));
        assert_eq!(input.extra_fields.get("pages").map(String::as_str), Some("1234-1245"));
    }

    #[test]
    fn crossref_omits_missing_fields() {
        let msg = json!({
            "type": "journal-article",
            "title": ["Bare Bones"],
        });

        let input = crossref_to_input(&msg, "10.0/x");

        assert!(input.extra_fields.is_empty());
        assert_eq!(input.title, "Bare Bones");
    }

    // ── v0.3.0 § CrossRef 著者 (ORCID / given / family) ──────────────────────

    #[test]
    fn crossref_extracts_orcid_and_splits_name() {
        let msg = json!({
            "type": "journal-article",
            "title": ["With ORCID"],
            "author": [
                {
                    "given": "Albert",
                    "family": "Einstein",
                    "ORCID": "https://orcid.org/0000-0002-1825-0097",
                },
                { "given": "Niels", "family": "Bohr" }
            ],
        });

        let input = crossref_to_input(&msg, "10.0/orcid");
        // 互換ルート: author_names も併設
        assert_eq!(input.author_names, vec!["Albert Einstein", "Niels Bohr"]);

        let authors = input.authors.as_ref().expect("authors を詰めるべき");
        assert_eq!(authors.len(), 2);

        // 1 人目: ORCID + given/family すべて埋まる
        assert_eq!(authors[0].name, "Albert Einstein");
        assert_eq!(authors[0].given_name.as_deref(), Some("Albert"));
        assert_eq!(authors[0].family_name.as_deref(), Some("Einstein"));
        assert_eq!(
            authors[0].orcid.as_deref(),
            Some("0000-0002-1825-0097"),
            "ORCID URL の末尾 ID 部分だけが入ること"
        );

        // 2 人目: ORCID 無し
        assert_eq!(authors[1].name, "Niels Bohr");
        assert!(authors[1].orcid.is_none());
    }

    #[test]
    fn crossref_orcid_bare_id_is_kept_as_is() {
        // CrossRef は通常 URL 形式だが、裸 ID で返ってきても壊れない
        let msg = json!({
            "type": "journal-article",
            "title": ["t"],
            "author": [{ "given": "G", "family": "F", "ORCID": "0000-0001-2345-6789" }],
        });
        let input = crossref_to_input(&msg, "10.0/x");
        let authors = input.authors.unwrap();
        assert_eq!(authors[0].orcid.as_deref(), Some("0000-0001-2345-6789"));
    }

    #[test]
    fn crossref_name_only_author_keeps_literal_name() {
        // given/family が無く name のみのケース（団体名 or 分離不能な表記）
        let msg = json!({
            "type": "journal-article",
            "title": ["t"],
            "author": [{ "name": "World Health Organization" }],
        });
        let input = crossref_to_input(&msg, "10.0/x");
        let authors = input.authors.unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "World Health Organization");
        assert!(authors[0].given_name.is_none());
        assert!(authors[0].family_name.is_none());
        // v0.3.0 では CrossRef レスポンスからは団体判定できないので false のまま
        assert!(!authors[0].is_organization);
    }

    #[test]
    fn crossref_no_authors_yields_none_in_authors_field() {
        let msg = json!({ "type": "journal-article", "title": ["t"] });
        let input = crossref_to_input(&msg, "10.0/x");
        assert!(input.authors.is_none(), "著者ゼロなら None（互換: author_names は空 Vec）");
        assert!(input.author_names.is_empty());
    }

    #[test]
    fn crossref_drops_authors_with_no_usable_field() {
        // given も family も name も無い空の author 要素は捨てる
        let msg = json!({
            "type": "journal-article",
            "title": ["t"],
            "author": [
                {},
                { "given": "A", "family": "B" }
            ],
        });
        let input = crossref_to_input(&msg, "10.0/x");
        let authors = input.authors.unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "A B");
    }

    #[test]
    fn normalize_orcid_strips_url_prefix_and_trailing_slash() {
        assert_eq!(normalize_orcid("https://orcid.org/0000-0002-1825-0097"), "0000-0002-1825-0097");
        assert_eq!(normalize_orcid("http://orcid.org/0000-0002-1825-0097"), "0000-0002-1825-0097");
        assert_eq!(normalize_orcid("https://orcid.org/0000-0002-1825-0097/"), "0000-0002-1825-0097");
        assert_eq!(normalize_orcid("  0000-0002-1825-0097  "), "0000-0002-1825-0097");
    }

    #[test]
    fn crossref_skips_empty_strings() {
        let msg = json!({
            "type": "journal-article",
            "title": ["X"],
            "volume": "",
            "page": "   ",
            "container-title": [""],
        });

        let input = crossref_to_input(&msg, "10.0/x");

        assert!(!input.extra_fields.contains_key("volume"));
        assert!(!input.extra_fields.contains_key("pages"));
        assert!(!input.extra_fields.contains_key("journal"));
    }

    // ── openlibrary_to_input ─────────────────────────────────────────────────

    #[test]
    fn openlibrary_extracts_publisher_and_pages() {
        let book = json!({
            "title": "CLRS",
            "authors": [{ "name": "Thomas H. Cormen" }],
            "publish_date": "2009",
            "publishers": [{ "name": "MIT Press" }],
            "number_of_pages": 1312,
            "publish_places": [{ "name": "Cambridge, MA" }],
        });

        let input = openlibrary_to_input(&book, "9780262033848");

        assert_eq!(input.title, "CLRS");
        assert_eq!(input.entry_type, "book");
        assert_eq!(input.extra_fields.get("publisher").map(String::as_str), Some("MIT Press"));
        assert_eq!(input.extra_fields.get("pages").map(String::as_str), Some("1312"));
        assert_eq!(input.extra_fields.get("address").map(String::as_str), Some("Cambridge, MA"));
    }

    #[test]
    fn openlibrary_falls_back_to_pagination_string() {
        let book = json!({
            "title": "Foo",
            "pagination": "xii, 480 p.",
        });

        let input = openlibrary_to_input(&book, "0000000000");

        assert_eq!(input.extra_fields.get("pages").map(String::as_str), Some("xii, 480 p."));
    }

    #[test]
    fn openlibrary_omits_missing_extras() {
        let book = json!({ "title": "Minimal" });
        let input = openlibrary_to_input(&book, "0000000000");
        assert!(input.extra_fields.is_empty());
    }
}
