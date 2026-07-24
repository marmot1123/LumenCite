//! LumenCite を MCP **サーバー**として公開する。
//!
//! これは外部 MCP サーバーへ接続する `mcp`（クライアント）とは逆向きで、
//! Claude Desktop / Claude Code などの MCP クライアントが LumenCite のライブラリを
//! ツール経由で参照・操作できるようにするもの。サーバー側では LLM を呼ばない（推論は
//! 接続元のサブスクリプション側が担う）ため、API キー等は不要。
//!
//! ## 範囲
//! - トランスポート: localhost にバインドする HTTP（JSON-RPC 2.0 / 単発 POST → JSON 応答）
//! - 認可: `Authorization: Bearer <token>`（インストールごとの token。キーチェーン保管）
//! - **read 系（常時公開）**: `search` モジュールの read ツール定義を流用（単一ソース）し、
//!   LaTeX 連携向けの `search_entries` / `resolve_citation_key` / `export_bibtex` を追加。
//! - **write 系（Phase 2・ゲート付き）**: `mcp_server.write_enabled`（既定 false）が有効なときだけ
//!   `add_tag` / `update_notes` / `add_to_collection` / `create_entry` / `update_entry` を公開する。
//!   承認 UI が無いためサーバー側でこのゲートを enforce する。**破壊系 `delete_entry` は常に非公開**。
//!   write 成功時は監査ログに記録し、`.bib` 同期キックと `entries-changed` イベントを発火する。
//!
//! プロトコルのディスパッチ（[`handle_rpc`]）はトランスポート非依存で、HTTP を介さず
//! 単体テストできる（副作用＝`.bib` 同期/UI イベントは HTTP 層が `RpcOutcome.mutated` を見て行う）。

pub mod clipper;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::SqlitePool;
use tokio::sync::mpsc::UnboundedSender;

use crate::llm::tools::{mutate, search, ToolContext, ToolError};
use crate::llm::ToolCallSpec;

/// MCP プロトコルバージョン（クライアント側 `mcp` と揃える）。
const PROTOCOL_VERSION: &str = "2024-11-05";

/// 設定が無いときの既定バインドポート。
pub const DEFAULT_PORT: u16 = 3917;

/// `search` モジュールから流用して公開する read ツール名。
const SHARED_READ_TOOLS: &[&str] = &[
    "fulltext_search",
    "get_entry",
    "list_collections",
    "list_tags",
];

/// `mcp_server.write_enabled` が有効なときだけ公開する write ツール名。
/// `mutate` モジュールの定義を流用するが、**破壊系 `delete_entry` は意図的に含めない**
/// （許可リスト外なので `tools/call` でも到達不可）。
const WRITE_TOOLS: &[&str] = &[
    "add_tag",
    "update_notes",
    "add_to_collection",
    "create_entry",
    "update_entry",
];

/// `handle_rpc` の結果。`response` は JSON-RPC 応答（通知なら None）、`mutated` は
/// write が成功したかどうか（HTTP 層が `.bib` 同期 / UI イベント発火の判断に使う）。
pub struct RpcOutcome {
    pub response: Option<Value>,
    pub mutated: bool,
}

/// `mcp_server.write_enabled` の現在値を読む（リクエスト毎に評価し、トグル変更を即反映）。
async fn write_enabled(pool: &SqlitePool) -> bool {
    crate::db::settings::get_setting(pool, crate::db::settings::MCP_SERVER_WRITE_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
}

// ─── ツール定義（tools/list） ────────────────────────────────────────────────

/// 公開するツールの MCP 形式定義（`{name, description, inputSchema}`）。
/// `write_on` が true のときは write 系（`WRITE_TOOLS`）も含める。
fn tool_specs(write_on: bool) -> Vec<Value> {
    // 既存チャットの read 系定義を流用する（定義の二重管理を避ける単一ソース）。
    let mut tools: Vec<Value> = search::specs()
        .into_iter()
        .filter(|s| SHARED_READ_TOOLS.contains(&s.name.as_str()))
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "inputSchema": s.parameters,
            })
        })
        .collect();

    // MCP 専用の read ツール（LaTeX ワークフロー向け）。
    tools.push(json!({
        "name": "search_entries",
        "description": "Search library entries by metadata (title, authors, tags, abstract, \
            identifiers, year) using the trigram FTS index. Returns lightweight entry summaries \
            ranked by relevance.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (space-separated terms are ANDed)." },
                "collection_id": { "type": "integer", "description": "Restrict the search to a collection id." },
                "tag_id": { "type": "integer", "description": "Restrict the search to a tag id." }
            },
            "required": ["query"]
        }
    }));
    tools.push(json!({
        "name": "resolve_citation_key",
        "description": "Return the BibTeX citation key actually used in LaTeX \\cite{} / .bib \
            exports for an entry — the user-pinned key, or an auto-generated first-author+year key \
            when none is pinned.",
        "inputSchema": {
            "type": "object",
            "properties": { "entry_id": { "type": "integer", "description": "Entry id." } },
            "required": ["entry_id"]
        }
    }));
    tools.push(json!({
        "name": "export_bibtex",
        "description": "Export entries as BibTeX. Pass citation_keys to export exactly the entries \
            for a set of LaTeX \\cite{} keys (the best way to build a paper's refs.bib): keys keep \
            the exact form used across the whole library — including disambiguating suffixes like \
            'smith2020a' — and unresolved keys are reported back in `missing`. Or pass entry_ids to \
            export specific entries by id, or omit both to export the whole library (trash \
            excluded). With citation_keys the result is a JSON object {bibtex, found, missing}; \
            otherwise the raw .bib text.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "citation_keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Citation keys (as in \\cite{}) to export; preserves library-wide keys and reports missing ones."
                },
                "entry_ids": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Entry ids to export; omit for the whole library."
                }
            }
        }
    }));
    tools.push(json!({
        "name": "find_entries_by_citation_keys",
        "description": "Resolve one or more BibTeX/LaTeX \\cite{} citation keys to library entries. \
            For each key, reports whether it was found and, if so, the matching entry (entry_id, \
            title, year, authors). Use this to bridge from \\cite keys in a .tex file to library \
            entry ids — users think in citation keys, not numeric ids. The keys are matched exactly \
            as they appear in .bib / \\cite{} exports (pinned or auto-generated, with suffixes).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "citation_keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Citation keys to resolve (as they appear in \\cite{})."
                }
            },
            "required": ["citation_keys"]
        }
    }));
    tools.push(json!({
        "name": "get_fulltext",
        "description": "Return the extracted full text of a library entry's indexed PDF, by \
            entry_id or citation_key. Use this to actually read and summarise a specific paper — \
            `get_entry` only returns metadata (abstract / notes), which are often empty. Returns \
            {entry_id, indexed, total_pages, truncated, next_page, text}. If the entry has no \
            attached/indexed PDF, `indexed` is false and there is no text — say so plainly and do \
            NOT answer from general knowledge. Long papers are paginated: pass `page_start` (from a \
            previous `next_page`) to keep reading, or raise `max_chars`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key (as in \\cite{}); alternative to entry_id." },
                "max_chars": { "type": "integer", "description": "Max characters to return this call (default 24000)." },
                "page_start": { "type": "integer", "description": "1-based PDF page to start from, for continuing a long paper (default 1)." }
            }
        }
    }));

    // LCIR（機械可読中間形式）の read ツール（Phase 3.5）。実験フラグ lcir.enabled で
    // 構築された論文だけが対象。未構築なら has_lcir=false を返す（get_fulltext に退避可能）。
    tools.push(json!({
        "name": "get_document_structure",
        "description": "Return the logical structure (LCIR) of a paper — its section outline, \
            block-type counts, and abstract — by entry_id or citation_key. Unlike get_fulltext \
            (flat page text), this exposes headings/sections with their numbers and reports how \
            many paragraphs, display equations, captions and bibliography entries were found. Two \
            representations can coexist per paper: \"tex\" (parsed from the arXiv TeX source — \
            exact structure, exact LaTeX math, but no page numbers) and \"pdf\" (heuristically \
            recovered from the PDF text layer — approximate, with pages and bounding boxes). By \
            default the best available is used (tex over pdf); pass `source` to switch explicitly. \
            Returns {has_lcir, source, available_sources, page_count (null for tex), block_count, \
            outline:[{kind, section_number, level, text, page}], counts, abstract}. If has_lcir is \
            false nothing is built (build it in the app) — fall back to get_fulltext. Then use \
            get_document_blocks to read the structured text or equations, and \
            search_document_nodes to locate content.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key (as in \\cite{}); alternative to entry_id." },
                "source": { "type": "string", "enum": ["tex", "pdf"], "description": "Force a representation: \"tex\" (arXiv TeX source; exact LaTeX) or \"pdf\" (PDF text layer; pages/bbox). Omit for the best available (tex preferred)." }
            }
        }
    }));
    tools.push(json!({
        "name": "get_document_blocks",
        "description": "Read a paper's content as structure-tagged blocks (LCIR) in reading order — \
            paragraphs, headings, captions and display equations — by entry_id or citation_key. \
            Better than get_fulltext for structured reading. Filter with `kinds` (e.g. \
            [\"display_math\"] to list just the equations, or [\"section\",\"paragraph\"] to read \
            prose). Math depends on the representation: blocks served from the arXiv TeX source \
            (source \"tex\", preferred when built) carry the EXACT LaTeX in `latex`; blocks from \
            the PDF (source \"pdf\") are surface-only Unicode text — approximate, no LaTeX. Pass \
            `source` to switch explicitly; `page` implies the pdf representation (tex has no \
            pages), so with `page` and no `source` the pdf version is used automatically. Block \
            indices are only valid within one source. Long documents are paginated: pass \
            block_start (from a previous next_block) or raise max_chars. Returns {has_lcir, \
            source, available_sources, total_blocks, returned, block_start, truncated, next_block, \
            blocks:[{index, kind, page, section_number?, equation_label?, latex?, text}]}.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Restrict to these block kinds (e.g. [\"display_math\"], [\"section\",\"paragraph\"]). Omit for all content blocks."
                },
                "page": { "type": "integer", "description": "Restrict to a single 1-based PDF page (pdf representation only; forces source \"pdf\" when source is omitted)." },
                "source": { "type": "string", "enum": ["tex", "pdf"], "description": "Force a representation: \"tex\" (exact LaTeX math, no pages) or \"pdf\" (pages/bbox, surface-only math). Omit for the best available (tex preferred)." },
                "block_start": { "type": "integer", "description": "0-based index into the (filtered) block list to start from, for continuing a long read (default 0)." },
                "max_chars": { "type": "integer", "description": "Max characters of block text to return this call (default 24000)." }
            }
        }
    }));
    tools.push(json!({
        "name": "search_document_nodes",
        "description": "Search the library at BLOCK granularity (paragraph / heading / caption / \
            display equation) using the LCIR node index — finer than fulltext_search, which is page \
            granularity. Each hit reports the entry, node_kind, page, a snippet, and the PDF \
            bounding box (bbox = [x, y, width, height] in PDF points, bottom-left origin) so the \
            exact block can be located/highlighted. Use this to pinpoint where a concept, term or \
            equation appears across papers. Only covers papers whose PDF-derived LCIR has been \
            built (TeX-derived text is not in this index; read it via get_document_blocks). Hit \
            pages refer to the pdf representation — follow up with get_document_blocks(page=...) \
            which uses the pdf source automatically. Returns {count, results:[{entry_id, title, \
            year, node_kind, page, snippet, bbox}]}. Short or CJK queries fall back to substring \
            matching.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (space-separated terms are ANDed)." },
                "collection_id": { "type": "integer", "description": "Restrict to a collection id." },
                "tag_id": { "type": "integer", "description": "Restrict to a tag id." }
            },
            "required": ["query"]
        }
    }));
    tools.push(json!({
        "name": "get_node_relations",
        "description": "Return the cross-reference graph (LCIR) of a paper — typed directed edges \
            between its blocks — by entry_id or citation_key. Edges are resolved from the source: \
            \"tex\" (from \\ref/\\eqref/\\cite matched against \\label and \\bibitem keys — high \
            confidence, origin tex_source) or \"pdf\" (from \"Theorem 2.3\"/\"Eq. (2.1)\" strings \
            matched against theorem/equation numbers — approximate, origin layout_model). tex is \
            preferred when built; pass `source` to switch. Relation types: cites, \
            refers_to_equation, refers_to_theorem, refers_to_figure, refers_to_table, \
            refers_to_section, refers_to, proves (proof → the theorem it proves), and caption_of \
            (a figure caption → its detected figure region, pdf only). Use it to \
            answer \"what does this proof prove\", \"what cites/uses equation (2.1)\", \"which \
            results does this section reference\". Filter with `relation_type` and/or `node_id` \
            (edges touching that block, either direction). Returns {has_lcir, source, \
            available_sources, count, counts_by_type, relations:[{relation_type, confidence, \
            origin, from:{node_id,kind,page,snippet}, to:{node_id,kind,page,snippet, \
            theorem_number?, equation_label?, section_number?, labels?}, metadata}]}. If has_lcir \
            is false nothing is built (build it in the app).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                "relation_type": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Restrict to these relation types (e.g. [\"proves\"], [\"cites\"], [\"refers_to_equation\",\"refers_to_theorem\"]). Omit for all."
                },
                "node_id": { "type": "integer", "description": "Only edges touching this node id (as from or to). Node ids come from get_document_blocks / search_document_nodes." },
                "source": { "type": "string", "enum": ["tex", "pdf"], "description": "Force a representation: \"tex\" (\\ref/\\cite resolution) or \"pdf\" (number-string resolution). Omit for the best available (tex preferred)." },
                "max_relations": { "type": "integer", "description": "Max edges to return (default 300)." }
            }
        }
    }));
    tools.push(json!({
        "name": "get_symbol_definitions",
        "description": "Return the notation/symbol definitions (LCIR) a paper introduces — by \
            entry_id or citation_key. Recognized heuristically from definition sentences in the \
            arXiv TeX source (\"let $U$ be ...\", \"define $H$ as ...\", \"denote by \
            $\\mathcal{H}$ ...\", \"$U := ...$\"), so this is **TeX-only** (PDF inline math cannot be \
            isolated reliably); returns empty for PDF-only entries. Each symbol carries its \
            surface_form (raw LaTeX like \"U\" or \"\\mathcal{H}\"), normalized_form, a \
            description extracted from the sentence, a best-effort symbol_type, the node where it \
            is defined (defined_at, for \"jump to definition\"), the enclosing section (scope), and \
            its occurrences in display equations. The surface/description text is verbatim from the \
            source but the definition ASSOCIATION is heuristic — hence a moderate confidence. \
            Use it to answer \"what is $U$ in this paper\", \"list the notation\", \"where is \
            $\\mathcal{H}$ defined\", \"which equations use $\\gamma$\". Filter with `symbol` \
            (exact surface) or `query` (substring over surface/normalized/description). Returns \
            {has_lcir, source, count, symbols:[{surface_form, normalized_form, description, \
            symbol_type, confidence, defined_at:{node_id,kind,snippet}, scope, occurrence_count, \
            occurrences:[{node_id, equation_label}]}]}.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                "symbol": { "type": "string", "description": "Exact surface_form to fetch (e.g. \"U\" or \"\\\\mathcal{H}\")." },
                "query": { "type": "string", "description": "Case-insensitive substring over surface_form / normalized_form / description." },
                "source": { "type": "string", "enum": ["tex", "pdf"], "description": "Force a representation. Symbols exist only for \"tex\"; omit for the best available (tex preferred)." },
                "max_symbols": { "type": "integer", "description": "Max symbols to return (default 200)." }
            }
        }
    }));
    tools.push(json!({
        "name": "get_figures",
        "description": "Return the detected figures (LCIR) of a paper — by entry_id or \
            citation_key. **PDF-only**: figure regions are detected from embedded raster images on \
            each page (origin layout_model, moderate confidence), so vector figures (TikZ/pgf, \
            common in math papers) legitimately yield zero figures — an empty list does NOT mean \
            the paper has no figures. Each figure carries its page and bbox ([x, y, width, height] \
            in PDF points, bottom-left origin), the figure number when a nearby \"Figure N\" \
            caption was paired (caption_of edge), the caption text, and its stored assets \
            (page-crop PNGs). Asset relative_path is a path under the app data directory as \
            METADATA — the file's existence is not guaranteed and no image bytes are returned. \
            Use it to answer \"what figures does this paper have\", \"what does Figure 2 show\" \
            (caption text), \"where is Figure 2 on the page\" (page + bbox). Returns {has_lcir, \
            source, available_sources, count, figures:[{node_id, page, bbox, figure_number?, \
            caption:{node_id, text}?, assets:[{role, relative_path, mime_type, width, height, \
            size_bytes}]}]}. If has_lcir is false no PDF version is built (build it in the app).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                "max_figures": { "type": "integer", "description": "Max figures to return (default 100)." }
            }
        }
    }));

    tools.push(json!({
        "name": "get_tables",
        "description": "Return the structured tables (LCIR) of a paper — by entry_id or \
            citation_key. **TeX-only**: cells are parsed from tabular/tabular*/tabularx \
            environments in the arXiv TeX source (origin tex_source), so PDF-only entries return \
            has_lcir:false and papers whose tables use longtable/tabu or nested layouts \
            legitimately yield zero or fewer tables — an empty list does NOT mean the paper has \
            no tables. Each table carries its caption (via the caption_of edge), the verbatim \
            LaTeX column_spec, n_rows/n_columns, per-column alignments (letters l/c/r/p/m/b/X, \
            present only when the column spec was fully parsed), and rows as \
            {cells:[{text, colspan?, rowspan?}], rule_above?} where cell text keeps LaTeX \
            verbatim (inline math as $..$). rule_above records a full-width rule above the row \
            (a fact from the source; header detection is NOT performed). Use it to answer \
            \"what tables does this paper have\", \"read Table 2's cells\", \"which column \
            holds the masses\". Returns {has_lcir, source, available_sources, count, truncated, \
            tables:[{node_id, caption:{node_id, text}?, column_spec, n_columns, n_rows, \
            alignments?, rows}]}.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "Entry id." },
                "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                "max_tables": { "type": "integer", "description": "Max tables to return (default 20)." },
                "max_chars": { "type": "integer", "description": "Approximate budget over cell text (default 24000); further tables are truncated." }
            }
        }
    }));

    // write 系（Phase 2・ゲート有効時のみ）。`mutate` の定義を流用し、許可リスト
    // （`WRITE_TOOLS`）に絞る。delete_entry はリストに無いので公開されない。
    if write_on {
        for s in mutate::specs() {
            if WRITE_TOOLS.contains(&s.name.as_str()) {
                tools.push(json!({
                    "name": s.name,
                    "description": s.description,
                    "inputSchema": s.parameters,
                }));
            }
        }
    }

    tools
}

