use sqlx::SqlitePool;

/// チャットセッション 1 行。`entry_count` は `chat_session_entries` の件数を毎回投影する。
/// `tool_calls` / `tool_call_id` の構造化（`ToolCallSpec`）は上位層（agentic ループ / Tauri 層）で行い、
/// DB 層では生 JSON 文字列のまま保持する。
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct ChatSession {
    pub id: i64,
    pub title: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: Option<String>,
    pub scope_mode: String, // 'all' | 'entries'
    pub entry_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct ChatMessage {
    pub id: i64,
    pub session_id: i64,
    pub role: String, // 'user' | 'assistant' | 'tool'
    pub content: String,
    pub tool_calls: Option<String>,   // JSON: assistant のツール呼び出し列
    pub tool_call_id: Option<String>, // role='tool' の結果が紐づく ID
    pub created_at: String,
    pub position: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct SessionWithMessages {
    pub session: ChatSession,
    pub messages: Vec<ChatMessage>,
    pub entry_ids: Vec<i64>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct NewChatSession {
    pub title: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: Option<String>,
    pub scope_mode: String,
    pub entry_ids: Vec<i64>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct NewChatMessage {
    pub session_id: i64,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<String>,
    pub tool_call_id: Option<String>,
}

fn valid_scope_mode(mode: &str) -> bool {
    matches!(mode, "all" | "entries")
}

fn valid_role(role: &str) -> bool {
    matches!(role, "user" | "assistant" | "tool")
}

const SESSION_COLUMNS: &str = "id, title, provider, model, system_prompt, scope_mode,
    (SELECT COUNT(*) FROM chat_session_entries WHERE session_id = chat_sessions.id) AS entry_count,
    created_at, updated_at, archived_at";

/// セッションを作成する。`scope_mode='entries'` を含め `entry_ids` はそのまま登録する。
pub async fn create_session(
    pool: &SqlitePool,
    input: &NewChatSession,
) -> Result<ChatSession, sqlx::Error> {
    if !valid_scope_mode(&input.scope_mode) {
        return Err(sqlx::Error::Protocol(format!(
            "invalid scope_mode: {}",
            input.scope_mode
        )));
    }

    let mut tx = pool.begin().await?;
    let id = sqlx::query(
        "INSERT INTO chat_sessions (title, provider, model, system_prompt, scope_mode)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&input.title)
    .bind(&input.provider)
    .bind(&input.model)
    .bind(&input.system_prompt)
    .bind(&input.scope_mode)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    for entry_id in &input.entry_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO chat_session_entries (session_id, entry_id) VALUES (?, ?)",
        )
        .bind(id)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    get_session(pool, id).await
}

/// 非アーカイブのセッションを更新日時降順で返す。サイドバー用。
pub async fn list_sessions(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<ChatSession>, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM chat_sessions
         WHERE archived_at IS NULL
         ORDER BY updated_at DESC, id DESC
         LIMIT ? OFFSET ?"
    ))
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
}

/// 単一セッションを取得する。
pub async fn get_session(pool: &SqlitePool, id: i64) -> Result<ChatSession, sqlx::Error> {
    sqlx::query_as(&format!(
        "SELECT {SESSION_COLUMNS} FROM chat_sessions WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
}

/// セッション本体 + メッセージ列（position 昇順）+ スコープ対象 entry_id 集合。
pub async fn get_session_with_messages(
    pool: &SqlitePool,
    id: i64,
) -> Result<SessionWithMessages, sqlx::Error> {
    let session = get_session(pool, id).await?;
    let messages = sqlx::query_as(
        "SELECT id, session_id, role, content, tool_calls, tool_call_id, created_at, position
         FROM chat_messages WHERE session_id = ? ORDER BY position ASC, id ASC",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let entry_ids = get_session_entries(pool, id).await?;
    Ok(SessionWithMessages {
        session,
        messages,
        entry_ids,
    })
}

/// タイトルを更新し、`updated_at` を現在時刻に更新する。
pub async fn update_title(
    pool: &SqlitePool,
    id: i64,
    title: &str,
) -> Result<ChatSession, sqlx::Error> {
    let rows =
        sqlx::query("UPDATE chat_sessions SET title = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(pool)
            .await?
            .rows_affected();
    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    get_session(pool, id).await
}

/// セッションをソフト削除する（`archived_at` をセット）。
pub async fn archive_session(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let rows = sqlx::query(
        "UPDATE chat_sessions SET archived_at = datetime('now') WHERE id = ? AND archived_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

/// アーカイブを取り消す（`archived_at` を NULL に戻す）。一覧上の位置は元の updated_at を保つ。
pub async fn unarchive_session(pool: &SqlitePool, id: i64) -> Result<ChatSession, sqlx::Error> {
    let rows = sqlx::query("UPDATE chat_sessions SET archived_at = NULL WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();
    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    get_session(pool, id).await
}

/// メッセージを末尾に追加する。`position` は当該セッション内の最大値 +1（最初は 0）。
/// 追加に伴いセッションの `updated_at` を更新する。
pub async fn append_message(
    pool: &SqlitePool,
    input: &NewChatMessage,
) -> Result<ChatMessage, sqlx::Error> {
    if !valid_role(&input.role) {
        return Err(sqlx::Error::Protocol(format!(
            "invalid role: {}",
            input.role
        )));
    }

    let mut tx = pool.begin().await?;
    let position: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM chat_messages WHERE session_id = ?",
    )
    .bind(input.session_id)
    .fetch_one(&mut *tx)
    .await?;

    let id = sqlx::query(
        "INSERT INTO chat_messages (session_id, role, content, tool_calls, tool_call_id, position)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(input.session_id)
    .bind(&input.role)
    .bind(&input.content)
    .bind(&input.tool_calls)
    .bind(&input.tool_call_id)
    .bind(position)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    sqlx::query("UPDATE chat_sessions SET updated_at = datetime('now') WHERE id = ?")
        .bind(input.session_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    sqlx::query_as(
        "SELECT id, session_id, role, content, tool_calls, tool_call_id, created_at, position
         FROM chat_messages WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
}

/// スコープ対象の entry 集合を入れ替える（全削除 → 再登録）。
pub async fn set_session_entries(
    pool: &SqlitePool,
    session_id: i64,
    entry_ids: &[i64],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM chat_session_entries WHERE session_id = ?")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    for entry_id in entry_ids {
        sqlx::query(
            "INSERT OR IGNORE INTO chat_session_entries (session_id, entry_id) VALUES (?, ?)",
        )
        .bind(session_id)
        .bind(entry_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE chat_sessions SET updated_at = datetime('now') WHERE id = ?")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// スコープ対象の entry_id を昇順で返す。
pub async fn get_session_entries(
    pool: &SqlitePool,
    session_id: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT entry_id FROM chat_session_entries WHERE session_id = ? ORDER BY entry_id ASC",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
}

/// scope_mode と対象 entry 集合をまとめて更新する（ScopePicker の「適用」）。
pub async fn set_scope(
    pool: &SqlitePool,
    id: i64,
    scope_mode: &str,
    entry_ids: &[i64],
) -> Result<ChatSession, sqlx::Error> {
    if !valid_scope_mode(scope_mode) {
        return Err(sqlx::Error::Protocol(format!(
            "invalid scope_mode: {scope_mode}"
        )));
    }
    let rows = sqlx::query(
        "UPDATE chat_sessions SET scope_mode = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(scope_mode)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();
    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    set_session_entries(pool, id, entry_ids).await?;
    get_session(pool, id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    async fn make_entry(pool: &SqlitePool, title: &str) -> i64 {
        create_entry(
            pool,
            &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .id
    }

    fn session(scope_mode: &str, entry_ids: Vec<i64>) -> NewChatSession {
        NewChatSession {
            title: "Untitled".to_string(),
            provider: "anthropic".to_string(),
            model: "sonnet".to_string(),
            system_prompt: None,
            scope_mode: scope_mode.to_string(),
            entry_ids,
        }
    }

    fn msg(session_id: i64, role: &str, content: &str) -> NewChatMessage {
        NewChatMessage {
            session_id,
            role: role.to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_all_scope_session(pool: SqlitePool) {
        let s = create_session(&pool, &session("all", vec![])).await.unwrap();
        assert!(s.id > 0);
        assert_eq!(s.scope_mode, "all");
        assert_eq!(s.entry_count, 0);
        assert!(s.archived_at.is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_entries_scope_session_records_entries(pool: SqlitePool) {
        let e1 = make_entry(&pool, "A").await;
        let e2 = make_entry(&pool, "B").await;
        let s = create_session(&pool, &session("entries", vec![e1, e2]))
            .await
            .unwrap();
        assert_eq!(s.entry_count, 2);

        let ids = get_session_entries(&pool, s.id).await.unwrap();
        assert_eq!(ids, vec![e1, e2]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_invalid_scope_mode(pool: SqlitePool) {
        let res = create_session(&pool, &session("bogus", vec![])).await;
        assert!(res.is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_orders_by_updated_desc(pool: SqlitePool) {
        let a = create_session(&pool, &session("all", vec![])).await.unwrap();
        let b = create_session(&pool, &session("all", vec![])).await.unwrap();
        // datetime('now') は秒解像度なので、updated_at を明示的にずらして順序を検証する。
        // a を新しく、b を古くする → a が先頭に来るべき。
        sqlx::query("UPDATE chat_sessions SET updated_at = '2002-01-01 00:00:00' WHERE id = ?")
            .bind(a.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE chat_sessions SET updated_at = '2001-01-01 00:00:00' WHERE id = ?")
            .bind(b.id)
            .execute(&pool)
            .await
            .unwrap();

        let list = list_sessions(&pool, 50, 0).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, a.id);
        assert_eq!(list[1].id, b.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_title_changes_title_and_bumps_updated_at(pool: SqlitePool) {
        let s = create_session(&pool, &session("all", vec![])).await.unwrap();
        sqlx::query("UPDATE chat_sessions SET updated_at = '2000-01-01 00:00:00' WHERE id = ?")
            .bind(s.id)
            .execute(&pool)
            .await
            .unwrap();

        let updated = update_title(&pool, s.id, "Quantum walks").await.unwrap();
        assert_eq!(updated.title, "Quantum walks");
        assert_ne!(updated.updated_at, "2000-01-01 00:00:00");

        assert!(update_title(&pool, 9999, "x").await.is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn append_messages_assigns_sequential_positions(pool: SqlitePool) {
        let s = create_session(&pool, &session("all", vec![])).await.unwrap();
        let m0 = append_message(&pool, &msg(s.id, "user", "hi")).await.unwrap();
        let m1 = append_message(&pool, &msg(s.id, "assistant", "yo"))
            .await
            .unwrap();
        assert_eq!(m0.position, 0);
        assert_eq!(m1.position, 1);

        let with = get_session_with_messages(&pool, s.id).await.unwrap();
        assert_eq!(with.messages.len(), 2);
        assert_eq!(with.messages[0].content, "hi");
        assert_eq!(with.messages[1].content, "yo");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn append_rejects_invalid_role(pool: SqlitePool) {
        let s = create_session(&pool, &session("all", vec![])).await.unwrap();
        let res = append_message(&pool, &msg(s.id, "system", "x")).await;
        assert!(res.is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn append_bumps_session_updated_at(pool: SqlitePool) {
        let s = create_session(&pool, &session("all", vec![])).await.unwrap();
        // updated_at を過去にずらしておく
        sqlx::query("UPDATE chat_sessions SET updated_at = '2000-01-01 00:00:00' WHERE id = ?")
            .bind(s.id)
            .execute(&pool)
            .await
            .unwrap();
        append_message(&pool, &msg(s.id, "user", "hi")).await.unwrap();
        let after = get_session(&pool, s.id).await.unwrap();
        assert_ne!(after.updated_at, "2000-01-01 00:00:00");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_hides_from_list(pool: SqlitePool) {
        let s = create_session(&pool, &session("all", vec![])).await.unwrap();
        archive_session(&pool, s.id).await.unwrap();
        let list = list_sessions(&pool, 50, 0).await.unwrap();
        assert!(list.is_empty());
        // 二重アーカイブはエラー
        assert!(archive_session(&pool, s.id).await.is_err());

        // unarchive で一覧に戻る
        let restored = unarchive_session(&pool, s.id).await.unwrap();
        assert!(restored.archived_at.is_none());
        assert_eq!(list_sessions(&pool, 50, 0).await.unwrap().len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_session_entries_replaces_set(pool: SqlitePool) {
        let e1 = make_entry(&pool, "A").await;
        let e2 = make_entry(&pool, "B").await;
        let e3 = make_entry(&pool, "C").await;
        let s = create_session(&pool, &session("entries", vec![e1, e2]))
            .await
            .unwrap();

        set_session_entries(&pool, s.id, &[e2, e3]).await.unwrap();
        let ids = get_session_entries(&pool, s.id).await.unwrap();
        assert_eq!(ids, vec![e2, e3]);
        assert_eq!(get_session(&pool, s.id).await.unwrap().entry_count, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_scope_updates_mode_and_entries(pool: SqlitePool) {
        let e1 = make_entry(&pool, "A").await;
        let e2 = make_entry(&pool, "B").await;
        let s = create_session(&pool, &session("all", vec![])).await.unwrap();

        let updated = set_scope(&pool, s.id, "entries", &[e1, e2]).await.unwrap();
        assert_eq!(updated.scope_mode, "entries");
        assert_eq!(updated.entry_count, 2);

        // "all" に戻すと entry をクリアする運用
        let back = set_scope(&pool, s.id, "all", &[]).await.unwrap();
        assert_eq!(back.scope_mode, "all");
        assert_eq!(back.entry_count, 0);

        assert!(set_scope(&pool, s.id, "bogus", &[]).await.is_err());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cascade_deletes_messages_and_entries(pool: SqlitePool) {
        let e1 = make_entry(&pool, "A").await;
        let s = create_session(&pool, &session("entries", vec![e1]))
            .await
            .unwrap();
        append_message(&pool, &msg(s.id, "user", "hi")).await.unwrap();

        sqlx::query("DELETE FROM chat_sessions WHERE id = ?")
            .bind(s.id)
            .execute(&pool)
            .await
            .unwrap();

        let msgs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages")
            .fetch_one(&pool)
            .await
            .unwrap();
        let ents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_session_entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(msgs, 0);
        assert_eq!(ents, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_entry_cascades_to_session_entries(pool: SqlitePool) {
        let e1 = make_entry(&pool, "A").await;
        let s = create_session(&pool, &session("entries", vec![e1]))
            .await
            .unwrap();
        sqlx::query("DELETE FROM entries WHERE id = ?")
            .bind(e1)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(get_session(&pool, s.id).await.unwrap().entry_count, 0);
    }
}
