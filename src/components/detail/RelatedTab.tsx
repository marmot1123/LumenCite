import { useTranslation } from "react-i18next";
import type { EntryDetail } from "../../types";

interface RelatedTabProps {
  entry: EntryDetail;
  onSelectEntry: (id: number) => void;
}

type RelLabelKey =
  | "detail.related.preprintOf"
  | "detail.related.versionOf"
  | "detail.related.supplementOf";

const REL_LABEL_KEYS: Record<string, RelLabelKey> = {
  preprint_of: "detail.related.preprintOf",
  version_of: "detail.related.versionOf",
  supplement_of: "detail.related.supplementOf",
};

export function RelatedTab({ entry, onSelectEntry }: RelatedTabProps) {
  const { t } = useTranslation();
  if (entry.relations.length === 0) {
    return (
      <div style={{ fontSize: 12, color: "var(--text-mute)", lineHeight: 1.55 }}>
        {t("detail.related.empty")}
      </div>
    );
  }
  return (
    <div>
      {entry.relations.map((rel, i) => {
        const labelKey = REL_LABEL_KEYS[rel.relation_type];
        const label: string = labelKey ? t(labelKey) : rel.relation_type;
        return (
          <div
            key={i}
            onClick={() => onSelectEntry(rel.entry.id)}
            style={{
              padding: "8px 10px", marginBottom: 6,
              borderRadius: 5, border: "1px solid var(--border)",
              background: "var(--surface-2)", cursor: "pointer",
            }}
          >
            <div style={{
              fontSize: 9.5, color: "var(--accent-strong)", fontWeight: 600,
              textTransform: "uppercase", letterSpacing: "0.04em", marginBottom: 3,
            }}>{label}</div>
            <div style={{
              fontSize: 11.5, color: "var(--text)", fontWeight: 500,
              lineHeight: 1.35,
            }}>{rel.entry.title}</div>
            {rel.entry.year && (
              <div style={{
                fontSize: 10.5, color: "var(--text-faint)",
                marginTop: 2, fontFamily: "var(--mono)",
              }}>{rel.entry.year}</div>
            )}
          </div>
        );
      })}
    </div>
  );
}
