//! Agentic chat ループ。
//!
//! `ChatProvider` を反復呼び出しし、ツール呼び出しがあれば
//! 承認チェック（[`tools::approval`]）→ 実行（[`tools::execute_tool`]）→
//! 結果を会話に追加、を「これ以上ツールを呼ばない」ターンに達するまで繰り返す。
//!
//! - assistant の自然言語テキストは同期コールバック `on_delta`（[`ChatLoopHost::on_delta`]）で素通しする。
//! - ツール提案 / 実行結果 / メッセージ永続化の通知、承認待ち、キャンセル判定は [`ChatLoopHost`] が担う。
//!   Tauri レベルのストリーミング（`ChatStreamEvent`）と承認の往復（`approve_tool_call`）は #11 が
//!   この trait を実装して接続する。テストは記録用モックで検証する。
//! - 会話履歴（assistant / tool メッセージ）は [`crate::db::chat`] に逐次永続化する。

use super::{ChatMessage, ChatProvider, ContentBlock, LlmError, Role, ToolCallSpec, ToolSpec};
use crate::db;
use crate::db::chat::NewChatMessage;
use crate::llm::tools::{self, ToolContext};

/// 暴走防止のためのデフォルト最大ターン数。
pub const DEFAULT_MAX_TURNS: usize = 12;

/// セッションに system_prompt が無い場合に使う既定のチャット用システムプロンプト。
pub const DEFAULT_CHAT_SYSTEM_PROMPT: &str = "You are a research assistant embedded in LumenCite, \
a reference manager. Answer the user's questions about their library by calling the provided tools \
to search the full text and read entries before answering — do not rely on prior knowledge for \
library-specific facts. Cite the entries you used by id and title. Be concise and precise. \
When the user asks you to modify the library (tag, note, create, etc.), use the appropriate tool. \
Reply in the same language as the user.";

/// ループのスカラ設定。
pub struct ChatLoopParams<'a> {
    pub api_key: &'a str,
    pub model: &'a str,
    pub system: &'a str,
    /// settings の `chat.tool_whitelist`（None なら既定ポリシー）
    pub whitelist: Option<&'a str>,
    pub max_turns: usize,
}

/// 進行中のループが呼び出し側（#11 の Tauri 層 / テスト）に通知・問い合わせするためのフック。
#[async_trait::async_trait]
pub trait ChatLoopHost: Send {
    /// assistant 自然言語テキストのトークン到着ごと（同期）。
    fn on_delta(&mut self, text: &str);
    /// ツール呼び出しが提案された（実行前）。`needs_approval=true` なら直後に [`request_approval`] が呼ばれる。
    async fn on_tool_proposed(&mut self, call: &ToolCallSpec, needs_approval: bool);
    /// 承認が必要なツールについて、許可（true）/拒否（false）を待つ。
    async fn request_approval(&mut self, call: &ToolCallSpec) -> bool;
    /// ツール実行が完了した（結果テキスト要約付き）。
    async fn on_tool_executed(&mut self, call_id: &str, result_summary: &str);
    /// メッセージが DB に永続化された。
    async fn on_message_persisted(&mut self, message_id: i64, role: Role);
    /// 中断要求が来ているか（毎ターン頭で確認）。
    fn is_cancelled(&self) -> bool;
}

/// assistant の各 `tool_use` には、直後に対応する `tool_result` が必要（OpenAI / Anthropic とも）。
/// 中断された過去ターンなどで結果の無い tool_call が履歴に残っていると API が 400 を返すため、
/// 欠けている tool_call にはプレースホルダの tool_result を補完して会話を整合させる。
pub fn reconcile_tool_results(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    use std::collections::HashSet;
    let mut out: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    let mut i = 0;
    while i < messages.len() {
        let calls = match (&messages[i].role, &messages[i].tool_calls) {
            (Role::Assistant, Some(c)) if !c.is_empty() => c.clone(),
            _ => {
                out.push(messages[i].clone());
                i += 1;
                continue;
            }
        };
        out.push(messages[i].clone());
        // 直後の連続する tool メッセージを引き継ぎつつ、見えた tool_call_id を記録
        let mut seen: HashSet<String> = HashSet::new();
        let mut j = i + 1;
        while j < messages.len() && messages[j].role == Role::Tool {
            if let Some(id) = &messages[j].tool_call_id {
                seen.insert(id.clone());
            }
            out.push(messages[j].clone());
            j += 1;
        }
        // 結果の無い tool_call にプレースホルダを補完
        for c in &calls {
            if !seen.contains(&c.call_id) {
                out.push(ChatMessage::tool_result(
                    &c.call_id,
                    "(tool result unavailable: the previous run was interrupted)",
                ));
            }
        }
        i = j;
    }
    out
}

