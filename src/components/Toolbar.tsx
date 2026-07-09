import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Icon } from "./icons";
import { FilterPanel } from "./FilterPanel";
import { filterCount } from "../types";
import type { EntryFilter, SearchScope, Tag, ViewMode } from "../types";

interface ToolbarProps {
  title: string;
  subtitle?: string;
  count: number;
  search: string;
  onSearchChange: (q: string) => void;
  searchScope: SearchScope;
  onSearchScopeChange: (scope: SearchScope) => void;
  onAddOpen: () => void;
  onImport: () => void;
  onExportBibtex: () => void;
  exportDisabled?: boolean;
  inTrash?: boolean;
  onEmptyTrash?: () => void;
  emptyTrashDisabled?: boolean;
  filter: EntryFilter;
  onFilterChange: (f: EntryFilter) => void;
  onClearFilter: () => void;
  tags: Tag[];
}

interface ViewTabsProps {
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
}

function ToolbarBtn({ icon, label, onClick, primary, danger, disabled, title }: {
  icon: Parameters<typeof Icon>[0]["name"];
  label?: string;
  onClick?: () => void;
  primary?: boolean;
  danger?: boolean;
  disabled?: boolean;
  title?: string;
}) {
  const [hover, setHover] = useState(false);
  const iconColor = disabled
    ? "var(--text-faint)"
    : primary
      ? "white"
      : danger
        ? "var(--danger-strong)"
        : "var(--text-mute)";
  return (
    <button
      onClick={disabled ? undefined : onClick}
      disabled={disabled}
      title={title}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "inline-flex", alignItems: "center", gap: 5,
        padding: label ? "5px 9px 5px 8px" : "5px 6px",
        borderRadius: 6,
        border: "1px solid transparent",
        background: primary
          ? "var(--accent-strong)"
          : hover && !disabled
            ? "var(--hover)"
            : "transparent",
        color: disabled
          ? "var(--text-faint)"
          : primary
            ? "white"
            : danger
              ? "var(--danger-text)"
              : "var(--text)",
        fontSize: 12, fontWeight: 500,
        cursor: disabled ? "not-allowed" : "pointer",
        transition: "background 80ms ease",
      }}
    >
      <Icon name={icon} size={13} color={iconColor} />
      {label && <span>{label}</span>}
    </button>
  );
}

const TABS: { id: ViewMode; icon: Parameters<typeof Icon>[0]["name"]; enabled: boolean }[] = [
  { id: "table",    icon: "list",  enabled: true },
  { id: "covers",   icon: "grid",  enabled: true },
  { id: "timeline", icon: "clock", enabled: false },
  { id: "graph",    icon: "sync",  enabled: false },
];

