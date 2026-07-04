//! v0.7.0 CLI の**書き込み経路（ハイブリッド C）**。
//!
//! 地雷＝「アプリ起動中 × 直接 DB 書込」による UI 陳腐化 / WAL 競合。これを次のルーティングで回避:
//!
//! 1. `--force` 指定 → 直接 DB 書込（アプリが開いていれば一覧が陳腐化しうる旨を警告）。
//! 2. MCP サーバーに到達可（keychain にトークン有 + `ping` 成功）→ **HTTP 経由**でサーバーに
//!    委譲する。サーバーが公開用の書込ゲート（`mcp_server.write_enabled`）を適用し、成功時は
//!    `.bib` 同期と GUI 一覧のリアルタイム更新まで行う（＝UI が陳腐化しない安全経路）。
//! 3. 到達不可（アプリ停止と判断）→ **直接 DB 書込**。成功後に `.bib` 同期を best-effort で行う。
//!
//! どちらの経路も MCP の `tools/call`（JSON-RPC）と同じリクエスト形状を作り、HTTP なら POST、
//! 直接なら [`crate::mcp_server::handle_rpc_with_write`] を `write_on = true` で呼ぶ（ツール実装・
//! 監査ログ・`mutated` フラグを単一ソースで共有）。

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::db;
use crate::keychain;
use crate::mcp_server;

use super::CmdOutput;

/// `tools/call` JSON-RPC リクエストを組み立てる。
pub fn tools_call(name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
}

/// 書込ルーティングの入口。`ro_pool` は execute() が開いた読取専用プール（ポート設定の読み出しに使う）。
pub async fn dispatch_write(
    db_path: &Path,
    ro_pool: &SqlitePool,
    request: Value,
    force: bool,
) -> Result<CmdOutput, String> {
    let server = if force {
        None
    } else {
        probe_server(ro_pool).await
    };
    match server {
        Some((url, token)) => write_via_http(&url, &token, &request).await,
        None => write_direct(db_path, &request, force).await,
    }
}

/// MCP サーバーが localhost で稼働中なら `(url, token)` を返す。トークンが無い / 到達不可なら None。
async fn probe_server(ro_pool: &SqlitePool) -> Option<(String, String)> {
    let token = keychain::get(&keychain::account_for_mcp_token())
        .ok()
        .flatten()?;
    if token.is_empty() {
        return None;
    }
    let port = db::settings::get_setting(ro_pool, db::settings::MCP_SERVER_PORT_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(mcp_server::DEFAULT_PORT);
    let url = format!("http://127.0.0.1:{port}/mcp");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1500))
        .build()
        .ok()?;
    let ping = json!({ "jsonrpc": "2.0", "id": 0, "method": "ping" });
    let resp = client
        .post(&url)
        .bearer_auth(&token)
        .json(&ping)
        .send()
        .await
        .ok()?;
    if resp.status().is_success() {
        Some((url, token))
    } else {
        None
    }
}

/// 稼働中サーバーへ HTTP でツール呼び出しを委譲する。
async fn write_via_http(url: &str, token: &str, request: &Value) -> Result<CmdOutput, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(url)
        .bearer_auth(token)
        .json(request)
        .send()
        .await
        .map_err(|e| format!("failed to reach LumenCite MCP server: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!(
            "LumenCite MCP server returned HTTP {}",
            status.as_u16()
        ));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("invalid JSON from MCP server: {e}"))?;
    interpret_tool_result(&body, true)
}

/// 直接 DB へ書き込む（`write_on = true` 強制）。成功後に `.bib` 同期を best-effort。
async fn write_direct(db_path: &Path, request: &Value, force: bool) -> Result<CmdOutput, String> {
    let pool = open_readwrite_pool(db_path).await?;
    let app_dir = db_path.parent().unwrap_or_else(|| Path::new("."));
    let outcome = mcp_server::handle_rpc_with_write(&pool, app_dir, true, request).await;

    let mut result = match &outcome.response {
        Some(resp) => interpret_tool_result(resp, false),
        None => Err("no response from write handler".to_string()),
    };

    // write が成功したら .bib 同期（設定されていれば）。GUI が無いので UI イベントは発火しない。
    if outcome.mutated {
        best_effort_bib_sync(&pool).await;
        // --force で（アプリ起動中の可能性を承知で）直接書いた場合のみ陳腐化を警告する。
        if force {
            if let Ok(out) = result.as_mut() {
                out.warnings.push(
                    "wrote directly to the database; if the LumenCite app is open, \
                     its list may show stale data until refreshed."
                        .to_string(),
                );
            }
        }
    }

    pool.close().await;
    result
}

