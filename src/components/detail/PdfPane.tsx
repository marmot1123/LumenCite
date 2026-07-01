import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { TextLayer } from "pdfjs-dist";
import type { PDFDocumentProxy, PageViewport } from "pdfjs-dist";
import type { Highlight, HighlightColor, HighlightInput } from "../../types";
import type { PdfMode } from "./PdfToolbar";

interface SelectionDraft {
  page: number;
  text: string;
  x: number; y: number; width: number; height: number;
  popupX: number; popupY: number;
}

interface PdfPaneProps {
  doc: PDFDocumentProxy | null;
  loading: boolean;
  error: string | null;
  zoom: number;
  currentPage: number;
  onCurrentPageChange: (page: number) => void;
  scrollToPageKey?: number;
  mode: PdfMode;
  highlights: Highlight[];
  entryId: number;
  onCreateHighlight: (input: HighlightInput) => void;
}

const HIGHLIGHT_FILL: Record<HighlightColor, string> = {
  yellow: "oklch(0.93 0.13 95 / 0.55)",
  green:  "oklch(0.92 0.13 145 / 0.5)",
  blue:   "oklch(0.92 0.10 240 / 0.5)",
};

const PICKER_CHIPS: { id: HighlightColor; color: string }[] = [
  { id: "yellow", color: "oklch(0.85 0.15 95)" },
  { id: "green",  color: "oklch(0.78 0.13 145)" },
  { id: "blue",   color: "oklch(0.7 0.13 240)" },
];

export function PdfPane({
  doc, loading, error, zoom, currentPage, onCurrentPageChange, scrollToPageKey,
  mode, highlights, entryId, onCreateHighlight,
}: PdfPaneProps) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const pageRefs = useRef<Map<number, HTMLDivElement>>(new Map());
  const programmaticScrollPage = useRef<number | null>(null);

  const numPages = doc?.numPages ?? 0;
  const scale = zoom / 100;
  const [draft, setDraft] = useState<SelectionDraft | null>(null);
  const modeRef = useRef(mode);
  useEffect(() => { modeRef.current = mode; }, [mode]);

  useEffect(() => {
    if (!doc) return;
    const el = pageRefs.current.get(currentPage);
    if (el && containerRef.current) {
      programmaticScrollPage.current = currentPage;
      el.scrollIntoView({ behavior: "smooth", block: "start" });
    }
  }, [scrollToPageKey, doc]);

  useEffect(() => {
    if (!doc) return;
    const root = containerRef.current;
    if (!root) return;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => b.intersectionRatio - a.intersectionRatio);
        if (visible.length > 0) {
          const p = Number(visible[0].target.getAttribute("data-page"));
          if (Number.isFinite(p)) {
            if (programmaticScrollPage.current === p) {
              programmaticScrollPage.current = null;
              return;
            }
            onCurrentPageChange(p);
          }
        }
      },
      { root, threshold: [0.1, 0.5, 0.9] },
    );
    for (const el of pageRefs.current.values()) {
      observer.observe(el);
    }
    return () => observer.disconnect();
  }, [doc, numPages, onCurrentPageChange]);

  // モード変更時に draft（ポップアップ）だけ閉じる。selection は触らない。
  useEffect(() => { setDraft(null); }, [mode]);

  const handleConfirm = (color: HighlightColor) => {
    if (!draft) return;
    onCreateHighlight({
      entry_id: entryId,
      page: draft.page,
      x: draft.x,
      y: draft.y,
      width: draft.width,
      height: draft.height,
      color,
      text: draft.text,
    });
    setDraft(null);
    window.getSelection()?.removeAllRanges();
  };

  if (loading) {
    return (
      <div style={paneStyle}>
        <div style={{ color: "var(--text-mute)", fontSize: 12, marginTop: 80 }}>
          {t("detail.viewer.loading")}
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div style={paneStyle}>
        <div style={{ color: "var(--danger-strong)", marginTop: 80, maxWidth: 600, textAlign: "center" }}>
          <div style={{ marginBottom: 6, fontWeight: 600, fontSize: 13 }}>{t("detail.viewer.error")}</div>
          <div style={{ fontSize: 11.5, color: "var(--text-mute)" }}>{error}</div>
        </div>
      </div>
    );
  }

  if (!doc) {
    return (
      <div style={paneStyle}>
        <div style={{ color: "var(--text-mute)", fontSize: 12, marginTop: 80 }}>
          {t("detail.viewer.noAttachment")}
        </div>
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1, overflow: "auto", position: "relative",
        display: "flex", flexDirection: "column",
        alignItems: "center", padding: "16px 0",
        background: "oklch(0.94 0.005 80)",
      }}
    >
      {Array.from({ length: numPages }).map((_, i) => {
        const pageNum = i + 1;
        const pageHls = highlights.filter(h => h.page === pageNum);
        return (
          <PdfPage
            key={pageNum}
            doc={doc}
            page={pageNum}
            scale={scale}
            mode={mode}
            pageHighlights={pageHls}
            onTextSelected={setDraft}
            registerRef={(el) => {
              if (el) pageRefs.current.set(pageNum, el);
              else pageRefs.current.delete(pageNum);
            }}
          />
        );
      })}
      {draft && (
        <ColorPicker
          x={draft.popupX}
          y={draft.popupY}
          onPick={handleConfirm}
          onCancel={() => { setDraft(null); window.getSelection()?.removeAllRanges(); }}
        />
      )}
    </div>
  );
}

