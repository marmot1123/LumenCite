// メッセージ吹き出し: user バブルと assistant（アバター + Markdown + ツールカード）。
import { useTranslation } from "react-i18next";
import { MathMarkdown } from "../MathMarkdown";
import { ToolCallCard } from "./ToolCallCard";
import type { UiChatMessage } from "../../types";

export function UserMessage({ content }: { content: string }) {
  return (
    <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 18 }}>
      <div style={{ maxWidth: "76%", padding: "10px 14px", borderRadius: 12, borderTopRightRadius: 4, background: "color-mix(in oklch, var(--accent-strong) 9%, var(--surface))", border: "1px solid color-mix(in oklch, var(--accent-strong) 22%, transparent)", color: "var(--text)", fontSize: 13.5, lineHeight: 1.6, boxShadow: "0 1px 0 oklch(0 0 0 / 0.02)", whiteSpace: "pre-wrap" }}>
        {content}
      </div>
    </div>
  );
}

interface AssistantMessageProps {
  message: UiChatMessage;
  onApprove: (callId: string, approved: boolean) => void;
}

export function AssistantMessage({ message, onApprove }: AssistantMessageProps) {
  const { t } = useTranslation();
  return (
    <div style={{ display: "flex", gap: 12, marginBottom: 22, alignItems: "flex-start" }}>
      <div style={{ flexShrink: 0, width: 26, height: 26, borderRadius: 7, marginTop: 1, display: "flex", alignItems: "center", justifyContent: "center", background: "linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))", boxShadow: "0 1px 2px rgba(120,80,20,0.20), inset 0 0.5px 0 rgba(255,255,255,0.5)" }}>
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M3.5 2.5v6.5a3.5 3.5 0 0 0 7 0V2.5" stroke="white" strokeWidth="1.5" strokeLinecap="round" />
          <circle cx="7" cy="11" r="1.1" fill="white" />
        </svg>
      </div>
      <div style={{ flex: 1, minWidth: 0, paddingTop: 1 }}>
        {message.content && (
          <div className="lc-markdown" style={{ fontSize: 13.5, lineHeight: 1.66 }}>
            <MathMarkdown value={message.content} />
            {message.streaming && <span className="lc-chat-caret" />}
          </div>
        )}
        {!message.content && message.streaming && (
          <div style={{ color: "var(--text-faint)", fontSize: 12 }}>{t("chat.thinking")}</div>
        )}
        {message.tool_calls.length > 0 && (
          <div style={{ display: "flex", flexDirection: "column", gap: 6, margin: "10px 0 4px" }}>
            {message.tool_calls.map((tc) => (
              <ToolCallCard key={tc.call_id} tc={tc} onApprove={onApprove} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
