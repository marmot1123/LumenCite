//! ツール呼び出しの自動承認判定。
//!
//! # ポリシー（v0.2.0）
//!
//! - **READ 系** (`fulltext_search`, `get_entry`, `list_*` で始まる名前):
//!   常に自動承認。whitelist で無効化不可。
//!
//! - **ALWAYS-CONFIRM** (`delete_*` で始まる名前, `mcp_` で始まる名前):
//!   常に要承認。whitelist でも override 不可。
//!
//! - **DEFAULT-AUTO write 系** (`add_tag`, `update_notes`, `attach_ocr_text`, `add_to_collection`):
//!   既定 `true`（自動承認）。whitelist でそのツール名を `false` にすることで
//!   都度承認に切り替え可能。
//!
//! - **CONFIRM-BY-DEFAULT write 系** (`create_entry`, `update_entry`):
//!   既定 `false`（要承認）。whitelist でそのツール名を `true` にすることで
//!   自動承認に切り替え可能。
//!
//! - **未知のツール**: 安全のため `false`（要承認）。
//!
//! # `whitelist_json` フォーマット
//!
//! `whitelist_json` は `Option<&str>` で、settings の `chat.tool_whitelist` に
//! 保存された JSON 文字列を渡す。フォーマットはツール名をキー、`bool` を値とする
//! JSON オブジェクト。例:
//!
//! ```json
//! {"add_tag": false, "create_entry": true}
//! ```
//!
//! - `None` または parse 失敗の場合は既定ポリシーを使う（パニックしない）。
//! - ALWAYS-TRUE (read 系) および ALWAYS-CONFIRM (delete_*/mcp_*) はホワイトリストで
//!   変更できない。

use std::collections::HashMap;

