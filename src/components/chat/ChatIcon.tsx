// LumenCite Chat — アイコン集（design handoff の ChatIcon を踏襲）。
// 16x16 viewBox、name で図形を選ぶ。既存 Library の Icon と同じ語彙。
import type { ReactNode } from "react";

export type ChatIconName =
  | "search" | "plus" | "chevronDown" | "chevronRight" | "chevronLeft"
  | "arrowLeft" | "sparkle" | "library" | "folder" | "edit" | "archive"
  | "more" | "send" | "stop" | "paperclip" | "check" | "x" | "warn"
  | "trash" | "plug" | "book" | "pencil" | "info" | "panel" | "cmd" | "enter";

interface ChatIconProps {
  name: ChatIconName;
  size?: number;
  color?: string;
  strokeWidth?: number;
}

export function ChatIcon({ name, size = 14, color = "currentColor", strokeWidth = 1.5 }: ChatIconProps) {
  const c = color;
  const sw = strokeWidth;
  const paths: Record<ChatIconName, ReactNode> = {
    search: <><circle cx="7" cy="7" r="4.5" stroke={c} strokeWidth={sw} /><path d="M10.5 10.5l3 3" stroke={c} strokeWidth={sw} strokeLinecap="round" /></>,
    plus: <path d="M8 3v10M3 8h10" stroke={c} strokeWidth={sw} strokeLinecap="round" />,
    chevronDown: <path d="M4 6l4 4 4-4" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" />,
    chevronRight: <path d="M6 4l4 4-4 4" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" />,
    chevronLeft: <path d="M10 4L6 8l4 4" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" />,
    arrowLeft: <path d="M9 4L4 8l5 4M4 8h9" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" />,
    sparkle: <path d="M8 2l1.2 3.8L13 7l-3.8 1.2L8 12l-1.2-3.8L3 7l3.8-1.2L8 2z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none" />,
    library: <><path d="M2.5 13V5l5.5-3 5.5 3v8" stroke={c} strokeWidth={sw} strokeLinejoin="round" /><path d="M2.5 13h11M5.5 13V8h5v5" stroke={c} strokeWidth={sw} /></>,
    folder: <path d="M2.5 4.5a1 1 0 0 1 1-1H6l1.5 1.5h5a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1V4.5z" stroke={c} strokeWidth={sw} strokeLinejoin="round" />,
    edit: <path d="M2.5 13.5h11M9.5 3.5l3 3-7 7H2.5v-3l7-7z" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none" />,
    archive: <><rect x="2" y="3.5" width="12" height="2.5" rx="0.5" stroke={c} strokeWidth={sw} /><path d="M3 6v6.5a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V6M6.5 8.5h3" stroke={c} strokeWidth={sw} strokeLinecap="round" /></>,
    more: <><circle cx="3.5" cy="8" r="1" fill={c} /><circle cx="8" cy="8" r="1" fill={c} /><circle cx="12.5" cy="8" r="1" fill={c} /></>,
    send: <path d="M2.5 8L13.5 3 11 13.5 7.5 9 2.5 8z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill={c} fillOpacity="0.95" />,
    stop: <rect x="3.5" y="3.5" width="9" height="9" rx="1.5" stroke={c} strokeWidth={sw} fill={c} fillOpacity="0.9" />,
    paperclip: <path d="M11 5L5.5 10.5a2 2 0 1 0 2.8 2.8L13 8.5a3.5 3.5 0 0 0-5-5L4 8" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none" />,
    check: <path d="M3 8.5l3 3 7-7" stroke={c} strokeWidth={sw + 0.2} strokeLinecap="round" strokeLinejoin="round" fill="none" />,
    x: <path d="M4 4l8 8M12 4l-8 8" stroke={c} strokeWidth={sw} strokeLinecap="round" />,
    warn: <path d="M8 2.5l6 11H2l6-11zM8 7v3.5M8 12v.5" stroke={c} strokeWidth={sw} strokeLinejoin="round" strokeLinecap="round" fill="none" />,
    trash: <path d="M3 5h10M5.5 5V3.5a1 1 0 0 1 1-1h3a1 1 0 0 1 1 1V5M4.5 5v8a1 1 0 0 0 1 1h5a1 1 0 0 0 1-1V5" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" />,
    plug: <><path d="M6 2v3M10 2v3" stroke={c} strokeWidth={sw} strokeLinecap="round" /><path d="M4 5h8v3a4 4 0 0 1-8 0V5z" stroke={c} strokeWidth={sw} strokeLinejoin="round" /><path d="M8 12v2" stroke={c} strokeWidth={sw} strokeLinecap="round" /></>,
    book: <path d="M3 2.5h8a1 1 0 0 1 1 1V14H4a1 1 0 0 1-1-1V2.5zM3 12h9" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none" />,
    pencil: <path d="M2.5 13.5h11M9.5 3.5l3 3-7 7H2.5v-3l7-7z" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none" />,
    info: <><circle cx="8" cy="8" r="5.5" stroke={c} strokeWidth={sw} /><path d="M8 7v4M8 5v.5" stroke={c} strokeWidth={sw} strokeLinecap="round" /></>,
    panel: <><rect x="2.5" y="3" width="11" height="10" rx="1" stroke={c} strokeWidth={sw} /><path d="M6.5 3v10" stroke={c} strokeWidth={sw} /></>,
    cmd: <path d="M5 3a1.5 1.5 0 1 0 0 3h6a1.5 1.5 0 1 0 0-3v10a1.5 1.5 0 1 0 0-3H5a1.5 1.5 0 1 0 0 3V3z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none" />,
    enter: <path d="M13 4v3a2 2 0 0 1-2 2H3M3 9l3-3M3 9l3 3" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none" />,
  };
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" style={{ display: "block", flexShrink: 0 }}>
      {paths[name]}
    </svg>
  );
}
