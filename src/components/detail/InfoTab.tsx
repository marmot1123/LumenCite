import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { TagPill } from "../TagPill";
import { Icon } from "../icons";
import { MathMarkdown } from "../MathMarkdown";
import type { EntryDetail } from "../../types";

interface InfoTabProps {
  entry: EntryDetail;
}

/** .bib 同期で実際に割り当てられる cite key を表示し、\cite{} 用にコピーできる行。 */
function CitationKeyField({ entry }: { entry: EntryDetail }) {
  const { t } = useTranslation();
  const [resolved, setResolved] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<string>("resolve_citation_key", { entryId: entry.id })
      .then(k => { if (!cancelled) setResolved(k); })
      .catch(() => { if (!cancelled) setResolved(null); });
    return () => { cancelled = true; };
    // citation_key 変更（ピン留め/解除）後も再解決する
  }, [entry.id, entry.citation_key]);

  if (!resolved) return null;

  const copy = async () => {
    try {
      await writeText(resolved);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch { /* クリップボード不可時は何もしない */ }
  };

  return (
    <div style={{ marginBottom: 12 }}>
      <div style={{
        fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
        textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 3,
      }}>{t("detail.info.citationKey")}</div>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <code style={{
          fontSize: 12.5, fontFamily: "var(--mono)", color: "var(--text)",
          background: "var(--surface-2)", border: "1px solid var(--border)",
          borderRadius: 4, padding: "2px 7px", wordBreak: "break-all",
        }}>{resolved}</code>
        <button onClick={copy} title={t("detail.info.citationKeyCopy")} style={{
          fontSize: 11, color: copied ? "var(--text-mute)" : "var(--accent-strong)",
          border: "none", background: "transparent", cursor: "pointer",
          padding: "2px 4px", whiteSpace: "nowrap",
        }}>{copied ? t("detail.info.citationKeyCopied") : t("detail.info.citationKeyCopy")}</button>
      </div>
      {!entry.citation_key && (
        <div style={{ fontSize: 10, color: "var(--text-faint)", marginTop: 3 }}>
          {t("detail.info.citationKeyAuto")}
        </div>
      )}
    </div>
  );
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
      <CitationKeyField entry={entry} />
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