/// `tool_name` が自動承認（ユーザー確認不要）かどうかを返す純粋関数。
///
/// 詳細は当モジュールのドキュメントを参照。
pub fn should_auto_approve(tool_name: &str, whitelist_json: Option<&str>) -> bool {
    // --- ALWAYS-TRUE: read 系 ---
    if tool_name == "fulltext_search"
        || tool_name == "get_entry"
        || tool_name.starts_with("list_")
    {
        return true;
    }

    // --- ALWAYS-CONFIRM: delete_* と mcp_* ---
    if tool_name.starts_with("delete_") || tool_name.starts_with("mcp_") {
        return false;
    }

    // whitelist を parse する（失敗は None 扱い）
    let whitelist: Option<HashMap<String, bool>> = whitelist_json.and_then(|s| {
        serde_json::from_str(s).ok()
    });

    // --- DEFAULT-AUTO write 系 ---
    let default_auto = matches!(
        tool_name,
        "add_tag" | "update_notes" | "attach_ocr_text" | "add_to_collection"
    );
    if default_auto {
        // whitelist の値が false なら都度承認に切り替え
        if let Some(ref wl) = whitelist {
            if let Some(&v) = wl.get(tool_name) {
                return v;
            }
        }
        return true;
    }

    // --- CONFIRM-BY-DEFAULT write 系 ---
    let confirm_by_default = matches!(tool_name, "create_entry" | "update_entry");
    if confirm_by_default {
        // whitelist の値が true なら自動承認に切り替え
        if let Some(ref wl) = whitelist {
            if let Some(&v) = wl.get(tool_name) {
                return v;
            }
        }
        return false;
    }

    // --- 未知のツール ---
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── READ 系: always true ────────────────────────────────────────────────

    #[test]
    fn fulltext_search_is_always_approved() {
        assert!(should_auto_approve("fulltext_search", None));
    }

    #[test]
    fn get_entry_is_always_approved() {
        assert!(should_auto_approve("get_entry", None));
    }

    #[test]
    fn list_prefixed_is_always_approved() {
        assert!(should_auto_approve("list_tags", None));
        assert!(should_auto_approve("list_collections", None));
        assert!(should_auto_approve("list_anything_else", None));
    }

    #[test]
    fn read_tools_ignore_whitelist_false() {
        // whitelist で false を指定しても read 系は常に true
        let wl = r#"{"fulltext_search": false, "get_entry": false, "list_tags": false}"#;
        assert!(should_auto_approve("fulltext_search", Some(wl)));
        assert!(should_auto_approve("get_entry", Some(wl)));
        assert!(should_auto_approve("list_tags", Some(wl)));
    }

    // ── ALWAYS-CONFIRM: delete_* と mcp_* ──────────────────────────────────

    #[test]
    fn delete_entry_is_always_confirm() {
        assert!(!should_auto_approve("delete_entry", None));
    }

    #[test]
    fn delete_prefix_is_always_confirm() {
        assert!(!should_auto_approve("delete_tag", None));
        assert!(!should_auto_approve("delete_collection", None));
    }

    #[test]
    fn mcp_tool_is_always_confirm() {
        assert!(!should_auto_approve("mcp_obsidian_append_note", None));
        assert!(!should_auto_approve("mcp_github_create_issue", None));
    }

    #[test]
    fn delete_tools_ignore_whitelist_true() {
        // whitelist で true を指定しても delete_*/mcp_* は常に false
        let wl = r#"{"delete_entry": true, "mcp_obsidian_append_note": true}"#;
        assert!(!should_auto_approve("delete_entry", Some(wl)));
        assert!(!should_auto_approve("mcp_obsidian_append_note", Some(wl)));
    }

    // ── DEFAULT-AUTO write 系 ───────────────────────────────────────────────

    #[test]
    fn add_tag_defaults_to_auto() {
        assert!(should_auto_approve("add_tag", None));
    }

    #[test]
    fn update_notes_defaults_to_auto() {
        assert!(should_auto_approve("update_notes", None));
    }

    #[test]
    fn attach_ocr_text_defaults_to_auto() {
        assert!(should_auto_approve("attach_ocr_text", None));
    }

    #[test]
    fn add_to_collection_defaults_to_auto() {
        assert!(should_auto_approve("add_to_collection", None));
    }

    #[test]
    fn default_auto_can_be_flipped_to_false_via_whitelist() {
        let wl = r#"{"add_tag": false}"#;
        assert!(!should_auto_approve("add_tag", Some(wl)));
        // others unchanged
        assert!(should_auto_approve("update_notes", Some(wl)));
    }

    #[test]
    fn default_auto_whitelist_true_stays_true() {
        let wl = r#"{"add_tag": true}"#;
        assert!(should_auto_approve("add_tag", Some(wl)));
    }

    // ── CONFIRM-BY-DEFAULT write 系 ─────────────────────────────────────────

    #[test]
    fn create_entry_defaults_to_confirm() {
        assert!(!should_auto_approve("create_entry", None));
    }

    #[test]
    fn update_entry_defaults_to_confirm() {
        assert!(!should_auto_approve("update_entry", None));
    }

    #[test]
    fn confirm_by_default_can_be_flipped_to_true_via_whitelist() {
        let wl = r#"{"create_entry": true, "update_entry": true}"#;
        assert!(should_auto_approve("create_entry", Some(wl)));
        assert!(should_auto_approve("update_entry", Some(wl)));
    }

    #[test]
    fn confirm_by_default_whitelist_false_stays_false() {
        let wl = r#"{"create_entry": false}"#;
        assert!(!should_auto_approve("create_entry", Some(wl)));
    }

    // ── 未知のツール ────────────────────────────────────────────────────────

    #[test]
    fn unknown_tool_returns_false() {
        assert!(!should_auto_approve("some_unknown_tool", None));
        assert!(!should_auto_approve("", None));
    }

    #[test]
    fn unknown_tool_whitelist_true_still_false() {
        // 未知ツールは whitelist で true を指定しても false のまま
        let wl = r#"{"some_unknown_tool": true}"#;
        assert!(!should_auto_approve("some_unknown_tool", Some(wl)));
    }

    // ── whitelist の形式エラー ──────────────────────────────────────────────

    #[test]
    fn unparseable_whitelist_uses_defaults() {
        // 壊れた JSON は None と同じ扱い（パニックしない）
        assert!(should_auto_approve("add_tag", Some("not-valid-json")));
        assert!(!should_auto_approve("create_entry", Some("not-valid-json")));
    }

    #[test]
    fn none_whitelist_uses_defaults() {
        assert!(should_auto_approve("add_tag", None));
        assert!(!should_auto_approve("create_entry", None));
    }

    #[test]
    fn empty_whitelist_object_uses_defaults() {
        let wl = "{}";
        assert!(should_auto_approve("add_tag", Some(wl)));
        assert!(!should_auto_approve("create_entry", Some(wl)));
    }

    // ── 複数ツールの混合ホワイトリスト ─────────────────────────────────────

    #[test]
    fn mixed_whitelist_applies_per_tool() {
        let wl = r#"{"add_tag": false, "create_entry": true, "update_entry": false}"#;
        // default-auto flipped to false
        assert!(!should_auto_approve("add_tag", Some(wl)));
        // unaffected default-auto
        assert!(should_auto_approve("update_notes", Some(wl)));
        // confirm-by-default flipped to true
        assert!(should_auto_approve("create_entry", Some(wl)));
        // confirm-by-default explicitly false (same as default)
        assert!(!should_auto_approve("update_entry", Some(wl)));
        // read still always true
        assert!(should_auto_approve("get_entry", Some(wl)));
        // delete still always false
        assert!(!should_auto_approve("delete_entry", Some(wl)));
    }
}