// ─── JSON-RPC ディスパッチ（トランスポート非依存） ──────────────────────────

/// JSON-RPC リクエスト 1 件を処理する。通知（`id` 無し）の場合は `response: None`。
/// `mutated` が true なら write が成功したので、呼び出し側が `.bib` 同期 / UI イベントを発火する。
///
/// write の可否は `mcp_server.write_enabled` 設定から評価する（公開サーバー用ゲート）。
pub async fn handle_rpc(pool: &SqlitePool, app_data_dir: &Path, req: &Value) -> RpcOutcome {
    let write_on = write_enabled(pool).await;
    handle_rpc_with_write(pool, app_data_dir, write_on, req).await
}

/// `handle_rpc` の write_on 明示版。CLI の**直接 DB 書込経路**は、公開サーバー用の
/// `mcp_server.write_enabled` 設定とは独立に（CLI 側でサーバー到達性ゲートを済ませた上で）
/// `write_on = true` を渡して同じツール実装・監査ログ・`mutated` フラグを再利用する。
pub async fn handle_rpc_with_write(
    pool: &SqlitePool,
    app_data_dir: &Path,
    write_on: bool,
    req: &Value,
) -> RpcOutcome {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    // 通知（id 無し）には応答しない（JSON-RPC 2.0）。
    let Some(id) = req.get("id").cloned() else {
        return RpcOutcome { response: None, mutated: false };
    };
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    let (resp, mutated) = match method {
        "initialize" => (
            ok(id, json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "LumenCite", "version": env!("CARGO_PKG_VERSION") }
            })),
            false,
        ),
        "ping" => (ok(id, json!({})), false),
        "tools/list" => (ok(id, json!({ "tools": tool_specs(write_on) })), false),
        "tools/call" => handle_tools_call(pool, app_data_dir, write_on, id, &params).await,
        other => (err(id, -32601, &format!("method not found: {other}")), false),
    };
    RpcOutcome { response: Some(resp), mutated }
}

fn ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// `(応答, mutated)` を返す。`mutated` は write が成功した場合のみ true。
async fn handle_tools_call(
    pool: &SqlitePool,
    app_data_dir: &Path,
    write_on: bool,
    id: Value,
    params: &Value,
) -> (Value, bool) {
    let Some(name) = params.get("name").and_then(|n| n.as_str()) else {
        return (err(id, -32602, "missing tool name"), false);
    };
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let is_write = WRITE_TOOLS.contains(&name);
    // write ツールだがゲートが無効 → 実行せず isError で拒否。
    if is_write && !write_on {
        return (
            ok(id, tool_content(format!("write tools are disabled on this MCP server: {name}"), true)),
            false,
        );
    }

    let result = exec_tool(pool, app_data_dir, write_on, name, args.clone()).await;

    // write は成否に関わらず監査ログに記録する（read は記録しない）。
    if is_write {
        let (summary, is_err) = match &result {
            Ok(s) => (s.clone(), false),
            Err(e) => (e.to_string(), true),
        };
        let args_str = serde_json::to_string(&args).unwrap_or_default();
        let _ = crate::db::mcp_audit::record(pool, name, &args_str, &summary, is_err).await;
    }

    match result {
        Ok(text) => (ok(id, tool_content(text, false)), is_write),
        // ツール実行エラーは JSON-RPC エラーではなく isError 結果として返す（MCP 慣例）。
        Err(ToolError::UnknownTool(_)) => (
            ok(id, tool_content(format!("unknown or unavailable tool: {name}"), true)),
            false,
        ),
        Err(e) => (ok(id, tool_content(e.to_string(), true)), false),
    }
}

fn tool_content(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

// ─── ツール実行 ──────────────────────────────────────────────────────────────

fn mcp_ctx<'a>(pool: &'a SqlitePool, app_data_dir: &'a Path) -> ToolContext<'a> {
    // MCP サーバーは scope を持たないため "all" 固定。外部 mcp_* ツールも使わない。
    ToolContext {
        pool,
        session_id: 0,
        scope_mode: "all",
        scope_entry_ids: &[],
        mcp: None,
        app_data_dir,
    }
}

async fn exec_tool(
    pool: &SqlitePool,
    app_data_dir: &Path,
    write_on: bool,
    name: &str,
    args: Value,
) -> Result<String, ToolError> {
    // 既存チャットの read 系をそのまま流用。
    if SHARED_READ_TOOLS.contains(&name) {
        let call = ToolCallSpec {
            call_id: "mcp-server".to_string(),
            tool_name: name.to_string(),
            arguments: args,
        };
        return search::try_execute(&mcp_ctx(pool, app_data_dir), &call)
            .await
            .unwrap_or_else(|| Err(ToolError::UnknownTool(name.to_string())));
    }

    // write 系（ゲートは呼び出し側で確認済みだが、二重に write_on を確認する）。
    if write_on && WRITE_TOOLS.contains(&name) {
        let call = ToolCallSpec {
            call_id: "mcp-server".to_string(),
            tool_name: name.to_string(),
            arguments: args,
        };
        return mutate::try_execute(&mcp_ctx(pool, app_data_dir), &call)
            .await
            .unwrap_or_else(|| Err(ToolError::UnknownTool(name.to_string())));
    }

    match name {
        "search_entries" => exec_search_entries(pool, &args).await,
        "resolve_citation_key" => exec_resolve_citation_key(pool, &args).await,
        "export_bibtex" => exec_export_bibtex(pool, &args).await,
        "find_entries_by_citation_keys" => exec_find_entries_by_citation_keys(pool, &args).await,
        "get_fulltext" => exec_get_fulltext(pool, &args).await,
        "get_document_structure" => exec_get_document_structure(pool, &args).await,
        "get_document_blocks" => exec_get_document_blocks(pool, &args).await,
        "search_document_nodes" => exec_search_document_nodes(pool, &args).await,
        "get_node_relations" => exec_get_node_relations(pool, &args).await,
        "get_symbol_definitions" => exec_get_symbol_definitions(pool, &args).await,
        "get_figures" => exec_get_figures(pool, &args).await,
        "get_tables" => exec_get_tables(pool, &args).await,
        // それ以外（delete_entry / ocr_* / 無効化中の write 等）は非公開。
        _ => Err(ToolError::UnknownTool(name.to_string())),
    }
}

async fn exec_search_entries(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArguments("missing required argument: query".to_string()))?;
    let collection_id = args.get("collection_id").and_then(|v| v.as_i64());
    let tag_id = args.get("tag_id").and_then(|v| v.as_i64());

    let results = crate::db::entries::search_entries(pool, query, collection_id, tag_id).await?;
    let items: Vec<Value> = results
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "title": e.title,
                "year": e.year,
                "entry_type": e.entry_type,
                "authors": e.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();

    Ok(serde_json::to_string(&json!({ "count": items.len(), "results": items })).unwrap_or_default())
}

async fn exec_resolve_citation_key(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = args
        .get("entry_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            ToolError::InvalidArguments("missing required argument: entry_id".to_string())
        })?;
    crate::bibtex::resolve_citation_key(pool, entry_id)
        .await
        .map_err(ToolError::Execution)
}

async fn exec_export_bibtex(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    // citation_keys が渡されたら「\cite キー → refs.bib」経路。全ライブラリの確定キーを
    // 維持し（サブセット再 dedup をしない）、未解決キーは `missing` に載せて返す。
    if let Some(arr) = args.get("citation_keys").and_then(|v| v.as_array()) {
        let keys: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
        let res = crate::bibtex::export_bibtex_by_keys(pool, &keys)
            .await
            .map_err(ToolError::Execution)?;
        return Ok(serde_json::to_string(&json!({
            "bibtex": res.bibtex,
            "found": res.found,
            "missing": res.missing,
        }))
        .unwrap_or_default());
    }

    let entry_ids = args
        .get("entry_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>());
    crate::bibtex::export_bibtex(pool, entry_ids)
        .await
        .map_err(ToolError::Execution)
}

async fn exec_find_entries_by_citation_keys(
    pool: &SqlitePool,
    args: &Value,
) -> Result<String, ToolError> {
    let keys: Vec<String> = args
        .get("citation_keys")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .ok_or_else(|| {
            ToolError::InvalidArguments(
                "missing required argument: citation_keys (array of strings)".to_string(),
            )
        })?;

    let index = crate::bibtex::citation_key_index(pool)
        .await
        .map_err(ToolError::Execution)?;
    let key_to_id: std::collections::HashMap<&str, i64> =
        index.iter().map(|(k, id)| (k.as_str(), *id)).collect();

    let mut results = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for k in &keys {
        if !seen.insert(k.as_str()) {
            continue;
        }
        match key_to_id.get(k.as_str()) {
            Some(&id) => {
                let d = crate::db::entries::get_entry(pool, id).await?;
                results.push(json!({
                    "citation_key": k,
                    "found": true,
                    "entry_id": id,
                    "title": d.title,
                    "year": d.year,
                    "authors": d.authors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                }));
            }
            None => results.push(json!({ "citation_key": k, "found": false })),
        }
    }

    Ok(serde_json::to_string(&json!({ "count": results.len(), "results": results }))
        .unwrap_or_default())
}

async fn exec_get_fulltext(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    // entry_id 優先。無ければ citation_key から逆引き。
    let entry_id = match args.get("entry_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => match args.get("citation_key").and_then(|v| v.as_str()) {
            Some(key) => match crate::bibtex::find_entry_id_by_citation_key(pool, key).await {
                Ok(Some(id)) => id,
                Ok(None) => {
                    return Ok(serde_json::to_string(&json!({
                        "indexed": false,
                        "message": format!("no entry found for citation key '{key}'")
                    }))
                    .unwrap_or_default())
                }
                Err(e) => return Err(ToolError::Execution(e)),
            },
            None => {
                return Err(ToolError::InvalidArguments(
                    "provide entry_id (integer) or citation_key (string)".to_string(),
                ))
            }
        },
    };

    let pages = crate::db::fulltext::get_entry_fulltext(pool, entry_id).await?;
    if pages.is_empty() {
        return Ok(serde_json::to_string(&json!({
            "entry_id": entry_id,
            "indexed": false,
            "message": "this entry has no indexed full text (no attached/indexed PDF)"
        }))
        .unwrap_or_default());
    }

    let total_pages = pages.len() as i64;
    let page_start = args.get("page_start").and_then(|v| v.as_i64()).unwrap_or(1).max(1);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_i64())
        .unwrap_or(24_000)
        .clamp(1_000, 200_000) as usize;

    // page_start 以降のページを、累計が max_chars に達するまでページ単位で連結する
    // （ページ途中では切らない）。入りきらなかった最初のページを next_page に載せて
    // 続き読みできるようにする。
    let mut text = String::new();
    let mut truncated = false;
    let mut next_page: Option<i64> = None;
    for (page, content) in pages.iter().filter(|(p, _)| *p >= page_start) {
        if text.chars().count() >= max_chars {
            next_page = Some(*page);
            truncated = true;
            break;
        }
        text.push_str(&format!("[page {page}]\n{content}\n\n"));
    }

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "indexed": true,
        "total_pages": total_pages,
        "returned_from_page": page_start,
        "truncated": truncated,
        "next_page": next_page,
        "text": text.trim_end(),
    }))
    .unwrap_or_default())
}

// ─── LCIR（機械可読中間形式）read ツール（Phase 3.5） ────────────────────────

/// entry_id 優先・無ければ citation_key から逆引き（get_fulltext と同じ規約）。
async fn resolve_entry_id(pool: &SqlitePool, args: &Value) -> Result<i64, ToolError> {
    if let Some(id) = args.get("entry_id").and_then(|v| v.as_i64()) {
        return Ok(id);
    }
    if let Some(key) = args.get("citation_key").and_then(|v| v.as_str()) {
        return match crate::bibtex::find_entry_id_by_citation_key(pool, key).await {
            Ok(Some(id)) => Ok(id),
            Ok(None) => Err(ToolError::InvalidArguments(format!(
                "no entry found for citation key '{key}'"
            ))),
            Err(e) => Err(ToolError::Execution(e)),
        };
    }
    Err(ToolError::InvalidArguments(
        "provide entry_id (integer) or citation_key (string)".to_string(),
    ))
}

/// MCP の `source` 引数（"tex"/"pdf"）→ extractor_name。
fn source_to_extractor(source: &str) -> Result<&'static str, ToolError> {
    crate::ingestion::source_to_extractor(source).map_err(ToolError::InvalidArguments)
}

/// extractor_name → MCP 応答の短い source 名。
fn short_source_name(extractor_name: &str) -> &str {
    crate::ingestion::short_source_name(extractor_name)
}

/// 併存する表現の列挙（`available_sources` 応答）。
fn sources_json(versions: &[crate::models::DocumentVersion]) -> Value {
    Value::Array(
        versions
            .iter()
            .map(|v| {
                json!({
                    "source": short_source_name(&v.extractor_name),
                    "attachment_id": v.attachment_id,
                    "extractor_name": v.extractor_name,
                    "extractor_version": v.extractor_version,
                })
            })
            .collect(),
    )
}

/// エントリの LCIR を読む。`source` 指定時はその抽出器の版に限定し、未指定なら
/// 優先度順（tex > pdfium）で最初に読めた版を返す。読めた/読めないに関わらず
/// 併存する版の一覧（`available_sources` 用）を返す — 無かったときの案内文を
/// 「実在する表現」に基づいて組み立てるため。
#[allow(clippy::type_complexity)]
async fn load_entry_lcir(
    pool: &SqlitePool,
    entry_id: i64,
    source: Option<&str>,
) -> Result<
    (
        Option<(i64, crate::document_ir::LcirDocument)>,
        Vec<crate::models::DocumentVersion>,
    ),
    ToolError,
> {
    let wanted: Option<&str> = match source {
        Some(s) => Some(source_to_extractor(s)?),
        None => None,
    };
    crate::ingestion::load_entry_lcir(pool, entry_id, wanted)
        .await
        .map_err(ToolError::Execution)
}

/// 本文つき論理ブロック（骨格の document/page/line は除く）。
fn is_content_block(kind: &str) -> bool {
    !matches!(kind, "document" | "page" | "line")
}

