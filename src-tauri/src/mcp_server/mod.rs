//! LumenCite を MCP **サーバー**として公開する。
//!
//! これは外部 MCP サーバーへ接続する `mcp`（クライアント）とは逆向きで、
//! Claude Desktop / Claude Code などの MCP クライアントが LumenCite のライブラリを
//! ツール経由で参照・操作できるようにするもの。サーバー側では LLM を呼ばない（推論は
//! 接続元のサブスクリプション側が担う）ため、API キー等は不要。
//!
//! ## 範囲
//! - トランスポート: localhost にバインドする HTTP（JSON-RPC 2.0 / 単発 POST → JSON 応答）
//! - 認可: `Authorization: Bearer <token>`（インストールごとの token。キーチェーン保管）
//! - **read 系（常時公開）**: `search` モジュールの read ツール定義を流用（単一ソース）し、
//!   LaTeX 連携向けの `search_entries` / `resolve_citation_key` / `export_bibtex` を追加。
//! - **write 系（Phase 2・ゲート付き）**: `mcp_server.write_enabled`（既定 false）が有効なときだけ
//!   `add_tag` / `update_notes` / `add_to_collection` / `create_entry` / `update_entry` を公開する。
//!   承認 UI が無いためサーバー側でこのゲートを enforce する。**破壊系 `delete_entry` は常に非公開**。
//!   write 成功時は監査ログに記録し、`.bib` 同期キックと `entries-changed` イベントを発火する。
//!
//! プロトコルのディスパッチ（[`handle_rpc`]）はトランスポート非依存で、HTTP を介さず
//! 単体テストできる（副作用＝`.bib` 同期/UI イベントは HTTP 層が `RpcOutcome.mutated` を見て行う）。

pub mod clipper;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::mpsc::UnboundedSender;

use crate::llm::tools::{mutate, search, ToolContext, ToolError};
use crate::llm::ToolCallSpec;

/// MCP プロトコルバージョン（クライアント側 `mcp` と揃える）。
const PROTOCOL_VERSION: &str = "2024-11-05";

/// 設定が無いときの既定バインドポート。
pub const DEFAULT_PORT: u16 = 3917;

/// `search` モジュールから流用して公開する read ツール名。
const SHARED_READ_TOOLS: &[&str] = &[
    "fulltext_search",
    "get_entry",
    "list_collections",
    "list_tags",
];

/// `mcp_server.write_enabled` が有効なときだけ公開する write ツール名。
/// `mutate` モジュールの定義を流用するが、**破壊系 `delete_entry` は意図的に含めない**
/// （許可リスト外なので `tools/call` でも到達不可）。
const WRITE_TOOLS: &[&str] = &[
    "add_tag",
    "update_notes",
    "add_to_collection",
    "create_entry",
    "update_entry",
];

/// `handle_rpc` の結果。`response` は JSON-RPC 応答（通知なら None）、`mutated` は
/// write が成功したかどうか（HTTP 層が `.bib` 同期 / UI イベント発火の判断に使う）。
pub struct RpcOutcome {
    pub response: Option<Value>,
    pub mutated: bool,
}

/// `mcp_server.write_enabled` の現在値を読む（リクエスト毎に評価し、トグル変更を即反映）。
async fn write_enabled(pool: &SqlitePool) -> bool {
    crate::db::settings::get_setting(pool, crate::db::settings::MCP_SERVER_WRITE_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
}

// ─── ツール定義（tools/list） ────────────────────────────────────────────────

/// 公開するツールの MCP 形式定義（`{name, description, inputSchema}`）。
/// `write_on` が true のときは write 系（`WRITE_TOOLS`）も含める。
fn tool_specs(write_on: bool) -> Vec<Value> {
    // 既存チャットの read 系定義を流用する（定義の二重管理を避ける単一ソース）。
    let mut tools: Vec<Value> = search::specs()
        .into_iter()
        .filter(|s| SHARED_READ_TOOLS.contains(&s.name.as_str()))
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "inputSchema": s.parameters,
            })
        })
        .collect();

    // MCP 専用の read ツール（LaTeX ワークフロー向け）。
    tools.push(json!({
        "name": "search_entries",
        "description": "Search library entries by metadata (title, authors, tags, abstract, \
            identifiers, year) using the trigram FTS index. Returns lightweight entry summaries \
            ranked by relevance.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (space-separated terms are ANDed)." },
                "collection_id": { "type": "integer", "description": "Restrict the search to a collection id." },
                "tag_id": { "type": "integer", "description": "Restrict the search to a tag id." }
            },
            "required": ["query"]
        }
    }));
    tools.push(json!({
        "name": "resolve_citation_key",
        "description": "Return the BibTeX citation key actually used in LaTeX \\cite{} / .bib \
            exports for an entry — the user-pinned key, or an auto-generated first-author+year key \
            when none is pinned.",
        "inputSchema": {
            "type": "object",
            "properties": { "entry_id": { "type": "integer", "description": "Entry id." } },
            "required": ["entry_id"]
        }
    }));
    tools.push(json!({
        "name": "export_bibtex",
        "description": "Export entries as BibTeX. Pass citation_keys to export exactly the entries \
            for a set of LaTeX \\cite{} keys (the best way to build a paper's refs.bib): keys keep \
            the exact form used across the whole library — including disambiguating suffixes like \
            'smith2020a' — and unresolved keys are reported back in `missing`. Or pass entry_ids to \
            export specific entries by id, or omit both to export the whole library (trash \
            excluded). With citation_keys the result is a JSON object {bibtex, found, missing}; \
            otherwise the raw .bib text.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "citation_keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Citation keys (as in \\cite{}) to export; preserves library-wide keys and reports missing ones."
                },
                "entry_ids": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Entry ids to export; omit for the whole library."
                }
            }
        }
    }));
    tools.push(json!({
        "name": "find_entries_by_citation_keys",
        "description": "Resolve one or more BibTeX/LaTeX \\cite{} citation keys to library entries. \
            For each key, reports whether it was found and, if so, the matching entry (entry_id, \
            title, year, authors). Use this to bridge from \\cite keys in a .tex file to library \
            entry ids — users think in citation keys, not numeric ids. The keys are matched exactly \
            as they appear in .bib / \\cite{} exports (pinned or auto-generated, with suffixes).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "citation_keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Citation keys to resolve (as they appear in \\cite{})."
                }
            },
            "required": ["citation_keys"]
        }
    }));
    tools.push(json!({
        "name": "get_fulltext",
        "description": "Return the extracted full text of a library entry's indexed PDF, by \
            entry_id or citation_key. Use this to actually read and summarise a specific paper — \
            `get_entry` only returns metadata (abstract / notes), which are often empty. Returns \
            {entry_id, indexed, total_pages, truncated, next_page, text}. If the entry has no \
            attached/indexed PDF, `indexed` is false and there is no text — say so plainly and do \
            NOT answer from general knowledge. Long papers are paginated: pass `page_start` (from a \
            previous `next_page`) to keep reading, or raise `max_chars`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key (as in \\cite{}); alternative to entry_id." },
                "max_chars": { "type": "integer", "description": "Max characters to return this call (default 24000)." },
                "page_start": { "type": "integer", "description": "1-based PDF page to start from, for continuing a long paper (default 1)." }
            }
        }
    }));

    // LCIR（機械可読中間形式）の read ツール（Phase 3.5）。実験フラグ lcir.enabled で
    // 構築された論文だけが対象。未構築なら has_lcir=false を返す（get_fulltext に退避可能）。
    tools.push(json!({
        "name": "get_document_structure",
        "description": "Return the recovered logical structure (LCIR) of a paper's PDF — its \
            section outline, block-type counts, and abstract — by entry_id or citation_key. Unlike \
            get_fulltext (flat page text), this exposes headings/sections with their numbers and \
            reports how many paragraphs, display equations, captions and bibliography entries were \
            found. Structure is heuristically recovered from the PDF text layer, so it is \
            approximate. Returns {has_lcir, page_count, outline:[{kind, section_number, level, \
            text, page}], counts, abstract}. If has_lcir is false the PDF has no built LCIR (enable \
            and build it in the app) — fall back to get_fulltext. Then use get_document_blocks to \
            read the structured text or equations, and search_document_nodes to locate content.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key (as in \\cite{}); alternative to entry_id." }
            }
        }
    }));
    tools.push(json!({
        "name": "get_document_blocks",
        "description": "Read a paper's content as structure-tagged blocks (LCIR) in reading order — \
            paragraphs, headings, captions and display equations — by entry_id or citation_key. \
            Better than get_fulltext for structured reading: multi-column layout is de-interleaved \
            and each block carries its kind and page. Filter with `kinds` (e.g. [\"display_math\"] \
            to list just the equations with their labels and surface text, or [\"section\", \
            \"paragraph\"] to read prose). Math is SURFACE ONLY — a cleaned Unicode linear form, \
            NOT LaTeX/MathML, since it is recovered from the PDF — so treat equations as \
            approximate. Long documents are paginated: pass block_start (from a previous next_block) \
            or raise max_chars. Returns {has_lcir, total_blocks, returned, block_start, truncated, \
            next_block, blocks:[{index, kind, page, section_number?, equation_label?, text}]}.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Restrict to these block kinds (e.g. [\"display_math\"], [\"section\",\"paragraph\"]). Omit for all content blocks."
                },
                "page": { "type": "integer", "description": "Restrict to a single 1-based PDF page." },
                "block_start": { "type": "integer", "description": "0-based index into the (filtered) block list to start from, for continuing a long read (default 0)." },
                "max_chars": { "type": "integer", "description": "Max characters of block text to return this call (default 24000)." }
            }
        }
    }));
    tools.push(json!({
        "name": "search_document_nodes",
        "description": "Search the library at BLOCK granularity (paragraph / heading / caption / \
            display equation) using the LCIR node index — finer than fulltext_search, which is page \
            granularity. Each hit reports the entry, node_kind, page, a snippet, and the PDF \
            bounding box (bbox = [x, y, width, height] in PDF points, bottom-left origin) so the \
            exact block can be located/highlighted. Use this to pinpoint where a concept, term or \
            equation appears across papers. Only covers papers whose LCIR has been built. Returns \
            {count, results:[{entry_id, title, year, node_kind, page, snippet, bbox}]}. Short or \
            CJK queries fall back to substring matching.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (space-separated terms are ANDed)." },
                "collection_id": { "type": "integer", "description": "Restrict to a collection id." },
                "tag_id": { "type": "integer", "description": "Restrict to a tag id." }
            },
            "required": ["query"]
        }
    }));

    // write 系（Phase 2・ゲート有効時のみ）。`mutate` の定義を流用し、許可リスト
    // （`WRITE_TOOLS`）に絞る。delete_entry はリストに無いので公開されない。
    if write_on {
        for s in mutate::specs() {
            if WRITE_TOOLS.contains(&s.name.as_str()) {
                tools.push(json!({
                    "name": s.name,
                    "description": s.description,
                    "inputSchema": s.parameters,
                }));
            }
        }
    }

    tools
}

