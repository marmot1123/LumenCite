import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { MathMarkdown } from "../MathMarkdown";
import type { EntryDetail } from "../../types";

interface NotesTabProps {
  entry: EntryDetail;
  onUpdate: (notes: string) => void;
}

export function NotesTab({ entry, onUpdate }: NotesTabProps) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(entry.notes ?? "");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setValue(entry.notes ?? "");
  }, [entry.id, entry.notes]);

  useEffect(() => {
    if (editing) textareaRef.current?.focus();
  }, [editing]);

  if (editing) {
    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <textarea
          ref={textareaRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onBlur={() => {
            setEditing(false);
            if (value !== (entry.notes ?? "")) onUpdate(value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setValue(entry.notes ?? "");
              setEditing(false);
            }
          }}
          placeholder={t("detail.notes.placeholder")}
          rows={12}
          style={{
            width: "100%", boxSizing: "border-box",
            padding: 10, borderRadius: 6,
            border: "1px solid var(--border-strong)",
            background: "var(--surface)", color: "var(--text)",
            fontSize: 12.5, lineHeight: 1.55, resize: "vertical",
            fontFamily: "inherit", outline: "none",
          }}
        />
      </div>
    );
  }

  if (!entry.notes) {
    return (
      <div>
        <div style={{ fontSize: 12, color: "var(--text-faint)", marginBottom: 10 }}>
          {t("detail.notes.empty")}
        </div>
        <button
          onClick={() => setEditing(true)}
          style={{
            padding: "5px 11px", borderRadius: 5,
            border: "1px solid var(--border-strong)", background: "var(--surface)",
            color: "var(--text)", fontSize: 11.5, cursor: "pointer",
          }}
        >{t("detail.notes.edit")}</button>
      </div>
    );
  }

  return (
    <div>
      <div
        onClick={() => setEditing(true)}
        style={{
          fontSize: 12.5, lineHeight: 1.65, color: "var(--text)",
          padding: "8px 0", cursor: "text",
        }}
      >
        <MathMarkdown value={entry.notes} />
      </div>
      <button
        onClick={() => setEditing(true)}
        style={{
          marginTop: 12, padding: "5px 11px", borderRadius: 5,
          border: "1px solid var(--border-strong)", background: "var(--surface)",
          color: "var(--text)", fontSize: 11.5, cursor: "pointer",
        }}
      >{t("detail.notes.edit")}</button>
    </div>
  );
}
