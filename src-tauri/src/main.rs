// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // MCP Phase 3: Claude Desktop 向け stdio shim。GUI/Tauri を起動せず、
    // stdio ↔ localhost HTTP プロキシとして本体内蔵 MCP サーバーへ橋渡しする。
    if std::env::args().any(|a| a == "--mcp-stdio") {
        std::process::exit(lumencite_lib::mcp_shim::run_stdio_proxy());
    }
    // v0.7.0 CLI: argv[1] が既知のサブコマンドなら GUI を起動せずヘッドレス実行する。
    let args: Vec<String> = std::env::args().collect();
    if lumencite_lib::cli::is_cli_invocation(&args) {
        std::process::exit(lumencite_lib::cli::run());
    }
    lumencite_lib::run()
}
