// ChatScreen — 3 ペインレイアウト（左 SessionList / 中央 会話 / 右 ContextPanel）。
// 会話ペインの MessageList / Composer / SessionHeader は #16/#17 で作り込み済みコンポーネントに、
// New chat は #17 の NewSessionDialog に置き換える。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useChatStore } from "../../chat/store";
import { SessionList } from "./SessionList";
import { SessionHeader } from "./SessionHeader";
import { MessageList } from "./MessageList";
import { Composer } from "./Composer";
import { ContextPanel } from "./ContextPanel";
import { ChatIcon } from "./ChatIcon";
import type { LlmSettings } from "../../types";

interface ChatScreenProps {
  onBack: () => void;
}

export function ChatScreen({ onBack }: ChatScreenProps) {
  const { t } = useTranslation();
  const sessions = useChatStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const loadSessions = useChatStore((s) => s.loadSessions);
  const createSession = useChatStore((s) => s.createSession);
  const [rightPanelOpen, setRightPanelOpen] = useState(true);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  // #17 で NewSessionDialog に置き換える暫定の即時作成。
  const handleNewChat = async () => {
    const settings = await invoke<LlmSettings>("get_llm_settings");
    await createSession({
      title: t("chat.newChat"),
      provider: settings.provider,
      model: settings.model,
      scopeMode: "all",
      entryIds: [],
    });
  };

  const activeSession = sessions.find((s) => s.id === activeSessionId) ?? null;

  return (
    <div style={{ width: "100%", height: "100%", background: "var(--bg)", color: "var(--text)", display: "flex", overflow: "hidden" }}>
      <SessionList onNew={handleNewChat} onBack={onBack} />

      <main style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, position: "relative" }}>
        {activeSession ? (
          <>
            <SessionHeader
              session={activeSession}
              rightPanelOpen={rightPanelOpen}
              onToggleRightPanel={() => setRightPanelOpen((o) => !o)}
            />
            <MessageList />
            <Composer />
          </>
        ) : (
          <EmptyConversation onNew={handleNewChat} />
        )}
      </main>

      {activeSession && rightPanelOpen && <ContextPanel />}
    </div>
  );
}

function EmptyConversation({ onNew }: { onNew: () => void }) {
  const { t } = useTranslation();
  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 18, padding: 40, background: "var(--surface)" }}>
      <div style={{ width: 56, height: 56, borderRadius: 14, display: "inline-flex", alignItems: "center", justifyContent: "center", background: "linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))", boxShadow: "0 8px 24px oklch(0.5 0.15 60 / 0.25)" }}>
        <ChatIcon name="sparkle" size={28} color="white" strokeWidth={1.6} />
      </div>
      <div style={{ textAlign: "center" }}>
        <div style={{ fontSize: 17, fontWeight: 600, color: "var(--text)", letterSpacing: "-0.01em" }}>{t("chat.emptyTitle")}</div>
        <div style={{ fontSize: 12.5, color: "var(--text-mute)", marginTop: 6, lineHeight: 1.6, maxWidth: 460 }}>{t("chat.emptyBody")}</div>
      </div>
      <button onClick={onNew} style={{ display: "inline-flex", alignItems: "center", gap: 8, padding: "8px 16px", borderRadius: 8, border: "none", background: "var(--accent-strong)", color: "#fff", cursor: "pointer", fontSize: 13, fontWeight: 600 }}>
        <ChatIcon name="plus" size={13} color="#fff" strokeWidth={2} />
        {t("chat.newChat")}
      </button>
    </div>
  );
}
