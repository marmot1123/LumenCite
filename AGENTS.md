# AGENTS.md

## Scope

These instructions apply to the entire repository unless a more specific
`AGENTS.md` exists below the directory being changed.

LumenCite is a desktop reference manager built with Tauri 2, Rust, SQLite via
sqlx, React 18, strict TypeScript, and Vite. The repository also contains a
Manifest V3 browser extension.

## Sources of Truth

Read the relevant documents before changing behavior, and keep them aligned
with the implementation:

- `docs/SPEC.md`: feature requirements, accepted behavior, and roadmap
- `docs/DATA_MODEL.md`: database schema and persistence decisions
- `docs/API_SPEC.md`: Tauri commands, CLI behavior, and data contracts
- `docs/CHAT_UI_BRIEF.md`: chat-specific interaction and UI decisions
- `docs/RELEASE.md`: signing, packaging, and release procedure
- `README.md`: supported setup, commands, and user-facing feature overview
- `CHANGELOG.md`: user-visible changes under `[Unreleased]`

Use spec-driven development. For new behavior, update the relevant spec first,
then add tests and implementation. A bug fix only needs a spec change when it
changes or clarifies the documented contract.

Files under `design/` are design references, not the runtime implementation.
When they differ from current specifications or application behavior, confirm
the intended behavior rather than copying them mechanically.

## Repository Layout

- `src/`: React/TypeScript frontend
- `src/App.tsx`: main application orchestration and much of the Tauri invoke wiring
- `src/components/`: reusable UI, including detail, chat, and settings surfaces
- `src/chat/`: chat state and formatting; Zustand state lives in `store.ts`
- `src/types.ts`: frontend DTOs and Tauri data contracts
- `src/i18n/locales/`: Japanese and English application translations
- `src/pdf-viewer.tsx`: entry point for the separate PDF viewer window
- `src-tauri/src/lib.rs`: Tauri setup, application state, commands, and command registration
- `src-tauri/src/db/`: database operations grouped by domain
- `src-tauri/src/models.rs`: Rust DTOs shared across backend boundaries
- `src-tauri/src/llm/`: LLM providers, chat orchestration, and tools
- `src-tauri/src/mcp/`, `mcp_server/`, and `mcp_shim.rs`: MCP integrations
- `src-tauri/src/cli/`: headless CLI behavior
- `src-tauri/migrations/`: ordered, immutable SQLite migrations
- `extension/`: independently built browser extension workspace package

Do not hand-edit generated or dependency directories such as `node_modules/`,
`dist/`, `extension/dist/`, or `src-tauri/target/`.

## Commands

Use Node.js 18+, pnpm 9+, and the stable Rust toolchain. Use `pnpm`; do not
create npm or Yarn lockfiles.

Run commands from the repository root unless a command explicitly changes
directory:

```bash
pnpm install
pnpm tauri dev
pnpm build
(cd src-tauri && cargo test)
pnpm --filter lumencite-clipper test
pnpm --filter lumencite-clipper build
pnpm tauri build
```

- `pnpm tauri dev` runs the complete application with Rust commands available.
- `pnpm dev` runs only Vite on fixed port `1420`; use it only for frontend work
  that does not require the Tauri runtime.
- `pnpm build` runs strict TypeScript checking and the production Vite build.
- Run Rust tests from `src-tauri/` so sqlx migration paths resolve consistently.
- `pnpm tauri build` creates platform bundles and is not required for every
  routine code change.

There is currently no repository-wide lint command or frontend unit-test suite.
Do not claim those checks ran. Rust formatting is not currently a clean
repository-wide gate; avoid unrelated formatting churn.

## Development Workflow

Follow Red-Green-Refactor where practical:

1. Add a failing test that describes the intended behavior.
2. Implement the smallest correct change.
3. Refactor while keeping the relevant tests green.

Choose the test type that matches the code:

- Use `#[sqlx::test(migrations = "./migrations")]` for database behavior. Each
  test gets a fresh SQLite database with all migrations; no `DATABASE_URL` is
  needed.
- Use `#[test]` or `#[tokio::test]` for Rust unit and async behavior.
- Add Vitest coverage under `extension/test/` for extension behavior.
- For frontend changes, run `pnpm build` and smoke-test affected Tauri flows
  when behavior depends on the desktop runtime.

Run the narrowest relevant test while iterating, then the complete applicable
checks before finishing. Cross-stack changes require both frontend and backend
validation.

## Cross-Layer Contracts

For a new or changed Tauri command:

1. Define the Rust command with `#[tauri::command]` and keep reusable domain or
   database logic in the appropriate module.
2. Register it in the `tauri::generate_handler!` list in `src-tauri/src/lib.rs`.
3. Update the frontend invocation and keep `src/types.ts`,
   `src-tauri/src/models.rs`, and `docs/API_SPEC.md` synchronized.
4. Preserve established serialization names. Top-level invoke arguments are
   normally camelCase in TypeScript; nested DTO fields follow their serde/API
   contract.

Adding a Tauri plugin or privileged API usually also requires a capability
change in `src-tauri/capabilities/default.json`. Changes specific to the PDF
window may require `src-tauri/capabilities/pdf-viewer.json` as well.

Core entry mutations can be reached through the GUI, CLI, MCP server, and web
clipper. Consider all affected entry points and preserve existing BibTeX sync,
UI refresh events, and MCP audit behavior.

## Database Rules

- SQLite is owned by `AppState`; startup applies `sqlx::migrate!`.
- WAL mode and foreign keys are configured in Rust connection options, not in
  migrations.
- Never edit an already-applied migration. Inspect the current highest number
  and add the next zero-padded `src-tauri/migrations/NNNN_description.sql` file.
- Update `docs/DATA_MODEL.md` for schema or persistence-design changes.
- Put database behavior in `src-tauri/src/db/` and cover it with sqlx tests.

## Frontend and Extension Rules

- Follow existing React component, hook, Zustand, and CSS token patterns.
- Keep TypeScript strict; avoid `any`, and centralize shared frontend DTOs in
  `src/types.ts` instead of redefining them inside components.
- Route application UI text through i18next and update both
  `src/i18n/locales/ja.json` and `src/i18n/locales/en.json` together.
- Use the existing Lucide-backed icon patterns in `src/components/icons.tsx`
  where an appropriate icon already exists.
- Treat the browser extension as a separate package. Edit `extension/src/` and
  `extension/public/`, then generate `extension/dist/` with its build command.
- Extension translations under `extension/public/_locales/` are separate from
  the desktop application's translations.

## Security and Releases

- Keep provider API keys in the OS keychain. Never log, persist in plain text,
  or commit secrets, MCP tokens, or clipper connection codes.
- Preserve loopback-only binding and token authentication for MCP server and
  web clipper endpoints unless the security model is explicitly redesigned.
- Do not change versions or release artifacts as part of an ordinary feature or
  fix unless requested.
- For an application version bump, keep `package.json`,
  `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json` synchronized.
- The extension version is independent and must match between
  `extension/package.json` and `extension/manifest.json`.
- Follow `docs/RELEASE.md` rather than inferring signing or publishing steps.