/// ノードの代表ページ（最初の source_fragment）。
fn node_page(n: &crate::document_ir::LcirNode) -> Option<i64> {
    n.source_fragments.first().map(|f| f.page)
}

fn no_lcir_response(entry_id: i64, source: Option<&str>) -> String {
    let message = match source {
        Some(s) => format!(
            "no built LCIR from source '{s}' for this entry. Omit `source` to use any available \
             representation, or download/build it in the app (arXiv entries can fetch the TeX \
             source from the detail panel)."
        ),
        None => "no built LCIR for this entry (enable and build LCIR in the app; arXiv entries \
            can also fetch the TeX source). Fall back to get_fulltext for flat page text."
            .to_string(),
    };
    serde_json::to_string(&json!({
        "entry_id": entry_id,
        "has_lcir": false,
        "message": message,
    }))
    .unwrap_or_default()
}

async fn exec_get_document_structure(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let source_arg = args.get("source").and_then(|v| v.as_str());
    let (loaded, versions) = load_entry_lcir(pool, entry_id, source_arg).await?;
    let Some((attachment_id, doc)) = loaded else {
        return Ok(no_lcir_response(entry_id, source_arg));
    };
    let is_tex = doc.source.extractor_name == crate::document_ir::schema::TEX_EXTRACTOR_NAME;

    let mut counts: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut outline: Vec<Value> = Vec::new();
    let mut abstract_parts: Vec<String> = Vec::new();
    let mut page_count = 0i64;
    let mut block_count = 0i64;
    for n in &doc.nodes {
        if n.kind == "page" {
            page_count += 1;
        }
        if !is_content_block(&n.kind) {
            continue;
        }
        block_count += 1;
        *counts.entry(n.kind.clone()).or_insert(0) += 1;
        match n.kind.as_str() {
            "section" | "subsection" | "heading" => {
                let sec = n
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("section_number"))
                    .and_then(|v| v.as_str());
                let level = n
                    .payload
                    .as_ref()
                    .and_then(|p| p.get("heading_level"))
                    .and_then(|v| v.as_i64());
                outline.push(json!({
                    "kind": n.kind,
                    "section_number": sec,
                    "level": level,
                    "text": n.plain_text,
                    "page": node_page(n),
                }));
            }
            "abstract" => {
                if let Some(t) = &n.plain_text {
                    abstract_parts.push(t.clone());
                }
            }
            _ => {}
        }
    }
    let abstract_text = if abstract_parts.is_empty() {
        None
    } else {
        Some(abstract_parts.join(" "))
    };

    // note と page_count は source 依存: TeX 版はページを持たない（page_count: null）。
    let note = if is_tex {
        "Parsed from the arXiv TeX source (origin=tex_source). Display equations carry exact \
         LaTeX; this representation has no page numbers or bounding boxes (use source=\"pdf\" \
         for page-anchored reading). Use get_document_blocks to read prose or equations."
    } else {
        "Structure is heuristically recovered from the PDF text layer (origin=layout_model, \
         per-node confidence). Equations are surface-only (no LaTeX). Use get_document_blocks to \
         read prose or equations, search_document_nodes to locate content."
    };
    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "attachment_id": attachment_id,
        "has_lcir": true,
        "source": short_source_name(&doc.source.extractor_name),
        "extractor_name": doc.source.extractor_name,
        "extractor_version": doc.source.extractor_version,
        "available_sources": sources_json(&versions),
        "page_count": if is_tex { Value::Null } else { json!(page_count) },
        "block_count": block_count,
        "outline": outline,
        "counts": counts,
        "abstract": abstract_text,
        "note": note,
    }))
    .unwrap_or_default())
}

async fn exec_get_document_blocks(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let source_arg = args.get("source").and_then(|v| v.as_str());
    let page_filter = args.get("page").and_then(|v| v.as_i64());

    // `page` は PDF 空間の概念（search_document_nodes のヒットも PDF 由来）なので、
    // source 未指定で page が来たら PDF 版へ自動フォールバックする。
    let effective_source = match (page_filter.is_some(), source_arg) {
        (true, None) => Some("pdf"),
        (_, s) => s,
    };
    let (loaded, versions) = load_entry_lcir(pool, entry_id, effective_source).await?;
    let has_tex = versions
        .iter()
        .any(|v| v.extractor_name == crate::document_ir::schema::TEX_EXTRACTOR_NAME);
    let has_pdf = versions
        .iter()
        .any(|v| v.extractor_name == crate::document_ir::schema::EXTRACTOR_NAME);
    let Some((attachment_id, doc)) = loaded else {
        // page 指定の自動 PDF フォールバックで PDF 版が無かった場合の案内は、
        // 実在する表現に基づいて出す（無い TeX 版を勧めない）。
        if page_filter.is_some() && source_arg.is_none() && has_tex {
            return Ok(serde_json::to_string(&json!({
                "entry_id": entry_id,
                "has_lcir": false,
                "available_sources": sources_json(&versions),
                "message": "page filtering needs a PDF-derived LCIR and none is built for this \
                    entry; omit `page` to read the TeX representation, or build the PDF LCIR in \
                    the app.",
            }))
            .unwrap_or_default());
        }
        return Ok(no_lcir_response(entry_id, source_arg));
    };
    let is_tex = doc.source.extractor_name == crate::document_ir::schema::TEX_EXTRACTOR_NAME;
    if is_tex && page_filter.is_some() {
        // 明示 source="tex" + page: 黙って 0 件を返すとエージェントが「中身が無い」と誤解する。
        let hint = if has_pdf {
            "the tex representation has no page mapping; omit `page` or use source=\"pdf\"."
        } else {
            "the tex representation has no page mapping and no PDF-derived LCIR is built; \
             omit `page` to read it."
        };
        return Ok(serde_json::to_string(&json!({
            "entry_id": entry_id,
            "attachment_id": attachment_id,
            "has_lcir": true,
            "source": "tex",
            "available_sources": sources_json(&versions),
            "total_blocks": 0,
            "returned": 0,
            "blocks": [],
            "message": hint,
        }))
        .unwrap_or_default());
    }

    // kinds フィルタ。
    let kind_filter: Option<Vec<String>> = args.get("kinds").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    });

    // 読み順の本文ブロック（load_lcir_document のノード順 = ページ→ordinal）。
    let blocks: Vec<&crate::document_ir::LcirNode> = doc
        .nodes
        .iter()
        .filter(|n| is_content_block(&n.kind))
        .filter(|n| {
            kind_filter
                .as_ref()
                .map(|ks| ks.iter().any(|k| k == &n.kind))
                .unwrap_or(true)
        })
        .filter(|n| page_filter.is_none_or(|p| node_page(n) == Some(p)))
        .collect();

    let total_blocks = blocks.len() as i64;
    let block_start = args.get("block_start").and_then(|v| v.as_i64()).unwrap_or(0).max(0);
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_i64())
        .unwrap_or(24_000)
        .clamp(1_000, 200_000) as usize;

    let mut out: Vec<Value> = Vec::new();
    let mut chars = 0usize;
    let mut truncated = false;
    let mut next_block: Option<i64> = None;
    for (i, n) in blocks.iter().enumerate().skip(block_start as usize) {
        let text = n.plain_text.clone().unwrap_or_default();
        // 1 ブロックでも返した上で上限超過なら、そこで切って続きを next_block に載せる。
        if chars + text.chars().count() > max_chars && !out.is_empty() {
            next_block = Some(i as i64);
            truncated = true;
            break;
        }
        chars += text.chars().count();
        let equation_label = n.math.as_ref().and_then(|m| m.equation_label.clone());
        // TeX 由来の数式は原文 LaTeX を持つ（Phase 4・semantic_status='source_provided'）。
        let latex = n.math.as_ref().and_then(|m| m.latex.clone());
        let payload_str = |key: &str| {
            n.payload
                .as_ref()
                .and_then(|p| p.get(key))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        };
        let payload_i64 = |key: &str| {
            n.payload
                .as_ref()
                .and_then(|p| p.get(key))
                .and_then(|v| v.as_i64())
        };
        let section_number = payload_str("section_number");
        // 定理系ノード（Phase 5）: 番号・付記名を surface して "定理 2.3 の証明" 取得に使えるようにする。
        let theorem_number = payload_str("theorem_number");
        let note = payload_str("note");
        // figure ノード（Phase 8a）は plain_text を持たない: 空 text の意味が分かるよう
        // 図番号とアセット数を付ける（画像本体は get_figures で）。
        let figure_number = payload_str("figure_number");
        let asset_count = if n.assets.is_empty() {
            None
        } else {
            Some(n.assets.len())
        };
        // table ノード（Phase 8b）: text はセルを " | " 結合した可読形。寸法だけ付けて
        // セル構造（rows/alignments）は get_tables に誘導する。
        let column_spec = payload_str("column_spec");
        let n_columns = payload_i64("n_columns");
        let n_rows = payload_i64("n_rows");
        out.push(json!({
            "index": i,
            "kind": n.kind,
            "page": node_page(n),
            "section_number": section_number,
            "theorem_number": theorem_number,
            "note": note,
            "figure_number": figure_number,
            "asset_count": asset_count,
            "column_spec": column_spec,
            "n_columns": n_columns,
            "n_rows": n_rows,
            "equation_label": equation_label,
            "latex": latex,
            "text": text,
        }));
    }

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "attachment_id": attachment_id,
        "has_lcir": true,
        "source": short_source_name(&doc.source.extractor_name),
        "available_sources": sources_json(&versions),
        "total_blocks": total_blocks,
        "block_start": block_start,
        "returned": out.len(),
        "truncated": truncated,
        "next_block": next_block,
        "blocks": out,
    }))
    .unwrap_or_default())
}

async fn exec_search_document_nodes(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArguments("missing required argument: query".to_string()))?;
    let collection_id = args.get("collection_id").and_then(|v| v.as_i64());
    let tag_id = args.get("tag_id").and_then(|v| v.as_i64());

    let hits = crate::db::document_nodes_fts::search_nodes(pool, query, collection_id, tag_id, None)
        .await?;
    let results: Vec<Value> = hits
        .iter()
        .map(|h| {
            json!({
                "entry_id": h.entry.id,
                "title": h.entry.title,
                "year": h.entry.year,
                "node_kind": h.node_kind,
                "page": h.page,
                "snippet": h.snippet,
                "bbox": h.bbox.as_ref().map(|b| json!([b.x, b.y, b.width, b.height])),
            })
        })
        .collect();

    Ok(serde_json::to_string(&json!({ "count": results.len(), "results": results }))
        .unwrap_or_default())
}

/// 短いスニペット（char 単位で安全に切る）。
fn relation_snippet(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

/// 関係辺の端点ノードを応答用 JSON にする（kind/page/snippet + 番号・label 等の識別子）。
fn relation_node_json(n: &crate::document_ir::LcirNode) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("node_id".to_string(), json!(n.id));
    obj.insert("kind".to_string(), json!(n.kind));
    obj.insert("page".to_string(), json!(node_page(n)));
    if let Some(t) = &n.plain_text {
        obj.insert("snippet".to_string(), json!(relation_snippet(t, 160)));
    }
    if let Some(p) = &n.payload {
        for key in ["theorem_number", "section_number", "labels"] {
            if let Some(v) = p.get(key) {
                obj.insert(key.to_string(), v.clone());
            }
        }
    }
    if let Some(el) = n.math.as_ref().and_then(|m| m.equation_label.as_ref()) {
        obj.insert("equation_label".to_string(), json!(el));
    }
    Value::Object(obj)
}

async fn exec_get_node_relations(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let source_arg = args.get("source").and_then(|v| v.as_str());
    let (loaded, versions) = load_entry_lcir(pool, entry_id, source_arg).await?;
    let Some((_attachment_id, doc)) = loaded else {
        return Ok(no_lcir_response(entry_id, source_arg));
    };

    // 型フィルタ（省略時は全種別）と node_id フィルタ（端点のどちらかが一致）。
    let type_filter: Option<Vec<String>> = args.get("relation_type").and_then(|v| v.as_array()).map(
        |arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        },
    );
    let node_filter = args.get("node_id").and_then(|v| v.as_i64());
    let max_relations = args
        .get("max_relations")
        .and_then(|v| v.as_i64())
        .unwrap_or(300)
        .max(1) as usize;

    let node_by_id: std::collections::HashMap<i64, &crate::document_ir::LcirNode> =
        doc.nodes.iter().map(|n| (n.id, n)).collect();

    let mut counts_by_type: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    let mut relations: Vec<Value> = Vec::new();
    let mut truncated = false;
    for r in &doc.relations {
        if let Some(types) = &type_filter {
            if !types.iter().any(|t| t == &r.relation_type) {
                continue;
            }
        }
        if let Some(nid) = node_filter {
            if r.from_node_id != nid && r.to_node_id != nid {
                continue;
            }
        }
        *counts_by_type.entry(r.relation_type.clone()).or_insert(0) += 1;
        if relations.len() >= max_relations {
            truncated = true;
            continue;
        }
        let from = node_by_id.get(&r.from_node_id).map(|n| relation_node_json(n));
        let to = node_by_id.get(&r.to_node_id).map(|n| relation_node_json(n));
        relations.push(json!({
            "relation_type": r.relation_type,
            "confidence": r.confidence,
            "origin": r.origin,
            "from": from,
            "to": to,
            "metadata": r.metadata,
        }));
    }

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "has_lcir": true,
        "source": short_source_name(&doc.source.extractor_name),
        "available_sources": sources_json(&versions),
        "count": relations.len(),
        "truncated": truncated,
        "counts_by_type": counts_by_type,
        "relations": relations,
    }))
    .unwrap_or_default())
}

async fn exec_get_symbol_definitions(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let source_arg = args.get("source").and_then(|v| v.as_str());
    let (loaded, versions) = load_entry_lcir(pool, entry_id, source_arg).await?;
    let Some((_attachment_id, doc)) = loaded else {
        return Ok(no_lcir_response(entry_id, source_arg));
    };

    let exact = args.get("symbol").and_then(|v| v.as_str());
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());
    let max_symbols = args
        .get("max_symbols")
        .and_then(|v| v.as_i64())
        .unwrap_or(200)
        .max(1) as usize;

    let node_by_id: std::collections::HashMap<i64, &crate::document_ir::LcirNode> =
        doc.nodes.iter().map(|n| (n.id, n)).collect();

    let mut symbols_out: Vec<Value> = Vec::new();
    let mut truncated = false;
    for s in &doc.symbols {
        if let Some(ex) = exact {
            if s.surface_form != ex {
                continue;
            }
        }
        if let Some(q) = &query {
            let hay = format!(
                "{} {} {}",
                s.surface_form,
                s.normalized_form.as_deref().unwrap_or(""),
                s.description.as_deref().unwrap_or("")
            )
            .to_lowercase();
            if !hay.contains(q) {
                continue;
            }
        }
        if symbols_out.len() >= max_symbols {
            truncated = true;
            break;
        }
        let defined_at = s.defined_at_node_id.and_then(|id| node_by_id.get(&id)).map(|n| {
            json!({
                "node_id": n.id,
                "kind": n.kind,
                "snippet": n.plain_text.as_deref().map(|t| relation_snippet(t, 200)),
            })
        });
        let scope = s.scope_node_id.and_then(|id| node_by_id.get(&id)).map(|n| {
            json!({
                "node_id": n.id,
                "section_number": n.payload.as_ref().and_then(|p| p.get("section_number")),
                "text": n.plain_text,
            })
        });
        let occurrences: Vec<Value> = s
            .occurrences
            .iter()
            .take(25)
            .map(|o| {
                let equation_label = node_by_id
                    .get(&o.node_id)
                    .and_then(|n| n.math.as_ref())
                    .and_then(|m| m.equation_label.clone());
                json!({ "node_id": o.node_id, "equation_label": equation_label })
            })
            .collect();
        symbols_out.push(json!({
            "id": s.id,
            "surface_form": s.surface_form,
            "normalized_form": s.normalized_form,
            "description": s.description,
            "symbol_type": s.symbol_type,
            "confidence": s.confidence,
            "origin": s.origin,
            "defined_at": defined_at,
            "scope": scope,
            "occurrence_count": s.occurrences.len(),
            "occurrences": occurrences,
        }));
    }

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "has_lcir": true,
        "source": short_source_name(&doc.source.extractor_name),
        "available_sources": sources_json(&versions),
        "count": symbols_out.len(),
        "truncated": truncated,
        "symbols": symbols_out,
    }))
    .unwrap_or_default())
}

