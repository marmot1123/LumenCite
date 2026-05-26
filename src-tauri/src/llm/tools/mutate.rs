//! write 系ツール: add_tag / update_notes / add_to_collection /
//! create_entry / update_entry / delete_entry。
//!
//! 契約は `super`（tools/mod.rs）参照。`specs()` で LLM 向け定義を、
//! `try_execute()` で実行を提供する。承認可否の判定は `super::approval` を参照。

use super::{ToolContext, ToolError};
use crate::llm::{ToolCallSpec, ToolSpec};
use crate::models::EntryInput;
use serde_json::json;

/// write 系ツールの定義一覧。
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "add_tag".to_string(),
            description: "Add a tag to an entry. The tag is created if it does not already exist. \
                          This is idempotent — calling it again with the same tag has no effect."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "The ID of the entry to tag."
                    },
                    "tag_name": {
                        "type": "string",
                        "description": "The name of the tag to attach (e.g. \"machine-learning\")."
                    }
                },
                "required": ["entry_id", "tag_name"]
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "update_notes".to_string(),
            description: "Set the personal notes field of an entry. \
                          The existing notes are replaced entirely with the provided text."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "The ID of the entry whose notes to update."
                    },
                    "notes": {
                        "type": "string",
                        "description": "The new notes text. Pass an empty string to clear notes."
                    }
                },
                "required": ["entry_id", "notes"]
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "add_to_collection".to_string(),
            description: "Add an entry to a collection. \
                          This is idempotent — if the entry is already in the collection, nothing changes."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "The ID of the entry to add."
                    },
                    "collection_id": {
                        "type": "integer",
                        "description": "The ID of the target collection."
                    }
                },
                "required": ["entry_id", "collection_id"]
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "create_entry".to_string(),
            description: "Create a new bibliography entry. \
                          Returns the ID of the newly created entry."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "The title of the work."
                    },
                    "entry_type": {
                        "type": "string",
                        "description": "BibTeX entry type, e.g. \"article\", \"book\", \"inproceedings\", \"misc\".",
                        "default": "misc"
                    },
                    "year": {
                        "type": "integer",
                        "description": "Publication year."
                    },
                    "abstract": {
                        "type": "string",
                        "description": "Abstract or summary of the work."
                    },
                    "doi": {
                        "type": "string",
                        "description": "Digital Object Identifier."
                    },
                    "isbn": {
                        "type": "string",
                        "description": "ISBN for books."
                    },
                    "arxiv_id": {
                        "type": "string",
                        "description": "arXiv paper ID (e.g. \"2303.12345\")."
                    },
                    "url": {
                        "type": "string",
                        "description": "URL to the work."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Personal notes."
                    },
                    "author_names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of author names in display order."
                    }
                },
                "required": ["title"]
            }),
            needs_approval: true,
        },
        ToolSpec {
            name: "update_entry".to_string(),
            description: "Update fields of an existing entry. \
                          Only the provided fields are changed; all other fields are preserved."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "The ID of the entry to update."
                    },
                    "title": {
                        "type": "string",
                        "description": "New title."
                    },
                    "entry_type": {
                        "type": "string",
                        "description": "New entry type (e.g. \"article\")."
                    },
                    "year": {
                        "type": "integer",
                        "description": "New publication year."
                    },
                    "abstract": {
                        "type": "string",
                        "description": "New abstract."
                    },
                    "doi": {
                        "type": "string",
                        "description": "New DOI."
                    },
                    "isbn": {
                        "type": "string",
                        "description": "New ISBN."
                    },
                    "arxiv_id": {
                        "type": "string",
                        "description": "New arXiv ID."
                    },
                    "url": {
                        "type": "string",
                        "description": "New URL."
                    },
                    "notes": {
                        "type": "string",
                        "description": "New notes."
                    },
                    "author_names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Replacement author list (replaces existing authors entirely)."
                    }
                },
                "required": ["entry_id"]
            }),
            needs_approval: true,
        },
        ToolSpec {
            name: "delete_entry".to_string(),
            description: "Permanently delete an entry and all its associated data \
                          (tags, attachments, FTS index). This action cannot be undone."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "The ID of the entry to delete."
                    }
                },
                "required": ["entry_id"]
            }),
            needs_approval: true,
        },
    ]
}

