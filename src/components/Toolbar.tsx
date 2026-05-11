import { useState } from "react";
import { Icon } from "./icons";
import type { SearchScope, ViewMode } from "../types";

interface ToolbarProps {
  title: string;
  subtitle?: string;
  count: number;
  search: string;
  onSearchChange: (q: string) => void;
  searchScope: SearchScope;
  onSearchScopeChange: (scope: SearchScope) => void;
  onAddOpen: () => void;
  onExportBibtex: () => void;
  exportDisabled?: boolean;
}

interface ViewTabsProps {
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
}

function ToolbarBtn({ icon, label, onClick, primary }: {
  icon: Parameters<typeof Icon>[0]["name"];
  label?: string;
  onClick?: () => void;
  primary?: boolean;
}) {
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "inline-flex", alignItems: "center", gap: 5,
        padding: label ? "5px 9px 5px 8px" : "5px 6px",
        borderRadius: 6,
        border: "1px solid transparent",
        background: primary ? "var(--accent-strong)" : hover ? "var(--hover)" : "transparent",
        color: primary ? "white" : "var(--text)",
        fontSize: 12, fontWeight: 500, cursor: "pointer",
        transition: "background 80ms ease",
      }}
    >
      <Icon name={icon} size={13} color={primary ? "white" : "var(--text-mute)"} />
      {label && <span>{label}</span>}
    </button>
  );
}

const TABS: { id: ViewMode; label: string; icon: Parameters<typeof Icon>[0]["name"]; enabled: boolean }[] = [
  { id: "table",    label: "表",           icon: "list",  enabled: true },
  { id: "covers",   label: "カバー",       icon: "grid",  enabled: true },
  { id: "timeline", label: "タイムライン", icon: "clock", enabled: false },
  { id: "graph",    label: "引用グラフ",   icon: "sync",  enabled: false },
];

export function ViewTabs({ viewMode, onViewModeChange }: ViewTabsProps) {
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 2,
      padding: "0 12px", height: 34, flexShrink: 0,
      borderBottom: "1px solid var(--border)",
      background: "var(--surface)",
    }}>
      {TABS.map(t => {
        const active = viewMode === t.id;
        return (
          <button key={t.id}
            onClick={() => t.enabled && onViewModeChange(t.id)}
            disabled={!t.enabled}
            style={{
              display: "inline-flex", alignItems: "center", gap: 5,
              padding: "0 10px", height: 34,
              border: "none", background: "transparent",
              fontSize: 12, fontWeight: active ? 600 : 500,
              color: !t.enabled ? "var(--text-faint)" : active ? "var(--text)" : "var(--text-mute)",
              cursor: t.enabled ? "pointer" : "not-allowed",
              opacity: t.enabled ? 1 : 0.55,
              borderBottom: active ? "2px solid var(--accent-strong)" : "2px solid transparent",
              marginBottom: -1,
            }}>
            <Icon name={t.icon} size={12} color={active ? "var(--text)" : "var(--text-mute)"} />
            {t.label}
            {!t.enabled && (
              <span style={{
                fontSize: 9.5, padding: "1px 4px", borderRadius: 3,
                background: "var(--surface-2)", color: "var(--text-faint)",
                marginLeft: 2, fontWeight: 500,
              }}>soon</span>
            )}
          </button>
        );
      })}
      <div style={{ flex: 1 }} />
      <span style={{ fontSize: 11, color: "var(--text-faint)" }}>
        {viewMode === "table" ? "メタデータ重視" : viewMode === "covers" ? "PDFサムネイル" : ""}
      </span>
    </div>
  );
}

function ScopeToggle({ scope, onChange }: { scope: SearchScope; onChange: (s: SearchScope) => void }) {
  const opts: { id: SearchScope; label: string }[] = [
    { id: "meta",     label: "メタ" },
    { id: "fulltext", label: "全文" },
  ];
  return (
    <div style={{
      display: "inline-flex", gap: 0, padding: 2,
      background: "var(--surface-2)", border: "1px solid var(--border)",
      borderRadius: 6, height: 24,
    }}>
      {opts.map(o => {
        const active = scope === o.id;
        return (
          <button key={o.id}
            onClick={() => onChange(o.id)}
            style={{
              padding: "0 10px", height: 20,
              border: "none",
              background: active ? "var(--surface)" : "transparent",
              color: active ? "var(--text)" : "var(--text-mute)",
              fontSize: 11, fontWeight: active ? 600 : 500,
              borderRadius: 4, cursor: "pointer",
              boxShadow: active ? "0 1px 2px rgba(0,0,0,0.05)" : "none",
            }}>
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

export function Toolbar({ title, subtitle, count, search, onSearchChange, searchScope, onSearchScopeChange, onAddOpen, onExportBibtex, exportDisabled }: ToolbarProps) {
  return (
    <header style={{ flexShrink: 0, borderBottom: "1px solid var(--border)", background: "var(--surface)" }}>
      {/* row 1 */}
      <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "10px 16px 10px 14px", height: 50 }}>
        <div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0 }}>
          <h1 style={{
            margin: 0, fontSize: 15, fontWeight: 600, color: "var(--text)",
            letterSpacing: "-0.01em",
            display: "flex", alignItems: "center", gap: 8,
          }}>
            {title}
            <span style={{
              fontSize: 11, fontWeight: 500, color: "var(--text-faint)",
              padding: "1px 7px", borderRadius: 999,
              background: "var(--surface-2)",
              fontVariantNumeric: "tabular-nums",
            }}>{count}</span>
          </h1>
          {subtitle && (
            <div style={{ fontSize: 11.5, color: "var(--text-faint)", marginTop: 2 }}>{subtitle}</div>
          )}
        </div>
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <ToolbarBtn icon="upload" label="インポート" />
          <ToolbarBtn
            icon="download"
            label="BibTeX 書き出し"
            onClick={exportDisabled ? undefined : onExportBibtex}
          />
          <ToolbarBtn icon="plus" label="文献を追加" primary onClick={onAddOpen} />
        </div>
      </div>

      {/* row 2 */}
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "0 16px 10px" }}>
        <div style={{
          display: "flex", alignItems: "center", gap: 6,
          flex: 1, maxWidth: 460,
          padding: "5px 10px",
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          borderRadius: 6, height: 28,
        }}>
          <Icon name="search" size={12} color="var(--text-faint)" />
          <input
            value={search}
            onChange={e => onSearchChange(e.target.value)}
            placeholder={searchScope === "fulltext"
              ? "PDF本文を検索…"
              : "タイトル・著者・DOI で検索…"}
            style={{
              flex: 1, border: "none", outline: "none", background: "transparent",
              fontSize: 12.5, color: "var(--text)",
            }}
          />
          <ScopeToggle scope={searchScope} onChange={onSearchScopeChange} />
        </div>
        <ToolbarBtn icon="filter" label="フィルタ" />
        <div style={{ width: 1, height: 18, background: "var(--border)" }} />
        <ToolbarBtn icon="columns" label="列" />
        <div style={{ flex: 1 }} />
        <ToolbarBtn icon="sortAsc" label="並び替え" />
      </div>
    </header>
  );
}
