import { useState, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Icon, TypeIcon } from "./icons";
import { TagPill } from "./TagPill";
import { MathMarkdown } from "./MathMarkdown";
import { AuthorEditor } from "./AuthorEditor";
import { AuthorChip } from "./AuthorChip";
import { EXTRA_FIELDS_BY_TYPE, EXTRA_FIELD_LABEL_KEYS } from "../types";
import type { Collection, EntryDetail, EntryType, Tag } from "../types";
import { pickAndAttachPdf } from "../lib/attachments";

interface DetailPanelProps {
  entry: EntryDetail | null;
  width: number;
  inTrash?: boolean;
  onEdit?: () => void;
  onDelete?: () => void;
  onRestore?: () => void;
  onToggleStar?: () => void;
  allCollections: Collection[];
  onAddToCollection: (collectionId: number) => void;
  onRemoveFromCollection: (collectionId: number) => void;
  allTags: Tag[];
  onAddTag: (name: string) => void;
  onRemoveTag: (tagId: number) => void;
  onAttachmentsChanged?: () => void;
  onAttachmentAdded?: (attachmentId: number) => void;
  onUpdateField?: (field: "abstract_" | "notes", value: string) => void;
  onSelectEntry?: (id: number) => void;
  onSummarize?: () => void;
  onOpenDetail?: () => void;
  /** AuthorEditor で著者の表記や identifier が更新された後、親に entry の再フェッチを依頼する。 */
  onAuthorEdited?: () => void;
}

function flattenCollections(cols: Collection[], depth = 0): { col: Collection; depth: number }[] {
  return cols.flatMap(col => [
    { col, depth },
    ...flattenCollections(col.children, depth + 1),
  ]);
}

// extra_fields を「型固有の優先順 → それ以外（アルファベット順）」で並べる
// label は i18n キー（呼び出し側で t() で展開）
function orderedExtraFields(
  entryType: EntryType,
  extra: Record<string, string>,
): { key: string; labelKey: string; value: string }[] {
  const defs = EXTRA_FIELDS_BY_TYPE[entryType] ?? [];
  const definedKeys = new Set(defs.map(d => d.key));
  const ordered: { key: string; labelKey: string; value: string }[] = [];

  for (const def of defs) {
    const v = extra[def.key];
    if (v && v.trim()) ordered.push({ key: def.key, labelKey: def.labelKey, value: v });
  }
  const orphans = Object.entries(extra)
    .filter(([k, v]) => !definedKeys.has(k) && v?.trim())
    .sort(([a], [b]) => a.localeCompare(b));
  for (const [k, v] of orphans) {
    ordered.push({ key: k, labelKey: EXTRA_FIELD_LABEL_KEYS[k] ?? "", value: v });
  }
  return ordered;
}

function Field({ label, value, mono }: { label: string; value?: string | number | null; mono?: boolean }) {
  if (!value) return null;
  return (
    <div style={{ marginBottom: 12 }}>
      <div style={{
        fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
        textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 3,
      }}>{label}</div>
      <div style={{
        fontSize: 12.5, color: "var(--text)",
        fontFamily: mono ? "var(--mono)" : "inherit",
        wordBreak: "break-word", lineHeight: 1.45,
      }}>{value}</div>
    </div>
  );
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
      }}>{t("detailPanel.citationKey")}</div>
      <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
        <code style={{
          fontSize: 12.5, fontFamily: "var(--mono)", color: "var(--text)",
          background: "var(--surface-2)", border: "1px solid var(--border)",
          borderRadius: 4, padding: "2px 7px", wordBreak: "break-all",
        }}>{resolved}</code>
        <button onClick={copy} title={t("detailPanel.citationKeyCopy")} style={{
          fontSize: 11, color: copied ? "var(--text-mute)" : "var(--accent-strong)",
          border: "none", background: "transparent", cursor: "pointer",
          padding: "2px 4px", whiteSpace: "nowrap",
        }}>{copied ? t("detailPanel.citationKeyCopied") : t("detailPanel.citationKeyCopy")}</button>
      </div>
      {!entry.citation_key && (
        <div style={{ fontSize: 10, color: "var(--text-faint)", marginTop: 3 }}>
          {t("detailPanel.citationKeyAuto")}
        </div>
      )}
    </div>
  );
}

function Tab({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button onClick={onClick} style={{
      flex: 1, padding: "8px 0", border: "none", background: "transparent",
      fontSize: 12, fontWeight: active ? 600 : 500,
      color: active ? "var(--text)" : "var(--text-mute)",
      cursor: "pointer",
      borderBottom: active ? "1.5px solid var(--accent-strong)" : "1.5px solid transparent",
      marginBottom: -1,
    }}>{label}</button>
  );
}

