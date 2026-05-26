// 左ペイン: セッション一覧。design handoff の ChatSidebar を踏襲し、store に配線。
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useChatStore } from "../../chat/store";
import { useTheme } from "../../hooks/useTheme";
import { relativeTime, sessionGroup, type SessionGroup } from "../../chat/format";
import { ChatIcon } from "./ChatIcon";
import { Icon } from "../icons";
import type { ChatSession, Density } from "../../types";

export function ProviderBadge({ provider, model }: { provider: string; model: string }) {
  const label = provider === "anthropic" ? "Claude" : provider === "openai" ? "GPT" : provider;
  const short = model.replace(/^claude-/, "").replace(/^gpt-/, "") || model;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 3, fontSize: 10, color: "var(--text-faint)", fontFamily: "var(--mono)" }}>
      <span style={{ width: 5, height: 5, borderRadius: "50%", flexShrink: 0, background: provider === "anthropic" ? "oklch(0.62 0.14 35)" : "oklch(0.55 0.13 165)" }} />
      <span>{label}·{short}</span>
    </span>
  );
}

export function ScopeChip({ session, dense }: { session: ChatSession; dense?: boolean }) {
  const { t } = useTranslation();
  const isAll = session.scope_mode === "all";
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 3, fontSize: 10,
      padding: dense ? "0 5px" : "1px 6px", borderRadius: 3, fontWeight: 500,
      fontVariantNumeric: "tabular-nums", flexShrink: 0,
      background: isAll ? "var(--surface-2)" : "color-mix(in oklch, var(--accent-strong) 8%, transparent)",
      color: isAll ? "var(--text-mute)" : "var(--accent-strong)",
      border: "1px solid " + (isAll ? "var(--border)" : "color-mix(in oklch, var(--accent-strong) 25%, transparent)"),
    }}>
      {isAll ? t("chat.scopeAll") : t("chat.scopePapers", { count: session.entry_count })}
    </span>
  );
}

interface SessionListProps {
  onNew: () => void;
  onBack: () => void;
  onOpenSettings: () => void;
}

export function SessionList({ onNew, onBack, onOpenSettings }: SessionListProps) {
  const { t } = useTranslation();
  const { density } = useTheme();
  const sessions = useChatStore((s) => s.sessions);
  const activeSessionId = useChatStore((s) => s.activeSessionId);
  const openSession = useChatStore((s) => s.openSession);
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? sessions.filter((s) => s.title.toLowerCase().includes(q)) : sessions;
  }, [sessions, query]);

  const groups = useMemo(() => {
    const now = new Date();
    const g: Record<SessionGroup, ChatSession[]> = { today: [], yesterday: [], earlier: [] };
    filtered.forEach((s) => g[sessionGroup(s.updated_at, now)].push(s));
    return g;
  }, [filtered]);

  return (
    <aside style={{ width: 244, flexShrink: 0, height: "100%", borderRight: "1px solid var(--border)", background: "var(--sidebar)", display: "flex", flexDirection: "column", overflow: "hidden" }}>
      {/* Header */}
      <div style={{ padding: "12px 14px 10px", display: "flex", alignItems: "center", gap: 8 }}>
        <button onClick={onBack} title={t("chat.backToLibrary")} style={squareBtn}>
          <ChatIcon name="arrowLeft" size={12} color="var(--text-mute)" />
        </button>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ fontSize: 12, fontWeight: 600, color: "var(--text)", letterSpacing: "-0.005em", display: "flex", alignItems: "center", gap: 6 }}>
            <ChatIcon name="sparkle" size={11} color="var(--accent-strong)" />
            {t("chat.brand")}
          </div>
          <div style={{ fontSize: 10, color: "var(--text-faint)", marginTop: 1, fontFamily: "var(--mono)" }}>
            {t("chat.sessionsCount", { count: sessions.length })}
          </div>
        </div>
      </div>

      <div style={{ padding: "0 12px 10px" }}>
        <NewChatButton onClick={onNew} />
      </div>

      <div style={{ padding: "0 12px 10px" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "5px 9px", background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 6, height: 26 }}>
          <ChatIcon name="search" size={11} color="var(--text-faint)" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("chat.searchPlaceholder")}
            style={{ flex: 1, border: "none", outline: "none", background: "transparent", fontSize: 12, color: "var(--text)", minWidth: 0 }}
          />
        </div>
      </div>

      {/* Sessions */}
      <div style={{ flex: 1, overflow: "auto", paddingBottom: 16 }}>
        {sessions.length === 0 ? (
          <EmptyState />
        ) : (
          (["today", "yesterday", "earlier"] as const).map((key) =>
            groups[key].length > 0 ? (
              <div key={key}>
                <GroupHeader label={groupLabels(t)[key]} count={groups[key].length} />
                {groups[key].map((s) => (
                  <SessionRow
                    key={s.id}
                    session={s}
                    active={s.id === activeSessionId}
                    density={density}
                    onClick={() => void openSession(s.id)}
                  />
                ))}
              </div>
            ) : null,
          )
        )}
      </div>

      {/* Footer */}
      <div style={{ padding: "8px 10px 10px 14px", borderTop: "1px solid var(--border)", fontSize: 10.5, color: "var(--text-faint)", display: "flex", alignItems: "center", gap: 7 }}>
        <span style={{ width: 5, height: 5, borderRadius: "50%", background: "oklch(0.68 0.13 150)", boxShadow: "0 0 0 3px oklch(0.68 0.13 150 / 0.18)" }} />
        <span>{t("chat.storedLocally")}</span>
        <span style={{ flex: 1 }} />
        <span style={{ fontFamily: "var(--mono)" }}>chat.db</span>
        <button
          onClick={onOpenSettings}
          title={`${t("settings.title")} (⌘,)`}
          style={{ width: 24, height: 24, padding: 0, marginLeft: 2, border: "1px solid transparent", background: "transparent", borderRadius: 5, cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center", color: "var(--text-mute)" }}
          onMouseEnter={(e) => { e.currentTarget.style.background = "var(--hover)"; e.currentTarget.style.borderColor = "var(--border)"; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.borderColor = "transparent"; }}
        >
          <Icon name="settings" size={14} color="var(--text-mute)" />
        </button>
      </div>
    </aside>
  );
}

