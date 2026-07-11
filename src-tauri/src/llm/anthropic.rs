//! Anthropic Messages API のストリーミングクライアント。

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{json, Value};

use super::{
    build_user_prompt, ChatMessage, ChatProvider, ChatTurnResult, ContentBlock, LlmError, Role,
    StopReason, ToolCallSpec, ToolSpec,
};

/// tool use 対応の chat プロバイダ（#7 で実装）。
pub struct AnthropicProvider;

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// chat ターンのトークン上限。
const CHAT_MAX_TOKENS: u32 = 4096;

#[async_trait::async_trait]
impl ChatProvider for AnthropicProvider {
    async fn stream_chat(
        &self,
        api_key: &str,
        model: &str,
        system: &str,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<ChatTurnResult, LlmError> {
        if api_key.trim().is_empty() {
            return Err(LlmError::MissingApiKey);
        }
        let payload = build_messages_body(model, system, messages, tools);

        // ストリーミング（CR-033）: connect と read（チャンク間 idle）のタイムアウトのみ。
        // 全体 timeout は長い生成を切らないため張らない。
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .read_timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| LlmError::Stream(e.to_string()))?;
        let resp = client
            .post(ENDPOINT)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let message = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, message });
        }

        let mut acc = ChatAccumulator::new();
        let mut stream = resp.bytes_stream().eventsource();
        while let Some(event) = stream.next().await {
            let event = event.map_err(|e| LlmError::Stream(e.to_string()))?;
            let data = event.data.trim();
            if data.is_empty() {
                continue;
            }
            // error イベントは Err、message_stop で terminal 確定（CR-033）。
            if let Some(delta) = acc.ingest(data)? {
                on_delta(&delta);
            }
            if acc.is_terminal() {
                break;
            }
        }

        // message_stop を観測しないまま切断されたら成功扱いにしない（CR-033）。
        acc.into_result()
    }
}

/// (system, messages, tools, model) から Anthropic Messages API のリクエストボディを組み立てる。
/// ネットワークに触れない純粋関数。
fn build_messages_body(
    model: &str,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> Value {
    let out_messages = build_messages(messages);

    let mut body = json!({
        "model": model,
        "stream": true,
        "max_tokens": CHAT_MAX_TOKENS,
        "messages": out_messages,
    });

    if !system.trim().is_empty() {
        body["system"] = json!(system);
    }

    if !tools.is_empty() {
        let tool_defs: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect();
        body["tools"] = Value::Array(tool_defs);
    }

    body
}

/// `ChatMessage` 列を Anthropic の messages 配列へ変換する。
/// Anthropic は 1 つの assistant ターンの `tool_use` 群に対する `tool_result` を、直後の
/// **1 つの** user メッセージにまとめて要求するため、連続する Tool メッセージを 1 件に統合する。
fn build_messages(messages: &[ChatMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        if messages[i].role == Role::Tool {
            let mut blocks = Vec::new();
            while i < messages.len() && messages[i].role == Role::Tool {
                blocks.push(json!({
                    "type": "tool_result",
                    "tool_use_id": messages[i].tool_call_id.clone().unwrap_or_default(),
                    "content": concat_text(&messages[i].content),
                }));
                i += 1;
            }
            out.push(json!({ "role": "user", "content": blocks }));
        } else {
            out.push(convert_message(&messages[i]));
            i += 1;
        }
    }
    out
}

/// 1 つの `ChatMessage`（User / Assistant）を Anthropic の message オブジェクトへ変換する。
/// Tool は [`build_messages`] がまとめて処理するためここでは通常通らない。
fn convert_message(msg: &ChatMessage) -> Value {
    match msg.role {
        Role::Tool => {
            // Anthropic ではツール結果は user ロールの tool_result ブロックで渡す。
            json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "content": concat_text(&msg.content),
                }]
            })
        }
        Role::User | Role::Assistant => {
            let role = if msg.role == Role::User {
                "user"
            } else {
                "assistant"
            };
            // Anthropic は空のテキスト content block を拒否する（"text content blocks must be
            // non-empty"）。ツールのみ呼んだ（テキスト無し）assistant ターンでは空テキストを除外する。
            let mut blocks: Vec<Value> = msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } if text.trim().is_empty() => None,
                    ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
                    ContentBlock::Image { media_type, data } => Some(json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": data,
                        }
                    })),
                })
                .collect();
            if let Some(calls) = &msg.tool_calls {
                for c in calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": c.call_id,
                        "name": c.tool_name,
                        "input": c.arguments,
                    }));
                }
            }
            // content 配列が空になるのも拒否されるため、最低 1 ブロックを保証する。
            if blocks.is_empty() {
                blocks.push(json!({ "type": "text", "text": "(no content)" }));
            }
            json!({ "role": role, "content": blocks })
        }
    }
}

