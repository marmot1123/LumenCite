import { useState, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Icon, TypeIcon } from "./icons";
import { TagPill } from "./TagPill";
import type { EntrySummary, Density } from "../types";

// ── column width state ────────────────────────────────────────────────────────

const LS_COL_KEY = "lc-col-widths";
const MIN_COL_W  = 40;

const DEFAULT_WIDTHS = {
  title:   320,
  authors: 170,
  journal: 150,
  year:     56,
  venue:   110,
  tags:    150,
  added:    90,
} as const;

type ColKey = keyof typeof DEFAULT_WIDTHS;

function loadColWidths(): Record<ColKey, number> {
  try {
    const s = localStorage.getItem(LS_COL_KEY);
    if (s) return { ...DEFAULT_WIDTHS, ...JSON.parse(s) };
  } catch { /* ignore */ }
  return { ...DEFAULT_WIDTHS };
}

// ── resize handle ─────────────────────────────────────────────────────────────

function ResizeHandle({ onDelta }: { onDelta: (delta: number) => void }) {
  const [active, setActive] = useState(false);

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setActive(true);

    let prevX = e.clientX;
    const savedCursor   = document.body.style.cursor;
    const savedSelect   = document.body.style.userSelect;
    document.body.style.cursor     = "col-resize";
    document.body.style.userSelect = "none";

    const onMove = (ev: MouseEvent) => {
      const delta = ev.clientX - prevX;
      prevX = ev.clientX;
      if (delta !== 0) onDelta(delta);
    };
    const onUp = () => {
      setActive(false);
      document.body.style.cursor     = savedCursor;
      document.body.style.userSelect = savedSelect;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup",   onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup",   onUp);
  }, [onDelta]);

  return (
    <div
      onMouseDown={handleMouseDown}
      style={{
        position: "absolute", right: 0, top: 0, bottom: 0,
        width: 5, cursor: "col-resize", zIndex: 2,
        background: active ? "var(--accent-strong)" : "transparent",
        opacity: active ? 0.5 : 1,
      }}
    />
  );
}

// ── column header ─────────────────────────────────────────────────────────────

