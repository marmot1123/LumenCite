import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import * as pdfjsLib from "pdfjs-dist";
import type { PDFDocumentProxy, PageViewport } from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?url";

pdfjsLib.GlobalWorkerOptions.workerSrc = PdfWorker;

interface Props {
  attachmentId: number;
  initialPage?: number;
  /** 一時強調する領域 [x, y, width, height]（PDF user space・左下原点・pt）。 */
  initialRegion?: [number, number, number, number];
}

/** 強調中の領域（ページ + 矩形）。永続化しない。 */
interface FocusRegion {
  page: number;
  rect: [number, number, number, number];
}

interface PageInfo {
  pageNumber: number;
  width: number;
  height: number;
}

const SCALE_STEPS = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const DEFAULT_SCALE = 1.25;

export function PdfViewer({ attachmentId, initialPage, initialRegion }: Props) {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null);
  const [pages, setPages] = useState<PageInfo[]>([]);
  const [scale, setScale] = useState<number>(DEFAULT_SCALE);
  const [currentPage, setCurrentPage] = useState<number>(1);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const initialJumpDone = useRef(false);
  // 根拠領域の強調（Phase 10b）。**initialPage のジャンプ処理とは独立に持つ** —
  // あちらは `page > 1` かつ初回だけという条件付きなので、1 ページ目の根拠や
  // 2 回目のジャンプで強調が出なくなる。
  const [focus, setFocus] = useState<FocusRegion | null>(
    initialRegion && initialPage ? { page: initialPage, rect: initialRegion } : null,
  );

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
        setFocus(null);
      }
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, [pages.length]);

  // ── 根拠領域へのジャンプ（チャットのツール結果から・Phase 10b） ────────
  useEffect(() => {
    const win = getCurrentWebviewWindow();
    const unlistenPromise = win.listen<{ page: number; region: number[] }>("jump-to-region", (e) => {
      const p = Number(e.payload?.page);
      const r = e.payload?.region;
      if (!Number.isFinite(p) || p < 1 || p > pages.length) return;
      scrollToPage(p);
      setCurrentPage(p);
      setFocus(
        Array.isArray(r) && r.length === 4 && r.every(Number.isFinite)
          ? { page: p, rect: r as [number, number, number, number] }
          : null,
      );
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, [pages.length]);

  // 初回ロード時に URL 由来の領域へスクロールする（`initialPage` の効果は
  // page > 1 のときしか走らないので、1 ページ目の根拠のためにここでも面倒を見る）。
  useEffect(() => {
    if (!doc || !focus) return;
    requestAnimationFrame(() => {
      scrollToPage(focus.page, "auto");
      setCurrentPage(focus.page);
    });
    // 初回だけ。以降は jump-to-region が自分でスクロールする。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doc]);

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
      // 入力欄（ページ番号など）編集中は矢印キーをカーソル移動に譲る
      const target = e.target as HTMLElement | null;
      const editing = target && (
        target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable
      );
      if (editing) return;
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
        <ToolbarButton onClick={() => setScale(DEFAULT_SCALE)}>{t("pdfViewer.reset")}</ToolbarButton>
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
        {loading && <div style={{ color: "#999", marginTop: 80 }}>{t("pdfViewer.loading")}</div>}
        {error && (
          <div style={{ color: "#f87171", marginTop: 80, maxWidth: 600, textAlign: "center" }}>
            <div style={{ marginBottom: 6, fontWeight: 600 }}>{t("pdfViewer.error")}</div>
            <div style={{ fontSize: 11.5, color: "#fca5a5" }}>{error}</div>
          </div>
        )}
        {doc && pages.map((p) => (
          <PdfPage
            key={p.pageNumber}
            doc={doc}
            page={p.pageNumber}
            scale={scale}
            focusRect={focus?.page === p.pageNumber ? focus.rect : null}
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

function PdfPage({ doc, page, scale, focusRect, registerRef }: {
  doc: PDFDocumentProxy;
  page: number;
  scale: number;
  /** 強調する領域 [x, y, width, height]（PDF pt・左下原点）。このページに無ければ null。 */
  focusRect: [number, number, number, number] | null;
  registerRef: (el: HTMLDivElement | null) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // CSS 座標へ変換するための viewport。DetailPane の PdfPane と同じく、
  // pdf.js に y 反転・回転・CropBox 原点の面倒を見てもらう。
  const [viewport, setViewport] = useState<PageViewport | null>(null);

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
        if (!cancelled) setViewport(cssViewport);
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
      {viewport && focusRect && <FocusOverlay viewport={viewport} rect={focusRect} />}
      <div style={{
        position: "absolute", left: 8, bottom: 6,
        fontSize: 10, color: "rgba(0,0,0,0.4)",
        background: "rgba(255,255,255,0.6)",
        padding: "1px 6px", borderRadius: 3,
      }}>{page}</div>
    </div>
  );
}

/**
 * 根拠領域の一時強調（Phase 10b）。**永続ハイライトとは別物**なので枠線で描き、
 * 塗りは薄くする。座標変換は `PdfPane` の highlights overlay と同一
 * （LCIR の bbox は既存ハイライトと同じ PDF user space・左下原点・pt）。
 *
 * **ページ外を覆うスクリムは使わない** — このオーバーレイはページ要素の子なので、
 * 巨大な `box-shadow` で外周を暗くすると DOM 順で後ろに来るページの上には乗らず、
 * 「焦点より前のページだけが暗い」という壊れた見え方になる。枠線と塗りで足りる。
 */
function FocusOverlay({ viewport, rect }: {
  viewport: PageViewport;
  rect: [number, number, number, number];
}) {
  const [x, y, w, h] = rect;
  const r = viewport.convertToViewportRectangle([x, y, x + w, y + h]);
  const left = Math.min(r[0], r[2]);
  const top = Math.min(r[1], r[3]);
  const width = Math.abs(r[2] - r[0]);
  const height = Math.abs(r[3] - r[1]);
  return (
    <div
      style={{
        position: "absolute",
        left, top, width, height,
        border: "2px solid oklch(0.72 0.17 60)",
        background: "oklch(0.85 0.15 85 / 0.22)",
        borderRadius: 2,
        pointerEvents: "none",
      }}
    />
  );
}
