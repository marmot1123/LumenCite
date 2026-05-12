//! LLM 要約クライアント。OpenAI / Anthropic の 2 プロバイダに対応。
//! Server-Sent Events で配信されるトークンをコールバックで呼び出し元へ流す。

pub mod anthropic;
pub mod openai;

use std::fmt;

#[derive(Debug)]
pub enum LlmError {
    MissingApiKey,
    Network(String),
    Api { status: u16, message: String },
    Stream(String),
    UnsupportedProvider(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::MissingApiKey => write!(f, "API key is not configured"),
            LlmError::Network(m) => write!(f, "network error: {}", m),
            LlmError::Api { status, message } => write!(f, "API error {}: {}", status, message),
            LlmError::Stream(m) => write!(f, "stream error: {}", m),
            LlmError::UnsupportedProvider(p) => write!(f, "unsupported provider: {}", p),
        }
    }
}

impl std::error::Error for LlmError {}

impl From<reqwest::Error> for LlmError {
    fn from(e: reqwest::Error) -> Self {
        LlmError::Network(e.to_string())
    }
}

/// 要約用のデフォルトシステムプロンプト。ユーザーが設定で上書きできる。
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are an assistant that summarizes academic papers. \
Produce a concise summary in 3-5 sentences focusing on: \
(1) the problem the paper addresses, (2) the key contribution or method, \
(3) the headline result. Use the same language as the input. \
Do not include lists, headings, or markdown — produce a single short paragraph.";

/// ユーザーメッセージ本文を組み立てる。
pub fn build_user_prompt(title: &str, body: &str) -> String {
    format!(
        "Title: {title}\n\n---\n\n{body}",
        title = title,
        body = body,
    )
}

/// プロバイダに応じてストリーミング要約を実行する。
/// `on_delta` は各トークン到着時に呼ばれる（複数回）。
/// 戻り値は完成した全文。
/// `system_prompt` が空文字なら DEFAULT_SYSTEM_PROMPT を使う。
pub async fn generate_summary<F>(
    provider: &str,
    model: &str,
    api_key: &str,
    system_prompt: &str,
    title: &str,
    body: &str,
    on_delta: F,
) -> Result<String, LlmError>
where
    F: FnMut(&str) + Send,
{
    if api_key.trim().is_empty() {
        return Err(LlmError::MissingApiKey);
    }
    let prompt = if system_prompt.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT
    } else {
        system_prompt
    };
    match provider {
        "openai" => openai::stream_chat(model, api_key, prompt, title, body, on_delta).await,
        "anthropic" => anthropic::stream_messages(model, api_key, prompt, title, body, on_delta).await,
        other => Err(LlmError::UnsupportedProvider(other.to_string())),
    }
}

/// テスト接続: 1 トークンだけ返るようなプロンプトで疎通確認する。
pub async fn test_connection(provider: &str, model: &str, api_key: &str) -> Result<(), LlmError> {
    let _full = generate_summary(
        provider, model, api_key,
        "Reply with the single word: ok.",
        "ping", "ping",
        |_| {},
    )
    .await?;
    Ok(())
}
