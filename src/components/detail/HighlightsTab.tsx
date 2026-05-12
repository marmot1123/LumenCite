import { useTranslation } from "react-i18next";
import type { Highlight, HighlightColor } from "../../types";

interface HighlightsTabProps {
  highlights: Highlight[];
  onJumpToPage: (page: number) => void;
  onDelete: (id: number) => void;
}

const COLOR_CHIPS: Record<HighlightColor, string> = {
  yellow: "oklch(0.85 0.15 95)",
  green:  "oklch(0.78 0.13 145)",
  blue:   "oklch(0.7 0.13 240)",
};

export function HighlightsTab({ highlights, onJumpToPage, onDelete }: HighlightsTabProps) {
  const { t } = useTranslation();

  if (highlights.length === 0) {
    return (
      <div style={{ fontSize: 12, color: "var(--text-mute)", lineHeight: 1.55 }}>
        {t("detail.highlights.empty")}
      </div>
    );
  }

  return (
    <>
      <div style={{ fontSize: 11, color: "var(--text-faint)", marginBottom: 10 }}>
        {t("detail.highlights.count", { count: highlights.length })}
      </div>
      {highlights.map(h => (
        <div
          key={h.id}
          onClick={() => onJumpToPage(h.page)}
          style={{
            marginBottom: 10, padding: "10px 11px",
            borderRadius: 6, border: "1px solid var(--border)",
            background: "var(--surface-2)", cursor: "pointer",
          }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 6 }}>
            <span style={{
              width: 4, height: 14, borderRadius: 2,
              background: COLOR_CHIPS[h.color],
            }} />
            <span style={{ fontSize: 10.5, color: "var(--text-faint)", fontFamily: "var(--mono)" }}>
              p.{h.page}
            </span>
            <div style={{ flex: 1 }} />
            <button
              onClick={(e) => {
                e.stopPropagation();
                if (window.confirm(t("detail.highlights.deleteConfirm"))) onDelete(h.id);
              }}
              aria-label={t("detail.highlights.delete")}
              style={{
                width: 18, height: 18, padding: 0, border: "none",
                background: "transparent", color: "var(--text-faint)",
                cursor: "pointer", fontSize: 12,
              }}
            >×</button>
          </div>
          <div style={{
            fontSize: 11.5, color: "var(--text)", lineHeight: 1.5,
            fontFamily: "'IBM Plex Serif', Georgia, serif",
          }}>"{h.text}"</div>
          {h.note && (
            <div style={{
              marginTop: 7, paddingTop: 7, borderTop: "1px dashed var(--border)",
              fontSize: 11, color: "var(--text-mute)", lineHeight: 1.5,
            }}>{h.note}</div>
          )}
        </div>
      ))}
    </>
  );
}
