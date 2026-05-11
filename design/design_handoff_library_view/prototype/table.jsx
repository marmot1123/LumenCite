// LumenCite — Entries Table

const { useState: useStateT, useMemo: useMemoT } = React;
const { Icon: IconT, TypeIcon: TypeIconT, tagColor: tagColorT } = window.LumenCommon;

function ColumnHeader({ label, width, sortable, sorted, onSort, align = "left", noBorder }) {
  return (
    <div
      onClick={sortable ? onSort : undefined}
      style={{
        width, flexShrink: 0,
        padding: "0 10px", height: 28,
        display: "flex", alignItems: "center", gap: 4,
        justifyContent: align === "right" ? "flex-end" : "flex-start",
        fontSize: 11, fontWeight: 600, color: "var(--text-mute)",
        letterSpacing: "0.02em",
        borderRight: noBorder ? "none" : "1px solid var(--border)",
        cursor: sortable ? "pointer" : "default",
        userSelect: "none",
      }}
    >
      <span>{label}</span>
      {sorted && (
        <span style={{ color: "var(--accent-strong)", display: "inline-flex" }}>
          <svg width="9" height="9" viewBox="0 0 9 9" fill="none">
            {sorted === "asc"
              ? <path d="M2 6l2.5-3 2.5 3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
              : <path d="M2 3l2.5 3 2.5-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>}
          </svg>
        </span>
      )}
    </div>
  );
}

function TagPill({ name }) {
  const c = tagColorT(name);
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 4,
      padding: "1px 7px 1px 6px", borderRadius: 999,
      background: c.bg, color: c.fg,
      fontSize: 10.5, fontWeight: 500, letterSpacing: "0.01em",
      whiteSpace: "nowrap",
    }}>
      <span style={{ width: 5, height: 5, borderRadius: 999, background: c.dot }} />
      {name}
    </span>
  );
}

function formatAuthors(authors) {
  if (!authors || !authors.length) return "—";
  if (authors.length === 1) return authors[0];
  if (authors.length === 2) return authors[0] + ", " + authors[1];
  return authors[0] + " et al.";
}

function fmtDate(s) {
  if (!s) return "";
  const d = new Date(s);
  return d.getFullYear() + "/" + String(d.getMonth() + 1).padStart(2, "0") + "/" + String(d.getDate()).padStart(2, "0");
}