function SessionRow({ session, active, density, onClick }: { session: ChatSession; active: boolean; density: Density; onClick: () => void }) {
  const { t } = useTranslation();
  const archiveSession = useChatStore((s) => s.archiveSession);
  const renameSession = useChatStore((s) => s.renameSession);
  const [hover, setHover] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const padV = density === "compact" ? 7 : density === "comfortable" ? 12 : 9;

  const handleRename = () => {
    setMenuOpen(false);
    const next = window.prompt(t("chat.rename"), session.title);
    if (next && next.trim() && next !== session.title) void renameSession(session.id, next.trim());
  };

  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => { setHover(false); setMenuOpen(false); }}
      style={{
        position: "relative", padding: `${padV}px 12px`, margin: "0 6px", borderRadius: 6, cursor: "pointer",
        background: active ? "color-mix(in oklch, var(--accent-strong) 8%, var(--sidebar))" : hover ? "var(--hover)" : "transparent",
        outline: active ? "1px solid color-mix(in oklch, var(--accent-strong) 30%, transparent)" : "1px solid transparent",
        outlineOffset: -1, transition: "background 80ms ease", display: "flex", flexDirection: "column", gap: 4,
      }}
    >
      {active && <span style={{ position: "absolute", left: 0, top: 8, bottom: 8, width: 2, borderRadius: 2, background: "var(--accent-strong)" }} />}

      <div style={{
        fontSize: 12.5, fontWeight: active ? 600 : 500, color: "var(--text)", lineHeight: 1.35, letterSpacing: "-0.005em",
        display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: 2, overflow: "hidden",
        paddingRight: hover ? 20 : 0, transition: "padding 80ms ease",
      }}>{session.title || t("chat.untitled")}</div>

      <div style={{ display: "flex", alignItems: "center", gap: 6, marginTop: 1, minWidth: 0 }}>
        <ScopeChip session={session} dense={density === "compact"} />
        <span style={{ fontSize: 10.5, color: "var(--text-faint)", fontVariantNumeric: "tabular-nums", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
          {relativeTime(session.updated_at)}
        </span>
      </div>

      {hover && (
        <button
          onClick={(e) => { e.stopPropagation(); setMenuOpen((o) => !o); }}
          style={{ position: "absolute", top: 6, right: 6, width: 18, height: 18, padding: 0, border: "none", background: menuOpen ? "var(--surface)" : "transparent", borderRadius: 4, cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center", color: "var(--text-mute)", boxShadow: menuOpen ? "0 0 0 1px var(--border)" : "none" }}
          title={t("chat.sessionMenu")}
        >
          <ChatIcon name="more" size={11} color="var(--text-mute)" />
        </button>
      )}

      {menuOpen && (
        <div onClick={(e) => e.stopPropagation()} style={{ position: "absolute", top: 26, right: 6, zIndex: 5, background: "var(--surface)", border: "1px solid var(--border-strong)", borderRadius: 6, boxShadow: "0 6px 20px rgba(0,0,0,0.12)", overflow: "hidden", minWidth: 120 }}>
          <MenuItem icon="edit" label={t("chat.rename")} onClick={handleRename} />
          <MenuItem icon="archive" label={t("chat.archive")} onClick={() => { setMenuOpen(false); void archiveSession(session.id); }} />
        </div>
      )}
    </div>
  );
}

function MenuItem({ icon, label, onClick }: { icon: "edit" | "archive"; label: string; onClick: () => void }) {
  const [h, setH] = useState(false);
  return (
    <button
      onMouseEnter={() => setH(true)} onMouseLeave={() => setH(false)} onClick={onClick}
      style={{ display: "flex", alignItems: "center", gap: 8, width: "100%", padding: "7px 10px", border: "none", background: h ? "var(--hover)" : "transparent", color: "var(--text)", cursor: "pointer", fontSize: 12, textAlign: "left" }}
    >
      <ChatIcon name={icon} size={12} color="var(--text-mute)" />
      {label}
    </button>
  );
}

function NewChatButton({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation();
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={onClick} onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}
      style={{
        display: "flex", alignItems: "center", gap: 8, width: "100%", padding: "8px 10px",
        border: "1px solid " + (hover ? "var(--accent-strong)" : "var(--border-strong)"),
        background: hover ? "color-mix(in oklch, var(--accent-strong) 6%, var(--surface))" : "var(--surface)",
        color: hover ? "var(--accent-strong)" : "var(--text)", borderRadius: 6, cursor: "pointer",
        fontSize: 12.5, fontWeight: 500, boxShadow: "0 1px 0 rgba(0,0,0,0.02)", transition: "all 100ms ease",
      }}
    >
      <span style={{ display: "inline-flex", alignItems: "center", justifyContent: "center", width: 16, height: 16, borderRadius: 4, background: "var(--accent-strong)", color: "white" }}>
        <ChatIcon name="plus" size={10} color="white" strokeWidth={2} />
      </span>
      <span style={{ flex: 1, textAlign: "left" }}>{t("chat.newChat")}</span>
      <span style={{ fontSize: 10, color: hover ? "var(--accent-strong)" : "var(--text-faint)", padding: "1px 5px", border: "1px solid currentColor", borderRadius: 3, fontFamily: "var(--mono)", opacity: 0.7 }}>⌘N</span>
    </button>
  );
}

function GroupHeader({ label, count }: { label: string; count: number }) {
  return (
    <div style={{ padding: "10px 14px 4px", display: "flex", alignItems: "center", gap: 6, fontSize: 10, fontWeight: 600, letterSpacing: "0.08em", color: "var(--text-faint)", textTransform: "uppercase" }}>
      <span>{label}</span>
      <span style={{ fontFamily: "var(--mono)", opacity: 0.7 }}>{count}</span>
    </div>
  );
}

function EmptyState() {
  const { t } = useTranslation();
  return (
    <div style={{ padding: "32px 18px 18px", textAlign: "center", color: "var(--text-faint)" }}>
      <div style={{ width: 36, height: 36, borderRadius: 8, margin: "0 auto 10px", background: "var(--surface-2)", border: "1px solid var(--border)", display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
        <ChatIcon name="sparkle" size={16} color="var(--accent-strong)" />
      </div>
      <div style={{ fontSize: 12.5, color: "var(--text)", fontWeight: 550, marginBottom: 4 }}>{t("chat.emptySidebarTitle")}</div>
      <div style={{ fontSize: 11, lineHeight: 1.55 }}>{t("chat.emptySidebarBody")}</div>
    </div>
  );
}

function groupLabels(t: (k: "chat.groupToday" | "chat.groupYesterday" | "chat.groupEarlier") => string): Record<SessionGroup, string> {
  return {
    today: t("chat.groupToday"),
    yesterday: t("chat.groupYesterday"),
    earlier: t("chat.groupEarlier"),
  };
}

const squareBtn: React.CSSProperties = {
  width: 24, height: 24, padding: 0, border: "1px solid var(--border)", background: "var(--surface)",
  borderRadius: 5, cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center", color: "var(--text-mute)",
};