/// このモジュールが `call.tool_name` を扱うなら `Some(結果)`、扱わなければ `None`。
pub async fn try_execute(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Option<Result<String, ToolError>> {
    match call.tool_name.as_str() {
        "add_tag" => Some(execute_add_tag(ctx, call).await),
        "update_notes" => Some(execute_update_notes(ctx, call).await),
        "add_to_collection" => Some(execute_add_to_collection(ctx, call).await),
        "create_entry" => Some(execute_create_entry(ctx, call).await),
        "update_entry" => Some(execute_update_entry(ctx, call).await),
        "delete_entry" => Some(execute_delete_entry(ctx, call).await),
        _ => None,
    }
}

// ── Individual tool implementations ──────────────────────────────────────────

async fn execute_add_tag(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let entry_id = parse_i64(&call.arguments, "entry_id")?;
    let tag_name = parse_str(&call.arguments, "tag_name")?;

    // Get-or-create the tag by name
    let all_tags = crate::db::tags::get_tags(ctx.pool).await?;
    let tag = if let Some(existing) = all_tags.into_iter().find(|t| t.name == tag_name) {
        existing
    } else {
        crate::db::tags::create_tag(ctx.pool, &tag_name).await?
    };

    crate::db::tags::add_tag_to_entry(ctx.pool, entry_id, tag.id).await?;

    Ok(format!("Tag \"{}\" (id={}) added to entry {}.", tag.name, tag.id, entry_id))
}

async fn execute_update_notes(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let entry_id = parse_i64(&call.arguments, "entry_id")?;
    let notes = parse_str(&call.arguments, "notes")?;

    let rows = sqlx::query(
        "UPDATE entries SET notes = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&notes)
    .bind(entry_id)
    .execute(ctx.pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(ToolError::Db(sqlx::Error::RowNotFound));
    }

    Ok(format!("Notes updated for entry {}.", entry_id))
}

async fn execute_add_to_collection(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let entry_id = parse_i64(&call.arguments, "entry_id")?;
    let collection_id = parse_i64(&call.arguments, "collection_id")?;

    crate::db::collections::add_entry_to_collection(ctx.pool, entry_id, collection_id).await?;

    Ok(format!("Entry {} added to collection {}.", entry_id, collection_id))
}

async fn execute_create_entry(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let args = &call.arguments;

    let title = parse_str(args, "title")?;
    let entry_type = args
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("misc")
        .to_string();

    let year = args.get("year").and_then(|v| v.as_i64());
    let abstract_ = args.get("abstract").and_then(|v| v.as_str()).map(str::to_string);
    let doi = args.get("doi").and_then(|v| v.as_str()).map(str::to_string);
    let isbn = args.get("isbn").and_then(|v| v.as_str()).map(str::to_string);
    let arxiv_id = args.get("arxiv_id").and_then(|v| v.as_str()).map(str::to_string);
    let url = args.get("url").and_then(|v| v.as_str()).map(str::to_string);
    let notes = args.get("notes").and_then(|v| v.as_str()).map(str::to_string);

    let author_names: Vec<String> = args
        .get("author_names")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let input = EntryInput {
        title,
        entry_type,
        year,
        abstract_,
        doi,
        isbn,
        arxiv_id,
        url,
        notes,
        author_names,
        ..Default::default()
    };

    let entry = crate::db::entries::create_entry(ctx.pool, &input).await?;

    Ok(format!("Entry created with id={}.", entry.id))
}

async fn execute_update_entry(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let args = &call.arguments;
    let entry_id = parse_i64(args, "entry_id")?;

    // Fetch the current entry to preserve fields not provided
    let current = crate::db::entries::get_entry(ctx.pool, entry_id).await?;

    // Build an EntryInput from the current entry, then apply overrides
    let mut input = EntryInput {
        title: current.title.clone(),
        entry_type: current.entry_type.clone(),
        year: current.year,
        doi: current.doi.clone(),
        isbn: current.isbn.clone(),
        arxiv_id: current.arxiv_id.clone(),
        url: current.url.clone(),
        abstract_: current.abstract_.clone(),
        notes: current.notes.clone(),
        author_names: current.authors.iter().map(|a| a.name.clone()).collect(),
        extra_fields: current.extra_fields.clone(),
        ..Default::default()
    };

    // Apply provided overrides
    if let Some(v) = args.get("title").and_then(|v| v.as_str()) {
        input.title = v.to_string();
    }
    if let Some(v) = args.get("entry_type").and_then(|v| v.as_str()) {
        input.entry_type = v.to_string();
    }
    if let Some(v) = args.get("year").and_then(|v| v.as_i64()) {
        input.year = Some(v);
    }
    if let Some(v) = args.get("abstract").and_then(|v| v.as_str()) {
        input.abstract_ = Some(v.to_string());
    }
    if let Some(v) = args.get("doi").and_then(|v| v.as_str()) {
        input.doi = Some(v.to_string());
    }
    if let Some(v) = args.get("isbn").and_then(|v| v.as_str()) {
        input.isbn = Some(v.to_string());
    }
    if let Some(v) = args.get("arxiv_id").and_then(|v| v.as_str()) {
        input.arxiv_id = Some(v.to_string());
    }
    if let Some(v) = args.get("url").and_then(|v| v.as_str()) {
        input.url = Some(v.to_string());
    }
    if let Some(v) = args.get("notes").and_then(|v| v.as_str()) {
        input.notes = Some(v.to_string());
    }
    if let Some(arr) = args.get("author_names").and_then(|v| v.as_array()) {
        input.author_names = arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
    }

    crate::db::entries::update_entry(ctx.pool, entry_id, &input).await?;

    Ok(format!("Entry {} updated.", entry_id))
}

async fn execute_delete_entry(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let entry_id = parse_i64(&call.arguments, "entry_id")?;

    crate::db::entries::delete_entry(ctx.pool, entry_id).await?;

    Ok(format!("Entry {} permanently deleted.", entry_id))
}

// ── Argument parsing helpers ──────────────────────────────────────────────────

fn parse_i64(args: &serde_json::Value, key: &str) -> Result<i64, ToolError> {
    args.get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| ToolError::InvalidArguments(format!("missing or invalid integer field \"{}\"", key)))
}

fn parse_str(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArguments(format!("missing or invalid string field \"{}\"", key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;
    use sqlx::SqlitePool;

    async fn make_entry(pool: &SqlitePool, title: &str) -> i64 {
        create_entry(
            pool,
            &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    }

    fn make_ctx(pool: &SqlitePool) -> ToolContext<'_> {
        ToolContext {
            pool,
            session_id: 1,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
        }
    }

    fn call(tool_name: &str, args: serde_json::Value) -> ToolCallSpec {
        ToolCallSpec {
            call_id: "test-call-id".to_string(),
            tool_name: tool_name.to_string(),
            arguments: args,
        }
    }

    // ── specs ──────────────────────────────────────────────────────────────

    #[test]
    fn specs_returns_all_six_tools() {
        let s = specs();
        let names: Vec<&str> = s.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"add_tag"));
        assert!(names.contains(&"update_notes"));
        assert!(names.contains(&"add_to_collection"));
        assert!(names.contains(&"create_entry"));
        assert!(names.contains(&"update_entry"));
        assert!(names.contains(&"delete_entry"));
        assert_eq!(s.len(), 6);
    }

    #[test]
    fn specs_needs_approval_defaults_match_policy() {
        let s = specs();
        let spec = |name: &str| s.iter().find(|t| t.name == name).unwrap();
        assert!(!spec("add_tag").needs_approval);
        assert!(!spec("update_notes").needs_approval);
        assert!(!spec("add_to_collection").needs_approval);
        assert!(spec("create_entry").needs_approval);
        assert!(spec("update_entry").needs_approval);
        assert!(spec("delete_entry").needs_approval);
    }

    // ── unknown tool → None ───────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_tool_returns_none(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("nonexistent_tool", json!({}));
        let result = try_execute(&ctx, &c).await;
        assert!(result.is_none());
    }

    // ── add_tag ────────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_creates_and_attaches_new_tag(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper A").await;
        let ctx = make_ctx(&pool);
        let c = call("add_tag", json!({"entry_id": entry_id, "tag_name": "ml"}));

        let result = try_execute(&ctx, &c).await.unwrap().unwrap();
        assert!(result.contains("ml"));

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags et
             JOIN tags t ON t.id = et.tag_id
             WHERE et.entry_id = ? AND t.name = ?",
        )
        .bind(entry_id)
        .bind("ml")
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_reuses_existing_tag(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper A").await;
        let ctx = make_ctx(&pool);

        // Create the tag first
        crate::db::tags::create_tag(&pool, "nlp").await.unwrap();

        let c = call("add_tag", json!({"entry_id": entry_id, "tag_name": "nlp"}));
        try_execute(&ctx, &c).await.unwrap().unwrap();

        // Only one tag with this name should exist
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE name = ?")
            .bind("nlp")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_missing_entry_id_returns_error(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("add_tag", json!({"tag_name": "ml"}));
        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::InvalidArguments(_))));
    }

    // ── update_notes ───────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn update_notes_sets_notes(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper B").await;
        let ctx = make_ctx(&pool);
        let c = call(
            "update_notes",
            json!({"entry_id": entry_id, "notes": "Very interesting paper."}),
        );

        try_execute(&ctx, &c).await.unwrap().unwrap();

        let notes: Option<String> =
            sqlx::query_scalar("SELECT notes FROM entries WHERE id = ?")
                .bind(entry_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(notes.as_deref(), Some("Very interesting paper."));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_notes_not_found_returns_db_error(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("update_notes", json!({"entry_id": 9999, "notes": "hi"}));
        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::Db(_))));
    }

    // ── add_to_collection ──────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn add_to_collection_links_entry(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper C").await;
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Reading')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let ctx = make_ctx(&pool);
        let c = call(
            "add_to_collection",
            json!({"entry_id": entry_id, "collection_id": col_id}),
        );

        try_execute(&ctx, &c).await.unwrap().unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_collections WHERE entry_id = ? AND collection_id = ?",
        )
        .bind(entry_id)
        .bind(col_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    // ── create_entry ───────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_returns_id_and_persists(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call(
            "create_entry",
            json!({
                "title": "A New Paper",
                "entry_type": "article",
                "year": 2025
            }),
        );

        let result = try_execute(&ctx, &c).await.unwrap().unwrap();
        assert!(result.contains("id="));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE title = ?")
            .bind("A New Paper")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_defaults_type_to_misc(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("create_entry", json!({"title": "Minimal Entry"}));
        try_execute(&ctx, &c).await.unwrap().unwrap();

        let entry_type: String =
            sqlx::query_scalar("SELECT entry_type FROM entries WHERE title = ?")
                .bind("Minimal Entry")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(entry_type, "misc");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_missing_title_returns_error(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("create_entry", json!({"year": 2025}));
        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::InvalidArguments(_))));
    }

    // ── update_entry ───────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_changes_title(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Old Title").await;
        let ctx = make_ctx(&pool);
        let c = call(
            "update_entry",
            json!({"entry_id": entry_id, "title": "New Title"}),
        );

        try_execute(&ctx, &c).await.unwrap().unwrap();

        let title: String = sqlx::query_scalar("SELECT title FROM entries WHERE id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(title, "New Title");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_preserves_unchanged_fields(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                year: Some(2020),
                doi: Some("10.1/test".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let ctx = make_ctx(&pool);
        let c = call("update_entry", json!({"entry_id": id, "title": "Updated Paper"}));
        try_execute(&ctx, &c).await.unwrap().unwrap();

        let entry = crate::db::entries::get_entry(&pool, id).await.unwrap();
        assert_eq!(entry.title, "Updated Paper");
        assert_eq!(entry.year, Some(2020));
        assert_eq!(entry.doi.as_deref(), Some("10.1/test"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_not_found_returns_db_error(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("update_entry", json!({"entry_id": 9999, "title": "X"}));
        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::Db(_))));
    }

    // ── delete_entry ───────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_removes_it(pool: SqlitePool) {
        let entry_id = make_entry(&pool, "Paper D").await;
        let ctx = make_ctx(&pool);
        let c = call("delete_entry", json!({"entry_id": entry_id}));

        try_execute(&ctx, &c).await.unwrap().unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_not_found_returns_db_error(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("delete_entry", json!({"entry_id": 9999}));
        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::Db(_))));
    }
}
