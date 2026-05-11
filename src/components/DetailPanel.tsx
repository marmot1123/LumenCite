import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Icon, TypeIcon } from "./icons";
import { TagPill } from "./TagPill";
import { EXTRA_FIELDS_BY_TYPE, EXTRA_FIELD_LABELS } from "../types";
import type { Attachment, Collection, EntryDetail, EntryType, Tag } from "../types";

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
}

function flattenCollections(cols: Collection[], depth = 0): { col: Collection; depth: number }[] {
  return cols.flatMap(col => [
    { col, depth },
    ...flattenCollections(col.children, depth + 1),
  ]);
}

// extra_fields を「型固有の優先順 → それ以外（アルファベット順）」で並べる
function orderedExtraFields(
  entryType: EntryType,
  extra: Record<string, string>,
): { key: string; label: string; value: string }[] {
  const defs = EXTRA_FIELDS_BY_TYPE[entryType] ?? [];
  const definedKeys = new Set(defs.map(d => d.key));
  const ordered: { key: string; label: string; value: string }[] = [];

  for (const def of defs) {
    const v = extra[def.key];
    if (v && v.trim()) ordered.push({ key: def.key, label: def.label, value: v });
  }
  const orphans = Object.entries(extra)
    .filter(([k, v]) => !definedKeys.has(k) && v?.trim())
    .sort(([a], [b]) => a.localeCompare(b));
  for (const [k, v] of orphans) {
    ordered.push({ key: k, label: EXTRA_FIELD_LABELS[k] ?? k, value: v });
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

function ActionBtn({ icon, label, primary, onClick }: {
  icon?: Parameters<typeof Icon>[0]["name"];
  label: string;
  primary?: boolean;
  onClick?: () => void;
}) {
  return (
    <button onClick={onClick} style={{
      display: "inline-flex", alignItems: "center", gap: 5,
      padding: "5px 9px", borderRadius: 5,
      border: primary ? "none" : "1px solid var(--border-strong)",
      background: primary ? "var(--accent-strong)" : "var(--surface)",
      color: primary ? "white" : "var(--text)",
      fontSize: 11.5, fontWeight: 500, cursor: "pointer",
    }}>
      {icon && <Icon name={icon} size={11} color={primary ? "white" : "var(--text-mute)"} />}
      {label}
    </button>
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

export function DetailPanel({ entry, width, inTrash, onEdit, onDelete, onRestore, onToggleStar, allCollections, onAddToCollection, onRemoveFromCollection, allTags, onAddTag, onRemoveTag, onAttachmentsChanged, onAttachmentAdded }: DetailPanelProps) {
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

  useEffect(() => {
    setConfirmDelete(false);
    setShowColDropdown(false);
    setTagInputOpen(false);
    setTagInput("");
    setAttachError(null);
  }, [entry?.id]);

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
      const path = await invoke<string | null>("pick_pdf_file");
      if (!path) return;
      const att = await invoke<Attachment>("add_attachment", { entryId: entry.id, sourcePath: path });
      onAttachmentsChanged?.();
      onAttachmentAdded?.(att.id);
    } catch (e: any) {
      setAttachError(e?.message ?? String(e));
    } finally {
      setAttaching(false);
    }
  };

  const handleOpenPdf = async () => {
    if (!entry || entry.attachments.length === 0) return;
    try {
      await invoke("open_pdf_viewer", { id: entry.attachments[0].id });
    } catch (e) {
      console.error(e);
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

  const handleCopyBibtex = async () => {
    if (!entry) return;
    try {
      const bib = await invoke<string>("export_bibtex", { entryIds: [entry.id] });
      await writeText(bib);
      setBibtexLabel("コピーしました");
      setTimeout(() => setBibtexLabel("BibTeX"), 2000);
    } catch {
      setBibtexLabel("エラー");
      setTimeout(() => setBibtexLabel("BibTeX"), 2000);
    }
  };

  if (!entry) {
    return (
      <aside style={panelStyle(width)}>
        <div style={{
          flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
          padding: 24, color: "var(--text-faint)", fontSize: 12.5, textAlign: "center", lineHeight: 1.6,
        }}>
          文献を選択すると<br/>詳細が表示されます
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
            title={entry.starred ? "お気に入りから外す" : "お気に入りに追加"}
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
          <div style={{ marginTop: 8, fontSize: 12, color: "var(--text-mute)", lineHeight: 1.5 }}>
            {entry.authors.map(a => a.name).join(", ")}
          </div>
        )}

        {!confirmDelete ? (
          <div style={{ display: "flex", gap: 6, marginTop: 12, flexWrap: "wrap", alignItems: "center" }}>
            {inTrash ? (
              <>
                <ActionBtn icon="ext" label="復元" primary onClick={onRestore} />
                <div style={{ flex: 1 }} />
                <button
                  onClick={() => setConfirmDelete(true)}
                  style={{
                    padding: "4px 8px", borderRadius: 5, border: "none",
                    background: "transparent", color: "var(--text-faint)",
                    fontSize: 11, cursor: "pointer",
                  }}
                >永久削除</button>
              </>
            ) : (
              <>
                {entry.attachments.length > 0 ? (
                  <ActionBtn icon="ext" label="PDFを開く" primary onClick={handleOpenPdf} />
                ) : (
                  <ActionBtn
                    icon="paperclip"
                    label={attaching ? "添付中…" : "PDFを添付"}
                    onClick={handleAttachPdf}
                  />
                )}
                <ActionBtn icon="sparkle" label="要約" />
                <ActionBtn icon="download" label={bibtexLabel} onClick={handleCopyBibtex} />
                <ActionBtn label="編集" onClick={onEdit} />
                <div style={{ flex: 1 }} />
                <button
                  onClick={() => setConfirmDelete(true)}
                  style={{
                    padding: "4px 8px", borderRadius: 5, border: "none",
                    background: "transparent", color: "var(--text-faint)",
                    fontSize: 11, cursor: "pointer",
                  }}
                >削除</button>
              </>
            )}
          </div>
        ) : (
          <div style={{
            display: "flex", alignItems: "center", gap: 8, marginTop: 12,
            padding: "7px 10px", borderRadius: 7,
            background: "oklch(0.96 0.03 15)", border: "1px solid oklch(0.88 0.06 15)",
          }}>
            <span style={{ fontSize: 11.5, color: "oklch(0.45 0.1 15)", flex: 1 }}>
              {inTrash ? "完全に削除しますか？元に戻せません。" : "この文献をゴミ箱へ移動しますか？"}
            </span>
            <button
              onClick={() => setConfirmDelete(false)}
              style={{
                padding: "3px 9px", borderRadius: 4,
                border: "1px solid var(--border-strong)",
                background: "var(--surface)", color: "var(--text)",
                fontSize: 11, cursor: "pointer",
              }}
            >キャンセル</button>
            <button
              onClick={onDelete}
              style={{
                padding: "3px 9px", borderRadius: 4, border: "none",
                background: "oklch(0.55 0.18 15)", color: "white",
                fontSize: 11, fontWeight: 600, cursor: "pointer",
              }}
            >{inTrash ? "永久削除する" : "ゴミ箱へ"}</button>
          </div>
        )}
      </div>

      {/* tabs */}
      <div style={{
        display: "flex", borderBottom: "1px solid var(--border)",
        padding: "0 14px", flexShrink: 0,
      }}>
        <Tab label="情報" active={tab === "info"} onClick={() => setTab("info")} />
        <Tab label="抄録" active={tab === "abstract"} onClick={() => setTab("abstract")} />
        <Tab label="ノート" active={tab === "notes"} onClick={() => setTab("notes")} />
        <Tab label="関連" active={tab === "related"} onClick={() => setTab("related")} />
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
                  {items.map(({ key, label, value }) => (
                    <Field key={key} label={label} value={value} />
                  ))}
                </div>
              );
            })()}

            <Field label="DOI" value={entry.doi} mono />
            <Field label="arXiv" value={entry.arxiv_id} mono />
            <Field label="ISBN" value={entry.isbn} mono />
            <Field label="URL" value={entry.url} mono />
            <Field label="掲載年" value={entry.year} />
            <Field label="追加日" value={entry.created_at ? entry.created_at.slice(0, 10) : undefined} />

            <div style={{ marginTop: 4, marginBottom: 14 }}>
              <div style={{
                fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
                textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
              }}>添付ファイル</div>
              <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
                {entry.attachments.map(att => (
                  <div key={att.id} style={{
                    display: "flex", alignItems: "center", gap: 6,
                    padding: "3px 4px 3px 7px", borderRadius: 5,
                    background: "var(--surface-2)", fontSize: 12,
                    color: "var(--text)",
                  }}>
                    <Icon name="paperclip" size={11} color="var(--text-faint)" />
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
                    <button
                      onClick={() => handleDeleteAttachment(att.id)}
                      style={{
                        width: 16, height: 16, padding: 0, border: "none",
                        background: "transparent", cursor: "pointer",
                        display: "inline-flex", alignItems: "center", justifyContent: "center",
                        borderRadius: 3, color: "var(--text-faint)",
                      }}
                      title="削除"
                    >
                      <Icon name="close" size={9} color="var(--text-faint)" />
                    </button>
                  </div>
                ))}
                <button
                  onClick={handleAttachPdf}
                  disabled={attaching}
                  style={{
                    display: "inline-flex", alignItems: "center", gap: 3, alignSelf: "flex-start",
                    padding: "2px 8px", borderRadius: 5,
                    border: "1px dashed var(--border-strong)",
                    background: "transparent", color: "var(--text-faint)",
                    fontSize: 11, cursor: attaching ? "default" : "pointer",
                    opacity: attaching ? 0.5 : 1,
                  }}
                >
                  <Icon name="plus" size={9} color="var(--text-faint)" />
                  {attaching ? "添付中…" : "PDFを追加"}
                </button>
                {attachError && (
                  <div style={{ fontSize: 11, color: "oklch(0.55 0.18 15)", marginTop: 2 }}>
                    {attachError}
                  </div>
                )}
              </div>
            </div>

            <div style={{ marginTop: 4 }}>
              <div style={{
                fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
                textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6,
              }}>タグ</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4, alignItems: "center" }}>
                {entry.tags.map(t => (
                  <TagPill
                    key={t.id}
                    name={t.name}
                    onRemove={() => onRemoveTag(t.id)}
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
                      追加
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
                        placeholder="タグ名…"
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
                            {suggestions.map(t => (
                              <button
                                key={t.id}
                                onMouseDown={e => { e.preventDefault(); submitTag(t.name); }}
                                style={{
                                  display: "block", width: "100%", padding: "4px 10px",
                                  border: "none", background: "transparent", textAlign: "left",
                                  fontSize: 11.5, cursor: "pointer", color: "var(--text)",
                                }}
                                onMouseEnter={e => (e.currentTarget.style.background = "var(--hover)")}
                                onMouseLeave={e => (e.currentTarget.style.background = "transparent")}
                              >{t.name}</button>
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
              }}>コレクション</div>
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
                      title="コレクションから削除"
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
                    コレクションに追加
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
                            利用可能なコレクションがありません
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
          <div style={{ fontSize: 12.5, lineHeight: 1.65, color: "var(--text)" }}>
            {entry.abstract_ || (
              <span style={{ color: "var(--text-faint)" }}>抄録は登録されていません。</span>
            )}
          </div>
        )}

        {tab === "notes" && (
          entry.notes ? (
            <div style={{ fontSize: 12.5, lineHeight: 1.65, color: "var(--text)", whiteSpace: "pre-wrap" }}>
              {entry.notes}
            </div>
          ) : (
            <div style={{ padding: "40px 0", textAlign: "center", color: "var(--text-faint)", fontSize: 12 }}>
              <div style={{ marginBottom: 8 }}>ノートはまだありません</div>
              <button style={{
                padding: "5px 11px", borderRadius: 5,
                border: "1px solid var(--border-strong)",
                background: "var(--surface)", color: "var(--text)",
                fontSize: 11.5, cursor: "pointer",
              }}>ノートを作成</button>
            </div>
          )
        )}

        {tab === "related" && (
          <div style={{ fontSize: 12, color: "var(--text-faint)", lineHeight: 1.6 }}>
            arXiv プレプリント版や、引用関係にある文献がここに表示されます。
          </div>
        )}
      </div>
    </aside>
  );
}
