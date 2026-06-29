//! `lumencite-mcp` stdio shim（MCP Phase 3）。
//!
//! Claude Desktop は stdio トランスポートのみ対応し、リモート（HTTP）MCP に直結できない。
//! そこで本体バイナリを `--mcp-stdio` 付きで起動すると、Tauri/GUI を立ち上げずに
//! 「stdio ↔ localhost HTTP」プロキシとして振る舞い、アプリ内蔵 MCP サーバーへ橋渡しする。
//! 別バイナリ（externalBin/sidecar）にしないことで、追加の署名・notarize 対象を増やさない。
//!
//! 接続先 URL とトークンは Claude Desktop 設定の `env` から受け取る:
//! - `LUMENCITE_MCP_URL`   例: `http://127.0.0.1:3917/mcp`
//! - `LUMENCITE_MCP_TOKEN` キーチェーンと同じ Bearer トークン
//!
//! MCP の stdio フレーミングは「1 行 = 1 JSON-RPC メッセージ」（改行区切り）。

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{json, Value};

/// stdin から受け取った 1 行（JSON-RPC メッセージ）を localhost HTTP サーバーへ転送し、
/// stdout へ書き出すべき応答文字列を返す。応答不要（通知・空行）の場合は `None`。
///
/// HTTP 層が分離されているので、テストでは実サーバーを立てて直接呼べる。
pub fn proxy_line(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
    line: &str,
) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    match client
        .post(url)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .body(trimmed.to_owned())
        .send()
    {
        Ok(resp) => {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            if status.is_success() {
                if text.trim().is_empty() {
                    // 通知（id 無し / 202）は正常に無出力。ただし id 付きリクエストへ
                    // 空ボディが返るのは異常なので、クライアントを無限待機させないよう
                    // エラー応答へ変換する（error_response は id 無しなら None を返す）。
                    error_response(
                        trimmed,
                        -32000,
                        "empty response body from LumenCite MCP server",
                    )
                } else {
                    Some(text)
                }
            } else {
                // 401 など。生のボディを垂れ流さず JSON-RPC エラーへ変換する。
                error_response(
                    trimmed,
                    -32000,
                    &format!("LumenCite MCP server returned HTTP {}", status.as_u16()),
                )
            }
        }
        // 接続不可（アプリ未起動・ポート不一致など）。
        Err(e) => error_response(
            trimmed,
            -32000,
            &format!("cannot reach LumenCite MCP server: {e}"),
        ),
    }
}

/// リクエスト行から `id` を引き継いだ JSON-RPC エラー応答を作る。
/// `id` が無い（= 通知）場合は応答してはいけないので `None`。
fn error_response(request_line: &str, code: i64, message: &str) -> Option<String> {
    let id = serde_json::from_str::<Value>(request_line)
        .ok()
        .and_then(|v| v.get("id").cloned())
        .filter(|id| !id.is_null())?;
    Some(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        })
        .to_string(),
    )
}

/// stdio プロキシのメインループ。`main.rs` が `--mcp-stdio` 検出時に呼ぶ。
/// 戻り値はプロセス終了コード。
pub fn run_stdio_proxy() -> i32 {
    let url = std::env::var("LUMENCITE_MCP_URL").unwrap_or_default();
    let token = std::env::var("LUMENCITE_MCP_TOKEN").unwrap_or_default();
    if url.is_empty() || token.is_empty() {
        let missing = if url.is_empty() {
            "LUMENCITE_MCP_URL"
        } else {
            "LUMENCITE_MCP_TOKEN"
        };
        eprintln!(
            "lumencite-mcp: {missing} is not set. \
             Copy the Claude Desktop config snippet from LumenCite settings."
        );
        return 2;
    }

    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("lumencite-mcp: failed to build HTTP client: {e}");
            return 1;
        }
    };

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            // 非 UTF-8 の行は読み飛ばしてセッションを継続する（lines() は当該行を
            // 消費済みなのでスピンしない）。それ以外（EOF / パイプ切断）は終了。
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                eprintln!("lumencite-mcp: skipping a non-UTF8 line: {e}");
                continue;
            }
            Err(_) => break,
        };
        if let Some(resp) = proxy_line(&client, &url, &token, &line) {
            if writeln!(stdout, "{resp}").is_err() {
                break;
            }
            let _ = stdout.flush();
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_server::{McpServerManager, ServerDeps};
    use sqlx::SqlitePool;
    use std::path::PathBuf;

    /// テスト用に実 HTTP サーバーを 0 番ポートで起動し、(manager, url) を返す。
    fn start_test_server(pool: SqlitePool, token: &str) -> (McpServerManager, String) {
        let (sync_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let deps = ServerDeps {
            pool,
            app_data_dir: PathBuf::from(""),
            sync_tx,
            app: None,
        };
        let manager = McpServerManager::default();
        let port = manager.start(deps, 0, token.to_string()).expect("bind");
        (manager, format!("http://127.0.0.1:{port}/mcp"))
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn proxy_forwards_request_and_returns_response(pool: SqlitePool) {
        let token = "tok-shim".to_string();
        let (manager, url) = start_test_server(pool, &token);
        // reqwest::blocking は自前ランタイムを持つため、tokio ランタイム外の OS スレッドで実行する。
        let out = std::thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            let line = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{}}"#;
            let r = proxy_line(&client, &url, &token, line);
            manager.stop();
            r
        })
        .join()
        .unwrap();

        let v: Value = serde_json::from_str(&out.expect("response expected")).unwrap();
        assert_eq!(v["id"], 7);
        assert!(v["result"]["tools"].is_array());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn proxy_maps_bad_token_to_jsonrpc_error(pool: SqlitePool) {
        let (manager, url) = start_test_server(pool, "right-token");
        let out = std::thread::spawn(move || {
            let client = reqwest::blocking::Client::new();
            let line = r#"{"jsonrpc":"2.0","id":3,"method":"ping","params":{}}"#;
            let r = proxy_line(&client, &url, "wrong-token", line);
            manager.stop();
            r
        })
        .join()
        .unwrap();

        let v: Value = serde_json::from_str(&out.expect("error response expected")).unwrap();
        assert_eq!(v["id"], 3);
        assert_eq!(v["error"]["code"], -32000);
    }

    #[test]
    fn notification_without_id_yields_no_output_on_error() {
        let client = reqwest::blocking::Client::new();
        // 到達不可能なポート。id 無し（通知）はエラーでも応答しない。
        let line = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(proxy_line(&client, "http://127.0.0.1:1/mcp", "t", line).is_none());
    }

    #[test]
    fn request_with_id_maps_unreachable_to_error() {
        let client = reqwest::blocking::Client::new();
        let line = r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#;
        let v: Value =
            serde_json::from_str(&proxy_line(&client, "http://127.0.0.1:1/mcp", "t", line).unwrap())
                .unwrap();
        assert_eq!(v["id"], 9);
        assert_eq!(v["error"]["code"], -32000);
    }

    #[test]
    fn empty_line_is_ignored() {
        let client = reqwest::blocking::Client::new();
        assert!(proxy_line(&client, "http://127.0.0.1:1/mcp", "t", "   ").is_none());
    }
}
