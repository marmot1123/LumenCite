//! 文献本文を読む read ツール群（`get_fulltext` + LCIR 8 種）。
//!
//! **チャットと MCP サーバーの単一ソース**。定義（[`specs`]）と実行（[`try_execute`]）を
//! ここに置き、`mcp_server` は `tool_specs` / `exec_tool` からこのモジュールへ委譲する
//! （`search` / `mutate` と同じ関係）。Phase 10b でチャット側へ露出するにあたり、
//! 定義の二重管理を避けるため `mcp_server` から移設した。
//!
//! ツールの内訳:
//! - `get_fulltext` — 索引済み PDF の平文（LCIR 非依存）
//! - `get_document_structure` / `get_document_blocks` / `search_document_nodes` — LCIR の構造読み
//! - `get_node_relations` / `get_symbol_definitions` — 参照グラフ・記号（Phase 6）
//! - `get_figures` / `get_tables` — 図・表（Phase 8）
//! - `get_node_context` — 1 ブロックの読解文脈バンドル（Phase 10a）
//!
//! # スコープ（CR-024）
//!
//! チャットは `scope_mode="entries"` のとき対象エントリしか読めない。**`entry_id` が確定した
//! 時点で必ず [`ToolContext::ensure_entry_in_scope`] を通す**こと（`resolve_entry_id` を
//! 使わない `get_fulltext`、entry を引数に取らない `get_node_context` も含めて漏れなく）。
//! MCP 経路は `mcp_ctx` が `scope_mode="all"` 固定なので、これらの検査はすべて no-op になり
//! 挙動は変わらない。
//!
//! # 応答の大きさ
//!
//! チャットではツール結果が会話履歴に永続化され、以後のターンで毎回再送される
//! （`llm::chat::run_chat_loop`）。MCP の「1 回払えば済む」前提とは違うので、
//! **全ツールが件数/文字数の上限を持つ**必要がある。

use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::llm::tools::{ToolContext, ToolError};
use crate::llm::{ToolCallSpec, ToolSpec};

/// `search_document_nodes` の既定件数と上限。**上限が必要な理由はチャット側にある** —
/// node-FTS はブロック粒度なので実ライブラリでは 1 語が数千行に当たり、
/// `search_nodes` は行ごとに 2 クエリ引く N+1 でもある。
const DEFAULT_MAX_SEARCH_RESULTS: i64 = 50;
const MAX_SEARCH_RESULTS: i64 = 200;

/// このモジュールが提供するツール名。`mcp_server` の委譲判定にも使う。
pub const DOCUMENT_TOOLS: &[&str] = &[
    "get_fulltext",
    "get_document_structure",
    "get_document_blocks",
    "search_document_nodes",
    "get_node_relations",
    "get_symbol_definitions",
    "get_figures",
    "get_tables",
    "get_node_context",
];

/// LCIR に依存するツール（`lcir.enabled` が OFF のときチャットの一覧から隠す対象）。
/// `get_fulltext` は LCIR と無関係なので含めない。
pub const LCIR_TOOLS: &[&str] = &[
    "get_document_structure",
    "get_document_blocks",
    "search_document_nodes",
    "get_node_relations",
    "get_symbol_definitions",
    "get_figures",
    "get_tables",
    "get_node_context",
];

