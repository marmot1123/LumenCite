// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // MCP Phase 3: Claude Desktop 向け stdio shim。GUI/Tauri を起動せず、
    // stdio ↔ localhost HTTP プロキシとして本体内蔵 MCP サーバーへ橋渡しする。
    if std::env::args().any(|a| a == "--mcp-stdio") {
        std::process::exit(lumencite_lib::mcp_shim::run_stdio_proxy());
    }
    lumencite_lib::run()
}
