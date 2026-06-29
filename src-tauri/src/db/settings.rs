use sqlx::SqlitePool;

/// settings テーブルの単純な key-value 取得。未設定なら None。
pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

/// upsert。空文字も「設定されている空文字」として保存する（呼び出し側で適宜 delete を使う）。
pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_setting(pool: &SqlitePool, key: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// BibTeX 同期先パスの設定キー。
pub const BIBTEX_SYNC_PATH_KEY: &str = "bibtex_sync_path";

pub const LLM_PROVIDER_KEY: &str = "llm.provider";
pub const LLM_MODEL_KEY: &str = "llm.model";
pub const LLM_SUMMARY_SOURCE_KEY: &str = "llm.summary_source";
pub const LLM_SUMMARY_PROMPT_KEY: &str = "llm.summary_prompt";

/// Chat のツール別自動承認ホワイトリスト（JSON: tool_name -> bool）。
pub const CHAT_TOOL_WHITELIST_KEY: &str = "chat.tool_whitelist";

/// 外部 MCP サーバー設定（Claude Desktop の mcpServers 互換 JSON）。
pub const MCP_SERVERS_KEY: &str = "mcp.servers";

/// LumenCite 自身を MCP サーバーとして公開する機能の有効フラグ（"1" で有効）。
pub const MCP_SERVER_ENABLED_KEY: &str = "mcp_server.enabled";
/// MCP サーバーのバインドポート（未設定なら `mcp_server::DEFAULT_PORT`）。
pub const MCP_SERVER_PORT_KEY: &str = "mcp_server.port";

/// OCR 用 LLM プロバイダ / モデル（未設定なら chat の provider / model にフォールバック）。
pub const LLM_OCR_PROVIDER_KEY: &str = "llm.ocr_provider";
pub const LLM_OCR_MODEL_KEY: &str = "llm.ocr_model";

/// v0.3.0 で entries_fts.authors_text の合成 SQL が name_original / reading_* も
/// 含む形に変わったため、既存ライブラリの FTS を 1 回だけ起動時に再構築するフラグ。
/// 値は "1"（再構築済み）のみで、未設定なら未実施扱い。
pub const FTS_AUTHORS_V030_REBUILT_KEY: &str = "fts.authors_v030_rebuilt";

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn get_setting_returns_none_for_unset_key(pool: SqlitePool) {
        let v = get_setting(&pool, "missing").await.unwrap();
        assert_eq!(v, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_then_get_roundtrips(pool: SqlitePool) {
        set_setting(&pool, "k1", "hello").await.unwrap();
        let v = get_setting(&pool, "k1").await.unwrap();
        assert_eq!(v.as_deref(), Some("hello"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_setting_upserts_existing_key(pool: SqlitePool) {
        set_setting(&pool, "k1", "first").await.unwrap();
        set_setting(&pool, "k1", "second").await.unwrap();
        let v = get_setting(&pool, "k1").await.unwrap();
        assert_eq!(v.as_deref(), Some("second"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_setting_removes_key(pool: SqlitePool) {
        set_setting(&pool, "k1", "v").await.unwrap();
        delete_setting(&pool, "k1").await.unwrap();
        let v = get_setting(&pool, "k1").await.unwrap();
        assert_eq!(v, None);
    }
}
