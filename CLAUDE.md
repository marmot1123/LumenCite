# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

LumenCite is a desktop reference (bibliography) management application. The stack is **Tauri 2** (Rust backend) + **React 18 + TypeScript** (frontend) + **Vite**.

## Commands

| Task | Command |
|------|---------|
| Dev (frontend + Rust) | `pnpm tauri dev` |
| Frontend only | `pnpm dev` |
| Type-check + build frontend | `pnpm build` |
| Build distributable | `pnpm tauri build` |

The Vite dev server is fixed to port **1420**. `pnpm tauri dev` must be used (not `pnpm dev`) to get the full Tauri runtime with Rust commands available.

## Development methodology

This project uses **spec-driven development** combined with **TDD (Test-Driven Development)**.

### Spec-driven development

Specifications are written before implementation. The docs to consult and keep up to date are:

- `docs/SPEC.md` — feature requirements and phasing
- `docs/DATA_MODEL.md` — DB schema and design decisions
- `docs/API_SPEC.md` — Tauri command signatures and data types

When adding a new feature, update the relevant spec doc first, then implement.

### TDD cycle (Red → Green → Refactor)

1. **Red** — write a failing test that describes the desired behaviour (`#[sqlx::test]` for DB functions). Run `cargo test` and confirm it fails.
2. **Green** — write the minimum code to make the test pass. Run `cargo test` and confirm it passes.
3. **Refactor** — clean up while keeping tests green.

DB function tests use `#[sqlx::test(migrations = "./migrations")]`, which spins up a fresh SQLite database per test and runs all migrations automatically. No `DATABASE_URL` is needed for tests.

Run tests with:

```
cargo test
```

## Architecture

The project follows the standard Tauri 2 layout:

- `src/` — React/TypeScript frontend
- `src-tauri/src/` — Rust backend
  - `lib.rs` — Tauri commands and `run()` entry point
  - `main.rs` — binary entry point that calls `lib::run()`
- `src-tauri/tauri.conf.json` — app metadata, window config, build hooks

### Frontend ↔ Backend communication

Rust functions exposed to the frontend are annotated with `#[tauri::command]` and registered in `tauri::Builder::invoke_handler`. The frontend calls them with:

```ts
import { invoke } from "@tauri-apps/api/core";
const result = await invoke("command_name", { param: value });
```

New Tauri commands must be added to both `lib.rs` (definition + `#[tauri::command]`) and the `invoke_handler` macro.

### Database

SQLite via `sqlx 0.8`. The DB file is stored in the OS app data directory (`app.path().app_data_dir()`). On first launch, `sqlx::migrate!` runs all files in `src-tauri/migrations/` in order.

`AppState { db: SqlitePool }` is registered with `app.manage()` in `setup` and accessed in commands via `state: State<AppState>`.

WAL mode and `foreign_keys = ON` are set through `SqliteConnectOptions` on every connection (not in migrations).

New migrations must be added as `src-tauri/migrations/<N+1>_<description>.sql`. Never edit an already-applied migration.

### Capabilities / permissions

`src-tauri/capabilities/default.json` declares which Tauri plugin APIs the frontend is allowed to call. Adding a new plugin usually requires updating this file.
