import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { ExtraFieldInputs } from "./ExtraFieldInputs";
import { AuthorEditor } from "./AuthorEditor";
import type { EntryDetail, EntryInput, EntryType } from "../types";

interface EditSheetProps {
  entry: EntryDetail;
  onClose: () => void;
  onSaved: (entry: EntryDetail) => void;
}

const ENTRY_TYPES: EntryType[] = ["article", "book", "inproceedings", "thesis", "webpage", "misc"];

export function EditSheet({ entry, onClose, onSaved }: EditSheetProps) {
  const { t } = useTranslation();
  const [form, setForm] = useState<EntryInput>({
    title:      entry.title,
    entry_type: entry.entry_type,
    year:       entry.year,
    citation_key: entry.citation_key,
    doi:        entry.doi,
    arxiv_id:   entry.arxiv_id,
    isbn:       entry.isbn,
    url:        entry.url,
    abstract_:  entry.abstract_,
    notes:      entry.notes,
    author_names: entry.authors.length > 0 ? entry.authors.map(a => a.name) : [""],
    tag_ids:    entry.tags.map(t => t.id),
    extra_fields: { ...entry.extra_fields },
  });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const [keyAvailable, setKeyAvailable] = useState(true);

  // 著者詳細編集モーダル (AuthorEditor) を開く対象。null なら閉じている。
  // `index` は author_names のどの行に対応するかで、AuthorEditor 保存時に
  // 名前変更を反映する。
  const [editingAuthor, setEditingAuthor] = useState<{ id: number; index: number } | null>(null);
  // 編集モーダルから既存著者を引くには id が必要。EditSheet は entry.authors を
  // 持っているので、入力行 i の現在の入力テキストが entry.authors[i].name に一致するなら
  // その id を渡せる、というシンプルな対応を取る。
  const authorIdFor = (i: number): number | null => {
    const name = (form.author_names ?? [])[i]?.trim();
    if (!name) return null;
    const matched = entry.authors.find(a => a.name === name);
    return matched?.id ?? null;
  };

  const set = <K extends keyof EntryInput>(key: K, value: EntryInput[K]) =>
    setForm(f => ({ ...f, [key]: value }));

  // 固定 cite key の重複を 300ms デバウンスで事前チェックする（空なら自動扱いで常に OK）。
  useEffect(() => {
    const key = (form.citation_key ?? "").trim();
    if (!key) { setKeyAvailable(true); return; }
    let cancelled = false;
    const h = setTimeout(async () => {
      try {
        const ok = await invoke<boolean>("is_citation_key_available", { key, excludeId: entry.id });
        if (!cancelled) setKeyAvailable(ok);
      } catch { /* チェック失敗時は保存側の UNIQUE 制約に委ねる */ }
    }, 300);
    return () => { cancelled = true; clearTimeout(h); };
  }, [form.citation_key, entry.id]);

  const setAuthor = (i: number, v: string) => {
    const authors = [...(form.author_names ?? [])];
    authors[i] = v;
    set("author_names", authors);
  };

  const addAuthor = () => set("author_names", [...(form.author_names ?? []), ""]);
  const removeAuthor = (i: number) => {
    const authors = (form.author_names ?? []).filter((_, idx) => idx !== i);
    set("author_names", authors.length ? authors : [""]);
  };

  const handleSave = async () => {
    if (!form.title.trim() || !keyAvailable) return;
    setSaving(true);
    setError("");
    const trimmedExtra: Record<string, string> = {};
    for (const [k, v] of Object.entries(form.extra_fields ?? {})) {
      const t = v?.trim();
      if (t) trimmedExtra[k] = t;
    }
    const input: EntryInput = {
      ...form,
      title:      form.title.trim(),
      author_names: (form.author_names ?? []).map(a => a.trim()).filter(Boolean),
      citation_key: form.citation_key?.trim() || undefined,
      doi:        form.doi?.trim()       || undefined,
      arxiv_id:   form.arxiv_id?.trim()  || undefined,
      isbn:       form.isbn?.trim()      || undefined,
      url:        form.url?.trim()       || undefined,
      abstract_:  form.abstract_?.trim() || undefined,
      notes:      form.notes?.trim()     || undefined,
      extra_fields: trimmedExtra,
    };
    try {
      const updated = await invoke<EntryDetail>("update_entry", { id: entry.id, input });
      onSaved(updated);
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

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

  return (
    <div
      style={{
        position: "absolute", inset: 0, zIndex: 20,
        background: "rgba(20, 18, 14, 0.28)",
        backdropFilter: "blur(2px)",
        display: "flex", alignItems: "flex-start", justifyContent: "center",
        paddingTop: 70,
      }}
      onClick={onClose}
    >
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 480, background: "var(--surface)",
          borderRadius: 10, border: "1px solid var(--border-strong)",
          boxShadow: "0 20px 50px rgba(0,0,0,0.18), 0 1px 0 rgba(0,0,0,0.05)",
          display: "flex", flexDirection: "column", maxHeight: "80vh",
          overflow: "hidden",
        }}
      >
        {/* header */}
        <div style={{ padding: "14px 18px 12px", borderBottom: "1px solid var(--border)", flexShrink: 0 }}>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text)" }}>{t("editSheet.title")}</div>
          <div style={{
            fontSize: 11.5, color: "var(--text-faint)", marginTop: 3,
            overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
          }}>
            {entry.title}
          </div>
        </div>

        {/* form body */}
        <div style={{ padding: "14px 18px 0", overflowY: "auto", flex: 1 }}>
          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>{t("addSheet.field.type")}</label>
            <select value={form.entry_type} onChange={e => set("entry_type", e.target.value as EntryType)}
              style={{ ...fieldStyle, cursor: "pointer" }}>
              {ENTRY_TYPES.map(type => (
                <option key={type} value={type}>{t(`entryTypeLabel.${type}` as const)}</option>
              ))}
            </select>
          </div>

          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>{t("addSheet.field.titleRequired")}</label>
            <input value={form.title} onChange={e => set("title", e.target.value)}
              placeholder={t("addSheet.field.titlePlaceholder")} style={fieldStyle} autoFocus />
          </div>

          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>{t("addSheet.field.authors")}</label>
            {(form.author_names ?? [""]).map((name, i) => {
              const aid = authorIdFor(i);
              return (
                <div key={i} style={{ display: "flex", gap: 6, marginBottom: 5 }}>
                  <input value={name} onChange={e => setAuthor(i, e.target.value)}
                    placeholder={t("addSheet.field.authorPlaceholder", { index: i + 1 })}
                    style={{ ...fieldStyle, flex: 1 }} />
                  {/* 既存著者として DB に存在している行だけ詳細編集ボタンを出す。
                      テキストを変更中だったり、新規に追加した行 (= entry.authors に未登録)
                      では aid が null になり、ボタンは無効化される。 */}
                  <button
                    onClick={() => aid != null && setEditingAuthor({ id: aid, index: i })}
                    disabled={aid == null}
                    title={aid == null ? "" : t("authorEditor.title")}
                    style={{
                      padding: "0 8px", border: "1px solid var(--border-strong)",
                      borderRadius: 5,
                      background: "var(--surface)",
                      color: aid == null ? "var(--text-faint)" : "var(--text-mute)",
                      cursor: aid == null ? "default" : "pointer", fontSize: 13,
                      opacity: aid == null ? 0.4 : 1,
                    }}
                  >…</button>
                  {(form.author_names ?? []).length > 1 && (
                    <button onClick={() => removeAuthor(i)} style={{
                      padding: "0 8px", border: "1px solid var(--border-strong)",
                      borderRadius: 5, background: "var(--surface)", color: "var(--text-mute)",
                      cursor: "pointer", fontSize: 13,
                    }}>×</button>
                  )}
                </div>
              );
            })}
            <button onClick={addAuthor} style={{
              fontSize: 11.5, color: "var(--accent-strong)", border: "none",
              background: "transparent", cursor: "pointer", padding: "2px 0",
            }}>{t("addSheet.field.addAuthor")}</button>
          </div>

          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>{t("addSheet.field.yearPub")}</label>
            <input type="number" min={1000} max={2100}
              value={form.year ?? ""} onChange={e => set("year", e.target.value ? Number(e.target.value) : undefined)}
              placeholder="2024" style={{ ...fieldStyle, width: 120 }} />
          </div>

          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>{t("addSheet.field.citationKey")}</label>
            <input value={form.citation_key ?? ""} onChange={e => set("citation_key", e.target.value || undefined)}
              placeholder={t("addSheet.field.citationKeyPlaceholder")}
              style={{ ...fieldStyle, fontFamily: "var(--mono)",
                ...(keyAvailable ? {} : { borderColor: "var(--danger-text)" }) }} />
            <div style={{
              fontSize: 10.5, marginTop: 4, lineHeight: 1.4,
              color: keyAvailable ? "var(--text-faint)" : "var(--danger-text)",
            }}>
              {keyAvailable ? t("addSheet.field.citationKeyHint") : t("addSheet.field.citationKeyTaken")}
            </div>
          </div>

          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>DOI</label>
            <input value={form.doi ?? ""} onChange={e => set("doi", e.target.value || undefined)}
              placeholder="10.1234/example" style={{ ...fieldStyle, fontFamily: "var(--mono)" }} />
          </div>
          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>arXiv ID</label>
            <input value={form.arxiv_id ?? ""} onChange={e => set("arxiv_id", e.target.value || undefined)}
              placeholder="2301.00001" style={{ ...fieldStyle, fontFamily: "var(--mono)" }} />
          </div>
          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>ISBN</label>
            <input value={form.isbn ?? ""} onChange={e => set("isbn", e.target.value || undefined)}
              placeholder="978-0387310732" style={{ ...fieldStyle, fontFamily: "var(--mono)" }} />
          </div>
          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>URL</label>
            <input value={form.url ?? ""} onChange={e => set("url", e.target.value || undefined)}
              placeholder="https://..." style={{ ...fieldStyle, fontFamily: "var(--mono)" }} />
          </div>

          <ExtraFieldInputs
            entryType={form.entry_type}
            values={form.extra_fields ?? {}}
            onChange={next => set("extra_fields", next)}
          />

          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>{t("addSheet.field.abstract")}</label>
            <textarea value={form.abstract_ ?? ""} onChange={e => set("abstract_", e.target.value || undefined)}
              rows={4} placeholder={t("addSheet.field.abstractPlaceholder")}
              style={{ ...fieldStyle, resize: "vertical", lineHeight: 1.55 }} />
          </div>

          <div style={{ marginBottom: 12 }}>
            <label style={labelStyle}>{t("addSheet.field.notes")}</label>
            <textarea value={form.notes ?? ""} onChange={e => set("notes", e.target.value || undefined)}
              rows={3} placeholder={t("addSheet.field.notesPlaceholder")}
              style={{ ...fieldStyle, resize: "vertical", lineHeight: 1.55 }} />
          </div>

          {error && (
            <div style={{
              marginBottom: 12, padding: "8px 12px", borderRadius: 6,
              background: "var(--danger-bg)", color: "var(--danger-text)",
              fontSize: 12,
            }}>{error}</div>
          )}
        </div>

        {/* footer */}
        <div style={{
          flexShrink: 0,
          display: "flex", justifyContent: "flex-end", gap: 8,
          padding: "12px 18px", background: "var(--surface)",
          borderTop: "1px solid var(--border)",
        }}>
          <button onClick={onClose} style={{
            padding: "5px 12px", borderRadius: 5,
            border: "1px solid var(--border-strong)",
            background: "var(--surface)", color: "var(--text)",
            fontSize: 12, cursor: "pointer",
          }}>{t("common.cancel")}</button>
          <button onClick={handleSave} disabled={!form.title.trim() || saving || !keyAvailable} style={{
            padding: "5px 14px", borderRadius: 5, border: "none",
            background: "var(--accent-strong)", color: "white",
            fontSize: 12, fontWeight: 500, cursor: "pointer",
            opacity: !form.title.trim() || saving || !keyAvailable ? 0.5 : 1,
          }}>{saving ? t("editSheet.submitting") : t("editSheet.submit")}</button>
        </div>
      </div>

      {editingAuthor && (
        <AuthorEditor
          authorId={editingAuthor.id}
          onClose={() => setEditingAuthor(null)}
          onSaved={(updated) => {
            // 著者の表示名が変わった場合だけ、対応する author_names[i] を書き換える。
            // merge 経由（updated === null）では当該 entry の著者紐付け自体は変わらない
            // ので何もしない（次回 entry 再フェッチで反映）。
            if (updated && editingAuthor) {
              const i = editingAuthor.index;
              const next = [...(form.author_names ?? [])];
              next[i] = updated.name;
              set("author_names", next);
            }
          }}
        />
      )}
    </div>
  );
}
