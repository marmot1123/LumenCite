// 右パネル: スコープ文献 + このセッションで使われたツール集計 + 承認ポリシー。
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { useChatStore } from "../../chat/store";
import { ChatIcon } from "./ChatIcon";
import { KIND_META, toolKind, type ToolKind } from "./ToolCallCard";
import type { EntryDetail } from "../../types";

interface ScopeEntry {
  id: number;
  title: string;
}

export function ContextPanel() {
  const { t } = useTranslation();
  const entryIds = useChatStore((s) => s.entryIds);
  const messages = useChatStore((s) => s.messages);
  const [entries, setEntries] = useState<ScopeEntry[]>([]);

  // スコープ文献のタイトルを取得（少数想定）
  useEffect(() => {
    let cancelled = false;
    if (entryIds.length === 0) {
      setEntries([]);
      return;
    }
    void (async () => {
      const loaded: ScopeEntry[] = [];
      for (const id of entryIds) {
        try {
          const e = await invoke<EntryDetail>("get_entry", { id });
          loaded.push({ id, title: e.title });
        } catch {
          loaded.push({ id, title: `#${id}` });
        }
      }
      if (!cancelled) setEntries(loaded);
    })();
    return () => { cancelled = true; };
  }, [entryIds]);

  // メッセージからツール使用を集計（kind ごとに件数 + 承認待ち件数）
  const toolStats = useMemo(() => {
    const stats = new Map<ToolKind, { count: number; pending: number }>();
    for (const m of messages) {
      for (const tc of m.tool_calls) {
        const k = toolKind(tc.tool_name);
        const s = stats.get(k) ?? { count: 0, pending: 0 };
        s.count += 1;
        if (tc.state === "needs_approval") s.pending += 1;
        stats.set(k, s);
      }
    }
    return stats;
  }, [messages]);

  return (
    <aside style={{ width: 280, flexShrink: 0, height: "100%", background: "var(--surface)", borderLeft: "1px solid var(--border)", display: "flex", flexDirection: "column", overflow: "hidden" }}>
      <div style={{ padding: "12px 16px 11px", borderBottom: "1px solid var(--border)", display: "flex", alignItems: "center", gap: 8 }}>
        <div style={{ fontSize: 12, fontWeight: 600, color: "var(--text)", display: "flex", alignItems: "center", gap: 6 }}>
          {t("chat.contextTitle")}
          <span style={{ fontSize: 10.5, padding: "1px 6px", borderRadius: 999, background: "var(--surface-2)", color: "var(--text-faint)", fontVariantNumeric: "tabular-nums" }}>{entryIds.length}</span>
        </div>
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: "12px 14px" }}>
        <SectionLabel>{t("chat.contextScope")}</SectionLabel>
        {entries.length === 0 ? (
          <div style={{ fontSize: 11.5, color: "var(--text-faint)", marginBottom: 14, display: "flex", alignItems: "center", gap: 6 }}>
            <ChatIcon name="library" size={13} color="var(--text-faint)" />
            {t("chat.scopeModeAll")}
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 14 }}>
            {entries.map((e, i) => (
              <div key={e.id} style={{ padding: "7px 9px", borderRadius: 6, background: "var(--surface-2)", border: "1px solid var(--border)", display: "flex", gap: 8, alignItems: "flex-start" }}>
                <div style={{ width: 18, height: 18, borderRadius: 4, flexShrink: 0, marginTop: 1, background: "var(--accent-soft)", color: "var(--accent-strong)", display: "inline-flex", alignItems: "center", justifyContent: "center", fontSize: 10, fontWeight: 600, fontFamily: "var(--mono)" }}>{i + 1}</div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 11.5, fontWeight: 550, color: "var(--text)", lineHeight: 1.4 }}>{e.title}</div>
                  <div style={{ fontSize: 10, color: "var(--text-faint)", fontFamily: "var(--mono)", marginTop: 2 }}>entry #{e.id}</div>
                </div>
              </div>
            ))}
          </div>
        )}

        {toolStats.size > 0 && (
          <>
            <SectionLabel>{t("chat.contextTools")}</SectionLabel>
            <div style={{ display: "flex", flexDirection: "column", gap: 3, marginBottom: 14 }}>
              {[...toolStats.entries()].map(([kind, s]) => (
                <ToolCountRow key={kind} kind={kind} count={s.count} pending={s.pending} />
              ))}
            </div>
          </>
        )}

        <div style={{ padding: "10px 12px", borderRadius: 7, background: "var(--surface-2)", border: "1px dashed var(--border)", fontSize: 11, color: "var(--text-mute)", lineHeight: 1.55 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
            <ChatIcon name="info" size={11} color="var(--text-mute)" />
            <span style={{ fontWeight: 600, color: "var(--text)" }}>{t("chat.approvalPolicyTitle")}</span>
          </div>
          {t("chat.approvalPolicyBody")}
        </div>
      </div>
    </aside>
  );
}

function ToolCountRow({ kind, count, pending }: { kind: ToolKind; count: number; pending: number }) {
  const meta = KIND_META[kind];
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "4px 6px", fontSize: 11, color: "var(--text)" }}>
      <span style={{ width: 18, height: 18, borderRadius: 4, flexShrink: 0, display: "inline-flex", alignItems: "center", justifyContent: "center", background: "color-mix(in oklch, " + meta.fg + " 10%, transparent)" }}>
        <ChatIcon name={meta.glyph} size={10} color={meta.fg} />
      </span>
      <span style={{ flex: 1, fontFamily: "var(--mono)", fontSize: 10.5, color: "var(--text-mute)" }}>{meta.label}</span>
      <span style={{ fontSize: 10, fontFamily: "var(--mono)", fontWeight: pending > 0 ? 600 : 500, color: pending > 0 ? meta.fg : "var(--text-faint)" }}>
        {pending > 0 ? `● ${count}` : count}
      </span>
    </div>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ fontSize: 9.5, fontWeight: 600, letterSpacing: "0.08em", color: "var(--text-faint)", textTransform: "uppercase", margin: "2px 4px 6px" }}>{children}</div>
  );
}