// ─── JSON-RPC ディスパッチ（トランスポート非依存） ──────────────────────────

/// JSON-RPC リクエスト 1 件を処理する。通知（`id` 無し）の場合は `response: None`。
/// `mutated` が true なら write が成功したので、呼び出し側が `.bib` 同期 / UI イベントを発火する。
///
/// write の可否は `mcp_server.write_enabled` 設定から評価する（公開サーバー用ゲート）。
pub async fn handle_rpc(pool: &SqlitePool, app_data_dir: &Path, req: &Value) -> RpcOutcome {
    let write_on = write_enabled(pool).await;
    handle_rpc_with_write(pool, app_data_dir, write_on, req).await
}

/// `handle_rpc` の write_on 明示版。CLI の**直接 DB 書込経路**は、公開サーバー用の
/// `mcp_server.write_enabled` 設定とは独立に（CLI 側でサーバー到達性ゲートを済ませた上で）
/// `write_on = true` を渡して同じツール実装・監査ログ・`mutated` フラグを再利用する。
pub async fn handle_rpc_with_write(
    pool: &SqlitePool,
    app_data_dir: &Path,
    write_on: bool,
    req: &Value,
) -> RpcOutcome {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // 通知（id 無し）には応答しない（JSON-RPC 2.0）。
    let Some(id) = req.get("id").cloned() else {
        return RpcOutcome { response: None, mutated: false };
    };
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    let (resp, mutated) = match method {
        "initialize" => (
            ok(id, json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "LumenCite", "version": env!("CARGO_PKG_VERSION") }
            })),
            false,
        ),
        "ping" => (ok(id, json!({})), false),
        "tools/list" => (ok(id, json!({ "tools": tool_specs(write_on) })), false),
        "tools/call" => handle_tools_call(pool, app_data_dir, write_on, id, &params).await,
        other => (err(id, -32601, &format!("method not found: {other}")), false),
    };
    RpcOutcome { response: Some(resp), mutated }
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// `(応答, mutated)` を返す。`mutated` は write が成功した場合のみ true。
async fn handle_tools_call(
    pool: &SqlitePool,
    app_data_dir: &Path,
    write_on: bool,
    id: Value,
    params: &Value,
) -> (Value, bool) {
    let Some(name) = params.get("name").and_then(|n| n.as_str()) else {
        return (err(id, -32602, "missing tool name"), false);
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let is_write = WRITE_TOOLS.contains(&name);
    // write ツールだがゲートが無効 → 実行せず isError で拒否。
    if is_write && !write_on {
        return (
            ok(id, tool_content(format!("write tools are disabled on this MCP server: {name}"), true)),
            false,
        );
    }

    let result = exec_tool(pool, app_data_dir, write_on, name, args.clone()).await;

    // write は成否に関わらず監査ログに記録する（read は記録しない）。
    if is_write {
        let (summary, is_err) = match &result {
            Ok(s) => (s.clone(), false),
            Err(e) => (e.to_string(), true),
        };
        let args_str = serde_json::to_string(&args).unwrap_or_default();
        let _ = crate::db::mcp_audit::record(pool, name, &args_str, &summary, is_err).await;
    }

    match result {
        Ok(text) => (ok(id, tool_content(text, false)), is_write),
        // ツール実行エラーは JSON-RPC エラーではなく isError 結果として返す（MCP 慣例）。
        Err(ToolError::UnknownTool(_)) => (
            ok(id, tool_content(format!("unknown or unavailable tool: {name}"), true)),
            false,
        ),
        Err(e) => (ok(id, tool_content(e.to_string(), true)), false),
    }
}

fn tool_content(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

// ─── ツール実行 ──────────────────────────────────────────────────────────────

fn mcp_ctx<'a>(pool: &'a SqlitePool, app_data_dir: &'a Path) -> ToolContext<'a> {
    // MCP サーバーは scope を持たないため "all" 固定。外部 mcp_* ツールも使わない。
    ToolContext {
        pool,
        session_id: 0,
        scope_mode: "all",
        scope_entry_ids: &[],
        mcp: None,
        app_data_dir,
    }
}

async fn exec_tool(
    pool: &SqlitePool,
    app_data_dir: &Path,
    write_on: bool,
    name: &str,
    args: Value,
) -> Result<String, ToolError> {
    // 既存チャットの read 系をそのまま流用。
    if SHARED_READ_TOOLS.contains(&name) {
        let call = ToolCallSpec {
            call_id: "mcp-server".to_string(),
            tool_name: name.to_string(),
            arguments: args,
        };
        return search::try_execute(&mcp_ctx(pool, app_data_dir), &call)
            .await
            .unwrap_or_else(|| Err(ToolError::UnknownTool(name.to_string())));
    }

    // write 系（ゲートは呼び出し側で確認済みだが、二重に write_on を確認する）。
    if write_on && WRITE_TOOLS.contains(&name) {
        let call = ToolCallSpec {
            call_id: "mcp-server".to_string(),
            tool_name: name.to_string(),
            arguments: args,
        };
        return mutate::try_execute(&mcp_ctx(pool, app_data_dir), &call)
            .await
            .unwrap_or_else(|| Err(ToolError::UnknownTool(name.to_string())));
    }

    match name {
        "search_entries" => exec_search_entries(pool, &args).await,
        "resolve_citation_key" => exec_resolve_citation_key(pool, &args).await,
        "export_bibtex" => exec_export_bibtex(pool, &args).await,
        "find_entries_by_citation_keys" => exec_find_entries_by_citation_keys(pool, &args).await,
        "get_fulltext" => exec_get_fulltext(pool, &args).await,
        "get_document_structure" => exec_get_document_structure(pool, &args).await,
        "get_document_blocks" => exec_get_document_blocks(pool, &args).await,
        "search_document_nodes" => exec_search_document_nodes(pool, &args).await,
        // それ以外（delete_entry / ocr_* / 無効化中の write 等）は非公開。
        _ => Err(ToolError::UnknownTool(name.to_string())),
    }
}

async fn exec_search_entries(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArguments("missing required argument: query".to_string()))?;
    let collection_id = args.get("collection_id").and_then(|v| v.as_i64());
    let tag_id = args.get("tag_id").and_then(|v| v.as_i64());

    let results = crate::db::entries::search_entries(pool, query, collection_id, tag_id).await?;
    let items: Vec<Value> = results
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "title": e.title,
                "year": e.year,
                "entry_type": e.entry_type,
                "authors": e.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();

    Ok(serde_json::to_string(&json!({ "count": items.len(), "results": items })).unwrap_or_default())
}

async fn exec_resolve_citation_key(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = args
        .get("entry_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            ToolError::InvalidArguments("missing required argument: entry_id".to_string())
        })?;
    crate::bibtex::resolve_citation_key(pool, entry_id)
        .await
        .map_err(ToolError::Execution)
}

