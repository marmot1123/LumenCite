// 中央: メッセージ列。末尾追従スクロール + user/assistant 吹き出し（ToolCallCard 込み）。
import { useEffect, useRef } from "react";
import { useChatStore } from "../../chat/store";
import { UserMessage, AssistantMessage } from "./MessageBubble";

export function MessageList() {
  const messages = useChatStore((s) => s.messages);
  const error = useChatStore((s) => s.error);
  const approveToolCall = useChatStore((s) => s.approveToolCall);
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);

  // 末尾付近にいるときだけ自動追従（handoff: 末尾から 80px 以内）。
  useEffect(() => {
    const el = scrollRef.current;
    if (el && atBottomRef.current) el.scrollTop = el.scrollHeight;
  }, [messages]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    atBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 80;
  };

  return (
    <div ref={scrollRef} onScroll={onScroll} style={{ flex: 1, overflowY: "auto", background: "var(--surface)" }}>
      <div style={{ maxWidth: 820, margin: "0 auto", padding: "20px 40px 24px" }}>
        {messages.map((m, i) =>
          m.role === "user" ? (
            <UserMessage key={m.id ?? `tmp-${i}`} content={m.content} />
          ) : (
            <AssistantMessage key={m.id ?? `tmp-${i}`} message={m} onApprove={approveToolCall} />
          ),
        )}
        {error && (
          <div style={{ marginTop: 8, padding: "8px 12px", borderRadius: 7, fontSize: 12, background: "var(--danger-bg)", border: "1px solid var(--danger-border)", color: "var(--danger-text)" }}>{error}</div>
        )}
      </div>
    </div>
  );
}
