import { useTranslation } from "react-i18next";

interface StatusBarProps {
  total: number;
  filtered: number;
  selectedId: number | null;
  indexingCount?: number;
}

export function StatusBar({ total, filtered, selectedId, indexingCount = 0 }: StatusBarProps) {
  const { t } = useTranslation();
  const selectionState = selectedId != null ? t("statusBar.selectionOne") : t("statusBar.selectionNone");
  return (
    <div style={{
      flexShrink: 0, borderTop: "1px solid var(--border)",
      background: "var(--surface-2)",
      height: 24, padding: "0 14px",
      display: "flex", alignItems: "center", gap: 14,
      fontSize: 11, color: "var(--text-faint)",
    }}>
      <span style={{ fontVariantNumeric: "tabular-nums" }}>
        {t("statusBar.entriesCount", { filtered, total })}
      </span>
      <span style={{ width: 1, height: 10, background: "var(--border)", flexShrink: 0 }} />
      <span>{t("statusBar.selectionLabel", { state: selectionState })}</span>
      {indexingCount > 0 && (
        <>
          <span style={{ width: 1, height: 10, background: "var(--border)", flexShrink: 0 }} />
          <span style={{ display: "inline-flex", alignItems: "center", gap: 5, color: "var(--text-mute)" }}>
            <span style={{
              width: 8, height: 8, borderRadius: "50%",
              background: "oklch(0.7 0.15 90)",
              animation: "pulse 1.4s ease-in-out infinite",
            }} />
            {t("statusBar.indexingPdf", { count: indexingCount })}
          </span>
        </>
      )}
      <div style={{ flex: 1 }} />
      <span style={{ fontFamily: "var(--mono)" }}>SQLite · {total} entries</span>
    </div>
  );
}
