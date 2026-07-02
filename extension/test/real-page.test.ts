import { readFileSync } from "node:fs";
import { join } from "node:path";
import { Window } from "happy-dom";
import { describe, expect, it } from "vitest";
import { extractPage } from "../src/extract.js";

describe("real arXiv abs page", () => {
  it("extracts identifiers from the real HTML", () => {
    const html = readFileSync(join(__dirname, "fixtures-arxiv.html"), "utf8");
    const window = new Window();
    window.document.write(html);
    const p = extractPage(window.document as unknown as Document, "https://arxiv.org/abs/1706.03762");
    expect(p.arxiv_id).toBe("1706.03762");
    expect(p.title).toBe("Attention Is All You Need");
    expect(p.pdf_url).toBe("https://arxiv.org/pdf/1706.03762");
    // citation_author（"Family, Given"）が "Given Family" に変換されて全員入る
    expect(p.authors).toHaveLength(8);
    expect(p.authors?.[0]).toBe("Ashish Vaswani");
    expect(p.authors?.[7]).toBe("Illia Polosukhin");
  });
});
