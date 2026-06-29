//! write 系ツール: add_tag / update_notes / add_to_collection /
//! create_entry / update_entry / delete_entry。
//!
//! 契約は `super`（tools/mod.rs）参照。`specs()` で LLM 向け定義を、
//! `try_execute()` で実行を提供する。承認可否の判定は `super::approval` を参照。

use super::{ToolContext, ToolError};
use crate::llm::{ToolCallSpec, ToolSpec};
use crate::models::EntryInput;
use serde_json::json;
use std::collections::HashMap;

/// write 系ツールの定義一覧。
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "add_tag".to_string(),
            description: "Add a tag to one or more entries in a single call. The tag is created if it \
                          does not already exist. This is idempotent — re-tagging an entry has no effect. \
                          Use \"entry_ids\" to tag many entries at once instead of calling this repeatedly."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "The ID of a single entry to tag. Provide this or \"entry_ids\"."
                    },
                    "entry_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "IDs of multiple entries to tag in one call. Provide this or \"entry_id\"."
                    },
                    "tag_name": {
                        "type": "string",
                        "description": "The name of the tag to attach (e.g. \"machine-learning\")."
                    }
                },
                "required": ["tag_name"]
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
            description: "Add one or more entries to a collection in a single call. \
                          This is idempotent — entries already in the collection are unchanged. \
                          Use \"entry_ids\" to add many entries at once instead of calling this repeatedly."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": {
                        "type": "integer",
                        "description": "The ID of a single entry to add. Provide this or \"entry_ids\"."
                    },
                    "entry_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "IDs of multiple entries to add in one call. Provide this or \"entry_id\"."
                    },
                    "collection_id": {
                        "type": "integer",
                        "description": "The ID of the target collection."
                    }
                },
                "required": ["collection_id"]
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
                    "citation_key": {
                        "type": "string",
                        "description": "Pinned BibTeX citation key (the key used in LaTeX \\cite{...}). \
                            Optional and must be globally unique; if omitted, the key is auto-generated \
                            from first author + year at export time. Allowed characters: alphanumerics \
                            and _ : - . / + (others are stripped)."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Personal notes."
                    },
                    "author_names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of author names in display order."
                    },
                    "extra_fields": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Type-specific bibliographic fields as string key/value pairs. \
                            Common keys: \"journal\", \"volume\", \"issue\", \"number\", \"pages\", \
                            \"publisher\", \"booktitle\", \"address\", \"edition\", \"series\", \
                            \"school\", \"institution\", \"organization\", \"howpublished\"."
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
                    "citation_key": {
                        "type": "string",
                        "description": "New pinned BibTeX citation key (used in LaTeX \\cite{...}). \
                            Must be globally unique. Omit to keep the current key unchanged; pass an \
                            empty string to unpin and revert to an auto-generated key. Allowed \
                            characters: alphanumerics and _ : - . / + (others are stripped)."
                    },
                    "notes": {
                        "type": "string",
                        "description": "New notes."
                    },
                    "author_names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Replacement author list (replaces existing authors entirely)."
                    },
                    "extra_fields": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Type-specific bibliographic fields to set/overwrite, as string \
                            key/value pairs. Only the provided keys are changed; existing extra fields \
                            not listed here are preserved. Common keys: \"journal\", \"volume\", \
                            \"issue\", \"number\", \"pages\", \"publisher\", \"booktitle\", \"address\", \
                            \"edition\", \"series\", \"school\", \"institution\", \"organization\", \
                            \"howpublished\"."
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
    let entry_ids = parse_entry_ids(&call.arguments)?;
    let tag_name = parse_str(&call.arguments, "tag_name")?;

    // 存在しない ID は先に切り分け、実在 ID にのみ適用する。
    let present = existing_entry_ids(ctx.pool, &entry_ids).await?;
    let (found, missing) = partition_existing(entry_ids, &present);
    if found.is_empty() {
        return Err(ToolError::InvalidArguments(format!(
            "no matching entries to tag (not found: {missing:?})"
        )));
    }

    // Get-or-create the tag by name (once for the whole batch)
    let all_tags = crate::db::tags::get_tags(ctx.pool).await?;
    let tag = if let Some(existing) = all_tags.into_iter().find(|t| t.name == tag_name) {
        existing
    } else {
        crate::db::tags::create_tag(ctx.pool, &tag_name).await?
    };

    // 実在エントリにのみ適用。ここでの失敗は真の DB エラーなので伝播させる。
    for id in &found {
        crate::db::tags::add_tag_to_entry(ctx.pool, *id, tag.id).await?;
    }

    Ok(bulk_summary(&format!("Tag \"{}\" (id={})", tag.name, tag.id), &found, &missing))
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
    let entry_ids = parse_entry_ids(&call.arguments)?;
    let collection_id = parse_i64(&call.arguments, "collection_id")?;

    // コレクション不在を「エントリが見つからない」と誤報しないよう先に検証する。
    let collection_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?)")
            .bind(collection_id)
            .fetch_one(ctx.pool)
            .await?;
    if !collection_exists {
        return Err(ToolError::InvalidArguments(format!(
            "collection {collection_id} not found"
        )));
    }

    let present = existing_entry_ids(ctx.pool, &entry_ids).await?;
    let (found, missing) = partition_existing(entry_ids, &present);
    if found.is_empty() {
        return Err(ToolError::InvalidArguments(format!(
            "no matching entries to add (not found: {missing:?})"
        )));
    }

    // 実在エントリにのみ適用。ここでの失敗は真の DB エラーなので伝播させる。
    for id in &found {
        crate::db::collections::add_entry_to_collection(ctx.pool, *id, collection_id).await?;
    }

    Ok(bulk_summary(&format!("Added to collection {collection_id}"), &found, &missing))
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

    let extra_fields = parse_extra_fields(args).unwrap_or_default();

    // 省略時・空文字時は None（自動生成）。重複は事前検証で弾く。
    let citation_key = validate_citation_key_arg(ctx, args, None).await?.flatten();

    let input = EntryInput {
        title,
        entry_type,
        year,
        citation_key,
        abstract_,
        doi,
        isbn,
        arxiv_id,
        url,
        notes,
        author_names,
        extra_fields,
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
        // ピン留め済みの固定 cite key を引き継ぐ。引き継がないと update_entry が
        // citation_key = NULL で上書きし、ユーザーが固定したキーが自動生成に戻ってしまう。
        // LLM ツールには citation_key の上書き口を設けていないため、ここで常に現状維持する。
        citation_key: current.citation_key.clone(),
        doi: current.doi.clone(),
        isbn: current.isbn.clone(),
        arxiv_id: current.arxiv_id.clone(),
        url: current.url.clone(),
        abstract_: current.abstract_.clone(),
        notes: current.notes.clone(),
        author_names: current.authors.iter().map(|a| a.name.clone()).collect(),
        // タグも update_entry が「全削除 → tag_ids から再挿入」で全置換するため、
        // 引き継がないと tag_ids=[] で既存タグが丸ごと消える（citation_key と同じクラスのバグ）。
        // LLM ツールにはタグ編集口を設けていないので、ここで常に現状維持する（タグ操作は add_tag）。
        tag_ids: current.tags.iter().map(|t| t.id).collect(),
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
    // 既存の extra_fields に対し、指定されたキーだけを上書き/追加する（指定外は保持）。
    if let Some(provided) = parse_extra_fields(args) {
        input.extra_fields.extend(provided);
    }
    // citation_key は引数があるときだけ上書き。Some(key)=ピン留め、None=自動に戻す。
    // 引数が無ければ上で引き継いだ現状値を保持する。
    if let Some(new_key) = validate_citation_key_arg(ctx, args, Some(entry_id)).await? {
        input.citation_key = new_key;
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

/// `entry_id`（単一）または `entry_ids`（整数配列）から対象エントリ ID を取り出す。
/// 両方与えられたら結合し、重複は順序を保って除去する。どちらも無ければ／配列に
/// 非整数が混じればエラー（バルク系ツール add_tag / add_to_collection で共用）。
fn parse_entry_ids(args: &serde_json::Value) -> Result<Vec<i64>, ToolError> {
    let mut ids: Vec<i64> = Vec::new();
    if let Some(v) = args.get("entry_id").and_then(|v| v.as_i64()) {
        ids.push(v);
    }
    if let Some(arr) = args.get("entry_ids").and_then(|v| v.as_array()) {
        for item in arr {
            let id = item.as_i64().ok_or_else(|| {
                ToolError::InvalidArguments("\"entry_ids\" must be an array of integers".to_string())
            })?;
            ids.push(id);
        }
    }
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id));
    if ids.is_empty() {
        return Err(ToolError::InvalidArguments(
            "provide \"entry_id\" (integer) or \"entry_ids\" (array of integers)".to_string(),
        ));
    }
    Ok(ids)
}

