//! Chat の tool use 基盤。
//!
//! read 系（`search`）/ write 系（`mutate`）の各サブモジュールがそれぞれ
//! `specs()`（LLM へ提示するツール定義）と `try_execute()`（実行）を提供し、
//! ここで集約・ディスパッチする。MCP ツール（#13）は別経路でマージされる。
//!
//! 各サブモジュールの契約:
//! - `pub fn specs() -> Vec<ToolSpec>` — 当該モジュールが提供するツール定義。
//! - `pub async fn try_execute(ctx, call) -> Option<Result<String, ToolError>>`
//!   — `call.tool_name` を扱うなら `Some(結果)`、扱わないなら `None` を返す。
//!   返す `String` は LLM に渡すツール結果テキスト（人間可読 or JSON 文字列）。

pub mod approval;
pub mod mutate;
pub mod ocr;
pub mod search;

use super::{ToolCallSpec, ToolSpec};
use sqlx::SqlitePool;
use std::path::Path;

/// ツール実行時に渡されるコンテキスト。検索系はこのスコープを尊重する。
pub struct ToolContext<'a> {
    pub pool: &'a SqlitePool,
    pub session_id: i64,
    /// "all" | "entries"
    pub scope_mode: &'a str,
    /// scope_mode="entries" のときの対象 entry_id 集合（"all" のときは無視）
    pub scope_entry_ids: &'a [i64],
    /// MCP クライアント（`mcp_*` ツールのルーティング用）。テスト等で無い場合は None。
    pub mcp: Option<&'a crate::mcp::McpManager>,
    /// アプリデータディレクトリ（添付ファイルの相対パス解決用。OCR で使用）。
    pub app_data_dir: &'a Path,
}

#[derive(Debug)]
pub enum ToolError {
    UnknownTool(String),
    InvalidArguments(String),
    Db(sqlx::Error),
    /// ツールは見つかったが実行が論理的に失敗した（MCP 呼び出しの失敗など）。
    Execution(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::UnknownTool(n) => write!(f, "unknown tool: {}", n),
            ToolError::InvalidArguments(m) => write!(f, "invalid arguments: {}", m),
            ToolError::Db(e) => write!(f, "db error: {}", e),
            ToolError::Execution(m) => write!(f, "execution error: {}", m),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<sqlx::Error> for ToolError {
    fn from(e: sqlx::Error) -> Self {
        ToolError::Db(e)
    }
}

/// LLM に提示する全（組み込み）ツールの定義。MCP ツールは呼び出し側でこれに追加する。
pub fn all_tool_specs() -> Vec<ToolSpec> {
    let mut specs = search::specs();
    specs.extend(mutate::specs());
    specs.extend(ocr::specs());
    specs
}

/// `ToolCallSpec` を実行し、LLM に返すツール結果テキストを得る。
pub async fn execute_tool(ctx: &ToolContext<'_>, call: &ToolCallSpec) -> Result<String, ToolError> {
    // MCP ツールは外部サーバーへルーティングする。
    if call.tool_name.starts_with("mcp_") {
        return match ctx.mcp {
            Some(mcp) => mcp
                .call(&call.tool_name, &call.arguments)
                .await
                .map_err(|e| ToolError::Execution(e.to_string())),
            None => Err(ToolError::UnknownTool(call.tool_name.clone())),
        };
    }
    if let Some(r) = search::try_execute(ctx, call).await {
        return r;
    }
    if let Some(r) = mutate::try_execute(ctx, call).await {
        return r;
    }
    if let Some(r) = ocr::try_execute(ctx, call).await {
        return r;
    }
    Err(ToolError::UnknownTool(call.tool_name.clone()))
}