/// 図一覧（Phase 8a）。図領域は PDF 版のみに存在するため常に pdf 版を読む
/// （`get_document_blocks` の page フィルタが pdf を強制するのと同じ分担）。
/// アセットの `relative_path` はメタデータ参照でファイルの存在は保証しない（欠損許容・
/// base64 は返さない）。ベクター図（tikz）はアセット 0 件が正当。
async fn exec_get_figures(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let (loaded, versions) = load_entry_lcir(pool, entry_id, Some("pdf")).await?;
    let Some((attachment_id, doc)) = loaded else {
        return Ok(no_lcir_response(entry_id, Some("pdf")));
    };
    let max_figures = args
        .get("max_figures")
        .and_then(|v| v.as_i64())
        .unwrap_or(100)
        .max(1) as usize;

    // caption_of 辺（from = caption / to = figure）から caption を解決する。
    let mut caption_by_figure: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::new();
    for r in &doc.relations {
        if r.relation_type == "caption_of" {
            caption_by_figure.insert(r.to_node_id, r.from_node_id);
        }
    }
    let node_by_id: std::collections::HashMap<i64, &crate::document_ir::LcirNode> =
        doc.nodes.iter().map(|n| (n.id, n)).collect();

    let mut figures: Vec<Value> = Vec::new();
    let mut total = 0usize;
    let mut truncated = false;
    for n in &doc.nodes {
        if n.kind != "figure" {
            continue;
        }
        total += 1;
        if figures.len() >= max_figures {
            truncated = true;
            continue;
        }
        let bbox = n
            .source_fragments
            .first()
            .map(|f| json!([f.bbox.x, f.bbox.y, f.bbox.width, f.bbox.height]));
        let figure_number = n
            .payload
            .as_ref()
            .and_then(|p| p.get("figure_number"))
            .cloned();
        let caption = caption_by_figure
            .get(&n.id)
            .and_then(|cid| node_by_id.get(cid))
            .map(|c| {
                json!({
                    "node_id": c.id,
                    "text": c.plain_text,
                })
            });
        let assets: Vec<Value> = n
            .assets
            .iter()
            .map(|a| {
                json!({
                    "role": a.role,
                    "relative_path": a.relative_path,
                    "mime_type": a.mime_type,
                    "width": a.width,
                    "height": a.height,
                    "size_bytes": a.size_bytes,
                })
            })
            .collect();
        figures.push(json!({
            "node_id": n.id,
            "page": node_page(n),
            "bbox": bbox,
            "figure_number": figure_number,
            "caption": caption,
            "assets": assets,
        }));
    }

    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "attachment_id": attachment_id,
        "has_lcir": true,
        "source": short_source_name(&doc.source.extractor_name),
        "available_sources": sources_json(&versions),
        "count": total,
        "truncated": truncated,
        "figures": figures,
        "note": "figure regions come from embedded raster images (origin layout_model); vector \
            figures (TikZ/pgf) legitimately yield zero. asset relative_path is metadata only — \
            file existence is not guaranteed and no image bytes are returned.",
    }))
    .unwrap_or_default())
}

/// Phase 8b: 構造化テーブル（TeX 版のみ — tabular は TeX ソースからしかセル構造化できない）。
/// caption は caption_of 辺（from=caption / to=table）から解決する。rows は payload の
/// セル構造をそのまま返すが、原文スニペット `latex_source` は返さない（rows が構造を持ち、
/// 二重送出でレスポンスが肥大するため）。`max_chars` はセル文字量の概算予算（最低 1 表は返す）。
async fn exec_get_tables(pool: &SqlitePool, args: &Value) -> Result<String, ToolError> {
    let entry_id = resolve_entry_id(pool, args).await?;
    let (loaded, versions) = load_entry_lcir(pool, entry_id, Some("tex")).await?;
    let Some((attachment_id, doc)) = loaded else {
        return Ok(serde_json::to_string(&json!({
            "entry_id": entry_id,
            "has_lcir": false,
            "source": "tex",
            "message": "no TeX-derived LCIR for this entry. Tables are cell-structured from the \
                arXiv TeX source only; fetch the TeX source and build LCIR in the app first \
                (PDF-only entries have no structured tables).",
        }))
        .unwrap_or_default());
    };
    let max_tables = args
        .get("max_tables")
        .and_then(|v| v.as_i64())
        .unwrap_or(20)
        .max(1) as usize;
    let max_chars = args
        .get("max_chars")
        .and_then(|v| v.as_i64())
        .unwrap_or(24_000)
        .clamp(1_000, 200_000) as usize;

    let mut caption_by_table: std::collections::HashMap<i64, i64> =
        std::collections::HashMap::new();
    for r in &doc.relations {
        if r.relation_type == "caption_of" {
            caption_by_table.insert(r.to_node_id, r.from_node_id);
        }
    }
    let node_by_id: std::collections::HashMap<i64, &crate::document_ir::LcirNode> =
        doc.nodes.iter().map(|n| (n.id, n)).collect();

    let mut tables: Vec<Value> = Vec::new();
    let mut total = 0usize;
    let mut chars = 0usize;
    let mut truncated = false;
    for n in &doc.nodes {
        if n.kind != "table" {
            continue;
        }
        total += 1;
        // セル文字量の概算 = plain_text（セルを " | " 結合したもの）の長さ。予算・件数超過後は
        // 以降を**すべて**打ち切る（途中の大きい表だけ飛ばすと歯抜けの一覧になり、truncated の
        // 意味が「先頭から N 個」でなくなるため）。
        let approx = n.plain_text.as_deref().map_or(0, |t| t.chars().count());
        if truncated
            || tables.len() >= max_tables
            || (!tables.is_empty() && chars + approx > max_chars)
        {
            truncated = true;
            continue;
        }
        chars += approx;
        let payload = n.payload.as_ref();
        let caption = caption_by_table
            .get(&n.id)
            .and_then(|cid| node_by_id.get(cid))
            .map(|c| {
                json!({
                    "node_id": c.id,
                    "text": c.plain_text,
                })
            });
        let get = |key: &str| payload.and_then(|p| p.get(key)).cloned();
        tables.push(json!({
            "node_id": n.id,
            "caption": caption,
            "column_spec": get("column_spec"),
            "n_columns": get("n_columns"),
            "n_rows": get("n_rows"),
            "alignments": get("alignments"),
            "rows": get("rows"),
        }));
    }

    // 旧抽出器版（8b 前）の LCIR は table ノード自体を持たない — 「表が無い論文」と
    // 誤読させないため、count 0 かつ版が古いときは再構築を明示的に案内する。
    let outdated = doc.source.extractor_version != crate::document_ir::schema::TEX_EXTRACTOR_VERSION;
    let note = if total == 0 && outdated {
        format!(
            "no table nodes, but this LCIR was built by lumencite-tex {} (tables need {}). \
             Rebuild outdated LCIR in the app (Settings → Data) and retry.",
            doc.source.extractor_version,
            crate::document_ir::schema::TEX_EXTRACTOR_VERSION
        )
    } else {
        "tables come from tabular/tabular*/tabularx in the TeX source (origin tex_source); \
         longtable/tabu and nested layouts are intentionally not structured, so zero/fewer \
         tables does not mean the paper has none. Cell text keeps LaTeX verbatim. rule_above \
         records a full-width rule above the row; header rows are not inferred."
            .to_string()
    };
    Ok(serde_json::to_string(&json!({
        "entry_id": entry_id,
        "attachment_id": attachment_id,
        "has_lcir": true,
        "source": short_source_name(&doc.source.extractor_name),
        "extractor_version": doc.source.extractor_version,
        "available_sources": sources_json(&versions),
        "count": total,
        "truncated": truncated,
        "tables": tables,
        "note": note,
    }))
    .unwrap_or_default())
}

// ─── 認可トークン ────────────────────────────────────────────────────────────

/// SQLite の `randomblob` で 48 hex 文字（24 バイト）のトークンを生成する。
/// OS の乱数で seed される SQLite PRNG を使うため、追加の乱数クレートは不要。
pub async fn generate_token(pool: &SqlitePool) -> Result<String, String> {
    sqlx::query_scalar("SELECT lower(hex(randomblob(24)))")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

/// キーチェーンの token を取得。無ければ生成・保存して返す。
pub async fn get_or_create_token(pool: &SqlitePool) -> Result<String, String> {
    let account = crate::keychain::account_for_mcp_token();
    if let Some(t) = crate::keychain::get(&account).map_err(|e| e.to_string())? {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let token = generate_token(pool).await?;
    crate::keychain::set(&account, &token).map_err(|e| e.to_string())?;
    Ok(token)
}

// ─── HTTP トランスポート & ライフサイクル ────────────────────────────────────

/// サーバースレッドが書き込み後の副作用（`.bib` 同期キック・UI イベント）に使う依存。
/// `handle_rpc` 自体には渡さず HTTP 層だけが保持するので、ディスパッチは単体テスト可能。
#[derive(Clone)]
pub struct ServerDeps {
    pub pool: SqlitePool,
    pub app_data_dir: PathBuf,
    pub sync_tx: UnboundedSender<()>,
    /// UI ライブ反映イベント発火用。テストでは `None`、本番は `Some(app.handle())`。
    pub app: Option<tauri::AppHandle>,
}

/// 起動中サーバーの内部ハンドル。
struct RunningServer {
    stop: Arc<AtomicBool>,
    port: u16,
    join: Option<std::thread::JoinHandle<()>>,
}

/// MCP サーバーの起動/停止を管理する。AppState に `Arc` で保持する。
#[derive(Default)]
pub struct McpServerManager {
    inner: Mutex<Option<RunningServer>>,
}

impl McpServerManager {
    /// localhost にバインドしてサーバースレッドを起動する。既存が動いていれば先に停止。
    /// 実際にバインドできたポートを返す（`port=0` で OS 割り当ても可）。
    pub fn start(&self, deps: ServerDeps, port: u16, token: String) -> Result<u16, String> {
        self.stop();

        let addr = format!("127.0.0.1:{port}");
        let server = tiny_http::Server::http(&addr).map_err(|e| format!("bind {addr} failed: {e}"))?;
        let bound_port = server
            .server_addr()
            .to_ip()
            .map(|a| a.port())
            .unwrap_or(port);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let join = std::thread::spawn(move || {
            serve_loop(server, stop_thread, deps, token);
        });

        *self.inner.lock().unwrap() = Some(RunningServer {
            stop,
            port: bound_port,
            join: Some(join),
        });
        Ok(bound_port)
    }

    /// 起動中なら停止してスレッドを join する。未起動なら no-op。
    pub fn stop(&self) {
        if let Some(mut running) = self.inner.lock().unwrap().take() {
            running.stop.store(true, Ordering::SeqCst);
            if let Some(j) = running.join.take() {
                let _ = j.join();
            }
        }
    }

    /// 起動中なら実際のバインドポート、未起動なら None。
    pub fn running_port(&self) -> Option<u16> {
        self.inner.lock().unwrap().as_ref().map(|r| r.port)
    }
}

/// 同時に処理するリクエストの上限（CR-023）。
/// clip はメタデータ取得で最大 ~30s ブロックし得るため、直列だと 1 件で全 traffic が
/// 止まる。ワーカースレッドへ分散しつつ、暴走クライアントによるスレッド無制限生成は防ぐ。
const MAX_CONCURRENT_REQUESTS: usize = 8;

/// 単純なカウンティングセマフォ（`tiny_http` は std スレッドで回るため tokio のものは使わない）。
/// 容量待ちの間も `stop` を監視し、停止時は待ちを解いて `None` を返す。
struct Semaphore {
    state: Mutex<usize>,
    cv: std::sync::Condvar,
}

/// 取得した permit。drop で 1 枠解放する。
struct Permit(Arc<Semaphore>);

impl Semaphore {
    fn new(max: usize) -> Arc<Self> {
        Arc::new(Semaphore {
            state: Mutex::new(max),
            cv: std::sync::Condvar::new(),
        })
    }

    /// 1 枠確保する。容量が空くまで待つが、`stop` が立ったら `None` を返す。
    fn acquire(self: &Arc<Self>, stop: &AtomicBool) -> Option<Permit> {
        let mut avail = self.state.lock().unwrap();
        while *avail == 0 {
            if stop.load(Ordering::SeqCst) {
                return None;
            }
            let (guard, _) = self
                .cv
                .wait_timeout(avail, Duration::from_millis(300))
                .unwrap();
            avail = guard;
        }
        *avail -= 1;
        Some(Permit(self.clone()))
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut avail = self.0.state.lock().unwrap();
        *avail += 1;
        self.0.cv.notify_one();
    }
}

fn serve_loop(server: tiny_http::Server, stop: Arc<AtomicBool>, deps: ServerDeps, token: String) {
    // 同時処理数を上限付きで並列化する（CR-023: 1 件の遅い clip が全 traffic を止めない）。
    let sem = Semaphore::new(MAX_CONCURRENT_REQUESTS);
    // recv_timeout で定期的に stop フラグを確認しつつ accept する。
    while !stop.load(Ordering::SeqCst) {
        match server.recv_timeout(Duration::from_millis(300)) {
            Ok(Some(req)) => {
                // 容量待ち。stop が立てば None → ループ終了。
                let Some(permit) = sem.acquire(&stop) else {
                    break;
                };
                let deps = deps.clone();
                let token = token.clone();
                std::thread::spawn(move || {
                    let _permit = permit; // drop で枠を解放する
                    handle_http_request(req, &deps, &token);
                });
            }
            Ok(None) => continue, // タイムアウト → ループ先頭で stop を再確認
            Err(_) => break,
        }
    }
}

/// リクエストボディの上限。JSON-RPC には十分大きく、暴走クライアントによる
/// 無制限読み込みは防ぐ。
const MAX_BODY_BYTES: u64 = 10 * 1024 * 1024;

/// 定数時間の文字列比較（トークン照合用。早期 return によるタイミング差を避ける）。
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// リクエストの行き先。`route()` は pure なので単体テストできる。
#[derive(Debug, PartialEq)]
enum Route {
    /// `OPTIONS /clipper`・`OPTIONS /clipper/complete` — CORS preflight
    /// （**認証不要**: preflight は Authorization を持たない）
    ClipperPreflight,
    /// `GET /clipper` — ペアリング疎通確認（認証必須）
    ClipperPing,
    /// `POST /clipper` — クリップ本体（認証必須 + `clipper.enabled`）
    Clip,
    /// `POST /clipper/complete` — 重複エントリの欠落補完（認証必須 + `clipper.enabled`）
    ClipperComplete,
    /// `POST <その他>` — 既存の JSON-RPC（`/mcp` ほかパス不問。後方互換）
    Rpc,
    /// それ以外のメソッド → 405
    MethodNotAllowed,
}

fn route(method: &tiny_http::Method, path: &str) -> Route {
    use tiny_http::Method;
    // クエリ・末尾スラッシュを無視してパスを正規化する
    let path = path.split('?').next().unwrap_or(path);
    let path = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    // `/clipper/complete` は catch-all の `(Post, _) => Rpc` より**前**に置く。
    match (method, path) {
        (Method::Options, "/clipper" | "/clipper/complete") => Route::ClipperPreflight,
        (Method::Get, "/clipper") => Route::ClipperPing,
        (Method::Post, "/clipper") => Route::Clip,
        (Method::Post, "/clipper/complete") => Route::ClipperComplete,
        (Method::Post, _) => Route::Rpc,
        _ => Route::MethodNotAllowed,
    }
}

/// `Origin` が Chrome 拡張のときだけ返す CORS ヘッダ群。
/// Web ページ由来の Origin（https:// 等）には返さない。
fn cors_headers(origin: Option<&str>) -> Vec<tiny_http::Header> {
    let Some(origin) = origin.filter(|o| o.starts_with("chrome-extension://")) else {
        return vec![];
    };
    let h = |k: &[u8], v: &[u8]| tiny_http::Header::from_bytes(k, v).expect("static header");
    vec![
        h(b"Access-Control-Allow-Origin", origin.as_bytes()),
        h(b"Access-Control-Allow-Methods", b"GET, POST, OPTIONS"),
        h(b"Access-Control-Allow-Headers", b"Authorization, Content-Type"),
        // Private Network Access: 拡張 → 127.0.0.1（loopback）への preflight を、
        // 将来 Chrome が PNA を強制しても通す（現状は host_permissions で免除されるが無害）。
        h(b"Access-Control-Allow-Private-Network", b"true"),
        h(b"Access-Control-Max-Age", b"600"),
    ]
}

fn handle_http_request(mut req: tiny_http::Request, deps: &ServerDeps, token: &str) {
    use tauri::Emitter;
    use tiny_http::{Header, Response};

    let json_ct = || {
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("static header")
    };
    let origin: Option<String> = req
        .headers()
        .iter()
        .find(|h| h.field.equiv("Origin"))
        .map(|h| h.value.as_str().to_string());
    let cors = cors_headers(origin.as_deref());
    let with_cors = |mut resp: Response<std::io::Cursor<Vec<u8>>>| {
        for h in &cors {
            resp = resp.with_header(h.clone());
        }
        resp
    };

    let routed = route(req.method(), req.url());

    // preflight は Authorization ヘッダを持たないため、認証より先に処理する
    if routed == Route::ClipperPreflight {
        let _ = req.respond(with_cors(Response::from_string("").with_status_code(204)));
        return;
    }

    // 認可: Authorization: Bearer <token>
    let authorized = req.headers().iter().any(|h| {
        h.field.equiv("Authorization")
            && h.value
                .as_str()
                .strip_prefix("Bearer ")
                .map(|t| constant_time_eq(t, token))
                .unwrap_or(false)
    });
    if !authorized {
        let _ = req.respond(with_cors(
            Response::from_string("unauthorized").with_status_code(401),
        ));
        return;
    }

    match routed {
        Route::ClipperPreflight => unreachable!("handled before auth"),
        Route::MethodNotAllowed => {
            let _ = req.respond(Response::from_string("method not allowed").with_status_code(405));
        }
        Route::ClipperPing => {
            let body = json!({ "ok": true, "app": "LumenCite", "version": env!("CARGO_PKG_VERSION") });
            let _ = req.respond(with_cors(
                Response::from_string(body.to_string()).with_header(json_ct()),
            ));
        }
        Route::Clip => {
            if !tauri::async_runtime::block_on(clipper::clipper_enabled(&deps.pool)) {
                let body = json!({ "status": "error", "code": "clipper_disabled" });
                let _ = req.respond(with_cors(
                    Response::from_string(body.to_string())
                        .with_status_code(403)
                        .with_header(json_ct()),
                ));
                return;
            }
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err((code, msg)) => {
                    let _ = req.respond(with_cors(
                        Response::from_string(msg).with_status_code(code),
                    ));
                    return;
                }
            };
            let clip_req: clipper::ClipRequest = match serde_json::from_str(&body) {
                Ok(r) => r,
                Err(e) => {
                    let body =
                        json!({ "status": "error", "code": "bad_request", "message": e.to_string() });
                    let _ = req.respond(with_cors(
                        Response::from_string(body.to_string())
                            .with_status_code(400)
                            .with_header(json_ct()),
                    ));
                    return;
                }
            };
            // handle_clip は外部 API（CrossRef / arXiv / OpenLibrary）へ reqwest で
            // アクセスする。serve_loop スレッド上の block_on では reqwest の I/O が
            // 進まず必ずタイムアウトする（E2E で発覚）ため、ランタイムのワーカーへ
            // spawn して結果をチャネルで待つ。
            let outcome = {
                let pool = deps.pool.clone();
                run_on_runtime(async move { clipper::handle_clip(&pool, &clip_req).await })
            };
            if outcome.mutated {
                let _ = deps.sync_tx.send(());
                if let Some(app) = &deps.app {
                    let _ = app.emit("entries-changed", ());
                }
            }
            let pdf_job = outcome.pdf_job.clone();
            let tex_source_job = outcome.tex_source_job.clone();
            let _ = req.respond(with_cors(
                Response::from_string(outcome.response.to_string())
                    .with_status_code(outcome.status)
                    .with_header(json_ct()),
            ));
            // PDF / TeX ソースのダウンロードは応答後に非同期実行（serve loop を塞がない）
            if let Some(job) = pdf_job {
                spawn_pdf_job(deps, job);
            }
            if let Some(job) = tex_source_job {
                spawn_tex_source_job(deps, job);
            }
        }
        Route::ClipperComplete => {
            if !tauri::async_runtime::block_on(clipper::clipper_enabled(&deps.pool)) {
                let body = json!({ "status": "error", "code": "clipper_disabled" });
                let _ = req.respond(with_cors(
                    Response::from_string(body.to_string())
                        .with_status_code(403)
                        .with_header(json_ct()),
                ));
                return;
            }
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err((code, msg)) => {
                    let _ = req
                        .respond(with_cors(Response::from_string(msg).with_status_code(code)));
                    return;
                }
            };
            let complete_req: clipper::CompleteRequest = match serde_json::from_str(&body) {
                Ok(r) => r,
                Err(e) => {
                    let body = json!({ "status": "error", "code": "bad_request", "message": e.to_string() });
                    let _ = req.respond(with_cors(
                        Response::from_string(body.to_string())
                            .with_status_code(400)
                            .with_header(json_ct()),
                    ));
                    return;
                }
            };
            // handle_complete は DB のみ（欠落再検証 + 設定保存）なので block_on で足りる。
            // 実ダウンロードは応答後に spawn するジョブが担う（PDF ジョブと同じ契約）。
            let outcome =
                tauri::async_runtime::block_on(clipper::handle_complete(&deps.pool, &complete_req));
            let pdf_job = outcome.pdf_job.clone();
            let tex_source_job = outcome.tex_source_job.clone();
            let _ = req.respond(with_cors(
                Response::from_string(outcome.response.to_string())
                    .with_status_code(outcome.status)
                    .with_header(json_ct()),
            ));
            if let Some(job) = pdf_job {
                spawn_pdf_job(deps, job);
            }
            if let Some(job) = tex_source_job {
                spawn_tex_source_job(deps, job);
            }
        }
        Route::Rpc => {
            let body = match read_body(&mut req) {
                Ok(b) => b,
                Err((code, msg)) => {
                    let _ = req.respond(Response::from_string(msg).with_status_code(code));
                    return;
                }
            };
            let outcome: RpcOutcome = match serde_json::from_str::<Value>(&body) {
                Ok(v) => {
                    tauri::async_runtime::block_on(handle_rpc(&deps.pool, &deps.app_data_dir, &v))
                }
                Err(e) => RpcOutcome {
                    response: Some(json!({
                        "jsonrpc": "2.0", "id": null,
                        "error": { "code": -32700, "message": format!("parse error: {e}") }
                    })),
                    mutated: false,
                },
            };

            // write 成功の副作用: `.bib` 自動同期キック + 一覧へのライブ反映イベント。
            if outcome.mutated {
                let _ = deps.sync_tx.send(());
                if let Some(app) = &deps.app {
                    let _ = app.emit("entries-changed", ());
                }
            }

            match outcome.response {
                Some(v) => {
                    let _ = req
                        .respond(Response::from_string(v.to_string()).with_header(json_ct()));
                }
                // 通知のみ（応答不要）→ 202 Accepted
                None => {
                    let _ = req.respond(Response::from_string("").with_status_code(202));
                }
            }
        }
    }
}