async fn exec_export_bibtex(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    // citation_keys が渡されたら「\cite キー → refs.bib」経路。全ライブラリの確定キーを
    // 維持し（サブセット再 dedup をしない）、未解決キーは `missing` に載せて返す。
    if let Some(arr) = args.get("citation_keys").and_then(|v| v.as_array()) {
        let keys: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let res = crate::bibtex::export_bibtex_by_keys(pool, &keys)
            .await
            .map_err(ToolError::Execution)?;
        return Ok(serde_json::to_string(&json!({
            "bibtex": res.bibtex,
            "found": res.found,
            "missing": res.missing,
        }))
        .unwrap_or_default());
    }

    let entry_ids = args
        .get("entry_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>());
    crate::bibtex::export_bibtex(pool, entry_ids)
        .await
        .map_err(ToolError::Execution)
}

async fn exec_find_entries_by_citation_keys(
    pool: &SqlitePool,
    args: &Value,
) -> Result<String, ToolError> {
    let keys: Vec<String> = args
        .get("citation_keys")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .ok_or_else(|| {
            ToolError::InvalidArguments(
                "missing required argument: citation_keys (array of strings)".to_string(),
            )
        })?;

    let index = crate::bibtex::citation_key_index(pool)
        .await
        .map_err(ToolError::Execution)?;
    let key_to_id: std::collections::HashMap<&str, i64> =
        index.iter().map(|(k, id)| (k.as_str(), *id)).collect();

    let mut results = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for k in &keys {
        if !seen.insert(k.as_str()) {
            continue;
        }
        match key_to_id.get(k.as_str()) {
            Some(&id) => {
                let d = crate::db::entries::get_entry(pool, id).await?;
                results.push(json!({
                    "citation_key": k,
                    "found": true,
                    "entry_id": id,
                    "title": d.title,
                    "year": d.year,
                    "authors": d.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                }));
            }
            None => results.push(json!({ "citation_key": k, "found": false })),
        }
    }

    Ok(serde_json::to_string(&json!({ "count": results.len(), "results": results }))
        .unwrap_or_default())
}

async fn exec_get_fulltext(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    // entry_id 優先。無ければ citation_key から逆引き。
    let entry_id = match args.get("entry_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => match args.get("citation_key").and_then(|v| v.as_str()) {
            Some(key) => match crate::bibtex::find_entry_id_by_citation_key(pool, key).await {
                Ok(Some(id)) => id,
                Ok(None) => {
                    return Ok(serde_json::to_string(&json!({
                        "indexed": false,
                        "message": format!("no entry found for citation key '{key}'")
                    }))
                    .unwrap_or_default())
                }
                Err(e) => return Err(ToolError::Execution(e)),
            },
            None => {
                return Err(ToolError::InvalidArguments(
                    "provide entry_id (integer) or citation_key (string)".to_string(),
                ))
            }
        },
    };

    let pages = crate::db::fulltext::get_entry_fulltext(pool, entry_id).await?;
    if pages.is_empty() {
        return Ok(serde_json::to_string(&json!({
            "entry_id": entry_id,
            "indexed": false,
            "message": "this entry has no indexed full text (no attached/indexed PDF)"
        }))
        .unwrap_or_default());
    }

    let total_pages = pages.len() as i64;
    let page_start = args.get("page_start").and_then(|v| v.as_i64()).unwrap_or(1).max(1);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_i64())
        .unwrap_or(24_000)
        .clamp(1_000, 200_000) as usize;

    // page_start 以降のページを、累計が max_chars に達するまでページ単位で連結する
    // （ページ途中では切らない）。入りきらなかった最初のページを next_page に載せて
    // 続き読みできるようにする。
    let mut text = String::new();
    let mut truncated = false;
    let mut next_page: Option<i64> = None;
    for (page, content) in pages.iter().filter(|(p, _)| *p >= page_start) {
        if text.chars().count() >= max_chars {
            next_page = Some(*page);
            truncated = true;
            break;
        }
        text.push_str(&format!("[page {page}]\n{content}\n\n"));
    }

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "indexed": true,
        "total_pages": total_pages,
        "returned_from_page": page_start,
        "truncated": truncated,
        "next_page": next_page,
        "text": text.trim_end(),
    }))
    .unwrap_or_default())
}

// ─── LCIR（機械可読中間形式）read ツール（Phase 3.5） ────────────────────────

/// entry_id 優先・無ければ citation_key から逆引き（get_fulltext と同じ規約）。
async fn resolve_entry_id(pool: &SqlitePool, args: &Value) -> Result<i64, ToolError> {
    if let Some(id) = args.get("entry_id").and_then(|v| v.as_i64()) {
        return Ok(id);
    }
    if let Some(key) = args.get("citation_key").and_then(|v| v.as_str()) {
        return match crate::bibtex::find_entry_id_by_citation_key(pool, key).await {
            Ok(Some(id)) => Ok(id),
            Ok(None) => Err(ToolError::InvalidArguments(format!(
                "no entry found for citation key '{key}'"
            ))),
            Err(e) => Err(ToolError::Execution(e)),
        };
    }
    Err(ToolError::InvalidArguments(
        "provide entry_id (integer) or citation_key (string)".to_string(),
    ))
}

/// エントリの、LCIR が構築済みの最初の PDF 添付を `(attachment_id, LcirDocument)` で読む。
async fn load_entry_lcir(
    pool: &SqlitePool,
    entry_id: i64,
) -> Result<Option<(i64, crate::document_ir::LcirDocument)>, ToolError> {
    let att_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT a.id FROM attachments a
         WHERE a.entry_id = ?
           AND EXISTS (
               SELECT 1 FROM document_versions dv
               WHERE dv.attachment_id = a.id
                 AND dv.extraction_status IN ('completed', 'completed_with_warnings')
           )
         ORDER BY a.id",
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?;
    for att in att_ids {
        if let Some(doc) = crate::ingestion::load_lcir_document(pool, att)
            .await
            .map_err(ToolError::Execution)?
        {
            return Ok(Some((att, doc)));
        }
    }
    Ok(None)
}

/// 本文つき論理ブロック（骨格の document/page/line は除く）。
fn is_content_block(kind: &str) -> bool {
    !matches!(kind, "document" | "page" | "line")
}

/// ノードの代表ページ（最初の source_fragment）。
fn node_page(n: &crate::document_ir::LcirNode) -> Option<i64> {
    n.source_fragments.first().map(|f| f.page)
}

fn no_lcir_response(entry_id: i64) -> String {
    serde_json::to_string(&json!({
        "entry_id": entry_id,
        "has_lcir": false,
        "message": "no built LCIR for this entry's PDF (enable and build LCIR in the app, \
            or the entry has no attached PDF). Fall back to get_fulltext for flat page text."
    }))
    .unwrap_or_default()
}

async fn exec_get_document_structure(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let (attachment_id, doc) = match load_entry_lcir(pool, entry_id).await? {
        Some(x) => x,
        None => return Ok(no_lcir_response(entry_id)),
    };

    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut outline: Vec<Value> = Vec::new();
    let mut abstract_parts: Vec<String> = Vec::new();
    let mut page_count = 0i64;
    for n in &doc.nodes {
        if n.kind == "page" {
            page_count += 1;
        }
        if !is_content_block(&n.kind) {
            continue;
        }
        *counts.entry(n.kind.clone()).or_insert(0) += 1;
        match n.kind.as_str() {
            "section" | "subsection" | "heading" => {
                let sec = n
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("section_number"))
                    .and_then(|v| v.as_str());
                let level = n
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("heading_level"))
                    .and_then(|v| v.as_i64());
                outline.push(json!({
                    "kind": n.kind,
                    "section_number": sec,
                    "level": level,
                    "text": n.plain_text,
                    "page": node_page(n),
                }));
            }
            "abstract" => {
                if let Some(t) = &n.plain_text {
                    abstract_parts.push(t.clone());
                }
            }
            _ => {}
        }
    }
    let abstract_text = if abstract_parts.is_empty() {
        None
    } else {
        Some(abstract_parts.join(" "))
    };

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "attachment_id": attachment_id,
        "has_lcir": true,
        "extractor_version": doc.source.extractor_version,
        "page_count": page_count,
        "outline": outline,
        "counts": counts,
        "abstract": abstract_text,
        "note": "Structure is heuristically recovered from the PDF text layer (origin=layout_model, \
            per-node confidence). Equations are surface-only (no LaTeX). Use get_document_blocks to \
            read prose or equations, search_document_nodes to locate content.",
    }))
    .unwrap_or_default())
}