/// `ids` のうち実在するエントリ ID の集合を返す。存在判定を FK 違反の有無に頼らず
/// 明示クエリで行うことで、「存在しない ID（スキップ対象）」と「実在 ID への真の DB
/// エラー（伝播対象）」を確実に切り分けられる（テストでも FK pragma に依存しない）。
async fn existing_entry_ids(
    pool: &sqlx::SqlitePool,
    ids: &[i64],
) -> Result<std::collections::HashSet<i64>, ToolError> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT id FROM entries WHERE id IN (");
    let mut sep = qb.separated(", ");
    for id in ids {
        sep.push_bind(*id);
    }
    qb.push(")");
    let rows: Vec<i64> = qb.build_query_scalar().fetch_all(pool).await?;
    Ok(rows.into_iter().collect())
}

/// 対象 ID を実在（`found`）と不在（`missing`）に分ける。順序は入力に従う。
fn partition_existing(entry_ids: Vec<i64>, present: &std::collections::HashSet<i64>) -> (Vec<i64>, Vec<i64>) {
    entry_ids.into_iter().partition(|id| present.contains(id))
}

/// バルク write の結果サマリ（純粋なフォーマッタ）。`applied` は実在し適用済みの ID、
/// `skipped` は存在せずスキップした ID。呼び出し側が `applied` 非空を保証する。
fn bulk_summary(subject: &str, applied: &[i64], skipped: &[i64]) -> String {
    let mut msg = if applied.len() == 1 {
        format!("{subject} applied to entry {}.", applied[0])
    } else {
        format!("{subject} applied to {} entries: {applied:?}.", applied.len())
    };
    if !skipped.is_empty() {
        msg.push_str(&format!(" Skipped {} not found: {skipped:?}.", skipped.len()));
    }
    msg
}

