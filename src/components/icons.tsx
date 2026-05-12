import {
  Search, Plus, ChevronDown, ChevronRight, Folder, Star,
  Paperclip, Library, Clock, Inbox, Trash2, Tag, ArrowUpDown,
  Filter, Columns2, Grid2x2, List, Download, Upload, RefreshCw,
  Info, Sparkles, ExternalLink, X, Settings, Highlighter, Printer,
} from "lucide-react";

type IconName =
  | "search" | "plus" | "chevronDown" | "chevronRight" | "folder"
  | "star" | "starFill" | "paperclip" | "library" | "clock" | "inbox"
  | "trash" | "tag" | "sortAsc" | "filter" | "columns" | "grid" | "list"
  | "download" | "upload" | "sync" | "info" | "sparkle" | "ext" | "close"
  | "settings" | "highlighter" | "printer";

interface IconProps {
  name: IconName;
  size?: number;
  color?: string;
  strokeWidth?: number;
}

export function Icon({ name, size = 14, color = "currentColor", strokeWidth = 1.5 }: IconProps) {
  const props = { size, color, strokeWidth };
  switch (name) {
    case "search":       return <Search {...props} />;
    case "plus":         return <Plus {...props} />;
    case "chevronDown":  return <ChevronDown {...props} />;
    case "chevronRight": return <ChevronRight {...props} />;
    case "folder":       return <Folder {...props} />;
    case "star":         return <Star {...props} />;
    case "starFill":     return <Star {...props} fill={color} />;
    case "paperclip":    return <Paperclip {...props} />;
    case "library":      return <Library {...props} />;
    case "clock":        return <Clock {...props} />;
    case "inbox":        return <Inbox {...props} />;
    case "trash":        return <Trash2 {...props} />;
    case "tag":          return <Tag {...props} />;
    case "sortAsc":      return <ArrowUpDown {...props} />;
    case "filter":       return <Filter {...props} />;
    case "columns":      return <Columns2 {...props} />;
    case "grid":         return <Grid2x2 {...props} />;
    case "list":         return <List {...props} />;
    case "download":     return <Download {...props} />;
    case "upload":       return <Upload {...props} />;
    case "sync":         return <RefreshCw {...props} />;
    case "info":         return <Info {...props} />;
    case "sparkle":      return <Sparkles {...props} />;
    case "ext":          return <ExternalLink {...props} />;
    case "close":        return <X {...props} />;
    case "settings":     return <Settings {...props} />;
    case "highlighter":  return <Highlighter {...props} />;
    case "printer":      return <Printer {...props} />;
    default:             return null;
  }
}

interface TypeIconProps {
  type: string;
  size?: number;
  color?: string;
}

export function TypeIcon({ type, size = 14, color = "currentColor" }: TypeIconProps) {
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
}
