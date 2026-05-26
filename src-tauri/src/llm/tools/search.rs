//! read 系ツール（#8 で実装）: fulltext_search / get_entry / list_collections / list_tags。
//!
//! 契約は `super`（tools/mod.rs）参照。`specs()` で LLM 向け定義を、
//! `try_execute()` で実行を提供する。検索系は `ToolContext` の scope を尊重すること。

use super::{ToolContext, ToolError};
use crate::llm::{ToolCallSpec, ToolSpec};

/// read 系ツールの定義一覧。
pub fn specs() -> Vec<ToolSpec> {
    // TODO(#8)
    vec![]
}

/// このモジュールが `call.tool_name` を扱うなら `Some(結果)`、扱わなければ `None`。
pub async fn try_execute(
    _ctx: &ToolContext<'_>,
    _call: &ToolCallSpec,
) -> Option<Result<String, ToolError>> {
    // TODO(#8)
    None
}