/// `extra_fields` 引数を `{string: string}` の対として取り出す。
/// 文字列以外の値は無視する。引数が無ければ `None`。
fn parse_extra_fields(args: &serde_json::Value) -> Option<HashMap<String, String>> {
    args.get("extra_fields")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
}

/// `citation_key` 引数を取り出してサニタイズ・衝突検証する。戻り値の意味:
/// - `None`            … 引数が無い（呼び出し側は「変更なし」として扱う）
/// - `Some(None)`      … 値が空 / サニタイズ後に空（= 自動生成にフォールバック）
/// - `Some(Some(key))` … 有効なピン留めキー（他エントリと重複しないことを確認済み）
///
/// `exclude_id` には更新中エントリ自身の id を渡し、自分との衝突を除外する。
/// 重複時は `InvalidArguments` を返し、LLM が別キーを選び直せるようにする
/// （DB 側の UNIQUE 制約も最終防衛として効くが、ここで分かりやすく弾く）。
async fn validate_citation_key_arg(
    ctx: &ToolContext<'_>,
    args: &serde_json::Value,
    exclude_id: Option<i64>,
) -> Result<Option<Option<String>>, ToolError> {
    let Some(raw) = args.get("citation_key").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let sanitized = crate::db::entries::sanitize_citation_key(raw);
    if let Some(ref key) = sanitized {
        if !crate::db::entries::is_citation_key_available(ctx.pool, key, exclude_id).await? {
            return Err(ToolError::InvalidArguments(format!(
                "citation key \"{key}\" is already in use by another entry; choose a different key"
            )));
        }
    }
    Ok(Some(sanitized))
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
            app_data_dir: std::path::Path::new(""),
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

    // ── bulk: entry_ids ────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_bulk_tags_multiple_entries(pool: SqlitePool) {
        let e1 = make_entry(&pool, "P1").await;
        let e2 = make_entry(&pool, "P2").await;
        let e3 = make_entry(&pool, "P3").await;
        let ctx = make_ctx(&pool);
        let c = call("add_tag", json!({"entry_ids": [e1, e2, e3], "tag_name": "ml"}));

        let msg = try_execute(&ctx, &c).await.unwrap().unwrap();
        assert!(msg.contains("3 entries"), "summary was: {msg}");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags et JOIN tags t ON t.id = et.tag_id WHERE t.name = 'ml'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 3);

        // タグは1度だけ作成される（バッチで使い回す）。
        let tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE name = 'ml'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tags, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_combines_entry_id_and_entry_ids_and_dedups(pool: SqlitePool) {
        let e1 = make_entry(&pool, "P1").await;
        let e2 = make_entry(&pool, "P2").await;
        let ctx = make_ctx(&pool);
        // entry_id と entry_ids 併用、e1 は重複 → 2 件に畳まれる。
        let c = call("add_tag", json!({"entry_id": e1, "entry_ids": [e1, e2], "tag_name": "z"}));

        let msg = try_execute(&ctx, &c).await.unwrap().unwrap();
        assert!(msg.contains("2 entries"), "summary was: {msg}");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags et JOIN tags t ON t.id = et.tag_id WHERE t.name = 'z'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_to_collection_bulk_links_multiple_entries(pool: SqlitePool) {
        let e1 = make_entry(&pool, "P1").await;
        let e2 = make_entry(&pool, "P2").await;
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Reading')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();
        let ctx = make_ctx(&pool);
        let c = call(
            "add_to_collection",
            json!({"entry_ids": [e1, e2], "collection_id": col_id}),
        );

        try_execute(&ctx, &c).await.unwrap().unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_collections WHERE collection_id = ?",
        )
        .bind(col_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 2);
    }

    // ── bulk helpers (pure) ────────────────────────────────────────────────

    #[test]
    fn parse_entry_ids_combines_and_dedups() {
        let ids = parse_entry_ids(&json!({"entry_id": 1, "entry_ids": [1, 2, 3]})).unwrap();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn parse_entry_ids_requires_at_least_one_id() {
        assert!(matches!(
            parse_entry_ids(&json!({})),
            Err(ToolError::InvalidArguments(_))
        ));
    }

    #[test]
    fn parse_entry_ids_rejects_non_integer_array() {
        assert!(matches!(
            parse_entry_ids(&json!({"entry_ids": ["a", "b"]})),
            Err(ToolError::InvalidArguments(_))
        ));
    }

    #[test]
    fn bulk_summary_reports_skipped() {
        let m = bulk_summary("Tag x", &[1, 2], &[9]);
        assert!(m.contains("2 entries"));
        assert!(m.contains("Skipped 1"));
    }

    #[test]
    fn bulk_summary_no_skips_omits_skipped_clause() {
        let m = bulk_summary("Tag x", &[1], &[]);
        assert!(m.contains("entry 1"));
        assert!(!m.contains("Skipped"));
    }

    // 存在判定は明示クエリなので FK pragma に依存せずテストできる（add_tag の skip 系）。
    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_skips_missing_entries_and_tags_present(pool: SqlitePool) {
        let e1 = make_entry(&pool, "P1").await;
        let ctx = make_ctx(&pool);
        let c = call("add_tag", json!({"entry_ids": [e1, 99999], "tag_name": "x"}));

        let msg = try_execute(&ctx, &c).await.unwrap().unwrap();
        assert!(msg.contains("Skipped"), "summary was: {msg}");
        assert!(msg.contains("99999"), "summary was: {msg}");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags et JOIN tags t ON t.id = et.tag_id WHERE t.name = 'x'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "only the existing entry should be tagged");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_all_missing_returns_error_and_creates_no_tag(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call("add_tag", json!({"entry_ids": [99998, 99999], "tag_name": "ghost"}));

        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::InvalidArguments(_))));

        // 1件も該当しなければタグも作らない。
        let tags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE name = 'ghost'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tags, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_to_collection_unknown_collection_reports_collection_not_entries(pool: SqlitePool) {
        let e1 = make_entry(&pool, "P1").await;
        let ctx = make_ctx(&pool);
        // 実在エントリ + 存在しない collection_id → エントリではなくコレクションのエラー。
        let c = call(
            "add_to_collection",
            json!({"entry_ids": [e1], "collection_id": 4242}),
        );

        let result = try_execute(&ctx, &c).await.unwrap();
        match result {
            Err(ToolError::InvalidArguments(m)) => {
                assert!(m.contains("collection 4242"), "message was: {m}");
            }
            other => panic!("expected InvalidArguments about the collection, got {other:?}"),
        }
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
    async fn create_entry_persists_extra_fields(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call(
            "create_entry",
            json!({
                "title": "Journal Paper",
                "entry_type": "article",
                "extra_fields": { "journal": "Nature", "volume": "42", "pages": "1-9" }
            }),
        );
        try_execute(&ctx, &c).await.unwrap().unwrap();

        let id: i64 = sqlx::query_scalar("SELECT id FROM entries WHERE title = ?")
            .bind("Journal Paper")
            .fetch_one(&pool)
            .await
            .unwrap();
        let entry = crate::db::entries::get_entry(&pool, id).await.unwrap();
        assert_eq!(entry.extra_fields.get("journal").map(String::as_str), Some("Nature"));
        assert_eq!(entry.extra_fields.get("volume").map(String::as_str), Some("42"));
        assert_eq!(entry.extra_fields.get("pages").map(String::as_str), Some("1-9"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_merges_extra_fields(pool: SqlitePool) {
        // 既存の extra_fields: journal=Old, volume=1
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                extra_fields: HashMap::from([
                    ("journal".to_string(), "Old Journal".to_string()),
                    ("volume".to_string(), "1".to_string()),
                ]),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let ctx = make_ctx(&pool);
        // journal を上書き + issue を追加。volume は触らない。
        let c = call(
            "update_entry",
            json!({
                "entry_id": id,
                "extra_fields": { "journal": "New Journal", "issue": "3" }
            }),
        );
        try_execute(&ctx, &c).await.unwrap().unwrap();

        let entry = crate::db::entries::get_entry(&pool, id).await.unwrap();
        assert_eq!(entry.extra_fields.get("journal").map(String::as_str), Some("New Journal"));
        assert_eq!(entry.extra_fields.get("issue").map(String::as_str), Some("3"));
        // 指定しなかった volume は保持される。
        assert_eq!(entry.extra_fields.get("volume").map(String::as_str), Some("1"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_pins_citation_key(pool: SqlitePool) {
        let ctx = make_ctx(&pool);
        let c = call(
            "create_entry",
            json!({ "title": "Paper", "entry_type": "article", "citation_key": "lovelace1843" }),
        );
        try_execute(&ctx, &c).await.unwrap().unwrap();

        let key: Option<String> =
            sqlx::query_scalar("SELECT citation_key FROM entries WHERE title = ?")
                .bind("Paper")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(key.as_deref(), Some("lovelace1843"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entry_duplicate_citation_key_is_invalid(pool: SqlitePool) {
        // 既に同じキーをピン留めしたエントリがある状態。
        create_entry(
            &pool,
            &EntryInput {
                title: "First".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("dup2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let ctx = make_ctx(&pool);
        let c = call(
            "create_entry",
            json!({ "title": "Second", "citation_key": "dup2020" }),
        );
        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::InvalidArguments(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_sets_and_unpins_citation_key(pool: SqlitePool) {
        let id = make_entry(&pool, "Paper").await;
        let ctx = make_ctx(&pool);

        // ピン留めする。
        let c = call("update_entry", json!({ "entry_id": id, "citation_key": "pinned2024" }));
        try_execute(&ctx, &c).await.unwrap().unwrap();
        let key: Option<String> =
            sqlx::query_scalar("SELECT citation_key FROM entries WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(key.as_deref(), Some("pinned2024"));

        // 空文字で unpin → NULL（自動生成）に戻る。
        let c = call("update_entry", json!({ "entry_id": id, "citation_key": "" }));
        try_execute(&ctx, &c).await.unwrap().unwrap();
        let key: Option<String> =
            sqlx::query_scalar("SELECT citation_key FROM entries WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(key, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_duplicate_citation_key_is_invalid(pool: SqlitePool) {
        create_entry(
            &pool,
            &EntryInput {
                title: "Owner".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("taken2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let other = make_entry(&pool, "Other").await;

        let ctx = make_ctx(&pool);
        let c = call("update_entry", json!({ "entry_id": other, "citation_key": "taken2020" }));
        let result = try_execute(&ctx, &c).await.unwrap();
        assert!(matches!(result, Err(ToolError::InvalidArguments(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_repins_same_key_succeeds(pool: SqlitePool) {
        // 自分が既に持つキーを再指定しても自己衝突で弾かれないこと。
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Self".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("self2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let ctx = make_ctx(&pool);
        let c = call("update_entry", json!({ "entry_id": id, "citation_key": "self2020" }));
        try_execute(&ctx, &c).await.unwrap().unwrap();

        let entry = crate::db::entries::get_entry(&pool, id).await.unwrap();
        assert_eq!(entry.citation_key.as_deref(), Some("self2020"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_preserves_existing_tags(pool: SqlitePool) {
        // タグの付いたエントリ。
        let entry_id = make_entry(&pool, "Tagged Paper").await;
        let tag = crate::db::tags::create_tag(&pool, "ml").await.unwrap();
        crate::db::tags::add_tag_to_entry(&pool, entry_id, tag.id)
            .await
            .unwrap();

        let ctx = make_ctx(&pool);
        // タグには一切触れない update（notes だけ変更）。
        let c = call(
            "update_entry",
            json!({ "entry_id": entry_id, "notes": "updated via chat" }),
        );
        try_execute(&ctx, &c).await.unwrap().unwrap();

        // 既存タグが消えずに残っていること。
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags WHERE entry_id = ? AND tag_id = ?",
        )
        .bind(entry_id)
        .bind(tag.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "existing tags must survive update_entry");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_entry_preserves_pinned_citation_key(pool: SqlitePool) {
        // ユーザーがピン留めした固定 cite key を持つエントリ。
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("smith2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let ctx = make_ctx(&pool);
        // citation_key には一切触れない update（notes だけ変更）。
        let c = call(
            "update_entry",
            json!({ "entry_id": id, "notes": "updated via chat" }),
        );
        try_execute(&ctx, &c).await.unwrap().unwrap();

        // ピン留めキーが NULL に巻き戻されず保持されていること。
        let entry = crate::db::entries::get_entry(&pool, id).await.unwrap();
        assert_eq!(entry.citation_key.as_deref(), Some("smith2020"));
        assert_eq!(entry.notes.as_deref(), Some("updated via chat"));
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
