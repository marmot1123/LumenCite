import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import * as pdfjsLib from "pdfjs-dist";
import type { PDFDocumentProxy } from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjsLib.GlobalWorkerOptions.workerSrc = PdfWorker;

interface Props {
  attachmentId: number;
  initialPage?: number;
}

interface PageInfo {
  pageNumber: number;
  width: number;
  height: number;
}

const SCALE_STEPS = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const DEFAULT_SCALE = 1.25;

export function PdfViewer({ attachmentId, initialPage }: Props) {
  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null);
  const [pages, setPages] = useState<PageInfo[]>([]);
  const [scale, setScale] = useState<number>(DEFAULT_SCALE);
  const [currentPage, setCurrentPage] = useState<number>(1);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const initialJumpDone = useRef(false);

  const containerRef = useRef<HTMLDivElement>(null);
  const pageRefs = useRef<Map<number, HTMLDivElement>>(new Map());

  // ── PDF を読み込む ──────────────────────────────────────────────────────
  useEffect(() => {
    let cancelled = false;
    let loadedDoc: PDFDocumentProxy | null = null;

    (async () => {
      try {
        setLoading(true);
        const bytes = await invoke<number[] | Uint8Array>("read_attachment_bytes", {
          id: attachmentId,
        });
        // Tauri は Vec<u8> を number[] として返すため Uint8Array に変換する
        const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        const task = pdfjsLib.getDocument({ data });
        const pdf = await task.promise;
        if (cancelled) {
          pdf.destroy();
          return;
        }
        loadedDoc = pdf;

        // 1ページ目から物理サイズを取得（残りは描画時に取る）
        const first = await pdf.getPage(1);
        const viewport = first.getViewport({ scale: 1 });
        const initial: PageInfo[] = [{
          pageNumber: 1,
          width: viewport.width,
          height: viewport.height,
        }];
        for (let i = 2; i <= pdf.numPages; i++) {
          initial.push({ pageNumber: i, width: viewport.width, height: viewport.height });
        }
        setDoc(pdf);
        setPages(initial);
      } catch (e: any) {
        if (!cancelled) setError(e?.message ?? String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();

    return () => {
      cancelled = true;
      if (loadedDoc) loadedDoc.destroy();
    };
  }, [attachmentId]);

  // ── スクロールに応じて currentPage を更新 ───────────────────────────────
  useEffect(() => {
    const root = containerRef.current;
    if (!root) return;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => b.intersectionRatio - a.intersectionRatio);
        if (visible.length > 0) {
          const p = Number(visible[0].target.getAttribute("data-page"));
          if (Number.isFinite(p)) setCurrentPage(p);
        }
      },
      { root, threshold: [0.1, 0.5, 0.9] },
    );
    for (const el of pageRefs.current.values()) {
      observer.observe(el);
    }
    return () => observer.disconnect();
  }, [pages.length]);

  const scrollToPage = (p: number, behavior: ScrollBehavior = "smooth") => {
    const el = pageRefs.current.get(p);
    if (el && containerRef.current) {
      el.scrollIntoView({ behavior, block: "start" });
    }
  };

  // ── 初回ロード時に initialPage へジャンプ ─────────────────────────────
  useEffect(() => {
    if (!doc || initialJumpDone.current) return;
    if (initialPage && initialPage > 1 && initialPage <= pages.length) {
      // ページの DOM が生成されるのを 1 フレーム待つ
      requestAnimationFrame(() => {
        scrollToPage(initialPage, "auto");
        setCurrentPage(initialPage);
      });
    }
    initialJumpDone.current = true;
  }, [doc, pages.length, initialPage]);

  // ── 別ウィンドウから jump-to-page イベントを受ける ───────────────────
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    const unlistenPromise = win.listen<number>("jump-to-page", (e) => {
      const p = Number(e.payload);
      if (Number.isFinite(p) && p >= 1 && p <= pages.length) {
        scrollToPage(p);
        setCurrentPage(p);
      }
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, [pages.length]);

  const zoomIn = () => {
    const next = SCALE_STEPS.find((s) => s > scale);
    if (next) setScale(next);
  };
  const zoomOut = () => {
    const prev = [...SCALE_STEPS].reverse().find((s) => s < scale);
    if (prev) setScale(prev);
  };

  // ── キーボードショートカット ────────────────────────────────────────────
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey) {
        if (e.key === "+" || e.key === "=") { zoomIn(); e.preventDefault(); }
        if (e.key === "-")                   { zoomOut(); e.preventDefault(); }
        if (e.key === "0")                   { setScale(DEFAULT_SCALE); e.preventDefault(); }
      }
      if (e.key === "PageDown" || e.key === "ArrowRight") {
        if (currentPage < pages.length) scrollToPage(currentPage + 1);
      }
      if (e.key === "PageUp" || e.key === "ArrowLeft") {
        if (currentPage > 1) scrollToPage(currentPage - 1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [currentPage, pages.length, scale]);

  return (
    <div style={{
      display: "flex", flexDirection: "column",
      width: "100vw", height: "100vh",
      background: "#2b2b2b", color: "#e8e8e8",
      fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif",
    }}>
      {/* toolbar */}
      <div style={{
        display: "flex", alignItems: "center", gap: 8,
        padding: "8px 14px", background: "#222",
        borderBottom: "1px solid #111", fontSize: 12.5,
        flexShrink: 0,
      }}>
        <ToolbarButton onClick={() => scrollToPage(Math.max(1, currentPage - 1))} disabled={currentPage <= 1}>‹</ToolbarButton>
        <PageInput
          value={currentPage}
          total={pages.length}
          onChange={(p) => { setCurrentPage(p); scrollToPage(p); }}
        />
        <ToolbarButton onClick={() => scrollToPage(Math.min(pages.length, currentPage + 1))} disabled={currentPage >= pages.length}>›</ToolbarButton>

        <div style={{ width: 1, height: 18, background: "#444", margin: "0 4px" }} />

        <ToolbarButton onClick={zoomOut} disabled={scale <= SCALE_STEPS[0]}>−</ToolbarButton>
        <span style={{ minWidth: 48, textAlign: "center", color: "#bbb" }}>
          {Math.round(scale * 100)}%
        </span>
        <ToolbarButton onClick={zoomIn} disabled={scale >= SCALE_STEPS[SCALE_STEPS.length - 1]}>+</ToolbarButton>
        <ToolbarButton onClick={() => setScale(DEFAULT_SCALE)}>リセット</ToolbarButton>
      </div>

      {/* viewport */}
      <div
        ref={containerRef}
        style={{
          flex: 1, overflow: "auto",
          display: "flex", flexDirection: "column",
          alignItems: "center", padding: "16px 0",
          background: "#2b2b2b",
        }}
      >
        {loading && <div style={{ color: "#999", marginTop: 80 }}>読み込み中…</div>}
        {error && (
          <div style={{ color: "#f87171", marginTop: 80, maxWidth: 600, textAlign: "center" }}>
            <div style={{ marginBottom: 6, fontWeight: 600 }}>PDFを開けませんでした</div>
            <div style={{ fontSize: 11.5, color: "#fca5a5" }}>{error}</div>
          </div>
        )}
        {doc && pages.map((p) => (
          <PdfPage
            key={p.pageNumber}
            doc={doc}
            page={p.pageNumber}
            scale={scale}
            registerRef={(el) => {
              if (el) pageRefs.current.set(p.pageNumber, el);
              else pageRefs.current.delete(p.pageNumber);
            }}
          />
        ))}
      </div>
    </div>
  );
}

function ToolbarButton({ children, onClick, disabled }: {
  children: React.ReactNode; onClick?: () => void; disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      style={{
        border: "1px solid #3a3a3a",
        background: disabled ? "#1d1d1d" : "#2e2e2e",
        color: disabled ? "#555" : "#e0e0e0",
        borderRadius: 4,
        padding: "3px 10px",
        fontSize: 12,
        cursor: disabled ? "default" : "pointer",
      }}
    >{children}</button>
  );
}

function PageInput({ value, total, onChange }: { value: number; total: number; onChange: (n: number) => void }) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);
  return (
    <div style={{ display: "inline-flex", alignItems: "center", gap: 6 }}>
      <input
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={() => {
          const n = parseInt(text, 10);
          if (Number.isFinite(n) && n >= 1 && n <= total) onChange(n);
          else setText(String(value));
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
        }}
        style={{
          width: 42, padding: "2px 6px",
          background: "#1a1a1a", color: "#e8e8e8",
          border: "1px solid #3a3a3a", borderRadius: 4,
          fontSize: 12, textAlign: "center", outline: "none",
        }}
      />
      <span style={{ color: "#888", fontSize: 12 }}>/ {total}</span>
    </div>
  );
}

