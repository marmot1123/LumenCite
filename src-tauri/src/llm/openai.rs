//! OpenAI Chat Completions API のストリーミングクライアント。
//
// chat (tool use) 用のヘルパは `OpenAiProvider::stream_chat` 経由でのみ使われるが、
// その入口（`provider_for` / agentic ループ #10）がまだ未配線のため、非テストビルドでは
// dead_code 警告になる。配線され次第解消する想定のスキャフォルドなので、ここでは抑制する。
#![allow(dead_code)]

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{json, Value};

use super::{
    build_user_prompt, ChatMessage, ChatProvider, ChatTurnResult, ContentBlock, LlmError, Role,
    StopReason, ToolCallSpec, ToolSpec,
};

/// tool use 対応の chat プロバイダ（#7 で実装）。
pub struct OpenAiProvider;

const CHAT_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

#[async_trait::async_trait]
impl ChatProvider for OpenAiProvider {
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
        let payload = build_chat_body(model, system, messages, tools);

        let client = reqwest::Client::new();
        let resp = client
            .post(CHAT_ENDPOINT)
            .bearer_auth(api_key)
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
            if data == "[DONE]" {
                break;
            }
            let parsed: Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(delta) = acc.push_event(&parsed) {
                on_delta(&delta);
            }
        }

        Ok(acc.finish())
    }
}

/// (system, messages, tools, model) から OpenAI chat completions のリクエストボディを組み立てる。
/// ネットワークに触れない純粋関数なのでユニットテストできる。
fn build_chat_body(
    model: &str,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolSpec],
) -> Value {
    let mut out_messages: Vec<Value> = Vec::new();

    if !system.trim().is_empty() {
        out_messages.push(json!({ "role": "system", "content": system }));
    }

    for msg in messages {
        match msg.role {
            Role::User => {
                out_messages.push(json!({
                    "role": "user",
                    "content": openai_content(&msg.content),
                }));
            }
            Role::Assistant => {
                let mut obj = serde_json::Map::new();
                obj.insert("role".into(), json!("assistant"));
                // assistant の content はテキストのみ連結。空なら null。
                let text = concat_text(&msg.content);
                if text.is_empty() {
                    obj.insert("content".into(), Value::Null);
                } else {
                    obj.insert("content".into(), json!(text));
                }
                if let Some(calls) = &msg.tool_calls {
                    let tool_calls: Vec<Value> = calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.call_id,
                                "type": "function",
                                "function": {
                                    "name": c.tool_name,
                                    "arguments": c.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    obj.insert("tool_calls".into(), Value::Array(tool_calls));
                }
                out_messages.push(Value::Object(obj));
            }
            Role::Tool => {
                out_messages.push(json!({
                    "role": "tool",
                    "tool_call_id": msg.tool_call_id.clone().unwrap_or_default(),
                    "content": concat_text(&msg.content),
                }));
            }
        }
    }

    let mut body = json!({
        "model": model,
        "stream": true,
        "messages": out_messages,
    });

    if !tools.is_empty() {
        let tool_defs: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = Value::Array(tool_defs);
    }

    body
}

/// content block 群を、すべてテキストなら 1 個の文字列に、画像を含むなら配列にする。
fn openai_content(blocks: &[ContentBlock]) -> Value {
    let has_image = blocks
        .iter()
        .any(|b| matches!(b, ContentBlock::Image { .. }));
    if !has_image {
        return json!(concat_text(blocks));
    }
    let parts: Vec<Value> = blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text { text } => json!({ "type": "text", "text": text }),
            ContentBlock::Image { media_type, data } => json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", media_type, data) }
            }),
        })
        .collect();
    Value::Array(parts)
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

/// ストリーミングで組み立て中の 1 件のツール呼び出し。
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// 既にパース済みの SSE イベント JSON を 1 件ずつ食べて、
/// テキスト delta / tool_calls / stop_reason を組み立てるステートフルなアキュムレータ。
/// ネットワークから切り離してあるのでユニットテストできる。
struct ChatAccumulator {
    text: String,
    /// index 順に並ぶ partial tool call
    calls: Vec<PartialToolCall>,
    stop_reason: Option<StopReason>,
}

impl ChatAccumulator {
    fn new() -> Self {
        ChatAccumulator {
            text: String::new(),
            calls: Vec::new(),
            stop_reason: None,
        }
    }

