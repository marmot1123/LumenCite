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
use std::time::{Duration, Instant};

mod trace;

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
    proxy_exchange(client, url, token, line).response
}

struct ProxyOutcome {
    response: Option<String>,
    status: &'static str,
    http_status: Option<u16>,
    response_bytes: Option<usize>,
}

fn proxy_exchange(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
    line: &str,
) -> ProxyOutcome {
    let trimmed = line.trim();
    let mut outcome = ProxyOutcome {
        response: None,
        status: "notification",
        http_status: None,
        response_bytes: None,
    };
    if trimmed.is_empty() {
        return outcome;
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
            outcome.http_status = Some(status.as_u16());
            // Keep the existing empty-body fallback if response decoding fails.
            let (text, read_failed) = match resp.text() {
                Ok(text) => {
                    outcome.response_bytes = Some(text.len());
                    (text, false)
                }
                Err(_) => (String::new(), true),
            };
            if status.is_success() {
                if text.trim().is_empty() {
                    outcome.response = error_response(
                        trimmed,
                        -32000,
                        "empty response body from LumenCite MCP server",
                    );
                    outcome.status = if read_failed {
                        "read_error"
                    } else if outcome.response.is_some() {
                        "empty_response"
                    } else {
                        "notification"
                    };
                } else {
                    outcome.response = Some(text);
                    outcome.status = "ok";
                }
            } else {
                outcome.response = error_response(
                    trimmed,
                    -32000,
                    &format!("LumenCite MCP server returned HTTP {}", status.as_u16()),
                );
                outcome.status = "http_error";
            }
        }
        Err(e) => {
            outcome.status = if e.is_timeout() {
                "timeout"
            } else {
                "transport_error"
            };
            outcome.response = error_response(
                trimmed,
                -32000,
                &format!("cannot reach LumenCite MCP server: {e}"),
            );
        }
    }
    outcome
}

/// The same forwarding path with optional metadata-only observation.
fn forward_line(
    client: &reqwest::blocking::Client,
    url: &str,
    token: &str,
    line: &str,
    stdout: &mut impl Write,
    trace: &mut Option<trace::Trace>,
) -> bool {
    let ticket = trace.as_mut().and_then(|t| t.begin(line));
    let forward_start = Instant::now();
    let outcome = proxy_exchange(client, url, token, line);
    let forward_end = Instant::now();
    let delivery = match &outcome.response {
        None => "none",
        Some(resp) => {
            if writeln!(stdout, "{resp}").is_err() {
                "write_error"
            } else if stdout.flush().is_err() {
                "flush_error"
            } else {
                "sent"
            }
        }
    };
    if let (Some(t), Some(ticket)) = (trace.as_mut(), ticket) {
        t.finish(ticket, &outcome, forward_start, forward_end, delivery);
    }
    // Preserve the existing policy: write failure stops, flush failure does not.
    delivery != "write_error"
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
    // token は env 優先だが、shim は LumenCite 本体と同じバイナリなので、未指定なら
    // OS keychain から直接読む（CR-013）。これで生成 config / shell history にトークンを
    // 書かずに済む。env は後方互換のフォールバックとして残す。
    let mut token = std::env::var("LUMENCITE_MCP_TOKEN").unwrap_or_default();
    if token.is_empty() {
        if let Ok(Some(k)) = crate::keychain::get(&crate::keychain::account_for_mcp_token()) {
            token = k;
        }
    }
    if url.is_empty() || token.is_empty() {
        let missing = if url.is_empty() {
            "LUMENCITE_MCP_URL"
        } else {
            "LUMENCITE_MCP_TOKEN (and no token found in the keychain)"
        };
        eprintln!(
            "lumencite-mcp: {missing} is not set. \
             Copy the config snippet from LumenCite settings (and make sure the app has run \
             once so the token exists in the keychain)."
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

    let mut trace = trace::Trace::from_env();
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
        if !forward_line(&client, &url, &token, &line, &mut stdout, &mut trace) {
            break;
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
        let v: Value = serde_json::from_str(
            &proxy_line(&client, "http://127.0.0.1:1/mcp", "t", line).unwrap(),
        )
        .unwrap();
        assert_eq!(v["id"], 9);
        assert_eq!(v["error"]["code"], -32000);
    }

    #[test]
    fn tracing_preserves_http_requests_and_stdout() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", server.server_addr());
        let request = r#"{"jsonrpc":"2.0","id":"PRIVATE","method":"tools/call","params":{"name":"search_entries","arguments":{"query":"PRIVATE"}}}"#;
        let responses = [
            (
                200,
                r#"{"jsonrpc":"2.0","id":"PRIVATE","result":{"content":[{"text":"PRIVATE"}]}}"#,
            ),
            (200, r#"{"error":{"message":"PRIVATE"}}"#),
            (403, "PRIVATE"),
            (200, "PRIVATE"),
            (204, ""),
        ];
        let worker = std::thread::spawn(move || {
            for (status, body) in responses {
                for _ in 0..2 {
                    let mut req = server.recv().unwrap();
                    let mut received = String::new();
                    req.as_reader().read_to_string(&mut received).unwrap();
                    assert_eq!(received, request);
                    req.respond(tiny_http::Response::from_string(body).with_status_code(status))
                        .unwrap();
                }
            }
        });
        let dir = std::env::temp_dir().join(format!(
            "shim-trace-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut trace = Some(trace::Trace::open(&dir).unwrap());
        let client = reqwest::blocking::Client::new();
        for _ in responses {
            let mut off = Vec::new();
            let mut on = Vec::new();
            assert!(forward_line(
                &client, &url, "PRIVATE", request, &mut off, &mut None
            ));
            assert!(forward_line(
                &client, &url, "PRIVATE", request, &mut on, &mut trace
            ));
            assert_eq!(off, on);
        }
        worker.join().unwrap();
        drop(trace);
        let path = std::fs::read_dir(&dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let log = std::fs::read_to_string(path).unwrap();
        assert!(!log.contains("PRIVATE"));
        let rows: Vec<Value> = log
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let ends: Vec<_> = rows
            .iter()
            .filter(|row| row["event"] == "call_end")
            .collect();
        assert_eq!(
            ends.iter()
                .map(|row| row["status"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "ok",
                "rpc_error",
                "http_error",
                "invalid_response",
                "empty_response"
            ]
        );
        for row in ends {
            assert!(
                row["duration_ms"].as_f64().unwrap()
                    >= row["forward_duration_ms"].as_f64().unwrap()
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_line_is_ignored() {
        let client = reqwest::blocking::Client::new();
        assert!(proxy_line(&client, "http://127.0.0.1:1/mcp", "t", "   ").is_none());
    }
}
