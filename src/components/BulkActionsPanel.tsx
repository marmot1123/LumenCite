import { useState, useRef, useEffect } from "react";
import { Icon } from "./icons";
import type { Collection } from "../types";

interface BulkActionsPanelProps {
  width: number;
  count: number;
  inTrash: boolean;
  allCollections: Collection[];
  onClearSelection: () => void;
  onTrash: () => void;
  onRestore: () => void;
  onPurge: () => void;
  onAddToCollection: (collectionId: number) => void;
  onAddTag: (name: string) => void;
  onExportBibtex: () => void;
}

function flattenCollections(cols: Collection[], depth = 0): { col: Collection; depth: number }[] {
  return cols.flatMap(col => [
    { col, depth },
    ...flattenCollections(col.children, depth + 1),
  ]);
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div style={{
      fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
      textTransform: "uppercase", letterSpacing: "0.06em",
      marginBottom: 6,
    }}>{children}</div>
  );
}

export function BulkActionsPanel({
  width, count, inTrash, allCollections,
  onClearSelection, onTrash, onRestore, onPurge,
  onAddToCollection, onAddTag, onExportBibtex,
}: BulkActionsPanelProps) {
  const [confirmPurge, setConfirmPurge] = useState(false);
  const [tagInput, setTagInput] = useState("");
  const [collectionPicker, setCollectionPicker] = useState<number | "">("");
  const tagRef = useRef<HTMLInputElement>(null);

  // 選択件数が変わったら確認状態をリセット（別グループ操作の取り違えを防ぐ）
  useEffect(() => { setConfirmPurge(false); }, [count]);

  const flat = flattenCollections(allCollections);

  const submitTag = () => {
    const v = tagInput.trim();
    if (!v) return;
    onAddTag(v);
    setTagInput("");
  };

  return (
    <aside style={{
      width, flexShrink: 0, height: "100%",
      borderLeft: "1px solid var(--border)",
      background: "var(--surface)",
      display: "flex", flexDirection: "column",
      overflow: "hidden",
    }}>
      {/* header */}
      <div style={{
        padding: "16px 18px 14px", borderBottom: "1px solid var(--border)",
        display: "flex", alignItems: "center", gap: 10,
      }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 11, color: "var(--text-faint)", marginBottom: 2 }}>選択中</div>
          <div style={{ fontSize: 18, fontWeight: 600, color: "var(--text)" }}>
            {count} 件
          </div>
        </div>
        <button
          onClick={onClearSelection}
          title="選択を解除 (Esc)"
          style={{
            padding: "4px 8px", borderRadius: 5,
            border: "1px solid var(--border-strong)",
            background: "var(--surface)", color: "var(--text-mute)",
            fontSize: 11, cursor: "pointer",
          }}
        >解除</button>
      </div>

      {/* body */}
      <div style={{ flex: 1, overflow: "auto", padding: "16px 18px" }}>
        {inTrash ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            <button
              onClick={onRestore}
              style={{
                padding: "8px 12px", borderRadius: 6,
                border: "1px solid var(--border-strong)",
                background: "var(--surface)", color: "var(--text)",
                fontSize: 12.5, fontWeight: 500, cursor: "pointer",
                display: "inline-flex", alignItems: "center", justifyContent: "center", gap: 6,
              }}
            >
              <Icon name="ext" size={12} />
              {count} 件を復元
            </button>

            {!confirmPurge ? (
              <button
                onClick={() => setConfirmPurge(true)}
                style={{
                  padding: "8px 12px", borderRadius: 6,
                  border: "none", background: "transparent",
                  color: "oklch(0.55 0.18 15)",
                  fontSize: 12.5, cursor: "pointer",
                }}
              >{count} 件を永久削除</button>
            ) : (
              <div style={{
                padding: "10px", borderRadius: 7,
                background: "oklch(0.96 0.03 15)", border: "1px solid oklch(0.88 0.06 15)",
              }}>
                <div style={{ fontSize: 11.5, color: "oklch(0.45 0.1 15)", marginBottom: 8 }}>
                  {count} 件を完全に削除しますか？元に戻せません。
                </div>
                <div style={{ display: "flex", gap: 6 }}>
                  <button
                    onClick={() => setConfirmPurge(false)}
                    style={{
                      flex: 1, padding: "4px 10px", borderRadius: 4,
                      border: "1px solid var(--border-strong)",
                      background: "var(--surface)", color: "var(--text)",
                      fontSize: 11, cursor: "pointer",
                    }}
                  >キャンセル</button>
                  <button
                    onClick={onPurge}
                    style={{
                      flex: 1, padding: "4px 10px", borderRadius: 4, border: "none",
                      background: "oklch(0.55 0.18 15)", color: "white",
                      fontSize: 11, fontWeight: 600, cursor: "pointer",
                    }}
                  >永久削除する</button>
                </div>
              </div>
            )}
          </div>
        ) : (
          <>
            {/* primary actions */}
            <div style={{ display: "flex", flexDirection: "column", gap: 8, marginBottom: 18 }}>
              <button
                onClick={onExportBibtex}
                style={{
                  padding: "8px 12px", borderRadius: 6,
                  border: "1px solid var(--border-strong)",
                  background: "var(--surface)", color: "var(--text)",
                  fontSize: 12.5, fontWeight: 500, cursor: "pointer",
                  display: "inline-flex", alignItems: "center", justifyContent: "center", gap: 6,
                }}
              >
                <Icon name="download" size={12} />
                BibTeX を書き出し
              </button>
              <button
                onClick={onTrash}
                style={{
                  padding: "8px 12px", borderRadius: 6,
                  border: "none", background: "transparent",
                  color: "oklch(0.55 0.18 15)",
                  fontSize: 12.5, cursor: "pointer",
                }}
              >{count} 件をゴミ箱へ</button>
            </div>

            {/* collection picker */}
            <div style={{ marginBottom: 18 }}>
              <SectionLabel>コレクションに追加</SectionLabel>
              {flat.length === 0 ? (
                <div style={{ fontSize: 11.5, color: "var(--text-faint)" }}>コレクションがありません</div>
              ) : (
                <div style={{ display: "flex", gap: 6 }}>
                  <select
                    value={collectionPicker}
                    onChange={e => setCollectionPicker(e.target.value === "" ? "" : Number(e.target.value))}
                    style={{
                      flex: 1, padding: "5px 8px", fontSize: 12,
                      border: "1px solid var(--border-strong)",
                      borderRadius: 5, background: "var(--surface)", color: "var(--text)",
                    }}
                  >
                    <option value="">— 選択 —</option>
                    {flat.map(({ col, depth }) => (
                      <option key={col.id} value={col.id}>
                        {"  ".repeat(depth) + col.name}
                      </option>
                    ))}
                  </select>
                  <button
                    onClick={() => {
                      if (typeof collectionPicker === "number") {
                        onAddToCollection(collectionPicker);
                        setCollectionPicker("");
                      }
                    }}
                    disabled={collectionPicker === ""}
                    style={{
                      padding: "5px 10px", fontSize: 12,
                      border: "1px solid var(--border-strong)",
                      borderRadius: 5,
                      background: collectionPicker === "" ? "var(--surface-2)" : "var(--surface)",
                      color: collectionPicker === "" ? "var(--text-faint)" : "var(--text)",
                      cursor: collectionPicker === "" ? "default" : "pointer",
                    }}
                  >追加</button>
                </div>
              )}
            </div>

            {/* tag input */}
            <div>
              <SectionLabel>タグを追加</SectionLabel>
              <div style={{ display: "flex", gap: 6 }}>
                <input
                  ref={tagRef}
                  value={tagInput}
                  onChange={e => setTagInput(e.target.value)}
                  onKeyDown={e => {
                    if (e.key === "Enter") { submitTag(); e.preventDefault(); }
                  }}
                  placeholder="タグ名…"
                  style={{
                    flex: 1, padding: "5px 8px", fontSize: 12,
                    border: "1px solid var(--border-strong)",
                    borderRadius: 5, background: "var(--surface)", color: "var(--text)",
                  }}
                />
                <button
                  onClick={submitTag}
                  disabled={!tagInput.trim()}
                  style={{
                    padding: "5px 10px", fontSize: 12,
                    border: "1px solid var(--border-strong)",
                    borderRadius: 5,
                    background: !tagInput.trim() ? "var(--surface-2)" : "var(--surface)",
                    color: !tagInput.trim() ? "var(--text-faint)" : "var(--text)",
                    cursor: !tagInput.trim() ? "default" : "pointer",
                  }}
                >追加</button>
              </div>
            </div>
          </>
        )}
      </div>
    </aside>
  );
}
