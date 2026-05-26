//! ツール呼び出しの自動承認判定（#9 で実装）。
//!
//! ポリシー（v0.2.0）:
//! - read 系（`fulltext_search` / `get_entry` / `list_*`）: 常に自動承認
//! - `add_tag` / `update_notes` / `attach_ocr_text` / `add_to_collection`: 既定で自動（whitelist で都度承認に変更可）
//! - `create_entry` / `update_entry`: 都度承認
//! - `delete_*` / MCP の write 系: 常時承認（whitelist で上書き不可）
//!
//! `whitelist_json` は settings の `chat.tool_whitelist`（None なら既定ポリシー）。

/// `tool_name` が自動承認されるか。
pub fn should_auto_approve(_tool_name: &str, _whitelist_json: Option<&str>) -> bool {
    // TODO(#9)
    false
}
