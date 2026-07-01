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
pub async fn handle_rpc(pool: &SqlitePool, app_data_dir: &Path, req: &Value) -> RpcOutcome {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // 通知（id 無し）には応答しない（JSON-RPC 2.0）。
    let Some(id) = req.get("id").cloned() else {
        return RpcOutcome { response: None, mutated: false };
    };
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
    let write_on = write_enabled(pool).await;

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

fn serve_loop(server: tiny_http::Server, stop: Arc<AtomicBool>, deps: ServerDeps, token: String) {
    // recv_timeout で定期的に stop フラグを確認しつつ accept する。
    while !stop.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(300)) {
            Ok(Some(req)) => handle_http_request(req, &deps, &token),
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
            let outcome = tauri::async_runtime::block_on(clipper::handle_clip(&deps.pool, &clip_req));
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

/// PDF ダウンロードジョブを応答後に spawn する（実体は M3 で実装）。
fn spawn_pdf_job(_deps: &ServerDeps, _job: clipper::PdfJob) {
    // M3: download_and_attach を tauri::async_runtime::spawn で実行し、
    // 成功時に sync_tx + entries-changed を発火する。
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