/// serve_loop スレッドから、非同期ランタイムの**ワーカー上で** future を実行して
/// 完了を待つ。`tauri::async_runtime::block_on` はこのスレッド上で future を駆動する
/// ため、reqwest のようなネットワーク I/O を含む future が進行しない。
/// DB のみの future（sqlx）は block_on で問題ないが、外部 HTTP を含むものは必ず
/// こちらを使うこと。
fn run_on_runtime<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.recv().expect("runtime task dropped without sending a result")
}

/// ボディを上限付きで読む（Content-Length は詐称できるため実読で判定）。
fn read_body(req: &mut tiny_http::Request) -> Result<String, (u16, &'static str)> {
    use std::io::Read;
    let mut body = String::new();
    let mut limited = std::io::Read::take(req.as_reader(), MAX_BODY_BYTES + 1);
    if limited.read_to_string(&mut body).is_err() {
        return Err((400, "bad request body"));
    }
    if body.len() as u64 > MAX_BODY_BYTES {
        return Err((413, "payload too large"));
    }
    Ok(body)
}

/// 同一エントリへの PDF ダウンロードが同時に走らないようにする in-flight 集合。
///
/// 欠落補完の `plan_completion`（PDF なし判定）は `spawn_pdf_job` のダウンロード開始と
/// 非アトミックなので、同じ論文を複数タブ / 連打で重複クリップすると、両リクエストとも
/// 「PDF なし」と見て 2 本の PDF ジョブを出し得る。`download_and_attach` は既存 PDF を
/// dedup せず別名で積む（`create_unique_file`）ため、放置すると同一エントリに PDF が
/// 二重添付される。エントリ単位で 1 本に絞ることでこれを防ぐ（TeX は上書き契約なので不要）。
static PDF_JOBS_IN_FLIGHT: std::sync::Mutex<std::collections::BTreeSet<i64>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// [`PDF_JOBS_IN_FLIGHT`] からエントリ id を Drop で必ず外す（早期 return / panic でも）。
struct PdfJobGuard(i64);
impl Drop for PdfJobGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = PDF_JOBS_IN_FLIGHT.lock() {
            set.remove(&self.0);
        }
    }
}

/// PDF ダウンロードジョブを応答後に非同期実行する。成功したら `entries-changed` で
/// UI に反映する（添付は .bib の内容に影響しないため sync はキックしない）。
/// 失敗（ペイウォール・サイズ超過等）はログのみ — エントリ作成は既に成功している。
fn spawn_pdf_job(deps: &ServerDeps, job: clipper::PdfJob) {
    let pool = deps.pool.clone();
    let app_data_dir = deps.app_data_dir.clone();
    let app = deps.app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        // 同一エントリの PDF ジョブが既に走っていれば二重添付を避けてスキップ。
        {
            let mut set = match PDF_JOBS_IN_FLIGHT.lock() {
                Ok(s) => s,
                Err(poisoned) => poisoned.into_inner(),
            };
            if !set.insert(job.entry_id) {
                eprintln!(
                    "clipper: PDF job for entry {} already in flight; skipping duplicate",
                    job.entry_id
                );
                return;
            }
        }
        let _in_flight = PdfJobGuard(job.entry_id);
        match crate::download::download_and_attach(
            &pool,
            &app_data_dir,
            job.entry_id,
            &job.url,
            crate::download::DownloadCaps::default(),
        )
        .await
        {
            Ok(att) => {
                eprintln!("clipper: attached {} to entry {}", att.file_name, job.entry_id);
                // 添付後に全文索引する（クリッパー経路も自動索引・CR-027）。
                let abs = app_data_dir
                    .join("attachments")
                    .join(job.entry_id.to_string())
                    .join(&att.file_name);
                crate::db::fulltext::extract_and_index(&pool, abs, att.id).await;
                if let Some(app) = &app {
                    let _ = app.emit("entries-changed", ());
                }
            }
            Err(e) => {
                eprintln!("clipper: PDF download failed for entry {}: {e}", job.entry_id);
            }
        }
    });
}

