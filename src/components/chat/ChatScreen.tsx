// ChatScreen — 3 ペインレイアウト（左 SessionList / 中央 会話 / 右 ContextPanel）+ オーバーレイ。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useChatStore } from "../../chat/store";
import { SessionList } from "./SessionList";
import { SessionHeader } from "./SessionHeader";
import { MessageList } from "./MessageList";
import { Composer } from "./Composer";
import { ContextPanel } from "./ContextPanel";
import { ScopePicker } from "./ScopePicker";
import { NewSessionDialog } from "./NewSessionDialog";
import { ChatIcon } from "./ChatIcon";

interface ChatScreenProps {
  onBack: () => void;
}

export function ChatScreen({ onBack }: ChatScreenProps) {
  const sessions = useChatStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const loadSessions = useChatStore((s) => s.loadSessions);
  const [rightPanelOpen, setRightPanelOpen] = useState(true);
  const [scopeOpen, setScopeOpen] = useState(false);
  const [newSessionOpen, setNewSessionOpen] = useState(false);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  const activeSession = sessions.find((s) => s.id === activeSessionId) ?? null;

  return (
    <div style={{ width: "100%", height: "100%", background: "var(--bg)", color: "var(--text)", display: "flex", overflow: "hidden" }}>
      <SessionList onNew={() => setNewSessionOpen(true)} onBack={onBack} />

      <main style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, position: "relative" }}>
        {activeSession ? (
          <>
            <SessionHeader
              session={activeSession}
              scopeOpen={scopeOpen}
              onScopeOpen={() => setScopeOpen((o) => !o)}
              rightPanelOpen={rightPanelOpen}
              onToggleRightPanel={() => setRightPanelOpen((o) => !o)}
            />
            <MessageList />
            <Composer />
            {scopeOpen && <ScopePicker session={activeSession} onClose={() => setScopeOpen(false)} />}
          </>
        ) : (
          <EmptyConversation onNew={() => setNewSessionOpen(true)} />
        )}

        {newSessionOpen && <NewSessionDialog onClose={() => setNewSessionOpen(false)} />}
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
