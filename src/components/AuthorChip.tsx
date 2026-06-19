/**
 * 著者表示用の小さなチップ。
 * - 団体著者 (`is_organization`) は建物アイコンで人名と区別
 * - 原語表記 / 読み仮名 / ORCID をホバー時に title 属性で表示
 * - クリックで `onClick` を呼ぶ（呼び出し側で AuthorEditor を開く想定）
 *
 * フォントサイズや色は `font: inherit` / `currentColor` を使うので、
 * 親の `<div>` の文字スタイルをそのまま継承する。
 */
import { Icon } from "./icons";
import type { Author } from "../types";

interface AuthorChipProps {
  author: Author;
  onClick: () => void;
}

export function AuthorChip({ author, onClick }: AuthorChipProps) {
  const tooltip = [
    author.name_original ?? null,
    author.reading_family || author.reading_given
      ? `${author.reading_family ?? ""} ${author.reading_given ?? ""}`.trim()
      : null,
    author.orcid ? `ORCID: ${author.orcid}` : null,
  ]
    .filter(Boolean)
    .join("\n");

  return (
    <button
      type="button"
      onClick={onClick}
      title={tooltip || undefined}
      style={{
        display: "inline-flex", alignItems: "center", gap: 4,
        padding: "1px 6px", borderRadius: 4,
        border: "1px solid transparent",
        background: "transparent",
        color: "inherit",
        font: "inherit",
        cursor: "pointer",
      }}
      onMouseOver={e => {
        e.currentTarget.style.borderColor = "var(--border)";
        e.currentTarget.style.background = "var(--surface-2)";
      }}
      onMouseOut={e => {
        e.currentTarget.style.borderColor = "transparent";
        e.currentTarget.style.background = "transparent";
      }}
    >
      {author.is_organization && (
        <Icon name="organization" size={11} color="var(--text-faint)" />
      )}
      <span>{author.name}</span>
    </button>
  );
}
