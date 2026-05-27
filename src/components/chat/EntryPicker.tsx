// 文献の検索 + 複数選択（ScopePicker / NewSessionDialog で共用）。
// 選択状態は親が Set<number> で保持し、onToggle で更新する。
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { ChatIcon } from "./ChatIcon";
import type { EntrySummary } from "../../types";

interface EntryPickerProps {
  selected: Set<number>;
  onToggle: (entry: EntrySummary) => void;
  maxHeight?: number;
}

export function EntryPicker({ selected, onToggle, maxHeight = 240 }: EntryPickerProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<EntrySummary[]>([]);

  useEffect(() => {
    let cancelled = false;
    const q = query.trim();
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const res = q
            ? await invoke<EntrySummary[]>("search_entries", { query: q })
            : await invoke<EntrySummary[]>("get_entries", {});
          if (!cancelled) setResults(res.slice(0, 50));
        } catch {
          if (!cancelled) setResults([]);
        }
      })();
    }, 180);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [query]);

  return (
    <>
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "5px 9px", background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 5, height: 26, marginBottom: 4 }}>
        <ChatIcon name="search" size={11} color="var(--text-faint)" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("chat.scopeSearchPlaceholder")}
          style={{ flex: 1, border: "none", outline: "none", background: "transparent", fontSize: 12, color: "var(--text)", minWidth: 0 }}
        />
      </div>
      <div style={{ maxHeight, overflow: "auto" }}>
        {results.length === 0 ? (
          <div style={{ padding: "16px 8px", textAlign: "center", fontSize: 11.5, color: "var(--text-faint)" }}>{t("chat.noResults")}</div>
        ) : (
          results.map((e) => {
            const on = selected.has(e.id);
            return (
              <div
                key={e.id}
                onClick={() => onToggle(e)}
                style={{ display: "flex", alignItems: "center", gap: 9, padding: "6px 8px", margin: "1px 0", borderRadius: 5, cursor: "pointer", background: on ? "var(--accent-soft)" : "transparent" }}
              >
                <span style={{ width: 14, height: 14, borderRadius: 3, flexShrink: 0, border: on ? "none" : "1.2px solid var(--border-strong)", background: on ? "var(--accent-strong)" : "var(--surface)", display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
                  {on && <ChatIcon name="check" size={9} color="white" strokeWidth={2.4} />}
                </span>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12, color: "var(--text)", fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{e.title}</div>
                  <div style={{ fontSize: 10.5, color: "var(--text-faint)", fontFamily: "var(--mono)" }}>
                    #{e.id}{e.year ? ` · ${e.year}` : ""}{e.authors[0] ? ` · ${e.authors[0].name}` : ""}
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
    </>
  );
}

/** 2 つのモードボタン（all / entries）。 */
export function ScopeModeButton({ label, sub, active, onClick }: { label: string; sub: string; active: boolean; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      style={{ flex: 1, textAlign: "left", padding: "8px 10px", borderRadius: 6, cursor: "pointer", border: "1px solid " + (active ? "var(--accent-strong)" : "var(--border)"), background: active ? "var(--accent-soft)" : "var(--surface)", color: active ? "var(--accent-strong)" : "var(--text)" }}
    >
      <div style={{ fontSize: 12, fontWeight: 600 }}>{label}</div>
      <div style={{ fontSize: 10.5, marginTop: 2, color: active ? "var(--accent-strong)" : "var(--text-faint)", fontFamily: "var(--mono)", opacity: 0.85 }}>{sub}</div>
    </button>
  );
}