/// テキスト block を連結する（画像 block は無視）。
fn concat_text(blocks: &[ContentBlock]) -> String {
    let mut s = String::new();
    for b in blocks {
        if let ContentBlock::Text { text } = b {
            s.push_str(text);
        }
    }
    s
}

/// ストリーミングで組み立て中の 1 件の tool_use ブロック。
struct PartialToolCall {
    id: String,
    name: String,
    input_json: String,
}

/// 1 イベント処理の結果。`delta` はこのイベントで到着したテキスト、
/// `stop` は `message_stop` を受け取ったか。
struct EventOutcome {
    delta: Option<String>,
    stop: bool,
}

/// 既にパース済みの SSE イベント JSON を 1 件ずつ食べて、テキスト delta / tool_use /
/// stop_reason を組み立てるステートフルなアキュムレータ。ネットワークから独立。
struct ChatAccumulator {
    text: String,
    /// content block index -> partial tool call
    calls: Vec<Option<PartialToolCall>>,
    stop_reason: Option<StopReason>,
    /// `message_stop` を観測したか（CR-033: 終端マーカーの必須化）。
    saw_stop: bool,
}

impl ChatAccumulator {
    fn new() -> Self {
        ChatAccumulator {
            text: String::new(),
            calls: Vec::new(),
            stop_reason: None,
            saw_stop: false,
        }
    }

    /// 1 件の SSE データ文字列を処理する（CR-033）。
    /// - `type == "error"` の event は成功扱いせず `Err`。
    /// - `message_stop` を観測したら terminal を立てる。
    /// - パース不能なデータは無視する（`Ok(None)`）。
    ///
    /// 戻り値はこのイベントで到着したテキスト delta（無ければ `None`）。
    fn ingest(&mut self, data: &str) -> Result<Option<String>, LlmError> {
        let parsed: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        if let Some(msg) = sse_error_message(&parsed) {
            return Err(LlmError::Stream(format!("provider stream error: {msg}")));
        }
        let outcome = self.push_event(&parsed);
        if outcome.stop {
            self.saw_stop = true;
        }
        Ok(outcome.delta)
    }

    fn is_terminal(&self) -> bool {
        self.saw_stop
    }

    /// `message_stop` を観測していれば結果を、していなければ truncation エラーを返す（CR-033）。
    fn into_result(self) -> Result<ChatTurnResult, LlmError> {
        if !self.saw_stop {
            return Err(LlmError::Stream(
                "Anthropic stream ended without message_stop; response may be truncated"
                    .to_string(),
            ));
        }
        Ok(self.finish())
    }

