//! MCP (Model Context Protocol) クライアント。
//!
//! 設定された外部 MCP サーバーを stdio で起動し、初期化ハンドシェイク → `tools/list` で
//! ツール一覧を取得して Chat のツールスキーマに `mcp_<id>_<tool>` プレフィックスでマージする。
//! LLM が `mcp_*` ツールを呼ぶと `tools/call` で当該サーバーへ JSON-RPC 転送する。
//!
//! v0.2.0 はクライアントのみ（LumenCite を MCP サーバーとして公開するのは v0.3.0）。
//! JSON-RPC over stdio は公式 SDK に依存せず最小限を自前実装する。

use std::collections::HashMap;
use std::process::Stdio;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::llm::ToolSpec;

const PROTOCOL_VERSION: &str = "2024-11-05";
const TOOL_PREFIX: &str = "mcp_";

/// 外部 MCP サーバー 1 件の設定（Claude Desktop の mcpServers 1 エントリに相当）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// MCP サーバー 1 件の起動状態。`list_mcp_servers` で UI に返し、起動失敗を可視化する。
/// serde: `{ "state": "running", "tool_count": N }` / `{ "state": "failed", "error": "..." }`
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum McpServerStatus {
    /// 起動成功（ハンドシェイク + tools/list 完了）。`tool_count` は取得できたツール数。
    Running { tool_count: usize },
    /// 起動またはハンドシェイクに失敗。`error` は表示用メッセージ。
    Failed { error: String },
}

#[derive(Debug)]
pub enum McpError {
    Spawn(String),
    Protocol(String),
    NotFound(String),
    Io(String),
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Spawn(m) => write!(f, "failed to start MCP server: {m}"),
            McpError::Protocol(m) => write!(f, "MCP protocol error: {m}"),
            McpError::NotFound(m) => write!(f, "MCP tool/server not found: {m}"),
            McpError::Io(m) => write!(f, "MCP io error: {m}"),
        }
    }
}
impl std::error::Error for McpError {}

// ── プロトコル整形（ネットワーク/プロセス非依存・テスト可能） ──────────────────

