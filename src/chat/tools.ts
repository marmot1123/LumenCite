// チャットのツール名の分類。**backend の `llm/tools/approval.rs` と一致させること。**
//
// 以前はこの分類が ToolCallCard（カードの色分け）と store（ライブラリ再読込の判定）に
// 別々に書かれており、どちらも「read の allowlist に無ければ write」を既定にしていた。
// Phase 10b で read ツールが 2 → 11 に増えたので 1 か所に集約する。片方だけ足すと、
// read ツールがオレンジの write バッジで表示され、呼ぶたびに一覧が全再読込される。

/** 常に自動承認される read 系（`list_*` 接頭辞は別ルールで拾う）。 */
export const READ_ONLY_TOOLS = [
  "fulltext_search",
  "get_entry",
  // 文献本文の read 系（Phase 10b・backend の `llm::tools::document`）
  "get_fulltext",
  "get_document_structure",
  "get_document_blocks",
  "search_document_nodes",
  "get_node_relations",
  "get_symbol_definitions",
  "get_figures",
  "get_tables",
  "get_node_context",
] as const;

/**
 * 結果から根拠ノード（node_id + page）を取り出せるツール。
 * ツールカードに「PDF で見る」チップを出すかの判定に使う。
 * backend の `llm::tools::document::provenance_refs` が実際に扱う集合と一致させること。
 */
export const REF_BEARING_TOOLS = [
  "search_document_nodes",
  "get_document_blocks",
  "get_figures",
  "get_node_context",
] as const;

/** read 系（ライブラリを一切書き換えない）か。 */
export function isReadOnlyTool(name: string): boolean {
  return (READ_ONLY_TOOLS as readonly string[]).includes(name) || name.startsWith("list_");
}

/** 根拠チップを出しうるツールか。 */
export function isRefBearingTool(name: string): boolean {
  return (REF_BEARING_TOOLS as readonly string[]).includes(name);
}
