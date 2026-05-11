import { EXTRA_FIELDS_BY_TYPE } from "../types";
import type { EntryType } from "../types";

interface ExtraFieldInputsProps {
  entryType: EntryType;
  values: Record<string, string>;
  onChange: (next: Record<string, string>) => void;
}

const fieldStyle: React.CSSProperties = {
  width: "100%", padding: "7px 10px", borderRadius: 5,
  border: "1px solid var(--border-strong)",
  background: "var(--surface-2)", color: "var(--text)",
  fontSize: 12.5, outline: "none", boxSizing: "border-box",
};
const labelStyle: React.CSSProperties = {
  fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
  textTransform: "uppercase", letterSpacing: "0.06em",
  display: "block", marginBottom: 4,
};

// entry_type が切り替わったとき、新しい型に存在しないフィールドは
// 値が入っていれば残す（誤って消えないように）。空なら捨てる。
export function ExtraFieldInputs({ entryType, values, onChange }: ExtraFieldInputsProps) {
  const defs = EXTRA_FIELDS_BY_TYPE[entryType] ?? [];
  const definedKeys = new Set(defs.map(d => d.key));
  const orphanEntries = Object.entries(values).filter(([k, v]) => !definedKeys.has(k) && v?.trim());

  const setField = (key: string, value: string) => {
    const next = { ...values };
    if (value.trim()) next[key] = value;
    else delete next[key];
    onChange(next);
  };

  return (
    <>
      {defs.map(def => (
        <div key={def.key} style={{ marginBottom: 12 }}>
          <label style={labelStyle}>{def.label}</label>
          <input
            value={values[def.key] ?? ""}
            onChange={e => setField(def.key, e.target.value)}
            placeholder={def.placeholder}
            style={{ ...fieldStyle, fontFamily: def.mono ? "var(--mono)" : undefined }}
          />
        </div>
      ))}
      {orphanEntries.length > 0 && (
        <div style={{
          marginBottom: 12, padding: "8px 10px",
          borderRadius: 6, border: "1px dashed var(--border-strong)",
          background: "var(--surface-2)",
        }}>
          <div style={{ ...labelStyle, marginBottom: 6 }}>その他のフィールド</div>
          {orphanEntries.map(([k, v]) => (
            <div key={k} style={{ display: "flex", gap: 6, marginBottom: 5, alignItems: "center" }}>
              <span style={{
                width: 110, fontSize: 11.5, color: "var(--text-mute)",
                fontFamily: "var(--mono)", flexShrink: 0,
              }}>{k}</span>
              <input
                value={v}
                onChange={e => setField(k, e.target.value)}
                style={{ ...fieldStyle, flex: 1 }}
              />
            </div>
          ))}
          <div style={{ fontSize: 10.5, color: "var(--text-faint)", marginTop: 2 }}>
            この種別では通常使われないフィールドです（BibTeX 取り込み時等に保存されたもの）
          </div>
        </div>
      )}
    </>
  );
}
