// 中央: メッセージ列（v1）。#16 で MessageBubble / AssistantMessage / ToolCallCard に作り込む。
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { MathMarkdown } from "../MathMarkdown";
import { useChatStore } from "../../chat/store";
import type { UiChatMessage, UiToolCall } from "../../types";

export function MessageList() {
  const { t } = useTranslation();
  const messages = useChatStore((s) => s.messages);
  const error = useChatStore((s) => s.error);
  const approveToolCall = useChatStore((s) => s.approveToolCall);
  const scrollRef = useRef<HTMLDivElement>(null);
  const atBottomRef = useRef(true);

  // 末尾追従（ユーザーが末尾付近にいるときだけ）
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
        {messages.map((m, i) => (
          <MessageItem key={m.id ?? `tmp-${i}`} message={m} onApprove={approveToolCall} thinkingLabel={t("chat.thinking")} rejectedLabel={t("chat.toolRejected")} approveLabel={t("chat.approve")} rejectLabel={t("chat.reject")} />
        ))}
        {error && (
          <div style={{ marginTop: 8, padding: "8px 12px", borderRadius: 7, fontSize: 12, background: "var(--danger-bg)", border: "1px solid var(--danger-border)", color: "var(--danger-text)" }}>{error}</div>
        )}
      </div>
    </div>
  );
}

interface MessageItemProps {
  message: UiChatMessage;
  onApprove: (callId: string, approved: boolean) => void;
  thinkingLabel: string;
  rejectedLabel: string;
  approveLabel: string;
  rejectLabel: string;
}

function MessageItem({ message, onApprove, thinkingLabel, rejectedLabel, approveLabel, rejectLabel }: MessageItemProps) {
  if (message.role === "user") {
    return (
      <div style={{ display: "flex", justifyContent: "flex-end", margin: "12px 0" }}>
        <div style={{ maxWidth: "76%", padding: "10px 14px", borderRadius: "12px 4px 12px 12px", background: "color-mix(in oklch, var(--accent-strong) 9%, var(--surface))", border: "1px solid color-mix(in oklch, var(--accent-strong) 22%, transparent)", color: "var(--text)", whiteSpace: "pre-wrap", fontSize: 13.5, lineHeight: 1.6 }}>
          {message.content}
        </div>
      </div>
    );
  }
  // assistant
  return (
    <div style={{ display: "flex", gap: 10, margin: "14px 0", alignItems: "flex-start" }}>
      <div style={{ width: 26, height: 26, borderRadius: "50%", flexShrink: 0, marginTop: 1, background: "linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))" }} />
      <div style={{ flex: 1, minWidth: 0, fontSize: 13.5, lineHeight: 1.66 }}>
        {message.content && (
          <div className="lc-markdown">
            <MathMarkdown value={message.content} />
            {message.streaming && <span className="lc-chat-caret" />}
          </div>
        )}
        {!message.content && message.streaming && (
          <div style={{ color: "var(--text-faint)", fontSize: 12 }}>{thinkingLabel}</div>
        )}
        {message.tool_calls.length > 0 && (
          <div style={{ display: "flex", flexDirection: "column", gap: 6, margin: "10px 0 4px" }}>
            {message.tool_calls.map((tc) => (
              <ToolCallRow key={tc.call_id} tc={tc} onApprove={onApprove} rejectedLabel={rejectedLabel} approveLabel={approveLabel} rejectLabel={rejectLabel} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// v1 のツールカード。#16 で 5 系統 (read/write/approve/delete/mcp) × 状態の本実装に置き換える。
function ToolCallRow({ tc, onApprove, rejectedLabel, approveLabel, rejectLabel }: { tc: UiToolCall; onApprove: (id: string, ok: boolean) => void; rejectedLabel: string; approveLabel: string; rejectLabel: string }) {
  const needsApproval = tc.state === "needs_approval";
  return (
    <div style={{ padding: "8px 10px", borderRadius: 7, fontSize: 11.5, border: "1px solid " + (needsApproval ? "var(--tc-approve-bd)" : "var(--border)"), background: needsApproval ? "var(--tc-approve-bg)" : "var(--surface-2)", animation: needsApproval ? "pulseApprove 2.4s ease-in-out infinite" : undefined }}>
      <div style={{ fontFamily: "var(--mono)", display: "flex", gap: 6, alignItems: "center" }}>
        <span style={{ fontWeight: 600 }}>{tc.tool_name}</span>
        <span style={{ color: "var(--text-faint)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>({tc.args_preview})</span>
        <span style={{ flex: 1 }} />
        <span style={{ color: "var(--text-faint)", fontSize: 10 }}>{tc.state === "rejected" ? rejectedLabel : tc.state}</span>
      </div>
      {tc.result_summary && (
        <div style={{ color: "var(--text-mute)", marginTop: 4, whiteSpace: "pre-wrap", fontFamily: "inherit" }}>{tc.result_summary}</div>
      )}
      {needsApproval && (
        <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
          <button onClick={() => onApprove(tc.call_id, false)} style={{ padding: "4px 12px", borderRadius: 5, border: "1px solid var(--border-strong)", background: "var(--surface)", color: "var(--text)", cursor: "pointer", fontSize: 12 }}>{rejectLabel}</button>
          <button onClick={() => onApprove(tc.call_id, true)} style={{ padding: "4px 12px", borderRadius: 5, border: "none", background: "var(--tc-approve-fg)", color: "#fff", cursor: "pointer", fontSize: 12, fontWeight: 600 }}>{approveLabel}</button>
        </div>
      )}
    </div>
  );
}
