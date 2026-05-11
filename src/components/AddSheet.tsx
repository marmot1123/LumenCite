import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "./icons";
import { ExtraFieldInputs } from "./ExtraFieldInputs";
import type { EntryDetail, EntryInput, EntryType } from "../types";

interface AddSheetProps {
  onClose: () => void;
  onCreated: (entry: EntryDetail) => void;
  onImported: () => void;
}

type AddTab = "doi" | "arxiv" | "isbn" | "bibtex" | "manual";
type Phase = "input" | "loading" | "preview" | "error" | "saving";

const ENTRY_TYPES: { value: EntryType; label: string }[] = [
  { value: "article",        label: "論文（article）" },
  { value: "book",           label: "書籍（book）" },
  { value: "inproceedings",  label: "会議録（inproceedings）" },
  { value: "thesis",         label: "学位論文（thesis）" },
  { value: "webpage",        label: "Webページ（webpage）" },
  { value: "misc",           label: "その他（misc）" },
];

const FETCH_COMMANDS: Record<string, string> = {
  doi:   "fetch_metadata_by_doi",
  arxiv: "fetch_metadata_by_arxiv",
  isbn:  "fetch_metadata_by_isbn",
};

const FETCH_ARGS: Record<string, string> = {
  doi:   "doi",
  arxiv: "arxivId",   // Tauri converts snake_case params to camelCase
  isbn:  "isbn",
};

const PLACEHOLDERS: Record<AddTab, string> = {
  doi:    "10.48550/arXiv.1706.03762",
  arxiv:  "1706.03762",
  isbn:   "978-0387310732",
  bibtex: "@article{vaswani2017,...",
  manual: "",
};

// ── 識別子タブ（DOI / arXiv / ISBN）──────────────────────────────────────────

