//! Anthropic Messages API のストリーミングクライアント。

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{json, Value};

use super::{
    build_user_prompt, ChatMessage, ChatProvider, ChatTurnResult, LlmError, ToolSpec,
};

/// tool use 対応の chat プロバイダ（#7 で実装）。
pub struct AnthropicProvider;

#[async_trait::async_trait]
impl ChatProvider for AnthropicProvider {
    async fn stream_chat(
        &self,
        _api_key: &str,
        _model: &str,
        _system: &str,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
        _on_delta: &mut (dyn FnMut(&str) + Send),
    ) -> Result<ChatTurnResult, LlmError> {
        // TODO(#7): /v1/messages に stream=true + tools を投げ、content_block_delta(text) は
        // on_delta へ、tool_use ブロック（input_json_delta を連結）は ChatTurnResult.tool_calls へ。
        // ContentBlock::Image は { type: image, source: { type: base64, media_type, data } } として載せる。
        Err(LlmError::Stream(
            "AnthropicProvider::stream_chat not implemented".into(),
        ))
    }
}

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

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

    let client = reqwest::Client::new();
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
    let mut stream = resp.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        let event = event.map_err(|e| LlmError::Stream(e.to_string()))?;
        let data = event.data.trim();
        if data.is_empty() { continue; }
        let parsed: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // content_block_delta -> delta.text
        if parsed.get("type").and_then(|v| v.as_str()) == Some("content_block_delta") {
            if let Some(text) = parsed
                .get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
            {
                if !text.is_empty() {
                    full.push_str(text);
                    on_delta(text);
                }
            }
        }
        if parsed.get("type").and_then(|v| v.as_str()) == Some("message_stop") {
            break;
        }
    }

    Ok(full)
}