/// このモジュールのツール定義。
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_fulltext".to_string(),
            description: "Return the extracted full text of a library entry's indexed PDF, by \
                entry_id or citation_key. Use this to actually read and summarise a specific paper — \
                `get_entry` only returns metadata (abstract / notes), which are often empty. Returns \
                {entry_id, indexed, total_pages, truncated, next_page, text}. If the entry has no \
                attached/indexed PDF, `indexed` is false and there is no text — say so plainly and do \
                NOT answer from general knowledge. Long papers are paginated: pass `page_start` (from a \
                previous `next_page`) to keep reading, or raise `max_chars`.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": { "type": "integer", "description": "Entry id." },
                    "citation_key": { "type": "string", "description": "Citation key (as in \\cite{}); alternative to entry_id." },
                    "max_chars": { "type": "integer", "description": "Max characters to return this call (default 24000)." },
                    "page_start": { "type": "integer", "description": "1-based PDF page to start from, for continuing a long paper (default 1)." }
                }
            }),
            needs_approval: false,
        },
        // LCIR（機械可読中間形式）の read ツール（Phase 3.5）。実験フラグ lcir.enabled で
        // 構築された論文だけが対象。未構築なら has_lcir=false を返す（get_fulltext に退避可能）。
        ToolSpec {
            name: "get_document_structure".to_string(),
            description: "Return the logical structure (LCIR) of a paper — its section outline, \
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
                search_document_nodes to locate content.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": { "type": "integer", "description": "Entry id." },
                    "citation_key": { "type": "string", "description": "Citation key (as in \\cite{}); alternative to entry_id." },
                    "source": { "type": "string", "enum": ["tex", "pdf"], "description": "Force a representation: \"tex\" (arXiv TeX source; exact LaTeX) or \"pdf\" (PDF text layer; pages/bbox). Omit for the best available (tex preferred)." }
                }
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "get_document_blocks".to_string(),
            description: "Read a paper's content as structure-tagged blocks (LCIR) in reading order — \
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
                blocks:[{index, node_id, kind, page, origin, confidence, section_number?, equation_label?, \
                latex?, text}]}. \
                Pass a block's node_id to get_node_context to read it with its surrounding context.".to_string(),
            parameters: json!({
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
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "search_document_nodes".to_string(),
            description: "Search the library at BLOCK granularity (paragraph / heading / caption / \
                display equation) using the LCIR node index — finer than fulltext_search, which is page \
                granularity. Each hit reports the entry, node_kind, page, a snippet, and the PDF \
                bounding box (bbox = [x, y, width, height] in PDF points, bottom-left origin) so the \
                exact block can be located/highlighted. Use this to pinpoint where a concept, term or \
                equation appears across papers. Only covers papers whose PDF-derived LCIR has been \
                built (TeX-derived text is not in this index; read it via get_document_blocks). Hit \
                pages refer to the pdf representation — follow up with get_document_blocks(page=...) \
                which uses the pdf source automatically. Returns {count, truncated, results:[{entry_id, \
                title, year, node_kind, page, snippet, bbox}]}. Short or CJK queries fall back to \
                substring matching. Hits carry no `origin`; blocks in this index come from the pdf \
                text layer with layout_model structure — call get_node_context on a node_id when you \
                need per-node origin and confidence. On an empty result the response adds \
                `index_built`: when it is false nothing has been indexed yet, so use fulltext_search \
                instead of concluding the library has no match. `scope_filtered: true` means hits \
                were dropped because they fell outside the caller's current selection.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query (space-separated terms are ANDed)." },
                    "collection_id": { "type": "integer", "description": "Restrict to a collection id." },
                    "tag_id": { "type": "integer", "description": "Restrict to a tag id." },
                    "max_results": { "type": "integer", "description": "Max hits to return (default 50, max 200). Block-granularity matches are numerous; narrow the query rather than raising this." }
                },
                "required": ["query"]
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "get_node_relations".to_string(),
            description: "Return the cross-reference graph (LCIR) of a paper — typed directed edges \
                between its blocks — by entry_id or citation_key. Edges are resolved from the source: \
                \"tex\" (from \\ref/\\eqref/\\cite matched against \\label and \\bibitem keys — high \
                confidence, origin tex_source) or \"pdf\" (from \"Theorem 2.3\"/\"Eq. (2.1)\"/\"Figure \
                3\"/\"Fig. 3\"/\"Table 2\" strings matched against theorem/equation/figure/table \
                numbers — approximate, origin layout_model). tex is \
                preferred when built; pass `source` to switch. Relation types: cites, \
                refers_to_equation, refers_to_theorem, refers_to_figure, refers_to_table, \
                refers_to_section, refers_to, proves (proof → the theorem it proves), and caption_of \
                (a figure caption → its detected figure region, pdf only). Use it to \
                answer \"what does this proof prove\", \"what cites/uses equation (2.1)\", \"which \
                results does this section reference\". Figure/table edges point at the figure region \
                only when one was detected; otherwise they point at the caption block — \
                metadata.resolved_via is \"node\" or \"caption\", and caption_of gets you from the \
                caption to the region in one hop. Plural and range mentions (\"Figures 3 and 4\", \
                \"Figs. 1-3\") are deliberately left unresolved, so an absent edge does not mean the \
                text has no reference. Filter with `relation_type` and/or `node_id` \
                (edges touching that block, either direction). Returns {has_lcir, source, \
                available_sources, count, counts_by_type, relations:[{relation_type, confidence, \
                origin, from:{node_id,kind,page,snippet}, to:{node_id,kind,page,snippet, \
                theorem_number?, equation_label?, section_number?, labels?, figure_number?, \
                caption_number?}, metadata}]}. If has_lcir \
                is false nothing is built (build it in the app).".to_string(),
            parameters: json!({
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
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "get_symbol_definitions".to_string(),
            description: "Return the notation/symbol definitions (LCIR) a paper introduces — by \
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
                occurrences:[{node_id, equation_label}]}]}.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": { "type": "integer", "description": "Entry id." },
                    "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                    "symbol": { "type": "string", "description": "Exact surface_form to fetch (e.g. \"U\" or \"\\\\mathcal{H}\")." },
                    "query": { "type": "string", "description": "Case-insensitive substring over surface_form / normalized_form / description." },
                    "source": { "type": "string", "enum": ["tex", "pdf"], "description": "Force a representation. Symbols exist only for \"tex\"; omit for the best available (tex preferred)." },
                    "max_symbols": { "type": "integer", "description": "Max symbols to return (default 200)." }
                }
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "get_figures".to_string(),
            description: "Return the detected figures (LCIR) of a paper — by entry_id or \
                citation_key. **PDF-only**: figure regions are detected from embedded raster images on \
                each page (origin layout_model, moderate confidence), so vector figures (TikZ/pgf, \
                common in math papers) legitimately yield zero figures — an empty list does NOT mean \
                the paper has no figures. Each figure carries its page and bbox ([x, y, width, height] \
                in PDF points, bottom-left origin), the figure number when a nearby \"Figure N\" \
                caption was paired (caption_of edge), the caption text, and its stored assets \
                (page-crop PNGs). Asset relative_path is a path under the app data directory as \
                METADATA — the file's existence is not guaranteed and no image bytes are returned. \
                Use it to answer \"what figures does this paper have\", \"what does Figure 2 show\" \
                (caption text), \"where is Figure 2 on the page\" (page + bbox). A figure may also carry \
                alt_text — a description of the image itself that is NOT from the paper. **Read its \
                `origin` to decide how much to trust it**: `llm_inference` = generated by a vision \
                model (a hint only, never the authors' wording — prefer the caption when they \
                disagree); `user_edited` = written by the library owner (authoritative for what the \
                image shows). It is absent unless the user ran the opt-in generation batch in the app. \
                Returns {has_lcir, \
                source, available_sources, count, figures:[{node_id, page, bbox, figure_number?, \
                caption:{node_id, text}?, alt_text:{text, origin, confidence, model}?, \
                assets:[{role, relative_path, mime_type, width, height, \
                size_bytes}]}]}. If has_lcir is false no PDF version is built (build it in the app).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": { "type": "integer", "description": "Entry id." },
                    "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                    "max_figures": { "type": "integer", "description": "Max figures to return (default 100)." }
                }
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "get_tables".to_string(),
            description: "Return the structured tables (LCIR) of a paper — by entry_id or \
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
                alignments?, rows}]}.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "entry_id": { "type": "integer", "description": "Entry id." },
                    "citation_key": { "type": "string", "description": "Citation key; alternative to entry_id." },
                    "max_tables": { "type": "integer", "description": "Max tables to return (default 20)." },
                    "max_chars": { "type": "integer", "description": "Approximate budget over cell text (default 24000); further tables are truncated." }
                }
            }),
            needs_approval: false,
        },
        ToolSpec {
            name: "get_node_context".to_string(),
            description: "Assemble everything needed to READ AND CITE one block (LCIR) — by \
                node_id. Give it a theorem's node id and you get the statement (reassembled across \
                block and page breaks), the proof that proves it, the definitions it rests on, the \
                equations/figures/works it references, and the PDF region of every piece — in one \
                call. Node ids come from search_document_nodes, get_document_blocks, get_figures or \
                get_node_relations; the node itself selects the representation, so there is no \
                `source` argument. Why this exists: on the pdf representation a theorem node holds \
                only its FIRST layout block (~168 chars on average) and the rest of the statement lands \
                in sibling blocks that often continue onto the next page, so reading `focus` alone \
                loses the statement — read `focus` then `continuation` (blocks in reading order up to \
                the next structural boundary; on the tex representation the environment body is \
                already whole, so continuation is the text that FOLLOWS it). `premises` carries the \
                definitions the block depends on and each one states how it was derived in `via`: \
                \"reference\" = an explicit \\ref/\"Definition 3.1\" edge (rare but exact), \
                \"occurrence\" = a symbol recorded in a display equation, \"symbol\" = a symbol whose \
                surface form appears verbatim in the text and is defined earlier (tex only; both \
                symbol paths are heuristic associations — check confidence). Every node carries \
                origin and confidence, so you can tell source text (tex_source, pdf_text_layer) from \
                inference (layout_model, llm_inference) when you quote it. `proves` edges on the pdf \
                representation come mostly from reading-order adjacency, so they can point at a \
                remark or example — check node.kind. Figure/table references resolve the caption_of \
                hop for you: `figure` is the region (bbox, crop asset, alt text) and is often absent, \
                `caption` is the authors' wording. Read `notes` — it lists what this bundle could not \
                reach — and `continuation_stopped_at`, which says whether the continuation ended at \
                the next logical unit ({reason:\"boundary\", node_id, kind}) or at a size limit. A \
                boundary of kind figure_caption/table_caption means a float interrupted the text, not \
                that the statement ended. Returns {found:true, node_id, entry_id, citation_key, \
                attachment_id, version_id, source, extractor_version, available_sources, focus, \
                continuation_stopped_at} plus these keys, EACH OMITTED WHEN EMPTY: section_path, \
                before, continuation, proofs, proves, premises, equations, figures, citations, \
                references, notes. An unknown node_id returns {node_id, found:false, message} \
                instead. Nodes are {node_id, kind, text?, page?, bbox?, origin?, confidence?, \
                identifiers?, math?, alt_text?, assets?} with bbox [x, y, width, height] in PDF \
                points (bottom-left origin); the tex representation has no coordinates at all, so \
                page and bbox are absent on every node there. proofs/proves/equations/citations/\
                references entries are {relation_type, direction, from_node_id, confidence?, origin?, \
                metadata?, node}; figures entries are {relation_type, from_node_id, resolved_via?, \
                node, figure?, caption?}; premises entries are {via, node, symbol?, relation?}.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "node_id": { "type": "integer", "description": "Block id to center the bundle on (from search_document_nodes / get_document_blocks / get_node_relations / get_figures)." },
                    "max_before": { "type": "integer", "description": "Blocks of lead-in before the focus (default 2, max 8)." },
                    "max_continuation": { "type": "integer", "description": "Blocks after the focus, up to the next structural boundary (default 16, max 64)." },
                    "max_continuation_chars": { "type": "integer", "description": "Character budget over the continuation (default 6000, max 40000)." },
                    "max_related": { "type": "integer", "description": "Max entries per relation list (default 12, max 50)." },
                    "max_premises": { "type": "integer", "description": "Max premise definitions (default 12, max 20)." }
                },
                "required": ["node_id"]
            }),
            needs_approval: false,
        },
    ]
}