function IdentifierTab({ tabId, onCreated, onClose }: {
  tabId: "doi" | "arxiv" | "isbn";
  onCreated: (entry: EntryDetail) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState("");
  const [phase, setPhase] = useState<Phase>("input");
  const [preview, setPreview] = useState<EntryInput | null>(null);
  const [error, setError] = useState("");

  const handleFetch = async () => {
    if (!value.trim()) return;
    setPhase("loading");
    setError("");
    try {
      const data = await invoke<EntryInput>(FETCH_COMMANDS[tabId], {
        [FETCH_ARGS[tabId]]: value.trim(),
      });
      setPreview(data);
      setPhase("preview");
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  };

  const handleAdd = async () => {
    if (!preview) return;
    setPhase("saving");
    try {
      const entry = await invoke<EntryDetail>("create_entry", { input: preview });
      onCreated(entry);
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  };

  return (
    <div style={{ padding: 18 }}>
      <div style={{ display: "flex", gap: 8 }}>
        <input
          value={value}
          onChange={e => setValue(e.target.value)}
          onKeyDown={e => e.key === "Enter" && handleFetch()}
          placeholder={PLACEHOLDERS[tabId]}
          autoFocus
          disabled={phase === "loading" || phase === "saving"}
          style={{
            flex: 1, padding: "8px 10px", borderRadius: 6,
            border: "1px solid var(--border-strong)",
            fontFamily: "var(--mono)", fontSize: 12.5,
            color: "var(--text)", background: "var(--surface-2)",
            outline: "none",
          }}
        />
        <button
          onClick={handleFetch}
          disabled={!value.trim() || phase === "loading" || phase === "saving"}
          style={{
            padding: "8px 14px", borderRadius: 6, border: "none",
            background: "var(--accent-strong)", color: "white",
            fontSize: 12, fontWeight: 500, cursor: "pointer",
            opacity: !value.trim() || phase === "loading" ? 0.6 : 1,
          }}
        >
          {phase === "loading" ? "取得中…" : "取得"}
        </button>
      </div>

      <div style={{ marginTop: 10, fontSize: 11, color: "var(--text-faint)", display: "flex", alignItems: "center", gap: 6 }}>
        <Icon name="info" size={11} color="var(--text-faint)" />
        {tabId === "doi"   && "CrossRef から取得します"}
        {tabId === "arxiv" && "arXiv API から取得します"}
        {tabId === "isbn"  && "Open Library から取得します"}
      </div>

      {phase === "error" && (
        <div style={{
          marginTop: 12, padding: "8px 12px", borderRadius: 6,
          background: "oklch(0.95 0.04 15)", color: "oklch(0.45 0.13 15)",
          fontSize: 12,
        }}>{error}</div>
      )}

      {phase === "preview" && preview && (
        <div style={{
          marginTop: 14, padding: "12px 14px", borderRadius: 8,
          background: "var(--surface-2)", border: "1px solid var(--border)",
        }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text)", lineHeight: 1.35 }}>
            {preview.title}
          </div>
          {preview.author_names && preview.author_names.length > 0 && (
            <div style={{ marginTop: 4, fontSize: 12, color: "var(--text-mute)" }}>
              {preview.author_names.slice(0, 3).join(", ")}
              {preview.author_names.length > 3 ? ` 他${preview.author_names.length - 3}名` : ""}
            </div>
          )}
          <div style={{ marginTop: 4, fontSize: 11.5, color: "var(--text-faint)", display: "flex", gap: 10 }}>
            {preview.year && <span>{preview.year}</span>}
            <span style={{ fontFamily: "var(--mono)" }}>
              {preview.doi ?? preview.arxiv_id ?? preview.isbn ?? ""}
            </span>
          </div>
        </div>
      )}

      <div style={{
        marginTop: 16, display: "flex", justifyContent: "flex-end", gap: 8,
        paddingTop: 14, borderTop: "1px solid var(--border)",
      }}>
        <button onClick={onClose} style={{
          padding: "5px 12px", borderRadius: 5,
          border: "1px solid var(--border-strong)",
          background: "var(--surface)", color: "var(--text)",
          fontSize: 12, cursor: "pointer",
        }}>キャンセル</button>
        <button
          onClick={handleAdd}
          disabled={phase !== "preview" || !preview}
          style={{
            padding: "5px 14px", borderRadius: 5, border: "none",
            background: "var(--accent-strong)", color: "white",
            fontSize: 12, fontWeight: 500, cursor: "pointer",
            opacity: phase !== "preview" ? 0.4 : 1,
          }}
        >
          {phase === "saving" ? "追加中…" : "ライブラリに追加"}
        </button>
      </div>
    </div>
  );
}

// ── 手動入力タブ ──────────────────────────────────────────────────────────────

function ManualTab({ onCreated, onClose }: {
  onCreated: (entry: EntryDetail) => void;
  onClose: () => void;
}) {
  const [form, setForm] = useState<EntryInput>({
    title: "",
    entry_type: "article",
    author_names: [""],
    year: undefined,
    doi: undefined,
    arxiv_id: undefined,
    isbn: undefined,
    url: undefined,
    abstract_: undefined,
    notes: undefined,
    extra_fields: {},
  });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  const set = <K extends keyof EntryInput>(key: K, value: EntryInput[K]) =>
    setForm(f => ({ ...f, [key]: value }));

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

  const handleAdd = async () => {
    if (!form.title.trim()) return;
    setSaving(true);
    setError("");
    const trimmedExtra: Record<string, string> = {};
    for (const [k, v] of Object.entries(form.extra_fields ?? {})) {
      const t = v?.trim();
      if (t) trimmedExtra[k] = t;
    }
    const input: EntryInput = {
      ...form,
      title: form.title.trim(),
      author_names: (form.author_names ?? []).map(a => a.trim()).filter(Boolean),
      doi:      form.doi?.trim()      || undefined,
      arxiv_id: form.arxiv_id?.trim() || undefined,
      isbn:     form.isbn?.trim()     || undefined,
      url:      form.url?.trim()      || undefined,
      abstract_: form.abstract_?.trim() || undefined,
      notes:    form.notes?.trim()    || undefined,
      extra_fields: trimmedExtra,
    };
    try {
      const entry = await invoke<EntryDetail>("create_entry", { input });
      onCreated(entry);
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
    <div style={{ padding: "14px 18px 0", maxHeight: 460, overflowY: "auto" }}>
      {/* entry_type */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>種別</label>
        <select value={form.entry_type} onChange={e => set("entry_type", e.target.value as EntryType)}
          style={{ ...fieldStyle, cursor: "pointer" }}>
          {ENTRY_TYPES.map(t => <option key={t.value} value={t.value}>{t.label}</option>)}
        </select>
      </div>

      {/* title */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>タイトル *</label>
        <input value={form.title} onChange={e => set("title", e.target.value)}
          placeholder="論文・書籍のタイトル" style={fieldStyle} />
      </div>

      {/* authors */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>著者</label>
        {(form.author_names ?? [""]).map((name, i) => (
          <div key={i} style={{ display: "flex", gap: 6, marginBottom: 5 }}>
            <input value={name} onChange={e => setAuthor(i, e.target.value)}
              placeholder={`著者 ${i + 1}`} style={{ ...fieldStyle, flex: 1 }} />
            {(form.author_names ?? []).length > 1 && (
              <button onClick={() => removeAuthor(i)} style={{
                padding: "0 8px", border: "1px solid var(--border-strong)",
                borderRadius: 5, background: "var(--surface)", color: "var(--text-mute)",
                cursor: "pointer", fontSize: 13,
              }}>×</button>
            )}
          </div>
        ))}
        <button onClick={addAuthor} style={{
          fontSize: 11.5, color: "var(--accent-strong)", border: "none",
          background: "transparent", cursor: "pointer", padding: "2px 0",
        }}>+ 著者を追加</button>
      </div>

      {/* year */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>出版年</label>
        <input type="number" min={1000} max={2100}
          value={form.year ?? ""} onChange={e => set("year", e.target.value ? Number(e.target.value) : undefined)}
          placeholder="2024" style={{ ...fieldStyle, width: 120 }} />
      </div>

      {/* identifiers */}
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

      {/* type-specific extra_fields */}
      <ExtraFieldInputs
        entryType={form.entry_type}
        values={form.extra_fields ?? {}}
        onChange={next => set("extra_fields", next)}
      />

      {/* abstract */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>抄録</label>
        <textarea value={form.abstract_ ?? ""} onChange={e => set("abstract_", e.target.value || undefined)}
          rows={4} placeholder="Abstract…"
          style={{ ...fieldStyle, resize: "vertical", lineHeight: 1.55 }} />
      </div>

      {/* notes */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>ノート</label>
        <textarea value={form.notes ?? ""} onChange={e => set("notes", e.target.value || undefined)}
          rows={3} placeholder="メモ…"
          style={{ ...fieldStyle, resize: "vertical", lineHeight: 1.55 }} />
      </div>

      {error && (
        <div style={{
          marginBottom: 12, padding: "8px 12px", borderRadius: 6,
          background: "oklch(0.95 0.04 15)", color: "oklch(0.45 0.13 15)",
          fontSize: 12,
        }}>{error}</div>
      )}

      <div style={{
        position: "sticky", bottom: 0,
        display: "flex", justifyContent: "flex-end", gap: 8,
        padding: "12px 0", background: "var(--surface)",
        borderTop: "1px solid var(--border)",
        marginTop: 4,
      }}>
        <button onClick={onClose} style={{
          padding: "5px 12px", borderRadius: 5,
          border: "1px solid var(--border-strong)",
          background: "var(--surface)", color: "var(--text)",
          fontSize: 12, cursor: "pointer",
        }}>キャンセル</button>
        <button onClick={handleAdd} disabled={!form.title.trim() || saving} style={{
          padding: "5px 14px", borderRadius: 5, border: "none",
          background: "var(--accent-strong)", color: "white",
          fontSize: 12, fontWeight: 500, cursor: "pointer",
          opacity: !form.title.trim() || saving ? 0.5 : 1,
        }}>{saving ? "追加中…" : "ライブラリに追加"}</button>
      </div>
    </div>
  );
}

// ── BibTeX タブ ───────────────────────────────────────────────────────────────

interface ImportResult {
  imported: number;
  skipped: number;
  errors: string[];
}

function BibtexTab({ onClose, onImported }: {
  onClose: () => void;
  onImported: () => void;
}) {
  const [content, setContent] = useState("");
  const [phase, setPhase] = useState<"input" | "loading" | "done" | "error">("input");
  const [result, setResult] = useState<ImportResult | null>(null);
  const [error, setError] = useState("");

  const handleImport = async () => {
    if (!content.trim()) return;
    setPhase("loading");
    try {
      const res = await invoke<ImportResult>("import_bibtex", { content });
      setResult(res);
      setPhase("done");
      onImported();
    } catch (e) {
      setError(String(e));
      setPhase("error");
    }
  };

  return (
    <div style={{ padding: 18 }}>
      {phase !== "done" ? (
        <>
          <textarea
            value={content}
            onChange={e => setContent(e.target.value)}
            rows={8}
            placeholder={"@article{vaswani2017,\n  title  = {Attention Is All You Need},\n  author = {Vaswani, Ashish and Shazeer, Noam},\n  year   = {2017},\n  doi    = {10.48550/arXiv.1706.03762}\n}"}
            style={{
              width: "100%", padding: "8px 10px", borderRadius: 6,
              border: "1px solid var(--border-strong)",
              background: "var(--surface-2)", color: "var(--text)",
              fontFamily: "var(--mono)", fontSize: 11.5, outline: "none",
              resize: "vertical", boxSizing: "border-box", lineHeight: 1.55,
            }}
          />
          {phase === "error" && (
            <div style={{
              marginTop: 10, padding: "8px 12px", borderRadius: 6,
              background: "oklch(0.95 0.04 15)", color: "oklch(0.45 0.13 15)",
              fontSize: 12,
            }}>{error}</div>
          )}
          <div style={{
            marginTop: 16, display: "flex", justifyContent: "flex-end", gap: 8,
            paddingTop: 14, borderTop: "1px solid var(--border)",
          }}>
            <button onClick={onClose} style={{
              padding: "5px 12px", borderRadius: 5,
              border: "1px solid var(--border-strong)",
              background: "var(--surface)", color: "var(--text)",
              fontSize: 12, cursor: "pointer",
            }}>キャンセル</button>
            <button
              onClick={handleImport}
              disabled={!content.trim() || phase === "loading"}
              style={{
                padding: "5px 14px", borderRadius: 5, border: "none",
                background: "var(--accent-strong)", color: "white",
                fontSize: 12, fontWeight: 500, cursor: "pointer",
                opacity: !content.trim() || phase === "loading" ? 0.5 : 1,
              }}
            >{phase === "loading" ? "インポート中…" : "インポート"}</button>
          </div>
        </>
      ) : (
        /* 完了画面 */
        <div>
          <div style={{
            padding: "16px 18px", borderRadius: 8,
            background: "var(--surface-2)", border: "1px solid var(--border)",
            marginBottom: 14,
          }}>
            <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text)", marginBottom: 8 }}>
              インポート完了
            </div>
            <div style={{ display: "flex", gap: 24 }}>
              <div>
                <div style={{ fontSize: 24, fontWeight: 700, color: "var(--accent-strong)", fontVariantNumeric: "tabular-nums" }}>
                  {result!.imported}
                </div>
                <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>件 追加</div>
              </div>
              {result!.skipped > 0 && (
                <div>
                  <div style={{ fontSize: 24, fontWeight: 700, color: "var(--text-mute)", fontVariantNumeric: "tabular-nums" }}>
                    {result!.skipped}
                  </div>
                  <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>件 スキップ</div>
                </div>
              )}
            </div>
            {result!.errors.length > 0 && (
              <div style={{ marginTop: 12, fontSize: 11, color: "var(--text-mute)" }}>
                <div style={{ marginBottom: 4, fontWeight: 600 }}>スキップされたエントリ:</div>
                {result!.errors.slice(0, 5).map((e, i) => (
                  <div key={i} style={{ fontFamily: "var(--mono)", color: "var(--text-faint)" }}>{e}</div>
                ))}
              </div>
            )}
          </div>
          <div style={{ display: "flex", justifyContent: "flex-end" }}>
            <button onClick={onClose} style={{
              padding: "5px 14px", borderRadius: 5, border: "none",
              background: "var(--accent-strong)", color: "white",
              fontSize: 12, fontWeight: 500, cursor: "pointer",
            }}>閉じる</button>
          </div>
        </div>
      )}
    </div>
  );
}

// ── メインコンポーネント ──────────────────────────────────────────────────────

const TABS: { id: AddTab; label: string }[] = [
  { id: "doi",    label: "DOI" },
  { id: "arxiv",  label: "arXiv" },
  { id: "isbn",   label: "ISBN" },
  { id: "bibtex", label: "BibTeX 貼付" },
  { id: "manual", label: "手動入力" },
];

export function AddSheet({ onClose, onCreated, onImported }: AddSheetProps) {
  const [tab, setTab] = useState<AddTab>("doi");

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
          overflow: "hidden",
        }}
      >
        {/* header */}
        <div style={{ padding: "14px 18px 12px", borderBottom: "1px solid var(--border)" }}>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text)" }}>文献を追加</div>
          <div style={{ fontSize: 11.5, color: "var(--text-faint)", marginTop: 3 }}>
            識別子から自動でメタデータを取得するか、手動で入力してください
          </div>
        </div>

        {/* tabs */}
        <div style={{ display: "flex", padding: "0 14px", borderBottom: "1px solid var(--border)", overflowX: "auto" }}>
          {TABS.map(({ id, label }) => (
            <button key={id} onClick={() => setTab(id)} style={{
              padding: "9px 11px", border: "none", background: "transparent",
              fontSize: 12, fontWeight: tab === id ? 600 : 500,
              color: tab === id ? "var(--text)" : "var(--text-mute)",
              borderBottom: tab === id ? "1.5px solid var(--accent-strong)" : "1.5px solid transparent",
              marginBottom: -1, cursor: "pointer", whiteSpace: "nowrap",
            }}>{label}</button>
          ))}
        </div>

        {/* content */}
        {(tab === "doi" || tab === "arxiv" || tab === "isbn") && (
          <IdentifierTab key={tab} tabId={tab} onCreated={onCreated} onClose={onClose} />
        )}
        {tab === "bibtex" && <BibtexTab onClose={onClose} onImported={onImported} />}
        {tab === "manual" && <ManualTab onCreated={onCreated} onClose={onClose} />}
      </div>
    </div>
  );
}
