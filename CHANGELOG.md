# Changelog

All notable changes to LumenCite will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **LCIR — reading context around one block (Phase 10a, experimental)** — a new read tool, `get_node_context`, assembles everything needed to read and cite a single block in one call: the statement, the blocks that continue it, the proof that proves it, the definitions it rests on, the equations/figures/works it references, and the PDF region of every piece. This matters most for PDF-derived documents, where a theorem node holds only its first layout block and the rest of the statement continues into sibling blocks — often onto the next page. Every element carries its `origin` and `confidence`, so source text and inference stay distinguishable when quoted, and a `notes` list states what the bundle could not reach. Available over MCP as `get_node_context` and from the CLI as `lumencite node-context <node_id>`. No migration; nothing new is stored.
- **LCIR — the in-app assistant can now read papers by structure (Phase 10b, experimental)** — the chat assistant gains the nine document-reading tools that were previously reachable only over MCP: `get_fulltext` plus the eight LCIR tools (`get_document_structure`, `get_document_blocks`, `search_document_nodes`, `get_node_relations`, `get_symbol_definitions`, `get_figures`, `get_tables`, `get_node_context`). The LCIR eight appear only once something has actually been built, so a library without LCIR sees exactly one new tool. Answers now distinguish the paper's own words (`tex_source`, `pdf_text_layer`) from LumenCite's estimates (`layout_model`, `llm_inference`, including generated figure alt text), and tool results respect the chat's entry scope — when hits are filtered out, the assistant is told so rather than reporting an empty library. No migration; nothing new is stored.
- **Jump from a chat answer to the evidence in the PDF** — tool result cards show the blocks the answer rests on; clicking one opens the PDF viewer at that page with the region highlighted. Available for PDF-derived LCIR (the TeX representation carries exact LaTeX but no coordinates).
- **`get_document_blocks` and `search_document_nodes` now return each block's `node_id`** — the stable handle you pass to `get_node_context` and `get_node_relations`. (`get_document_blocks`' existing `index` only numbers blocks within one response and was never a block id.)
- **Storage view and reclamation of superseded LCIR versions** — Settings → Data now shows how much of the database file is in use and how much is reusable, and can reclaim the old LCIR versions left behind by rebuilds. In a real library those old versions held **83% of all document nodes**. Reclaiming is offered even when LCIR is disabled, since that is exactly when the old versions are pure overhead. Versions that hold generated or hand-written figure descriptions are protected, and versions that carried-over descriptions point back to keep their record while their contents are freed. The database file itself does not shrink — the space becomes reusable and is reclaimed by the next backup and the next rebuild. No migration.

- **LCIR is now built automatically (experimental; requires LCIR enabled)** — attaching a PDF, downloading one from arXiv, or clipping one from the browser now builds its LCIR in the background, and an existing library fills in gradually on launch. Until now LCIR only ever appeared if you pressed a button, which is why the in-app assistant's document tools stayed hidden for most libraries. The backfill is deliberately unhurried: it works within a time budget per run, checked between papers, and stands aside the moment you start a batch yourself, generate figure descriptions, fetch TeX sources, or a backup begins. It does not run when a second copy of LumenCite is open on the same library, and it never re-builds papers just because the extractor version changed — raising a version still requires the explicit "rebuild outdated" button. If the PDF library (pdfium) cannot be loaded, PDFs are skipped and **counted**, so "nothing to do" and "nothing ran" no longer look alike; TeX sources keep building since they don't need it. No migration.

### Changed

- **LCIR is now on by default, and fetching arXiv sources became a separate choice** — the structural analysis of PDFs is no longer an experimental switch you have to find: new libraries get it on. It only counts as "off" if you actually turned it off, so anyone who switched it off stays off across updates. Downloading e-prints (the LaTeX sources authors submit to arXiv) is now its own consent setting rather than something LCIR implies — building LCIR happens on your machine, but fetching an e-print is a few megabytes pulled from arXiv on every paper you add or clip, and that should not start silently just because the analysis is on. If you had LCIR switched on before this version you were already fetching them, so that choice is carried over; everyone else starts with fetching off and can turn it on in Settings → Data. Turning LCIR itself off also stops fetching (there would be nothing to build from what it downloads), and turning the fetch off partway through the bulk "fetch missing TeX sources" run stops it at the next paper. No migration.

