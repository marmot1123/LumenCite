//! LumenCite を MCP **サーバー**として公開する（Phase 1: read-only）。
//!
//! これは外部 MCP サーバーへ接続する `mcp`（クライアント）とは逆向きで、
//! Claude Desktop / Claude Code などの MCP クライアントが LumenCite のライブラリを
//! ツール経由で参照できるようにするもの。サーバー側では LLM を呼ばない（推論は
//! 接続元のサブスクリプション側が担う）ため、API キー等は不要。
//!
//! ## Phase 1 の範囲
//! - トランスポート: localhost にバインドする HTTP（JSON-RPC 2.0 / 単発 POST → JSON 応答）
//! - 認可: `Authorization: Bearer <token>`（インストールごとの token。キーチェーン保管）
//! - 公開ツール: **read 系のみ**。`search` モジュールの read ツール定義を流用（単一ソース）し、
//!   LaTeX 連携向けの `search_entries` / `resolve_citation_key` / `export_bibtex` を追加。
//!   write/mutate/ocr ツールは公開しない（許可リスト外として拒否）。
//!
//! プロトコルのディスパッチ（[`handle_rpc`]）はトランスポート非依存で、HTTP を介さず
//! 単体テストできる。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::llm::tools::{search, ToolContext, ToolError};
use crate::llm::ToolCallSpec;

/// MCP プロトコルバージョン（クライアント側 `mcp` と揃える）。
const PROTOCOL_VERSION: &str = "2024-11-05";

/// 設定が無いときの既定バインドポート。
pub const DEFAULT_PORT: u16 = 3917;

/// `search` モジュールから流用して公開する read ツール名。
/// これ以外（write/mutate/ocr）は公開せず、`tools/call` でも拒否する。
const SHARED_READ_TOOLS: &[&str] = &[
    "fulltext_search",
    "get_entry",
    "list_collections",
    "list_tags",
];

// ─── ツール定義（tools/list） ────────────────────────────────────────────────

/// 公開する read-only ツールの MCP 形式定義（`{name, description, inputSchema}`）。
fn read_tool_specs() -> Vec<Value> {
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
        "description": "Export entries as BibTeX. Pass entry_ids to export specific entries, or omit \
            to export the whole library (trash excluded). Returns the .bib text.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_ids": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Entry ids to export; omit for the whole library."
                }
            }
        }
    }));

    tools
}

// ─── JSON-RPC ディスパッチ（トランスポート非依存） ──────────────────────────