export function ViewTabs({ viewMode, onViewModeChange }: ViewTabsProps) {
  const { t } = useTranslation();
  const subtitle =
    viewMode === "table" ? t("toolbar.viewMode.tableDesc")
    : viewMode === "covers" ? t("toolbar.viewMode.coversDesc")
    : "";
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 2,
      padding: "0 12px", height: 34, flexShrink: 0,
      borderBottom: "1px solid var(--border)",
      background: "var(--surface)",
    }}>
      {TABS.map(tab => {
        const active = viewMode === tab.id;
        return (
          <button key={tab.id}
            onClick={() => tab.enabled && onViewModeChange(tab.id)}
            disabled={!tab.enabled}
            style={{
              display: "inline-flex", alignItems: "center", gap: 5,
              padding: "0 10px", height: 34,
              border: "none", background: "transparent",
              fontSize: 12, fontWeight: active ? 600 : 500,
              color: !tab.enabled ? "var(--text-faint)" : active ? "var(--text)" : "var(--text-mute)",
              cursor: tab.enabled ? "pointer" : "not-allowed",
              opacity: tab.enabled ? 1 : 0.55,
              borderBottom: active ? "2px solid var(--accent-strong)" : "2px solid transparent",
              marginBottom: -1,
            }}>
            <Icon name={tab.icon} size={12} color={active ? "var(--text)" : "var(--text-mute)"} />
            {t(`toolbar.viewMode.${tab.id}`)}
            {!tab.enabled && (
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
      <span style={{ fontSize: 11, color: "var(--text-faint)" }}>{subtitle}</span>
    </div>
  );
}

function ScopeToggle({ scope, onChange }: { scope: SearchScope; onChange: (s: SearchScope) => void }) {
  const { t } = useTranslation();
  const opts: { id: SearchScope; label: string }[] = [
    { id: "meta",     label: t("toolbar.searchScope.meta") },
    { id: "fulltext", label: t("toolbar.searchScope.fulltext") },
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

export function Toolbar({ title, subtitle, count, search, onSearchChange, searchScope, onSearchScopeChange, onAddOpen, onImport, onExportBibtex, exportDisabled, inTrash, onEmptyTrash, emptyTrashDisabled, filter, onFilterChange, onClearFilter, tags }: ToolbarProps) {
  const { t } = useTranslation();
  const [showFilter, setShowFilter] = useState(false);
  const activeCount = filterCount(filter);
  // フィルタは全文検索結果には適用しない（v0.6.0 スコープ外）ため fulltext では無効化。
  const filterDisabled = searchScope === "fulltext";
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
          {inTrash ? (
            <ToolbarBtn
              icon="trash"
              label={t("toolbar.emptyTrash")}
              danger
              disabled={emptyTrashDisabled}
              onClick={onEmptyTrash}
              title={emptyTrashDisabled ? t("toolbar.emptyTrashEmpty") : t("toolbar.emptyTrashWarn")}
            />
          ) : (
            <>
              <ToolbarBtn icon="upload" label={t("toolbar.import")} onClick={onImport} />
              <ToolbarBtn
                icon="download"
                label={t("toolbar.exportBibtex")}
                onClick={exportDisabled ? undefined : onExportBibtex}
              />
              <ToolbarBtn icon="plus" label={t("toolbar.addEntry")} primary onClick={onAddOpen} />
            </>
          )}
        </div>
      </div>

      {/* row 2 */}
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "0 16px 10px" }}>
        <div style={{
          display: "flex", alignItems: "center", gap: 6,
          flex: 1, maxWidth: 560,
          padding: "5px 6px 5px 10px",
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          borderRadius: 6, height: 28,
        }}>
          <Icon name="search" size={12} color="var(--text-faint)" />
          <input
            id="toolbar-search"
            value={search}
            onChange={e => onSearchChange(e.target.value)}
            placeholder={searchScope === "fulltext"
              ? t("toolbar.searchPlaceholder.fulltext")
              : t("toolbar.searchPlaceholder.meta")}
            style={{
              flex: 1, minWidth: 0, border: "none", outline: "none", background: "transparent",
              fontSize: 12.5, color: "var(--text)",
            }}
          />
          {search && (
            <button
              onClick={() => onSearchChange("")}
              title={t("toolbar.clearSearch")}
              aria-label={t("toolbar.clearSearch")}
              style={{
                display: "inline-flex", alignItems: "center", justifyContent: "center",
                width: 16, height: 16, padding: 0, flexShrink: 0,
                border: "none", borderRadius: 999, background: "transparent",
                color: "var(--text-faint)", cursor: "pointer",
              }}
            >
              <Icon name="close" size={12} color="var(--text-faint)" />
            </button>
          )}
        </div>
        <ScopeToggle scope={searchScope} onChange={onSearchScopeChange} />
        <div style={{ position: "relative", display: "inline-flex" }}>
          <button
            onClick={filterDisabled ? undefined : () => setShowFilter(s => !s)}
            disabled={filterDisabled}
            title={filterDisabled ? t("filter.fulltextDisabled") : t("toolbar.filter")}
            style={{
              display: "inline-flex", alignItems: "center", gap: 5,
              padding: "5px 9px 5px 8px", borderRadius: 6,
              border: activeCount > 0 ? "1px solid var(--accent-strong)" : "1px solid transparent",
              background: showFilter ? "var(--hover)" : "transparent",
              color: filterDisabled ? "var(--text-faint)" : activeCount > 0 ? "var(--accent-strong)" : "var(--text)",
              fontSize: 12, fontWeight: 500,
              cursor: filterDisabled ? "not-allowed" : "pointer",
            }}
          >
            <Icon name="filter" size={13} color={filterDisabled ? "var(--text-faint)" : activeCount > 0 ? "var(--accent-strong)" : "var(--text-mute)"} />
            <span>{t("toolbar.filter")}</span>
            {activeCount > 0 && (
              <span style={{
                minWidth: 15, height: 15, padding: "0 4px",
                display: "inline-flex", alignItems: "center", justifyContent: "center",
                borderRadius: 999, background: "var(--accent-strong)", color: "white",
                fontSize: 9.5, fontWeight: 700, fontVariantNumeric: "tabular-nums",
              }}>{activeCount}</span>
            )}
          </button>
          {showFilter && !filterDisabled && (
            <FilterPanel
              filter={filter}
              onChange={onFilterChange}
              onClear={onClearFilter}
              tags={tags}
              onClose={() => setShowFilter(false)}
            />
          )}
        </div>
      </div>
    </header>
  );
}