- **Full-text search now reads its text from LCIR when a paper has been built** (experimental; requires LCIR enabled). The page text pdfium extracted for LCIR replaces the older extractor's output as the source of the search index — for newly attached papers, for every paper you build LCIR for, and, for an existing library, on demand from Settings → Data. In a real 138-paper library, converting the whole library added **112 pages**, of which **57 came from two papers that had no searchable text at all**, and removed the control characters that were splitting words inside the index — **729 index rows contained them, now none do**. Measured on twelve frequent terms, every term matched more pages than before (512 pages started matching that had not, against 216 that stopped — the new text is not a superset of the old one). Pages whose LCIR has no text keep their existing rows, so nothing is lost where pdfium reads nothing; papers you OCR from this version on are left untouched entirely. No migration; the conversion takes seconds and does not re-parse any PDF.

  ⚠ **Text you OCR'd before this version is not recorded as OCR-derived**, so the manual conversion can replace it with the LCIR text. The same is true of text from an OCR run you stopped part-way, and of the brief window while a completed run is being saved. The automatic pass on launch therefore leaves an index of unknown origin alone — it only fills attachments that have no index at all. (An index it *did* record as its own is still replaced, so OCR pages written into an LCIR-recorded attachment by a run you stopped part-way are not covered.) Three paths still replace an unrecorded index: the Settings → Data button (that is what it is for), the per-attachment re-index button, and building LCIR for a paper — which is also what automatic building does on launch and when you attach a PDF.

### Fixed

- **Re-indexing a scanned PDF no longer deletes the text you paid to OCR** — pressing the re-index button on an attachment whose PDF has no text layer used to wipe its index and put nothing back: the old extractor returns "no text" for such files, and the app took that as the new contents. There was no confirmation step, and the only way back was to run OCR again and pay for it again (rebuilding LCIR does not help here: a PDF with no text layer yields no LCIR page text either). Now an extraction that yields no text at all leaves the existing index alone and says so. Shrinkage is still allowed — an extraction that reads one page where there used to be five replaces all five. That boundary is deliberate but narrow: it keeps re-indexing meaningful for a PDF whose text layer is partly readable, and it leaves one case open — text from an OCR run you stopped part-way, on a PDF whose text layer partly works, can still be replaced by pressing re-index. Because the re-index button was also the only non-destructive way to *discard* an index, indexed attachments now have a **"Remove index"** button beside it (the PDF itself is untouched). A related mislabel is fixed with it: skipping because the index came from LCIR used to report "indexed *n* pages" when nothing had been written.

- **The launch pass that converts full-text search to LCIR no longer replaces text it cannot account for** — the check that protected an existing index of unknown origin ran *before* the write, in a separate step, and read "how many pages are indexed" with a fallback of zero. So a database read that failed for any reason (a busy pool, an I/O error) was indistinguishable from "this attachment has no index", and the pass went ahead and replaced it — silently, with nothing in the log. It also left a window: an OCR run finishing between the check and the write slipped through unprotected. The decision now happens inside the same transaction as the write, so a read that fails means nothing is written at all, and the run does not mark itself done — it retries on the next launch.

- **OCR can now be stopped, and no longer throws away pages you already paid for** — OCR bills one API call per page, and you cannot know the page count before starting: a scanned book in a real library is 527 pages. Until now, pressing "OCR" committed you to all of them — no progress, no page count, and no way to stop short of quitting the app; OCR started from chat kept billing even after you stopped the chat; and one API failure on page 500 threw away 499 pages of paid transcription. Now a reader-started run shows `n/total pages` in the reader with a Stop button beside it, **and Settings → Data shows any running OCR — started from the reader or from chat — with its own Stop button**, so leaving the reader no longer strands a run; only one OCR can run at a time. Stopping or failing mid-run keeps every page transcribed so far, and a mid-run failure names the actual PDF page it failed on. Pages the model reads as blank are billed but never overwrite existing text, and the result reports how many pages were processed and how many actually contained text — a run that found no text on any page no longer looks like a success. **There is no resume: running OCR again starts over from the first page and bills every page again** — the app says so instead of pretending otherwise (chat, additionally, refuses a full re-run of an entry that already has indexed pages and says how to proceed). Only a run that completes replaces the attachment's index and marks it OCR-derived; an interrupted or failed run merges what it got and leaves the rest — and the mark that protects OCR text from being overwritten later is set only on completion, so stopping at page 3 of a 527-page book can no longer freeze 524 pages of broken extracted text in place.

- **Turning LCIR off now really stops everything it pays for or downloads** — three places asked the wrong question. Adding an arXiv paper fetched its e-print whenever the fetch setting was on, *even with LCIR switched off* — a few megabytes pulled from arXiv on every paper, for an analysis that then did nothing with them, and the consent checkbox greys out when LCIR is off so you could not withdraw it either. A running "generate figure descriptions" job re-checked only the description consent, so switching LCIR off did not stop the billing — and, again, switching LCIR off greyed out the one checkbox that would have stopped it. And "fetch missing TeX sources" stayed clickable with LCIR off while doing nothing at all when pressed, showing no message whatsoever. All three now go through a single decision — *may we actually do this?* — that requires both the specific consent and LCIR itself, re-checked on every paper and every figure of a running job. A job that stops because a consent closed now says *which* one closed, rather than blaming the checkbox you did not touch. Explicitly pressing "fetch TeX source" on one paper still works with everything off: asking for it *is* the consent.

