import ReactMarkdown from "react-markdown";
import remarkMath from "remark-math";
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
 * remark-math が math ノードに変換し、rehype-katex が KaTeX 出力に変換する。
 * 編集時の textarea は使わず、表示用途のみで利用する。
 */
export function MathMarkdown({ value, fallback, block = true }: MathMarkdownProps) {
  if (!value || !value.trim()) {
    return <>{fallback}</>;
  }
  return (
    <div className="lc-markdown" style={{ whiteSpace: block ? "normal" : "pre-wrap" }}>
      <ReactMarkdown
        remarkPlugins={[remarkMath]}
        rehypePlugins={[rehypeKatex]}
      >
        {value}
      </ReactMarkdown>
    </div>
  );
}