function Row({ entry, selected, onClick, density }) {
  const [hover, setHover] = useStateT(false);
  const rowH = density === "compact" ? 30 : density === "comfortable" ? 42 : 36;
  const subFont = density === "compact" ? 11 : 11.5;
  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "flex", alignItems: "center",
        height: rowH, fontSize: 12.5,
        background: selected ? "var(--row-selected)" : hover ? "var(--row-hover)" : "transparent",
        color: selected ? "var(--text)" : "var(--text)",
        borderBottom: "1px solid var(--border-subtle)",
        cursor: "default",
        position: "relative",
      }}
    >
      {/* selection accent */}
      {selected && <span style={{
        position: "absolute", left: 0, top: 0, bottom: 0, width: 2,
        background: "var(--accent-strong)",
      }} />}

      {/* star col (24) */}
      <div style={{ width: 28, padding: "0 4px 0 10px", display: "flex", justifyContent: "center" }}>
        {entry.starred
          ? <IconT name="starFill" size={11} color="oklch(0.72 0.14 70)" />
          : (hover && <IconT name="star" size={11} color="var(--text-faint)" />)
        }
      </div>

      {/* type (28) */}
      <div style={{ width: 28, display: "flex", justifyContent: "center", color: "var(--text-mute)" }}>
        <TypeIconT type={entry.type} size={13} />
      </div>

      {/* title (flex) */}
      <div style={{
        flex: 1, minWidth: 0, padding: "0 12px 0 4px",
        display: "flex", alignItems: "center", gap: 8,
      }}>
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", justifyContent: "center" }}>
          <div style={{
            fontSize: 13, fontWeight: 500, color: "var(--text)",
            overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
            letterSpacing: "-0.005em",
          }}>{entry.title}</div>
          {density === "comfortable" && entry.venue && (
            <div style={{
              fontSize: subFont, color: "var(--text-faint)", marginTop: 2,
              overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
            }}>{entry.venue}{entry.year ? " · " + entry.year : ""}</div>
          )}
        </div>
        {entry.attached && (
          <span style={{ color: "var(--text-mute)", flexShrink: 0 }} title="PDF添付あり">
            <IconT name="paperclip" size={12} />
          </span>
        )}
        {!entry.read && (
          <span style={{
            width: 6, height: 6, borderRadius: "50%",
            background: "var(--accent-strong)", flexShrink: 0,
          }} title="未読" />
        )}
      </div>

      {/* authors (220) */}
      <div style={{
        width: 200, padding: "0 10px",
        fontSize: 12, color: "var(--text-mute)",
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
      }}>{formatAuthors(entry.authors)}</div>

      {/* year (60) */}
      <div style={{
        width: 56, padding: "0 10px",
        fontSize: 12, color: "var(--text-mute)",
        fontVariantNumeric: "tabular-nums",
      }}>{entry.year || "—"}</div>

      {/* venue (160) */}
      <div style={{
        width: 150, padding: "0 10px",
        fontSize: 12, color: "var(--text-mute)",
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
        fontStyle: "italic",
      }}>{entry.venue || "—"}</div>

      {/* tags (200) */}
      <div style={{
        width: 200, padding: "0 10px",
        display: "flex", gap: 4, alignItems: "center",
        overflow: "hidden",
      }}>
        {(entry.tags || []).slice(0, 3).map((t) => <TagPill key={t} name={t} />)}
        {entry.tags && entry.tags.length > 3 && (
          <span style={{ fontSize: 10.5, color: "var(--text-faint)" }}>+{entry.tags.length - 3}</span>
        )}
      </div>

      {/* added (90) */}
      <div style={{
        width: 100, padding: "0 14px 0 10px",
        fontSize: 11.5, color: "var(--text-faint)",
        fontVariantNumeric: "tabular-nums",
        textAlign: "right",
      }}>{fmtDate(entry.added)}</div>
    </div>
  );
}

function EntriesTable({ entries, selectedId, onSelect, sort, onSort, density }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* header */}
      <div style={{
        display: "flex", flexShrink: 0,
        borderBottom: "1px solid var(--border)",
        background: "var(--surface-2)",
        position: "sticky", top: 0, zIndex: 1,
      }}>
        <div style={{ width: 28 }} />
        <div style={{ width: 28 }} />
        <ColumnHeader label="タイトル" width="auto" sortable
          sorted={sort.key === "title" ? sort.dir : null}
          onSort={() => onSort("title")} />
        <ColumnHeader label="著者" width={200} sortable
          sorted={sort.key === "authors" ? sort.dir : null}
          onSort={() => onSort("authors")} />
        <ColumnHeader label="年" width={56} sortable align="left"
          sorted={sort.key === "year" ? sort.dir : null}
          onSort={() => onSort("year")} />
        <ColumnHeader label="掲載" width={150} />
        <ColumnHeader label="タグ" width={200} />
        <ColumnHeader label="追加日" width={100} align="right" noBorder
          sortable sorted={sort.key === "added" ? sort.dir : null}
          onSort={() => onSort("added")} />
      </div>
      {/* rows */}
      <div style={{ flex: 1, overflow: "auto" }} className="entries-scroll">
        <div style={{ display: "flex", minWidth: 'fit-content' }}>
          <div style={{ width: "100%", minWidth: 720 }}>
            {entries.length === 0 ? (
              <div style={{
                padding: "60px 20px", textAlign: "center",
                color: "var(--text-faint)", fontSize: 13,
              }}>該当する文献はありません</div>
            ) : entries.map((e) => (
              <Row
                key={e.id} entry={e}
                selected={selectedId === e.id}
                onClick={() => onSelect(e.id)}
                density={density}
              />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

window.LumenTable = EntriesTable;
window.LumenTableHelpers = { formatAuthors, fmtDate, TagPill };