- **Closing Settings during a long job no longer loses track of it — and figure descriptions can no longer be billed against a stale count** — the running state, progress, and last result of the long jobs in Settings → Data (LCIR build/rebuild, full-text re-derivation, storage reclamation, figure descriptions, TeX fetching) now live in the app itself rather than in the settings window. Reopening the window shows a job still running, with its progress, and shows the result — including the failure count — of a job that finished while the window was closed. Jobs started elsewhere, such as generating a figure description from the detail panel, are now reflected too. (Two jobs that are not tracked this way: filling in missing full-text indexes, and backup/restore.) Most importantly, pressing "Generate descriptions" re-checks how many figures are pending at that moment: if the number has changed since it was displayed, the app asks again with the new number instead of billing for it. The same re-check now guards the per-paper button in the detail panel. Previously the count came from when the window was opened, and a rebuild running in the background could leave it showing roughly a quarter of what would actually be charged.

- **A newly attached PDF no longer has its LCIR-derived index overwritten by a redundant re-index** — the app used to re-index every attachment a second time from the front end, as if you had asked for it explicitly. With automatic LCIR building that second pass could undo the first, and which one won depended on timing. The redundant pass is gone; the "indexing" indicator now follows the work the backend is actually doing.

- **Saving in the app while a paper is being processed in the background no longer fails with "database is locked"** — the app's database connection now waits up to 30 seconds for a writer instead of 5 (the `lumencite` CLI's direct-write path is unchanged). Building a large paper writes its whole structure in one transaction, which can exceed five seconds; that only used to happen while you watched a batch run, and now happens on its own.

- **A newly attached PDF can no longer lose its LCIR-derived index to the slower extractor finishing later** — the two writers are now one decision made in a single transaction. Previously, a paper whose text the older extractor could not read at all would disappear from search even though LumenCite had read it correctly.

- **Regenerating a missing figure crop no longer loses its description** — when the app re-renders a crop that has gone missing and the new image differs byte-for-byte, the stored description now follows the new image. Previously the link broke silently, and the next rebuild would discard the paid-for description and bill for it again.

- **The assistant no longer re-OCRs a PDF whose text is already indexed** — it now reads the existing text with `get_fulltext` instead. A full re-OCR replaces the attachment's index, so this used to overwrite good extracted text with vision output and bill for it. Explicitly running OCR yourself from the app is unchanged.
- **Read-only chat tools no longer reload the whole library list after every call**, and they are no longer displayed as write operations awaiting approval.
- **Opening a PDF page from the app no longer scrolls every other open PDF window** — the jump is now addressed to the intended window instead of broadcast to all of them.

## [0.10.0] - 2026-07-28

This release completes **Phase 8** of **LCIR** — the experimental, machine-readable intermediate representation for papers — and adds its first **export** path. Papers can now be written out as LCIR JSON or as structured Markdown; figures are detected and cropped out of PDFs, TeX tables are structured cell by cell, and figures can optionally be described by an LLM Vision model as alt text. Everything stays gated behind the off-by-default `lcir.enabled` flag, so default behaviour is **unchanged**. Two additive migrations (`0019`, `0020`) create new tables that remain empty unless the flag is on — no data migration, and existing libraries upgrade unchanged. The Web Clipper extension is unchanged from v0.8.0 (v0.2.0).

### Added

- **LCIR — export (Phase 9a, experimental)** — write an entry out as **LCIR JSON** (the validated structure, with provenance, coordinates and confidence) or as **structured Markdown** (sections, raw LaTeX math, theorems and proofs, GFM tables, and a reference list carrying cite keys). Available from two buttons in the detail panel, and from the CLI as `lumencite export-lcir <id|key> [--format json|md] [--source tex|pdf]`.
- **LCIR — figures from PDFs (Phase 8a, experimental)** — detects figure regions on each page, saves a cropped PNG per figure, records them as `figure` nodes, and links each one to its caption (`caption_of`). Adds migration `0019` (`assets`, `node_assets`).
- **LCIR — structured tables from TeX (Phase 8b, experimental)** — parses `tabular` / `tabular*` / `tabularx` into a row-by-cell grid (`table` nodes) with column spec, alignments, `\multicolumn` spans and rules, keeping LaTeX inside cells intact. Markdown export renders these as GFM pipe tables. No new tables.
- **LCIR — figure alt text via LLM Vision (Phase 8c, opt-in, incurs API cost)** — describes figure crops with a Vision model and stores the result in `node_alt_texts`. Gated behind its own consent flag, `lcir.vision_alt_text.enabled`, which is **separate from `lcir.enabled` and also off by default**; both must be on. Runs as an explicit batch (never during a build), can be scoped to a single entry or attachment, skips very small crops, and shows the target count before you start. Generated text is stored as `origin='llm_inference'` with a confidence and model name — it never overwrites the author's caption and never enters full-text search. Descriptions carry across extractor versions by crop fingerprint, so a rebuild does not re-bill. Adds migration `0020`.
- **Rebuilding LCIR for an existing library** — raising an extractor version no longer leaves old documents behind: Settings → Data gains a **"rebuild outdated LCIR"** batch (with progress and a guard against concurrent runs), and each attachment row in the detail panel gains a per-attachment build/rebuild button.
- **New LCIR read tools over MCP** — `get_figures` (bounding boxes, figure numbers, captions, crop paths and alt text) and `get_tables` (captions and cell grids), available when `lcir.enabled` is on.

