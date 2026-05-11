// LumenCite — Library main view
// Three-pane layout: Collections | Entries Table | Detail

const { useState, useMemo, useEffect, useRef } = React;
const { ENTRIES, COLLECTIONS, TAGS_USED } = window.LUMEN_DATA;

// ──────────────────────────────────────────────────────────
// Type icons (small, distinct, non-emoji)
// ──────────────────────────────────────────────────────────
const TypeIcon = ({ type, size = 14, color = "currentColor" }) => {
  const s = size, c = color;
  if (type === "article") return (
    <svg width={s} height={s} viewBox="0 0 16 16" fill="none">
      <path d="M3 2h7l3 3v9a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z" stroke={c} strokeWidth="1.2" strokeLinejoin="round"/>
      <path d="M10 2v3h3" stroke={c} strokeWidth="1.2" strokeLinejoin="round"/>
      <path d="M5 8.5h6M5 11h4" stroke={c} strokeWidth="1.2" strokeLinecap="round"/>
    </svg>
  );
  if (type === "book") return (
    <svg width={s} height={s} viewBox="0 0 16 16" fill="none">
      <path d="M3 2.5h8a1 1 0 0 1 1 1V14H4a1 1 0 0 1-1-1V2.5z" stroke={c} strokeWidth="1.2" strokeLinejoin="round"/>
      <path d="M3 12h9" stroke={c} strokeWidth="1.2"/>
    </svg>
  );
  if (type === "inproceedings") return (
    <svg width={s} height={s} viewBox="0 0 16 16" fill="none">
      <rect x="2.5" y="3" width="11" height="8" rx="1" stroke={c} strokeWidth="1.2"/>
      <path d="M5 14h6M8 11v3" stroke={c} strokeWidth="1.2" strokeLinecap="round"/>
    </svg>
  );
  if (type === "thesis") return (
    <svg width={s} height={s} viewBox="0 0 16 16" fill="none">
      <path d="M2 6l6-3 6 3-6 3-6-3z" stroke={c} strokeWidth="1.2" strokeLinejoin="round"/>
      <path d="M5 7.5v3c0 1 1.5 2 3 2s3-1 3-2v-3" stroke={c} strokeWidth="1.2" strokeLinejoin="round"/>
    </svg>
  );
  if (type === "webpage") return (
    <svg width={s} height={s} viewBox="0 0 16 16" fill="none">
      <circle cx="8" cy="8" r="5.5" stroke={c} strokeWidth="1.2"/>
      <path d="M2.5 8h11M8 2.5c1.7 1.5 2.5 3.5 2.5 5.5S9.7 12 8 13.5C6.3 12 5.5 10 5.5 8S6.3 4 8 2.5z" stroke={c} strokeWidth="1.2"/>
    </svg>
  );
  return (
    <svg width={s} height={s} viewBox="0 0 16 16" fill="none">
      <circle cx="8" cy="8" r="2" fill={c}/>
    </svg>
  );
};

