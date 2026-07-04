# Changelog

All notable changes to LumenCite will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] - 2026-07-04

The headline is the **composite entry filter** — the toolbar "Filter" button (a placeholder until now) opens a panel that narrows the list by several conditions at once. This is a broad-audience UX feature that needs no migration; it uses only existing schema.

### Added

- **Composite entry filter** — the list toolbar's Filter button now opens a popover that stacks multiple conditions with **AND**: entry **type** (multi-select, OR within the axis), **year** range (min / max), **starred** (3-state), **has PDF attachment** (3-state), and **tags** (multi-select with an **AND / OR** toggle, independent of the sidebar's single-tag scope). The filter composes with the sidebar view scope (collection / tag / starred / unfiled / trash) and with metadata search (`search_entries`), and is active in the trash view. Filter state persists across view switches until explicitly cleared, and the toolbar shows an active-condition count badge with one-click clear. Full-text search (`fulltext_search`) is out of scope for this release, so the Filter button is disabled there. Backend adds an `EntryFilter` object shared by a `push_filter()` query-builder helper across the FTS / LIKE / plain paths; no migration is required. Unread/read filtering was deferred (documented as a future item, since it needs a schema column).

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
