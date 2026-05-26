//! LLM 要約クライアント。OpenAI / Anthropic の 2 プロバイダに対応。
//! Server-Sent Events で配信されるトークンをコールバックで呼び出し元へ流す。

pub mod anthropic;
pub mod chat;
pub mod ocr;
pub mod openai;
pub mod tools;

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

// =====================================================================
// Chat (v0.2.0): tool use 対応のマルチターン会話
// =====================================================================
//
// `generate_summary` 系（単発要約）とは別系統。agentic ループ（src/llm/chat.rs, #10）が
// この `ChatProvider` を呼んでツール呼び出しを反復する。フロントへ送る Tauri レベルの
// `ChatStreamEvent`（session_started / tool_call_proposed / ...）は #10/#11 で別途定義する。
// ここで定義するのは「LLM 1 回呼び出し」のプロバイダ抽象まで。

/// チャットメッセージのロール。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

/// マルチモーダル content block。Vision OCR（#12）で画像を載せるため text/image を持つ。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    /// base64 エンコードされた画像。`media_type` 例: "image/png" / "image/jpeg"
    Image { media_type: String, data: String },
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }
}

/// assistant が要求した 1 件のツール呼び出し。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallSpec {
    /// プロバイダ横断で一意な呼び出し ID（tool 結果の突き合わせに使う）
    pub call_id: String,
    /// 例: "fulltext_search" / "add_tag" / "mcp_obsidian_append_note"
    pub tool_name: String,
    /// JSON オブジェクト（ツール引数）
    pub arguments: serde_json::Value,
}

/// LLM に提示するツール定義。`parameters` は JSON Schema（object）。
/// `needs_approval` は既定値であり、最終的な承認可否は
/// `tools::approval::should_auto_approve` がセッションのホワイトリストを見て決める。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub needs_approval: bool,
}

/// プロバイダ非依存の 1 メッセージ。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Vec<ContentBlock>,
    /// role=Assistant がツール呼び出しを行った場合に入る
    pub tool_calls: Option<Vec<ToolCallSpec>>,
    /// role=Tool の結果が紐づく呼び出し ID
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    /// テキストのみの user メッセージ。
    pub fn user_text(s: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::User,
            content: vec![ContentBlock::text(s)],
            tool_calls: None,
            tool_call_id: None,
        }
    }

    /// tool 実行結果メッセージ。
    pub fn tool_result(call_id: impl Into<String>, result: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::Tool,
            content: vec![ContentBlock::text(result)],
            tool_calls: None,
            tool_call_id: Some(call_id.into()),
        }
    }
}

/// LLM がターンを終えた理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// 通常終了（これ以上ツールを呼ばない）
    EndTurn,
    /// ツール呼び出しを要求して停止
    ToolUse,
    /// トークン上限到達
    MaxTokens,
    Other(String),
}

/// 1 ターン（LLM 1 回呼び出し）の結果。
#[derive(Debug, Clone)]
pub struct ChatTurnResult {
    /// このターンで生成された自然言語テキスト全体（on_delta で逐次流したものの連結）
    pub text: String,
    /// このターンで要求されたツール呼び出し（完成形）
    pub tool_calls: Vec<ToolCallSpec>,
    /// LLM がターンを終えた理由。プロバイダが設定する診断用フィールド
    /// （ループは tool_calls の有無で継続判定するため現状は読まない）。
    #[allow(dead_code)]
    pub stop_reason: StopReason,
}

/// tool use 対応の chat ストリーミングを行うプロバイダ抽象。
///
/// - `on_delta` は assistant の自然言語テキストのトークン到着ごとに呼ばれる。
/// - ツール呼び出しは（ストリーミングで組み立てたうえで）完成形を
///   `ChatTurnResult.tool_calls` として返す。
/// - `messages` の `ContentBlock::Image` は各プロバイダのマルチモーダル形式へ変換する。
#[async_trait::async_trait]
pub trait ChatProvider: Send + Sync {
    async fn stream_chat(
        &self,
        api_key: &str,
        model: &str,
        system: &str,
        messages: &[ChatMessage],
        tools: &[ToolSpec],
        // HRTB (`for<'a>`) is required: under async_trait an elided `FnMut(&str)`
        // pins the &str to the method lifetime and cannot be called with strings
        // produced inside the async body (E0597). The explicit higher-ranked bound
        // lets implementations forward streamed deltas.
        on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
    ) -> Result<ChatTurnResult, LlmError>;
}

/// プロバイダ名から `ChatProvider` 実装を返す。
pub fn provider_for(name: &str) -> Result<Box<dyn ChatProvider>, LlmError> {
    match name {
        "openai" => Ok(Box::new(openai::OpenAiProvider)),
        "anthropic" => Ok(Box::new(anthropic::AnthropicProvider)),
        other => Err(LlmError::UnsupportedProvider(other.to_string())),
    }
}