    /// イベントを 1 件処理し、結果（テキスト delta と message_stop か否か）を返す。
    /// `on_delta` の呼び出しは呼び出し側に任せ、ライフタイムの制約なしに使えるようにする。
    fn push_event(&mut self, parsed: &Value) -> EventOutcome {
        let mut delta_text: Option<String> = None;
        let ev_type = parsed.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ev_type {
            "content_block_start" => {
                let index = parsed
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(0) as usize;
                let block = parsed.get("content_block");
                let is_tool_use = block
                    .and_then(|b| b.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("tool_use");
                self.ensure_len(index);
                if is_tool_use {
                    let id = block
                        .and_then(|b| b.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = block
                        .and_then(|b| b.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    self.calls[index] = Some(PartialToolCall {
                        id,
                        name,
                        input_json: String::new(),
                    });
                }
            }
            "content_block_delta" => {
                let index = parsed
                    .get("index")
                    .and_then(|i| i.as_u64())
                    .unwrap_or(0) as usize;
                let delta = parsed.get("delta");
                let delta_type = delta
                    .and_then(|d| d.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                match delta_type {
                    "text_delta" => {
                        if let Some(text) =
                            delta.and_then(|d| d.get("text")).and_then(|t| t.as_str())
                        {
                            if !text.is_empty() {
                                self.text.push_str(text);
                                delta_text = Some(text.to_string());
                            }
                        }
                    }
                    "input_json_delta" => {
                        if let Some(partial) = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(|t| t.as_str())
                        {
                            self.ensure_len(index);
                            if let Some(slot) = self.calls.get_mut(index).and_then(|c| c.as_mut()) {
                                slot.input_json.push_str(partial);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(reason) = parsed
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|r| r.as_str())
                {
                    self.stop_reason = Some(map_stop_reason(reason));
                }
            }
            "message_stop" => {
                return EventOutcome {
                    delta: delta_text,
                    stop: true,
                }
            }
            _ => {}
        }
        EventOutcome {
            delta: delta_text,
            stop: false,
        }
    }

    fn ensure_len(&mut self, index: usize) {
        while self.calls.len() <= index {
            self.calls.push(None);
        }
    }

    fn finish(self) -> ChatTurnResult {
        let tool_calls: Vec<ToolCallSpec> = self
            .calls
            .into_iter()
            .flatten()
            .map(|c| ToolCallSpec {
                call_id: c.id,
                tool_name: c.name,
                arguments: parse_input(&c.input_json),
            })
            .collect();
        let stop_reason = self.stop_reason.unwrap_or(StopReason::EndTurn);
        ChatTurnResult {
            text: self.text,
            tool_calls,
            stop_reason,
        }
    }
}

/// 蓄積された partial_json を JSON Value に。空 / パース失敗時は `{}`。
fn parse_input(s: &str) -> Value {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| json!({}))
}

/// SSE データ中の provider エラーイベントを検出する（CR-033）。
/// Anthropic は `{"type": "error", "error": {"type": ..., "message": ...}}` を
/// ストリーム内で送ることがあり、無視すると途中終了を正常完了と誤認する。
fn sse_error_message(parsed: &Value) -> Option<String> {
    if parsed.get("type").and_then(|v| v.as_str()) != Some("error") {
        return None;
    }
    let msg = parsed
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| parsed.to_string());
    Some(msg)
}

/// 単発要約ストリームの 1 データを処理する（CR-033）。
/// `message_stop` で `*saw_stop` を立て、error event は `Err`。戻り値は本文 delta。
fn summary_step(data: &str, saw_stop: &mut bool) -> Result<Option<String>, LlmError> {
    let parsed: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if let Some(msg) = sse_error_message(&parsed) {
        return Err(LlmError::Stream(format!("provider stream error: {msg}")));
    }
    match parsed.get("type").and_then(|v| v.as_str()) {
        Some("message_stop") => {
            *saw_stop = true;
            Ok(None)
        }
        Some("content_block_delta") => {
            let text = parsed
                .get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
                .filter(|t| !t.is_empty())
                .map(str::to_string);
            Ok(text)
        }
        _ => Ok(None),
    }
}

fn map_stop_reason(reason: &str) -> StopReason {
    match reason {
        "tool_use" => StopReason::ToolUse,
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    }
}

pub async fn stream_messages<F>(
    model: &str,
    api_key: &str,
    system_prompt: &str,
    title: &str,
    body: &str,
    mut on_delta: F,
) -> Result<String, LlmError>
where
    F: FnMut(&str) + Send,
{
    let user_prompt = build_user_prompt(title, body);
    let payload = json!({
        "model": model,
        "stream": true,
        "max_tokens": 1024,
        "system": system_prompt,
        "messages": [
            { "role": "user", "content": user_prompt },
        ],
    });

    // OCR/vision（単発応答・CR-033）: connect + 全体タイムアウトを張る。
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| LlmError::Stream(e.to_string()))?;
    let resp = client
        .post(ENDPOINT)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let message = resp.text().await.unwrap_or_default();
        return Err(LlmError::Api { status, message });
    }

    let mut full = String::new();
    let mut saw_stop = false;
    let mut stream = resp.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| LlmError::Stream(e.to_string()))?;
        let data = event.data.trim();
        if data.is_empty() { continue; }
        if let Some(delta) = summary_step(data, &mut saw_stop)? {
            full.push_str(&delta);
            on_delta(&delta);
        }
        if saw_stop { break; }
    }

    // message_stop 無しで切断されたら truncation とみなし成功扱いしない（CR-033）。
    if !saw_stop {
        return Err(LlmError::Stream(
            "Anthropic stream ended without message_stop; response may be truncated".to_string(),
        ));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ChatMessage;

    fn drain(parser: &mut ChatAccumulator, events: &[Value]) -> String {
        let mut collected = String::new();
        for ev in events {
            if let Some(delta) = parser.push_event(ev).delta {
                collected.push_str(&delta);
            }
        }
        collected
    }

    #[test]
    fn body_sets_system_top_level_and_max_tokens() {
        let messages = vec![ChatMessage::user_text("hi")];
        let body = build_messages_body("claude-3-5-sonnet", "be precise", &messages, &[]);
        assert_eq!(body["model"], json!("claude-3-5-sonnet"));
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["max_tokens"], json!(4096));
        assert_eq!(body["system"], json!("be precise"));
        // user message: content is array of blocks
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], json!("user"));
        assert_eq!(msgs[0]["content"][0]["type"], json!("text"));
        assert_eq!(msgs[0]["content"][0]["text"], json!("hi"));
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn body_omits_empty_system() {
        let messages = vec![ChatMessage::user_text("hi")];
        let body = build_messages_body("claude", "  ", &messages, &[]);
        assert!(body.get("system").is_none());
    }