/// `call` を処理する。名前が一致しなければ `None`。
pub async fn try_execute(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Option<Result<String, ToolError>> {
    match call.tool_name.as_str() {
        "get_fulltext" => Some(exec_get_fulltext(ctx, &call.arguments).await),
        "get_document_structure" => Some(exec_get_document_structure(ctx, &call.arguments).await),
        "get_document_blocks" => Some(exec_get_document_blocks(ctx, &call.arguments).await),
        "search_document_nodes" => Some(exec_search_document_nodes(ctx, &call.arguments).await),
        "get_node_relations" => Some(exec_get_node_relations(ctx, &call.arguments).await),
        "get_symbol_definitions" => Some(exec_get_symbol_definitions(ctx, &call.arguments).await),
        "get_figures" => Some(exec_get_figures(ctx, &call.arguments).await),
        "get_tables" => Some(exec_get_tables(ctx, &call.arguments).await),
        "get_node_context" => Some(exec_get_node_context(ctx, &call.arguments).await),
        _ => None,
    }
}

// ─── 個別ツールの実行 ────────────────────────────────────────────────────────

async fn exec_get_fulltext(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
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

    // スコープ（CR-024）。**このツールは resolve_entry_id を通らない**（未知キーの返し方が
    // 違うので共有していない）ので、検査を独立に置く。
    ctx.ensure_entry_in_scope(entry_id)?;

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

async fn exec_get_document_structure(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let entry_id = resolve_entry_id(pool, args).await?;
    ctx.ensure_entry_in_scope(entry_id)?;
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

async fn exec_get_document_blocks(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let entry_id = resolve_entry_id(pool, args).await?;
    ctx.ensure_entry_in_scope(entry_id)?;
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
            // ブロック粒度の安定ハンドル。get_node_context / get_node_relations は
            // この id を取る（`index` はこの応答の中でしか意味を持たない）。
            "node_id": n.id,
            "kind": n.kind,
            "page": node_page(n),
            // 由来と確からしさ。原文由来（tex_source / pdf_text_layer）と推定
            // （layout_model / llm_inference）を読み手が区別できるようにする（Phase 10b）。
            "origin": n.origin,
            "confidence": n.confidence,
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

async fn exec_search_document_nodes(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidArguments("missing required argument: query".to_string()))?;
    let collection_id = args.get("collection_id").and_then(|v| v.as_i64());
    let tag_id = args.get("tag_id").and_then(|v| v.as_i64());
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_i64())
        .unwrap_or(DEFAULT_MAX_SEARCH_RESULTS)
        .clamp(1, MAX_SEARCH_RESULTS);

    // 1 件多く引いて「まだある」を検出する（COUNT の 2 回引きを避ける）。
    let mut hits = crate::db::document_nodes_fts::search_nodes(
        pool,
        query,
        collection_id,
        tag_id,
        None,
        Some(max_results + 1),
    )
    .await?;
    let truncated = hits.len() as i64 > max_results;
    hits.truncate(max_results as usize);

    // スコープ（CR-024）。SQL ではなく取得後に落とすので、絞られたことを応答に明示する
    // ——「ライブラリに無い」と「選択範囲に無い」は LLM にとって全く違う事実。
    let before = hits.len();
    if ctx.scope_mode == "entries" {
        hits.retain(|h| ctx.scope_entry_ids.contains(&h.entry.id));
    }
    let scope_filtered = hits.len() < before;

    let results: Vec<Value> = hits
        .iter()
        .map(|h| {
            json!({
                "entry_id": h.entry.id,
                "title": h.entry.title,
                "year": h.entry.year,
                // ヒットしたブロックの id。get_node_context / get_node_relations に渡せる。
                "node_id": h.node_id,
                "node_kind": h.node_kind,
                "page": h.page,
                "snippet": h.snippet,
                "bbox": h.bbox.as_ref().map(|b| json!([b.x, b.y, b.width, b.height])),
            })
        })
        .collect();

    let mut obj = serde_json::Map::new();
    obj.insert("count".to_string(), json!(results.len()));
    obj.insert("truncated".to_string(), json!(truncated));
    if scope_filtered {
        obj.insert("scope_filtered".to_string(), json!(true));
    }
    if results.is_empty() && !scope_filtered {
        // 0 件が「一致しない」なのか「索引が無い」なのかを区別できるようにする。
        // node-FTS は PDF 由来 LCIR だけを載せるので、未構築のライブラリでは常に 0 件になる。
        obj.insert(
            "index_built".to_string(),
            json!(node_index_exists(pool).await),
        );
    }
    obj.insert("results".to_string(), Value::Array(results));

    Ok(serde_json::to_string(&Value::Object(obj)).unwrap_or_default())
}

/// node-FTS に 1 行でもあるか。0 件応答の意味づけにだけ使う（失敗時は true 扱い＝黙る）。
async fn node_index_exists(pool: &SqlitePool) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM document_nodes_fts LIMIT 1)")
        .fetch_one(pool)
        .await
        .unwrap_or(1)
        == 1
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
        // Phase 8d-7: 図表参照の端点がどの図表かを示す（figure ノードは figure_number、
        // caption ノードは caption_number を payload に持つ）。
        for key in [
            "theorem_number",
            "section_number",
            "labels",
            "figure_number",
            "caption_number",
        ] {
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

async fn exec_get_node_relations(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let entry_id = resolve_entry_id(pool, args).await?;
    ctx.ensure_entry_in_scope(entry_id)?;
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

async fn exec_get_symbol_definitions(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let entry_id = resolve_entry_id(pool, args).await?;
    ctx.ensure_entry_in_scope(entry_id)?;
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
async fn exec_get_figures(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let entry_id = resolve_entry_id(pool, args).await?;
    ctx.ensure_entry_in_scope(entry_id)?;
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
        // Phase 8c: 代替テキストは**生成物**なので origin/model を必ず添えて返す（原文 caption と
        // 混同させない）。バッチ未実行なら欠落する。
        let alt_text = n.alt_text.as_ref().map(|a| {
            json!({
                "text": a.text,
                "origin": a.origin,
                "confidence": a.confidence,
                "model": a.model,
            })
        });
        figures.push(json!({
            "node_id": n.id,
            "page": node_page(n),
            "bbox": bbox,
            "figure_number": figure_number,
            "caption": caption,
            "alt_text": alt_text,
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
            file existence is not guaranteed and no image bytes are returned. alt_text, when \
            present, is never the authors' wording: check its origin — llm_inference is a vision \
            model's guess (the caption is the source of truth), user_edited is the library \
            owner's own description.",
    }))
    .unwrap_or_default())
}

/// Phase 8b: 構造化テーブル（TeX 版のみ — tabular は TeX ソースからしかセル構造化できない）。
/// caption は caption_of 辺（from=caption / to=table）から解決する。rows は payload の
/// セル構造をそのまま返すが、原文スニペット `latex_source` は返さない（rows が構造を持ち、
/// 二重送出でレスポンスが肥大するため）。`max_chars` はセル文字量の概算予算（最低 1 表は返す）。
async fn exec_get_tables(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let entry_id = resolve_entry_id(pool, args).await?;
    ctx.ensure_entry_in_scope(entry_id)?;
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

/// Phase 10a: 文脈バンドル。**ノード id 起点**なので `entry_id`/`source` は取らない —
/// ノードが載っている版がそのまま答えになる（エントリ起点の tex > pdf 優先で版を選び直すと、
/// 呼び出し側が握っている id が引けない版に化ける）。組み立ては `context` の純関数が全部やる。
async fn exec_get_node_context(ctx: &ToolContext<'_>, args: &Value) -> Result<String, ToolError> {
    let pool = ctx.pool;
    let node_id = args
        .get("node_id")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| {
            ToolError::InvalidArguments("missing required argument: node_id (integer)".to_string())
        })?;

    let loaded = crate::ingestion::load_node_lcir(pool, node_id)
        .await
        .map_err(ToolError::Execution)?;
    let Some((version, entry_id, doc)) = loaded else {
        return Ok(serde_json::to_string(&json!({
            "node_id": node_id,
            "found": false,
            "message": "no LCIR node with this id. Node ids are per-representation and change \
                when LCIR is rebuilt — re-run search_document_nodes / get_document_blocks to get \
                current ids.",
        }))
        .unwrap_or_default());
    };
    // スコープ（CR-024）。**引数は node_id だけなので、entry が判るのはここが最初**。
    // バンドルを組む前に落とす（組んでから返す形にすると、後の編集 1 つで本文が漏れる）。
    ctx.ensure_entry_in_scope(entry_id)?;

    // 上限つきで読む。**上限が要る理由はチャット側にある** — 応答は会話履歴に永続化されて
    // 以後のターンで毎回再送されるので、`max_continuation: 1_000_000` の 1 回でセッションが
    // 壊れる。既定値は `ContextOptions::default()`（実測に基づく）で、ここは「際限なく
    // 大きくできない」ことだけを担保する。
    let usize_arg = |key: &str, default: usize, cap: usize| -> usize {
        args.get(key)
            .and_then(|v| v.as_i64())
            .filter(|n| *n >= 0)
            .map_or(default, |n| (n as usize).min(cap))
    };
    let d = crate::context::ContextOptions::default();
    let opts = crate::context::ContextOptions {
        max_before: usize_arg("max_before", d.max_before, 8),
        max_continuation: usize_arg("max_continuation", d.max_continuation, 64),
        max_continuation_chars: usize_arg("max_continuation_chars", d.max_continuation_chars, 40_000)
            .max(1),
        max_related: usize_arg("max_related", d.max_related, 50),
        max_premises: usize_arg("max_premises", d.max_premises, 20),
    };

    let Some(bundle) = crate::context::build_node_context(&doc, node_id, &opts) else {
        // version_id_for_node が引けた以上ここには来ないが、木の読み込みと不整合な場合の保険。
        return Err(ToolError::Execution(format!(
            "node {node_id} is not present in its own document version"
        )));
    };

    // 書誌側の見出し（エントリ・併存表現）を封筒として足す。既存 LCIR ツールと同じキー名。
    let versions = crate::ingestion::entry_lcir_versions(pool, entry_id)
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;
    let citation_key = crate::bibtex::resolve_citation_key(pool, entry_id).await.ok();

    let mut obj = match serde_json::to_value(&bundle) {
        Ok(Value::Object(m)) => m,
        _ => return Err(ToolError::Execution("failed to serialize bundle".to_string())),
    };
    obj.insert("found".to_string(), json!(true));
    obj.insert("entry_id".to_string(), json!(entry_id));
    obj.insert("citation_key".to_string(), json!(citation_key));
    obj.insert("attachment_id".to_string(), json!(version.attachment_id));
    obj.insert(
        "source".to_string(),
        json!(short_source_name(&version.extractor_name)),
    );
    obj.insert(
        "extractor_version".to_string(),
        json!(version.extractor_version),
    );
    obj.insert("available_sources".to_string(), sources_json(&versions));
    Ok(serde_json::to_string(&Value::Object(obj)).unwrap_or_default())
}

// ─── 根拠参照（Phase 10b・UI 用） ────────────────────────────────────────────

/// ツール結果が指し示す「PDF 上の根拠」1 件。UI のチップになる。
///
/// **`page` を持つものだけを作る** — TeX 版のノードには座標が無いので、チップを出しても
/// 飛び先が無い。飛べない UI を出すより出さない方がよい（TeX 版が既定になる arXiv 論文では
/// チップは出ない。これは既知の制限で、tex → pdf の位置解決は post-1.0）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ToolResultRef {
    pub node_id: i64,
    pub kind: String,
    pub page: i64,
}

/// 1 つのツール結果から取り出す根拠の上限。カード 1 枚がチップで埋まらない程度。
const MAX_REFS_PER_RESULT: usize = 5;

/// ツール結果 JSON から根拠参照を取り出す**決定的な純関数**。
///
/// なぜ Rust 側でやるか: ライブ配信の `result_summary` は 500 文字で切られるので
/// （`clip_text`）、フロントでは JSON として読めない。
///
/// なぜ再パースか: `try_execute` の契約が `String` を返すことなので、結果は一度 JSON 文字列に
/// なる。型付きの値を返す設計に変えると MCP と共有している契約が壊れる。
/// **`tool_name` で先に振り分ける**ので、JSON でない結果（`export_bibtex` の生 .bib、
/// `"Tool \`x\` failed: …"`、`"The user denied…"`）は `from_str` に到達しない。
///
/// 各ツールで鍵名が違う（`node_kind` / `kind` / 種別なし）ので、汎用の走査ではなく
/// 明示的な分岐で書く。
pub fn provenance_refs(tool_name: &str, result_json: &str) -> Vec<ToolResultRef> {
    // 対象外のツールは JSON を読まない。
    if !matches!(
        tool_name,
        "search_document_nodes" | "get_document_blocks" | "get_figures" | "get_node_context"
    ) {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<Value>(result_json) else {
        return Vec::new();
    };

    // `page` は「鍵が無い」（`get_node_context` の ContextNode）と「null」
    // （`get_document_blocks` の tex 版）の両方がありうるので as_i64 で判定する。
    fn one(node: &Value, kind_key: Option<&str>, fallback_kind: &str) -> Option<ToolResultRef> {
        let node_id = node.get("node_id").and_then(Value::as_i64)?;
        let page = node.get("page").and_then(Value::as_i64)?;
        let kind = kind_key
            .and_then(|k| node.get(k))
            .and_then(Value::as_str)
            .unwrap_or(fallback_kind)
            .to_string();
        Some(ToolResultRef {
            node_id,
            kind,
            page,
        })
    }

    let mut out: Vec<ToolResultRef> = Vec::new();
    match tool_name {
        "search_document_nodes" => {
            if let Some(items) = v.get("results").and_then(Value::as_array) {
                for it in items {
                    if let Some(r) = one(it, Some("node_kind"), "block") {
                        out.push(r);
                    }
                }
            }
        }
        "get_document_blocks" => {
            if let Some(items) = v.get("blocks").and_then(Value::as_array) {
                for it in items {
                    if let Some(r) = one(it, Some("kind"), "block") {
                        out.push(r);
                    }
                }
            }
        }
        "get_figures" => {
            // 図の応答には種別の鍵が無い（全部 figure なので）。
            if let Some(items) = v.get("figures").and_then(Value::as_array) {
                for it in items {
                    if let Some(r) = one(it, None, "figure") {
                        out.push(r);
                    }
                }
            }
        }
        "get_node_context" => {
            // 焦点だけ。continuation まで出すとチップが並びすぎて「どれが答えか」が消える。
            if let Some(r) = v.get("focus").and_then(|f| one(f, Some("kind"), "block")) {
                out.push(r);
            }
        }
        _ => {}
    }

    // 同じノードを 2 回出さない（`get_document_blocks` の重複は無いが、契約として持つ）。
    out.dedup_by_key(|r| r.node_id);
    out.truncate(MAX_REFS_PER_RESULT);
    out
}

// ─── テスト ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::document_nodes::{insert_node, NewDocumentNode};
    use crate::db::document_versions::{insert_version, NewDocumentVersion};
    use crate::db::entries::create_entry;
    use crate::db::fulltext::index_attachment;
    use crate::db::source_fragments::{insert_fragment, NewSourceFragment};
    use crate::db::document_nodes_fts::{index_nodes, NodeFtsInput};
    use crate::document_ir::{schema, ExtractionStatus, NodeKind};
    use crate::models::EntryInput;
    use sqlx::SqlitePool;

    // fixture は mcp_server のテストとは共有しない（あちらは移設の回帰ハーネスなので、
    // 被験モジュールと可変なテストコードを共有させない）。ここは routing とスコープを
    // 見るだけなので最小で足りる。

    fn call(tool: &str, args: Value) -> ToolCallSpec {
        ToolCallSpec {
            call_id: "c1".to_string(),
            tool_name: tool.to_string(),
            arguments: args,
        }
    }

    fn ctx_all(pool: &SqlitePool) -> ToolContext<'_> {
        ToolContext {
            pool,
            session_id: 1,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        }
    }

    fn ctx_scoped<'a>(pool: &'a SqlitePool, ids: &'a [i64]) -> ToolContext<'a> {
        ToolContext {
            pool,
            session_id: 1,
            scope_mode: "entries",
            scope_entry_ids: ids,
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        }
    }

    async fn make_entry(pool: &SqlitePool, title: &str) -> i64 {
        create_entry(
            pool,
            &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    }

    /// 索引済み PDF を 1 つ持つエントリ。
    async fn entry_with_fulltext(pool: &SqlitePool, title: &str, text: &str) -> i64 {
        let eid = make_entry(pool, title).await;
        let att = add_attachment(pool, eid, &format!("a/{eid}/p.pdf"), "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(pool, att.id, &[(1, text.to_string())])
            .await
            .unwrap();
        eid
    }

    /// PDF 由来 LCIR（document > page > paragraph）を 1 本持つエントリ。
    /// 戻り値は (entry_id, paragraph の node_id)。
    async fn entry_with_lcir(pool: &SqlitePool, title: &str, text: &str) -> (i64, i64) {
        let eid = make_entry(pool, title).await;
        let att = add_attachment(pool, eid, &format!("a/{eid}/p.pdf"), "p.pdf", "application/pdf")
            .await
            .unwrap()
            .id;
        let vid = insert_version(
            pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: &format!("ck-{eid}"),
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
        let root = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: None,
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
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
        let para = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(page),
                node_kind: NodeKind::Paragraph.as_str(),
                ordinal: 0,
                plain_text: Some(text),
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        insert_fragment(
            pool,
            &NewSourceFragment {
                node_id: para,
                page_number: 1,
                x: 72.0,
                y: 500.0,
                width: 300.0,
                height: 12.0,
                rotation: 0.0,
                reading_order: Some(0),
                fragment_type: Some("block"),
            },
        )
        .await
        .unwrap();
        index_nodes(
            pool,
            att,
            &[NodeFtsInput {
                node_id: para,
                page: 1,
                node_kind: NodeKind::Paragraph.as_str().to_string(),
                content: text.to_string(),
            }],
        )
        .await
        .unwrap();
        (eid, para)
    }

    // ── routing ─────────────────────────────────────────────────────────────

    /// **チャットの入口から**引けること。`document::try_execute` を直接叩くテストだと、
    /// `llm::tools::execute_tool` への配線漏れ（＝チャットからは全部 unknown tool）を
    /// 見逃す。
    #[sqlx::test(migrations = "./migrations")]
    async fn every_document_tool_is_reachable_from_execute_tool(pool: SqlitePool) {
        let ctx = ctx_all(&pool);
        for name in DOCUMENT_TOOLS {
            let args = match *name {
                "get_node_context" => json!({ "node_id": 1 }),
                "search_document_nodes" => json!({ "query": "x" }),
                _ => json!({ "entry_id": 1 }),
            };
            let r = crate::llm::tools::execute_tool(&ctx, &call(name, args)).await;
            assert!(
                !matches!(r, Err(ToolError::UnknownTool(_))),
                "{name} is advertised but not routed from execute_tool"
            );
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unknown_tool_returns_none(pool: SqlitePool) {
        let ctx = ctx_all(&pool);
        assert!(try_execute(&ctx, &call("nope", json!({}))).await.is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn specs_cover_exactly_document_tools(_pool: SqlitePool) {
        let names: Vec<String> = specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, DOCUMENT_TOOLS.to_vec());
        // LCIR ツールは get_fulltext を含まない（あれは LCIR に依存しない）。
        assert!(!LCIR_TOOLS.contains(&"get_fulltext"));
        for n in LCIR_TOOLS {
            assert!(DOCUMENT_TOOLS.contains(n), "{n}");
        }
    }

    // ── スコープ（CR-024） ───────────────────────────────────────────────────

    /// `get_fulltext` は `resolve_entry_id` を通らない独自の解決を持つ。
    /// スコープ検査をそちらに足し忘れると、論文 1 本が丸ごと漏れる。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_fulltext_refuses_out_of_scope_entry(pool: SqlitePool) {
        let inside = entry_with_fulltext(&pool, "In scope", "alpha beta").await;
        let outside = entry_with_fulltext(&pool, "Out of scope", "secret gamma").await;

        let ids = vec![inside];
        let ctx = ctx_scoped(&pool, &ids);

        let ok = try_execute(&ctx, &call("get_fulltext", json!({ "entry_id": inside })))
            .await
            .unwrap()
            .unwrap();
        assert!(ok.contains("alpha beta"), "in-scope entry must be readable: {ok}");

        let denied = try_execute(&ctx, &call("get_fulltext", json!({ "entry_id": outside })))
            .await
            .unwrap();
        match denied {
            Err(ToolError::Execution(m)) => assert!(m.contains("outside the current chat scope"), "{m}"),
            other => panic!("expected a scope error, got {other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn entry_arg_tools_refuse_out_of_scope_entry(pool: SqlitePool) {
        let inside = make_entry(&pool, "In scope").await;
        let (outside, _) = entry_with_lcir(&pool, "Out of scope", "secret theorem").await;
        let ids = vec![inside];
        let ctx = ctx_scoped(&pool, &ids);

        for name in [
            "get_document_structure",
            "get_document_blocks",
            "get_node_relations",
            "get_symbol_definitions",
            "get_figures",
            "get_tables",
        ] {
            let r = try_execute(&ctx, &call(name, json!({ "entry_id": outside })))
                .await
                .unwrap();
            match r {
                Err(ToolError::Execution(m)) => {
                    assert!(m.contains("outside the current chat scope"), "{name}: {m}")
                }
                other => panic!("{name} must refuse out-of-scope entry, got {other:?}"),
            }
        }
    }

    /// `get_node_context` は entry を引数に取らない。node → entry を解決した直後に
    /// 検査しないと本文が漏れる。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_node_context_refuses_out_of_scope_node(pool: SqlitePool) {
        let inside = make_entry(&pool, "In scope").await;
        let (_outside, node_id) = entry_with_lcir(&pool, "Out of scope", "secret theorem body").await;
        let ids = vec![inside];
        let ctx = ctx_scoped(&pool, &ids);

        let r = try_execute(&ctx, &call("get_node_context", json!({ "node_id": node_id })))
            .await
            .unwrap();
        match r {
            Err(ToolError::Execution(m)) => {
                assert!(m.contains("outside the current chat scope"), "{m}");
                assert!(!m.contains("secret theorem body"), "must not leak the text: {m}");
            }
            other => panic!("expected a scope error, got {other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_node_context_reads_in_scope_node(pool: SqlitePool) {
        let (eid, node_id) = entry_with_lcir(&pool, "Mine", "the statement").await;
        let ids = vec![eid];
        let ctx = ctx_scoped(&pool, &ids);
        let s = try_execute(&ctx, &call("get_node_context", json!({ "node_id": node_id })))
            .await
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["found"], true);
        assert_eq!(v["entry_id"], eid);
    }

    /// 横断検索はスコープで落とすが、**落としたことを応答に出す** —
    /// 黙って絞ると「ライブラリに無い」と読まれる。
    #[sqlx::test(migrations = "./migrations")]
    async fn search_document_nodes_reports_scope_filtering(pool: SqlitePool) {
        let (inside, _) = entry_with_lcir(&pool, "Mine", "transformer architecture").await;
        let _ = entry_with_lcir(&pool, "Theirs", "transformer architecture").await;

        let all = try_execute(&ctx_all(&pool), &call("search_document_nodes", json!({ "query": "transformer" })))
            .await
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_str(&all).unwrap();
        assert_eq!(v["count"], 2);
        assert!(v.get("scope_filtered").is_none(), "unscoped must not set the flag");

        let ids = vec![inside];
        let scoped = try_execute(
            &ctx_scoped(&pool, &ids),
            &call("search_document_nodes", json!({ "query": "transformer" })),
        )
        .await
        .unwrap()
        .unwrap();
        let v2: Value = serde_json::from_str(&scoped).unwrap();
        assert_eq!(v2["count"], 1);
        assert_eq!(v2["scope_filtered"], true);
    }

    // ── 応答の大きさ ────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn search_document_nodes_caps_results_and_reports_truncation(pool: SqlitePool) {
        for i in 0..4 {
            entry_with_lcir(&pool, &format!("Paper {i}"), "convergence theorem").await;
        }
        let ctx = ctx_all(&pool);

        let s = try_execute(
            &ctx,
            &call("search_document_nodes", json!({ "query": "convergence", "max_results": 2 })),
        )
        .await
        .unwrap()
        .unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["count"], 2);
        assert_eq!(v["truncated"], true);

        let s2 = try_execute(&ctx, &call("search_document_nodes", json!({ "query": "convergence" })))
            .await
            .unwrap()
            .unwrap();
        let v2: Value = serde_json::from_str(&s2).unwrap();
        assert_eq!(v2["count"], 4);
        assert_eq!(v2["truncated"], false);
    }

    /// 0 件のとき「一致しない」と「まだ索引が無い」を区別できること。
    #[sqlx::test(migrations = "./migrations")]
    async fn search_document_nodes_distinguishes_empty_index_from_no_match(pool: SqlitePool) {
        let ctx = ctx_all(&pool);
        let s = try_execute(&ctx, &call("search_document_nodes", json!({ "query": "anything" })))
            .await
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["count"], 0);
        assert_eq!(v["index_built"], false);

        entry_with_lcir(&pool, "Paper", "convergence theorem").await;
        let s2 = try_execute(&ctx, &call("search_document_nodes", json!({ "query": "unrelated" })))
            .await
            .unwrap()
            .unwrap();
        let v2: Value = serde_json::from_str(&s2).unwrap();
        assert_eq!(v2["count"], 0);
        assert_eq!(v2["index_built"], true);
    }

    /// バンドルの大きさは呼び出し側が際限なく広げられない（履歴に残って毎ターン再送される）。
    #[sqlx::test(migrations = "./migrations")]
    async fn get_node_context_clamps_size_arguments(pool: SqlitePool) {
        let (_eid, node_id) = entry_with_lcir(&pool, "Paper", "the statement").await;
        let ctx = ctx_all(&pool);
        // 上限を超える値を渡しても落ちず、応答が返る（clamp されるので巨大にならない）。
        let s = try_execute(
            &ctx,
            &call(
                "get_node_context",
                json!({
                    "node_id": node_id,
                    "max_continuation": 10_000_000i64,
                    "max_continuation_chars": 100_000_000i64,
                    "max_related": 10_000i64,
                }),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["found"], true);
    }

    // ── provenance ──────────────────────────────────────────────────────────

    // ── 根拠参照（provenance_refs） ─────────────────────────────────────────

    /// **実際のツール出力**を入力にする（手組み JSON だと鍵名の差異を取り違えたまま通る）。
    #[sqlx::test(migrations = "./migrations")]
    async fn provenance_refs_reads_each_real_tool_shape(pool: SqlitePool) {
        let (eid, node_id) = entry_with_lcir(&pool, "Paper", "convergence theorem").await;
        let ctx = ctx_all(&pool);

        let s = try_execute(&ctx, &call("search_document_nodes", json!({ "query": "convergence" })))
            .await
            .unwrap()
            .unwrap();
        let refs = provenance_refs("search_document_nodes", &s);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].node_id, node_id);
        assert_eq!(refs[0].kind, "paragraph");
        assert_eq!(refs[0].page, 1);

        let s = try_execute(&ctx, &call("get_document_blocks", json!({ "entry_id": eid })))
            .await
            .unwrap()
            .unwrap();
        let refs = provenance_refs("get_document_blocks", &s);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].node_id, node_id);
        assert_eq!(refs[0].kind, "paragraph");

        let s = try_execute(&ctx, &call("get_node_context", json!({ "node_id": node_id })))
            .await
            .unwrap()
            .unwrap();
        let refs = provenance_refs("get_node_context", &s);
        assert_eq!(refs.len(), 1, "focus only");
        assert_eq!(refs[0].node_id, node_id);

        // 図は無いので空。has_lcir:true でも figures が空なら参照も空。
        let s = try_execute(&ctx, &call("get_figures", json!({ "entry_id": eid })))
            .await
            .unwrap()
            .unwrap();
        assert!(provenance_refs("get_figures", &s).is_empty());
    }

    /// 座標を持たない版（TeX）のノードからはチップを作らない。飛び先が無いので。
    #[test]
    fn provenance_refs_skips_nodes_without_a_page() {
        // tex 版の get_document_blocks は page を null で返す。
        let tex = r#"{"has_lcir":true,"source":"tex","blocks":[
            {"index":0,"node_id":7,"kind":"theorem","page":null,"text":"..."}]}"#;
        assert!(provenance_refs("get_document_blocks", tex).is_empty());

        // ContextNode は page の鍵ごと落とす。
        let ctx_tex = r#"{"found":true,"source":"tex","focus":{"node_id":7,"kind":"theorem","text":"..."}}"#;
        assert!(provenance_refs("get_node_context", ctx_tex).is_empty());
    }

    /// JSON でない結果や対象外のツールで落ちない（生 .bib・失敗メッセージ・拒否メッセージ）。
    #[test]
    fn provenance_refs_ignores_non_json_and_unrelated_tools() {
        assert!(provenance_refs("export_bibtex", "@article{smith2020,\n  title={x}\n}").is_empty());
        assert!(provenance_refs("get_document_blocks", "Tool `x` failed: db error").is_empty());
        assert!(provenance_refs("get_node_context", "").is_empty());
        assert!(provenance_refs("get_fulltext", r#"{"indexed":true,"text":"..."}"#).is_empty());
        // 同名のツールを MCP クライアント経由で呼ぶと mcp_ 接頭辞が付くので対象外になる。
        assert!(provenance_refs("mcp_lumencite_get_figures", r#"{"figures":[]}"#).is_empty());
    }

    #[test]
    fn provenance_refs_caps_the_number_of_chips() {
        let blocks: Vec<String> = (1..=12)
            .map(|i| format!(r#"{{"node_id":{i},"kind":"paragraph","page":1}}"#))
            .collect();
        let s = format!(r#"{{"blocks":[{}]}}"#, blocks.join(","));
        assert_eq!(provenance_refs("get_document_blocks", &s).len(), MAX_REFS_PER_RESULT);
    }

    /// 完了条件「AI 推定部分を回答中で識別できる」は、まず**データ**が origin を運ぶこと。
    #[sqlx::test(migrations = "./migrations")]
    async fn document_blocks_carry_origin_and_confidence(pool: SqlitePool) {
        let (eid, _) = entry_with_lcir(&pool, "Paper", "a paragraph").await;
        let ctx = ctx_all(&pool);
        let s = try_execute(&ctx, &call("get_document_blocks", json!({ "entry_id": eid })))
            .await
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        let b = &v["blocks"][0];
        assert_eq!(b["origin"], "layout_model");
        assert_eq!(b["confidence"], 0.6);
    }
}