### Notes

- **Disk usage grows when LCIR is enabled and rebuilt.** Figure crops are written as PNGs under `attachments/<entry>/.lcir/` and are **included in backup archives**. On the development library, 888 figures came to roughly 529 MB, taking the backup set from 531 MB to 726 MB. Check free space before enabling and rebuilding a large library.
- **Set the OCR model to `claude-sonnet-5` before generating alt text.** Vision alt text reuses the OCR provider/model settings (`llm.ocr_provider` / `llm.ocr_model`), and description quality differs sharply between models — in our testing only `claude-sonnet-5` was reliably accurate about shapes, mappings and edge labels, while smaller models misread figures or asserted relationships that were not there. For scale, generating alt text for 888 figures with `claude-sonnet-5` cost about **$6**.
- **Default behaviour is unchanged.** All `lcir.*` flags are off by default; full-text search, BibTeX, the Web Clipper and existing CLI commands are unaffected.

## [0.9.0] - 2026-07-23

This release advances **LCIR** — the experimental, machine-readable intermediate representation for papers introduced in v0.8.0 — with three more phases: typed theorem/definition/proof nodes, a cross-reference graph, and symbol definitions. Everything stays gated behind the off-by-default `lcir.enabled` flag, so default behaviour is **byte-for-byte unchanged** and the Web Clipper extension is unchanged from v0.8.0. Two additive migrations (`0017`, `0018`) create new tables that remain empty unless the flag is on — no data migration, and existing libraries upgrade unchanged.

### Added

- **LCIR — typed theorem/definition/proof nodes (Phase 5, experimental)** — recognizes theorem-like environments (theorem, lemma, definition, proof, …) as first-class typed nodes in the document structure, from both PDF text and arXiv TeX source. No new tables.
- **LCIR — cross-reference graph (Phase 6a, experimental)** — a typed, directed edge graph (`node_relations`) linking document nodes: TeX `\ref`/`\eqref`/`\cite` resolved to labels and cite keys, PDF cross-references ("Theorem 2.3") matched by number and kind, and proofs linked to the theorems they prove. Adds migration `0017`.
- **LCIR — symbol definitions (Phase 6b, experimental)** — extracts mathematical symbols and their definitions from inline TeX math in definition sentences (e.g. "let $X$ be…", "$H := …$"). Adds migration `0018`.
- **New LCIR read tools over MCP** — `get_document_blocks` (with a node-kind filter), `get_node_relations`, and `get_symbol_definitions`, available when `lcir.enabled` is on.

## [0.8.0] - 2026-07-22

The headline is **multiple PDF attachments per entry** — a paper and its supplemental material (SI) can now live on the same entry, both readable in the full-screen reader and both full-text searchable. This release also bundles the reliability and code-review fixes accumulated since v0.7.0 (complete backups with automatic restore, full-text self-heal, identifier de-duplication), Web Clipper acquisition hardening, and the first experimental phases of **LCIR**, a machine-readable intermediate representation for papers, gated behind an off-by-default flag. No migration is required; existing libraries upgrade unchanged.

### Added

- **Multiple PDF attachments per entry (body + supplemental)** — an entry can now hold several PDFs (e.g. the main article plus its Supplementary Information) and read them all. The full-screen reader (`DetailView`) gains an **attachment switcher** (shown only when an entry has 2+ attachments) that drives the PDF viewer, OCR, print and detached-window views, plus an **in-reader "add PDF"** control so supplements can be attached without leaving the reader. Each attachment is indexed independently, so a newly added supplement becomes full-text searchable in the background right away. This builds on existing schema and commands — **no migration and no new API**. Returning from a supplement to the body no longer clobbers the body's last-read page.
- **LCIR — machine-readable intermediate representation (experimental, `lcir.enabled` default off)** — the first phases (0–4) of a structured, machine-readable form of each paper, built for AI-agent consumption: a pdfium-based foundation that captures text with page coordinates, bulk backfill of already-attached PDFs with a settings toggle, logical-structure recognition with node-level full-text search, math-surface recognition (`math_expressions`), read tools exposed over MCP, and arXiv **TeX source** ingestion with multiple-representation priority and source switching (raw LaTeX math preserved from the TeX when available). The entire feature sits behind an off-by-default experimental flag and does not affect existing behaviour when disabled.
- **Web Clipper acquisition hardening** — when a clip lands on an entry that already exists, the extension can now **fill in a missing primary** (PDF / TeX source) instead of silently returning `duplicate`, via a service-worker-owned confirmation popup and a `POST /clipper/complete` route. A **bulk TeX-source fetch** (`fetch_missing_arxiv_sources`) backfills arXiv TeX for entries that lack it, and arXiv clips/adds auto-fetch the TeX source when `lcir.enabled` is on. Includes an AddSheet quirk fix. Requires the updated extension (v0.2.0) from this release's assets.
- **Toolbar simplification** — the list toolbar drops the "Columns" and "Sort" buttons, enlarges the search box, moves the metadata/full-text toggle out to the toolbar, and adds a one-click clear button.