const paneStyle: React.CSSProperties = {
  flex: 1, display: "flex", flexDirection: "column", alignItems: "center",
  background: "oklch(0.94 0.005 80)",
};

function ColorPicker({ x, y, onPick, onCancel }: {
  x: number; y: number;
  onPick: (color: HighlightColor) => void;
  onCancel: () => void;
}) {
  return (
    <div
      // ピッカーをクリックしただけで selection が解除されないように mousedown のデフォルトを抑止
      onMouseDown={(e) => e.preventDefault()}
      style={{
        position: "fixed",
        left: Math.max(8, x - 60),
        top: Math.max(8, y - 40),
        zIndex: 9999,
        background: "var(--surface)",
        border: "1px solid var(--border-strong)",
        borderRadius: 6,
        boxShadow: "0 4px 16px rgba(0,0,0,0.18)",
        padding: 4,
        display: "flex", gap: 4, alignItems: "center",
      }}
    >
      {PICKER_CHIPS.map(c => (
        <button
          key={c.id}
          onClick={() => onPick(c.id)}
          aria-label={c.id}
          style={{
            width: 22, height: 22, padding: 0, border: "1px solid var(--border)",
            borderRadius: 3, background: c.color, cursor: "pointer",
          }}
        />
      ))}
      <button
        onClick={onCancel}
        aria-label="cancel"
        style={{
          width: 22, height: 22, padding: 0, border: "none",
          background: "transparent", color: "var(--text-faint)",
          cursor: "pointer", fontSize: 12,
        }}
      >×</button>
    </div>
  );
}