async fn exec_get_document_blocks(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let (attachment_id, doc) = match load_entry_lcir(pool, entry_id).await? {
        Some(x) => x,
        None => return Ok(no_lcir_response(entry_id)),
    };

    // kinds / page フィルタ。
    let kind_filter: Option<Vec<String>> = args.get("kinds").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    });
    let page_filter = args.get("page").and_then(|v| v.as_i64());

    // 読み順の本文ブロック（load_lcir_document のノード順 = ページ→ordinal）。
    let blocks: Vec<&crate::document_ir::LcirNode> = doc
        .nodes
        .iter()
        .filter(|n| is_content_block(&n.kind))
        .filter(|n| {
            kind_filter
                .as_ref()
                .map(|ks| ks.iter().any(|k| k == &n.kind))
                .unwrap_or(true)
        })
        .filter(|n| page_filter.is_none_or(|p| node_page(n) == Some(p)))
        .collect();

    let total_blocks = blocks.len() as i64;
    let block_start = args.get("block_start").and_then(|v| v.as_i64()).unwrap_or(0).max(0);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_i64())
        .unwrap_or(24_000)
        .clamp(1_000, 200_000) as usize;

    let mut out: Vec<Value> = Vec::new();
    let mut chars = 0usize;
    let mut truncated = false;
    let mut next_block: Option<i64> = None;
    for (i, n) in blocks.iter().enumerate().skip(block_start as usize) {
        let text = n.plain_text.clone().unwrap_or_default();
        // 1 ブロックでも返した上で上限超過なら、そこで切って続きを next_block に載せる。
        if chars + text.chars().count() > max_chars && !out.is_empty() {
            next_block = Some(i as i64);
            truncated = true;
            break;
        }
        chars += text.chars().count();
        let equation_label = n.math.as_ref().and_then(|m| m.equation_label.clone());
        let section_number = n
            .payload
            .as_ref()
            .and_then(|p| p.get("section_number"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        out.push(json!({
            "index": i,
            "kind": n.kind,
            "page": node_page(n),
            "section_number": section_number,
            "equation_label": equation_label,
            "text": text,
        }));
    }

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "attachment_id": attachment_id,
        "has_lcir": true,
        "total_blocks": total_blocks,
        "block_start": block_start,
        "returned": out.len(),
        "truncated": truncated,
        "next_block": next_block,
        "blocks": out,
    }))
    .unwrap_or_default())
}

async fn exec_search_document_nodes(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArguments("missing required argument: query".to_string()))?;
    let collection_id = args.get("collection_id").and_then(|v| v.as_i64());
    let tag_id = args.get("tag_id").and_then(|v| v.as_i64());

    let hits = crate::db::document_nodes_fts::search_nodes(pool, query, collection_id, tag_id, None)
        .await?;
    let results: Vec<Value> = hits
        .iter()
        .map(|h| {
            json!({
                "entry_id": h.entry.id,
                "title": h.entry.title,
                "year": h.entry.year,
                "node_kind": h.node_kind,
                "page": h.page,
                "snippet": h.snippet,
                "bbox": h.bbox.as_ref().map(|b| json!([b.x, b.y, b.width, b.height])),
            })
        })
        .collect();

    Ok(serde_json::to_string(&json!({ "count": results.len(), "results": results }))
        .unwrap_or_default())
}

// ─── 認可トークン ────────────────────────────────────────────────────────────

/// SQLite の `randomblob` で 48 hex 文字（24 バイト）のトークンを生成する。
/// OS の乱数で seed される SQLite PRNG を使うため、追加の乱数クレートは不要。
pub async fn generate_token(pool: &SqlitePool) -> Result<String, String> {
    sqlx::query_scalar("SELECT lower(hex(randomblob(24)))")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

/// キーチェーンの token を取得。無ければ生成・保存して返す。
pub async fn get_or_create_token(pool: &SqlitePool) -> Result<String, String> {
    let account = crate::keychain::account_for_mcp_token();
    if let Some(t) = crate::keychain::get(&account).map_err(|e| e.to_string())? {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let token = generate_token(pool).await?;
    crate::keychain::set(&account, &token).map_err(|e| e.to_string())?;
    Ok(token)
}

// ─── HTTP トランスポート & ライフサイクル ────────────────────────────────────

/// サーバースレッドが書き込み後の副作用（`.bib` 同期キック・UI イベント）に使う依存。
/// `handle_rpc` 自体には渡さず HTTP 層だけが保持するので、ディスパッチは単体テスト可能。
#[derive(Clone)]
pub struct ServerDeps {
    pub pool: SqlitePool,
    pub app_data_dir: PathBuf,
    pub sync_tx: UnboundedSender<()>,
    /// UI ライブ反映イベント発火用。テストでは `None`、本番は `Some(app.handle())`。
    pub app: Option<tauri::AppHandle>,
}

/// 起動中サーバーの内部ハンドル。
struct RunningServer {
    stop: Arc<AtomicBool>,
    port: u16,
    join: Option<std::thread::JoinHandle<()>>,
}

/// MCP サーバーの起動/停止を管理する。AppState に `Arc` で保持する。
#[derive(Default)]
pub struct McpServerManager {
    inner: Mutex<Option<RunningServer>>,
}

impl McpServerManager {
    /// localhost にバインドしてサーバースレッドを起動する。既存が動いていれば先に停止。
    /// 実際にバインドできたポートを返す（`port=0` で OS 割り当ても可）。
    pub fn start(&self, deps: ServerDeps, port: u16, token: String) -> Result<u16, String> {
        self.stop();

        let addr = format!("127.0.0.1:{port}");
        let server = tiny_http::Server::http(&addr).map_err(|e| format!("bind {addr} failed: {e}"))?;
        let bound_port = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .unwrap_or(port);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let join = std::thread::spawn(move || {
            serve_loop(server, stop_thread, deps, token);
        });

        *self.inner.lock().unwrap() = Some(RunningServer {
            stop,
            port: bound_port,
            join: Some(join),
        });
        Ok(bound_port)
    }

    /// 起動中なら停止してスレッドを join する。未起動なら no-op。
    pub fn stop(&self) {
        if let Some(mut running) = self.inner.lock().unwrap().take() {
            running.stop.store(true, Ordering::SeqCst);
            if let Some(j) = running.join.take() {
                let _ = j.join();
            }
        }
    }

    /// 起動中なら実際のバインドポート、未起動なら None。
    pub fn running_port(&self) -> Option<u16> {
        self.inner.lock().unwrap().as_ref().map(|r| r.port)
    }
}

/// 同時に処理するリクエストの上限（CR-023）。
/// clip はメタデータ取得で最大 ~30s ブロックし得るため、直列だと 1 件で全 traffic が
/// 止まる。ワーカースレッドへ分散しつつ、暴走クライアントによるスレッド無制限生成は防ぐ。
const MAX_CONCURRENT_REQUESTS: usize = 8;

/// 単純なカウンティングセマフォ（`tiny_http` は std スレッドで回るため tokio のものは使わない）。
/// 容量待ちの間も `stop` を監視し、停止時は待ちを解いて `None` を返す。
struct Semaphore {
    state: Mutex<usize>,
    cv: std::sync::Condvar,
}

/// 取得した permit。drop で 1 枠解放する。
struct Permit(Arc<Semaphore>);

impl Semaphore {
    fn new(max: usize) -> Arc<Self> {
        Arc::new(Semaphore {
            state: Mutex::new(max),
            cv: std::sync::Condvar::new(),
        })
    }

    /// 1 枠確保する。容量が空くまで待つが、`stop` が立ったら `None` を返す。
    fn acquire(self: &Arc<Self>, stop: &AtomicBool) -> Option<Permit> {
        let mut avail = self.state.lock().unwrap();
        while *avail == 0 {
            if stop.load(Ordering::SeqCst) {
                return None;
            }
            let (guard, _) = self
                .cv
                .wait_timeout(avail, Duration::from_millis(300))
                .unwrap();
            avail = guard;
        }
        *avail -= 1;
        Some(Permit(self.clone()))
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut avail = self.0.state.lock().unwrap();
        *avail += 1;
        self.0.cv.notify_one();
    }
}

fn serve_loop(server: tiny_http::Server, stop: Arc<AtomicBool>, deps: ServerDeps, token: String) {
    // 同時処理数を上限付きで並列化する（CR-023: 1 件の遅い clip が全 traffic を止めない）。
    let sem = Semaphore::new(MAX_CONCURRENT_REQUESTS);
    // recv_timeout で定期的に stop フラグを確認しつつ accept する。
    while !stop.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(300)) {
            Ok(Some(req)) => {
                // 容量待ち。stop が立てば None → ループ終了。
                let Some(permit) = sem.acquire(&stop) else {
                    break;
                };
                let deps = deps.clone();
                let token = token.clone();
                std::thread::spawn(move || {
                    let _permit = permit; // drop で枠を解放する
                    handle_http_request(req, &deps, &token);
                });
            }
            Ok(None) => continue, // タイムアウト → ループ先頭で stop を再確認
            Err(_) => break,
        }
    }
}

