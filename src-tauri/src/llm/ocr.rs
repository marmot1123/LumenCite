//! LLM Vision を用いた OCR（プロバイダ非依存）。
//! 1 ページ分の画像（base64）を受け取り、文字起こしテキストを返す。

use crate::llm::{provider_for, ChatMessage, ContentBlock, LlmError, Role};

pub const OCR_SYSTEM_PROMPT: &str = "You are an OCR engine. Transcribe ALL text visible in the \
image faithfully, preserving reading order and paragraph structure. Do not summarize, translate, \
or add any commentary — output only the transcribed text. If the page has no text, output nothing.";

/// 画像 1 枚（base64）を OCR してテキストを返す。`media_type` 例: "image/png"。
pub async fn ocr_image(
    provider: &str,
    model: &str,
    api_key: &str,
    media_type: &str,
    data_base64: &str,
) -> Result<String, LlmError> {
    if api_key.trim().is_empty() {
        return Err(LlmError::MissingApiKey);
    }
    let p = provider_for(provider)?;
    let messages = vec![ChatMessage {
        role: Role::User,
        content: vec![
            ContentBlock::text("Transcribe the text in this page image."),
            ContentBlock::Image {
                media_type: media_type.to_string(),
                data: data_base64.to_string(),
            },
        ],
        tool_calls: None,
        tool_call_id: None,
    }];
    let mut noop = |_: &str| {};
    let result = p
        .stream_chat(api_key, model, OCR_SYSTEM_PROMPT, &messages, &[], &mut noop)
        .await?;
    Ok(result.text)
}