    #[test]
    fn body_serializes_assistant_tool_use_block() {
        let assistant = ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text("let me search")],
            tool_calls: Some(vec![ToolCallSpec {
                call_id: "toolu_1".into(),
                tool_name: "search".into(),
                arguments: json!({ "q": "rust" }),
            }]),
            tool_call_id: None,
        };
        let body = build_messages_body("claude", "", &[assistant], &[]);
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], json!("text"));
        assert_eq!(content[0]["text"], json!("let me search"));
        assert_eq!(content[1]["type"], json!("tool_use"));
        assert_eq!(content[1]["id"], json!("toolu_1"));
        assert_eq!(content[1]["name"], json!("search"));
        // input must be a JSON object, not a string
        assert_eq!(content[1]["input"], json!({ "q": "rust" }));
    }

    #[test]
    fn body_omits_empty_text_block_for_tool_only_turn() {
        // テキスト無しでツールだけ呼んだ assistant ターン: 空テキストブロックを出さない。
        let assistant = ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text("")],
            tool_calls: Some(vec![ToolCallSpec {
                call_id: "toolu_1".into(),
                tool_name: "search".into(),
                arguments: json!({}),
            }]),
            tool_call_id: None,
        };
        let body = build_messages_body("claude", "", &[assistant], &[]);
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1, "empty text block should be dropped");
        assert_eq!(content[0]["type"], json!("tool_use"));
    }

    #[test]
    fn body_guards_against_empty_content_array() {
        // 空テキストのみ・ツール無しでも content 配列を空にしない。
        let msg = ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text("   ")],
            tool_calls: None,
            tool_call_id: None,
        };
        let body = build_messages_body("claude", "", &[msg], &[]);
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], json!("text"));
    }

    #[test]
    fn body_coalesces_consecutive_tool_results_into_one_user_message() {
        // 1 ターンで 2 ツール → assistant(tool_use×2) の直後は tool_result×2 を持つ
        // 単一 user メッセージでなければ Anthropic に拒否される。
        let assistant = ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text("")],
            tool_calls: Some(vec![
                ToolCallSpec { call_id: "a".into(), tool_name: "x".into(), arguments: json!({}) },
                ToolCallSpec { call_id: "b".into(), tool_name: "y".into(), arguments: json!({}) },
            ]),
            tool_call_id: None,
        };
        let msgs = vec![
            ChatMessage::user_text("hi"),
            assistant,
            ChatMessage::tool_result("a", "res-a"),
            ChatMessage::tool_result("b", "res-b"),
        ];
        let body = build_messages_body("claude", "", &msgs, &[]);
        let out = body["messages"].as_array().unwrap();
        // user, assistant, (merged tool user) = 3 件
        assert_eq!(out.len(), 3);
        assert_eq!(out[2]["role"], json!("user"));
        let results = out[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["type"], json!("tool_result"));
        assert_eq!(results[0]["tool_use_id"], json!("a"));
        assert_eq!(results[1]["tool_use_id"], json!("b"));
    }

    #[test]
    fn body_maps_tool_result_to_user_message() {
        let tool = ChatMessage::tool_result("toolu_1", "found 3 papers");
        let body = build_messages_body("claude", "", &[tool], &[]);
        let m = &body["messages"][0];
        assert_eq!(m["role"], json!("user"));
        let block = &m["content"][0];
        assert_eq!(block["type"], json!("tool_result"));
        assert_eq!(block["tool_use_id"], json!("toolu_1"));
        assert_eq!(block["content"], json!("found 3 papers"));
    }

    #[test]
    fn body_emits_image_source_block() {
        let msg = ChatMessage {
            role: Role::User,
            content: vec![ContentBlock::Image {
                media_type: "image/jpeg".into(),
                data: "QUJD".into(),
            }],
            tool_calls: None,
            tool_call_id: None,
        };
        let body = build_messages_body("claude", "", &[msg], &[]);
        let block = &body["messages"][0]["content"][0];
        assert_eq!(block["type"], json!("image"));
        assert_eq!(block["source"]["type"], json!("base64"));
        assert_eq!(block["source"]["media_type"], json!("image/jpeg"));
        assert_eq!(block["source"]["data"], json!("QUJD"));
    }

    #[test]
    fn body_includes_tools_with_input_schema() {
        let tools = vec![ToolSpec {
            name: "search".into(),
            description: "search papers".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            needs_approval: false,
        }];
        let body = build_messages_body("claude", "", &[ChatMessage::user_text("hi")], &tools);
        let def = &body["tools"][0];
        assert_eq!(def["name"], json!("search"));
        assert_eq!(def["description"], json!("search papers"));
        assert_eq!(def["input_schema"]["type"], json!("object"));
    }

    #[test]
    fn accumulator_collects_text_and_end_turn() {
        let mut parser = ChatAccumulator::new();
        let events = vec![
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "Hel" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "lo" } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" } }),
            json!({ "type": "message_stop" }),
        ];
        let streamed = drain(&mut parser, &events);
        let result = parser.finish();
        assert_eq!(streamed, "Hello");
        assert_eq!(result.text, "Hello");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn accumulator_assembles_tool_use_with_input_json_deltas() {
        let mut parser = ChatAccumulator::new();
        let events = vec![
            json!({ "type": "content_block_start", "index": 0,
                    "content_block": { "type": "tool_use", "id": "toolu_x", "name": "search", "input": {} } }),
            json!({ "type": "content_block_delta", "index": 0,
                    "delta": { "type": "input_json_delta", "partial_json": "{\"q\":" } }),
            json!({ "type": "content_block_delta", "index": 0,
                    "delta": { "type": "input_json_delta", "partial_json": "\"rust\"}" } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
            json!({ "type": "message_stop" }),
        ];
        let streamed = drain(&mut parser, &events);
        let result = parser.finish();
        assert_eq!(streamed, "");
        assert_eq!(result.stop_reason, StopReason::ToolUse);
        assert_eq!(result.tool_calls.len(), 1);
        let call = &result.tool_calls[0];
        assert_eq!(call.call_id, "toolu_x");
        assert_eq!(call.tool_name, "search");
        assert_eq!(call.arguments, json!({ "q": "rust" }));
    }

    #[test]
    fn accumulator_handles_text_then_tool_use_blocks() {
        let mut parser = ChatAccumulator::new();
        let events = vec![
            json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text" } }),
            json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "ok" } }),
            json!({ "type": "content_block_start", "index": 1,
                    "content_block": { "type": "tool_use", "id": "t1", "name": "add_tag" } }),
            json!({ "type": "content_block_delta", "index": 1,
                    "delta": { "type": "input_json_delta", "partial_json": "{}" } }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
            json!({ "type": "message_stop" }),
        ];
        let streamed = drain(&mut parser, &events);
        let result = parser.finish();
        assert_eq!(streamed, "ok");
        assert_eq!(result.text, "ok");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].call_id, "t1");
        assert_eq!(result.tool_calls[0].tool_name, "add_tag");
        assert_eq!(result.tool_calls[0].arguments, json!({}));
        assert_eq!(result.stop_reason, StopReason::ToolUse);
    }

    #[test]
    fn push_event_signals_stop_on_message_stop() {
        let mut parser = ChatAccumulator::new();
        let outcome = parser.push_event(&json!({ "type": "message_stop" }));
        assert!(outcome.stop);
    }

    #[test]
    fn map_stop_reason_covers_variants() {
        assert_eq!(map_stop_reason("tool_use"), StopReason::ToolUse);
        assert_eq!(map_stop_reason("end_turn"), StopReason::EndTurn);
        assert_eq!(map_stop_reason("max_tokens"), StopReason::MaxTokens);
        assert_eq!(
            map_stop_reason("pause_turn"),
            StopReason::Other("pause_turn".into())
        );
    }

    #[test]
    fn parse_input_defaults_to_empty_object() {
        assert_eq!(parse_input(""), json!({}));
        assert_eq!(parse_input("garbage"), json!({}));
        assert_eq!(parse_input("{\"a\":1}"), json!({ "a": 1 }));
    }

    /// CR-033: message_stop 無しで切断されたら成功扱いにしない。
    #[test]
    fn into_result_errs_when_stream_truncated() {
        let mut acc = ChatAccumulator::new();
        acc.ingest(&json!({ "type": "content_block_start", "index": 0,
                            "content_block": { "type": "text" } }).to_string())
            .unwrap();
        acc.ingest(&json!({ "type": "content_block_delta", "index": 0,
                            "delta": { "type": "text_delta", "text": "Hi" } }).to_string())
            .unwrap();
        // EOF（message_stop 未観測）。
        assert!(!acc.is_terminal());
        let err = acc.into_result().unwrap_err();
        assert!(matches!(err, LlmError::Stream(_)), "truncation は Stream エラー: {err:?}");
    }

    /// CR-033: message_stop を観測すれば正常完了。
    #[test]
    fn into_result_ok_on_message_stop() {
        let mut acc = ChatAccumulator::new();
        acc.ingest(&json!({ "type": "content_block_delta", "index": 0,
                            "delta": { "type": "text_delta", "text": "ok" } }).to_string())
            .unwrap();
        acc.ingest(&json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" } }).to_string())
            .unwrap();
        acc.ingest(&json!({ "type": "message_stop" }).to_string()).unwrap();
        assert!(acc.is_terminal());
        let result = acc.into_result().unwrap();
        assert_eq!(result.text, "ok");
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    /// CR-033: ストリーム内の error event は Err として即座に浮上させる。
    #[test]
    fn ingest_surfaces_stream_error_event() {
        let mut acc = ChatAccumulator::new();
        let err = acc
            .ingest(&json!({ "type": "error",
                             "error": { "type": "overloaded_error", "message": "overloaded" } }).to_string())
            .unwrap_err();
        match err {
            LlmError::Stream(m) => assert!(m.contains("overloaded"), "message 透過: {m}"),
            other => panic!("expected Stream error, got {other:?}"),
        }
    }

    /// CR-033: 単発要約ストリームも同じ規約（error 検出 + message_stop 必須）。
    #[test]
    fn summary_step_detects_error_and_terminal() {
        let mut stop = false;
        let d = summary_step(
            &json!({ "type": "content_block_delta", "delta": { "type": "text_delta", "text": "x" } }).to_string(),
            &mut stop,
        )
        .unwrap();
        assert_eq!(d.as_deref(), Some("x"));
        assert!(!stop);
        let err = summary_step(
            &json!({ "type": "error", "error": { "message": "boom" } }).to_string(),
            &mut stop,
        )
        .unwrap_err();
        assert!(matches!(err, LlmError::Stream(_)));
        let none = summary_step(&json!({ "type": "message_stop" }).to_string(), &mut stop).unwrap();
        assert!(none.is_none());
        assert!(stop);
    }
}
