import { useTranslation } from "react-i18next";
import { Icon } from "../icons";
import type { EntryDetail } from "../../types";

interface HeaderProps {
  entry: EntryDetail;
  onBack: () => void;
  onToggleStar: () => void;
  onSummarize?: () => void;
  onOcr?: () => void;
  ocrBusy?: boolean;
  onDownload?: () => void;
  onPrint?: () => void;
  onMore?: () => void;
}

function HeaderBtn({ children, onClick, title, ariaLabel }: {
  children: React.ReactNode;
  onClick?: () => void;
  title?: string;
  ariaLabel?: string;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      aria-label={ariaLabel ?? title}
      style={{
        display: "inline-flex", alignItems: "center", gap: 5,
        padding: "5px 9px", border: "1px solid transparent",
        borderRadius: 5, background: "transparent",
        fontSize: 12, fontWeight: 500, color: "var(--text)",
        cursor: "pointer",
      }}
    >{children}</button>
  );
}

export function Header({ entry, onBack, onToggleStar, onSummarize, onOcr, ocrBusy, onDownload, onPrint, onMore }: HeaderProps) {
  const { t } = useTranslation();
  return (
    <header style={{
      flexShrink: 0, height: 50, padding: "0 14px",
      borderBottom: "1px solid var(--border)", background: "var(--surface)",
      display: "flex", alignItems: "center", gap: 12,
    }}>
      <HeaderBtn onClick={onBack} title={t("detail.back")}>
        <svg width="13" height="13" viewBox="0 0 16 16" fill="none">
          <path d="M9 3L4 8l5 5M4 8h9" stroke="var(--text-mute)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
        <span>{t("detail.back")}</span>
      </HeaderBtn>
      <div style={{ width: 1, height: 18, background: "var(--border)" }} />
      <span style={{
        fontSize: 10.5, fontWeight: 600, padding: "1px 6px",
        borderRadius: 4, background: "var(--surface-2)", color: "var(--text-mute)",
        letterSpacing: "0.04em", textTransform: "uppercase",
      }}>{entry.entry_type}</span>
      <h1 style={{
        margin: 0, fontSize: 14, fontWeight: 600, color: "var(--text)",
        flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
        letterSpacing: "-0.005em",
      }}>{entry.title}</h1>
      <HeaderBtn onClick={onToggleStar} title={entry.starred ? t("detail.header.starOn") : t("detail.header.starOff")}>
        <Icon name={entry.starred ? "starFill" : "star"} size={13}
              color={entry.starred ? "oklch(0.72 0.14 70)" : "var(--text-mute)"} />
      </HeaderBtn>
      <HeaderBtn onClick={onSummarize} title={t("detail.header.summarize")}>
        <Icon name="sparkle" size={13} color="var(--text-mute)" />
        <span>{t("detail.header.summarize")}</span>
      </HeaderBtn>
      {onOcr && (
        <HeaderBtn onClick={ocrBusy ? undefined : onOcr} title={ocrBusy ? t("detail.header.ocrRunning") : t("detail.header.ocr")}>
          <Icon name="search" size={13} color="var(--text-mute)" />
          <span>{ocrBusy ? t("detail.header.ocrRunning") : t("detail.header.ocr")}</span>
        </HeaderBtn>
      )}
      <HeaderBtn onClick={onDownload} title={t("detail.header.download")}>
        <Icon name="download" size={13} color="var(--text-mute)" />
      </HeaderBtn>
      {onPrint && (
        <HeaderBtn onClick={onPrint} title={t("detail.header.print")}>
          <Icon name="printer" size={13} color="var(--text-mute)" />
        </HeaderBtn>
      )}
      <HeaderBtn onClick={onMore} title={t("detail.header.more")}>
        <span style={{ fontSize: 14, lineHeight: 1, color: "var(--text-mute)" }}>⋯</span>
      </HeaderBtn>
    </header>
  );
}