const Icon = ({ name, size = 14, color = "currentColor", strokeWidth = 1.5 }) => {
  const s = size, c = color, sw = strokeWidth;
  const paths = {
    search: <><circle cx="7" cy="7" r="4.5" stroke={c} strokeWidth={sw}/><path d="M10.5 10.5l3 3" stroke={c} strokeWidth={sw} strokeLinecap="round"/></>,
    plus: <path d="M8 3v10M3 8h10" stroke={c} strokeWidth={sw} strokeLinecap="round"/>,
    chevronDown: <path d="M4 6l4 4 4-4" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round"/>,
    chevronRight: <path d="M6 4l4 4-4 4" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round"/>,
    folder: <path d="M2.5 4.5a1 1 0 0 1 1-1H6l1.5 1.5h5a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1V4.5z" stroke={c} strokeWidth={sw} strokeLinejoin="round"/>,
    star: <path d="M8 2l1.8 3.6 4 .6-2.9 2.8.7 4L8 11.2 4.4 13l.7-4L2.2 6.2l4-.6L8 2z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
    starFill: <path d="M8 2l1.8 3.6 4 .6-2.9 2.8.7 4L8 11.2 4.4 13l.7-4L2.2 6.2l4-.6L8 2z" fill={c}/>,
    paperclip: <path d="M11 5L5.5 10.5a2 2 0 1 0 2.8 2.8L13 8.5a3.5 3.5 0 0 0-5-5L4 8" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    library: <><path d="M2.5 13V5l5.5-3 5.5 3v8" stroke={c} strokeWidth={sw} strokeLinejoin="round"/><path d="M2.5 13h11M5.5 13V8h5v5" stroke={c} strokeWidth={sw}/></>,
    clock: <><circle cx="8" cy="8" r="5.5" stroke={c} strokeWidth={sw}/><path d="M8 5v3l2 1.5" stroke={c} strokeWidth={sw} strokeLinecap="round"/></>,
    inbox: <><path d="M2.5 9V4a1 1 0 0 1 1-1h9a1 1 0 0 1 1 1v5" stroke={c} strokeWidth={sw}/><path d="M2.5 9h3l1 2h3l1-2h3v3.5a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1V9z" stroke={c} strokeWidth={sw} strokeLinejoin="round"/></>,
    trash: <path d="M3 5h10M5.5 5V3.5a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1V5M4.5 5v8a1 1 0 0 0 1 1h5a1 1 0 0 0 1-1V5" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round"/>,
    tag: <path d="M2.5 8.5V3a.5.5 0 0 1 .5-.5h5.5L13 7l-5 5-5.5-3.5z" stroke={c} strokeWidth={sw} strokeLinejoin="round"/>,
    sortAsc: <path d="M4 12V4M4 4l-2 2M4 4l2 2M8 5h6M8 8h4M8 11h2" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round"/>,
    filter: <path d="M2.5 4h11l-4 5v4l-3-1.5V9l-4-5z" stroke={c} strokeWidth={sw} strokeLinejoin="round"/>,
    columns: <><rect x="2.5" y="3" width="11" height="10" rx="1" stroke={c} strokeWidth={sw}/><path d="M6 3v10M10 3v10" stroke={c} strokeWidth={sw}/></>,
    grid: <><rect x="2.5" y="3" width="4.5" height="4.5" stroke={c} strokeWidth={sw}/><rect x="9" y="3" width="4.5" height="4.5" stroke={c} strokeWidth={sw}/><rect x="2.5" y="8.5" width="4.5" height="4.5" stroke={c} strokeWidth={sw}/><rect x="9" y="8.5" width="4.5" height="4.5" stroke={c} strokeWidth={sw}/></>,
    list: <path d="M5 4h9M5 8h9M5 12h9M2.5 4h.5M2.5 8h.5M2.5 12h.5" stroke={c} strokeWidth={sw} strokeLinecap="round"/>,
    download: <path d="M8 2v8M5 7l3 3 3-3M3 13h10" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    upload: <path d="M8 13V5M5 8l3-3 3 3M3 13h10" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    sync: <path d="M3 8a5 5 0 0 1 9-3l1 1M13 8a5 5 0 0 1-9 3l-1-1M12 3v3h-3M4 13v-3h3" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    info: <><circle cx="8" cy="8" r="5.5" stroke={c} strokeWidth={sw}/><path d="M8 7v4M8 5v.5" stroke={c} strokeWidth={sw} strokeLinecap="round"/></>,
    sparkle: <path d="M8 2l1.2 3.8L13 7l-3.8 1.2L8 12l-1.2-3.8L3 7l3.8-1.2L8 2z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
    ext: <path d="M9 3h4v4M13 3l-6 6M11 9v3.5a.5.5 0 0 1-.5.5h-7a.5.5 0 0 1-.5-.5v-7a.5.5 0 0 1 .5-.5H7" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
  };
  return <svg width={s} height={s} viewBox="0 0 16 16" fill="none">{paths[name]}</svg>;
};

// Tag color palette
const TAG_COLORS = {
  amber: { bg: "oklch(0.95 0.05 75)", fg: "oklch(0.42 0.12 65)", dot: "oklch(0.7 0.13 70)" },
  blue: { bg: "oklch(0.95 0.04 240)", fg: "oklch(0.42 0.12 245)", dot: "oklch(0.6 0.13 245)" },
  green: { bg: "oklch(0.95 0.04 150)", fg: "oklch(0.4 0.10 150)", dot: "oklch(0.62 0.12 150)" },
  violet: { bg: "oklch(0.95 0.04 295)", fg: "oklch(0.42 0.12 295)", dot: "oklch(0.6 0.13 295)" },
  rose: { bg: "oklch(0.95 0.04 15)", fg: "oklch(0.45 0.13 15)", dot: "oklch(0.65 0.15 15)" },
  cyan: { bg: "oklch(0.95 0.04 200)", fg: "oklch(0.42 0.10 210)", dot: "oklch(0.6 0.12 210)" },
  neutral: { bg: "var(--surface-2)", fg: "var(--text-mute)", dot: "var(--text-faint)" },
};
const tagColor = (name) => {
  const found = TAGS_USED.find((t) => t.name === name);
  return TAG_COLORS[found?.color || "neutral"];
};

window.LumenCommon = { TypeIcon, Icon, TAG_COLORS, tagColor };
