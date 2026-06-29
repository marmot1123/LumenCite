-- MCP サーバー公開（Phase 2）経由の write ツール実行を記録する監査ログ。
-- 外部 MCP クライアント（Claude Desktop/Code 等）からの書き込みを追跡するための
-- append-only ログ。read 系は記録しない（機微でなくノイズになるため）。
CREATE TABLE mcp_audit_log (
    id INTEGER PRIMARY KEY,
    tool_name  TEXT NOT NULL,
    arguments  TEXT NOT NULL,          -- ツール引数（JSON 文字列）
    result     TEXT,                   -- 成功サマリ or エラーメッセージ
    is_error   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX ix_mcp_audit_log_id_desc ON mcp_audit_log(id DESC);