/// 書込可能なプール（WAL / foreign_keys）。`query_only` は付けない。
async fn open_readwrite_pool(db_path: &Path) -> Result<SqlitePool, String> {
    if !db_path.exists() {
        return Err(format!(
            "LumenCite library not found at {}.\n       \
             Launch the LumenCite app once to create it, or set LUMENCITE_DB_PATH.",
            db_path.display()
        ));
    }
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(|e| format!("failed to open library at {}: {e}", db_path.display()))
}

async fn best_effort_bib_sync(pool: &SqlitePool) {
    if let Ok(Some(path)) =
        db::settings::get_setting(pool, db::settings::BIBTEX_SYNC_PATH_KEY).await
    {
        if !path.trim().is_empty() {
            let _ = crate::bibtex::sync_bibtex(pool, &std::path::PathBuf::from(path)).await;
        }
    }
}

/// JSON-RPC 応答（HTTP でも直接でも同形）からツール結果テキストを取り出す。
/// `isError` の書込拒否は分かりやすいガイダンスに変換する。
fn interpret_tool_result(body: &Value, via_server: bool) -> Result<CmdOutput, String> {
    if let Some(e) = body.get("error") {
        let msg = e
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        return Err(format!("MCP error: {msg}"));
    }
    let result = body
        .get("result")
        .ok_or_else(|| "missing `result` in MCP response".to_string())?;
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let text = result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.iter().find_map(|it| it.get("text").and_then(|t| t.as_str())))
        .unwrap_or("")
        .to_string();

    if is_error {
        if via_server && text.contains("disabled") {
            return Err(format!(
                "{text}\n       The LumenCite app is running but MCP write access is disabled. \
                 Enable it in the app settings, or re-run with --force to write directly \
                 (the app's open window may show stale data until refreshed)."
            ));
        }
        return Err(text);
    }
    Ok(CmdOutput::new(text))
}

/// `--field key=value` の集合を JSON オブジェクトへ。`=` が無ければエラー。
pub fn parse_fields(fields: &[String]) -> Result<serde_json::Map<String, Value>, String> {
    let mut map = serde_json::Map::new();
    for f in fields {
        let (k, v) = f
            .split_once('=')
            .ok_or_else(|| format!("invalid --field '{f}' (expected key=value)"))?;
        if k.is_empty() {
            return Err(format!("invalid --field '{f}' (empty key)"));
        }
        map.insert(k.to_string(), json!(v));
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_call_shapes_jsonrpc() {
        let r = tools_call("add_tag", json!({ "entry_id": 3, "tag_name": "ml" }));
        assert_eq!(r["method"], "tools/call");
        assert_eq!(r["params"]["name"], "add_tag");
        assert_eq!(r["params"]["arguments"]["tag_name"], "ml");
    }

    #[test]
    fn parse_fields_builds_map_and_rejects_bad() {
        let m = parse_fields(&["journal=Nature".to_string(), "volume=12".to_string()]).unwrap();
        assert_eq!(m["journal"], "Nature");
        assert_eq!(m["volume"], "12");
        assert!(parse_fields(&["nokey".to_string()]).is_err());
        assert!(parse_fields(&["=v".to_string()]).is_err());
    }

    #[test]
    fn interpret_ok_result_extracts_text() {
        let body = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "content": [{ "type": "text", "text": "Created entry 42." }], "isError": false }
        });
        let out = interpret_tool_result(&body, false).unwrap();
        assert_eq!(out.stdout, "Created entry 42.");
    }

    #[test]
    fn interpret_error_result_is_err() {
        let body = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "content": [{ "type": "text", "text": "boom" }], "isError": true }
        });
        assert!(interpret_tool_result(&body, false).is_err());
    }

    #[test]
    fn interpret_write_disabled_via_server_gives_guidance() {
        let body = json!({
            "jsonrpc": "2.0", "id": 1,
            "result": { "content": [{ "type": "text", "text": "write tools are disabled on this MCP server: add_tag" }], "isError": true }
        });
        let err = interpret_tool_result(&body, true).unwrap_err();
        assert!(err.contains("--force"));
        assert!(err.contains("disabled"));
    }

    #[test]
    fn interpret_jsonrpc_error_is_err() {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": "nope" } });
        let err = interpret_tool_result(&body, false).unwrap_err();
        assert!(err.contains("nope"));
    }
}
