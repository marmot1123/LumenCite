# Changelog

All notable changes to LumenCite will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/marmot1123/lumencite/releases/tag/v0.1.0
