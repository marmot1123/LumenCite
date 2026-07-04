import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { Icon } from "./icons";
import { ExtraFieldInputs } from "./ExtraFieldInputs";
import type { EntryDetail, EntryInput, EntryType } from "../types";
import { ENTRY_TYPES } from "../types";

interface AddSheetProps {
  onClose: () => void;
  onCreated: (entry: EntryDetail) => void;
  onImported: () => void;
  onSelectExisting?: (id: number) => void;
}

type AddTab = "doi" | "arxiv" | "isbn" | "bibtex" | "manual";
type Phase = "input" | "loading" | "preview" | "error" | "saving" | "downloading";

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

function IdentifierTab({ tabId, onCreated, onClose, onSelectExisting }: {
  tabId: "doi" | "arxiv" | "isbn";
  onCreated: (entry: EntryDetail) => void;
  onClose: () => void;
  onSelectExisting?: (id: number) => void;
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState("");
  const [phase, setPhase] = useState<Phase>("input");
  const [preview, setPreview] = useState<EntryInput | null>(null);
  const [error, setError] = useState("");
  const [duplicateId, setDuplicateId] = useState<number | null>(null);
  // arXiv タブでは PDF も一括ダウンロードする（デフォルト ON）。
  const [downloadPdf, setDownloadPdf] = useState(true);

  const handleFetch = async () => {
    if (!value.trim()) return;
    setPhase("loading");
    setError("");
    setDuplicateId(null);
    try {
      const data = await invoke<EntryInput>(FETCH_COMMANDS[tabId], {
        [FETCH_ARGS[tabId]]: value.trim(),
      });
      setPreview(data);
      setPhase("preview");
      // 取得後すぐに既存ライブラリ内の重複を確認。失敗しても警告は出さない。
      try {
        const hit = await invoke<number | null>("find_duplicate_entry", {
          doi:     data.doi     ?? null,
          arxivId: data.arxiv_id ?? null,
          isbn:    data.isbn    ?? null,
        });
        setDuplicateId(hit ?? null);
      } catch {
        setDuplicateId(null);
      }
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
      // arXiv の場合は PDF も一括でダウンロードして添付する。
      // ダウンロード失敗（ペイウォール・ネットワーク等）はエントリ作成を
      // 妨げない — 警告だけ出して詳細パネルから後で添付できるようにする。
      const arxivId = preview.arxiv_id?.trim();
      if (tabId === "arxiv" && downloadPdf && arxivId) {
        setPhase("downloading");
        try {
          await invoke("download_arxiv_pdf", { entryId: entry.id, arxivId });
        } catch (e) {
          // エントリは作成済み。PDF 失敗は詳細パネルから後で添付できるので
          // 作成を妨げず、ログのみに留めて閉じる。
          console.warn("arXiv PDF download failed:", e);
        }
      }
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
          disabled={phase === "loading" || phase === "saving" || phase === "downloading"}
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
          disabled={!value.trim() || phase === "loading" || phase === "saving" || phase === "downloading"}
          style={{
            padding: "8px 14px", borderRadius: 6, border: "none",
            background: "var(--accent-strong)", color: "white",
            fontSize: 12, fontWeight: 500, cursor: "pointer",
            opacity: !value.trim() || phase === "loading" ? 0.6 : 1,
          }}
        >
          {phase === "loading" ? t("addSheet.identifier.fetching") : t("addSheet.identifier.fetch")}
        </button>
      </div>

      <div style={{ marginTop: 10, fontSize: 11, color: "var(--text-faint)", display: "flex", alignItems: "center", gap: 6 }}>
        <Icon name="info" size={11} color="var(--text-faint)" />
        {tabId === "doi"   && t("addSheet.identifier.fetchFromDoi")}
        {tabId === "arxiv" && t("addSheet.identifier.fetchFromArxiv")}
        {tabId === "isbn"  && t("addSheet.identifier.fetchFromIsbn")}
      </div>

      {phase === "error" && (
        <div style={{
          marginTop: 12, padding: "8px 12px", borderRadius: 6,
          background: "var(--danger-bg)", color: "var(--danger-text)",
          fontSize: 12,
        }}>{error}</div>
      )}

      {phase === "preview" && preview && duplicateId != null && (
        <div style={{
          marginTop: 12, padding: "9px 12px", borderRadius: 7,
          background: "var(--warn-bg)",
          border: "1px solid var(--warn-border)",
          display: "flex", alignItems: "center", gap: 10,
        }}>
          <Icon name="info" size={12} color="var(--warn-text)" />
          <span style={{ flex: 1, fontSize: 11.5, color: "var(--warn-text)", lineHeight: 1.5 }}>
            {t("addSheet.identifier.duplicateWarn")}
          </span>
          {onSelectExisting && (
            <button
              onClick={() => { onSelectExisting(duplicateId); onClose(); }}
              style={{
                padding: "3px 9px", borderRadius: 4, border: "none",
                background: "var(--warn-strong)", color: "white",
                fontSize: 11, fontWeight: 600, cursor: "pointer",
              }}
            >{t("addSheet.identifier.showExisting")}</button>
          )}
        </div>
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
              {preview.author_names.length > 3 ? t("addSheet.identifier.authorMore", { count: preview.author_names.length - 3 }) : ""}
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

      {/* arXiv のみ: PDF も一括ダウンロードするか */}
      {(phase === "preview" || phase === "saving" || phase === "downloading") && preview && tabId === "arxiv" && (
        <label style={{
          marginTop: 12, display: "flex", alignItems: "center", gap: 8,
          fontSize: 12, color: "var(--text-mute)", cursor: "pointer",
        }}>
          <input
            type="checkbox"
            checked={downloadPdf}
            onChange={e => setDownloadPdf(e.target.checked)}
            disabled={phase === "saving" || phase === "downloading"}
            style={{ cursor: "pointer" }}
          />
          {t("addSheet.identifier.downloadPdf")}
        </label>
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
        }}>{t("common.cancel")}</button>
        <button
          onClick={handleAdd}
          disabled={phase !== "preview" || !preview}
          style={{
            padding: "5px 14px", borderRadius: 5, border: "none",
            background: duplicateId != null ? "var(--warn-strong)" : "var(--accent-strong)",
            color: "white",
            fontSize: 12, fontWeight: 500, cursor: "pointer",
            opacity: phase !== "preview" ? 0.4 : 1,
          }}
        >
          {phase === "downloading"
            ? t("addSheet.identifier.downloadingPdf")
            : phase === "saving"
            ? t("addSheet.addingToLibrary")
            : duplicateId != null ? t("addSheet.identifier.createAnyway") : t("addSheet.addToLibrary")}
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
  const { t } = useTranslation();
  const [form, setForm] = useState<EntryInput>({
    title: "",
    entry_type: "article",
    author_names: [""],
    year: undefined,
    citation_key: undefined,
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
  const [keyAvailable, setKeyAvailable] = useState(true);

  const set = <K extends keyof EntryInput>(key: K, value: EntryInput[K]) =>
    setForm(f => ({ ...f, [key]: value }));

  // 固定 cite key の重複を 300ms デバウンスで事前チェックする（空なら自動扱いで常に OK）。
  useEffect(() => {
    const key = (form.citation_key ?? "").trim();
    if (!key) { setKeyAvailable(true); return; }
    let cancelled = false;
    const h = setTimeout(async () => {
      try {
        const ok = await invoke<boolean>("is_citation_key_available", { key });
        if (!cancelled) setKeyAvailable(ok);
      } catch { /* チェック失敗時は保存側の UNIQUE 制約に委ねる */ }
    }, 300);
    return () => { cancelled = true; clearTimeout(h); };
  }, [form.citation_key]);

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
      title: form.title.trim(),
      author_names: (form.author_names ?? []).map(a => a.trim()).filter(Boolean),
      citation_key: form.citation_key?.trim() || undefined,
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
        <label style={labelStyle}>{t("addSheet.field.type")}</label>
        <select value={form.entry_type} onChange={e => set("entry_type", e.target.value as EntryType)}
          style={{ ...fieldStyle, cursor: "pointer" }}>
          {ENTRY_TYPES.map(type => (
            <option key={type} value={type}>{t(`entryTypeLabel.${type}` as const)}</option>
          ))}
        </select>
      </div>

      {/* title */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>{t("addSheet.field.titleRequired")}</label>
        <input value={form.title} onChange={e => set("title", e.target.value)}
          placeholder={t("addSheet.field.titlePlaceholder")} style={fieldStyle} />
      </div>

      {/* authors */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>{t("addSheet.field.authors")}</label>
        {(form.author_names ?? [""]).map((name, i) => (
          <div key={i} style={{ display: "flex", gap: 6, marginBottom: 5 }}>
            <input value={name} onChange={e => setAuthor(i, e.target.value)}
              placeholder={t("addSheet.field.authorPlaceholder", { index: i + 1 })}
              style={{ ...fieldStyle, flex: 1 }} />
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
        }}>{t("addSheet.field.addAuthor")}</button>
      </div>

      {/* year */}
      <div style={{ marginBottom: 12 }}>
        <label style={labelStyle}>{t("addSheet.field.yearPub")}</label>
        <input type="number" min={1000} max={2100}
          value={form.year ?? ""} onChange={e => set("year", e.target.value ? Number(e.target.value) : undefined)}
          placeholder="2024" style={{ ...fieldStyle, width: 120 }} />
      </div>

      {/* citation key */}
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
        <label style={labelStyle}>{t("addSheet.field.abstract")}</label>
        <textarea value={form.abstract_ ?? ""} onChange={e => set("abstract_", e.target.value || undefined)}
          rows={4} placeholder={t("addSheet.field.abstractPlaceholder")}
          style={{ ...fieldStyle, resize: "vertical", lineHeight: 1.55 }} />
      </div>

      {/* notes */}
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
        }}>{t("common.cancel")}</button>
        <button onClick={handleAdd} disabled={!form.title.trim() || saving || !keyAvailable} style={{
          padding: "5px 14px", borderRadius: 5, border: "none",
          background: "var(--accent-strong)", color: "white",
          fontSize: 12, fontWeight: 500, cursor: "pointer",
          opacity: !form.title.trim() || saving || !keyAvailable ? 0.5 : 1,
        }}>{saving ? t("addSheet.addingToLibrary") : t("addSheet.addToLibrary")}</button>
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
  const { t } = useTranslation();
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
              background: "var(--danger-bg)", color: "var(--danger-text)",
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
            }}>{t("common.cancel")}</button>
            <button
              onClick={handleImport}
              disabled={!content.trim() || phase === "loading"}
              style={{
                padding: "5px 14px", borderRadius: 5, border: "none",
                background: "var(--accent-strong)", color: "white",
                fontSize: 12, fontWeight: 500, cursor: "pointer",
                opacity: !content.trim() || phase === "loading" ? 0.5 : 1,
              }}
            >{phase === "loading" ? t("addSheet.bibtex.importing") : t("addSheet.bibtex.import")}</button>
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
              {t("addSheet.bibtex.completeTitle")}
            </div>
            <div style={{ display: "flex", gap: 24 }}>
              <div>
                <div style={{ fontSize: 24, fontWeight: 700, color: "var(--accent-strong)", fontVariantNumeric: "tabular-nums" }}>
                  {result!.imported}
                </div>
                <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>{t("addSheet.bibtex.addedSuffix")}</div>
              </div>
              {result!.skipped > 0 && (
                <div>
                  <div style={{ fontSize: 24, fontWeight: 700, color: "var(--text-mute)", fontVariantNumeric: "tabular-nums" }}>
                    {result!.skipped}
                  </div>
                  <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>{t("addSheet.bibtex.skippedSuffix")}</div>
                </div>
              )}
            </div>
            {result!.errors.length > 0 && (
              <div style={{ marginTop: 12, fontSize: 11, color: "var(--text-mute)" }}>
                <div style={{ marginBottom: 4, fontWeight: 600 }}>{t("addSheet.bibtex.errorsLabel")}</div>
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
            }}>{t("common.close")}</button>
          </div>
        </div>
      )}
    </div>
  );
}

// ── メインコンポーネント ──────────────────────────────────────────────────────

export function AddSheet({ onClose, onCreated, onImported, onSelectExisting }: AddSheetProps) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<AddTab>("doi");

  const TABS: { id: AddTab; label: string }[] = [
    { id: "doi",    label: "DOI" },
    { id: "arxiv",  label: "arXiv" },
    { id: "isbn",   label: "ISBN" },
    { id: "bibtex", label: t("addSheet.tab.bibtex") },
    { id: "manual", label: t("addSheet.tab.manual") },
  ];

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
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text)" }}>{t("addSheet.title")}</div>
          <div style={{ fontSize: 11.5, color: "var(--text-faint)", marginTop: 3 }}>
            {t("addSheet.subtitle")}
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
          <IdentifierTab key={tab} tabId={tab} onCreated={onCreated} onClose={onClose} onSelectExisting={onSelectExisting} />
        )}
        {tab === "bibtex" && <BibtexTab onClose={onClose} onImported={onImported} />}
        {tab === "manual" && <ManualTab onCreated={onCreated} onClose={onClose} />}
      </div>
    </div>
  );
}
