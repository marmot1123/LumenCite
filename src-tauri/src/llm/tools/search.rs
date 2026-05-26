//! Read-only ("search") tools the chat LLM can call.
//!
//! Tools provided:
//! - `fulltext_search`  — keyword search over indexed attachment text
//! - `get_entry`        — fetch full metadata for a single entry by id
//! - `list_collections` — return all collections
//! - `list_tags`        — return all tags

use serde_json::json;

use crate::db::{collections, entries, fulltext, tags};
use crate::llm::{ToolCallSpec, ToolSpec};
use crate::llm::tools::{ToolContext, ToolError};

/// Return the `ToolSpec` descriptors for all tools in this module.
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "fulltext_search".to_string(),
            description: "Search the full text of all indexed attachments (PDFs) in the library. \
                Returns a list of matching pages with snippets. Use this to find papers that \
                discuss a specific concept, method, or term. Respects the current scope.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keywords to search for in the library's full text (space-separated; multiple words are ANDed)"
                    }
                },
                "required": ["query"]
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "get_entry".to_string(),
            description: "Retrieve full metadata for a single library entry by its numeric id. \
                Returns title, year, authors, abstract, tags, DOI/arXiv id, and notes. \
                Use this after fulltext_search to fetch details about a specific result.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "Numeric id of the entry to retrieve"
                    }
                },
                "required": ["entry_id"]
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "list_collections".to_string(),
            description: "List all collections (folders) in the library. \
                Returns id, name, and parent_id for each collection. \
                Use this to understand the library organisation or to look up collection ids \
                that can be passed to other queries.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "list_tags".to_string(),
            description: "List all tags in the library. \
                Returns id and name for each tag. \
                Use this to discover available tags or to look up tag ids.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
            needs_approval: false,
        },
    ]
}

/// Attempt to handle `call`. Returns `Some(result)` if the tool name matches one
/// of this module's tools, `None` otherwise.
pub async fn try_execute(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Option<Result<String, ToolError>> {
    match call.tool_name.as_str() {
        "fulltext_search" => Some(fulltext_search(ctx, call).await),
        "get_entry" => Some(get_entry_tool(ctx, call).await),
        "list_collections" => Some(list_collections_tool(ctx).await),
        "list_tags" => Some(list_tags_tool(ctx).await),
        _ => None,
    }
}

// ─── individual tool handlers ────────────────────────────────────────────────

async fn fulltext_search(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let query = call
        .arguments
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArguments("missing required argument: query".to_string()))?;

    let mut hits = fulltext::search_fulltext(ctx.pool, query, None, None).await?;

    // Scope filtering: when mode is "entries", keep only hits whose entry_id is in scope.
    if ctx.scope_mode == "entries" {
        hits.retain(|h| ctx.scope_entry_ids.contains(&h.entry.id));
    }

    let n = hits.len();

    let items: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            json!({
                "entry_id": h.entry.id,
                "page": h.page,
                "snippet": h.snippet
            })
        })
        .collect();

    let result = json!({
        "count": n,
        "hits": items
    });

    Ok(format!("{n} hits\n{}", serde_json::to_string(&result).unwrap_or_default()))
}

async fn get_entry_tool(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Result<String, ToolError> {
    let entry_id = call
        .arguments
        .get("entry_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            ToolError::InvalidArguments("missing required argument: entry_id (integer)".to_string())
        })?;

    let detail = match entries::get_entry(ctx.pool, entry_id).await {
        Ok(d) => d,
        Err(sqlx::Error::RowNotFound) => {
            return Ok(format!("entry {entry_id} not found"));
        }
        Err(e) => return Err(ToolError::Db(e)),
    };

    let author_names: Vec<&str> = detail.authors.iter().map(|a| a.name.as_str()).collect();
    let tag_names: Vec<&str> = detail.tags.iter().map(|t| t.name.as_str()).collect();

    let obj = json!({
        "id": detail.id,
        "title": detail.title,
        "year": detail.year,
        "entry_type": detail.entry_type,
        "authors": author_names,
        "tags": tag_names,
        "doi": detail.doi,
        "arxiv_id": detail.arxiv_id,
        "abstract": detail.abstract_,
        "notes": detail.notes
    });

    Ok(serde_json::to_string(&obj).unwrap_or_default())
}

async fn list_collections_tool(ctx: &ToolContext<'_>) -> Result<String, ToolError> {
    let cols = collections::get_collections(ctx.pool).await?;

    // Flatten the nested tree into a flat list with parent_id preserved.
    fn flatten(cols: &[crate::models::Collection], out: &mut Vec<serde_json::Value>) {
        for c in cols {
            out.push(json!({
                "id": c.id,
                "name": c.name,
                "parent_id": c.parent_id
            }));
            flatten(&c.children, out);
        }
    }

    let mut items = Vec::new();
    flatten(&cols, &mut items);

    Ok(serde_json::to_string(&items).unwrap_or_default())
}

