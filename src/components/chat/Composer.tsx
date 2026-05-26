// 入力欄（v1）。#17 でツールバー・添付・scope ラベル等の作り込みを行う。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useChatStore } from "../../chat/store";
import { ChatIcon } from "./ChatIcon";

export function Composer() {
  const { t } = useTranslation();
  const streaming = useChatStore((s) => s.streaming);
  const blocking = useChatStore((s) => s.blocking);
  const sendMessage = useChatStore((s) => s.sendMessage);
  const cancelStream = useChatStore((s) => s.cancelStream);
  const [draft, setDraft] = useState("");

  const send = () => {
    const text = draft.trim();
    if (!text || streaming || blocking) return;
    setDraft("");
    void sendMessage(text);
  };

  return (
    <div style={{ flexShrink: 0, background: "var(--surface)", borderTop: "1px solid var(--border-subtle)", padding: "10px 40px 16px" }}>
      <div style={{ maxWidth: 820, margin: "0 auto" }}>
        {blocking && (
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8, padding: "7px 11px", borderRadius: 7, fontSize: 11.5, background: "var(--tc-approve-bg)", border: "1px solid var(--tc-approve-bd)", color: "var(--tc-approve-fg)" }}>
            <ChatIcon name="warn" size={12} color="var(--tc-approve-fg)" />
            {t("chat.composerBlocked")}
          </div>
        )}
        <div style={{ border: "1px solid var(--border-strong)", borderRadius: 10, background: "var(--surface)", boxShadow: "0 1px 0 rgba(0,0,0,0.03)", opacity: blocking ? 0.65 : 1 }}>
          <textarea
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); send(); } }}
            disabled={blocking}
            placeholder={blocking ? t("chat.composerBlocked") : t("chat.composerPlaceholder")}
            rows={2}
            style={{ width: "100%", resize: "none", minHeight: 56, padding: "11px 14px 4px", border: "none", outline: "none", background: "transparent", color: "var(--text)", fontSize: 13.5, lineHeight: 1.55 }}
          />
          <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 8px 8px 12px" }}>
            <span style={{ flex: 1 }} />
            {streaming ? (
              <button onClick={() => void cancelStream()} style={{ display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 12px", borderRadius: 6, border: "1px solid var(--border-strong)", background: "var(--surface)", color: "var(--text)", cursor: "pointer", fontSize: 12, fontWeight: 600 }}>
                <ChatIcon name="stop" size={12} color="var(--text)" />
                {t("chat.stop")}
              </button>
            ) : (
              <button onClick={send} disabled={blocking || !draft.trim()} style={{ display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 12px 5px 11px", borderRadius: 6, border: "none", background: "var(--accent-strong)", color: "#fff", cursor: blocking || !draft.trim() ? "default" : "pointer", opacity: blocking || !draft.trim() ? 0.5 : 1, fontSize: 12, fontWeight: 600 }}>
                {t("chat.send")}
                <ChatIcon name="enter" size={12} color="#fff" />
              </button>
            )}
          </div>
        </div>
        <div style={{ marginTop: 6, fontSize: 10.5, color: "var(--text-faint)" }}>{t("chat.composerHint")}</div>
      </div>
    </div>
  );
}
