import ReactMarkdown from "react-markdown";
import remarkMath from "remark-math";
import remarkGfm from "remark-gfm";
import rehypeKatex from "rehype-katex";

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
      >
        {value}
      </ReactMarkdown>
    </div>
  );
}
