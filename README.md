# LumenCite

A desktop reference management application built with Tauri 2 + React + TypeScript.

## Requirements

- [Node.js](https://nodejs.org/) and [pnpm](https://pnpm.io/)
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)
- Tauri prerequisites for your OS: https://tauri.app/start/prerequisites/

## Development

```bash
pnpm install
pnpm tauri dev
```

This starts Vite (port 1420) and the Rust backend together with hot-reloading.

## Build

```bash
pnpm tauri build
```

Produces a platform-native installer in `src-tauri/target/release/bundle/`.
