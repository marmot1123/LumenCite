//! MCP サーバー公開（Phase 2）の write ツール実行を記録する監査ログ。
//!
//! 外部 MCP クライアントからの書き込みは承認 UI を介さないため、何が・いつ・成否
//! を後から追えるように append-only で残す。read 系は記録しない。

use sqlx::SqlitePool;

#[derive(Debug, serde::Serialize, sqlx::FromRow, Clone)]
pub struct McpAuditEntry {
    pub id: i64,
    pub tool_name: String,
    pub arguments: String,
    pub result: Option<String>,
    pub is_error: bool,
    pub created_at: String,
}

/// write ツール実行を 1 件記録する。記録自体の失敗はツール実行の成否に影響させない
/// 想定（呼び出し側は `let _ =` で握りつぶす）。
pub async fn record(
    pool: &SqlitePool,
    tool_name: &str,
    arguments: &str,
    result: &str,
    is_error: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO mcp_audit_log (tool_name, arguments, result, is_error)
         VALUES (?, ?, ?, ?)",
    )
    .bind(tool_name)
    .bind(arguments)
    .bind(result)
    .bind(is_error as i64)
    .execute(pool)
    .await?;
    Ok(())
}

/// 直近の監査ログを新しい順で返す。
pub async fn recent(pool: &SqlitePool, limit: i64) -> Result<Vec<McpAuditEntry>, sqlx::Error> {
    sqlx::query_as::<_, McpAuditEntry>(
        "SELECT id, tool_name, arguments, result, is_error, created_at
         FROM mcp_audit_log ORDER BY id DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn record_then_recent_roundtrips(pool: SqlitePool) {
        record(&pool, "add_tag", "{\"tag_name\":\"ml\"}", "ok", false)
            .await
            .unwrap();
        record(&pool, "create_entry", "{\"title\":\"X\"}", "failed: dup", true)
            .await
            .unwrap();

        let rows = recent(&pool, 10).await.unwrap();
        assert_eq!(rows.len(), 2);
        // 新しい順（id DESC）。
        assert_eq!(rows[0].tool_name, "create_entry");
        assert!(rows[0].is_error);
        assert_eq!(rows[1].tool_name, "add_tag");
        assert!(!rows[1].is_error);
    }
}