function PdfPage({ doc, page, scale, mode, pageHighlights, onTextSelected, registerRef }: {
  doc: PDFDocumentProxy;
  page: number;
  scale: number;
  mode: PdfMode;
  pageHighlights: Highlight[];
  onTextSelected: (draft: SelectionDraft) => void;
  registerRef: (el: HTMLDivElement | null) => void;
}) {
  const wrapperRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const textLayerRef = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState<PageViewport | null>(null);
  // selectionchange の最新スナップショット（mouseup 時点で selection が collapsed に
  // なる WKWebView 対策）
  const selSnapshotRef = useRef<{ rect: DOMRect; text: string } | null>(null);
  const modeRef = useRef(mode);
  useEffect(() => { modeRef.current = mode; }, [mode]);

  useEffect(() => {
    let cancelled = false;
    let renderTask: any;
    let textLayerInstance: TextLayer | null = null;
    (async () => {
      try {
        const p = await doc.getPage(page);
        if (cancelled) return;
        const dpr = window.devicePixelRatio || 1;
        const renderViewport = p.getViewport({ scale: scale * dpr });
        const cssViewport = p.getViewport({ scale });

        const canvas = canvasRef.current;
        if (!canvas) return;
        canvas.width = renderViewport.width;
        canvas.height = renderViewport.height;
        canvas.style.width = `${cssViewport.width}px`;
        canvas.style.height = `${cssViewport.height}px`;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;
        // @ts-ignore
        renderTask = p.render({ canvasContext: ctx, viewport: renderViewport });
        await renderTask.promise;
        if (cancelled) return;

        setViewport(cssViewport);

        // ── text layer (pdfjs-dist 公式の TextLayer を使う) ─────────────
        const textLayer = textLayerRef.current;
        if (!textLayer) return;
        textLayer.style.width = `${cssViewport.width}px`;
        textLayer.style.height = `${cssViewport.height}px`;
        // TextLayer は --total-scale-factor を CSS 変数で参照する
        textLayer.style.setProperty("--total-scale-factor", String(scale));
        textLayer.innerHTML = "";

        const textContentSource = p.streamTextContent({
          includeMarkedContent: true,
          disableNormalization: true,
        });
        textLayerInstance = new TextLayer({
          textContentSource,
          container: textLayer,
          viewport: cssViewport,
        });
        await textLayerInstance.render();
      } catch (_e) {
        // 描画キャンセル / 失敗時は無視
      }
    })();
    return () => {
      cancelled = true;
      try { renderTask?.cancel?.(); } catch { /* ignore */ }
      try { textLayerInstance?.cancel(); } catch { /* ignore */ }
    };
  }, [doc, page, scale]);

  // selectionchange を購読し、選択が確定している間は Range のスナップショットを ref に保持する。
  // WKWebView では mouseup 時点で selection が collapsed になることがあるため、
  // mouseup ではこのスナップショットを使う。
  useEffect(() => {
    const onChange = () => {
      const sel = document.getSelection();
      const textLayer = textLayerRef.current;
      if (!sel || sel.rangeCount === 0 || sel.isCollapsed || !textLayer) {
        // 選択が消えても直前のスナップショットは保持する。
        // 次の mousedown で明示的にクリアする方が安全。
        return;
      }
      const anchorIn = sel.anchorNode && textLayer.contains(sel.anchorNode);
      const focusIn = sel.focusNode && textLayer.contains(sel.focusNode);
      if (!anchorIn && !focusIn) return;
      const text = sel.toString();
      if (!text.trim()) return;
      const rect = sel.getRangeAt(0).getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) return;
      selSnapshotRef.current = { rect, text };
    };
    document.addEventListener("selectionchange", onChange);
    return () => document.removeEventListener("selectionchange", onChange);
  }, [page]);

  // モード切替時は保持中のスナップショットを破棄する。Select モードでの選択が
  // Highlight モードに残ると、無関係なクリックで古い選択のカラーピッカーが開く。
  useEffect(() => {
    selSnapshotRef.current = null;
  }, [mode]);

  // mousedown でスナップショットをクリア（新しい選択操作の開始）
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const textLayer = textLayerRef.current;
      if (textLayer && textLayer.contains(e.target as Node)) {
        selSnapshotRef.current = null;
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, []);

  // ドラッグ終了で、ハイライトモードのときに保存済みスナップショットからポップアップを出す。
  useEffect(() => {
    if (!viewport) return;
    const onUp = (e: MouseEvent) => {
      if (modeRef.current !== "highlight") return;
      const snap = selSnapshotRef.current;
      if (!snap) return;
      const wrapper = wrapperRef.current;
      if (!wrapper) return;
      const wrapperRect = wrapper.getBoundingClientRect();
      const { rect, text } = snap;
      const cssLeft = rect.left - wrapperRect.left;
      const cssTop = rect.top - wrapperRect.top;
      const [x1, y1] = viewport.convertToPdfPoint(cssLeft, cssTop);
      const [x2, y2] = viewport.convertToPdfPoint(cssLeft + rect.width, cssTop + rect.height);
      onTextSelected({
        page,
        text: text.trim(),
        x: Math.min(x1, x2),
        y: Math.min(y1, y2),
        width: Math.abs(x2 - x1),
        height: Math.abs(y2 - y1),
        popupX: e.clientX,
        popupY: e.clientY,
      });
      // スナップショットは確定後にクリア（同じ選択で連続発火しないよう）
      selSnapshotRef.current = null;
    };
    document.addEventListener("mouseup", onUp);
    return () => document.removeEventListener("mouseup", onUp);
  }, [viewport, onTextSelected, page]);

  // text layer のポインターイベント: select モードと highlight モードで有効、それ以外無効
  const textLayerInteractive = mode === "select" || mode === "highlight";

  return (
    <div
      ref={(el) => {
        wrapperRef.current = el;
        registerRef(el);
      }}
      data-page={page}
      style={{
        marginBottom: 12,
        background: "white",
        boxShadow: "0 2px 12px rgba(0,0,0,0.12)",
        position: "relative",
      }}
    >
      <canvas ref={canvasRef} style={{ display: "block" }} />

      {/* highlights overlay (canvas の上、textLayer の下) */}
      {viewport && pageHighlights.map(h => {
        const rect = viewport.convertToViewportRectangle([h.x, h.y, h.x + h.width, h.y + h.height]);
        const left = Math.min(rect[0], rect[2]);
        const top = Math.min(rect[1], rect[3]);
        const width = Math.abs(rect[2] - rect[0]);
        const height = Math.abs(rect[3] - rect[1]);
        return (
          <div
            key={h.id}
            title={h.note || h.text}
            style={{
              position: "absolute",
              left, top, width, height,
              background: HIGHLIGHT_FILL[h.color],
              pointerEvents: "none",
              borderRadius: 1,
              mixBlendMode: "multiply",
            }}
          />
        );
      })}

      {/* text layer (最前面) — pdfjs-dist 公式の .textLayer クラスに合わせる */}
      <div
        ref={textLayerRef}
        className="textLayer"
        style={{
          pointerEvents: textLayerInteractive ? "auto" : "none",
          cursor: textLayerInteractive ? "text" : "default",
        }}
      />
    </div>
  );
}
