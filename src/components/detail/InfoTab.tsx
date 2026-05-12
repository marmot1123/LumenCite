import { useTranslation } from "react-i18next";
import { TagPill } from "../TagPill";
import { Icon } from "../icons";
import { MathMarkdown } from "../MathMarkdown";
import type { EntryDetail } from "../../types";

interface InfoTabProps {
  entry: EntryDetail;
}

function Field({ label, value, mono }: { label: string; value?: string | number | null; mono?: boolean }) {
  if (value === null || value === undefined || value === "") return null;
  return (
    <div style={{ marginBottom: 12 }}>
      <div style={{
        fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
        textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 3,
      }}>{label}</div>
      <div style={{
        fontSize: 12.5, color: "var(--text)",
        fontFamily: mono ? "var(--mono)" : "inherit",
        lineHeight: 1.45, wordBreak: "break-word",
      }}>{value}</div>
    </div>
  );
}

export function InfoTab({ entry }: InfoTabProps) {
  const { t } = useTranslation();
  const authors = entry.authors.map(a => a.name).join(", ");
  const venue = entry.extra_fields?.journal ?? entry.extra_fields?.booktitle ?? null;
  const venueLine = venue && entry.year
    ? t("detail.info.venueYear", { venue, year: entry.year })
    : venue || (entry.year ? String(entry.year) : null);

  return (
    <div>
      <h3 style={{
        margin: 0, fontSize: 13.5, fontWeight: 600, lineHeight: 1.35,
        color: "var(--text)", letterSpacing: "-0.005em",
      }}>{entry.title}</h3>
      {authors && (
        <div style={{ marginTop: 8, fontSize: 11.5, color: "var(--text-mute)", lineHeight: 1.55 }}>
          {authors}
        </div>
      )}
      {venueLine && (
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-faint)", fontStyle: "italic" }}>
          {venueLine}
        </div>
      )}

      <div style={{ height: 14 }} />
      <Field label={t("detail.info.doi")} value={entry.doi} mono />
      <Field label={t("detail.info.arxiv")} value={entry.arxiv_id} mono />
      <Field label={t("detail.info.isbn")} value={entry.isbn} mono />
      <Field label={t("detail.info.url")} value={entry.url} mono />

      <div style={{ marginBottom: 12 }}>
        <div style={{
          fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
          textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 3,
        }}>{t("detail.info.abstract")}</div>
        <div style={{ fontSize: 12.5, color: "var(--text)", lineHeight: 1.55 }}>
          <MathMarkdown
            value={entry.abstract_}
            fallback={<span style={{ color: "var(--text-faint)" }}>{t("detail.info.noAbstract")}</span>}
          />
        </div>
      </div>

      {entry.tags.length > 0 && (
        <div style={{ marginBottom: 12 }}>
          <div style={{
            fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
            textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
          }}>{t("detail.info.tags")}</div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
            {entry.tags.map(tag => <TagPill key={tag.id} name={tag.name} />)}
          </div>
        </div>
      )}

      {entry.collections.length > 0 && (
        <div style={{ marginBottom: 12 }}>
          <div style={{
            fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
            textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
          }}>{t("detail.info.collections")}</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            {entry.collections.map(col => (
              <div key={col.id} style={{
                display: "inline-flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text)",
              }}>
                <Icon name="folder" size={12} color="var(--text-mute)" />
                {col.name}
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