/// Claude Desktop 互換の JSON を McpServerConfig 配列にパースする。
/// `{ "mcpServers": { "<id>": {command, args?, env?} } }` と、
/// トップレベルが直接 `{ "<id>": {...} }` の両方を受け付ける。空/不正は空配列。
pub fn parse_servers_config(json_str: &str) -> Vec<McpServerConfig> {
    let root: Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let map = root
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .or_else(|| root.as_object());
    let Some(map) = map else { return Vec::new() };

    let mut out = Vec::new();
    for (id, v) in map {
        let Some(command) = v.get("command").and_then(|c| c.as_str()) else {
            continue;
        };
        let args = v
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let env = v
            .get("env")
            .and_then(|e| e.as_object())
            .map(|e| {
                e.iter()
                    .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        out.push(McpServerConfig {
            id: id.clone(),
            command: command.to_string(),
            args,
            env,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// McpServerConfig 配列を Claude Desktop 互換の JSON 文字列にする（設定保存用）。
pub fn serialize_servers_config(servers: &[McpServerConfig]) -> String {
    let mut map = serde_json::Map::new();
    for s in servers {
        map.insert(
            s.id.clone(),
            json!({ "command": s.command, "args": s.args, "env": s.env }),
        );
    }
    json!({ "mcpServers": Value::Object(map) }).to_string()
}

fn prefixed_tool_name(server_id: &str, tool: &str) -> String {
    format!("{TOOL_PREFIX}{server_id}_{tool}")
}

fn build_request(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// `tools/list` の結果から ToolSpec 群を作る（needs_approval は MCP なので常に true）。
fn parse_tools_list(server_id: &str, result: &Value) -> Vec<(String, ToolSpec)> {
    let Some(tools) = result.get("tools").and_then(|t| t.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for t in tools {
        let Some(name) = t.get("name").and_then(|n| n.as_str()) else {
            continue;
        };
        let description = t
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
        let parameters = t
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
        out.push((
            name.to_string(),
            ToolSpec {
                name: prefixed_tool_name(server_id, name),
                description,
                parameters,
                needs_approval: true,
            },
        ));
    }
    out
}

/// `tools/call` の結果 content から人間可読テキストを取り出す。
fn extract_call_text(result: &Value) -> String {
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        let parts: Vec<String> = content
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(String::from))
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    result.to_string()
}

// ── ランタイム（プロセス管理） ───────────────────────────────────────────────

struct ServerIo {
    stdin: ChildStdin,
    reader: Lines<BufReader<ChildStdout>>,
    next_id: i64,
}

struct ServerHandle {
    config: McpServerConfig,
    child: Child,
    io: Mutex<ServerIo>,
    /// orig tool name -> 提示用 ToolSpec
    tools: Vec<(String, ToolSpec)>,
}

impl ServerIo {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        self.next_id += 1;
        let id = self.next_id;
        let line = serde_json::to_string(&build_request(id, method, params)).unwrap() + "\n";
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| McpError::Io(e.to_string()))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| McpError::Io(e.to_string()))?;
        // 当該 id の応答が来るまで読む（通知・ログ行はスキップ）
        loop {
            let line = self
                .reader
                .next_line()
                .await
                .map_err(|e| McpError::Io(e.to_string()))?
                .ok_or_else(|| McpError::Protocol("server closed the stream".into()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            if v.get("id").and_then(|i| i.as_i64()) == Some(id) {
                if let Some(err) = v.get("error") {
                    return Err(McpError::Protocol(err.to_string()));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
            // それ以外（別 id / 通知）は無視
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpError> {
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0", "method": method, "params": params
        }))
        .unwrap()
            + "\n";
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| McpError::Io(e.to_string()))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| McpError::Io(e.to_string()))
    }
}

/// 起動中の MCP サーバー群を保持し、ツール一覧の提供と tools/call のルーティングを行う。
/// `statuses` は起動成否を id 単位で記録し（失敗サーバーは `servers` に入らないため別管理）、
/// UI に「起動失敗」を表示するために使う。
#[derive(Default)]
pub struct McpManager {
    servers: Mutex<HashMap<String, ServerHandle>>,
    statuses: Mutex<HashMap<String, McpServerStatus>>,
}

impl McpManager {
    /// サーバーを起動し、初期化ハンドシェイク → tools/list でツールを登録する。
    /// 成否に関わらず最新状態を `statuses` に記録してから結果を返す（失敗も UI に出すため）。
    pub async fn start(&self, config: McpServerConfig) -> Result<usize, McpError> {
        let id = config.id.clone();
        let result = self.start_inner(config).await;
        let status = match &result {
            Ok(tool_count) => McpServerStatus::Running { tool_count: *tool_count },
            Err(e) => McpServerStatus::Failed { error: e.to_string() },
        };
        self.statuses.lock().await.insert(id, status);
        result
    }

    async fn start_inner(&self, config: McpServerConfig) -> Result<usize, McpError> {
        // 既存の同 id は停止
        self.stop(&config.id).await;

        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| McpError::Spawn(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Spawn("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Spawn("no stdout".into()))?;
        let mut io = ServerIo {
            stdin,
            reader: BufReader::new(stdout).lines(),
            next_id: 0,
        };

        // initialize ハンドシェイク
        io.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "LumenCite", "version": env!("CARGO_PKG_VERSION") }
            }),
        )
        .await?;
        io.notify("notifications/initialized", json!({})).await?;

        let tools_result = io.request("tools/list", json!({})).await?;
        let tools = parse_tools_list(&config.id, &tools_result);
        let count = tools.len();

        let handle = ServerHandle {
            config: config.clone(),
            child,
            io: Mutex::new(io),
            tools,
        };
        self.servers.lock().await.insert(config.id.clone(), handle);
        Ok(count)
    }

    /// サーバーを停止してプロセスを終了し、記録した状態も消す。
    pub async fn stop(&self, id: &str) {
        if let Some(mut handle) = self.servers.lock().await.remove(id) {
            let _ = handle.child.start_kill();
        }
        self.statuses.lock().await.remove(id);
    }

    /// id -> 起動状態のスナップショット。`list_mcp_servers` が設定一覧に重ねて UI へ返す。
    pub async fn statuses(&self) -> HashMap<String, McpServerStatus> {
        self.statuses.lock().await.clone()
    }

    /// 起動中の全サーバーのツール定義（プレフィックス済み）。
    pub async fn tool_specs(&self) -> Vec<ToolSpec> {
        let servers = self.servers.lock().await;
        servers
            .values()
            .flat_map(|h| h.tools.iter().map(|(_, spec)| spec.clone()))
            .collect()
    }

    /// 起動中サーバーの設定一覧。
    pub async fn list_configs(&self) -> Vec<McpServerConfig> {
        let servers = self.servers.lock().await;
        let mut v: Vec<McpServerConfig> = servers.values().map(|h| h.config.clone()).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// `mcp_<id>_<tool>` を解決して tools/call を実行し、結果テキストを返す。
    pub async fn call(&self, prefixed_name: &str, arguments: &Value) -> Result<String, McpError> {
        let mut servers = self.servers.lock().await;
        // プレフィックス名に一致するサーバーとオリジナル tool 名を探す
        for handle in servers.values_mut() {
            if let Some((orig, _)) = handle
                .tools
                .iter()
                .find(|(_, spec)| spec.name == prefixed_name)
            {
                let orig = orig.clone();
                let mut io = handle.io.lock().await;
                let result = io
                    .request(
                        "tools/call",
                        json!({ "name": orig, "arguments": arguments }),
                    )
                    .await?;
                return Ok(extract_call_text(&result));
            }
        }
        Err(McpError::NotFound(prefixed_name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_desktop_format() {
        let s = r#"{"mcpServers":{"obsidian":{"command":"npx","args":["-y","obsidian-mcp"],"env":{"VAULT":"/x"}},"fs":{"command":"mcp-fs"}}}"#;
        let cfgs = parse_servers_config(s);
        assert_eq!(cfgs.len(), 2);
        // id 昇順
        assert_eq!(cfgs[0].id, "fs");
        assert_eq!(cfgs[1].id, "obsidian");
        assert_eq!(cfgs[1].command, "npx");
        assert_eq!(cfgs[1].args, vec!["-y", "obsidian-mcp"]);
        assert_eq!(cfgs[1].env.get("VAULT").map(|s| s.as_str()), Some("/x"));
    }

    #[test]
    fn parse_accepts_bare_object_and_rejects_garbage() {
        let bare = r#"{"srv":{"command":"x"}}"#;
        assert_eq!(parse_servers_config(bare).len(), 1);
        assert!(parse_servers_config("not json").is_empty());
        assert!(parse_servers_config("{}").is_empty());
        // command の無いエントリはスキップ
        assert!(parse_servers_config(r#"{"a":{"args":[]}}"#).is_empty());
    }

    #[test]
    fn roundtrip_serialize_parse() {
        let cfgs = vec![McpServerConfig {
            id: "obsidian".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "obsidian-mcp".into()],
            env: HashMap::from([("VAULT".to_string(), "/vault".to_string())]),
        }];
        let s = serialize_servers_config(&cfgs);
        let back = parse_servers_config(&s);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].command, "npx");
        assert_eq!(back[0].args.len(), 2);
    }

    #[test]
    fn tools_list_builds_prefixed_specs() {
        let result = json!({
            "tools": [
                { "name": "append_note", "description": "Append to a note", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } } } },
                { "name": "search" }
            ]
        });
        let specs = parse_tools_list("obsidian", &result);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].1.name, "mcp_obsidian_append_note");
        assert!(specs[0].1.needs_approval);
        assert_eq!(specs[1].1.name, "mcp_obsidian_search");
        // inputSchema 欠落時は空 object スキーマ
        assert_eq!(
            specs[1].1.parameters,
            json!({ "type": "object", "properties": {} })
        );
    }

    #[test]
    fn extract_text_from_content_blocks() {
        let r = json!({ "content": [{ "type": "text", "text": "line1" }, { "type": "text", "text": "line2" }] });
        assert_eq!(extract_call_text(&r), "line1\nline2");
        // content 無しは JSON 文字列フォールバック
        let r2 = json!({ "ok": true });
        assert_eq!(extract_call_text(&r2), r2.to_string());
    }

    #[test]
    fn build_request_shape() {
        let req = build_request(3, "tools/list", json!({}));
        assert_eq!(req["jsonrpc"], "2.0");
        assert_eq!(req["id"], 3);
        assert_eq!(req["method"], "tools/list");
    }

    fn bad_config(id: &str) -> McpServerConfig {
        McpServerConfig {
            id: id.to_string(),
            // 存在しないバイナリ → spawn が即失敗する
            command: "lumencite-nonexistent-mcp-binary-xyz".to_string(),
            args: vec![],
            env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn start_records_failed_status_for_bad_command() {
        let mgr = McpManager::default();
        let res = mgr.start(bad_config("bad")).await;
        assert!(res.is_err(), "存在しないコマンドは起動失敗するはず");

        match mgr.statuses().await.get("bad") {
            Some(McpServerStatus::Failed { error }) => assert!(!error.is_empty()),
            other => panic!("expected Failed status, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stop_clears_recorded_status() {
        let mgr = McpManager::default();
        let _ = mgr.start(bad_config("bad")).await;
        assert!(mgr.statuses().await.contains_key("bad"));

        mgr.stop("bad").await;
        assert!(!mgr.statuses().await.contains_key("bad"), "stop で状態も消える");
    }

    #[test]
    fn status_serializes_with_state_tag() {
        let running = serde_json::to_value(McpServerStatus::Running { tool_count: 3 }).unwrap();
        assert_eq!(running, json!({ "state": "running", "tool_count": 3 }));
        let failed = serde_json::to_value(McpServerStatus::Failed { error: "boom".into() }).unwrap();
        assert_eq!(failed, json!({ "state": "failed", "error": "boom" }));
    }
}
