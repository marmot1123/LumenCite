import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import * as pdfjsLib from "pdfjs-dist";
import type { PDFDocumentProxy } from "pdfjs-dist";
import PdfWorker from "pdfjs-dist/build/pdf.worker.min.mjs?url";

import { Header } from "./Header";
import { PdfToolbar, type PdfMode } from "./PdfToolbar";
import { Thumbnails } from "./Thumbnails";
import { PdfPane } from "./PdfPane";
import { MetaPanel, type MetaTabId } from "./MetaPanel";
import { Icon } from "../icons";
import { pickAndAttachPdf } from "../../lib/attachments";
import type { EntryDetail, Highlight, HighlightInput } from "../../types";

pdfjsLib.GlobalWorkerOptions.workerSrc = PdfWorker;

interface DetailViewProps {
  entry: EntryDetail;
  onBack: () => void;
  onToggleStar: () => void;
  onUpdateNotes: (notes: string) => void;
  onSelectEntry: (id: number) => void;
  onOpenInWindow?: (attachmentId: number) => void;
  onSummarize: () => void;
  onChat: () => void;
  /** info タブの AuthorEditor で著者が更新された後、親で entry の再フェッチを行う。 */
  onAuthorEdited?: () => void;
  /** リーダー内で PDF を追加した後、親で entry の再フェッチを行う。 */
  onAttachmentsChanged?: () => void;
}

function readNum(key: string, fallback: number): number {
  try {
    const v = localStorage.getItem(key);
    if (v === null) return fallback;
    const n = parseFloat(v);
    return Number.isFinite(n) ? n : fallback;
  } catch {
    return fallback;
  }
}

function readBool(key: string, fallback: boolean): boolean {
  try {
    const v = localStorage.getItem(key);
    if (v === null) return fallback;
    return v === "true";
  } catch {
    return fallback;
  }
}

function readMetaTab(): MetaTabId {
  try {
    const v = localStorage.getItem("lc-detail-metaTab");
    if (v === "info" || v === "highlights" || v === "notes" || v === "related") return v;
  } catch { /* noop */ }
  return "info";
}

