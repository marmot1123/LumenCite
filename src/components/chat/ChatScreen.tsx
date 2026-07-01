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
  onOpenSettings: () => void;
}

export function ChatScreen({ onBack, onOpenSettings }: ChatScreenProps) {
  const sessions = useChatStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const loadSessions = useChatStore((s) => s.loadSessions);
  const [rightPanelOpen, setRightPanelOpen] = useState(true);
  const [scopeOpen, setScopeOpen] = useState(false);
  const [newSessionOpen, setNewSessionOpen] = useState(false);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  // ⌘N で新規チャット（SessionList のボタンに表示しているバッジと対応）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setNewSessionOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const activeSession = sessions.find((s) => s.id === activeSessionId) ?? null;

  return (
    <div style={{ width: "100%", height: "100%", background: "var(--bg)", color: "var(--text)", display: "flex", overflow: "hidden" }}>
      <SessionList onNew={() => setNewSessionOpen(true)} onBack={onBack} onOpenSettings={onOpenSettings} />

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

      <ArchiveToast />
    </div>
  );
}

function ArchiveToast() {
  const { t } = useTranslation();
  const toast = useChatStore((s) => s.archiveToast);
  const undoArchive = useChatStore((s) => s.undoArchive);
  const dismiss = useChatStore((s) => s.dismissArchiveToast);

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(dismiss, 6000);
    return () => clearTimeout(timer);
  }, [toast, dismiss]);

  if (!toast) return null;
  return (
    <div style={{ position: "absolute", left: "50%", bottom: 24, transform: "translateX(-50%)", zIndex: 60, display: "flex", alignItems: "center", gap: 14, padding: "9px 12px 9px 14px", borderRadius: 8, background: "var(--surface)", border: "1px solid var(--border-strong)", boxShadow: "0 8px 28px rgba(0,0,0,0.18)", fontSize: 12.5, color: "var(--text)", maxWidth: 460 }}>
      <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {t("chat.archivedToast", { title: toast.title || t("chat.untitled") })}
      </span>
      <button
        onClick={() => void undoArchive()}
        style={{ flexShrink: 0, padding: "4px 12px", borderRadius: 6, border: "none", background: "var(--accent-strong)", color: "#fff", cursor: "pointer", fontSize: 12, fontWeight: 600 }}
      >
        {t("chat.undo")}
      </button>
      <button
        onClick={dismiss}
        title="Dismiss"
        style={{ flexShrink: 0, width: 22, height: 22, padding: 0, border: "none", background: "transparent", color: "var(--text-faint)", cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center" }}
      >
        <ChatIcon name="x" size={12} color="var(--text-faint)" />
      </button>
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