/// agentic ループを実行する。`messages` は事前の会話（直近の user メッセージを含む）。
/// 生成された assistant / tool メッセージは `db::chat` に永続化し、`host` へ通知する。
pub async fn run_chat_loop(
    provider: &dyn ChatProvider,
    ctx: &ToolContext<'_>,
    tools: &[ToolSpec],
    messages: Vec<ChatMessage>,
    params: &ChatLoopParams<'_>,
    host: &mut dyn ChatLoopHost,
) -> Result<(), LlmError> {
    // 過去ターンの未完了 tool_call を補完してから開始する。
    let mut messages = reconcile_tool_results(messages);
    for _turn in 0..params.max_turns {
        if host.is_cancelled() {
            break;
        }

        // --- 1 ターン分の LLM 呼び出し（テキストは on_delta で素通し） ---
        let turn = {
            let mut on_delta = |s: &str| host.on_delta(s);
            provider
                .stream_chat(
                    params.api_key,
                    params.model,
                    params.system,
                    &messages,
                    tools,
                    &mut on_delta,
                )
                .await?
        };

        // --- assistant メッセージを永続化 + 会話に追加 ---
        let tool_calls_json = if turn.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&turn.tool_calls).unwrap_or_default())
        };
        let assistant_row = db::chat::append_message(
            ctx.pool,
            &NewChatMessage {
                session_id: ctx.session_id,
                role: "assistant".to_string(),
                content: turn.text.clone(),
                tool_calls: tool_calls_json,
                tool_call_id: None,
            },
        )
        .await
        .map_err(|e| LlmError::Stream(format!("failed to persist assistant message: {e}")))?;
        host.on_message_persisted(assistant_row.id, Role::Assistant).await;

        messages.push(ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text(turn.text)],
            tool_calls: if turn.tool_calls.is_empty() {
                None
            } else {
                Some(turn.tool_calls.clone())
            },
            tool_call_id: None,
        });

        // ツール呼び出しが無ければターン終了 = ループ完了。
        if turn.tool_calls.is_empty() {
            break;
        }

        // --- 各ツール呼び出しを承認チェック → 実行 → 結果を会話に追加 ---
        for call in &turn.tool_calls {
            let auto = tools::approval::should_auto_approve(&call.tool_name, params.whitelist);
            host.on_tool_proposed(call, !auto).await;

            let approved = if auto {
                true
            } else {
                host.request_approval(call).await
            };

            let result_text = if !approved {
                format!("The user denied the tool call `{}`.", call.tool_name)
            } else {
                match tools::execute_tool(ctx, call).await {
                    Ok(s) => s,
                    Err(e) => format!("Tool `{}` failed: {e}", call.tool_name),
                }
            };
            host.on_tool_executed(&call.call_id, &result_text).await;

            let tool_row = db::chat::append_message(
                ctx.pool,
                &NewChatMessage {
                    session_id: ctx.session_id,
                    role: "tool".to_string(),
                    content: result_text.clone(),
                    tool_calls: None,
                    tool_call_id: Some(call.call_id.clone()),
                },
            )
            .await
            .map_err(|e| LlmError::Stream(format!("failed to persist tool message: {e}")))?;
            host.on_message_persisted(tool_row.id, Role::Tool).await;

            messages.push(ChatMessage::tool_result(&call.call_id, &result_text));
        }
        // ループ継続: 次の呼び出しで LLM はツール結果を見て続きを生成する。
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ChatTurnResult, StopReason};
    use sqlx::SqlitePool;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    // ---- モック ChatProvider: 事前に積んだターンを順に返す ----
    struct MockProvider {
        turns: Mutex<VecDeque<ChatTurnResult>>,
        /// キューが空になったときに返す「常にツールを呼ぶ」ターン（max_turns テスト用）。
        repeat_last: bool,
        calls: Mutex<usize>,
    }

    impl MockProvider {
        fn new(turns: Vec<ChatTurnResult>) -> Self {
            MockProvider {
                turns: Mutex::new(turns.into()),
                repeat_last: false,
                calls: Mutex::new(0),
            }
        }
        fn looping(turn: ChatTurnResult) -> Self {
            MockProvider {
                turns: Mutex::new(VecDeque::from(vec![turn])),
                repeat_last: true,
                calls: Mutex::new(0),
            }
        }
        fn call_count(&self) -> usize {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl ChatProvider for MockProvider {
        async fn stream_chat(
            &self,
            _api_key: &str,
            _model: &str,
            _system: &str,
            _messages: &[ChatMessage],
            _tools: &[ToolSpec],
            on_delta: &mut (dyn for<'a> FnMut(&'a str) + Send),
        ) -> Result<ChatTurnResult, LlmError> {
            *self.calls.lock().unwrap() += 1;
            let turn = {
                let mut q = self.turns.lock().unwrap();
                if self.repeat_last {
                    q.front().cloned().expect("looping mock needs one turn")
                } else {
                    q.pop_front().expect("MockProvider: no more turns queued")
                }
            };
            if !turn.text.is_empty() {
                on_delta(&turn.text);
            }
            Ok(turn)
        }
    }

    // ---- 記録用 ChatLoopHost ----
    #[derive(Default)]
    struct RecordingHost {
        deltas: Vec<String>,
        proposed: Vec<(String, bool)>,
        executed: Vec<(String, String)>,
        persisted: Vec<(i64, Role)>,
        approvals: VecDeque<bool>,
        cancelled: bool,
    }

    #[async_trait::async_trait]
    impl ChatLoopHost for RecordingHost {
        fn on_delta(&mut self, text: &str) {
            self.deltas.push(text.to_string());
        }
        async fn on_tool_proposed(&mut self, call: &ToolCallSpec, needs_approval: bool) {
            self.proposed.push((call.tool_name.clone(), needs_approval));
        }
        async fn request_approval(&mut self, _call: &ToolCallSpec) -> bool {
            self.approvals.pop_front().unwrap_or(false)
        }
        async fn on_tool_executed(&mut self, call_id: &str, result_summary: &str) {
            self.executed
                .push((call_id.to_string(), result_summary.to_string()));
        }
        async fn on_message_persisted(&mut self, message_id: i64, role: Role) {
            self.persisted.push((message_id, role));
        }
        fn is_cancelled(&self) -> bool {
            self.cancelled
        }
    }

    fn text_turn(text: &str) -> ChatTurnResult {
        ChatTurnResult {
            text: text.to_string(),
            tool_calls: vec![],
            stop_reason: StopReason::EndTurn,
        }
    }

    fn tool_turn(call_id: &str, tool: &str, args: serde_json::Value) -> ChatTurnResult {
        ChatTurnResult {
            text: String::new(),
            tool_calls: vec![ToolCallSpec {
                call_id: call_id.to_string(),
                tool_name: tool.to_string(),
                arguments: args,
            }],
            stop_reason: StopReason::ToolUse,
        }
    }

    async fn make_session(pool: &SqlitePool) -> i64 {
        db::chat::create_session(
            pool,
            &db::chat::NewChatSession {
                title: "t".into(),
                provider: "mock".into(),
                model: "mock".into(),
                system_prompt: None,
                scope_mode: "all".into(),
                entry_ids: vec![],
            },
        )
        .await
        .unwrap()
        .id
    }

    fn params() -> ChatLoopParams<'static> {
        ChatLoopParams {
            api_key: "k",
            model: "m",
            system: "s",
            whitelist: None,
            max_turns: DEFAULT_MAX_TURNS,
        }
    }

    async fn run(
        pool: &SqlitePool,
        session_id: i64,
        provider: &MockProvider,
        host: &mut RecordingHost,
        first_user: &str,
    ) {
        let ctx = ToolContext {
            pool,
            session_id,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        };
        let tools = tools::all_tool_specs();
        run_chat_loop(
            provider,
            &ctx,
            &tools,
            vec![ChatMessage::user_text(first_user)],
            &params(),
            host,
        )
        .await
        .unwrap();
    }

    #[test]
    fn reconcile_fills_missing_tool_results() {
        // assistant(tool_use a, b) の直後に a の結果しか無い → b の結果を補完する。
        let assistant = ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text("")],
            tool_calls: Some(vec![
                ToolCallSpec { call_id: "a".into(), tool_name: "x".into(), arguments: serde_json::json!({}) },
                ToolCallSpec { call_id: "b".into(), tool_name: "y".into(), arguments: serde_json::json!({}) },
            ]),
            tool_call_id: None,
        };
        let msgs = vec![
            ChatMessage::user_text("hi"),
            assistant,
            ChatMessage::tool_result("a", "res-a"),
        ];
        let out = reconcile_tool_results(msgs);
        // user, assistant, tool(a), tool(b 補完) = 4
        assert_eq!(out.len(), 4);
        assert_eq!(out[3].role, Role::Tool);
        assert_eq!(out[3].tool_call_id.as_deref(), Some("b"));
    }

    #[test]
    fn reconcile_leaves_complete_conversations_untouched() {
        let assistant = ChatMessage {
            role: Role::Assistant,
            content: vec![ContentBlock::text("ok")],
            tool_calls: Some(vec![ToolCallSpec {
                call_id: "a".into(),
                tool_name: "x".into(),
                arguments: serde_json::json!({}),
            }]),
            tool_call_id: None,
        };
        let msgs = vec![assistant, ChatMessage::tool_result("a", "res")];
        assert_eq!(reconcile_tool_results(msgs).len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn plain_text_turn_streams_and_persists(pool: SqlitePool) {
        let sid = make_session(&pool).await;
        let provider = MockProvider::new(vec![text_turn("Hello there.")]);
        let mut host = RecordingHost::default();
        run(&pool, sid, &provider, &mut host, "hi").await;

        assert_eq!(provider.call_count(), 1);
        assert_eq!(host.deltas, vec!["Hello there.".to_string()]);
        assert!(host.proposed.is_empty());
        // assistant メッセージが 1 件永続化される
        assert_eq!(host.persisted.len(), 1);
        assert_eq!(host.persisted[0].1, Role::Assistant);
        let msgs = db::chat::get_session_with_messages(&pool, sid)
            .await
            .unwrap()
            .messages;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "assistant");
        assert_eq!(msgs[0].content, "Hello there.");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn auto_approved_tool_then_final_answer(pool: SqlitePool) {
        let sid = make_session(&pool).await;
        // list_tags は read 系で自動承認
        let provider = MockProvider::new(vec![
            tool_turn("c1", "list_tags", serde_json::json!({})),
            text_turn("Here are the tags."),
        ]);
        let mut host = RecordingHost::default();
        run(&pool, sid, &provider, &mut host, "list tags").await;

        assert_eq!(provider.call_count(), 2);
        // 提案は needs_approval=false（自動承認）
        assert_eq!(host.proposed, vec![("list_tags".to_string(), false)]);
        assert_eq!(host.executed.len(), 1);
        assert_eq!(host.executed[0].0, "c1");
        // assistant(tool_call) + tool + assistant(text) = 3 件
        let msgs = db::chat::get_session_with_messages(&pool, sid)
            .await
            .unwrap()
            .messages;
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].tool_calls.is_some());
        assert_eq!(msgs[1].role, "tool");
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(msgs[2].role, "assistant");
        assert_eq!(msgs[2].content, "Here are the tags.");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn approval_required_and_rejected_skips_execution(pool: SqlitePool) {
        let sid = make_session(&pool).await;
        // create_entry は要承認
        let provider = MockProvider::new(vec![
            tool_turn(
                "c1",
                "create_entry",
                serde_json::json!({ "title": "Should Not Exist" }),
            ),
            text_turn("ok"),
        ]);
        let mut host = RecordingHost::default();
        host.approvals.push_back(false); // 拒否

        run(&pool, sid, &provider, &mut host, "make an entry").await;

        assert_eq!(host.proposed, vec![("create_entry".to_string(), true)]);
        assert!(host.executed[0].1.contains("denied"));
        // エントリは作られていない
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn approval_required_and_granted_executes(pool: SqlitePool) {
        let sid = make_session(&pool).await;
        let provider = MockProvider::new(vec![
            tool_turn(
                "c1",
                "create_entry",
                serde_json::json!({ "title": "Quantum Walks", "entry_type": "article" }),
            ),
            text_turn("created"),
        ]);
        let mut host = RecordingHost::default();
        host.approvals.push_back(true); // 許可

        run(&pool, sid, &provider, &mut host, "make an entry").await;

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn stops_at_max_turns(pool: SqlitePool) {
        let sid = make_session(&pool).await;
        // 常に list_tags を呼び続けるモック → max_turns で止まるべき
        let provider = MockProvider::looping(tool_turn("c", "list_tags", serde_json::json!({})));
        let mut host = RecordingHost::default();
        let ctx = ToolContext {
            pool: &pool,
            session_id: sid,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        };
        let tools = tools::all_tool_specs();
        let p = ChatLoopParams {
            max_turns: 3,
            ..params()
        };
        run_chat_loop(
            &provider,
            &ctx,
            &tools,
            vec![ChatMessage::user_text("loop")],
            &p,
            &mut host,
        )
        .await
        .unwrap();
        assert_eq!(provider.call_count(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cancelled_before_start_does_nothing(pool: SqlitePool) {
        let sid = make_session(&pool).await;
        let provider = MockProvider::new(vec![text_turn("never")]);
        let mut host = RecordingHost {
            cancelled: true,
            ..Default::default()
        };
        run(&pool, sid, &provider, &mut host, "hi").await;
        assert_eq!(provider.call_count(), 0);
        assert!(host.persisted.is_empty());
    }
}