export function DetailView({
  entry, onBack, onToggleStar, onUpdateNotes, onSelectEntry, onOpenInWindow,
  onSummarize,
  onChat,
  onAuthorEdited,
  onAttachmentsChanged,
}: DetailViewProps) {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<PDFDocumentProxy | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const [scrollTick, setScrollTick] = useState(0);
  const [ocrBusy, setOcrBusy] = useState(false);
  const [ocrMsg, setOcrMsg] = useState<string | null>(null);

  // 複数添付の切替。null のときは先頭（primary）を表示する。
  const [selectedAttachmentId, setSelectedAttachmentId] = useState<number | null>(null);
  const [attaching, setAttaching] = useState(false);
  const [attachError, setAttachError] = useState<string | null>(null);

  // 表示中の添付。複数あれば選択中の 1 件、無選択なら先頭（primary）を開く。
  const attachments = entry.attachments;
  const activeAttachment =
    attachments.find(a => a.id === selectedAttachmentId) ?? attachments[0] ?? null;
  const activeIndex = activeAttachment
    ? attachments.findIndex(a => a.id === activeAttachment.id)
    : -1;
  const isPrimaryActive = activeAttachment != null && activeAttachment.id === attachments[0]?.id;

  const [zoom, setZoom] = useState<number>(() => readNum("lc-detail-zoom", 100));
  const [leftOpen, setLeftOpen] = useState<boolean>(() => readBool("lc-detail-leftOpen", true));
  const [rightOpen, setRightOpen] = useState<boolean>(() => readBool("lc-detail-rightOpen", true));
  const [metaTab, setMetaTab] = useState<MetaTabId>(() => readMetaTab());

  const [mode, setMode] = useState<PdfMode>("select");
  const [search, setSearch] = useState("");
  const [highlights, setHighlights] = useState<Highlight[]>([]);

  // 状態永続化
  useEffect(() => { try { localStorage.setItem("lc-detail-zoom", String(zoom)); } catch { /* noop */ } }, [zoom]);
  useEffect(() => { try { localStorage.setItem("lc-detail-leftOpen", String(leftOpen)); } catch { /* noop */ } }, [leftOpen]);
  useEffect(() => { try { localStorage.setItem("lc-detail-rightOpen", String(rightOpen)); } catch { /* noop */ } }, [rightOpen]);
  useEffect(() => { try { localStorage.setItem("lc-detail-metaTab", metaTab); } catch { /* noop */ } }, [metaTab]);

  // エントリごとの last_page をロード / 保存
  const lastPageLoaded = useRef(false);
  const lastPersistedPage = useRef<number>(-1);
  useEffect(() => {
    invoke<string | null>("get_setting", { key: `pdf.last_page.${entry.id}` })
      .then(v => {
        const n = v ? parseInt(v, 10) : NaN;
        if (Number.isFinite(n) && n > 0) {
          lastPersistedPage.current = n; // 保存済みの値なので再書き込み不要
          setPage(n);
          setScrollTick(t => t + 1);
        }
      })
      .catch(() => { /* noop */ })
      .finally(() => { lastPageLoaded.current = true; });
  }, [entry.id]);

  // page が変わったら DB の last_page を更新（簡易デバウンス）。
  // 保存値のロード完了前は書き込まない（初期値 1 で保存値を潰さないため）。
  // last_page はエントリ単位（primary PDF 基準）なので、補助添付を閲覧中は
  // 本文の記憶ページを潰さないよう書き込まない。
  useEffect(() => {
    if (!lastPageLoaded.current || lastPersistedPage.current === page || !isPrimaryActive) return;
    const handle = setTimeout(() => {
      lastPersistedPage.current = page;
      invoke("set_setting", { key: `pdf.last_page.${entry.id}`, value: String(page) }).catch(() => { /* noop */ });
    }, 500);
    return () => clearTimeout(handle);
  }, [entry.id, page, isPrimaryActive]);

  // 添付切替: 補助添付はページ 1 から表示する。primary へ戻るときは
  // last_page（primary 基準で永続化）を復元し、記憶ページを 1 で潰さない。
  const handleSelectAttachment = useCallback((id: number) => {
    setSelectedAttachmentId(id);
    const backToPrimary = id === attachments[0]?.id;
    const nextPage = backToPrimary && lastPersistedPage.current > 0
      ? lastPersistedPage.current
      : 1;
    setPage(nextPage);
    setScrollTick(t => t + 1);
  }, [attachments]);

  // リーダー内から PDF を追加（本文＋補助資料など）。追加分を即座に表示する。
  const handleAttachPdf = useCallback(async () => {
    if (attaching) return;
    setAttachError(null);
    try {
      setAttaching(true);
      const att = await pickAndAttachPdf(entry.id);
      if (!att) return;
      setSelectedAttachmentId(att.id);
      setPage(1);
      setScrollTick(t => t + 1);
      onAttachmentsChanged?.();
    } catch (e: any) {
      setAttachError(e?.message ?? String(e));
    } finally {
      setAttaching(false);
    }
  }, [entry.id, attaching, onAttachmentsChanged]);

  // OCR（スキャン PDF を Vision で文字起こしして全文検索に取り込む）
  const handleOcr = useCallback(async () => {
    setOcrBusy(true);
    setOcrMsg(t("detail.header.ocrRunning"));
    try {
      const summary = await invoke<string>("ocr_pdf", { entryId: entry.id });
      setOcrMsg(t("detail.header.ocrDone", { summary }));
    } catch (e) {
      const msg = typeof e === "string" ? e : (e as Error)?.message ?? String(e);
      setOcrMsg(t("detail.header.ocrError", { error: msg }));
    } finally {
      setOcrBusy(false);
    }
  }, [entry.id, t]);
  useEffect(() => {
    if (!activeAttachment) {
      setDoc(null);
      return;
    }
    let cancelled = false;
    let loaded: PDFDocumentProxy | null = null;
    setLoading(true);
    setLoadError(null);
    (async () => {
      try {
        const bytes = await invoke<number[] | Uint8Array>("read_attachment_bytes", {
          id: activeAttachment.id,
        });
        const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        const task = pdfjsLib.getDocument({ data });
        const pdf = await task.promise;
        if (cancelled) { pdf.destroy(); return; }
        loaded = pdf;
        setDoc(pdf);
      } catch (e: any) {
        if (!cancelled) setLoadError(e?.message ?? String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; if (loaded) loaded.destroy(); };
  }, [activeAttachment?.id]);

  // ハイライト読み込み
  const reloadHighlights = useCallback(() => {
    invoke<Highlight[]>("get_highlights", { entryId: entry.id })
      .then(setHighlights)
      .catch(() => { /* noop */ });
  }, [entry.id]);

  useEffect(() => { reloadHighlights(); }, [reloadHighlights]);

  // ページ手動切替（ツールバー / サムネ クリック）でスクロール
  const handlePageRequest = useCallback((p: number) => {
    setPage(p);
    setScrollTick(t => t + 1);
  }, []);

  // pdf.js で全ページを画像化し、印刷専用 div に並べて window.print() を呼ぶ。
  // WKWebView は <iframe src=*.pdf> の中身（PDF プラグイン）に contentWindow.print() を
  // 渡さないため、本体ウィンドウで印刷する方式を採る。
  // 各画像は <div class="lc-print-page"> でラップしないと WebKit が改ページを無視する。
  const [printing, setPrinting] = useState(false);
  const handlePrint = useCallback(async () => {
    if (!doc || printing) return;
    setPrinting(true);
    try {
      const pageImages: string[] = [];
      for (let i = 1; i <= doc.numPages; i++) {
        const p = await doc.getPage(i);
        const viewport = p.getViewport({ scale: 2 });
        const canvas = document.createElement("canvas");
        canvas.width = viewport.width;
        canvas.height = viewport.height;
        const ctx = canvas.getContext("2d");
        if (!ctx) continue;
        // @ts-ignore
        await p.render({ canvasContext: ctx, viewport }).promise;
        pageImages.push(canvas.toDataURL("image/png"));
      }

      const root = document.createElement("div");
      root.className = "lc-print-root";
      root.innerHTML = pageImages
        .map(src => `<div class="lc-print-page"><img src="${src}" alt="" /></div>`)
        .join("");
      document.body.appendChild(root);

      const prevTitle = document.title;
      document.title = entry.title;
      const cleanup = () => {
        try { document.body.removeChild(root); } catch { /* ignore */ }
        document.title = prevTitle;
      };

      // 画像のデコード完了を待つ（dataURL でも decode() で確実に）
      const imgs = Array.from(root.querySelectorAll("img")) as HTMLImageElement[];
      await Promise.all(imgs.map(img =>
        img.decode().catch(() => undefined)
      ));
      // レイアウト確定のため 2 フレーム待つ
      await new Promise<void>(res => requestAnimationFrame(() => requestAnimationFrame(() => res())));

      window.addEventListener("afterprint", cleanup, { once: true });
      window.setTimeout(cleanup, 120_000);
      window.print();
    } catch (e) {
      console.error("print failed:", e);
    } finally {
      setPrinting(false);
    }
  }, [doc, entry.title, printing]);

  // キーボードショートカット
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      const editable = target && (
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.isContentEditable
      );

      if (e.key === "Escape" && !editable) {
        onBack();
        return;
      }
      if (e.metaKey || e.ctrlKey) {
        if (e.key === "+" || e.key === "=") { setZoom(z => Math.min(200, z + 10)); e.preventDefault(); return; }
        if (e.key === "-")                   { setZoom(z => Math.max(50, z - 10));  e.preventDefault(); return; }
        if (e.key === "0")                   { setZoom(100); e.preventDefault(); return; }
        if (e.key === "[")                   { setLeftOpen(v => !v); e.preventDefault(); return; }
        if (e.key === "]")                   { setRightOpen(v => !v); e.preventDefault(); return; }
        if (e.key === "p" || e.key === "P")  { e.preventDefault(); void handlePrint(); return; }
      }
      if (editable) return;
      if (e.key === "ArrowRight" || e.key === "PageDown") {
        if (doc && page < doc.numPages) handlePageRequest(page + 1);
        return;
      }
      if (e.key === "ArrowLeft" || e.key === "PageUp") {
        if (page > 1) handlePageRequest(page - 1);
        return;
      }
      if ((e.key === "h" || e.key === "H") && !editable) { setMode("highlight"); return; }
      if ((e.key === "n" || e.key === "N") && !editable) { setMode("note"); return; }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [doc, page, handlePageRequest, onBack, handlePrint]);

  const handleDeleteHighlight = (id: number) => {
    invoke("delete_highlight", { id })
      .then(reloadHighlights)
      .catch(console.error);
  };

  const handleCreateHighlight = (input: HighlightInput) => {
    invoke<Highlight>("create_highlight", { input })
      .then(reloadHighlights)
      .catch(console.error);
  };

  return (
    <div style={{
      width: "100%", height: "100%", background: "var(--bg)", color: "var(--text)",
      display: "flex", flexDirection: "column", overflow: "hidden",
    }}>
      <Header
        entry={entry}
        onBack={onBack}
        onToggleStar={onToggleStar}
        onSummarize={onSummarize}
        onChat={onChat}
        onOcr={activeAttachment ? handleOcr : undefined}
        ocrBusy={ocrBusy}
        onDownload={activeAttachment ? () => onOpenInWindow?.(activeAttachment.id) : undefined}
        onPrint={activeAttachment ? handlePrint : undefined}
      />
      {ocrMsg && (
        <div
          onClick={() => setOcrMsg(null)}
          style={{ flexShrink: 0, padding: "6px 14px", fontSize: 12, cursor: "pointer", background: "var(--surface-2)", borderBottom: "1px solid var(--border)", color: "var(--text-mute)" }}
        >
          {ocrMsg}
        </div>
      )}
      <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
        <div style={{
          flex: 1, display: "flex", flexDirection: "column", minWidth: 0,
          background: "oklch(0.94 0.005 80)",
        }}>
          <PdfToolbar
            page={page}
            pages={doc?.numPages ?? 0}
            onPageChange={handlePageRequest}
            zoom={zoom}
            onZoomChange={setZoom}
            search={search}
            onSearchChange={setSearch}
            mode={mode}
            onModeChange={setMode}
            leftOpen={leftOpen}
            onLeftOpenChange={setLeftOpen}
            rightOpen={rightOpen}
            onRightOpenChange={setRightOpen}
            onOpenInWindow={activeAttachment ? () => onOpenInWindow?.(activeAttachment.id) : undefined}
          />
          {attachments.length >= 1 && (
            <div style={{
              flexShrink: 0, display: "flex", alignItems: "center", gap: 8,
              padding: "5px 10px", borderBottom: "1px solid var(--border)",
              background: "var(--surface-2)", fontSize: 12, color: "var(--text)",
            }}>
              <Icon name="paperclip" size={12} color="var(--text-faint)" />
              {attachments.length >= 2 ? (
                <>
                  <select
                    aria-label={t("detail.attachments.select")}
                    value={activeAttachment?.id ?? ""}
                    onChange={e => handleSelectAttachment(Number(e.target.value))}
                    title={activeAttachment?.file_name}
                    style={{
                      maxWidth: 360, minWidth: 0, padding: "2px 6px", borderRadius: 5,
                      border: "1px solid var(--border)", background: "var(--surface)",
                      color: "var(--text)", fontSize: 12,
                    }}
                  >
                    {attachments.map(att => (
                      <option key={att.id} value={att.id}>{att.file_name}</option>
                    ))}
                  </select>
                  <span style={{ color: "var(--text-faint)", flexShrink: 0 }}>
                    {t("detail.attachments.position", { current: activeIndex + 1, total: attachments.length })}
                  </span>
                </>
              ) : (
                <span
                  title={activeAttachment?.file_name}
                  style={{
                    flex: 1, minWidth: 0, color: "var(--text-mute)",
                    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
                  }}
                >{activeAttachment?.file_name}</span>
              )}
              <div style={{ flex: 1 }} />
              <button
                onClick={handleAttachPdf}
                disabled={attaching}
                style={{
                  flexShrink: 0, display: "flex", alignItems: "center", gap: 5,
                  padding: "3px 9px", borderRadius: 5, border: "1px solid var(--border)",
                  background: "var(--surface)", color: "var(--text)", fontSize: 12,
                  cursor: attaching ? "default" : "pointer", opacity: attaching ? 0.6 : 1,
                }}
              >
                <Icon name="plus" size={11} color="var(--text-mute)" />
                {attaching ? t("detail.attachments.adding") : t("detail.attachments.add")}
              </button>
            </div>
          )}
          {attachError && (
            <div
              onClick={() => setAttachError(null)}
              style={{
                flexShrink: 0, padding: "5px 12px", fontSize: 12, cursor: "pointer",
                background: "var(--surface-2)", borderBottom: "1px solid var(--border)",
                color: "var(--danger, #c0392b)",
              }}
            >{t("detail.attachments.addError", { error: attachError })}</div>
          )}
          <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
            {leftOpen && doc && (
              <Thumbnails doc={doc} current={page} onSelect={handlePageRequest} />
            )}
            <PdfPane
              doc={doc}
              loading={loading}
              error={loadError}
              zoom={zoom}
              currentPage={page}
              onCurrentPageChange={setPage}
              scrollToPageKey={scrollTick}
              mode={mode}
              highlights={highlights}
              entryId={entry.id}
              onCreateHighlight={handleCreateHighlight}
            />
          </div>
        </div>
        {rightOpen && (
          <MetaPanel
            entry={entry}
            tab={metaTab}
            onTabChange={setMetaTab}
            highlights={highlights}
            onJumpToPage={handlePageRequest}
            onDeleteHighlight={handleDeleteHighlight}
            onUpdateNotes={onUpdateNotes}
            onSelectEntry={onSelectEntry}
            onAuthorEdited={onAuthorEdited}
          />
        )}
      </div>
    </div>
  );
}