/// クリップ後の arXiv TeX ソース自動取得 + LCIR 構築（LCIR Phase 4 の自動化・best-effort）。
///
/// ジョブは `lcir.enabled` ON のときだけ発行される（`clipper::derive_tex_source_job`）。
/// 失敗はログのみでクリップ自体は成功扱い（PDF ジョブと同じ契約）。ビルドは内部でも
/// フラグを再確認するので、発行後に OFF へ切り替わっても DB には書かない。
fn spawn_tex_source_job(deps: &ServerDeps, job: clipper::TexSourceJob) {
    let pool = deps.pool.clone();
    let app_data_dir = deps.app_data_dir.clone();
    let app = deps.app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let url = format!("https://arxiv.org/e-print/{}", job.arxiv_id);
        match crate::download::download_and_attach_arxiv_source(
            &pool,
            &app_data_dir,
            job.entry_id,
            &job.arxiv_id,
            &url,
            crate::download::DownloadCaps::default(),
        )
        .await
        {
            Ok(att) => {
                eprintln!(
                    "clipper: attached TeX source {} to entry {}",
                    att.file_name, job.entry_id
                );
                match crate::ingestion::build_lcir_for_attachment(&pool, &app_data_dir, att.id)
                    .await
                {
                    Ok(r) => {
                        eprintln!("clipper: LCIR build for attachment {}: {}", att.id, r.message)
                    }
                    Err(e) => {
                        eprintln!("clipper: LCIR build failed for attachment {}: {e}", att.id)
                    }
                }
                if let Some(app) = &app {
                    let _ = app.emit("entries-changed", ());
                }
            }
            Err(e) => {
                eprintln!(
                    "clipper: TeX source download failed for entry {}: {e}",
                    job.entry_id
                );
            }
        }
    });
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    fn req(method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    /// CR-023: セマフォは上限まで permit を出し、超過は解放待ちにする。
    /// drop で枠が戻り、stop が立てば待ちを解いて `None` を返す。
    #[test]
    fn semaphore_bounds_concurrency_and_respects_stop() {
        let sem = Semaphore::new(2);
        let stop = AtomicBool::new(false);
        let p1 = sem.acquire(&stop).expect("1st permit");
        let _p2 = sem.acquire(&stop).expect("2nd permit");
        // 満杯: stop を立てると 3 つ目は待たずに None。
        stop.store(true, Ordering::SeqCst);
        assert!(sem.acquire(&stop).is_none(), "満杯 + stop で None");
        // 1 枠戻せば（stop 中でも空きがあるので）取得できる。
        drop(p1);
        assert!(sem.acquire(&stop).is_some(), "解放後は取得できる");
    }

    async fn call_tool(pool: &SqlitePool, name: &str, args: Value) -> Value {
        let r = req("tools/call", json!({ "name": name, "arguments": args }));
        handle_rpc(pool, Path::new(""), &r).await.response.unwrap()
    }

    async fn enable_writes(pool: &SqlitePool) {
        crate::db::settings::set_setting(
            pool,
            crate::db::settings::MCP_SERVER_WRITE_ENABLED_KEY,
            "1",
        )
        .await
        .unwrap();
    }

    #[test]
    fn route_dispatch_table() {
        use tiny_http::Method;
        assert_eq!(route(&Method::Options, "/clipper"), Route::ClipperPreflight);
        assert_eq!(route(&Method::Get, "/clipper"), Route::ClipperPing);
        assert_eq!(route(&Method::Post, "/clipper"), Route::Clip);
        // 欠落補完エンドポイント（catch-all Rpc より前に一致する）
        assert_eq!(route(&Method::Post, "/clipper/complete"), Route::ClipperComplete);
        assert_eq!(route(&Method::Options, "/clipper/complete"), Route::ClipperPreflight);
        // 末尾スラッシュ・クエリは無視する
        assert_eq!(route(&Method::Post, "/clipper/"), Route::Clip);
        assert_eq!(route(&Method::Post, "/clipper/complete/"), Route::ClipperComplete);
        assert_eq!(route(&Method::Post, "/clipper/complete?x=1"), Route::ClipperComplete);
        assert_eq!(route(&Method::Get, "/clipper?x=1"), Route::ClipperPing);
        // /clipper/complete は POST 専用（GET は 405）
        assert_eq!(route(&Method::Get, "/clipper/complete"), Route::MethodNotAllowed);
        // 既存 JSON-RPC: POST は任意パスで従来どおり
        assert_eq!(route(&Method::Post, "/mcp"), Route::Rpc);
        assert_eq!(route(&Method::Post, "/"), Route::Rpc);
        // その他メソッドは 405（従来挙動の維持）
        assert_eq!(route(&Method::Get, "/mcp"), Route::MethodNotAllowed);
        assert_eq!(route(&Method::Options, "/mcp"), Route::MethodNotAllowed);
        assert_eq!(route(&Method::Delete, "/clipper"), Route::MethodNotAllowed);
    }

    #[test]
    fn cors_headers_only_for_chrome_extension_origin() {
        assert!(cors_headers(None).is_empty());
        assert!(cors_headers(Some("https://evil.example")).is_empty());
        let hs = cors_headers(Some("chrome-extension://abcdef"));
        assert!(hs.iter().any(|h| {
            h.field.equiv("Access-Control-Allow-Origin")
                && h.value.as_str() == "chrome-extension://abcdef"
        }));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn http_clipper_routes_auth_cors_and_gating(pool: SqlitePool) {
        let manager = McpServerManager::default();
        let token = "test-token-clipper".to_string();
        let (sync_tx, mut sync_rx) = tokio::sync::mpsc::unbounded_channel();
        let deps = ServerDeps {
            pool: pool.clone(),
            app_data_dir: PathBuf::from(""),
            sync_tx,
            app: None,
        };
        let port = manager.start(deps, 0, token.clone()).expect("server should bind");
        let url = format!("http://127.0.0.1:{port}/clipper");
        let client = reqwest::Client::new();

        // OPTIONS preflight は認証なしで 204。chrome-extension Origin にだけ CORS を返す
        let resp = client
            .request(reqwest::Method::OPTIONS, &url)
            .header("Origin", "chrome-extension://abc")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
        assert_eq!(
            resp.headers().get("access-control-allow-origin").map(|v| v.to_str().unwrap()),
            Some("chrome-extension://abc")
        );
        let resp = client
            .request(reqwest::Method::OPTIONS, &url)
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 204);
        assert!(resp.headers().get("access-control-allow-origin").is_none());

        // 認証なしの GET/POST は 401
        assert_eq!(client.get(&url).send().await.unwrap().status(), 401);
        assert_eq!(client.post(&url).body("{}").send().await.unwrap().status(), 401);

        // GET /clipper（ペアリング疎通確認）
        let resp = client.get(&url).bearer_auth(&token).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let ping: Value = resp.json().await.unwrap();
        assert_eq!(ping["ok"], true);
        assert_eq!(ping["app"], "LumenCite");

        // clipper.enabled 未設定 → POST は 403
        let resp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&json!({ "url": "https://example.com/a" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 403);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["code"], "clipper_disabled");

        // 有効化 → 作成成功 + sync キック
        crate::db::settings::set_setting(&pool, crate::db::settings::CLIPPER_ENABLED_KEY, "1")
            .await
            .unwrap();
        let resp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&json!({ "url": "https://example.com/a", "title": "Clipped Page" }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "created");
        assert_eq!(body["title"], "Clipped Page");
        assert!(sync_rx.try_recv().is_ok(), "作成成功で .bib 同期がキックされる");

        // 既存 JSON-RPC ルートは従来どおり動く（後方互換）
        let rpc_url = format!("http://127.0.0.1:{port}/mcp");
        let resp = client
            .post(&rpc_url)
            .bearer_auth(&token)
            .json(&req("tools/list", json!({})))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        manager.stop();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn initialize_returns_protocol_and_server_info(pool: SqlitePool) {
        let resp = handle_rpc(&pool, Path::new(""), &req("initialize", json!({})))
            .await
            .response
            .unwrap();
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["result"]["serverInfo"]["name"], "LumenCite");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_list_has_read_tools_and_excludes_mutate(pool: SqlitePool) {
        // 既定（write_enabled 未設定 = false）では write 系は出ない。
        let resp = handle_rpc(&pool, Path::new(""), &req("tools/list", json!({})))
            .await
            .response
            .unwrap();
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in [
            "fulltext_search",
            "get_entry",
            "list_collections",
            "list_tags",
            "search_entries",
            "resolve_citation_key",
            "export_bibtex",
            "get_document_structure",
            "get_document_blocks",
            "search_document_nodes",
            "get_node_relations",
            "get_symbol_definitions",
            "get_figures",
            "get_tables",
        ] {
            assert!(names.contains(&expected), "missing read tool: {expected}");
        }
        // write/mutate/ocr は公開しない。
        for forbidden in ["create_entry", "update_entry", "delete_entry", "add_tag", "ocr_pdf"] {
            assert!(!names.contains(&forbidden), "must not expose: {forbidden}");
        }
    }

    /// ツール結果 content[0].text（JSON 文字列）をパースする。
    fn tool_json(resp: &Value) -> Value {
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    }

    /// block ノード + block fragment を 1 個作る（LCIR テスト用）。
    async fn add_block(
        pool: &SqlitePool,
        vid: i64,
        page: i64,
        kind: &str,
        ordinal: i64,
        text: &str,
        payload: Option<&str>,
    ) -> i64 {
        let id = crate::db::document_nodes::insert_node(
            pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(page),
                node_kind: kind,
                ordinal,
                plain_text: Some(text),
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: payload,
            },
        )
        .await
        .unwrap();
        crate::db::source_fragments::insert_fragment(
            pool,
            &crate::db::source_fragments::NewSourceFragment {
                node_id: id,
                page_number: 1,
                x: 72.0,
                y: 500.0,
                width: 300.0,
                height: 12.0,
                rotation: 0.0,
                reading_order: Some(ordinal),
                fragment_type: Some("block"),
            },
        )
        .await
        .unwrap();
        id
    }

    /// LCIR 構築済みエントリ（abstract/section/paragraph/display_math）を作り entry_id を返す。
    async fn setup_entry_with_lcir(pool: &SqlitePool) -> i64 {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "Quantum walk paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = crate::db::document_nodes::insert_node(
            pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("full page text"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        add_block(pool, vid, page, "abstract", 0, "We study quantum walks.", None).await;
        add_block(
            pool,
            vid,
            page,
            "section",
            1,
            "1 Introduction",
            Some(r#"{"heading_level":1,"section_number":"1"}"#),
        )
        .await;
        add_block(
            pool,
            vid,
            page,
            "paragraph",
            2,
            "Quantum walks are discrete analogues of diffusion.",
            None,
        )
        .await;
        let eq = add_block(pool, vid, page, "display_math", 3, "U = S2 C2 S1 C1 (1.1)", None).await;
        crate::db::math_expressions::insert_math(
            pool,
            &crate::db::math_expressions::NewMathExpression {
                node_id: eq,
                display_mode: "display",
                equation_label: Some("(1.1)"),
                latex: None,
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("U = S2 C2 S1 C1 (1.1)"),
                ast_json: None,
                semantic_status: "surface_only",
                confidence: Some(0.75),
                origin: Some("pdf_text_layer"),
            },
        )
        .await
        .unwrap();
        // node-FTS を張る（search_document_nodes 用）。
        crate::ingestion::regenerate_node_fts_from_lcir(pool, att)
            .await
            .unwrap();
        entry.id
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_document_structure_returns_outline_counts_and_abstract(pool: SqlitePool) {
        let entry_id = setup_entry_with_lcir(&pool).await;
        let resp = call_tool(&pool, "get_document_structure", json!({ "entry_id": entry_id })).await;
        let j = tool_json(&resp);
        assert_eq!(j["has_lcir"], true);
        assert_eq!(j["counts"]["display_math"], 1);
        assert_eq!(j["counts"]["paragraph"], 1);
        assert_eq!(j["abstract"], "We study quantum walks.");
        // アウトラインに節が節番号つきで入る。
        let outline = j["outline"].as_array().unwrap();
        assert_eq!(outline.len(), 1);
        assert_eq!(outline[0]["section_number"], "1");
        assert_eq!(outline[0]["page"], 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_document_blocks_filters_by_kind_and_exposes_math(pool: SqlitePool) {
        let entry_id = setup_entry_with_lcir(&pool).await;
        // kinds=["display_math"] → 数式だけ。equation_label が付く。
        let resp = call_tool(
            &pool,
            "get_document_blocks",
            json!({ "entry_id": entry_id, "kinds": ["display_math"] }),
        )
        .await;
        let j = tool_json(&resp);
        assert_eq!(j["total_blocks"], 1);
        let blocks = j["blocks"].as_array().unwrap();
        assert_eq!(blocks[0]["kind"], "display_math");
        assert_eq!(blocks[0]["equation_label"], "(1.1)");
        assert!(blocks[0]["text"].as_str().unwrap().contains("S2 C2"));

        // フィルタ無し → 本文ブロック 4 個（document/page/line は除外）。
        let all = tool_json(&call_tool(&pool, "get_document_blocks", json!({ "entry_id": entry_id })).await);
        assert_eq!(all["total_blocks"], 4);
    }

    /// Phase 6a: get_node_relations が参照グラフを端点ノード情報つきで返し、型/ノードで絞れること。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_node_relations_returns_edges_with_endpoints(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Math paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("full page text"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let thm = add_block(
            &pool,
            vid,
            page,
            "theorem",
            0,
            "Theorem 2.3. Bounded implies compact.",
            Some(r#"{"theorem_number":"2.3"}"#),
        )
        .await;
        let para = add_block(
            &pool,
            vid,
            page,
            "paragraph",
            1,
            "The result follows by Theorem 2.3.",
            None,
        )
        .await;
        let proof = add_block(&pool, vid, page, "proof", 2, "Proof. Immediate.", None).await;
        // 参照辺を手で入れる（build 経路の代わり・端点 enrich と絞り込みの検証）。
        crate::db::node_relations::insert_relation(
            &pool,
            &crate::db::node_relations::NewNodeRelation {
                document_version_id: vid,
                from_node_id: para,
                relation_type: "refers_to_theorem",
                to_node_id: thm,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                metadata_json: Some(r#"{"number":"2.3"}"#),
            },
        )
        .await
        .unwrap();
        crate::db::node_relations::insert_relation(
            &pool,
            &crate::db::node_relations::NewNodeRelation {
                document_version_id: vid,
                from_node_id: proof,
                relation_type: "proves",
                to_node_id: thm,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                metadata_json: Some(r#"{"by":"adjacency"}"#),
            },
        )
        .await
        .unwrap();

        let j = tool_json(&call_tool(&pool, "get_node_relations", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], true);
        assert_eq!(j["source"], "pdf");
        assert_eq!(j["count"], 2);
        assert_eq!(j["counts_by_type"]["proves"], 1);
        assert_eq!(j["counts_by_type"]["refers_to_theorem"], 1);
        let rels = j["relations"].as_array().unwrap();
        let proves = rels.iter().find(|r| r["relation_type"] == "proves").unwrap();
        assert_eq!(proves["from"]["kind"], "proof");
        assert_eq!(proves["to"]["kind"], "theorem");
        assert_eq!(proves["to"]["theorem_number"], "2.3");
        assert_eq!(proves["to"]["page"], 1);

        // relation_type フィルタ。
        let only = tool_json(
            &call_tool(
                &pool,
                "get_node_relations",
                json!({ "entry_id": entry.id, "relation_type": ["proves"] }),
            )
            .await,
        );
        assert_eq!(only["count"], 1);

        // node_id フィルタ: 定理に触れる辺は 2 本（refers_to_theorem と proves）。
        let touching = tool_json(
            &call_tool(
                &pool,
                "get_node_relations",
                json!({ "entry_id": entry.id, "node_id": thm }),
            )
            .await,
        );
        assert_eq!(touching["count"], 2);
    }

    /// Phase 8a: get_figures が figure ノードを bbox・図番号・caption（caption_of 辺から）・
    /// アセット（相対パスのみ）つきで返すこと。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_figures_returns_figures_with_caption_and_assets(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Figure paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-fig",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("page text"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let caption = add_block(
            &pool,
            vid,
            page,
            "figure_caption",
            0,
            "Figure 2: The apparatus.",
            Some(r#"{"caption_label":"Figure","caption_number":"2"}"#),
        )
        .await;
        // figure ノード（plain_text 無し・bbox fragment 付き）。
        let figure = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(page),
                node_kind: NodeKind::Figure.as_str(),
                ordinal: 1,
                plain_text: None,
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: Some(r#"{"figure_index":1,"figure_number":"2"}"#),
            },
        )
        .await
        .unwrap();
        crate::db::source_fragments::insert_fragment(
            &pool,
            &crate::db::source_fragments::NewSourceFragment {
                node_id: figure,
                page_number: 1,
                x: 100.0,
                y: 400.0,
                width: 300.0,
                height: 200.0,
                rotation: 0.0,
                reading_order: None,
                fragment_type: Some("block"),
            },
        )
        .await
        .unwrap();
        let asset_id = crate::db::assets::insert_asset(
            &pool,
            &crate::db::assets::NewAsset {
                document_version_id: vid,
                sha256: "abc",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/deadbeef/fig-p001-00.png",
                width: Some(800),
                height: Some(534),
                size_bytes: Some(4321),
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        crate::db::assets::insert_node_asset(
            &pool,
            &crate::db::assets::NewNodeAsset {
                node_id: figure,
                asset_id,
            },
            "page_crop",
        )
        .await
        .unwrap();
        crate::db::node_relations::insert_relation(
            &pool,
            &crate::db::node_relations::NewNodeRelation {
                document_version_id: vid,
                from_node_id: caption,
                relation_type: "caption_of",
                to_node_id: figure,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let j = tool_json(&call_tool(&pool, "get_figures", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], true);
        assert_eq!(j["source"], "pdf");
        assert_eq!(j["count"], 1);
        let f = &j["figures"][0];
        assert_eq!(f["node_id"], figure);
        assert_eq!(f["page"], 1);
        assert_eq!(f["bbox"], json!([100.0, 400.0, 300.0, 200.0]));
        assert_eq!(f["figure_number"], "2");
        assert_eq!(f["caption"]["node_id"], caption);
        assert_eq!(f["caption"]["text"], "Figure 2: The apparatus.");
        let a = &f["assets"][0];
        assert_eq!(a["role"], "page_crop");
        assert_eq!(a["relative_path"], "attachments/1/.lcir/1/deadbeef/fig-p001-00.png");
        assert_eq!(a["width"], 800);
        assert_eq!(a["size_bytes"], 4321);
        // バイト列は返さない（メタデータ参照のみ）。
        assert!(a.get("data").is_none() && a.get("base64").is_none());

        // blocks 経由でも figure ノードが figure_number/asset_count つきで見える（空 text の説明）。
        let blocks = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry.id, "kinds": ["figure"] }),
            )
            .await,
        );
        assert_eq!(blocks["returned"], 1);
        assert_eq!(blocks["blocks"][0]["figure_number"], "2");
        assert_eq!(blocks["blocks"][0]["asset_count"], 1);
        assert_eq!(blocks["blocks"][0]["text"], "");
    }

    /// Phase 8a: PDF 版が無い（TeX のみ）エントリでは get_figures は has_lcir:false を返すこと。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_figures_requires_pdf_version(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Tex only".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/s.gz", entry.id),
            "s.gz",
            "application/gzip",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-tex",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/gzip",
                extractor_name: schema::TEX_EXTRACTOR_NAME,
                extractor_version: schema::TEX_EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("tex_source"),
                payload_json: None,
            },
        )
        .await
        .unwrap();

        let j = tool_json(&call_tool(&pool, "get_figures", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], false);
    }

    /// Phase 8b: get_tables が table ノードをセル構造・caption（caption_of 辺から）つきで返し、
    /// latex_source は返さないこと。blocks 経由では寸法（n_rows/n_columns）が付くこと。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_tables_returns_structured_rows_with_caption(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Table paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/s.gz", entry.id),
            "s.gz",
            "application/gzip",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-tab",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/gzip",
                extractor_name: schema::TEX_EXTRACTOR_NAME,
                extractor_version: schema::TEX_EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("tex_source"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let caption = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::TableCaption.as_str(),
                ordinal: 0,
                plain_text: Some("Particle masses."),
                language: None,
                confidence: Some(0.95),
                origin: Some("tex_source"),
                payload_json: Some(r#"{"labels":["tab:m"]}"#),
            },
        )
        .await
        .unwrap();
        let payload = serde_json::json!({
            "column_spec": "lc",
            "n_columns": 2,
            "n_rows": 2,
            "alignments": ["l", "c"],
            "rows": [
                {"cells": [{"text": "Particle"}, {"text": "Mass"}]},
                {"cells": [{"text": "e"}, {"text": "$0.511$"}], "rule_above": true},
            ],
            "latex_source": "\\begin{tabular}{lc}...\\end{tabular}",
        })
        .to_string();
        let table = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Table.as_str(),
                ordinal: 1,
                plain_text: Some("Particle | Mass\ne | $0.511$"),
                language: None,
                confidence: Some(0.9),
                origin: Some("tex_source"),
                payload_json: Some(&payload),
            },
        )
        .await
        .unwrap();
        crate::db::node_relations::insert_relation(
            &pool,
            &crate::db::node_relations::NewNodeRelation {
                document_version_id: vid,
                from_node_id: caption,
                relation_type: "caption_of",
                to_node_id: table,
                confidence: Some(0.95),
                origin: Some("tex_source"),
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let j = tool_json(&call_tool(&pool, "get_tables", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], true);
        assert_eq!(j["source"], "tex");
        assert_eq!(j["count"], 1);
        assert_eq!(j["truncated"], false);
        let t = &j["tables"][0];
        assert_eq!(t["node_id"], table);
        assert_eq!(t["caption"]["node_id"], caption);
        assert_eq!(t["caption"]["text"], "Particle masses.");
        assert_eq!(t["column_spec"], "lc");
        assert_eq!(t["n_columns"], 2);
        assert_eq!(t["n_rows"], 2);
        assert_eq!(t["alignments"], json!(["l", "c"]));
        assert_eq!(t["rows"][0]["cells"][0]["text"], "Particle");
        assert_eq!(t["rows"][1]["cells"][1]["text"], "$0.511$");
        assert_eq!(t["rows"][1]["rule_above"], true);
        // 原文スニペットは返さない（rows が構造を持つ・二重送出の抑制）。
        assert!(t.get("latex_source").is_none());

        // blocks 経由では寸法つきの可読テキストとして流れる（セル構造は get_tables へ誘導）。
        let blocks = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry.id, "kinds": ["table"] }),
            )
            .await,
        );
        assert_eq!(blocks["returned"], 1);
        assert_eq!(blocks["blocks"][0]["n_columns"], 2);
        assert_eq!(blocks["blocks"][0]["n_rows"], 2);
        assert_eq!(blocks["blocks"][0]["column_spec"], "lc");
        assert_eq!(blocks["blocks"][0]["text"], "Particle | Mass\ne | $0.511$");
    }

    /// Phase 8b: TeX 版が無い（PDF のみ）エントリでは get_tables は has_lcir:false を返すこと。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_tables_requires_tex_version(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Pdf only".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-pdf",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();

        let j = tool_json(&call_tool(&pool, "get_tables", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], false);
        assert!(j["message"].as_str().unwrap().contains("TeX"));
    }

    /// Phase 8b (wip 修正): 予算打ち切りは**連続**であること（途中の大表だけ飛ばして後続の小表を
    /// 拾う歯抜けを作らない）。小・大・小の 3 表で 2 番目に予算超過したら 3 番目も返さない。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_tables_truncation_is_contiguous(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Many tables".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/s.gz", entry.id),
            "s.gz",
            "application/gzip",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-many",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/gzip",
                extractor_name: schema::TEX_EXTRACTOR_NAME,
                extractor_version: schema::TEX_EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("tex_source"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let big = "x".repeat(4000);
        let payload = r#"{"n_columns":1,"n_rows":1,"rows":[]}"#;
        let plains = ["a | b", big.as_str(), "c | d"];
        let mut ids = Vec::new();
        for (i, pt) in plains.iter().enumerate() {
            let id = crate::db::document_nodes::insert_node(
                &pool,
                &crate::db::document_nodes::NewDocumentNode {
                    document_version_id: vid,
                    parent_id: Some(root),
                    node_kind: NodeKind::Table.as_str(),
                    ordinal: (i + 1) as i64,
                    plain_text: Some(pt),
                    language: None,
                    confidence: Some(0.9),
                    origin: Some("tex_source"),
                    payload_json: Some(payload),
                },
            )
            .await
            .unwrap();
            ids.push(id);
        }

        let j = tool_json(
            &call_tool(
                &pool,
                "get_tables",
                json!({ "entry_id": entry.id, "max_chars": 1000 }),
            )
            .await,
        );
        assert_eq!(j["count"], 3, "総数は 3");
        assert_eq!(j["truncated"], true);
        let tables = j["tables"].as_array().unwrap();
        assert_eq!(tables.len(), 1, "打ち切りは連続 — 先頭の小表 1 個のみ");
        assert_eq!(tables[0]["node_id"], ids[0]);
    }

    /// Phase 8b (wip 修正): 8b 前の LCIR は table ノードを持たない。count 0 かつ抽出器版が古いとき
    /// は「表が無い論文」と誤読させず、再構築を案内する note を返す。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_tables_notes_outdated_extractor_when_empty(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Old lcir".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/s.gz", entry.id),
            "s.gz",
            "application/gzip",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-old",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/gzip",
                extractor_name: schema::TEX_EXTRACTOR_NAME,
                extractor_version: "0.4.0", // 8b 前の版（table ノード無し）
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("tex_source"),
                payload_json: None,
            },
        )
        .await
        .unwrap();

        let j = tool_json(&call_tool(&pool, "get_tables", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], true);
        assert_eq!(j["count"], 0);
        assert_eq!(j["extractor_version"], "0.4.0");
        let note = j["note"].as_str().unwrap();
        assert!(note.contains("Rebuild"), "{note}");
        assert!(note.contains("0.4.0"), "{note}");
    }

    /// Phase 6b: get_symbol_definitions が TeX 版の記号定義を defined_at/scope/occurrences つきで返し、
    /// symbol/query で絞れること。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_symbol_definitions_returns_symbols_with_scope_and_occurrences(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Tex paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/p.gz", entry.id),
            "p.gz",
            "application/gzip",
        )
        .await
        .unwrap()
        .id;
        // TeX 版（記号系は TeX のみ）。
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/gzip",
                extractor_name: schema::TEX_EXTRACTOR_NAME,
                extractor_version: schema::TEX_EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let mk = |kind: &'static str, ord: i64, text: &'static str, payload: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                crate::db::document_nodes::insert_node(
                    &pool,
                    &crate::db::document_nodes::NewDocumentNode {
                        document_version_id: vid,
                        parent_id: None,
                        node_kind: kind,
                        ordinal: ord,
                        plain_text: Some(text),
                        language: None,
                        confidence: Some(0.9),
                        origin: Some("tex_source"),
                        payload_json: payload,
                    },
                )
                .await
                .unwrap()
            }
        };
        let sec = mk("section", 0, "2 Preliminaries", Some(r#"{"section_number":"2"}"#)).await;
        let para = mk("paragraph", 1, "Let $U$ be the time evolution operator.", None).await;
        let eq = mk("display_math", 2, "U = S_2 C_2 S_1 C_1", None).await;
        crate::db::math_expressions::insert_math(
            &pool,
            &crate::db::math_expressions::NewMathExpression {
                node_id: eq,
                display_mode: "display",
                equation_label: Some("(1.1)"),
                latex: Some("U = S_2 C_2 S_1 C_1"),
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("U = S_2 C_2 S_1 C_1"),
                ast_json: None,
                semantic_status: "source_provided",
                confidence: Some(0.98),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();
        let sid = crate::db::symbols::insert_symbol(
            &pool,
            &crate::db::symbols::NewSymbol {
                document_version_id: vid,
                surface_form: "U",
                normalized_form: Some("U"),
                description: Some("the time evolution operator"),
                symbol_type: Some("operator"),
                defined_at_node_id: Some(para),
                scope_node_id: Some(sec),
                semantic_json: None,
                confidence: Some(0.6),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();
        crate::db::symbols::insert_occurrence(
            &pool,
            &crate::db::symbols::NewSymbolOccurrence {
                symbol_id: sid,
                node_id: eq,
                local_offset_json: None,
                surface_form: "U",
                confidence: Some(0.5),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();

        let j = tool_json(
            &call_tool(&pool, "get_symbol_definitions", json!({ "entry_id": entry.id })).await,
        );
        assert_eq!(j["has_lcir"], true);
        assert_eq!(j["source"], "tex");
        assert_eq!(j["count"], 1);
        let s = &j["symbols"][0];
        assert_eq!(s["surface_form"], "U");
        assert_eq!(s["description"], "the time evolution operator");
        assert_eq!(s["symbol_type"], "operator");
        assert_eq!(s["defined_at"]["node_id"], para);
        assert_eq!(s["defined_at"]["kind"], "paragraph");
        assert_eq!(s["scope"]["section_number"], "2");
        assert_eq!(s["occurrence_count"], 1);
        assert_eq!(s["occurrences"][0]["equation_label"], "(1.1)");

        // query / symbol フィルタ。
        let hit = tool_json(
            &call_tool(
                &pool,
                "get_symbol_definitions",
                json!({ "entry_id": entry.id, "query": "evolution" }),
            )
            .await,
        );
        assert_eq!(hit["count"], 1);
        let miss = tool_json(
            &call_tool(
                &pool,
                "get_symbol_definitions",
                json!({ "entry_id": entry.id, "query": "zzz-nope" }),
            )
            .await,
        );
        assert_eq!(miss["count"], 0);
        let exact = tool_json(
            &call_tool(
                &pool,
                "get_symbol_definitions",
                json!({ "entry_id": entry.id, "symbol": "U" }),
            )
            .await,
        );
        assert_eq!(exact["count"], 1);
    }

    /// Phase 5 完了条件「定理と証明を一つの問い合わせで取得できる」: `kinds` フィルタで
    /// theorem + proof をまとめて取り、定理番号・付記名が surface されること。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_document_blocks_returns_theorem_and_proof_with_number(pool: SqlitePool) {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Math paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            &pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = crate::db::document_nodes::insert_node(
            &pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("full page text"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        add_block(&pool, vid, page, "paragraph", 0, "Introductory prose.", None).await;
        add_block(
            &pool,
            vid,
            page,
            "theorem",
            1,
            "Every bounded sequence has a convergent subsequence.",
            Some(r#"{"theorem_number":"2.3","note":"Bolzano--Weierstrass"}"#),
        )
        .await;
        add_block(&pool, vid, page, "proof", 2, "Consider a monotone subsequence.", None).await;

        let j = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry.id, "kinds": ["theorem", "proof"] }),
            )
            .await,
        );
        assert_eq!(j["total_blocks"], 2, "定理 + 証明だけを取れる");
        let blocks = j["blocks"].as_array().unwrap();
        let thm = blocks.iter().find(|b| b["kind"] == "theorem").unwrap();
        assert_eq!(thm["theorem_number"], "2.3");
        assert_eq!(thm["note"], "Bolzano--Weierstrass");
        assert!(blocks.iter().any(|b| b["kind"] == "proof"));

        // 構造カウントにも定理系が汎用的に現れる。
        let s = tool_json(&call_tool(&pool, "get_document_structure", json!({ "entry_id": entry.id })).await);
        assert_eq!(s["counts"]["theorem"], 1);
        assert_eq!(s["counts"]["proof"], 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_document_nodes_finds_block_with_bbox(pool: SqlitePool) {
        setup_entry_with_lcir(&pool).await;
        let resp = call_tool(&pool, "search_document_nodes", json!({ "query": "quantum walks" })).await;
        let j = tool_json(&resp);
        assert!(j["count"].as_i64().unwrap() >= 1);
        let hit = &j["results"][0];
        assert!(hit["node_kind"].is_string());
        assert_eq!(hit["page"], 1);
        // bbox が [x,y,w,h] で返る（ハイライト用）。
        assert!(hit["bbox"].as_array().map(|a| a.len() == 4).unwrap_or(false));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn document_tools_report_has_lcir_false_without_lcir(pool: SqlitePool) {
        // LCIR 未構築のエントリ → has_lcir:false（get_fulltext に退避可能）。
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "No LCIR".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let j = tool_json(&call_tool(&pool, "get_document_structure", json!({ "entry_id": entry.id })).await);
        assert_eq!(j["has_lcir"], false);
    }

    /// 既存エントリに TeX 由来の LCIR（別添付・lumencite-tex・原文 LaTeX・fragment 無し）を足す。
    async fn add_tex_lcir(pool: &SqlitePool, entry_id: i64) -> i64 {
        use crate::document_ir::{schema, ExtractionStatus, NodeKind};
        let att = crate::db::attachments::add_attachment(
            pool,
            entry_id,
            &format!("attachments/{entry_id}/arxiv-src.gz"),
            "arxiv-src.gz",
            crate::ingestion::TEX_SOURCE_MIME,
        )
        .await
        .unwrap()
        .id;
        let vid = crate::db::document_versions::insert_version(
            pool,
            &crate::db::document_versions::NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-tex",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha-tex",
                source_mime_type: crate::ingestion::TEX_SOURCE_MIME,
                extractor_name: schema::TEX_EXTRACTOR_NAME,
                extractor_version: schema::TEX_EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = crate::db::document_nodes::insert_node(
            pool,
            &crate::db::document_nodes::NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("tex_source"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let add = |kind: &'static str, ordinal: i64, text: &'static str, payload: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                crate::db::document_nodes::insert_node(
                    &pool,
                    &crate::db::document_nodes::NewDocumentNode {
                        document_version_id: vid,
                        parent_id: Some(root),
                        node_kind: kind,
                        ordinal,
                        plain_text: Some(text),
                        language: None,
                        confidence: Some(0.95),
                        origin: Some("tex_source"),
                        payload_json: payload,
                    },
                )
                .await
                .unwrap()
            }
        };
        add("abstract", 0, "We study quantum walks from source.", None).await;
        add(
            "section",
            1,
            "1 Introduction",
            Some(r#"{"heading_level":1,"section_number":"1"}"#),
        )
        .await;
        let eq = add("display_math", 2, "U = S_2 C_2 S_1 C_1", None).await;
        crate::db::math_expressions::insert_math(
            pool,
            &crate::db::math_expressions::NewMathExpression {
                node_id: eq,
                display_mode: "display",
                equation_label: None,
                latex: Some("\\begin{equation}U = S_2 C_2 S_1 C_1\\end{equation}"),
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("U = S_2 C_2 S_1 C_1"),
                ast_json: None,
                semantic_status: "source_provided",
                confidence: Some(0.98),
                origin: Some("tex_source"),
            },
        )
        .await
        .unwrap();
        att
    }

    /// Phase 4: PDF 版と TeX 版が併存するとき、既定では TeX 版が選ばれ（優先順位）、
    /// available_sources に両方が載り、数式は原文 LaTeX を返す。
    #[sqlx::test(migrations = "./migrations")]
    async fn document_tools_prefer_tex_source_and_expose_latex(pool: SqlitePool) {
        let entry_id = setup_entry_with_lcir(&pool).await;
        add_tex_lcir(&pool, entry_id).await;

        let s = tool_json(&call_tool(&pool, "get_document_structure", json!({ "entry_id": entry_id })).await);
        assert_eq!(s["has_lcir"], true);
        assert_eq!(s["source"], "tex");
        assert!(s["page_count"].is_null(), "TeX 版に page は無い: {}", s["page_count"]);
        assert!(s["block_count"].as_i64().unwrap() >= 3);
        let sources = s["available_sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2, "{sources:?}");
        assert_eq!(sources[0]["source"], "tex", "優先度順（tex が先頭）");
        assert_eq!(sources[1]["source"], "pdf");
        assert!(s["note"].as_str().unwrap().contains("TeX source"));

        let b = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry_id, "kinds": ["display_math"] }),
            )
            .await,
        );
        assert_eq!(b["source"], "tex");
        let blocks = b["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert!(
            blocks[0]["latex"].as_str().unwrap().contains("\\begin{equation}"),
            "TeX 由来は原文 LaTeX を返す: {}",
            blocks[0]["latex"]
        );
    }

    /// Phase 4: source="pdf" で明示切替でき、page_count が数値に戻る。
    #[sqlx::test(migrations = "./migrations")]
    async fn document_tools_source_pdf_override(pool: SqlitePool) {
        let entry_id = setup_entry_with_lcir(&pool).await;
        add_tex_lcir(&pool, entry_id).await;

        let s = tool_json(
            &call_tool(
                &pool,
                "get_document_structure",
                json!({ "entry_id": entry_id, "source": "pdf" }),
            )
            .await,
        );
        assert_eq!(s["source"], "pdf");
        assert_eq!(s["page_count"], 1);

        // 無い source を明示すると has_lcir:false + 明示メッセージ。
        let entry2 = create_entry(
            &pool,
            &EntryInput {
                title: "PDF only".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let none = tool_json(
            &call_tool(
                &pool,
                "get_document_structure",
                json!({ "entry_id": entry2.id, "source": "tex" }),
            )
            .await,
        );
        assert_eq!(none["has_lcir"], false);
        assert!(none["message"].as_str().unwrap().contains("'tex'"));
    }

    /// Phase 4: `page` は PDF 空間の概念 — source 未指定なら pdf 版へ自動フォールバックし、
    /// source="tex" と併用したら明示メッセージ（黙って 0 件にしない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn document_blocks_page_filter_falls_back_to_pdf(pool: SqlitePool) {
        let entry_id = setup_entry_with_lcir(&pool).await;
        add_tex_lcir(&pool, entry_id).await;

        let b = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry_id, "page": 1 }),
            )
            .await,
        );
        assert_eq!(b["source"], "pdf", "page 指定で pdf 版に自動フォールバック");
        assert!(b["total_blocks"].as_i64().unwrap() >= 1);

        let t = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry_id, "page": 1, "source": "tex" }),
            )
            .await,
        );
        assert_eq!(t["total_blocks"], 0);
        assert!(t["message"].as_str().unwrap().contains("no page mapping"));
    }

    /// 手動 E2E: 実 DB コピー + 実 PDF を pdfium で LCIR 構築し、外部 LLM が MCP で受け取る
    /// JSON（get_document_structure / get_document_blocks / search_document_nodes）を印字する。
    /// native lib が要るため `#[ignore]`。env 未設定なら skip。ATT の entry を対象にする。
    /// 例:
    /// `LCIR_SMOKE_DB=/path/copy.db LCIR_SMOKE_APPDIR="$HOME/Library/Application Support/com.lumencite.app" \
    ///  LCIR_SMOKE_ATT=8 cargo test --lib mcp_lcir_tools_e2e -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "manual pdfium E2E; needs LCIR_SMOKE_* env + libpdfium"]
    async fn mcp_lcir_tools_e2e() {
        let (db, appdir, att) = match (
            std::env::var("LCIR_SMOKE_DB"),
            std::env::var("LCIR_SMOKE_APPDIR"),
            std::env::var("LCIR_SMOKE_ATT"),
        ) {
            (Ok(d), Ok(a), Ok(t)) => (d, a, t.parse::<i64>().expect("LCIR_SMOKE_ATT must be int")),
            _ => {
                eprintln!("skip: set LCIR_SMOKE_DB / LCIR_SMOKE_APPDIR / LCIR_SMOKE_ATT");
                return;
            }
        };
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        crate::db::settings::set_setting(&pool, crate::db::settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
        // 一時 appdir に対象添付だけをコピーして build する（実 appdir へ書き込まない・
        // lcir_build_real_pdf と同方式・Phase 8a のアセット書き出しを隔離）。
        let (file_path,): (String,) =
            sqlx::query_as("SELECT file_path FROM attachments WHERE id = ?")
                .bind(att)
                .fetch_one(&pool)
                .await
                .unwrap();
        let build_root = std::env::temp_dir().join(format!(
            "lumencite-mcp-smoke-{att}-{}",
            std::process::id()
        ));
        let dest = build_root.join(&file_path);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(Path::new(&appdir).join(&file_path), &dest).unwrap();
        // 実 PDF を LCIR 構築（既存なら reuse）。
        let build = crate::ingestion::build_lcir_for_attachment(&pool, &build_root, att)
            .await
            .unwrap();
        eprintln!("build: built={} reused={}", build.built, build.reused);
        let entry_id: i64 = sqlx::query_scalar("SELECT entry_id FROM attachments WHERE id = ?")
            .bind(att)
            .fetch_one(&pool)
            .await
            .unwrap();

        let structure = tool_json(
            &call_tool(&pool, "get_document_structure", json!({ "entry_id": entry_id })).await,
        );
        eprintln!(
            "\n=== get_document_structure ===\n{}",
            serde_json::to_string_pretty(&structure).unwrap()
        );

        let eqs = tool_json(
            &call_tool(
                &pool,
                "get_document_blocks",
                json!({ "entry_id": entry_id, "kinds": ["display_math"], "max_chars": 1500 }),
            )
            .await,
        );
        eprintln!(
            "\n=== get_document_blocks kinds=[display_math] (first ~1500 chars) ===\n{}",
            serde_json::to_string_pretty(&eqs).unwrap()
        );

        let found = tool_json(
            &call_tool(&pool, "search_document_nodes", json!({ "query": "wave operator" })).await,
        );
        eprintln!(
            "\n=== search_document_nodes 'wave operator' ===\n{}",
            serde_json::to_string_pretty(&found).unwrap()
        );

        // Phase 6a: 参照グラフ全体と、proves だけ絞った結果。
        let rels = tool_json(&call_tool(&pool, "get_node_relations", json!({ "entry_id": entry_id })).await);
        eprintln!(
            "\n=== get_node_relations (counts_by_type) ===\n{}",
            serde_json::to_string_pretty(&rels["counts_by_type"]).unwrap()
        );
        let proves = tool_json(
            &call_tool(
                &pool,
                "get_node_relations",
                json!({ "entry_id": entry_id, "relation_type": ["proves"] }),
            )
            .await,
        );
        eprintln!(
            "\n=== get_node_relations relation_type=[proves] (first few) ===\n{}",
            serde_json::to_string_pretty(&proves["relations"]).unwrap()
        );

        // Phase 6b: 記号定義（TeX 版がある場合のみ非空）。
        let syms = tool_json(&call_tool(&pool, "get_symbol_definitions", json!({ "entry_id": entry_id })).await);
        eprintln!(
            "\n=== get_symbol_definitions (source={}, count={}) first few ===",
            syms["source"], syms["count"]
        );
        for s in syms["symbols"].as_array().map(|a| a.as_slice()).unwrap_or(&[]).iter().take(12) {
            eprintln!(
                "  [{}] type={} conf={} occ={} desc={}",
                s["surface_form"], s["symbol_type"], s["confidence"], s["occurrence_count"], s["description"]
            );
        }

        // Phase 8a: 図一覧（tikz ベクター図の論文では count 0 が正当なので数はアサートしない）。
        let figs = tool_json(&call_tool(&pool, "get_figures", json!({ "entry_id": entry_id })).await);
        eprintln!(
            "\n=== get_figures (source={}, count={}) ===",
            figs["source"], figs["count"]
        );
        for f in figs["figures"].as_array().map(|a| a.as_slice()).unwrap_or(&[]).iter().take(8) {
            eprintln!(
                "  [figure] page={} number={} bbox={} caption={} assets={}",
                f["page"],
                f["figure_number"],
                f["bbox"],
                f["caption"]["text"],
                f["assets"],
            );
        }

        assert_eq!(structure["has_lcir"], true);
        assert!(structure["counts"]["display_math"].as_i64().unwrap_or(0) > 0);
        assert_eq!(rels["has_lcir"], true);
        assert_eq!(syms["has_lcir"], true);
        assert_eq!(figs["has_lcir"], true);

        let _ = std::fs::remove_dir_all(&build_root);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_entry_includes_citation_key(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("doe2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let resp = call_tool(&pool, "get_entry", json!({ "entry_id": id })).await;
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["citation_key"], "doe2020");
        assert_eq!(parsed["resolved_citation_key"], "doe2020");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_resolve_citation_key(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("smith2021".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let resp = call_tool(&pool, "resolve_citation_key", json!({ "entry_id": id })).await;
        assert_eq!(resp["result"]["isError"], false);
        assert_eq!(resp["result"]["content"][0]["text"], "smith2021");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_export_bibtex_returns_bib(pool: SqlitePool) {
        create_entry(
            &pool,
            &EntryInput {
                title: "Exported".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("exp2022".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let resp = call_tool(&pool, "export_bibtex", json!({})).await;
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("exp2022"), "bib should contain the cite key: {text}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_entry_by_citation_key(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("doe2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        // entry_id を知らなくても cite key だけで引ける。
        let resp = call_tool(&pool, "get_entry", json!({ "citation_key": "doe2020" })).await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["id"], id);
        assert_eq!(parsed["citation_key"], "doe2020");

        // 未知キーは（エラーではなく）見つからない旨のメッセージ。
        let miss = call_tool(&pool, "get_entry", json!({ "citation_key": "nope1999" })).await;
        assert_eq!(miss["result"]["isError"], false);
        assert!(miss["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no entry found"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_find_entries_by_citation_keys(pool: SqlitePool) {
        let id = create_entry(
            &pool,
            &EntryInput {
                title: "Findable".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("wong2019".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id;

        let resp = call_tool(
            &pool,
            "find_entries_by_citation_keys",
            json!({ "citation_keys": ["wong2019", "missing2000"] }),
        )
        .await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        let results = parsed["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["found"], true);
        assert_eq!(results[0]["entry_id"], id);
        assert_eq!(results[0]["title"], "Findable");
        assert_eq!(results[1]["found"], false);
        assert_eq!(results[1]["citation_key"], "missing2000");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_export_bibtex_by_citation_keys(pool: SqlitePool) {
        create_entry(
            &pool,
            &EntryInput {
                title: "Wanted".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("keep2021".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        create_entry(
            &pool,
            &EntryInput {
                title: "Unwanted".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("skip2021".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let resp = call_tool(
            &pool,
            "export_bibtex",
            json!({ "citation_keys": ["keep2021", "ghost2000"] }),
        )
        .await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(parsed["bibtex"].as_str().unwrap().contains("keep2021"));
        assert!(!parsed["bibtex"].as_str().unwrap().contains("skip2021"));
        assert_eq!(parsed["found"], json!(["keep2021"]));
        assert_eq!(parsed["missing"], json!(["ghost2000"]));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_fulltext_by_key_and_missing(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Full Paper".to_string(),
                entry_type: "article".to_string(),
                citation_key: Some("full2020".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            "attachments/x/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();
        crate::db::fulltext::index_attachment(
            &pool,
            att.id,
            &[
                (1, "Introduction to widgets.".to_string()),
                (2, "Widget conclusions.".to_string()),
            ],
        )
        .await
        .unwrap();

        // citation_key で全文取得できる。
        let resp = call_tool(&pool, "get_fulltext", json!({ "citation_key": "full2020" })).await;
        assert_eq!(resp["result"]["isError"], false);
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["indexed"], true);
        assert_eq!(parsed["total_pages"], 2);
        assert_eq!(parsed["truncated"], false);
        let text = parsed["text"].as_str().unwrap();
        assert!(text.contains("widgets"));
        assert!(text.contains("conclusions"));

        // PDF 未索引のエントリは indexed:false（捏造させないための明示シグナル）。
        let bare = create_entry(
            &pool,
            &EntryInput {
                title: "No PDF".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let resp2 = call_tool(&pool, "get_fulltext", json!({ "entry_id": bare.id })).await;
        let parsed2: Value =
            serde_json::from_str(resp2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed2["indexed"], false);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_get_fulltext_paginates(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Long".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = crate::db::attachments::add_attachment(
            &pool,
            entry.id,
            "attachments/y/p.pdf",
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();
        crate::db::fulltext::index_attachment(
            &pool,
            att.id,
            &[
                (1, "a".repeat(3000)),
                (2, "b".repeat(3000)),
                (3, "c".repeat(3000)),
            ],
        )
        .await
        .unwrap();

        // max_chars を小さくすると 1 ページで打ち切り、next_page=2 が返る。
        let resp = call_tool(
            &pool,
            "get_fulltext",
            json!({ "entry_id": entry.id, "max_chars": 1000 }),
        )
        .await;
        let parsed: Value =
            serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed["truncated"], true);
        assert_eq!(parsed["next_page"], 2);

        // page_start=2 で続きから読める。
        let resp2 = call_tool(
            &pool,
            "get_fulltext",
            json!({ "entry_id": entry.id, "max_chars": 1000, "page_start": 2 }),
        )
        .await;
        let parsed2: Value =
            serde_json::from_str(resp2["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(parsed2["returned_from_page"], 2);
        assert_eq!(parsed2["next_page"], 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_search_entries(pool: SqlitePool) {
        create_entry(
            &pool,
            &EntryInput {
                title: "Quantum Computing Survey".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let resp = call_tool(&pool, "search_entries", json!({ "query": "quantum" })).await;
        assert_eq!(resp["result"]["isError"], false);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(parsed["count"].as_i64().unwrap() >= 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_call_forbidden_mutate_tool_is_error(pool: SqlitePool) {
        // write 系はサーバーに公開されておらず、呼んでも isError で弾かれる。
        let resp = call_tool(&pool, "create_entry", json!({ "title": "X" })).await;
        assert_eq!(resp["result"]["isError"], true);
        // 実際に作成されていないこと。
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_method_returns_jsonrpc_error(pool: SqlitePool) {
        let resp = handle_rpc(&pool, Path::new(""), &req("frobnicate", json!({})))
            .await
            .response
            .unwrap();
        assert_eq!(resp["error"]["code"], -32601);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn notification_without_id_returns_none(pool: SqlitePool) {
        let notif = json!({ "jsonrpc": "2.0", "method": "notifications/initialized", "params": {} });
        let outcome = handle_rpc(&pool, Path::new(""), &notif).await;
        assert!(outcome.response.is_none());
        assert!(!outcome.mutated);
    }

    // ── Phase 2: write gate ────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn tools_list_includes_write_tools_when_enabled(pool: SqlitePool) {
        enable_writes(&pool).await;
        let resp = handle_rpc(&pool, Path::new(""), &req("tools/list", json!({})))
            .await
            .response
            .unwrap();
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for expected in ["add_tag", "update_notes", "add_to_collection", "create_entry", "update_entry"] {
            assert!(names.contains(&expected), "missing write tool: {expected}");
        }
        // 破壊系は write 有効でも公開しない。
        assert!(!names.contains(&"delete_entry"), "delete_entry must never be exposed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn write_tool_blocked_when_disabled_and_no_mutation(pool: SqlitePool) {
        // 既定（無効）では create_entry は isError、mutated=false、DB 変化なし。
        let r = req("tools/call", json!({ "name": "create_entry", "arguments": { "title": "X" } }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        assert_eq!(outcome.response.unwrap()["result"]["isError"], true);
        assert!(!outcome.mutated);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries").fetch_one(&pool).await.unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn write_create_entry_when_enabled_mutates_and_audits(pool: SqlitePool) {
        enable_writes(&pool).await;
        let r = req("tools/call", json!({ "name": "create_entry", "arguments": { "title": "Made via MCP", "entry_type": "article" } }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        assert_eq!(outcome.response.unwrap()["result"]["isError"], false);
        assert!(outcome.mutated, "successful write must set mutated=true");

        // エントリが作成された。
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE title = ?")
            .bind("Made via MCP").fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);

        // 監査ログに記録された。
        let audit = crate::db::mcp_audit::recent(&pool, 10).await.unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].tool_name, "create_entry");
        assert!(!audit[0].is_error);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_never_exposed_even_when_writes_enabled(pool: SqlitePool) {
        enable_writes(&pool).await;
        let id = create_entry(
            &pool,
            &EntryInput { title: "Keep".to_string(), entry_type: "article".to_string(), ..Default::default() },
        ).await.unwrap().id;

        let r = req("tools/call", json!({ "name": "delete_entry", "arguments": { "entry_id": id } }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        // 許可リスト外 → isError、mutated=false、エントリは残る。
        assert_eq!(outcome.response.unwrap()["result"]["isError"], true);
        assert!(!outcome.mutated);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries WHERE id = ?")
            .bind(id).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn read_tool_does_not_mutate(pool: SqlitePool) {
        let r = req("tools/call", json!({ "name": "list_tags", "arguments": {} }));
        let outcome = handle_rpc(&pool, Path::new(""), &r).await;
        assert!(!outcome.mutated);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn generate_token_is_nonempty_hex(pool: SqlitePool) {
        let token = generate_token(&pool).await.unwrap();
        assert_eq!(token.len(), 48);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// HTTP トランスポート全体の疎通: バインド → 認可 → JSON-RPC 応答。
    #[sqlx::test(migrations = "./migrations")]
    async fn http_server_serves_tools_list_with_bearer_auth(pool: SqlitePool) {
        let manager = McpServerManager::default();
        let token = "test-token-abc".to_string();
        let (sync_tx, _sync_rx) = tokio::sync::mpsc::unbounded_channel();
        let deps = ServerDeps {
            pool: pool.clone(),
            app_data_dir: PathBuf::from(""),
            sync_tx,
            app: None, // テストでは UI イベントを発火しない
        };
        // port 0 で OS 割り当て。実バインドポートが返る。
        let port = manager.start(deps, 0, token.clone()).expect("server should bind");
        let url = format!("http://127.0.0.1:{port}/mcp");
        let client = reqwest::Client::new();
        let body = req("tools/list", json!({}));

        // 認可ヘッダ無し → 401
        let resp = client.post(&url).json(&body).send().await.unwrap();
        assert_eq!(resp.status(), 401);

        // 正しい Bearer → 200 + ツール一覧
        let resp = client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let parsed: Value = resp.json().await.unwrap();
        let names: Vec<&str> = parsed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"export_bibtex"));

        manager.stop();
    }
}

#[cfg(test)]
mod block_on_mechanism_tests {
    use super::*;

    /// serve_loop 相当（素の std::thread 上の tauri::async_runtime::block_on）で
    /// reqwest + tokio::time::timeout が機能するかの機構テスト（ローカル fixture）。
    #[test]
    fn reqwest_inside_block_on_on_foreign_thread_works() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let _ = req.respond(tiny_http::Response::from_string("hello"));
            }
        });
        let url = format!("http://127.0.0.1:{port}/x");

        let handle = std::thread::spawn(move || {
            tauri::async_runtime::block_on(async move {
                tokio::time::timeout(Duration::from_secs(5), async {
                    reqwest::get(&url).await.unwrap().text().await.unwrap()
                })
                .await
            })
        });
        let result = handle.join().unwrap();
        assert_eq!(result.expect("must not time out"), "hello");
    }
}
