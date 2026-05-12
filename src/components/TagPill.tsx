import { useTranslation } from "react-i18next";

const TAG_COLOR_LIST = ["amber", "blue", "green", "violet", "rose", "cyan"] as const;
type TagColorKey = typeof TAG_COLOR_LIST[number] | "neutral";

const TAG_COLORS: Record<TagColorKey, { bg: string; fg: string; dot: string }> = {
  amber:   { bg: "oklch(0.95 0.05 75)",  fg: "oklch(0.42 0.12 65)",  dot: "oklch(0.7  0.13 70)" },
  blue:    { bg: "oklch(0.95 0.04 240)", fg: "oklch(0.42 0.12 245)", dot: "oklch(0.6  0.13 245)" },
  green:   { bg: "oklch(0.95 0.04 150)", fg: "oklch(0.4  0.10 150)", dot: "oklch(0.62 0.12 150)" },
  violet:  { bg: "oklch(0.95 0.04 295)", fg: "oklch(0.42 0.12 295)", dot: "oklch(0.6  0.13 295)" },
  rose:    { bg: "oklch(0.95 0.04 15)",  fg: "oklch(0.45 0.13 15)",  dot: "oklch(0.65 0.15 15)" },
  cyan:    { bg: "oklch(0.95 0.04 200)", fg: "oklch(0.42 0.10 210)", dot: "oklch(0.6  0.12 210)" },
  neutral: { bg: "var(--surface-2)",     fg: "var(--text-mute)",     dot: "var(--text-faint)" },
};

export function tagColorForName(name: string): typeof TAG_COLORS.amber {
  let hash = 0;
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) >>> 0;
  const key = TAG_COLOR_LIST[hash % TAG_COLOR_LIST.length];
  return TAG_COLORS[key];
}

export function TagPill({ name, onRemove }: { name: string; onRemove?: () => void }) {
  const { t } = useTranslation();
  const c = tagColorForName(name);
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 4,
      padding: onRemove ? "1px 3px 1px 6px" : "1px 7px 1px 6px",
      borderRadius: 999,
      background: c.bg, color: c.fg,
      fontSize: 10.5, fontWeight: 500, letterSpacing: "0.01em",
      whiteSpace: "nowrap",
    }}>
      <span style={{ width: 5, height: 5, borderRadius: 999, background: c.dot, flexShrink: 0 }} />
      {name}
      {onRemove && (
        <button
          onClick={onRemove}
          title={t("tag.removeTitle")}
          style={{
            width: 13, height: 13, padding: 0, marginLeft: 1,
            border: "none", background: "transparent", cursor: "pointer",
            display: "inline-flex", alignItems: "center", justifyContent: "center",
            borderRadius: 999, color: c.fg, opacity: 0.6,
            fontSize: 12, lineHeight: 1,
          }}
          onMouseEnter={e => (e.currentTarget.style.opacity = "1")}
          onMouseLeave={e => (e.currentTarget.style.opacity = "0.6")}
        >×</button>
      )}
    </span>
  );
}
