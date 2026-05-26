-- Chat: agentic LLM チャットのセッションとメッセージ履歴 (v0.2.0)

CREATE TABLE chat_sessions (
    id            INTEGER PRIMARY KEY,
    title         TEXT    NOT NULL,
    provider      TEXT    NOT NULL,
    model         TEXT    NOT NULL,
    system_prompt TEXT,
    scope_mode    TEXT    NOT NULL DEFAULT 'all' CHECK (scope_mode IN ('all', 'entries')),
    created_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT    NOT NULL DEFAULT (datetime('now')),
    archived_at   TEXT
);

CREATE TABLE chat_messages (
    id           INTEGER PRIMARY KEY,
    session_id   INTEGER NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    role         TEXT    NOT NULL CHECK (role IN ('user', 'assistant', 'tool')),
    content      TEXT    NOT NULL,
    tool_calls   TEXT,   -- JSON: assistant のツール呼び出し列
    tool_call_id TEXT,   -- role='tool' の結果が紐づく ID
    created_at   TEXT    NOT NULL DEFAULT (datetime('now')),
    position     INTEGER NOT NULL
);

CREATE TABLE chat_session_entries (
    session_id INTEGER NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
    entry_id   INTEGER NOT NULL REFERENCES entries(id)       ON DELETE CASCADE,
    PRIMARY KEY (session_id, entry_id)
);

CREATE INDEX idx_chat_messages_session ON chat_messages(session_id, position);
CREATE INDEX idx_chat_sessions_updated ON chat_sessions(updated_at DESC);
