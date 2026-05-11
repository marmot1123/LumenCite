import { useState } from "react";
import { Icon } from "./icons";
import type { EntrySummary } from "../types";

function CoverCard({ entry, selected, onClick }: {
  entry: EntrySummary;
  selected: boolean;
  onClick: () => void;
}) {
  const [hover, setHover] = useState(false);
  const hue = (entry.id * 47) % 360;
  const firstAuthor = entry.authors[0]?.name ?? "";
  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "flex", flexDirection: "column", gap: 8,
        padding: 10, borderRadius: 8, cursor: "pointer",
        background: selected ? "var(--row-selected)" : hover ? "var(--row-hover)" : "transparent",
        outline: selected ? "1.5px solid var(--accent-strong)" : "1.5px solid transparent",
        outlineOffset: -1,
        transition: "background 100ms ease",
      }}>
      {/* cover */}
      <div style={{
        position: "relative", aspectRatio: "0.72",
        borderRadius: 4, overflow: "hidden",
        background: `linear-gradient(160deg, oklch(0.96 0.02 ${hue}) 0%, oklch(0.88 0.04 ${hue}) 100%)`,
        boxShadow: "0 1px 0 rgba(0,0,0,0.06), 0 4px 14px rgba(20,15,8,0.10), 0 0 0 0.5px oklch(0 0 0 / 0.10)",
      }}>
        <svg width="100%" height="100%" style={{ position: "absolute", inset: 0, opacity: 0.18 }}>
          <defs>
            <pattern id={`stripe-${entry.id}`} width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <line x1="0" y1="0" x2="0" y2="6" stroke={`oklch(0.4 0.05 ${hue})`} strokeWidth="1"/>
            </pattern>
          </defs>
          <rect width="100%" height="100%" fill={`url(#stripe-${entry.id})`}/>
        </svg>

        <div style={{
          position: "absolute", top: 8, left: 8,
          padding: "1px 6px", borderRadius: 3,
          background: "rgba(255,255,255,0.85)",
          fontSize: 9, fontWeight: 600, letterSpacing: "0.04em",
          color: `oklch(0.35 0.08 ${hue})`,
          textTransform: "uppercase",
        }}>{entry.entry_type}</div>

        <div style={{
          position: "absolute", left: 12, right: 12, top: "32%",
          fontFamily: "'IBM Plex Serif', Georgia, serif",
          fontSize: 8.5, fontWeight: 600,
          color: `oklch(0.30 0.06 ${hue})`,
          lineHeight: 1.25,
          display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: 4, overflow: "hidden",
        } as React.CSSProperties}>{entry.title}</div>

        <div style={{
          position: "absolute", left: 12, right: 12, bottom: 14,
          fontSize: 7, color: `oklch(0.40 0.05 ${hue})`, fontStyle: "italic",
        }}>
          {firstAuthor}{entry.authors.length > 1 ? " et al." : ""}
        </div>

        {entry.year && <div style={{
          position: "absolute", right: 10, bottom: 6,
          fontSize: 7.5, fontFamily: "var(--mono)", color: `oklch(0.40 0.05 ${hue})`,
        }}>{entry.year}</div>}

        {entry.has_attachment && <div style={{
          position: "absolute", top: 8, right: 8,
          width: 16, height: 16, borderRadius: "50%",
          background: "rgba(255,255,255,0.85)",
          display: "flex", alignItems: "center", justifyContent: "center",
        }}><Icon name="paperclip" size={9} color={`oklch(0.35 0.08 ${hue})`} /></div>}
      </div>

      {/* meta */}
      <div style={{ minHeight: 38 }}>
        <div style={{
          fontSize: 11.5, fontWeight: 550, color: "var(--text)",
          lineHeight: 1.3, letterSpacing: "-0.005em",
          display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: 2, overflow: "hidden",
        } as React.CSSProperties}>{entry.title}</div>
        <div style={{
          marginTop: 3, fontSize: 10.5, color: "var(--text-faint)",
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
        }}>
          {firstAuthor}{entry.authors.length > 1 ? " et al." : ""}{entry.year ? ` · ${entry.year}` : ""}
        </div>
      </div>
    </div>
  );
}

export function CoversGrid({ entries, selectedId, onSelect }: {
  entries: EntrySummary[];
  selectedId: number | null;
  onSelect: (id: number) => void;
}) {
  return (
    <div style={{ flex: 1, overflow: "auto", padding: "16px 18px", background: "var(--surface)" }}>
      <div style={{ display: "grid", gap: 10, gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))" }}>
        {entries.map(e => (
          <CoverCard key={e.id} entry={e} selected={selectedId === e.id} onClick={() => onSelect(e.id)} />
        ))}
      </div>
    </div>
  );
}
