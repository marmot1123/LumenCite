// 会話ヘッダ（v1）。#17 でタイトルのインライン編集 + ScopePicker ポップオーバーに作り込む。
import { useTranslation } from "react-i18next";
import { useChatStore } from "../../chat/store";
import { ChatIcon } from "./ChatIcon";
import { ProviderBadge, ScopeChip } from "./SessionList";
import type { ChatSession } from "../../types";

interface SessionHeaderProps {
  session: ChatSession;
  rightPanelOpen: boolean;
  onToggleRightPanel: () => void;
}

export function SessionHeader({ session, rightPanelOpen, onToggleRightPanel }: SessionHeaderProps) {
  const { t } = useTranslation();
  const archiveSession = useChatStore((s) => s.archiveSession);

  return (
    <div style={{ height: 50, flexShrink: 0, background: "var(--surface)", borderBottom: "1px solid var(--border)", padding: "0 18px 0 22px", display: "flex", alignItems: "center", gap: 14 }}>
      <div style={{ fontSize: 14.5, fontWeight: 600, letterSpacing: "-0.01em", color: "var(--text)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", maxWidth: 360 }}>
        {session.title || t("chat.untitled")}
      </div>
      <ScopeChip session={session} />
      <span style={{ display: "inline-flex", alignItems: "center", gap: 5, padding: "2px 7px", borderRadius: 999, background: "var(--surface-2)", border: "1px solid var(--border)" }}>
        <ProviderBadge provider={session.provider} model={session.model} />
      </span>
      <span style={{ flex: 1 }} />
      <HeaderButton icon="archive" title={t("chat.archive")} onClick={() => void archiveSession(session.id)} />
      <HeaderButton icon="panel" title="Toggle context panel" active={rightPanelOpen} onClick={onToggleRightPanel} />
    </div>
  );
}

function HeaderButton({ icon, title, onClick, active }: { icon: "archive" | "panel"; title: string; onClick: () => void; active?: boolean }) {
  return (
    <button
      onClick={onClick}
      title={title}
      style={{ width: 26, height: 26, padding: 0, border: "1px solid var(--border)", borderRadius: 6, cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center", background: active ? "var(--accent-soft)" : "var(--surface)", color: active ? "var(--accent-strong)" : "var(--text-mute)" }}
    >
      <ChatIcon name={icon} size={13} color={active ? "var(--accent-strong)" : "var(--text-mute)"} />
    </button>
  );
}
