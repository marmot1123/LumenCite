use std::collections::HashMap;

use nom_bibtex::Bibtex;
use sqlx::{Row, SqlitePool};

use crate::db::entries::{create_entry, get_entry};
use crate::models::{EntryDetail, EntryInput, ImportResult};

pub async fn import_bibtex(pool: &SqlitePool, content: &str) -> Result<ImportResult, String> {
    let bib = Bibtex::parse(content).map_err(|e| format!("BibTeX 解析エラー: {:?}", e))?;

    let mut imported = 0i64;
    let mut skipped = 0i64;
    let mut errors: Vec<String> = Vec::new();

    for entry in bib.bibliographies() {
        match bibliography_to_input(entry) {
            Some(input) => match create_entry(pool, &input).await {
                Ok(_) => imported += 1,
                Err(e) => {
                    skipped += 1;
                    errors.push(format!("{}: {}", entry.citation_key(), e));
                }
            },
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
    let author_names = tags.get("author").map(|a| parse_authors(a)).unwrap_or_default();

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
        author_names,
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
        "inproceedings" | "conference" | "inbook" | "incollection"
        | "inreference" | "suppbook" | "suppcollection" | "proceedings"
        | "mvproceedings" => "inproceedings",
        "phdthesis" | "mastersthesis" | "thesis" => "thesis",
        "online" | "electronic" | "www" | "webpage" => "webpage",
        _ => "misc",
    }
    .to_string()
}

fn parse_authors(author_str: &str) -> Vec<String> {
    author_str
        .split(" and ")
        .map(|name| {
            let name = name.trim();
            // "Last, First Middle" → "First Middle Last"
            if let Some(comma) = name.find(',') {
                let last = name[..comma].trim();
                let first = name[comma + 1..].trim();
                if first.is_empty() {
                    last.to_string()
                } else {
                    format!("{} {}", first, last)
                }
            } else {
                name.to_string()
            }
        })
        .filter(|n| !n.is_empty())
        .collect()
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

    let mut parts = Vec::with_capacity(ids.len());
    for id in ids {
        let entry = get_entry(pool, id).await.map_err(|e| e.to_string())?;
        parts.push(entry_to_bibtex(&entry));
    }

    Ok(parts.join("\n"))
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

fn entry_to_bibtex(entry: &EntryDetail) -> String {
    let bib_type = match entry.entry_type.as_str() {
        "article"        => "article",
        "book"           => "book",
        "inproceedings"  => "inproceedings",
        "thesis"         => "phdthesis",
        "webpage"        => "online",
        _                => "misc",
    };

    let key = make_citation_key(entry);
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
    let author_part = entry.authors.first()
        .map(|a| a.name.split_whitespace().last().unwrap_or(&a.name).to_lowercase())
        .unwrap_or_else(|| {
            entry.title.split_whitespace().next().unwrap_or("unknown").to_lowercase()
        });

    let year_part = entry.year.map(|y| y.to_string()).unwrap_or_else(|| "nd".to_string());

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
        assert_eq!(authors, vec!["Albert Einstein", "Niels Bohr"]);
    }

    #[test]
    fn parse_author_first_last() {
        let authors = parse_authors("Alan Turing and John McCarthy");
        assert_eq!(authors, vec!["Alan Turing", "John McCarthy"]);
    }

    #[test]
    fn map_entry_type_variants() {
        assert_eq!(map_entry_type("article"),       "article");
        assert_eq!(map_entry_type("ARTICLE"),       "article");
        assert_eq!(map_entry_type("inproceedings"), "inproceedings");
        assert_eq!(map_entry_type("conference"),    "inproceedings");
        assert_eq!(map_entry_type("phdthesis"),     "thesis");
        assert_eq!(map_entry_type("online"),        "webpage");
        assert_eq!(map_entry_type("techreport"),    "misc");
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
