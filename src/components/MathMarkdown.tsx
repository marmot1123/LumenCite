import ReactMarkdown, { type Components } from "react-markdown";
import remarkMath from "remark-math";
import remarkGfm from "remark-gfm";
import rehypeKatex from "rehype-katex";
import { openUrl } from "@tauri-apps/plugin-opener";

// LLM 生成物やノートの Markdown は信頼できない内容を含み得るため、リンク・画像を安全化する（CR-026）。
// - リンク: http/https/mailto のみ許可し、WebView 遷移ではなく OS ブラウザで開く
//   （アプリ画面が外部サイトに乗っ取られるのを防ぐ）。それ以外はプレーンテキスト化。
// - 画像: https / data のみ許可し、referrer を送らない（トラッキング抑止）。それ以外は alt 表示。
function isSafeHref(href?: string): boolean {
  if (!href) return false;
  try {
    const u = new URL(href, "app://localhost/");
    return u.protocol === "http:" || u.protocol === "https:" || u.protocol === "mailto:";
  } catch {
    return false;
  }
}

const SAFE_MARKDOWN_COMPONENTS: Components = {
  a({ href, children }) {
    if (!isSafeHref(href)) {
      return <span>{children}</span>;
    }
    return (
      <a
        href={href}
        rel="noopener noreferrer"
        onClick={(e) => {
          e.preventDefault();
          void openUrl(href!).catch(() => {});
        }}
      >
        {children}
      </a>
    );
  },
  img({ src, alt }) {
    const ok = typeof src === "string" && (src.startsWith("https://") || src.startsWith("data:"));
    if (!ok) {
      return <span style={{ color: "var(--text-faint)" }}>{alt || "[image]"}</span>;
    }
    return (
      <img src={src} alt={alt} referrerPolicy="no-referrer" loading="lazy" style={{ maxWidth: "100%" }} />
    );
  },
};

interface MathMarkdownProps {
  value: string | null | undefined;
  /** value が空の時に表示するフォールバック（プレースホルダなど） */
  fallback?: React.ReactNode;
  /** 段落・コードブロックのリスト表示などを許すか。false なら単一段落として表示 */
  block?: boolean;
}

/**
 * Markdown + 数式（`$…$` 内インライン / `$$…$$` 内ディスプレイ）を描画する共通レンダラ。
 * remark-gfm が表・打ち消し線・タスクリスト等の GitHub 拡張を、remark-math が math ノードを
 * 変換し、rehype-katex が KaTeX 出力に変換する。編集時の textarea は使わず表示用途のみ。
 */
export function MathMarkdown({ value, fallback, block = true }: MathMarkdownProps) {
  if (!value || !value.trim()) {
    return <>{fallback}</>;
  }
  return (
    <div className="lc-markdown" style={{ whiteSpace: block ? "normal" : "pre-wrap" }}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkMath]}
        rehypePlugins={[rehypeKatex]}
        components={SAFE_MARKDOWN_COMPONENTS}
      >
        {value}
      </ReactMarkdown>
    </div>
  );
}
