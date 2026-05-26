//! OpenAI Chat Completions API のストリーミングクライアント。

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde_json::{json, Value};

use super::{
    build_user_prompt, ChatMessage, ChatProvider, ChatTurnResult, LlmError, ToolSpec,
};

/// tool use 対応の chat プロバイダ（#7 で実装）。
pub struct OpenAiProvider;

#[async_trait::async_trait]
impl ChatProvider for OpenAiProvider {
    async fn stream_chat(
        &self,
        _api_key: &str,
        _model: &str,
        _system: &str,
        _messages: &[ChatMessage],
        _tools: &[ToolSpec],
        _on_delta: &mut (dyn FnMut(&str) + Send),
    ) -> Result<ChatTurnResult, LlmError> {
        // TODO(#7): /v1/chat/completions に stream=true + tools を投げ、
        // text delta は on_delta へ、tool_calls の delta は組み立てて ChatTurnResult に返す。
        // ContentBlock::Image は data URL (image_url) として messages に載せる。
        Err(LlmError::Stream(
            "OpenAiProvider::stream_chat not implemented".into(),
        ))
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