async fn list_tags_tool(ctx: &ToolContext<'_>) -> Result<String, ToolError> {
    let tag_list = tags::get_tags(ctx.pool).await?;

    let items: Vec<serde_json::Value> = tag_list
        .iter()
        .map(|t| json!({ "id": t.id, "name": t.name }))
        .collect();

    Ok(serde_json::to_string(&items).unwrap_or_default())
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    use crate::db::attachments::add_attachment;
    use crate::db::collections::create_collection;
    use crate::db::entries::create_entry;
    use crate::db::fulltext::index_attachment;
    use crate::db::tags::create_tag;
    use crate::models::EntryInput;

    fn make_call(tool_name: &str, args: serde_json::Value) -> ToolCallSpec {
        ToolCallSpec {
            call_id: "test-call-1".to_string(),
            tool_name: tool_name.to_string(),
            arguments: args,
        }
    }

    fn ctx_all(pool: &SqlitePool) -> ToolContext<'_> {
        ToolContext {
            pool,
            session_id: 1,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
        }
    }

    // ── fulltext_search ──────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn fulltext_search_returns_hits(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Attention Paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let att = add_attachment(
            &pool,
            entry.id,
            "attachments/a/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();

        index_attachment(
            &pool,
            att.id,
            &[(1, "Transformer architecture for NLP tasks.".to_string())],
        )
        .await
        .unwrap();

        let ctx = ctx_all(&pool);
        let call = make_call("fulltext_search", json!({ "query": "transformer" }));

        let result = try_execute(&ctx, &call).await;
        assert!(result.is_some(), "should handle fulltext_search");
        let s = result.unwrap().unwrap();
        assert!(s.contains("1 hits"), "should report 1 hit, got: {s}");
        assert!(s.contains("\"entry_id\""), "should contain entry_id");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn fulltext_search_missing_query_is_invalid_args(pool: SqlitePool) {
        let ctx = ctx_all(&pool);
        let call = make_call("fulltext_search", json!({}));
        let result = try_execute(&ctx, &call).await.unwrap();
        assert!(matches!(result, Err(ToolError::InvalidArguments(_))));
    }

    // ── scope filtering ──────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn fulltext_search_scope_entries_filters_results(pool: SqlitePool) {
        // Create two entries, both indexed with the same keyword.
        let e1 = create_entry(
            &pool,
            &EntryInput {
                title: "Entry One".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let e2 = create_entry(
            &pool,
            &EntryInput {
                title: "Entry Two".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        for (eid, label) in [(e1.id, "a"), (e2.id, "b")] {
            let att = add_attachment(
                &pool,
                eid,
                &format!("attachments/{label}/p.pdf"),
                "p.pdf",
                "application/pdf",
            )
            .await
            .unwrap();
            index_attachment(
                &pool,
                att.id,
                &[(1, "quantum computing research".to_string())],
            )
            .await
            .unwrap();
        }

        // Scope to only e1.
        let scope_ids = vec![e1.id];
        let ctx = ToolContext {
            pool: &pool,
            session_id: 1,
            scope_mode: "entries",
            scope_entry_ids: &scope_ids,
            mcp: None,
        };
        let call = make_call("fulltext_search", json!({ "query": "quantum" }));

        let s = try_execute(&ctx, &call).await.unwrap().unwrap();
        assert!(s.contains("1 hits"), "scope should exclude e2, got: {s}");
        let parsed: serde_json::Value = serde_json::from_str(s.splitn(2, '\n').nth(1).unwrap()).unwrap();
        assert_eq!(parsed["hits"][0]["entry_id"], e1.id);
    }

    // ── get_entry ────────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_returns_title(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Test Paper".to_string(),
                entry_type: "article".to_string(),
                year: Some(2024),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let ctx = ctx_all(&pool);
        let call = make_call("get_entry", json!({ "entry_id": entry.id }));

        let s = try_execute(&ctx, &call).await.unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["title"], "Test Paper");
        assert_eq!(parsed["year"], 2024);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_not_found_returns_ok_message(pool: SqlitePool) {
        let ctx = ctx_all(&pool);
        let call = make_call("get_entry", json!({ "entry_id": 99999 }));

        let result = try_execute(&ctx, &call).await.unwrap();
        let s = result.unwrap(); // should be Ok, not Err
        assert!(s.contains("not found"), "expected 'not found' message, got: {s}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_entry_missing_arg_is_invalid(pool: SqlitePool) {
        let ctx = ctx_all(&pool);
        let call = make_call("get_entry", json!({}));
        let result = try_execute(&ctx, &call).await.unwrap();
        assert!(matches!(result, Err(ToolError::InvalidArguments(_))));
    }

    // ── list_collections ─────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn list_collections_returns_created_rows(pool: SqlitePool) {
        create_collection(&pool, "Inbox", None).await.unwrap();
        create_collection(&pool, "Archive", None).await.unwrap();

        let ctx = ctx_all(&pool);
        let call = make_call("list_collections", json!({}));

        let s = try_execute(&ctx, &call).await.unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let names: Vec<&str> = arr.iter().map(|v| v["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"Inbox"));
        assert!(names.contains(&"Archive"));
    }

    // ── list_tags ────────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn list_tags_returns_created_rows(pool: SqlitePool) {
        create_tag(&pool, "NLP").await.unwrap();
        create_tag(&pool, "CV").await.unwrap();

        let ctx = ctx_all(&pool);
        let call = make_call("list_tags", json!({}));

        let s = try_execute(&ctx, &call).await.unwrap().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        let names: Vec<&str> = arr.iter().map(|v| v["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"NLP"));
        assert!(names.contains(&"CV"));
    }

    // ── unknown tool ─────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_tool_returns_none(pool: SqlitePool) {
        let ctx = ctx_all(&pool);
        let call = make_call("nonexistent_tool", json!({}));
        let result = try_execute(&ctx, &call).await;
        assert!(result.is_none(), "unknown tool should return None");
    }
}