/// リクエストボディの上限。JSON-RPC には十分大きく、暴走クライアントによる
/// 無制限読み込みは防ぐ。
const MAX_BODY_BYTES: u64 = 10 * 1024 * 1024;

/// 定数時間の文字列比較（トークン照合用。早期 return によるタイミング差を避ける）。
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// リクエストの行き先。`route()` は pure なので単体テストできる。
#[derive(Debug, PartialEq)]
enum Route {
    /// `OPTIONS /clipper` — CORS preflight（**認証不要**: preflight は Authorization を持たない）
    ClipperPreflight,
    /// `GET /clipper` — ペアリング疎通確認（認証必須）
    ClipperPing,
    /// `POST /clipper` — クリップ本体（認証必須 + `clipper.enabled`）
    Clip,
    /// `POST <その他>` — 既存の JSON-RPC（`/mcp` ほかパス不問。後方互換）
    Rpc,
    /// それ以外のメソッド → 405
    MethodNotAllowed,
}

fn route(method: &tiny_http::Method, path: &str) -> Route {
    use tiny_http::Method;
    // クエリ・末尾スラッシュを無視してパスを正規化する
    let path = path.split('?').next().unwrap_or(path);
    let path = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    let is_clipper = path == "/clipper";
    match (method, is_clipper) {
        (Method::Options, true) => Route::ClipperPreflight,
        (Method::Get, true) => Route::ClipperPing,
        (Method::Post, true) => Route::Clip,
        (Method::Post, false) => Route::Rpc,
        _ => Route::MethodNotAllowed,
    }
}

/// `Origin` が Chrome 拡張のときだけ返す CORS ヘッダ群。
/// Web ページ由来の Origin（https:// 等）には返さない。
fn cors_headers(origin: Option<&str>) -> Vec<tiny_http::Header> {
    let Some(origin) = origin.filter(|o| o.starts_with("chrome-extension://")) else {
        return vec![];
    };
    let h = |k: &[u8], v: &[u8]| tiny_http::Header::from_bytes(k, v).expect("static header");
    vec![
        h(b"Access-Control-Allow-Origin", origin.as_bytes()),
        h(b"Access-Control-Allow-Methods", b"GET, POST, OPTIONS"),
        h(b"Access-Control-Allow-Headers", b"Authorization, Content-Type"),
        h(b"Access-Control-Max-Age", b"600"),
    ]
}

fn handle_http_request(mut req: tiny_http::Request, deps: &ServerDeps, token: &str) {
    use tauri::Emitter;
    use tiny_http::{Header, Response};

    let json_ct = || {
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("static header")
    };
    let origin: Option<String> = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string());
    let cors = cors_headers(origin.as_deref());
    let with_cors = |mut resp: Response<std::io::Cursor<Vec<u8>>>| {
        for h in &cors {
            resp = resp.with_header(h.clone());
        }
        resp
    };

    let routed = route(req.method(), req.url());

    // preflight は Authorization ヘッダを持たないため、認証より先に処理する
    if routed == Route::ClipperPreflight {
        let _ = req.respond(with_cors(Response::from_string("").with_status_code(204)));
        return;
    }

    // 認可: Authorization: Bearer <token>
    let authorized = req.headers().iter().any(|h| {
        h.field.equiv("Authorization")
            && h.value
                .as_str()
                .strip_prefix("Bearer ")
                .map(|t| constant_time_eq(t, token))
                .unwrap_or(false)
    });
    if !authorized {
        let _ = req.respond(with_cors(
            Response::from_string("unauthorized").with_status_code(401),
        ));
        return;
    }

    match routed {
        Route::ClipperPreflight => unreachable!("handled before auth"),
        Route::MethodNotAllowed => {
            let _ = req.respond(Response::from_string("method not allowed").with_status_code(405));
        }
        Route::ClipperPing => {
            let body = json!({ "ok": true, "app": "LumenCite", "version": env!("CARGO_PKG_VERSION") });
            let _ = req.respond(with_cors(
                Response::from_string(body.to_string()).with_header(json_ct()),
            ));
        }
        Route::Clip => {
            if !tauri::async_runtime::block_on(clipper::clipper_enabled(&deps.pool)) {
                let body = json!({ "status": "error", "code": "clipper_disabled" });
                let _ = req.respond(with_cors(
                    Response::from_string(body.to_string())
                        .with_status_code(403)
                        .with_header(json_ct()),
                ));
                return;
            }
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err((code, msg)) => {
                    let _ = req.respond(with_cors(
                        Response::from_string(msg).with_status_code(code),
                    ));
                    return;
                }
            };
            let clip_req: clipper::ClipRequest = match serde_json::from_str(&body) {
                Ok(r) => r,
                Err(e) => {
                    let body =
                        json!({ "status": "error", "code": "bad_request", "message": e.to_string() });
                    let _ = req.respond(with_cors(
                        Response::from_string(body.to_string())
                            .with_status_code(400)
                            .with_header(json_ct()),
                    ));
                    return;
                }
            };
            // handle_clip は外部 API（CrossRef / arXiv / OpenLibrary）へ reqwest で
            // アクセスする。serve_loop スレッド上の block_on では reqwest の I/O が
            // 進まず必ずタイムアウトする（E2E で発覚）ため、ランタイムのワーカーへ
            // spawn して結果をチャネルで待つ。
            let outcome = {
                let pool = deps.pool.clone();
                run_on_runtime(async move { clipper::handle_clip(&pool, &clip_req).await })
            };
            if outcome.mutated {
                let _ = deps.sync_tx.send(());
                if let Some(app) = &deps.app {
                    let _ = app.emit("entries-changed", ());
                }
            }
            let pdf_job = outcome.pdf_job.clone();
            let _ = req.respond(with_cors(
                Response::from_string(outcome.response.to_string())
                    .with_status_code(outcome.status)
                    .with_header(json_ct()),
            ));
            // PDF ダウンロードは応答後に非同期実行（serve loop を塞がない）
            if let Some(job) = pdf_job {
                spawn_pdf_job(deps, job);
            }
        }
        Route::Rpc => {
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err((code, msg)) => {
                    let _ = req.respond(Response::from_string(msg).with_status_code(code));
                    return;
                }
            };
            let outcome: RpcOutcome = match serde_json::from_str::<Value>(&body) {
                Ok(v) => {
                    tauri::async_runtime::block_on(handle_rpc(&deps.pool, &deps.app_data_dir, &v))
                }
                Err(e) => RpcOutcome {
                    response: Some(json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": { "code": -32700, "message": format!("parse error: {e}") }
                    })),
                    mutated: false,
                },
            };

            // write 成功の副作用: `.bib` 自動同期キック + 一覧へのライブ反映イベント。
            if outcome.mutated {
                let _ = deps.sync_tx.send(());
                if let Some(app) = &deps.app {
                    let _ = app.emit("entries-changed", ());
                }
            }

            match outcome.response {
                Some(v) => {
                    let _ = req
                        .respond(Response::from_string(v.to_string()).with_header(json_ct()));
                }
                // 通知のみ（応答不要）→ 202 Accepted
                None => {
                    let _ = req.respond(Response::from_string("").with_status_code(202));
                }
            }
        }
    }
}

/// serve_loop スレッドから、非同期ランタイムの**ワーカー上で** future を実行して
/// 完了を待つ。`tauri::async_runtime::block_on` はこのスレッド上で future を駆動する
/// ため、reqwest のようなネットワーク I/O を含む future が進行しない。
/// DB のみの future（sqlx）は block_on で問題ないが、外部 HTTP を含むものは必ず
/// こちらを使うこと。
fn run_on_runtime<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.recv().expect("runtime task dropped without sending a result")
}

/// ボディを上限付きで読む（Content-Length は詐称できるため実読で判定）。
fn read_body(req: &mut tiny_http::Request) -> Result<String, (u16, &'static str)> {
    use std::io::Read;
    let mut body = String::new();
    let mut limited = std::io::Read::take(req.as_reader(), MAX_BODY_BYTES + 1);
    if limited.read_to_string(&mut body).is_err() {
        return Err((400, "bad request body"));
    }
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err((413, "payload too large"));
    }
    Ok(body)
}