### Fixed

- **Complete, self-restoring backups + full-text self-heal (CR-018)** — backups now **bundle the attachment files** into a complete `.zip` (previously the DB only), and a backup can be **restored/imported automatically on the next launch** (a two-phase apply that runs before the app opens the library). The full-text index also **self-heals** on startup when the FTS table is missing or out of sync.
- **Identifier canonicalization + de-duplication (CR-019)** — identifiers (DOI / arXiv) are canonicalized before comparison, duplicates are detected across all add paths, and a best-effort UNIQUE constraint guards against re-introducing them.
- **Code-review fixes (2026-07-11)** — a batch of correctness and robustness fixes from the July code review (36 of 39 findings, with the remaining partial items landed in a follow-up).

## [0.7.0] - 2026-07-05

The headline is a **command-line interface** for reading and writing the library headlessly, built for AI-agent × LaTeX workflows (the `lumencite-bib` Skill) and shell scripting. This release also adds manual/bulk full-text index triggers and one-shot arXiv PDF download when adding an entry. No migration is required; existing libraries upgrade unchanged.

### Added

- **CLI (read + write)** — the app binary doubles as a headless CLI (no new signed/notarized binary): when `argv[1]` is a known subcommand it runs as a CLI instead of launching the GUI, same shape as the existing `--mcp-stdio` bridge. **Read** commands (`search` / `get` / `bib` / `export` / `tags` / `collections` / `fulltext`) open a read-only pool with `PRAGMA query_only = ON`, so they coexist safely as WAL readers whether or not the GUI is running. `bib <citation_key…>` is the LaTeX core command — it emits `refs.bib` while preserving global keys (`smith2020a` won't get mangled). **Write** commands (`add` / `update` / `notes` / `tag` / `collect`) use **hybrid-C routing**: `--force` writes the DB directly (warns that a running app's list may go stale); otherwise if the MCP server is reachable it delegates over HTTP so the write goes through the publish-side write gate, `.bib` sync, and live GUI refresh; if the app is stopped it writes directly and best-effort syncs `.bib`. Both paths share a single source (`mcp_server::handle_rpc_with_write`), so tool logic and the audit log are shared. Output is JSON by default (`--human` for readable text); the DB path follows the Tauri `app_data_dir` rule and can be overridden with `LUMENCITE_DB_PATH`. Destructive commands (`delete` / trash) are out of scope.
- **Manual & bulk full-text index triggers** — attachments are normally indexed on attach, but entries whose PDF was attached earlier or failed to index can now be (re)indexed on demand: the detail panel shows an **index-status badge + index/reindex button** (`index_attachment`) per attachment, and Settings → Data adds **"Index missing PDFs"** (`index_missing_attachments`, which finds un-indexed PDFs via `attachments_without_fulltext` and runs `pdf-extract` → full-text on each). PDFs with no text layer (0 pages) are counted as OCR candidates and steered toward the detail-view OCR.
- **arXiv PDF download on add** — the AddSheet arXiv tab shows an "Also download the PDF from arXiv" checkbox (**default on**) under the metadata preview. On "Add to library", after `create_entry` the app calls `download_arxiv_pdf`, which fetches `https://arxiv.org/pdf/<id>` via the same `download::download_and_attach` path as the Web Clipper (50 MB cap, `%PDF-` validation, timeout), attaches it, and best-effort full-text indexes it in the background. A failed download (paywall / network / bad id) does not block entry creation — the user is pointed at manual attach in the detail panel. arXiv tab only (DOI / ISBN PDFs are publisher-dependent and out of scope).

## [0.6.0] - 2026-07-04

The headline is the **composite entry filter** — the toolbar "Filter" button (a placeholder until now) opens a panel that narrows the list by several conditions at once. This is a broad-audience UX feature that needs no migration; it uses only existing schema.

### Added