function ColumnHeader({ label, width, sortable, sorted, onSort, align = "left", onResize }: {
  label: string;
  width: number;
  sortable?: boolean;
  sorted?: "asc" | "desc" | null;
  onSort?: () => void;
  align?: "left" | "right";
  onResize?: (delta: number) => void;
}) {
  return (
    <div
      onClick={sortable ? onSort : undefined}
      style={{
        width, flexShrink: 0, position: "relative",
        padding: "0 10px", height: 28,
        display: "flex", alignItems: "center", gap: 4,
        justifyContent: align === "right" ? "flex-end" : "flex-start",
        fontSize: 11, fontWeight: 600, color: "var(--text-mute)",
        letterSpacing: "0.02em",
        borderRight: "1px solid var(--border)",
        cursor: sortable ? "pointer" : "default",
        userSelect: "none",
      }}
    >
      <span>{label}</span>
      {sorted && (
        <span style={{ color: "var(--accent-strong)", display: "inline-flex" }}>
          <svg width="9" height="9" viewBox="0 0 9 9" fill="none">
            {sorted === "asc"
              ? <path d="M2 6l2.5-3 2.5 3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
              : <path d="M2 3l2.5 3 2.5-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>}
          </svg>
        </span>
      )}
      {onResize && <ResizeHandle onDelta={onResize} />}
    </div>
  );
}

// ── helpers ───────────────────────────────────────────────────────────────────

function fmtDate(s: string): string {
  // DB stores "YYYY-MM-DD HH:MM:SS" (SQLite datetime)
  const d = new Date(s.replace(" ", "T") + "Z");
  if (isNaN(d.getTime())) return s.slice(0, 10);
  return `${d.getFullYear()}/${String(d.getMonth() + 1).padStart(2, "0")}/${String(d.getDate()).padStart(2, "0")}`;
}

function formatAuthors(authors: EntrySummary["authors"]): string {
  if (!authors.length) return "—";
  if (authors.length === 1) return authors[0].name;
  if (authors.length === 2) return `${authors[0].name}, ${authors[1].name}`;
  return `${authors[0].name} et al.`;
}

// ── row ───────────────────────────────────────────────────────────────────────

function Row({ entry, selected, onClick, onDoubleClick, onStartDrag, isDragging, density, widths, onToggleStar }: {
  entry: EntrySummary;
  selected: boolean;
  onClick: (mods: { meta?: boolean; shift?: boolean }) => void;
  onDoubleClick?: () => void;
  onStartDrag: (id: number, e: React.MouseEvent) => void;
  isDragging: boolean;
  density: Density;
  widths: Record<ColKey, number>;
  onToggleStar: (id: number, starred: boolean) => void;
}) {
  const { t } = useTranslation();
  const [hover, setHover] = useState(false);
  const rowH = density === "compact" ? 30 : density === "comfortable" ? 42 : 36;

  return (
    <div
      onClick={(e) => {
        if (isDragging) return;
        onClick({ meta: e.metaKey || e.ctrlKey, shift: e.shiftKey });
      }}
      onDoubleClick={onDoubleClick}
      onMouseDown={(e) => {
        // Shift+Click は範囲選択（テキスト選択ではなく行選択）なので preventDefault でデフォルト動作を抑止。
        // ドラッグ開始は通常クリックのみ（Cmd/Shift では選択操作を優先）。
        e.preventDefault();
        if (e.metaKey || e.ctrlKey || e.shiftKey) return;
        onStartDrag(entry.id, e);
      }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "flex", alignItems: "center",
        height: rowH, fontSize: 12.5,
        background: selected ? "var(--row-selected)" : hover ? "var(--row-hover)" : "transparent",
        borderBottom: "1px solid var(--border-subtle)",
        cursor: "default", position: "relative",
      }}
    >
      {selected && (
        <span style={{
          position: "absolute", left: 0, top: 0, bottom: 0, width: 2,
          background: "var(--accent-strong)",
        }} />
      )}

      {/* star */}
      <div
        onMouseDown={(e) => { e.stopPropagation(); }}
        onClick={(e) => { e.stopPropagation(); onToggleStar(entry.id, !entry.starred); }}
        style={{
          width: 28, flexShrink: 0, padding: "0 4px 0 10px",
          display: "flex", justifyContent: "center", alignItems: "center",
          cursor: "pointer", height: "100%",
        }}
      >
        {(entry.starred || hover) && (
          <Icon
            name="star"
            size={12}
            color={entry.starred ? "var(--accent-strong)" : "var(--text-faint)"}
          />
        )}
      </div>

      {/* type */}
      <div style={{ width: 28, flexShrink: 0, display: "flex", justifyContent: "center", color: "var(--text-mute)" }}>
        <TypeIcon type={entry.entry_type} size={13} />
      </div>

      {/* title */}
      <div style={{ width: widths.title, flexShrink: 0, padding: "0 12px 0 4px", display: "flex", alignItems: "center", gap: 8, overflow: "hidden" }}>
        <div style={{
          flex: 1, minWidth: 0,
          fontSize: 13, fontWeight: 500, color: "var(--text)",
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
          letterSpacing: "-0.005em",
        }}>{entry.title}</div>
        {entry.has_attachment && (
          <span style={{ color: "var(--text-mute)", flexShrink: 0 }} title={t("entriesTable.hasPdf")}>
            <Icon name="paperclip" size={12} />
          </span>
        )}
      </div>

      {/* authors */}
      <div style={{
        width: widths.authors, flexShrink: 0, padding: "0 10px",
        fontSize: 12, color: "var(--text-mute)",
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
      }}>{formatAuthors(entry.authors)}</div>

      {/* journal */}
      <div style={{
        width: widths.journal, flexShrink: 0, padding: "0 10px",
        fontSize: 12, color: "var(--text-mute)",
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
        fontStyle: "italic",
      }}>{entry.journal ?? "—"}</div>

      {/* year */}
      <div style={{
        width: widths.year, flexShrink: 0, padding: "0 10px",
        fontSize: 12, color: "var(--text-mute)", fontVariantNumeric: "tabular-nums",
      }}>{entry.year ?? "—"}</div>

      {/* venue / entry_type */}
      <div style={{
        width: widths.venue, flexShrink: 0, padding: "0 10px",
        fontSize: 12, color: "var(--text-mute)",
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
        fontStyle: "italic",
      }}>{entry.entry_type}</div>

      {/* tags */}
      <div style={{
        width: widths.tags, flexShrink: 0, padding: "0 10px",
        display: "flex", gap: 4, alignItems: "center", overflow: "hidden",
      }}>
        {entry.tags.slice(0, 3).map(t => <TagPill key={t.id} name={t.name} />)}
        {entry.tags.length > 3 && (
          <span style={{ fontSize: 10.5, color: "var(--text-faint)", flexShrink: 0 }}>
            +{entry.tags.length - 3}
          </span>
        )}
      </div>

      {/* added */}
      <div style={{
        width: widths.added, flexShrink: 0, padding: "0 14px 0 10px",
        fontSize: 11.5, color: "var(--text-faint)",
        fontVariantNumeric: "tabular-nums", textAlign: "right",
      }}>{fmtDate(entry.created_at)}</div>
    </div>
  );
}

// ── main component ────────────────────────────────────────────────────────────