/// PDF ダウンロードジョブを応答後に非同期実行する。成功したら `entries-changed` で
/// UI に反映する（添付は .bib の内容に影響しないため sync はキックしない）。
/// 失敗（ペイウォール・サイズ超過等）はログのみ — エントリ作成は既に成功している。
fn spawn_pdf_job(deps: &ServerDeps, job: clipper::PdfJob) {
    let pool = deps.pool.clone();
    let app_data_dir = deps.app_data_dir.clone();
    let app = deps.app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        match crate::download::download_and_attach(
            &pool,
            &app_data_dir,
            job.entry_id,
            &job.url,
            crate::download::DownloadCaps::default(),
        )
        .await
        {
            Ok(att) => {
                eprintln!("clipper: attached {} to entry {}", att.file_name, job.entry_id);
                // 添付後に全文索引する（クリッパー経路も自動索引・CR-027）。
                let abs = app_data_dir
                    .join("attachments")
                    .join(job.entry_id.to_string())
                    .join(&att.file_name);
                crate::db::fulltext::extract_and_index(&pool, abs, att.id).await;
                if let Some(app) = &app {
                    let _ = app.emit("entries-changed", ());
                }
            }
            Err(e) => {
                eprintln!("clipper: PDF download failed for entry {}: {e}", job.entry_id);
            }
        }
    });
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    fn req(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    /// CR-023: セマフォは上限まで permit を出し、超過は解放待ちにする。
    /// drop で枠が戻り、stop が立てば待ちを解いて `None` を返す。
    #[test]
    fn semaphore_bounds_concurrency_and_respects_stop() {
        let sem = Semaphore::new(2);
        let stop = AtomicBool::new(false);
        let p1 = sem.acquire(&stop).expect("1st permit");
        let _p2 = sem.acquire(&stop).expect("2nd permit");
        // 満杯: stop を立てると 3 つ目は待たずに None。
        stop.store(true, Ordering::SeqCst);
        assert!(sem.acquire(&stop).is_none(), "満杯 + stop で None");
        // 1 枠戻せば（stop 中でも空きがあるので）取得できる。
        drop(p1);
        assert!(sem.acquire(&stop).is_some(), "解放後は取得できる");
    }

    async fn call_tool(pool: &SqlitePool, name: &str, args: Value) -> Value {
        let r = req("tools/call", json!({ "name": name, "arguments": args }));
        handle_rpc(pool, Path::new(""), &r).await.response.unwrap()
    }

    async fn enable_writes(pool: &SqlitePool) {
        crate::db::settings::set_setting(
            pool,
            crate::db::settings::MCP_SERVER_WRITE_ENABLED_KEY,
            "1",
        )
        .await
        .unwrap();
    }

    #[test]
    fn route_dispatch_table() {
        use tiny_http::Method;
        assert_eq!(route(&Method::Options, "/clipper"), Route::ClipperPreflight);
        assert_eq!(route(&Method::Get, "/clipper"), Route::ClipperPing);
        assert_eq!(route(&Method::Post, "/clipper"), Route::Clip);
        // 末尾スラッシュ・クエリは無視する
        assert_eq!(route(&Method::Post, "/clipper/"), Route::Clip);
        assert_eq!(route(&Method::Get, "/clipper?x=1"), Route::ClipperPing);
        // 既存 JSON-RPC: POST は任意パスで従来どおり
        assert_eq!(route(&Method::Post, "/mcp"), Route::Rpc);
        assert_eq!(route(&Method::Post, "/"), Route::Rpc);
        // その他メソッドは 405（従来挙動の維持）
        assert_eq!(route(&Method::Get, "/mcp"), Route::MethodNotAllowed);
        assert_eq!(route(&Method::Options, "/mcp"), Route::MethodNotAllowed);
        assert_eq!(route(&Method::Delete, "/clipper"), Route::MethodNotAllowed);
    }

    #[test]
    fn cors_headers_only_for_chrome_extension_origin() {
        assert!(cors_headers(None).is_empty());
        assert!(cors_headers(Some("https://evil.example")).is_empty());
        let hs = cors_headers(Some("chrome-extension://abcdef"));
        assert!(hs.iter().any(|h| {
            h.field.equiv("Access-Control-Allow-Origin")
                && h.value.as_str() == "chrome-extension://abcdef"
        }));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn http_clipper_routes_auth_cors_and_gating(pool: SqlitePool) {
        let manager = McpServerManager::default();
        let token = "test-token-clipper".to_string();
        let (sync_tx, mut sync_rx) = tokio::sync::mpsc::unbounded_channel();
        let deps = ServerDeps {
            pool: pool.clone(),
            app_data_dir: PathBuf::from(""),
            sync_tx,
            app: None,
        };
        let port = manager.start(deps, 0, token.clone()).expect("server should bind");
        let url = format!("http://127.0.0.1:{port}/clipper");
        let client = reqwest::Client::new();

        // OPTIONS preflight は認証なしで 204。chrome-extension Origin にだけ CORS を返す
        let resp = client
            .request(reqwest::Method::OPTIONS, &url)
            .header("Origin", "chrome-extension://abc")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
        assert_eq!(
            resp.headers().get("access-control-allow-origin").map(|v| v.to_str().unwrap()),
            Some("chrome-extension://abc")
        );
        let resp = client
            .request(reqwest::Method::OPTIONS, &url)
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
        assert!(resp.headers().get("access-control-allow-origin").is_none());

        // 認証なしの GET/POST は 401
        assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
        assert_eq!(client.post(&url).body("{}").send().await.unwrap().status(), 401);

        // GET /clipper（ペアリング疎通確認）
        let resp = client.get(&url).bearer_auth(&token).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let ping: Value = resp.json().await.unwrap();
        assert_eq!(ping["ok"], true);
        assert_eq!(ping["app"], "LumenCite");

        // clipper.enabled 未設定 → POST は 403
        let resp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&json!({ "url": "https://example.com/a" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["code"], "clipper_disabled");

        // 有効化 → 作成成功 + sync キック
        crate::db::settings::set_setting(&pool, crate::db::settings::CLIPPER_ENABLED_KEY, "1")
            .await
            .unwrap();
        let resp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&json!({ "url": "https://example.com/a", "title": "Clipped Page" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "created");
        assert_eq!(body["title"], "Clipped Page");
        assert!(sync_rx.try_recv().is_ok(), "作成成功で .bib 同期がキックされる");

        // 既存 JSON-RPC ルートは従来どおり動く（後方互換）
        let rpc_url = format!("http://127.0.0.1:{port}/mcp");
        let resp = client
            .post(&rpc_url)
            .bearer_auth(&token)
            .json(&req("tools/list", json!({})))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        manager.stop();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn initialize_returns_protocol_and_server_info(pool: SqlitePool) {
        let resp = handle_rpc(&pool, Path::new(""), &req("initialize", json!({})))
            .await
            .response
            .unwrap();
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "LumenCite");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_list_has_read_tools_and_excludes_mutate(pool: SqlitePool) {
        // 既定（write_enabled 未設定 = false）では write 系は出ない。
        let resp = handle_rpc(&pool, Path::new(""), &req("tools/list", json!({})))
            .await
            .response
            .unwrap();
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in [
            "fulltext_search",
            "get_entry",
            "list_collections",
            "list_tags",
            "search_entries",
            "resolve_citation_key",
            "export_bibtex",
            "get_document_structure",
            "get_document_blocks",
            "search_document_nodes",
        ] {
            assert!(names.contains(&expected), "missing read tool: {expected}");
        }
        // write/mutate/ocr は公開しない。
        for forbidden in ["create_entry", "update_entry", "delete_entry", "add_tag", "ocr_pdf"] {
            assert!(!names.contains(&forbidden), "must not expose: {forbidden}");
        }
    }

    /// ツール結果 content[0].text（JSON 文字列）をパースする。
    fn tool_json(resp: &Value) -> Value {
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    /// block ノード + block fragment を 1 個作る（LCIR テスト用）。
    async fn add_block(
        pool: &SqlitePool,
        vid: i64,
        page: i64,
        kind: &str,
        ordinal: i64,
        text: &str,
        payload: Option<&str>,
    ) -> i64 {
        let id = crate::db::document_nodes::insert_node(
            pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(page),
                node_kind: kind,
                ordinal,
                plain_text: Some(text),
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: payload,
            },
        )
        .await
        .unwrap();
        crate::db::source_fragments::insert_fragment(
            pool,
            &crate::db::source_fragments::NewSourceFragment {
                node_id: id,
                page_number: 1,
                x: 72.0,
                y: 500.0,
                width: 300.0,
                height: 12.0,
                rotation: 0.0,
                reading_order: Some(ordinal),
                fragment_type: Some("block"),
            },
        )
        .await
        .unwrap();
        id
    }

    /// LCIR 構築済みエントリ（abstract/section/paragraph/display_math）を作り entry_id を返す。
    async fn setup_entry_with_lcir(pool: &SqlitePool) -> i64 {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "Quantum walk paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = crate::db::document_nodes::insert_node(
            pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("full page text"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        add_block(pool, vid, page, "abstract", 0, "We study quantum walks.", None).await;
        add_block(
            pool,
            vid,
            page,
            "section",
            1,
            "1 Introduction",
            Some(r#"{"heading_level":1,"section_number":"1"}"#),
        )
        .await;
        add_block(
            pool,
            vid,
            page,
            "paragraph",
            2,
            "Quantum walks are discrete analogues of diffusion.",
            None,
        )
        .await;
        let eq = add_block(pool, vid, page, "display_math", 3, "U = S2 C2 S1 C1 (1.1)", None).await;
        crate::db::math_expressions::insert_math(
            pool,
            &crate::db::math_expressions::NewMathExpression {
                node_id: eq,
                display_mode: "display",
                equation_label: Some("(1.1)"),
                latex: None,
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("U = S2 C2 S1 C1 (1.1)"),
                ast_json: None,
                semantic_status: "surface_only",
                confidence: Some(0.75),
                origin: Some("pdf_text_layer"),
            },
        )
        .await
        .unwrap();
        // node-FTS を張る（search_document_nodes 用）。
        crate::ingestion::regenerate_node_fts_from_lcir(pool, att)
            .await
            .unwrap();
        entry.id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_document_structure_returns_outline_counts_and_abstract(pool: SqlitePool) {
        let entry_id = setup_entry_with_lcir(&pool).await;
        let resp = call_tool(&pool, "get_document_structure", json!({ "entry_id": entry_id })).await;
        let j = tool_json(&resp);
        assert_eq!(j["has_lcir"], true);
        assert_eq!(j["counts"]["display_math"], 1);
        assert_eq!(j["counts"]["paragraph"], 1);
        assert_eq!(j["abstract"], "We study quantum walks.");
        // アウトラインに節が節番号つきで入る。
        let outline = j["outline"].as_array().unwrap();
        assert_eq!(outline.len(), 1);
        assert_eq!(outline[0]["section_number"], "1");
        assert_eq!(outline[0]["page"], 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_document_blocks_filters_by_kind_and_exposes_math(pool: SqlitePool) {
        let entry_id = setup_entry_with_lcir(&pool).await;
        // kinds=["display_math"] → 数式だけ。equation_label が付く。
        let resp = call_tool(
            &pool,
            "get_document_blocks",
            json!({ "entry_id": entry_id, "kinds": ["display_math"] }),
        )
        .await;
        let j = tool_json(&resp);
        assert_eq!(j["total_blocks"], 1);
        let blocks = j["blocks"].as_array().unwrap();
        assert_eq!(blocks[0]["kind"], "display_math");
        assert_eq!(blocks[0]["equation_label"], "(1.1)");
        assert!(blocks[0]["text"].as_str().unwrap().contains("S2 C2"));

        // フィルタ無し → 本文ブロック 4 個（document/page/line は除外）。
        let all = tool_json(&call_tool(&pool, "get_document_blocks", json!({ "entry_id": entry_id })).await);
        assert_eq!(all["total_blocks"], 4);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_document_nodes_finds_block_with_bbox(pool: SqlitePool) {
        setup_entry_with_lcir(&pool).await;
        let resp = call_tool(&pool, "search_document_nodes", json!({ "query": "quantum walks" })).await;
        let j = tool_json(&resp);
        assert!(j["count"].as_i64().unwrap() >= 1);
        let hit = &j["results"][0];
        assert!(hit["node_kind"].is_string());
        assert_eq!(hit["page"], 1);
        // bbox が [x,y,w,h] で返る（ハイライト用）。
        assert!(hit["bbox"].as_array().map(|a| a.len() == 4).unwrap_or(false));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn document_tools_report_has_lcir_false_without_lcir(pool: SqlitePool) {
        // LCIR 未構築のエントリ → has_lcir:false（get_fulltext に退避可能）。
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "No LCIR".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let j = tool_json(&call_tool(&pool, "get_document_structure", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], false);
    }

    /// 手動 E2E: 実 DB コピー + 実 PDF を pdfium で LCIR 構築し、外部 LLM が MCP で受け取る
    /// JSON（get_document_structure / get_document_blocks / search_document_nodes）を印字する。
    /// native lib が要るため `#[ignore]`。env 未設定なら skip。ATT の entry を対象にする。
    /// 例:
    /// `LCIR_SMOKE_DB=/path/copy.db LCIR_SMOKE_APPDIR="$HOME/Library/Application Support/com.lumencite.app" \
    ///  LCIR_SMOKE_ATT=8 cargo test --lib mcp_lcir_tools_e2e -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "manual pdfium E2E; needs LCIR_SMOKE_* env + libpdfium"]
    async fn mcp_lcir_tools_e2e() {
        let (db, appdir, att) = match (
            std::env::var("LCIR_SMOKE_DB"),
            std::env::var("LCIR_SMOKE_APPDIR"),
            std::env::var("LCIR_SMOKE_ATT"),
        ) {
            (Ok(d), Ok(a), Ok(t)) => (d, a, t.parse::<i64>().expect("LCIR_SMOKE_ATT must be int")),
            _ => {
                eprintln!("skip: set LCIR_SMOKE_DB / LCIR_SMOKE_APPDIR / LCIR_SMOKE_ATT");
                return;
            }
        };
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        crate::db::settings::set_setting(&pool, crate::db::settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
        // 実 PDF を LCIR 構築（既存なら reuse）。
        let build = crate::ingestion::build_lcir_for_attachment(&pool, Path::new(&appdir), att)
            .await
            .unwrap();
        eprintln!("build: built={} reused={}", build.built, build.reused);
        let entry_id: i64 = sqlx::query_scalar("SELECT entry_id FROM attachments WHERE id = ?")
            .bind(att)
            .fetch_one(&pool)
            .await
            .unwrap();

        let structure = tool_json(
            &call_tool(&pool, "get_document_structure", json!({ "entry_id": entry_id })).await,
        );
        eprintln!(
            "\n=== get_document_structure ===\n{}",
            serde_json::to_string_pretty(&structure).unwrap()
        );

        let eqs = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry_id, "kinds": ["display_math"], "max_chars": 1500 }),
            )
            .await,
        );
        eprintln!(
            "\n=== get_document_blocks kinds=[display_math] (first ~1500 chars) ===\n{}",
            serde_json::to_string_pretty(&eqs).unwrap()
        );

        let found = tool_json(
            &call_tool(&pool, "search_document_nodes", json!({ "query": "wave operator" })).await,
        );
        eprintln!(
            "\n=== search_document_nodes 'wave operator' ===\n{}",
            serde_json::to_string_pretty(&found).unwrap()
        );

        assert_eq!(structure["has_lcir"], true);
        assert!(structure["counts"]["display_math"].as_i64().unwrap_or(0) > 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_entry_includes_citation_key(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("doe2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let resp = call_tool(&pool, "get_entry", json!({ "entry_id": id })).await;
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["citation_key"], "doe2020");
        assert_eq!(parsed["resolved_citation_key"], "doe2020");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_resolve_citation_key(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("smith2021".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let resp = call_tool(&pool, "resolve_citation_key", json!({ "entry_id": id })).await;
        assert_eq!(resp["result"]["isError"], false);
        assert_eq!(resp["result"]["content"][0]["text"], "smith2021");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_export_bibtex_returns_bib(pool: SqlitePool) {
        create_entry(
            &pool,
            &EntryInput {
                title: "Exported".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("exp2022".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let resp = call_tool(&pool, "export_bibtex", json!({})).await;
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("exp2022"), "bib should contain the cite key: {text}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_entry_by_citation_key(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("doe2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        // entry_id を知らなくても cite key だけで引ける。
        let resp = call_tool(&pool, "get_entry", json!({ "citation_key": "doe2020" })).await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["id"], id);
        assert_eq!(parsed["citation_key"], "doe2020");

        // 未知キーは（エラーではなく）見つからない旨のメッセージ。
        let miss = call_tool(&pool, "get_entry", json!({ "citation_key": "nope1999" })).await;
        assert_eq!(miss["result"]["isError"], false);
        assert!(miss["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no entry found"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_find_entries_by_citation_keys(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Findable".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("wong2019".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let resp = call_tool(
            &pool,
            "find_entries_by_citation_keys",
            json!({ "citation_keys": ["wong2019", "missing2000"] }),
        )
        .await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["found"], true);
        assert_eq!(results[0]["entry_id"], id);
        assert_eq!(results[0]["title"], "Findable");
        assert_eq!(results[1]["found"], false);
        assert_eq!(results[1]["citation_key"], "missing2000");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_export_bibtex_by_citation_keys(pool: SqlitePool) {
        create_entry(
            &pool,
            &EntryInput {
                title: "Wanted".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("keep2021".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        create_entry(
            &pool,
            &EntryInput {
                title: "Unwanted".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("skip2021".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let resp = call_tool(
            &pool,
            "export_bibtex",
            json!({ "citation_keys": ["keep2021", "ghost2000"] }),
        )
        .await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(parsed["bibtex"].as_str().unwrap().contains("keep2021"));
        assert!(!parsed["bibtex"].as_str().unwrap().contains("skip2021"));
        assert_eq!(parsed["found"], json!(["keep2021"]));
        assert_eq!(parsed["missing"], json!(["ghost2000"]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_fulltext_by_key_and_missing(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Full Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("full2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            "attachments/x/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();
        crate::db::fulltext::index_attachment(
            &pool,
            att.id,
            &[
                (1, "Introduction to widgets.".to_string()),
                (2, "Widget conclusions.".to_string()),
            ],
        )
        .await
        .unwrap();

        // citation_key で全文取得できる。
        let resp = call_tool(&pool, "get_fulltext", json!({ "citation_key": "full2020" })).await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["indexed"], true);
        assert_eq!(parsed["total_pages"], 2);
        assert_eq!(parsed["truncated"], false);
        let text = parsed["text"].as_str().unwrap();
        assert!(text.contains("widgets"));
        assert!(text.contains("conclusions"));

        // PDF 未索引のエントリは indexed:false（捏造させないための明示シグナル）。
        let bare = create_entry(
            &pool,
            &EntryInput {
                title: "No PDF".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let resp2 = call_tool(&pool, "get_fulltext", json!({ "entry_id": bare.id })).await;
        let parsed2: Value =
            serde_json::from_str(resp2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed2["indexed"], false);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_fulltext_paginates(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Long".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            "attachments/y/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();
        crate::db::fulltext::index_attachment(
            &pool,
            att.id,
            &[
                (1, "a".repeat(3000)),
                (2, "b".repeat(3000)),
                (3, "c".repeat(3000)),
            ],
        )
        .await
        .unwrap();

        // max_chars を小さくすると 1 ページで打ち切り、next_page=2 が返る。
        let resp = call_tool(
            &pool,
            "get_fulltext",
            json!({ "entry_id": entry.id, "max_chars": 1000 }),
        )
        .await;
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["truncated"], true);
        assert_eq!(parsed["next_page"], 2);

        // page_start=2 で続きから読める。
        let resp2 = call_tool(
            &pool,
            "get_fulltext",
            json!({ "entry_id": entry.id, "max_chars": 1000, "page_start": 2 }),
        )
        .await;
        let parsed2: Value =
            serde_json::from_str(resp2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed2["returned_from_page"], 2);
        assert_eq!(parsed2["next_page"], 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_search_entries(pool: SqlitePool) {
        create_entry(
            &pool,
            &EntryInput {
                title: "Quantum Computing Survey".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let resp = call_tool(&pool, "search_entries", json!({ "query": "quantum" })).await;
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(parsed["count"].as_i64().unwrap() >= 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_forbidden_mutate_tool_is_error(pool: SqlitePool) {
        // write 系はサーバーに公開されておらず、呼んでも isError で弾かれる。
        let resp = call_tool(&pool, "create_entry", json!({ "title": "X" })).await;
        assert_eq!(resp["result"]["isError"], true);
        // 実際に作成されていないこと。
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_method_returns_jsonrpc_error(pool: SqlitePool) {
        let resp = handle_rpc(&pool, Path::new(""), &req("frobnicate", json!({})))
            .await
            .response
            .unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn notification_without_id_returns_none(pool: SqlitePool) {
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized", "params": {} });
        let outcome = handle_rpc(&pool, Path::new(""), &notif).await;
        assert!(outcome.response.is_none());
        assert!(!outcome.mutated);
    }

    // ── Phase 2: write gate ────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_list_includes_write_tools_when_enabled(pool: SqlitePool) {
        enable_writes(&pool).await;
        let resp = handle_rpc(&pool, Path::new(""), &req("tools/list", json!({})))
            .await
            .response
            .unwrap();
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in ["add_tag", "update_notes", "add_to_collection", "create_entry", "update_entry"] {
            assert!(names.contains(&expected), "missing write tool: {expected}");
        }
        // 破壊系は write 有効でも公開しない。
        assert!(!names.contains(&"delete_entry"), "delete_entry must never be exposed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn write_tool_blocked_when_disabled_and_no_mutation(pool: SqlitePool) {
        // 既定（無効）では create_entry は isError、mutated=false、DB 変化なし。
        let r = req("tools/call", json!({ "name": "create_entry", "arguments": { "title": "X" } }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        assert_eq!(outcome.response.unwrap()["result"]["isError"], true);
        assert!(!outcome.mutated);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries").fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn write_create_entry_when_enabled_mutates_and_audits(pool: SqlitePool) {
        enable_writes(&pool).await;
        let r = req("tools/call", json!({ "name": "create_entry", "arguments": { "title": "Made via MCP", "entry_type": "article" } }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        assert_eq!(outcome.response.unwrap()["result"]["isError"], false);
        assert!(outcome.mutated, "successful write must set mutated=true");

        // エントリが作成された。
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE title = ?")
            .bind("Made via MCP").fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);

        // 監査ログに記録された。
        let audit = crate::db::mcp_audit::recent(&pool, 10).await.unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].tool_name, "create_entry");
        assert!(!audit[0].is_error);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_never_exposed_even_when_writes_enabled(pool: SqlitePool) {
        enable_writes(&pool).await;
        let id = create_entry(
            &pool,
            &EntryInput { title: "Keep".to_string(), entry_type: "article".to_string(), ..Default::default() },
        ).await.unwrap().id;

        let r = req("tools/call", json!({ "name": "delete_entry", "arguments": { "entry_id": id } }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        // 許可リスト外 → isError、mutated=false、エントリは残る。
        assert_eq!(outcome.response.unwrap()["result"]["isError"], true);
        assert!(!outcome.mutated);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE id = ?")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn read_tool_does_not_mutate(pool: SqlitePool) {
        let r = req("tools/call", json!({ "name": "list_tags", "arguments": {} }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        assert!(!outcome.mutated);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn generate_token_is_nonempty_hex(pool: SqlitePool) {
        let token = generate_token(&pool).await.unwrap();
        assert_eq!(token.len(), 48);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// HTTP トランスポート全体の疎通: バインド → 認可 → JSON-RPC 応答。
    #[sqlx::test(migrations = "./migrations")]
    async fn http_server_serves_tools_list_with_bearer_auth(pool: SqlitePool) {
        let manager = McpServerManager::default();
        let token = "test-token-abc".to_string();
        let (sync_tx, _sync_rx) = tokio::sync::mpsc::unbounded_channel();
        let deps = ServerDeps {
            pool: pool.clone(),
            app_data_dir: PathBuf::from(""),
            sync_tx,
            app: None, // テストでは UI イベントを発火しない
        };
        // port 0 で OS 割り当て。実バインドポートが返る。
        let port = manager.start(deps, 0, token.clone()).expect("server should bind");
        let url = format!("http://127.0.0.1:{port}/mcp");
        let client = reqwest::Client::new();
        let body = req("tools/list", json!({}));

        // 認可ヘッダ無し → 401
        let resp = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(resp.status(), 401);

        // 正しい Bearer → 200 + ツール一覧
        let resp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let parsed: Value = resp.json().await.unwrap();
        let names: Vec<&str> = parsed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"export_bibtex"));

        manager.stop();
    }
}

#[cfg(test)]
mod block_on_mechanism_tests {
    use super::*;

    /// serve_loop 相当（素の std::thread 上の tauri::async_runtime::block_on）で
    /// reqwest + tokio::time::timeout が機能するかの機構テスト（ローカル fixture）。
    #[test]
    fn reqwest_inside_block_on_on_foreign_thread_works() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let _ = req.respond(tiny_http::Response::from_string("hello"));
            }
        });
        let url = format!("http://127.0.0.1:{port}/x");

        let handle = std::thread::spawn(move || {
            tauri::async_runtime::block_on(async move {
                tokio::time::timeout(Duration::from_secs(5), async {
                    reqwest::get(&url).await.unwrap().text().await.unwrap()
                })
                .await
            })
        });
        let result = handle.join().unwrap();
        assert_eq!(result.expect("must not time out"), "hello");
    }
}