- **Composite entry filter** — the list toolbar's Filter button now opens a popover that stacks multiple conditions with **AND**: entry **type** (multi-select, OR within the axis), **year** range (min / max), **starred** (3-state), **has PDF attachment** (3-state), and **tags** (multi-select with an **AND / OR** toggle, independent of the sidebar's single-tag scope). The filter composes with the sidebar view scope (collection / tag / starred / unfiled / trash) and with metadata search (`search_entries`), and is active in the trash view. Filter state persists across view switches until explicitly cleared, and the toolbar shows an active-condition count badge with one-click clear. Full-text search (`fulltext_search`) is out of scope for this release, so the Filter button is disabled there. Backend adds an `EntryFilter` object shared by a `push_filter()` query-builder helper across the FTS / LIKE / plain paths; no migration is required. Unread/read filtering was deferred (documented as a future item, since it needs a schema column).
- **MCP: citation-key lookup and per-entry full text** — two new read tools close gaps in the LaTeX-writing loop: `find_entries_by_citation_keys` batch-resolves `\cite{}` keys to entries (unknown keys are reported as `found:false`), and `get_fulltext` returns the indexed PDF text of one entry (with `indexed:false` made explicit so clients don't fall back to general knowledge, and `page_start` chunked continuation for long papers). `export_bibtex` additionally accepts `citation_keys` and returns `{bibtex, found, missing}` with the same key assignment as the full-library sync, and `get_entry` now also accepts a `citation_key` and reports `has_fulltext`.

## [0.5.0] - 2026-07-03

The headline is the **Web Clipper** — a Chrome extension that saves the paper on the current browser page to LumenCite with one click. This release also adds all-OS update notifications and Codex (OpenAI CLI) support for the MCP server. No migration is needed; the only new setting is `clipper.enabled` (default off), so existing libraries upgrade unchanged.

### Added

- **Web Clipper (Chrome extension)** — a toolbar button that saves the paper on the current page to LumenCite. The extension extracts identifiers (DOI / arXiv / ISBN from `citation_*` meta tags, URL patterns and `doi.org` canonical links); the app resolves metadata (CrossRef / arXiv / OpenLibrary), skips duplicates, and creates the entry. Pages without an identifier are saved as `webpage` entries. arXiv PDFs (and `citation_pdf_url`) are downloaded and attached automatically (50 MB cap, `%PDF-` validation). Served by the existing localhost HTTP server on a new `/clipper` route, gated by its own opt-in toggle (`clipper.enabled`, default off) independent of MCP write access. Pairing uses a copyable connect code from Settings → Chat → Web Clipper. The repository is now a pnpm workspace with the extension under `extension/`. Installation is via load-unpacked from the release zip (Chrome Web Store listing pending) — see the README.
- **Update notification on all platforms** — the Settings → Updates tab now also checks the GitHub Releases API and, when in-app update isn't available (Windows / Linux, whose `latest.json` carries only macOS entries), shows a notify-only "new version available → Open Releases" banner. It only compares versions and opens the Releases page — no download, no updater signing key — so it is safe on every OS and no longer leaves Windows/Linux users unaware of new versions.
- **Codex (OpenAI CLI) MCP support** — the MCP server config snippet generator gains a `codex` target that produces the `[mcp_servers.lumencite]` TOML for `~/.codex/config.toml`, reusing the existing `--mcp-stdio` bridge (Windows backslash paths are TOML-escaped). The Settings → Chat panel shows the ready-to-paste TOML alongside the Claude Code / Claude Desktop snippets. Verified end-to-end against the Codex CLI.
- **BibTeX export hardening** — TeX special characters are escaped on export (with `$…$` math protection so formulas in titles/abstracts survive), and a new option excludes abstract/note fields from all BibTeX outputs.

### Fixed

- Data-loss and race fixes from the 2026-07 code review: OCR no longer destroys the fulltext index on failure, hard-deleting entries removes attachment files from disk, chat write tools trigger `.bib` auto-sync, per-entry PDF page state no longer leaks across entries, shared theme/language state, real app version in Settings, and more (PRs #18 / #19).

## [0.4.0] - 2026-06-29

Two headline features. The entry-type set expands from 6 to 19 (Zotero-aligned), and LumenCite can now act as an **MCP server**, so Claude Desktop / Claude Code can read and (optionally) write your library using your Claude subscription instead of API tokens — LumenCite never calls an LLM itself, so no API key is needed. See `docs/SPEC.md` (「MCP サーバー公開」section) and `docs/API_SPEC.md`.

### Added

- **Entry types 6 → 19 (Zotero-aligned)** — adds `book`, `bookSection`, `thesis`, `report`, `webpage`, `software`, `dataset`, `preprint`, and more. Existing BibTeX type keys are preserved and new types use camelCase; **no migration is needed**. Database changes made by the chat assistant now refresh the entry list in real time.
- **MCP server — read-only (Phase 1)** — LumenCite publishes itself as a localhost HTTP MCP server (JSON-RPC 2.0, `Authorization: Bearer <token>` with the token stored in the OS keychain). Read tools: `fulltext_search` / `get_entry` / `list_collections` / `list_tags` / `search_entries` / `resolve_citation_key` / `export_bibtex`. The settings panel can enable the server, show a running badge, copy the Claude Code connect command, and regenerate the token.
- **MCP server — gated writes + audit log (Phase 2)** — opt-in write tools (**default off**): `add_tag` / `update_notes` / `add_to_collection` / `create_entry` / `update_entry`. `delete_entry` is never exposed. Every write is recorded in an audit log (`mcp_audit_log`, migration 0010) and triggers `.bib` auto-sync plus a live entry-list refresh.
- **MCP server — Claude Desktop bridge (Phase 3)** — Claude Desktop speaks only stdio, so running the app as `lumencite --mcp-stdio` turns it into a stdio↔localhost-HTTP bridge to the in-app server. The settings panel generates the ready-to-paste `mcpServers` JSON. No separate binary is shipped, so there is no extra code-signing surface.
- **Bulk tagging / collections over MCP** — `add_tag` and `add_to_collection` accept an `entry_ids` array to apply to many entries in a single call (best-effort: non-existent entries are skipped and reported in the result).
- **LLM `citation_key` support** — the chat and MCP tools now read and write the pinned BibTeX citation key: `get_entry` returns `citation_key` (and the resolved key), and `create_entry` / `update_entry` accept `citation_key` with uniqueness validation.

### Fixed

- **`update_entry` no longer wipes a pinned `citation_key` or an entry's tags** — the LLM `update_entry` tool previously reset a pinned citation key to `NULL` and could drop existing tags when updating other fields; both are now preserved.

## [0.3.0] - 2026-06-20

Expands the `authors` table for multilingual names (kanji, kana readings, Hangul, Cyrillic), international identifiers beyond ORCID, organizational authors, and a full author editor in the UI. See `docs/SPEC.md` (v0.3.0 section) and `~/.claude/plans/v0-3-0-authors-radiant-kana.md` for details.

### Added

- **Multilingual author fields** (migration 0009) — `middle_name` / `suffix` / `name_particle` for CSL parity, `name_original` + `given_name_original` / `family_name_original` + `original_script` (ISO 15924) for kanji / Hangul / Cyrillic representations, `reading_family` / `reading_given` for kana sort and search, plus `email` / `homepage_url` / `notes` / `updated_at`.
- **`author_identifiers` table** — Normalized storage for non-ORCID identifiers (`scopus`, `dblp`, `semantic_scholar`, `wikidata`, `isni`, `viaf`, `researcher_id`, `google_scholar`, …). `(scheme, value)` is globally unique to prevent the same identifier from being attached to two different authors. ORCID is dual-written to both `authors.orcid` (compat) and `author_identifiers`.
- **Smarter name deduplication** — `get_or_create_author` now matches by ORCID first (across both `authors.orcid` and `author_identifiers (scheme='orcid')`), then by NFKC-normalized lowercase name (so `関 茂樹` / `ＳＥＫＩ` / `seki` / `  Seki  ` collapse to one author), and only inserts if no match is found.
- **Organization authors from BibTeX** — `author = {{IEEE}}` style literals are detected at import and stored with `is_organization=1`. The depth-aware `" and "` splitter protects names like `{Smith and Jones Inc}`.
- **CrossRef ORCID ingestion** — DOI lookups now populate `AuthorInput.orcid` (and `given_name` / `family_name` when available), so authors imported by DOI are correctly merged with existing ORCID entries.
- **FTS now indexes kanji + kana** — `entries_fts.authors_text` concatenates `name`, `name_original`, `reading_family`, and `reading_given`. Searching for `関` / `せき` / `Seki` all hit the same entry. On first launch after upgrade, every entry's FTS is rebuilt once (tracked by `settings.fts.authors_v030_rebuilt`).
- **Author editor modal** (`src/components/AuthorEditor.tsx`) — Edit every author field, manage identifiers, and merge same-name duplicates into one record. Reachable from the detail view and side panel by clicking an author chip, and from the edit sheet via the `…` button next to each saved author.
- **New Tauri commands** — `get_author`, `update_author`, `add_author_identifier`, `delete_author_identifier`. `search_authors` and `merge_authors` are also fully wired up (the former existed but is now richer; the latter is new).
- **Author chip with metadata hover** — The detail view and side panel render authors as chips that show the original-script name, kana reading, and ORCID on hover, and use a building icon for organizational authors.
- **ORCID auto-fill** — The author editor now has a "Fetch from ORCID" button next to the ORCID field. It calls the ORCID Public API (no auth required) and fills in `given_name` / `family_name` / `middle_name` / `email` / `homepage_url` plus any external identifiers (Scopus / ResearcherID / Wikidata / ISNI / VIAF / Loop / …). Existing user-entered values are preserved (only empty fields are filled). For records with non-Latin `other-names`, `name_original` / `original_script` are estimated heuristically (Han / Hangul / Hiragana / Katakana / Cyrillic / Arabic). Reading-kana fields are still entered manually since ORCID has no schema for them.

### Changed

- **`Author` (Rust + TS types) gained 13 fields and an `identifiers: AuthorIdentifier[]`** — Field-by-field deserialization is preserved; the new fields default to `null` for existing entries until the user edits them through the AuthorEditor.
- **`EntryInput` gained `authors?: AuthorInput[]`** — When set (by BibTeX import / CrossRef ingestion / AuthorEditor), it takes precedence over `author_names` and lets ORCIDs and organization flags flow through the create/update path.

## [0.2.1] - 2026-06-18

### Added

- **Windows code signing** — Windows installers (`.msi` / `.exe`) are now Authenticode-signed with a Certum Open Source Code Signing certificate (cloud HSM via SimplySign). SmartScreen reputation builds over download history. (Signed at release time from a local Windows build; SimplySign's interactive login prevents unattended CI signing.)

### Changed

- Editable BibTeX cite keys, graceful DB-init failure handling, MCP server `env` input, and MCP startup-status UI (carried over from the v0.2.1 development line).

### Notes

- The auto-updater remains **macOS-only** for now. Windows updates by manual download from GitHub Releases (Windows auto-updater deferred to avoid risky manual `latest.json` edits that could break the macOS updater).

## [0.2.0] - 2026-05-27

Turns LumenCite into a research sparring partner. See `docs/SPEC.md` (v0.2.0 section) and the implementation plan for details.

### Added

- **Agentic LLM Chat** — A dedicated chat screen where the LLM iteratively searches the full-text index (FTS5) via tool calls to answer questions across multiple references. Per-session context scope: search the whole library or a fixed set of entries. Tool calls (search / DB writes / MCP) are shown as collapsible blocks with a stop button for in-flight streaming.
- **Chat history persistence** — Sessions and messages are stored in SQLite (`chat_sessions` / `chat_messages` / `chat_session_entries`, migration 0007) and reopen from the sidebar after restart. Titles are auto-generated by the LLM (editable).
- **LLM DB-write tools** — The chat LLM can tag entries, append notes, and save OCR text via a per-tool approval whitelist (read & low-risk writes auto-approved; `create_entry` / `update_entry` confirmed each time; `delete_*` and MCP writes always confirmed).
- **MCP client** — The chat LLM can call tools from external MCP servers (e.g. Obsidian). Server config is compatible with Claude Desktop's `mcpServers` JSON. (MCP *server* support is deferred to v0.3.0.)
- **LLM Vision OCR** — Scanned PDFs without a text layer can be OCR'd via the LLM's vision capability and indexed for full-text search, triggered either from the detail view or by the chat LLM. OCR provider is configurable independently from the chat provider.

### Changed

- **Auto-updater enabled on macOS** — `tauri-plugin-updater` is now active for macOS builds, verifying `latest.json` with an ed25519 key. **Windows still requires manual download** from GitHub Releases; Windows signing + updater are planned for v0.2.1.

## [0.1.0] - 2026-05-21

Initial public release.

### Added

- **Entry management** — CRUD for papers, books, conference proceedings, web pages; tags; nested collections; favorites; trash.
- **Auto metadata fetch** — Resolve DOI / arXiv ID / ISBN via CrossRef, arXiv API, and Open Library.
- **PDF viewer** — pdf.js-based 3-pane detail view with page thumbnails, text selection, 3-color highlights (yellow / green / blue), print (⌘P), and zoom 50–200%.
- **LLM summarization** — OpenAI / Anthropic providers, streaming output via `tauri::ipc::Channel`, customizable system prompt, summaries persisted with model + timestamp. API keys stored in the OS keychain (macOS Keychain / Windows Credential Manager / Linux secret-service), never in the SQLite `settings` table.
- **KaTeX** — Render `$…$` / `$$…$$` math in abstracts and notes (`react-markdown` + `remark-math` + `rehype-katex`).
- **BibTeX import / export** — Plus optional auto-sync to a user-specified `.bib` path (debounced 800ms) for VSCode LaTeX Workshop workflows.
- **Command palette (⌘K)** — Global actions and cross-entry search via `cmdk`.
- **i18n & theming** — Japanese / English UI, light / dark / system-follow themes, 4 accent colors. PDF viewer window inherits the theme.
- **Automatic backups** — `VACUUM INTO` snapshots of the SQLite DB on app start and once per day, written to `<app_data_dir>/backups/`, retaining the latest 14 generations.
- **Manual export** — Full data export to JSON, BibTeX, and Markdown (notes + summaries).
- **Keyboard shortcuts** — `←/→` page navigation, `⌘+/⌘-/⌘0` zoom, `⌘F` in-PDF search, `⌘[ / ⌘]` toggle sidebars, `H` highlight, `N` note, `Esc` back.

### Known limitations

- **Auto-updater** is disabled in this release; download new versions manually from GitHub Releases. Will be enabled in a future version with signed update artifacts.
- **Windows installer is unsigned**: SmartScreen will warn on first launch. Click "More info" → "Run anyway". Code signing is planned for a future release once download volume warrants it.
- **macOS** builds are signed with a Developer ID certificate and notarized by Apple.

[Unreleased]: https://github.com/marmot1123/lumencite/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/marmot1123/lumencite/releases/tag/v0.2.0
[0.1.0]: https://github.com/marmot1123/lumencite/releases/tag/v0.1.0
