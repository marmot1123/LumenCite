// ScopePicker — セッションの検索対象（all / 特定文献）を切り替えるポップオーバー。
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useChatStore } from "../../chat/store";
import { ChatIcon } from "./ChatIcon";
import { EntryPicker, ScopeModeButton } from "./EntryPicker";
import type { ChatSession, EntrySummary, ScopeMode } from "../../types";

interface ScopePickerProps {
  session: ChatSession;
  onClose: () => void;
}

export function ScopePicker({ session, onClose }: ScopePickerProps) {
  const { t } = useTranslation();
  const entryIds = useChatStore((s) => s.entryIds);
  const setScope = useChatStore((s) => s.setScope);
  const [mode, setMode] = useState<ScopeMode>(session.scope_mode);
  const [picked, setPicked] = useState<Set<number>>(() => new Set(entryIds));

  const toggle = (e: EntrySummary) => {
    setPicked((prev) => {
      const next = new Set(prev);
      if (next.has(e.id)) next.delete(e.id);
      else next.add(e.id);
      return next;
    });
  };

  const apply = async () => {
    const ids = mode === "entries" ? [...picked] : [];
    await setScope(mode, ids);
    onClose();
  };

  return (
    <div onClick={onClose} style={{ position: "absolute", inset: 0, zIndex: 30, background: "rgba(20,18,14,0.05)" }}>
      <div onClick={(e) => e.stopPropagation()} style={{ position: "absolute", top: 56, left: 24, width: 420, background: "var(--surface)", border: "1px solid var(--border-strong)", borderRadius: 9, boxShadow: "0 20px 50px rgba(0,0,0,0.18), 0 1px 0 rgba(0,0,0,0.05)", overflow: "hidden" }}>
        <div style={{ padding: "12px 14px 10px", borderBottom: "1px solid var(--border)" }}>
          <div style={{ fontSize: 12, fontWeight: 600, color: "var(--text)" }}>{t("chat.scopeTitle")}</div>
          <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>{t("chat.scopeSub")}</div>
        </div>

        <div style={{ display: "flex", padding: "10px 12px 4px", gap: 6 }}>
          <ScopeModeButton label={t("chat.scopeModeAll")} sub={t("chat.scopeModeAllSub")} active={mode === "all"} onClick={() => setMode("all")} />
          <ScopeModeButton label={t("chat.scopeModeEntries")} sub={t("chat.selectedCount", { count: picked.size })} active={mode === "entries"} onClick={() => setMode("entries")} />
        </div>

        {mode === "entries" ? (
          <div style={{ padding: "8px 12px 6px" }}>
            <EntryPicker selected={picked} onToggle={toggle} />
          </div>
        ) : (
          <div style={{ padding: "20px 16px 22px", textAlign: "center", color: "var(--text-mute)", fontSize: 12, lineHeight: 1.6 }}>
            <ChatIcon name="library" size={20} color="var(--text-mute)" />
            <div style={{ marginTop: 8 }}>{t("chat.scopeAllDesc")}</div>
          </div>
        )}

        <div style={{ padding: "10px 12px", borderTop: "1px solid var(--border)", background: "var(--surface-2)", display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button onClick={onClose} style={btnGhost}>{t("chat.cancel")}</button>
          <button onClick={() => void apply()} style={btnPrimary}>{t("chat.apply")}</button>
        </div>
      </div>
    </div>
  );
}

const btnGhost: React.CSSProperties = { padding: "5px 12px", borderRadius: 5, border: "1px solid var(--border-strong)", background: "var(--surface)", color: "var(--text)", fontSize: 12, cursor: "pointer" };
const btnPrimary: React.CSSProperties = { padding: "5px 14px", borderRadius: 5, border: "none", background: "var(--accent-strong)", color: "white", fontSize: 12, fontWeight: 500, cursor: "pointer" };
