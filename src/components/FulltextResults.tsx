import { invoke } from "@tauri-apps/api/core";
import { Icon, TypeIcon } from "./icons";
import type { FulltextHit } from "../types";

interface Props {
  hits: FulltextHit[];
  query: string;
  selectedId: number | null;
  onSelect: (id: number) => void;
}

function Snippet({ text }: { text: string }) {
  // バックエンドが ⟨…⟩ でマッチを囲むのでパースしてハイライト表示する
  const parts: { text: string; hit: boolean }[] = [];
  const re = /⟨(.*?)⟩/g;
  let lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > lastIndex) parts.push({ text: text.slice(lastIndex, m.index), hit: false });
    parts.push({ text: m[1], hit: true });
    lastIndex = m.index + m[0].length;
  }
  if (lastIndex < text.length) parts.push({ text: text.slice(lastIndex), hit: false });

  return (
    <span style={{ fontSize: 12, color: "var(--text-mute)", lineHeight: 1.55 }}>
      {parts.map((p, i) => p.hit ? (
        <mark key={i} style={{
          background: "oklch(0.92 0.13 90)", color: "var(--text)",
          padding: "0 2px", borderRadius: 2,
        }}>{p.text}</mark>
      ) : (
        <span key={i}>{p.text}</span>
      ))}
    </span>
  );
}

export function FulltextResults({ hits, query, selectedId, onSelect }: Props) {
  if (!query.trim()) {
    return (
      <div style={{
        flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
        flexDirection: "column", gap: 6, color: "var(--text-faint)",
        background: "var(--surface)",
      }}>
        <div style={{ fontSize: 14, color: "var(--text-mute)", fontWeight: 500 }}>PDF全文検索</div>
        <div style={{ fontSize: 12 }}>キーワードを入力すると本文を横断検索します</div>
      </div>
    );
  }

  if (hits.length === 0) {
    return (
      <div style={{
        flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
        flexDirection: "column", gap: 6, color: "var(--text-faint)",
        background: "var(--surface)",
      }}>
        <div style={{ fontSize: 13, color: "var(--text-mute)" }}>「{query}」 に該当する本文はありません</div>
        <div style={{ fontSize: 11 }}>PDFが添付されているか、インデックスが完了しているか確認してください</div>
      </div>
    );
  }

  return (
    <div style={{
      flex: 1, overflow: "auto", background: "var(--surface)",
      padding: "8px 0",
    }}>
      {hits.map((hit, idx) => {
        const selected = hit.entry.id === selectedId;
        return (
          <div
            key={`${hit.attachment_id}-${hit.page}-${idx}`}
            onClick={() => onSelect(hit.entry.id)}
            onDoubleClick={() => {
              invoke("open_pdf_viewer", { id: hit.attachment_id, page: hit.page }).catch(console.error);
            }}
            style={{
              padding: "10px 16px",
              borderBottom: "1px solid var(--border)",
              background: selected ? "var(--row-selected)" : "transparent",
              cursor: "pointer",
            }}
            onMouseEnter={(e) => { if (!selected) e.currentTarget.style.background = "var(--hover)"; }}
            onMouseLeave={(e) => { if (!selected) e.currentTarget.style.background = "transparent"; }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 4 }}>
              <TypeIcon type={hit.entry.entry_type} size={12} color="var(--text-faint)" />
              <span style={{ fontSize: 13, fontWeight: 600, color: "var(--text)", flex: 1, minWidth: 0,
                            overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {hit.entry.title}
              </span>
              {hit.entry.year && (
                <span style={{ fontSize: 11, color: "var(--text-faint)", fontVariantNumeric: "tabular-nums" }}>
                  {hit.entry.year}
                </span>
              )}
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  invoke("open_pdf_viewer", { id: hit.attachment_id, page: hit.page }).catch(console.error);
                }}
                style={{
                  display: "inline-flex", alignItems: "center", gap: 3,
                  padding: "2px 7px", borderRadius: 4,
                  border: "1px solid var(--border-strong)",
                  background: "var(--surface)", color: "var(--text)",
                  fontSize: 11, fontWeight: 500, cursor: "pointer",
                  fontVariantNumeric: "tabular-nums",
                }}
                title={`P.${hit.page} を開く`}
              >
                <Icon name="ext" size={10} color="var(--text-mute)" />
                P.{hit.page}
              </button>
            </div>
            {hit.entry.authors.length > 0 && (
              <div style={{ fontSize: 11, color: "var(--text-faint)", marginBottom: 4 }}>
                {hit.entry.authors.slice(0, 4).map(a => a.name).join(", ")}
                {hit.entry.authors.length > 4 && ` +${hit.entry.authors.length - 4}`}
              </div>
            )}
            <Snippet text={hit.snippet} />
          </div>
        );
      })}
    </div>
  );
}
