import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { tagColorForName } from "./TagPill";
import { ENTRY_TYPES, isFilterActive } from "../types";
import type { EntryFilter, EntryType, Tag, TagMatch } from "../types";

interface FilterPanelProps {
  filter: EntryFilter;
  onChange: (f: EntryFilter) => void;
  onClear: () => void;
  tags: Tag[];
  onClose: () => void;
}

/** セクション見出し。 */
function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ padding: "10px 12px", borderBottom: "1px solid var(--border)" }}>
      <div style={{
        fontSize: 10.5, fontWeight: 600, letterSpacing: "0.04em",
        textTransform: "uppercase", color: "var(--text-faint)", marginBottom: 7,
      }}>{title}</div>
      {children}
    </div>
  );
}

/** any / true / false の三値トグル（スター・添付 用）。 */
function TriToggle({ value, onChange, labels }: {
  value: boolean | undefined;
  onChange: (v: boolean | undefined) => void;
  labels: { any: string; yes: string; no: string };
}) {
  const opts: { v: boolean | undefined; label: string }[] = [
    { v: undefined, label: labels.any },
    { v: true, label: labels.yes },
    { v: false, label: labels.no },
  ];
  return (
    <div style={{
      display: "inline-flex", padding: 2, gap: 0,
      background: "var(--surface-2)", border: "1px solid var(--border)",
      borderRadius: 6,
    }}>
      {opts.map(o => {
        const active = value === o.v;
        return (
          <button key={String(o.v)}
            onClick={() => onChange(o.v)}
            style={{
              padding: "3px 10px", height: 22, border: "none",
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

/** 選択トグル用の小さなチップ。 */
function Chip({ label, active, onClick, color }: {
  label: string; active: boolean; onClick: () => void;
  color?: { bg: string; fg: string; dot: string };
}) {
  return (
    <button
      onClick={onClick}
      style={{
        display: "inline-flex", alignItems: "center", gap: 4,
        padding: color ? "2px 8px 2px 6px" : "3px 9px",
        borderRadius: 999,
        border: active ? "1px solid var(--accent-strong)" : "1px solid var(--border)",
        background: active
          ? (color ? color.bg : "var(--accent-soft, var(--surface-2))")
          : "var(--surface)",
        color: active ? (color ? color.fg : "var(--accent-strong)") : "var(--text-mute)",
        fontSize: 11, fontWeight: active ? 600 : 500,
        cursor: "pointer", whiteSpace: "nowrap",
        boxShadow: active ? "inset 0 0 0 1px var(--accent-strong)" : "none",
      }}>
      {color && <span style={{ width: 5, height: 5, borderRadius: 999, background: color.dot, flexShrink: 0 }} />}
      {label}
    </button>
  );
}

export function FilterPanel({ filter, onChange, onClear, tags, onClose }: FilterPanelProps) {
  const { t } = useTranslation();
  const ref = useRef<HTMLDivElement>(null);

  // パネル外クリック・Esc で閉じる。
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } };
    // capture フェーズで拾い、他のグローバル Esc ハンドラより先に閉じる
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [onClose]);

  const toggleType = (ty: EntryType) => {
    const has = filter.entry_types.includes(ty);
    onChange({
      ...filter,
      entry_types: has ? filter.entry_types.filter(x => x !== ty) : [...filter.entry_types, ty],
    });
  };

  const toggleTag = (id: number) => {
    const has = filter.tag_ids.includes(id);
    onChange({
      ...filter,
      tag_ids: has ? filter.tag_ids.filter(x => x !== id) : [...filter.tag_ids, id],
    });
  };

  const setYear = (key: "year_min" | "year_max", raw: string) => {
    const n = parseInt(raw, 10);
    onChange({ ...filter, [key]: Number.isFinite(n) ? n : undefined });
  };

  const active = isFilterActive(filter);

  return (
    <div
      ref={ref}
      style={{
        position: "absolute", top: "calc(100% + 4px)", left: 0, zIndex: 200,
        width: 320, maxHeight: "70vh", overflowY: "auto",
        background: "var(--surface)",
        border: "1px solid var(--border-strong)",
        borderRadius: 10,
        boxShadow: "0 12px 32px rgba(0,0,0,0.18)",
      }}
    >
      {/* header */}
      <div style={{
        display: "flex", alignItems: "center", justifyContent: "space-between",
        padding: "9px 12px", borderBottom: "1px solid var(--border)",
      }}>
        <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text)" }}>
          {t("filter.title")}
        </span>
        <button
          onClick={onClear}
          disabled={!active}
          style={{
            padding: "3px 8px", borderRadius: 5,
            border: "1px solid var(--border)",
            background: "transparent",
            color: active ? "var(--text-mute)" : "var(--text-faint)",
            fontSize: 11, fontWeight: 500,
            cursor: active ? "pointer" : "not-allowed",
          }}
        >{t("filter.clearAll")}</button>
      </div>

      {/* entry types */}
      <Section title={t("filter.entryType")}>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
          {ENTRY_TYPES.map(ty => (
            <Chip key={ty}
              label={t(`entryType.${ty}`)}
              active={filter.entry_types.includes(ty)}
              onClick={() => toggleType(ty)}
            />
          ))}
        </div>
      </Section>

      {/* year range */}
      <Section title={t("filter.year")}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <input
            type="number" inputMode="numeric"
            value={filter.year_min ?? ""}
            onChange={e => setYear("year_min", e.target.value)}
            placeholder={t("filter.yearFrom")}
            style={yearInputStyle}
          />
          <span style={{ color: "var(--text-faint)", fontSize: 12 }}>–</span>
          <input
            type="number" inputMode="numeric"
            value={filter.year_max ?? ""}
            onChange={e => setYear("year_max", e.target.value)}
            placeholder={t("filter.yearTo")}
            style={yearInputStyle}
          />
        </div>
      </Section>

      {/* starred */}
      <Section title={t("filter.starred")}>
        <TriToggle
          value={filter.starred}
          onChange={v => onChange({ ...filter, starred: v })}
          labels={{ any: t("filter.any"), yes: t("filter.starredYes"), no: t("filter.starredNo") }}
        />
      </Section>

      {/* attachment */}
      <Section title={t("filter.attachment")}>
        <TriToggle
          value={filter.has_attachment}
          onChange={v => onChange({ ...filter, has_attachment: v })}
          labels={{ any: t("filter.any"), yes: t("filter.attachmentYes"), no: t("filter.attachmentNo") }}
        />
      </Section>

      {/* tags */}
      <Section title={t("filter.tags")}>
        {tags.length === 0 ? (
          <div style={{ fontSize: 11, color: "var(--text-faint)" }}>{t("filter.noTags")}</div>
        ) : (
          <>
            <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 8 }}>
              <span style={{ fontSize: 11, color: "var(--text-mute)" }}>{t("filter.tagMatch")}</span>
              <div style={{
                display: "inline-flex", padding: 2,
                background: "var(--surface-2)", border: "1px solid var(--border)", borderRadius: 6,
              }}>
                {(["or", "and"] as TagMatch[]).map(m => {
                  const on = filter.tag_match === m;
                  return (
                    <button key={m}
                      onClick={() => onChange({ ...filter, tag_match: m })}
                      style={{
                        padding: "2px 9px", height: 20, border: "none",
                        background: on ? "var(--surface)" : "transparent",
                        color: on ? "var(--text)" : "var(--text-mute)",
                        fontSize: 10.5, fontWeight: on ? 600 : 500,
                        borderRadius: 4, cursor: "pointer",
                      }}>
                      {t(`filter.tagMatch_${m}`)}
                    </button>
                  );
                })}
              </div>
            </div>
            <div style={{ display: "flex", flexWrap: "wrap", gap: 5 }}>
              {tags.map(tag => (
                <Chip key={tag.id}
                  label={tag.name}
                  active={filter.tag_ids.includes(tag.id)}
                  onClick={() => toggleTag(tag.id)}
                  color={tagColorForName(tag.name)}
                />
              ))}
            </div>
          </>
        )}
      </Section>
    </div>
  );
}

const yearInputStyle: React.CSSProperties = {
  width: 90, padding: "4px 8px", height: 26,
  border: "1px solid var(--border)", borderRadius: 6,
  background: "var(--surface-2)", color: "var(--text)",
  fontSize: 12, outline: "none",
  fontVariantNumeric: "tabular-nums",
};
