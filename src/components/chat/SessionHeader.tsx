// 会話ヘッダ: タイトルのインライン編集 + scope チップ(ScopePicker を開く) + provider/model + パネルトグル。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useChatStore } from "../../chat/store";
import { ChatIcon } from "./ChatIcon";
import { ProviderBadge } from "./SessionList";
import type { ChatSession } from "../../types";

interface SessionHeaderProps {
  session: ChatSession;
  scopeOpen: boolean;
  onScopeOpen: () => void;
  rightPanelOpen: boolean;
  onToggleRightPanel: () => void;
}

export function SessionHeader({ session, scopeOpen, onScopeOpen, rightPanelOpen, onToggleRightPanel }: SessionHeaderProps) {
  const { t } = useTranslation();
  const renameSession = useChatStore((s) => s.renameSession);
  const archiveSession = useChatStore((s) => s.archiveSession);
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(session.title);

  useEffect(() => { setTitle(session.title); }, [session.title]);

  const commit = () => {
    setEditing(false);
    const next = title.trim();
    if (next && next !== session.title) void renameSession(session.id, next);
    else setTitle(session.title);
  };

  return (
    <header style={{ flexShrink: 0, borderBottom: "1px solid var(--border)", background: "var(--surface)", padding: "10px 18px 11px 22px", display: "flex", alignItems: "center", gap: 14 }}>
      <div style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 10 }}>
        {editing ? (
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => { if (e.key === "Enter") commit(); if (e.key === "Escape") { setTitle(session.title); setEditing(false); } }}
            autoFocus
            style={{ fontSize: 14.5, fontWeight: 600, color: "var(--text)", letterSpacing: "-0.01em", border: "1px solid var(--accent-strong)", outline: "none", borderRadius: 5, padding: "3px 8px", background: "var(--surface)", flex: 1, minWidth: 0, fontFamily: "inherit" }}
          />
        ) : (
          <h1
            onClick={() => setEditing(true)}
            title={session.title}
            style={{ margin: 0, fontSize: 14.5, fontWeight: 600, color: "var(--text)", letterSpacing: "-0.01em", cursor: "text", padding: "3px 6px", borderRadius: 5, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis", maxWidth: 420 }}
          >{session.title || t("chat.untitled")}</h1>
        )}

        <button
          onClick={onScopeOpen}
          style={{ display: "inline-flex", alignItems: "center", gap: 5, padding: "3px 8px 3px 7px", borderRadius: 999, border: "1px solid var(--border-strong)", background: scopeOpen ? "var(--accent-soft)" : "var(--surface-2)", color: scopeOpen ? "var(--accent-strong)" : "var(--text)", fontSize: 11.5, fontWeight: 500, cursor: "pointer", flexShrink: 0 }}
        >
          <span style={{ fontSize: 9.5, fontFamily: "var(--mono)", color: scopeOpen ? "var(--accent-strong)" : "var(--text-faint)", letterSpacing: "0.06em", textTransform: "uppercase" }}>{t("chat.scopeButton")}:</span>
          {session.scope_mode === "all" ? <span>{t("chat.scopeModeAll")}</span> : <span>{t("chat.scopePapers", { count: session.entry_count })}</span>}
          <ChatIcon name="chevronDown" size={9} color={scopeOpen ? "var(--accent-strong)" : "var(--text-mute)"} />
        </button>

        <span style={{ display: "inline-flex", alignItems: "center", padding: "2px 7px", borderRadius: 4, background: "var(--surface-2)", border: "1px solid var(--border)", flexShrink: 0 }}>
          <ProviderBadge provider={session.provider} model={session.model} />
        </span>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        <HeaderBtn icon="archive" title={t("chat.archive")} onClick={() => void archiveSession(session.id)} />
        <HeaderBtn icon="panel" title={t("chat.contextTitle")} active={rightPanelOpen} onClick={onToggleRightPanel} />
      </div>
    </header>
  );
}

function HeaderBtn({ icon, title, active, onClick }: { icon: "archive" | "panel"; title: string; active?: boolean; onClick: () => void }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={onClick} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)} title={title}
      style={{ width: 26, height: 26, padding: 0, border: "1px solid " + (active ? "var(--border-strong)" : "transparent"), background: active ? "var(--surface-2)" : hover ? "var(--hover)" : "transparent", borderRadius: 5, cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center" }}
    >
      <ChatIcon name={icon} size={12} color={active ? "var(--text)" : "var(--text-mute)"} />
    </button>
  );
}