interface EntriesTableProps {
  entries: EntrySummary[];
  selectedIds: Set<number>;
  onSelect: (id: number, mods: { meta?: boolean; shift?: boolean }) => void;
  onOpenDetail?: (id: number) => void;
  sort: { key: string; dir: "asc" | "desc" };
  onSort: (key: string) => void;
  density: Density;
  draggingId: number | null;
  onStartDrag: (id: number, e: React.MouseEvent) => void;
  onToggleStar: (id: number, starred: boolean) => void;
  isEmptyLibrary?: boolean;
  onAddEntry?: () => void;
}

export function EntriesTable({ entries, selectedIds, onSelect, onOpenDetail, sort, onSort, density, draggingId, onStartDrag, onToggleStar, isEmptyLibrary, onAddEntry }: EntriesTableProps) {
  const { t } = useTranslation();
  const [widths, setWidths] = useState<Record<ColKey, number>>(loadColWidths);

  const resize = useCallback((col: ColKey, delta: number) => {
    setWidths(prev => {
      const next = { ...prev, [col]: Math.max(MIN_COL_W, prev[col] + delta) };
      try { localStorage.setItem(LS_COL_KEY, JSON.stringify(next)); } catch { /* ignore */ }
      return next;
    });
  }, []);

  const FIXED = 28 + 28; // star + type
  const totalW = FIXED + widths.title + widths.authors + widths.journal + widths.year + widths.venue + widths.tags + widths.added;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ flex: 1, overflow: "auto" }}>
        {/* sticky header — scrolls horizontally with content */}
        <div style={{
          display: "flex", flexShrink: 0,
          borderBottom: "1px solid var(--border)",
          background: "var(--surface-2)",
          position: "sticky", top: 0, zIndex: 1,
          minWidth: totalW,
        }}>
          <div style={{ width: 28, flexShrink: 0 }} />
          <div style={{ width: 28, flexShrink: 0 }} />
          <ColumnHeader label={t("entriesTable.colTitle")} width={widths.title} sortable
            sorted={sort.key === "title" ? sort.dir : null}
            onSort={() => onSort("title")}
            onResize={d => resize("title", d)} />
          <ColumnHeader label={t("entriesTable.colAuthors")} width={widths.authors} sortable
            sorted={sort.key === "authors" ? sort.dir : null}
            onSort={() => onSort("authors")}
            onResize={d => resize("authors", d)} />
          <ColumnHeader label={t("entriesTable.colJournal")} width={widths.journal}
            onResize={d => resize("journal", d)} />
          <ColumnHeader label={t("entriesTable.colYear")} width={widths.year} sortable
            sorted={sort.key === "year" ? sort.dir : null}
            onSort={() => onSort("year")}
            onResize={d => resize("year", d)} />
          <ColumnHeader label={t("entriesTable.colVenue")} width={widths.venue}
            onResize={d => resize("venue", d)} />
          <ColumnHeader label={t("entriesTable.colTags")} width={widths.tags}
            onResize={d => resize("tags", d)} />
          <ColumnHeader label={t("entriesTable.colAdded")} width={widths.added} align="right" sortable
            sorted={sort.key === "added" ? sort.dir : null}
            onSort={() => onSort("added")} />
        </div>

        {/* rows */}
        <div style={{ minWidth: totalW }}>
          {entries.length === 0 ? (
            isEmptyLibrary ? (
              <div style={{
                padding: "80px 24px 60px", textAlign: "center",
                display: "flex", flexDirection: "column", alignItems: "center", gap: 12,
              }}>
                <div style={{ fontSize: 15, fontWeight: 600, color: "var(--text)" }}>
                  {t("entriesTable.emptyLibTitle")}
                </div>
                <div style={{
                  fontSize: 12.5, color: "var(--text-mute)", lineHeight: 1.7, maxWidth: 360,
                }}>
                  {t("entriesTable.emptyLibBody")}<br />
                  {t("entriesTable.emptyLibBody2")}
                </div>
                {onAddEntry && (
                  <button
                    onClick={onAddEntry}
                    style={{
                      marginTop: 6,
                      padding: "7px 14px", borderRadius: 6,
                      border: "none", background: "var(--accent-strong)",
                      color: "white", fontSize: 12.5, fontWeight: 500,
                      cursor: "pointer",
                    }}
                  >
                    {t("entriesTable.emptyAction")}
                  </button>
                )}
                <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>
                  {t("entriesTable.emptyShortcut")}
                </div>
              </div>
            ) : (
              <div style={{
                padding: "60px 20px", textAlign: "center",
                color: "var(--text-faint)", fontSize: 13,
              }}>
                {t("entriesTable.noResults")}
              </div>
            )
          ) : entries.map(e => (
            <Row
              key={e.id}
              entry={e}
              selected={selectedIds.has(e.id)}
              onClick={(mods) => onSelect(e.id, mods)}
              onDoubleClick={onOpenDetail ? () => onOpenDetail(e.id) : undefined}
              onStartDrag={onStartDrag}
              isDragging={draggingId !== null}
              density={density}
              widths={widths}
              onToggleStar={onToggleStar}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