/// JSON-RPC リクエスト 1 件を処理する。通知（`id` 無し）の場合は応答不要なので `None`。
pub async fn handle_rpc(pool: &SqlitePool, app_data_dir: &Path, req: &Value) -> Option<Value> {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // 通知（id 無し）には応答しない（JSON-RPC 2.0）。
    let id = req.get("id").cloned()?;
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    let resp = match method {
        "initialize" => ok(id, json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "LumenCite", "version": env!("CARGO_PKG_VERSION") }
        })),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": read_tool_specs() })),
        "tools/call" => handle_tools_call(pool, app_data_dir, id, &params).await,
        other => err(id, -32601, &format!("method not found: {other}")),
    };
    Some(resp)
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn handle_tools_call(
    pool: &SqlitePool,
    app_data_dir: &Path,
    id: Value,
    params: &Value,
) -> Value {
    let Some(name) = params.get("name").and_then(|n| n.as_str()) else {
        return err(id, -32602, "missing tool name");
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match exec_read_tool(pool, app_data_dir, name, args).await {
        Ok(text) => ok(id, tool_content(text, false)),
        // ツール実行エラーは JSON-RPC エラーではなく isError 結果として返す（MCP 慣例）。
        Err(ToolError::UnknownTool(_)) => ok(
            id,
            tool_content(format!("unknown or unavailable tool: {name}"), true),
        ),
        Err(e) => ok(id, tool_content(e.to_string(), true)),
    }
}

fn tool_content(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

// ─── read ツール実行 ─────────────────────────────────────────────────────────

async fn exec_read_tool(
    pool: &SqlitePool,
    app_data_dir: &Path,
    name: &str,
    args: Value,
) -> Result<String, ToolError> {
    // 既存チャットの read 系はそのまま流用（scope は "all" 固定）。
    if SHARED_READ_TOOLS.contains(&name) {
        let ctx = ToolContext {
            pool,
            session_id: 0,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir,
        };
        let call = ToolCallSpec {
            call_id: "mcp-server".to_string(),
            tool_name: name.to_string(),
            arguments: args,
        };
        return search::try_execute(&ctx, &call)
            .await
            .unwrap_or_else(|| Err(ToolError::UnknownTool(name.to_string())));
    }

    match name {
        "search_entries" => exec_search_entries(pool, &args).await,
        "resolve_citation_key" => exec_resolve_citation_key(pool, &args).await,
        "export_bibtex" => exec_export_bibtex(pool, &args).await,
        // write/mutate/ocr 等は Phase 1 では公開しない。
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
    let entry_ids = args
        .get("entry_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>());
    crate::bibtex::export_bibtex(pool, entry_ids)
        .await
        .map_err(ToolError::Execution)
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
    pub fn start(
        &self,
        pool: SqlitePool,
        app_data_dir: PathBuf,
        port: u16,
        token: String,
    ) -> Result<u16, String> {
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
            serve_loop(server, stop_thread, pool, app_data_dir, token);
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

fn serve_loop(
    server: tiny_http::Server,
    stop: Arc<AtomicBool>,
    pool: SqlitePool,
    app_data_dir: PathBuf,
    token: String,
) {
    // recv_timeout で定期的に stop フラグを確認しつつ accept する。
    while !stop.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(300)) {
            Ok(Some(req)) => handle_http_request(req, &pool, &app_data_dir, &token),
            Ok(None) => continue, // タイムアウト → ループ先頭で stop を再確認
            Err(_) => break,
        }
    }
}

fn handle_http_request(
    mut req: tiny_http::Request,
    pool: &SqlitePool,
    app_data_dir: &Path,
    token: &str,
) {
    use tiny_http::{Header, Response};

    // 認可: Authorization: Bearer <token>
    let authorized = req.headers().iter().any(|h| {
        h.field.equiv("Authorization")
            && h.value
                .as_str()
                .strip_prefix("Bearer ")
                .map(|t| t == token)
                .unwrap_or(false)
    });
    if !authorized {
        let _ = req.respond(Response::from_string("unauthorized").with_status_code(401));
        return;
    }

    if *req.method() != tiny_http::Method::Post {
        let _ = req.respond(Response::from_string("method not allowed").with_status_code(405));
        return;
    }

    let mut body = String::new();
    if req.as_reader().read_to_string(&mut body).is_err() {
        let _ = req.respond(Response::from_string("bad request body").with_status_code(400));
        return;
    }

    let response_value: Option<Value> = match serde_json::from_str::<Value>(&body) {
        Ok(v) => tauri::async_runtime::block_on(handle_rpc(pool, app_data_dir, &v)),
        Err(e) => Some(json!({
            "jsonrpc": "2.0", "id": null,
            "error": { "code": -32700, "message": format!("parse error: {e}") }
        })),
    };

    match response_value {
        Some(v) => {
            let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                .expect("static header");
            let _ = req.respond(Response::from_string(v.to_string()).with_header(ct));
        }
        // 通知のみ（応答不要）→ 202 Accepted
        None => {
            let _ = req.respond(Response::from_string("").with_status_code(202));
        }
    }
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

    async fn call_tool(pool: &SqlitePool, name: &str, args: Value) -> Value {
        let r = req("tools/call", json!({ "name": name, "arguments": args }));
        handle_rpc(pool, Path::new(""), &r).await.unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn initialize_returns_protocol_and_server_info(pool: SqlitePool) {
        let resp = handle_rpc(&pool, Path::new(""), &req("initialize", json!({})))
            .await
            .unwrap();
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "LumenCite");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_list_has_read_tools_and_excludes_mutate(pool: SqlitePool) {
        let resp = handle_rpc(&pool, Path::new(""), &req("tools/list", json!({})))
            .await
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
        ] {
            assert!(names.contains(&expected), "missing read tool: {expected}");
        }
        // write/mutate/ocr は公開しない。
        for forbidden in ["create_entry", "update_entry", "delete_entry", "add_tag", "ocr_pdf"] {
            assert!(!names.contains(&forbidden), "must not expose: {forbidden}");
        }
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
            .unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn notification_without_id_returns_none(pool: SqlitePool) {
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized", "params": {} });
        let resp = handle_rpc(&pool, Path::new(""), &notif).await;
        assert!(resp.is_none());
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
        // port 0 で OS 割り当て。実バインドポートが返る。
        let port = manager
            .start(pool.clone(), PathBuf::from(""), 0, token.clone())
            .expect("server should bind");
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
