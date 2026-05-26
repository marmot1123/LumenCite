//! write 系ツール（#9 で実装）: add_tag / update_notes / add_to_collection /
//! create_entry / update_entry / delete_entry。
//!
//! 契約は `super`（tools/mod.rs）参照。`specs()` で LLM 向け定義を、
//! `try_execute()` で実行を提供する。承認可否の判定は `super::approval` を参照。

use super::{ToolContext, ToolError};
use crate::llm::{ToolCallSpec, ToolSpec};

/// write 系ツールの定義一覧。
pub fn specs() -> Vec<ToolSpec> {
    // TODO(#9)
    vec![]
}

/// このモジュールが `call.tool_name` を扱うなら `Some(結果)`、扱わなければ `None`。
pub async fn try_execute(
    _ctx: &ToolContext<'_>,
    _call: &ToolCallSpec,
) -> Option<Result<String, ToolError>> {
    // TODO(#9)
    None
}
