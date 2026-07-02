import { Window } from "happy-dom";
import { describe, expect, it } from "vitest";
import { extractPage } from "../src/extract.js";

function docFrom(html: string): Document {
  const window = new Window();
  window.document.write(html);
  return window.document as unknown as Document;
}

describe("extractPage", () => {
  it("extracts arXiv id and pdf url from an arXiv abs page", () => {
    const doc = docFrom(`<!doctype html><html><head>
      <title>arXiv page</title>
      <meta name="citation_title" content="Attention Is All You Need" />
      <meta name="citation_arxiv_id" content="1706.03762" />
      <meta name="citation_pdf_url" content="https://arxiv.org/pdf/1706.03762" />
      <meta name="citation_doi" content="10.48550/arXiv.1706.03762" />
    </head><body></body></html>`);

    const p = extractPage(doc, "https://arxiv.org/abs/1706.03762");
    expect(p.title).toBe("Attention Is All You Need");
    expect(p.arxiv_id).toBe("1706.03762");
    expect(p.pdf_url).toBe("https://arxiv.org/pdf/1706.03762");
    expect(p.doi).toBe("10.48550/arXiv.1706.03762");
    expect(p.url).toBe("https://arxiv.org/abs/1706.03762");
  });

  it("falls back to the URL pattern when arXiv metas are absent", () => {
    const doc = docFrom(`<!doctype html><html><head><title>t</title></head><body></body></html>`);
    const p = extractPage(doc, "https://arxiv.org/pdf/2301.00001v2");
    expect(p.arxiv_id).toBe("2301.00001v2");
  });

  it("extracts DOI from publisher citation metas (with doi: prefix)", () => {
    const doc = docFrom(`<!doctype html><html><head>
      <meta name="citation_title" content="Some Paper" />
      <meta name="citation_doi" content="doi:10.1103/PhysRevLett.123.456789" />
      <meta name="citation_publication_date" content="2019/05/01" />
    </head><body></body></html>`);

    const p = extractPage(doc, "https://journals.aps.org/prl/abstract/x");
    expect(p.doi).toBe("10.1103/PhysRevLett.123.456789");
    expect(p.published_date).toBe("2019/05/01");
  });

  it("finds DOI from a doi.org canonical link", () => {
    const doc = docFrom(`<!doctype html><html><head>
      <link rel="canonical" href="https://doi.org/10.1000/xyz123" />
    </head><body></body></html>`);
    const p = extractPage(doc, "https://publisher.example/article");
    expect(p.doi).toBe("10.1000/xyz123");
  });

  it("rejects strings that are not DOIs", () => {
    const doc = docFrom(`<!doctype html><html><head>
      <meta name="DC.Identifier" content="urn:issn:1234-5678" />
    </head><body></body></html>`);
    const p = extractPage(doc, "https://example.com");
    expect(p.doi).toBeUndefined();
  });

  it("produces a plain webpage payload with og fallbacks", () => {
    const doc = docFrom(`<!doctype html><html><head>
      <title>Doc Title</title>
      <meta property="og:title" content="OG Title" />
      <meta property="og:site_name" content="Example Blog" />
      <meta property="article:published_time" content="2024-03-01T10:00:00Z" />
    </head><body></body></html>`);

    const p = extractPage(doc, "https://example.com/post");
    expect(p.title).toBe("OG Title");
    expect(p.site_name).toBe("Example Blog");
    expect(p.published_date).toBe("2024-03-01T10:00:00Z");
    expect(p.doi).toBeUndefined();
    expect(p.arxiv_id).toBeUndefined();
    expect(p.isbn).toBeUndefined();
  });
});
