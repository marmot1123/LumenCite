import { useTranslation } from "react-i18next";
import { Icon } from "../icons";

export type PdfMode = "select" | "highlight" | "note" | "pen";

interface PdfToolbarProps {
  page: number;
  pages: number;
  onPageChange: (page: number) => void;
  zoom: number;
  onZoomChange: (zoom: number) => void;
  search: string;
  onSearchChange: (search: string) => void;
  mode: PdfMode;
  onModeChange: (mode: PdfMode) => void;
  leftOpen: boolean;
  onLeftOpenChange: (open: boolean) => void;
  rightOpen: boolean;
  onRightOpenChange: (open: boolean) => void;
  onOpenInWindow?: () => void;
}

function IconBtn({ active, onClick, title, children }: {
  active?: boolean;
  onClick?: () => void;
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      aria-label={title}
      style={{
        width: 26, height: 26, padding: 0, border: "none",
        background: active ? "var(--surface-2)" : "transparent",
        boxShadow: active ? "0 0 0 1px var(--border) inset" : "none",
        borderRadius: 5, cursor: "pointer",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        color: "var(--text-mute)",
      }}
    >{children}</button>
  );
}

type ModeTitleKey =
  | "detail.toolbar.modeSelect"
  | "detail.toolbar.modeHighlight"
  | "detail.toolbar.modeNote"
  | "detail.toolbar.modePen";

// note / pen モードは未実装（テキストレイヤーは select/highlight のみ対話可能）なので
// 実装するまでツールバーから隠す（CR-028）。PdfMode 型・ショートカットは将来のため残さない。
const MODES: { id: PdfMode; iconName: Parameters<typeof Icon>[0]["name"] | null; titleKey: ModeTitleKey }[] = [
  { id: "select",    iconName: null,          titleKey: "detail.toolbar.modeSelect" },
  { id: "highlight", iconName: "highlighter", titleKey: "detail.toolbar.modeHighlight" },
];

export function PdfToolbar({
  page, pages, onPageChange,
  zoom, onZoomChange,
  search, onSearchChange,
  mode, onModeChange,
  leftOpen, onLeftOpenChange,
  rightOpen, onRightOpenChange,
  onOpenInWindow,
}: PdfToolbarProps) {
  const { t } = useTranslation();

  return (
    <div style={{
      flexShrink: 0, height: 38, padding: "0 12px",
      borderBottom: "1px solid var(--border)", background: "var(--surface)",
      display: "flex", alignItems: "center", gap: 8,
    }}>
      <IconBtn active={leftOpen} onClick={() => onLeftOpenChange(!leftOpen)} title={t("detail.toolbar.leftToggle")}>
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none">
          <rect x="2" y="3" width="12" height="10" rx="1.5" stroke="currentColor" strokeWidth="1.5"/>
          <path d="M6 3v10" stroke="currentColor" strokeWidth="1.5"/>
        </svg>
      </IconBtn>

      <div style={{ width: 1, height: 18, background: "var(--border)" }} />

      <IconBtn onClick={() => onPageChange(Math.max(1, page - 1))} title={t("detail.toolbar.prevPage")}>
        <Icon name="chevronRight" size={12} color="var(--text-mute)" /* rotated 180 below */ />
      </IconBtn>
      <div style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 12, color: "var(--text)" }}>
        <input
          value={page}
          onChange={(e) => {
            const n = parseInt(e.target.value, 10);
            if (Number.isFinite(n)) onPageChange(Math.max(1, Math.min(pages || 1, n)));
          }}
          style={{
            width: 36, padding: "3px 6px", borderRadius: 4,
            border: "1px solid var(--border-strong)",
            background: "var(--surface-2)", textAlign: "center",
            fontFamily: "var(--mono)", fontSize: 12, color: "var(--text)",
            outline: "none",
          }}
        />
        <span style={{ color: "var(--text-faint)", fontSize: 11.5, fontFamily: "var(--mono)" }}>/ {pages || "—"}</span>
      </div>
      <IconBtn onClick={() => onPageChange(Math.min(pages || 1, page + 1))} title={t("detail.toolbar.nextPage")}>
        <Icon name="chevronRight" size={12} color="var(--text-mute)" />
      </IconBtn>

      <div style={{ width: 1, height: 18, background: "var(--border)" }} />

      <IconBtn onClick={() => onZoomChange(Math.max(50, zoom - 10))} title={t("detail.toolbar.zoomOut")}>
        <span style={{ fontSize: 14, lineHeight: 1 }}>−</span>
      </IconBtn>
      <div style={{
        fontSize: 11.5, color: "var(--text)", minWidth: 42, textAlign: "center",
        fontFamily: "var(--mono)", fontVariantNumeric: "tabular-nums",
      }}>{zoom}%</div>
      <IconBtn onClick={() => onZoomChange(Math.min(200, zoom + 10))} title={t("detail.toolbar.zoomIn")}>
        <Icon name="plus" size={12} color="var(--text-mute)" />
      </IconBtn>
      <IconBtn onClick={() => onZoomChange(100)} title={t("detail.toolbar.fit")}>
        <span style={{ fontSize: 10, color: "var(--text-mute)" }}>⛶</span>
      </IconBtn>

      <div style={{ width: 1, height: 18, background: "var(--border)" }} />

      <div style={{
        display: "inline-flex", padding: 1, gap: 1,
        background: "var(--surface-2)", borderRadius: 5,
        border: "1px solid var(--border)",
      }}>
        {MODES.map(m => {
          const active = mode === m.id;
          return (
            <button
              key={m.id}
              onClick={() => onModeChange(m.id)}
              title={t(m.titleKey)}
              style={{
                width: 26, height: 22, padding: 0, border: "none",
                background: active ? "var(--surface)" : "transparent",
                boxShadow: active ? "0 0 0 1px var(--border) inset" : "none",
                borderRadius: 3, cursor: "pointer",
                display: "inline-flex", alignItems: "center", justifyContent: "center",
                color: active ? "var(--text)" : "var(--text-mute)",
              }}
            >
              {m.iconName ? (
                <Icon name={m.iconName} size={12} color={active ? "var(--text)" : "var(--text-mute)"} />
              ) : (
                <span style={{ fontSize: 11 }}>↖</span>
              )}
            </button>
          );
        })}
      </div>

      <div style={{ flex: 1 }} />

      <div style={{
        display: "flex", alignItems: "center", gap: 5,
        width: 200, height: 26, padding: "0 8px",
        background: "var(--surface-2)", border: "1px solid var(--border)",
        borderRadius: 5,
      }}>
        <Icon name="search" size={11} color="var(--text-faint)" />
        <input
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder={t("detail.toolbar.searchPlaceholder")}
          style={{
            flex: 1, border: "none", background: "transparent", outline: "none",
            fontSize: 12, color: "var(--text)",
          }}
        />
      </div>

      {onOpenInWindow && (
        <IconBtn onClick={onOpenInWindow} title={t("detail.toolbar.openInWindow")}>
          <Icon name="ext" size={12} color="var(--text-mute)" />
        </IconBtn>
      )}

      <IconBtn active={rightOpen} onClick={() => onRightOpenChange(!rightOpen)} title={t("detail.toolbar.rightToggle")}>
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none">
          <rect x="2" y="3" width="12" height="10" rx="1.5" stroke="currentColor" strokeWidth="1.5"/>
          <path d="M10 3v10" stroke="currentColor" strokeWidth="1.5"/>
        </svg>
      </IconBtn>
    </div>
  );
}