function PdfPage({ doc, page, scale, registerRef }: {
  doc: PDFDocumentProxy;
  page: number;
  scale: number;
  registerRef: (el: HTMLDivElement | null) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    let cancelled = false;
    let renderTask: ReturnType<PDFDocumentProxy["getPage"]> extends Promise<infer P>
      ? P extends { render: (...args: any[]) => infer R } ? R : never
      : never;

    (async () => {
      const p = await doc.getPage(page);
      if (cancelled) return;
      const dpr = window.devicePixelRatio || 1;
      const viewport = p.getViewport({ scale: scale * dpr });
      const cssViewport = p.getViewport({ scale });

      const canvas = canvasRef.current;
      if (!canvas) return;
      canvas.width = viewport.width;
      canvas.height = viewport.height;
      canvas.style.width = `${cssViewport.width}px`;
      canvas.style.height = `${cssViewport.height}px`;

      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      // @ts-ignore - render の型は引数オブジェクト
      renderTask = p.render({ canvasContext: ctx, viewport });
      try {
        // @ts-ignore - render は { promise } を返す
        await renderTask.promise;
      } catch (_e) {
        // キャンセル時の例外は無視
      }
    })();

    return () => {
      cancelled = true;
      try {
        // @ts-ignore
        renderTask?.cancel?.();
      } catch (_e) {
        // ignore
      }
    };
  }, [doc, page, scale]);

  return (
    <div
      ref={registerRef}
      data-page={page}
      style={{
        marginBottom: 12,
        background: "white",
        boxShadow: "0 2px 12px rgba(0,0,0,0.4)",
        position: "relative",
      }}
    >
      <canvas ref={canvasRef} style={{ display: "block" }} />
      <div style={{
        position: "absolute", left: 8, bottom: 6,
        fontSize: 10, color: "rgba(0,0,0,0.4)",
        background: "rgba(255,255,255,0.6)",
        padding: "1px 6px", borderRadius: 3,
      }}>{page}</div>
    </div>
  );
}
