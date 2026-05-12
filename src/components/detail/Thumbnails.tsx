import { useEffect, useRef } from "react";
import type { PDFDocumentProxy } from "pdfjs-dist";

interface ThumbnailsProps {
  doc: PDFDocumentProxy | null;
  current: number;
  onSelect: (page: number) => void;
}

export function Thumbnails({ doc, current, onSelect }: ThumbnailsProps) {
  const numPages = doc?.numPages ?? 0;

  return (
    <aside style={{
      width: 96, flexShrink: 0, height: "100%",
      background: "var(--sidebar)", borderRight: "1px solid var(--border)",
      overflow: "auto", padding: "10px 0",
    }}>
      {Array.from({ length: numPages }).map((_, i) => (
        <ThumbItem
          key={i}
          doc={doc!}
          page={i + 1}
          active={current === i + 1}
          onClick={() => onSelect(i + 1)}
        />
      ))}
    </aside>
  );
}

function ThumbItem({ doc, page, active, onClick }: {
  doc: PDFDocumentProxy;
  page: number;
  active: boolean;
  onClick: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const p = await doc.getPage(page);
        if (cancelled) return;
        const viewport = p.getViewport({ scale: 1 });
        // 72x92 程度に収めるスケール（縦長前提）
        const targetH = 92;
        const scale = targetH / viewport.height;
        const dpr = window.devicePixelRatio || 1;
        const renderViewport = p.getViewport({ scale: scale * dpr });
        const canvas = canvasRef.current;
        if (!canvas) return;
        canvas.width = renderViewport.width;
        canvas.height = renderViewport.height;
        canvas.style.width = `${renderViewport.width / dpr}px`;
        canvas.style.height = `${renderViewport.height / dpr}px`;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        // @ts-ignore - render は引数オブジェクト
        await p.render({ canvasContext: ctx, viewport: renderViewport }).promise;
      } catch (_e) {
        // キャンセル / 失敗時は無視
      }
    })();
    return () => { cancelled = true; };
  }, [doc, page]);

  return (
    <div
      onClick={onClick}
      style={{
        margin: "0 12px 8px", cursor: "pointer", textAlign: "center",
      }}
    >
      <div style={{
        width: 72, height: 92,
        background: "white",
        border: active ? "2px solid var(--accent-strong)" : "1px solid var(--border)",
        borderRadius: 2, position: "relative",
        boxShadow: "0 1px 2px rgba(0,0,0,0.04)",
        overflow: "hidden",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}>
        <canvas ref={canvasRef} style={{ display: "block" }} />
      </div>
      <div style={{
        marginTop: 3, fontSize: 10,
        color: active ? "var(--accent-strong)" : "var(--text-faint)",
        fontFamily: "var(--mono)", fontWeight: active ? 600 : 400,
      }}>{page}</div>
    </div>
  );
}