    /// イベントを 1 件処理し、このイベントで新たに到着したテキスト delta を返す
    /// （無ければ `None`）。`on_delta` の呼び出しは呼び出し側に任せることで、
    /// ストリーミング本体（trait method）とテストの双方からライフタイムの制約なしに使える。
    fn push_event(&mut self, parsed: &Value) -> Option<String> {
        let choice = parsed.get("choices").and_then(|c| c.get(0))?;
        let mut delta_text: Option<String> = None;

        if let Some(delta) = choice.get("delta") {
            if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    self.text.push_str(content);
                    delta_text = Some(content.to_string());
                }
            }
            if let Some(tool_calls) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    while self.calls.len() <= index {
                        self.calls.push(PartialToolCall {
                            id: String::new(),
                            name: String::new(),
                            arguments: String::new(),
                        });
                    }
                    let slot = &mut self.calls[index];
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() {
                            slot.id = id.to_string();
                        }
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                            if !name.is_empty() {
                                slot.name = name.to_string();
                            }
                        }
                        if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                            slot.arguments.push_str(args);
                        }
                    }
                }
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            self.stop_reason = Some(map_finish_reason(reason));
        }

        delta_text
    }

    fn finish(self) -> ChatTurnResult {
        let tool_calls: Vec<ToolCallSpec> = self
            .calls
            .into_iter()
            .filter(|c| !c.id.is_empty() || !c.name.is_empty())
            .map(|c| ToolCallSpec {
                call_id: c.id,
                tool_name: c.name,
                arguments: parse_arguments(&c.arguments),
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

/// 蓄積された arguments 文字列を JSON Value に。空 / パース失敗時は `{}`。
fn parse_arguments(s: &str) -> Value {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| json!({}))
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "tool_calls" => StopReason::ToolUse,
        "stop" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        other => StopReason::Other(other.to_string()),
    }
}

const ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";

pub async fn stream_chat<F>(
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
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt },
        ],
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(ENDPOINT)
        .bearer_auth(api_key)
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
    let mut stream = resp.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| LlmError::Stream(e.to_string()))?;
        let data = event.data.trim();
        if data.is_empty() { continue; }
        if data == "[DONE]" { break; }
        let parsed: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(content) = parsed
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("delta"))
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            if !content.is_empty() {
                full.push_str(content);
                on_delta(content);
            }
        }
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
            if let Some(delta) = parser.push_event(ev) {
                collected.push_str(&delta);
            }
        }
        collected
    }

    #[test]
    fn body_prepends_system_and_maps_user_text() {
        let messages = vec![ChatMessage::user_text("hello")];
        let body = build_chat_body("gpt-4o", "be nice", &messages, &[]);
        assert_eq!(body["model"], json!("gpt-4o"));
        assert_eq!(body["stream"], json!(true));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], json!("system"));
        assert_eq!(msgs[0]["content"], json!("be nice"));
        assert_eq!(msgs[1]["role"], json!("user"));
        assert_eq!(msgs[1]["content"], json!("hello"));
        // empty tools -> no "tools" key
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn body_skips_empty_system() {
        let messages = vec![ChatMessage::user_text("hi")];
        let body = build_chat_body("gpt-4o", "   ", &messages, &[]);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], json!("user"));
    }

    #[test]
    fn body_serializes_assistant_tool_calls_and_tool_result() {
        let assistant = ChatMessage {
            role: Role::Assistant,
            content: vec![],
            tool_calls: Some(vec![ToolCallSpec {
                call_id: "call_1".into(),
                tool_name: "search".into(),
                arguments: json!({ "q": "rust" }),
            }]),
            tool_call_id: None,
        };
        let tool = ChatMessage::tool_result("call_1", "found 3 papers");
        let messages = vec![assistant, tool];
        let body = build_chat_body("gpt-4o", "", &messages, &[]);
        let msgs = body["messages"].as_array().unwrap();

        // assistant with tool_calls, content null (no text)
        assert_eq!(msgs[0]["role"], json!("assistant"));
        assert_eq!(msgs[0]["content"], Value::Null);
        let tc = &msgs[0]["tool_calls"][0];
        assert_eq!(tc["id"], json!("call_1"));
        assert_eq!(tc["type"], json!("function"));
        assert_eq!(tc["function"]["name"], json!("search"));
        // arguments must be a JSON *string*
        assert_eq!(tc["function"]["arguments"], json!("{\"q\":\"rust\"}"));

        // tool result message
        assert_eq!(msgs[1]["role"], json!("tool"));
        assert_eq!(msgs[1]["tool_call_id"], json!("call_1"));
        assert_eq!(msgs[1]["content"], json!("found 3 papers"));
    }

    #[test]
    fn body_emits_image_as_content_array() {
        let msg = ChatMessage {
            role: Role::User,
            content: vec![
                ContentBlock::text("look:"),
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "QUJD".into(),
                },
            ],
            tool_calls: None,
            tool_call_id: None,
        };
        let body = build_chat_body("gpt-4o", "", &[msg], &[]);
        let content = &body["messages"][0]["content"];
        assert!(content.is_array());
        assert_eq!(content[0]["type"], json!("text"));
        assert_eq!(content[0]["text"], json!("look:"));
        assert_eq!(content[1]["type"], json!("image_url"));
        assert_eq!(
            content[1]["image_url"]["url"],
            json!("data:image/png;base64,QUJD")
        );
    }

    #[test]
    fn body_includes_tools_when_present() {
        let tools = vec![ToolSpec {
            name: "search".into(),
            description: "search papers".into(),
            parameters: json!({ "type": "object", "properties": {} }),
            needs_approval: false,
        }];
        let body = build_chat_body("gpt-4o", "", &[ChatMessage::user_text("hi")], &tools);
        let defs = body["tools"].as_array().unwrap();
        assert_eq!(defs[0]["type"], json!("function"));
        assert_eq!(defs[0]["function"]["name"], json!("search"));
        assert_eq!(defs[0]["function"]["description"], json!("search papers"));
        assert_eq!(defs[0]["function"]["parameters"]["type"], json!("object"));
    }

    #[test]
    fn accumulator_collects_text_and_end_turn() {
        let mut parser = ChatAccumulator::new();
        let events = vec![
            json!({ "choices": [{ "delta": { "content": "Hel" } }] }),
            json!({ "choices": [{ "delta": { "content": "lo" } }] }),
            json!({ "choices": [{ "delta": {}, "finish_reason": "stop" }] }),
        ];
        let streamed = drain(&mut parser, &events);
        let result = parser.finish();
        assert_eq!(streamed, "Hello");
        assert_eq!(result.text, "Hello");
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn accumulator_assembles_multi_delta_tool_call() {
        let mut parser = ChatAccumulator::new();
        let events = vec![
            // first delta: id + name + start of args
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "call_abc", "function": { "name": "search", "arguments": "{\"q\":" } }
            ] } }] }),
            // second delta: more args (no id/name)
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "function": { "arguments": "\"rust\"}" } }
            ] } }] }),
            json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }] }),
        ];
        let streamed = drain(&mut parser, &events);
        let result = parser.finish();
        assert_eq!(streamed, "");
        assert_eq!(result.stop_reason, StopReason::ToolUse);
        assert_eq!(result.tool_calls.len(), 1);
        let call = &result.tool_calls[0];
        assert_eq!(call.call_id, "call_abc");
        assert_eq!(call.tool_name, "search");
        assert_eq!(call.arguments, json!({ "q": "rust" }));
    }

    #[test]
    fn accumulator_handles_two_parallel_tool_calls() {
        let mut parser = ChatAccumulator::new();
        let events = vec![
            json!({ "choices": [{ "delta": { "tool_calls": [
                { "index": 0, "id": "c0", "function": { "name": "a", "arguments": "{}" } },
                { "index": 1, "id": "c1", "function": { "name": "b", "arguments": "{\"x\":1}" } }
            ] } }] }),
            json!({ "choices": [{ "finish_reason": "tool_calls" }] }),
        ];
        drain(&mut parser, &events);
        let result = parser.finish();
        assert_eq!(result.tool_calls.len(), 2);
        assert_eq!(result.tool_calls[0].tool_name, "a");
        assert_eq!(result.tool_calls[0].arguments, json!({}));
        assert_eq!(result.tool_calls[1].tool_name, "b");
        assert_eq!(result.tool_calls[1].arguments, json!({ "x": 1 }));
    }

    #[test]
    fn map_finish_reason_covers_variants() {
        assert_eq!(map_finish_reason("tool_calls"), StopReason::ToolUse);
        assert_eq!(map_finish_reason("stop"), StopReason::EndTurn);
        assert_eq!(map_finish_reason("length"), StopReason::MaxTokens);
        assert_eq!(
            map_finish_reason("content_filter"),
            StopReason::Other("content_filter".into())
        );
    }

    #[test]
    fn parse_arguments_defaults_to_empty_object() {
        assert_eq!(parse_arguments(""), json!({}));
        assert_eq!(parse_arguments("not json"), json!({}));
        assert_eq!(parse_arguments("{\"a\":1}"), json!({ "a": 1 }));
    }
}
