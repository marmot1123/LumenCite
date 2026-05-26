// ChatScreen — v0.2.0 chat 画面の最小機能シェル。
// 基盤（zustand ストア + ストリーム配線 + ルーティング）を実機確認するためのもので、
// SessionList / MessageList / ToolCallCard / Composer の作り込み（デザイン handoff 準拠）は
// #15〜#17 でこの内部を置き換える。
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useChatStore } from "../../chat/store";
import type { LlmSettings, UiToolCall } from "../../types";

interface ChatScreenProps {
  onBack: () => void;
}

export function ChatScreen({ onBack }: ChatScreenProps) {
  const sessions = useChatStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const messages = useChatStore((s) => s.messages);
  const streaming = useChatStore((s) => s.streaming);
  const blocking = useChatStore((s) => s.blocking);
  const error = useChatStore((s) => s.error);
  const loadSessions = useChatStore((s) => s.loadSessions);
  const openSession = useChatStore((s) => s.openSession);
  const createSession = useChatStore((s) => s.createSession);
  const sendMessage = useChatStore((s) => s.sendMessage);
  const approveToolCall = useChatStore((s) => s.approveToolCall);
  const cancelStream = useChatStore((s) => s.cancelStream);

  const [draft, setDraft] = useState("");

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  const handleNewChat = async () => {
    const settings = await invoke<LlmSettings>("get_llm_settings");
    await createSession({
      title: "New chat",
      provider: settings.provider,
      model: settings.model,
      scopeMode: "all",
      entryIds: [],
    });
  };

  const handleSend = () => {
    const text = draft.trim();
    if (!text || streaming) return;
    setDraft("");
    void sendMessage(text);
  };

  return (
    <div style={{ width: "100%", height: "100%", display: "flex", background: "var(--bg)", color: "var(--text)" }}>
      {/* 左: セッション一覧（#15 で SessionList に差し替え） */}
      <aside style={{ width: 244, flexShrink: 0, borderRight: "1px solid var(--border)", background: "var(--sidebar)", display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "12px 14px", display: "flex", gap: 8, alignItems: "center" }}>
          <button onClick={onBack} title="Library" style={iconBtn}>←</button>
          <strong style={{ fontSize: 13 }}>LumenCite Chat</strong>
        </div>
        <div style={{ padding: "0 12px 8px" }}>
          <button onClick={handleNewChat} style={{ ...rowBtn, width: "100%", justifyContent: "center", border: "1px solid var(--border-strong)" }}>+ New chat</button>
        </div>
        <div style={{ flex: 1, overflowY: "auto" }}>
          {sessions.map((s) => (
            <button
              key={s.id}
              onClick={() => void openSession(s.id)}
              style={{ ...rowBtn, width: "100%", textAlign: "left", background: s.id === activeSessionId ? "var(--row-selected)" : "transparent" }}
            >
              <span style={{ display: "block", fontSize: 12.5, fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{s.title || "Untitled"}</span>
              <span style={{ display: "block", fontSize: 10, color: "var(--text-faint)", fontFamily: "var(--mono)" }}>
                {s.scope_mode === "all" ? "all" : `${s.entry_count} papers`}
              </span>
            </button>
          ))}
        </div>
      </aside>

      {/* 右: 会話（#16/#17 で MessageList + Composer に差し替え） */}
      <main style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        {activeSessionId == null ? (
          <div style={{ margin: "auto", color: "var(--text-faint)" }}>Start a chat about your library.</div>
        ) : (
          <>
            <div style={{ flex: 1, overflowY: "auto", padding: "20px 40px", maxWidth: 820, width: "100%", margin: "0 auto" }}>
              {messages.map((m, i) => (
                <div key={m.id ?? `tmp-${i}`} style={{ marginBottom: 14 }}>
                  <div style={{ fontSize: 10, color: "var(--text-faint)", fontFamily: "var(--mono)", marginBottom: 2 }}>{m.role}</div>
                  {m.content && <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.6 }}>{m.content}{m.streaming && <span className="lc-chat-caret" />}</div>}
                  {m.tool_calls.map((tc) => (
                    <ToolCallRow key={tc.call_id} tc={tc} onApprove={approveToolCall} />
                  ))}
                </div>
              ))}
              {error && <div style={{ color: "var(--danger-text)", fontSize: 12 }}>{error}</div>}
            </div>

            <div style={{ borderTop: "1px solid var(--border-subtle)", padding: "10px 40px 16px", maxWidth: 820, width: "100%", margin: "0 auto" }}>
              <textarea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); handleSend(); }
                }}
                disabled={blocking}
                placeholder={blocking ? "Approve or deny the tool call to continue…" : "Message LumenCite…  (⌘↩ to send)"}
                rows={2}
                style={{ width: "100%", resize: "none", padding: 11, borderRadius: 10, border: "1px solid var(--border-strong)", background: "var(--surface)", color: "var(--text)", opacity: blocking ? 0.65 : 1 }}
              />
              <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 6 }}>
                {streaming ? (
                  <button onClick={() => void cancelStream()} style={{ ...rowBtn, border: "1px solid var(--border-strong)" }}>■ Stop</button>
                ) : (
                  <button onClick={handleSend} disabled={blocking || !draft.trim()} style={{ ...rowBtn, background: "var(--accent-strong)", color: "#fff" }}>Send ↩</button>
                )}
              </div>
            </div>
          </>
        )}
      </main>
    </div>
  );
}

function ToolCallRow({ tc, onApprove }: { tc: UiToolCall; onApprove: (id: string, ok: boolean) => void }) {
  return (
    <div style={{ margin: "6px 0", padding: "8px 10px", borderRadius: 7, border: "1px solid var(--border)", background: "var(--surface-2)", fontSize: 12 }}>
      <div style={{ fontFamily: "var(--mono)", fontSize: 11.5 }}>
        🔧 {tc.tool_name}({tc.args_preview}) <span style={{ color: "var(--text-faint)" }}>· {tc.state}</span>
      </div>
      {tc.result_summary && <div style={{ color: "var(--text-mute)", marginTop: 4, whiteSpace: "pre-wrap" }}>{tc.result_summary}</div>}
      {tc.state === "needs_approval" && (
        <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
          <button onClick={() => onApprove(tc.call_id, false)} style={{ ...rowBtn, border: "1px solid var(--border-strong)" }}>Deny</button>
          <button onClick={() => onApprove(tc.call_id, true)} style={{ ...rowBtn, background: "var(--accent-strong)", color: "#fff" }}>Allow</button>
        </div>
      )}
    </div>
  );
}

const iconBtn: React.CSSProperties = {
  width: 24, height: 24, borderRadius: 6, border: "1px solid var(--border)",
  background: "var(--surface)", color: "var(--text)", cursor: "pointer",
};
const rowBtn: React.CSSProperties = {
  display: "flex", flexDirection: "column", gap: 2, padding: "8px 12px",
  borderRadius: 6, border: "none", background: "transparent", color: "var(--text)",
  cursor: "pointer", fontSize: 12,
};