function ActionBtn({ icon, label, primary, onClick, disabled, title }: {
  icon?: Parameters<typeof Icon>[0]["name"];
  label: string;
  primary?: boolean;
  onClick?: () => void;
  disabled?: boolean;
  title?: string;
}) {
  return (
    <button onClick={onClick} disabled={disabled} title={title} style={{
      display: "inline-flex", alignItems: "center", gap: 5,
      padding: "5px 9px", borderRadius: 5,
      border: primary ? "none" : "1px solid var(--border-strong)",
      background: primary ? "var(--accent-strong)" : "var(--surface)",
      color: primary ? "white" : "var(--text)",
      fontSize: 11.5, fontWeight: 500,
      cursor: disabled ? "not-allowed" : "pointer",
      opacity: disabled ? 0.5 : 1,
    }}>
      {icon && <Icon name={icon} size={11} color={primary ? "white" : "var(--text-mute)"} />}
      {label}
    </button>
  );
}

// 抄録／ノートのインライン編集用 textarea。フォーカス時に枠線が現れ、blur で
// 値が変わっていれば onSave を呼ぶ。Esc で編集を破棄してフォーカスを外す。
function EditableText({
  value,
  placeholder,
  minRows,
  onSave,
}: {
  value: string;
  placeholder: string;
  minRows: number;
  onSave: (next: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // entry 切替や外部更新で value が変わったら draft をリセット
  useEffect(() => { setDraft(value); }, [value]);

  useEffect(() => {
    if (editing) textareaRef.current?.focus();
  }, [editing]);

  const commit = () => {
    if (draft !== value) onSave(draft);
    setEditing(false);
  };

  if (editing) {
    return (
      <textarea
        ref={textareaRef}
        value={draft}
        placeholder={placeholder}
        onChange={e => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={e => {
          if (e.key === "Escape") {
            setDraft(value);
            setEditing(false);
          }
        }}
        rows={minRows}
        style={{
          width: "100%",
          fontSize: 12.5, lineHeight: 1.65,
          color: "var(--text)",
          padding: "8px 10px",
          background: "var(--surface-2)",
          border: "1px solid var(--border-strong)",
          borderRadius: 6,
          resize: "vertical", outline: "none",
          fontFamily: "inherit",
          boxSizing: "border-box",
        }}
      />
    );
  }

  return (
    <div
      onClick={() => setEditing(true)}
      style={{
        padding: "8px 10px",
        borderRadius: 6,
        border: "1px solid transparent",
        cursor: "text",
        fontSize: 12.5,
        color: "var(--text)",
        minHeight: minRows * 18,
      }}
    >
      {value ? (
        <MathMarkdown value={value} />
      ) : (
        <span style={{ color: "var(--text-faint)" }}>{placeholder}</span>
      )}
    </div>
  );
}

type TabId = "info" | "abstract" | "notes" | "related";

const panelStyle = (width: number): React.CSSProperties => ({
  width, flexShrink: 0, height: "100%",
  background: "var(--surface)",
  borderLeft: "1px solid var(--border)",
  display: "flex", flexDirection: "column",
  overflow: "hidden",
});

export function DetailPanel({ entry, width, inTrash, onEdit, onDelete, onRestore, onToggleStar, allCollections, onAddToCollection, onRemoveFromCollection, allTags, onAddTag, onRemoveTag, onAttachmentsChanged, onAttachmentAdded, onUpdateField, onSelectEntry, onSummarize, onOpenDetail, onAuthorEdited }: DetailPanelProps) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<TabId>("info");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [bibtexLabel, setBibtexLabel] = useState("BibTeX");
  const [showColDropdown, setShowColDropdown] = useState(false);
  const colDropdownRef = useRef<HTMLDivElement>(null);
  const [tagInputOpen, setTagInputOpen] = useState(false);
  const [tagInput, setTagInput] = useState("");
  const tagInputRef = useRef<HTMLDivElement>(null);
  const [attaching, setAttaching] = useState(false);
  const [attachError, setAttachError] = useState<string | null>(null);
  const [editingAuthorId, setEditingAuthorId] = useState<number | null>(null);
  // 添付ごとの全文索引状態。
  const [indexStatus, setIndexStatus] = useState<Record<number, "indexed" | "none" | "indexing">>({});
  const [indexNote, setIndexNote] = useState<string | null>(null);
  // arXiv TeX ソース取得（LCIR Phase 4）。
  const [texBusy, setTexBusy] = useState(false);
  // ダウンロード中にユーザーが別エントリへ移った場合、完了時の表示更新を捨てるための現在値参照
  // （texBusy 自体はリセットしない — 同一エントリへの二重ダウンロード防止のため）。
  const entryIdRef = useRef<number | null>(null);
  entryIdRef.current = entry?.id ?? null;

  // PDF 以外（arXiv TeX ソース .gz 等）はビューア・全文索引の対象外。
  const isPdfAttachment = (mime: string) => mime.toLowerCase().includes("pdf");

  useEffect(() => {
    setConfirmDelete(false);
    setShowColDropdown(false);
    setTagInputOpen(false);
    setTagInput("");
    setAttachError(null);
    setIndexNote(null);
  }, [entry?.id]);

  // 添付の全文索引状態を取得する（エントリ切替・添付増減で再取得）。PDF のみが対象。
  useEffect(() => {
    let cancelled = false;
    const atts = (entry?.attachments ?? []).filter(a => isPdfAttachment(a.mime_type));
    if (atts.length === 0) {
      setIndexStatus({});
      return;
    }
    Promise.all(
      atts.map(a =>
        invoke<boolean>("is_attachment_indexed", { id: a.id })
          .then(ok => [a.id, ok ? "indexed" : "none"] as const)
          .catch(() => [a.id, "none"] as const),
      ),
    ).then(pairs => {
      if (!cancelled) setIndexStatus(Object.fromEntries(pairs));
    });
    return () => {
      cancelled = true;
    };
  }, [entry?.id, entry?.attachments?.length]);

  const handleIndexAttachment = async (attId: number) => {
    setIndexNote(null);
    setIndexStatus(s => ({ ...s, [attId]: "indexing" }));
    try {
      const pages = await invoke<number>("index_attachment", { id: attId });
      setIndexStatus(s => ({ ...s, [attId]: pages > 0 ? "indexed" : "none" }));
      setIndexNote(
        pages > 0
          ? t("detailPanel.indexDonePages", { count: pages })
          : t("detailPanel.indexNoText"),
      );
    } catch (e: any) {
      setIndexStatus(s => ({ ...s, [attId]: "none" }));
      setIndexNote(e?.message ?? String(e));
    }
  };

  useEffect(() => {
    if (!showColDropdown) return;
    const handler = (e: MouseEvent) => {
      if (colDropdownRef.current && !colDropdownRef.current.contains(e.target as Node)) {
        setShowColDropdown(false);
      }
    };
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [showColDropdown]);

  useEffect(() => {
    if (!tagInputOpen) return;
    const handler = (e: MouseEvent) => {
      if (tagInputRef.current && !tagInputRef.current.contains(e.target as Node)) {
        setTagInputOpen(false);
        setTagInput("");
      }
    };
    window.addEventListener("mousedown", handler);
    return () => window.removeEventListener("mousedown", handler);
  }, [tagInputOpen]);

  const submitTag = (name: string) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    if (entry?.tags.some(t => t.name === trimmed)) return;
    onAddTag(trimmed);
    setTagInput("");
    setTagInputOpen(false);
  };

  const handleAttachPdf = async () => {
    if (!entry || attaching) return;
    setAttachError(null);
    try {
      setAttaching(true);
      const att = await pickAndAttachPdf(entry.id);
      if (!att) return;
      onAttachmentsChanged?.();
      onAttachmentAdded?.(att.id);
    } catch (e: any) {
      setAttachError(e?.message ?? String(e));
    } finally {
      setAttaching(false);
    }
  };

  const handleDeleteAttachment = async (attachmentId: number) => {
    try {
      await invoke("delete_attachment", { id: attachmentId });
      onAttachmentsChanged?.();
    } catch (e) {
      console.error(e);
    }
  };

  // arXiv の TeX ソース（e-print）を取得して添付し、続けて LCIR を構築する（Phase 4）。
  // フラグ OFF のときは「取得はしたが未構築」を明示する（隠れた状態を作らない）。
  // ダウンロードは数十秒かかりうるので、完了時に別エントリへ移っていたら表示更新は捨てる。
  const handleFetchTexSource = async () => {
    if (!entry?.arxiv_id || texBusy) return;
    const startedFor = entry.id;
    const stillHere = () => entryIdRef.current === startedFor;
    setTexBusy(true);
    setAttachError(null);
    setIndexNote(null);
    try {
      let att: { id: number };
      try {
        att = await invoke<{ id: number }>("download_arxiv_source", {
          entryId: startedFor,
          arxivId: entry.arxiv_id,
        });
      } catch (e: any) {
        if (stillHere()) {
          setAttachError(t("detailPanel.texSourceError", { error: e?.message ?? String(e) }));
        }
        return;
      }
      if (stillHere()) onAttachmentsChanged?.();
      // ここからは添付は成功済み: ビルド失敗は「取得失敗」と混同させない。
      try {
        const res = await invoke<{ enabled: boolean; built: boolean; reused: boolean; message: string }>(
          "build_lcir_for_attachment",
          { attachmentId: att.id },
        );
        if (stillHere()) {
          setIndexNote(
            res.enabled
              ? t("detailPanel.texSourceDoneBuilt")
              : t("detailPanel.texSourceDoneNoLcir"),
          );
        }
      } catch (e: any) {
        if (stillHere()) {
          setAttachError(t("detailPanel.texSourceBuildFailed", { error: e?.message ?? String(e) }));
        }
      }
    } finally {
      setTexBusy(false);
    }
  };

  const handleCopyBibtex = async () => {
    if (!entry) return;
    try {
      const bib = await invoke<string>("export_bibtex", { entryIds: [entry.id] });
      await writeText(bib);
      setBibtexLabel(t("detailPanel.copied"));
      setTimeout(() => setBibtexLabel("BibTeX"), 2000);
    } catch {
      setBibtexLabel(t("detailPanel.copyError"));
      setTimeout(() => setBibtexLabel("BibTeX"), 2000);
    }
  };

  if (!entry) {
    return (
      <aside style={panelStyle(width)}>
        <div style={{
          flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
          padding: 24, color: "var(--text-faint)", fontSize: 12.5, textAlign: "center", lineHeight: 1.6,
          whiteSpace: "pre-line",
        }}>
          {t("detailPanel.noSelection")}
        </div>
      </aside>
    );
  }

  return (
    <aside style={panelStyle(width)}>
      {/* hero */}
      <div style={{ padding: "16px 18px 14px", borderBottom: "1px solid var(--border)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
          <span style={{
            display: "inline-flex", alignItems: "center", gap: 5,
            padding: "1px 7px 1px 5px", borderRadius: 4,
            background: "var(--surface-2)", color: "var(--text-mute)",
            fontSize: 10.5, fontWeight: 500,
          }}>
            <TypeIcon type={entry.entry_type} size={11} />
            {entry.entry_type}
          </span>
          {entry.year && (
            <span style={{ fontSize: 11, color: "var(--text-faint)", fontVariantNumeric: "tabular-nums" }}>
              {entry.year}
            </span>
          )}
          <div style={{ flex: 1 }} />
          <button
            onClick={onToggleStar}
            title={entry.starred ? t("detailPanel.starOn") : t("detailPanel.starOff")}
            style={{
              width: 26, height: 26, padding: 0, border: "none", background: "transparent",
              borderRadius: 5, cursor: "pointer", display: "inline-flex", alignItems: "center", justifyContent: "center",
            }}
          >
            <Icon
              name="star"
              size={13}
              color={entry.starred ? "var(--accent-strong)" : "var(--text-mute)"}
            />
          </button>
        </div>

        <h2 style={{
          margin: 0, fontSize: 15.5, fontWeight: 600, lineHeight: 1.32,
          color: "var(--text)", letterSpacing: "-0.012em",
        }}>{entry.title}</h2>

        {entry.authors.length > 0 && (
          <div style={{
            marginTop: 8, fontSize: 12, color: "var(--text-mute)", lineHeight: 1.55,
            display: "flex", flexWrap: "wrap", alignItems: "center", gap: 0,
          }}>
            {entry.authors.map((a, i) => (
              <span key={a.id} style={{ display: "inline-flex", alignItems: "center" }}>
                <AuthorChip author={a} onClick={() => setEditingAuthorId(a.id)} />
                {i < entry.authors.length - 1 && <span style={{ marginRight: 2 }}>,</span>}
              </span>
            ))}
          </div>
        )}

        {!confirmDelete ? (
          <div style={{ display: "flex", gap: 6, marginTop: 12, flexWrap: "wrap", alignItems: "center" }}>
            {inTrash ? (
              <>
                <ActionBtn icon="ext" label={t("detailPanel.restore")} primary onClick={onRestore} />
                <div style={{ flex: 1 }} />
                <button
                  onClick={() => setConfirmDelete(true)}
                  style={{
                    padding: "4px 8px", borderRadius: 5, border: "none",
                    background: "transparent", color: "var(--text-faint)",
                    fontSize: 11, cursor: "pointer",
                  }}
                >{t("detailPanel.purge")}</button>
              </>
            ) : (
              <>
                {entry.attachments.length > 0 ? (
                  <ActionBtn
                    icon="ext"
                    label={t("detailPanel.openDetail")}
                    primary
                    onClick={onOpenDetail}
                    disabled={!onOpenDetail}
                  />
                ) : (
                  <ActionBtn
                    icon="paperclip"
                    label={attaching ? t("detailPanel.attaching") : t("detailPanel.attachPdf")}
                    onClick={handleAttachPdf}
                  />
                )}
                <ActionBtn
                  icon="sparkle"
                  label={t("detailPanel.summarize")}
                  onClick={onSummarize}
                  disabled={!onSummarize}
                  title={t("summary.title")}
                />
                <ActionBtn icon="download" label={bibtexLabel} onClick={handleCopyBibtex} />
                <ActionBtn label={t("detailPanel.edit")} onClick={onEdit} />
                <div style={{ flex: 1 }} />
                <button
                  onClick={() => setConfirmDelete(true)}
                  style={{
                    padding: "4px 8px", borderRadius: 5, border: "none",
                    background: "transparent", color: "var(--text-faint)",
                    fontSize: 11, cursor: "pointer",
                  }}
                >{t("detailPanel.delete")}</button>
              </>
            )}
          </div>
        ) : (
          <div style={{
            display: "flex", alignItems: "center", gap: 8, marginTop: 12,
            padding: "7px 10px", borderRadius: 7,
            background: "var(--danger-bg)", border: "1px solid var(--danger-border)",
          }}>
            <span style={{ fontSize: 11.5, color: "var(--danger-text)", flex: 1 }}>
              {inTrash ? t("detailPanel.purgeConfirm") : t("detailPanel.trashConfirm")}
            </span>
            <button
              onClick={() => setConfirmDelete(false)}
              style={{
                padding: "3px 9px", borderRadius: 4,
                border: "1px solid var(--border-strong)",
                background: "var(--surface)", color: "var(--text)",
                fontSize: 11, cursor: "pointer",
              }}
            >{t("detailPanel.cancel")}</button>
            <button
              onClick={onDelete}
              style={{
                padding: "3px 9px", borderRadius: 4, border: "none",
                background: "var(--danger-strong)", color: "white",
                fontSize: 11, fontWeight: 600, cursor: "pointer",
              }}
            >{inTrash ? t("detailPanel.purgeOk") : t("detailPanel.ok")}</button>
          </div>
        )}
      </div>

      {/* tabs */}
      <div style={{
        display: "flex", borderBottom: "1px solid var(--border)",
        padding: "0 14px", flexShrink: 0,
      }}>
        <Tab label={t("detailPanel.tab.info")} active={tab === "info"} onClick={() => setTab("info")} />
        <Tab label={t("detailPanel.tab.abstract")} active={tab === "abstract"} onClick={() => setTab("abstract")} />
        <Tab label={t("detailPanel.tab.notes")} active={tab === "notes"} onClick={() => setTab("notes")} />
        <Tab label={t("detailPanel.tab.related")} active={tab === "related"} onClick={() => setTab("related")} />
      </div>

      {/* body */}
      <div style={{ flex: 1, overflow: "auto", padding: "16px 18px" }}>
        {tab === "info" && (
          <>
            {(() => {
              const items = orderedExtraFields(entry.entry_type as EntryType, entry.extra_fields);
              if (items.length === 0) return null;
              return (
                <div style={{ marginBottom: 4, paddingBottom: 10, borderBottom: "1px solid var(--border)" }}>
                  {items.map(({ key, labelKey, value }) => (
                    <Field key={key} label={labelKey ? t(labelKey as any) : key} value={value} />
                  ))}
                </div>
              );
            })()}

            <CitationKeyField entry={entry} />
            <Field label="DOI" value={entry.doi} mono />
            <Field label="arXiv" value={entry.arxiv_id} mono />
            <Field label="ISBN" value={entry.isbn} mono />
            <Field label="URL" value={entry.url} mono />
            <Field label={t("detailPanel.venueYear")} value={entry.year} />
            <Field label={t("detailPanel.addedAt")} value={entry.created_at ? entry.created_at.slice(0, 10) : undefined} />

            <div style={{ marginTop: 4, marginBottom: 14 }}>
              <div style={{
                fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
                textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
              }}>{t("detailPanel.attachmentsLabel2")}</div>
              <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
                {entry.attachments.map(att => (
                  <div key={att.id} style={{
                    display: "flex", alignItems: "center", gap: 6,
                    padding: "3px 4px 3px 7px", borderRadius: 5,
                    background: "var(--surface-2)", fontSize: 12,
                    color: "var(--text)",
                  }}>
                    <Icon name="paperclip" size={11} color="var(--text-faint)" />
                    {isPdfAttachment(att.mime_type) ? (
                      <button
                        onClick={() => invoke("open_pdf_viewer", { id: att.id }).catch(console.error)}
                        style={{
                          flex: 1, minWidth: 0, padding: 0, border: "none",
                          background: "transparent", color: "var(--text)",
                          fontSize: 12, textAlign: "left", cursor: "pointer",
                          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
                        }}
                        title={att.file_name}
                      >{att.file_name}</button>
                    ) : (
                      // PDF ではない添付（arXiv TeX ソース等）はビューアを開けない。
                      <span
                        style={{
                          flex: 1, minWidth: 0, color: "var(--text)", fontSize: 12,
                          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
                        }}
                        title={att.file_name}
                      >{att.file_name}</span>
                    )}
                    {isPdfAttachment(att.mime_type) ? (
                      <>
                        <span
                          title={
                            indexStatus[att.id] === "indexed"
                              ? t("detailPanel.indexedBadge")
                              : t("detailPanel.notIndexedBadge")
                          }
                          style={{
                            width: 6, height: 6, borderRadius: "50%", flex: "none",
                            background: indexStatus[att.id] === "indexed"
                              ? "var(--accent)"
                              : "var(--border-strong)",
                          }}
                        />
                        <button
                          onClick={() => handleIndexAttachment(att.id)}
                          disabled={indexStatus[att.id] === "indexing"}
                          style={{
                            width: 16, height: 16, padding: 0, border: "none",
                            background: "transparent",
                            cursor: indexStatus[att.id] === "indexing" ? "default" : "pointer",
                            display: "inline-flex", alignItems: "center", justifyContent: "center",
                            borderRadius: 3, color: "var(--text-faint)",
                            opacity: indexStatus[att.id] === "indexing" ? 0.5 : 1,
                          }}
                          title={
                            indexStatus[att.id] === "indexed"
                              ? t("detailPanel.reindexTitle")
                              : t("detailPanel.indexNowTitle")
                          }
                        >
                          <Icon name="sync" size={11} color="var(--text-faint)" />
                        </button>
                      </>
                    ) : (
                      <span
                        title={t("detailPanel.texChipTitle")}
                        style={{
                          flex: "none", fontSize: 9.5, fontWeight: 600, letterSpacing: "0.04em",
                          padding: "0 5px", borderRadius: 4, lineHeight: "14px",
                          background: "var(--accent-soft)", color: "var(--accent-strong)",
                        }}
                      >TeX</span>
                    )}
                    <button
                      onClick={() => handleDeleteAttachment(att.id)}
                      style={{
                        width: 16, height: 16, padding: 0, border: "none",
                        background: "transparent", cursor: "pointer",
                        display: "inline-flex", alignItems: "center", justifyContent: "center",
                        borderRadius: 3, color: "var(--text-faint)",
                      }}
                      title={t("detailPanel.removeAttachment2")}
                    >
                      <Icon name="close" size={9} color="var(--text-faint)" />
                    </button>
                  </div>
                ))}
                <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                  <button
                    onClick={handleAttachPdf}
                    disabled={attaching}
                    style={{
                      display: "inline-flex", alignItems: "center", gap: 3,
                      padding: "2px 8px", borderRadius: 5,
                      border: "1px dashed var(--border-strong)",
                      background: "transparent", color: "var(--text-faint)",
                      fontSize: 11, cursor: attaching ? "default" : "pointer",
                      opacity: attaching ? 0.5 : 1,
                    }}
                  >
                    <Icon name="plus" size={9} color="var(--text-faint)" />
                    {attaching ? t("detailPanel.attaching") : t("detailPanel.addPdf")}
                  </button>
                  {entry.arxiv_id && (
                    <button
                      onClick={handleFetchTexSource}
                      disabled={texBusy}
                      title={t("detailPanel.texSourceTitle")}
                      style={{
                        display: "inline-flex", alignItems: "center", gap: 3,
                        padding: "2px 8px", borderRadius: 5,
                        border: "1px dashed var(--border-strong)",
                        background: "transparent", color: "var(--text-faint)",
                        fontSize: 11, cursor: texBusy ? "default" : "pointer",
                        opacity: texBusy ? 0.5 : 1,
                      }}
                    >
                      <Icon name="download" size={9} color="var(--text-faint)" />
                      {texBusy ? t("detailPanel.texSourceBusy") : t("detailPanel.texSource")}
                    </button>
                  )}
                </div>
                {attachError && (
                  <div style={{ fontSize: 11, color: "var(--danger-strong)", marginTop: 2 }}>
                    {attachError}
                  </div>
                )}
                {indexNote && (
                  <div style={{ fontSize: 11, color: "var(--text-mute)", marginTop: 2 }}>
                    {indexNote}
                  </div>
                )}
              </div>
            </div>

            <div style={{ marginTop: 4 }}>
              <div style={{
                fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
                textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
              }}>{t("detailPanel.tagsLabel2")}</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4, alignItems: "center" }}>
                {entry.tags.map(tag => (
                  <TagPill
                    key={tag.id}
                    name={tag.name}
                    onRemove={() => onRemoveTag(tag.id)}
                  />
                ))}
                <div ref={tagInputRef} style={{ position: "relative", display: "inline-flex" }}>
                  {!tagInputOpen ? (
                    <button
                      onClick={() => setTagInputOpen(true)}
                      style={{
                        display: "inline-flex", alignItems: "center", gap: 3,
                        padding: "1px 7px", borderRadius: 999,
                        border: "1px dashed var(--border-strong)",
                        background: "transparent", color: "var(--text-faint)",
                        fontSize: 10.5, cursor: "pointer",
                      }}
                    >
                      <Icon name="plus" size={9} color="var(--text-faint)" />
                      {t("detailPanel.addTag")}
                    </button>
                  ) : (
                    <>
                      <input
                        autoFocus
                        value={tagInput}
                        onChange={e => setTagInput(e.target.value)}
                        onKeyDown={e => {
                          if (e.key === "Enter") { submitTag(tagInput); e.preventDefault(); }
                          if (e.key === "Escape") {
                            setTagInputOpen(false); setTagInput("");
                            e.preventDefault();
                          }
                        }}
                        placeholder={t("detailPanel.tagsPlaceholder2")}
                        style={{
                          width: 120, padding: "2px 8px",
                          borderRadius: 999,
                          border: "1px solid var(--accent-strong)",
                          background: "var(--surface)", color: "var(--text)",
                          fontSize: 11, outline: "none",
                        }}
                      />
                      {tagInput.trim() && (() => {
                        const existing = entry.tags.map(t => t.name);
                        const q = tagInput.trim().toLowerCase();
                        const suggestions = allTags
                          .filter(t => !existing.includes(t.name) && t.name.toLowerCase().includes(q))
                          .slice(0, 6);
                        if (suggestions.length === 0) return null;
                        return (
                          <div style={{
                            position: "absolute", top: "100%", left: 0, marginTop: 4,
                            background: "var(--surface)",
                            border: "1px solid var(--border-strong)",
                            borderRadius: 6,
                            boxShadow: "0 4px 12px rgba(0,0,0,0.1)",
                            zIndex: 100, minWidth: 140, padding: "3px 0",
                          }}>
                            {suggestions.map(s => (
                              <button
                                key={s.id}
                                onMouseDown={e => { e.preventDefault(); submitTag(s.name); }}
                                style={{
                                  display: "block", width: "100%", padding: "4px 10px",
                                  border: "none", background: "transparent", textAlign: "left",
                                  fontSize: 11.5, cursor: "pointer", color: "var(--text)",
                                }}
                                onMouseEnter={e => (e.currentTarget.style.background = "var(--hover)")}
                                onMouseLeave={e => (e.currentTarget.style.background = "transparent")}
                              >{s.name}</button>
                            ))}
                          </div>
                        );
                      })()}
                    </>
                  )}
                </div>
              </div>
            </div>

            <div style={{ marginTop: 14 }}>
              <div style={{
                fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
                textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
              }}>{t("detailPanel.collectionsLabel2")}</div>
              <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
                {(entry.collections ?? []).map(col => (
                  <div key={col.id} style={{
                    display: "flex", alignItems: "center", gap: 6,
                    padding: "3px 7px", borderRadius: 5,
                    background: "var(--surface-2)", fontSize: 12,
                    color: "var(--text-mute)",
                  }}>
                    <Icon name="folder" size={11} color="var(--text-faint)" />
                    <span style={{ flex: 1 }}>{col.name}</span>
                    <button
                      onClick={() => onRemoveFromCollection(col.id)}
                      style={{
                        width: 16, height: 16, padding: 0, border: "none",
                        background: "transparent", cursor: "pointer",
                        display: "inline-flex", alignItems: "center", justifyContent: "center",
                        borderRadius: 3, color: "var(--text-faint)",
                      }}
                      title={t("detailPanel.removeFromCollection")}
                    >
                      <Icon name="close" size={9} color="var(--text-faint)" />
                    </button>
                  </div>
                ))}
                <div style={{ position: "relative" }} ref={colDropdownRef}>
                  <button
                    onClick={() => setShowColDropdown(v => !v)}
                    style={{
                      display: "inline-flex", alignItems: "center", gap: 3,
                      padding: "2px 8px", borderRadius: 5,
                      border: "1px dashed var(--border-strong)",
                      background: "transparent", color: "var(--text-faint)",
                      fontSize: 11, cursor: "pointer",
                    }}
                  >
                    <Icon name="plus" size={9} color="var(--text-faint)" />
                    {t("detailPanel.addToCollection2")}
                  </button>
                  {showColDropdown && (() => {
                    const entryColIds = new Set(entry.collections.map(c => c.id));
                    const available = flattenCollections(allCollections).filter(({ col }) => !entryColIds.has(col.id));
                    return (
                      <div style={{
                        position: "absolute", top: "100%", left: 0, marginTop: 4,
                        background: "var(--surface)",
                        border: "1px solid var(--border-strong)",
                        borderRadius: 7,
                        boxShadow: "0 4px 14px rgba(0,0,0,0.12)",
                        zIndex: 100, minWidth: 180, maxHeight: 220, overflow: "auto",
                        padding: "4px 0",
                      }}>
                        {available.length === 0 ? (
                          <div style={{ padding: "8px 14px", fontSize: 12, color: "var(--text-faint)" }}>
                            {t("detailPanel.noCollections")}
                          </div>
                        ) : available.map(({ col, depth }) => (
                          <button
                            key={col.id}
                            onClick={() => { onAddToCollection(col.id); setShowColDropdown(false); }}
                            style={{
                              display: "flex", width: "100%", alignItems: "center", gap: 7,
                              padding: `5px 14px 5px ${14 + depth * 12}px`,
                              border: "none", background: "transparent", textAlign: "left",
                              fontSize: 12.5, cursor: "pointer", color: "var(--text)",
                            }}
                            onMouseEnter={e => (e.currentTarget.style.background = "var(--hover)")}
                            onMouseLeave={e => (e.currentTarget.style.background = "transparent")}
                          >
                            <Icon name="folder" size={12} color="var(--text-faint)" />
                            {col.name}
                          </button>
                        ))}
                      </div>
                    );
                  })()}
                </div>
              </div>
            </div>

          </>
        )}

        {tab === "abstract" && (
          onUpdateField && !inTrash ? (
            <EditableText
              key={`abs-${entry.id}`}
              value={entry.abstract_ ?? ""}
              placeholder={t("detailPanel.abstractPlaceholder")}
              minRows={6}
              onSave={v => onUpdateField("abstract_", v)}
            />
          ) : (
            <div style={{ fontSize: 12.5, lineHeight: 1.65, color: "var(--text)", padding: "8px 10px", whiteSpace: "pre-wrap" }}>
              {entry.abstract_ || (
                <span style={{ color: "var(--text-faint)" }}>{t("detailPanel.abstractEmpty")}</span>
              )}
            </div>
          )
        )}

        {tab === "notes" && (
          onUpdateField && !inTrash ? (
            <EditableText
              key={`notes-${entry.id}`}
              value={entry.notes ?? ""}
              placeholder={t("detailPanel.notesPlaceholder")}
              minRows={5}
              onSave={v => onUpdateField("notes", v)}
            />
          ) : (
            <div style={{ fontSize: 12.5, lineHeight: 1.65, color: "var(--text)", padding: "8px 10px", whiteSpace: "pre-wrap" }}>
              {entry.notes || (
                <span style={{ color: "var(--text-faint)" }}>{t("detailPanel.notesEmpty")}</span>
              )}
            </div>
          )
        )}

        {tab === "related" && (
          entry.relations.length === 0 ? (
            <div style={{ fontSize: 12, color: "var(--text-faint)", lineHeight: 1.6, whiteSpace: "pre-line" }}>
              {t("detailPanel.relatedEmpty")}
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              {entry.relations.map(rel => (
                <button
                  key={`${rel.direction}-${rel.relation_type}-${rel.entry.id}`}
                  onClick={() => onSelectEntry?.(rel.entry.id)}
                  disabled={!onSelectEntry}
                  style={{
                    display: "flex", flexDirection: "column", alignItems: "stretch", gap: 4,
                    padding: "8px 10px", borderRadius: 6,
                    border: "1px solid var(--border)",
                    background: "var(--surface)",
                    cursor: onSelectEntry ? "pointer" : "default",
                    textAlign: "left",
                  }}
                  onMouseEnter={e => { if (onSelectEntry) e.currentTarget.style.background = "var(--hover)"; }}
                  onMouseLeave={e => { e.currentTarget.style.background = "var(--surface)"; }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <TypeIcon type={rel.entry.entry_type as EntryType} size={11} />
                    <span style={{
                      fontSize: 10.5, color: "var(--text-faint)",
                      textTransform: "uppercase", letterSpacing: "0.04em",
                    }}>
                      {rel.direction === "from" ? `→ ${rel.relation_type}` : `← ${rel.relation_type}`}
                    </span>
                    {rel.entry.year && (
                      <span style={{ fontSize: 10.5, color: "var(--text-faint)", marginLeft: "auto", fontVariantNumeric: "tabular-nums" }}>
                        {rel.entry.year}
                      </span>
                    )}
                  </div>
                  <div style={{ fontSize: 12.5, color: "var(--text)", lineHeight: 1.4, fontWeight: 500 }}>
                    {rel.entry.title}
                  </div>
                  {rel.entry.authors.length > 0 && (
                    <div style={{ fontSize: 11, color: "var(--text-mute)", lineHeight: 1.4 }}>
                      {rel.entry.authors.map(a => a.name).join(", ")}
                    </div>
                  )}
                </button>
              ))}
            </div>
          )
        )}
      </div>

      {editingAuthorId != null && (
        <AuthorEditor
          authorId={editingAuthorId}
          onClose={() => setEditingAuthorId(null)}
          onSaved={() => { onAuthorEdited?.(); }}
        />
      )}
    </aside>
  );
}
