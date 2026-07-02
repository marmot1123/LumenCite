import type { ClipPayload } from "./types.js";

/**
 * ページから識別子・タイトル等を抽出する。
 *
 * **注意**: この関数は `chrome.scripting.executeScript({ func: extractPage })` で
 * 文字列化されてページ側で実行されるため、**関数本体は完全に自己完結**でなければ
 * ならない（モジュールスコープの import / ヘルパーを参照できない）。すべての
 * ヘルパーは関数内に定義すること。引数はテスト用（ページ側では既定値が使われる）。
 */
export function extractPage(
  doc: Document = document,
  href: string = location.href,
): ClipPayload {
  const meta = (name: string): string | undefined => {
    const el =
      doc.querySelector(`meta[name="${name}"]`) ??
      doc.querySelector(`meta[property="${name}"]`);
    const v = el?.getAttribute("content")?.trim();
    return v || undefined;
  };

  // ── DOI ──────────────────────────────────────────────────────────────
  const cleanDoi = (s: string | undefined): string | undefined => {
    if (!s) return undefined;
    const t = s
      .trim()
      .replace(/^doi:/i, "")
      .replace(/^https?:\/\/(dx\.)?doi\.org\//i, "");
    return /^10\.\d{4,9}\//.test(t) ? t : undefined;
  };
  let doi = cleanDoi(meta("citation_doi")) ?? cleanDoi(meta("doi")) ?? cleanDoi(meta("DC.Identifier"));
  if (!doi) {
    // canonical / og:url / ページ URL が doi.org を指すケース
    const canonical = doc.querySelector('link[rel="canonical"]')?.getAttribute("href") ?? undefined;
    for (const candidate of [canonical, meta("og:url"), href]) {
      const hit = cleanDoi(candidate);
      if (hit) {
        doi = hit;
        break;
      }
    }
  }

  // ── arXiv ────────────────────────────────────────────────────────────
  let arxivId = meta("citation_arxiv_id");
  if (!arxivId) {
    const m = href.match(/arxiv\.org\/(?:abs|pdf)\/([0-9]{4}\.[0-9]{4,5}(?:v\d+)?|[a-z-]+(?:\.[A-Z]{2})?\/[0-9]{7})/i);
    if (m) arxivId = m[1];
  }

  // ── ISBN ─────────────────────────────────────────────────────────────
  const isbn = meta("citation_isbn");

  // ── 著者（citation_author は複数出現。"Family, Given" は "Given Family" へ） ──
  const authors = Array.from(doc.querySelectorAll('meta[name="citation_author"]'))
    .map((el) => el.getAttribute("content")?.trim() ?? "")
    .filter((s) => s.length > 0)
    .map((s) => {
      const parts = s.split(",");
      return parts.length === 2 ? `${parts[1]!.trim()} ${parts[0]!.trim()}` : s;
    });

  // ── PDF URL / タイトル / webpage フォールバック情報 ──────────────────
  const pdfUrl = meta("citation_pdf_url");
  const title = meta("citation_title") ?? meta("og:title") ?? (doc.title || undefined);
  const publishedDate =
    meta("citation_publication_date") ??
    meta("citation_date") ??
    meta("article:published_time") ??
    meta("date");
  const siteName = meta("og:site_name");

  const payload: ClipPayload = { url: href };
  if (title) payload.title = title;
  if (doi) payload.doi = doi;
  if (arxivId) payload.arxiv_id = arxivId;
  if (isbn) payload.isbn = isbn;
  if (pdfUrl) payload.pdf_url = pdfUrl;
  if (publishedDate) payload.published_date = publishedDate;
  if (siteName) payload.site_name = siteName;
  if (authors.length > 0) payload.authors = authors;
  return payload;
}
