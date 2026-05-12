import { useTranslation } from "react-i18next";
import { InfoTab } from "./InfoTab";
import { HighlightsTab } from "./HighlightsTab";
import { NotesTab } from "./NotesTab";
import { RelatedTab } from "./RelatedTab";
import type { EntryDetail, Highlight } from "../../types";

export type MetaTabId = "info" | "highlights" | "notes" | "related";

interface MetaPanelProps {
  entry: EntryDetail;
  tab: MetaTabId;
  onTabChange: (tab: MetaTabId) => void;
  highlights: Highlight[];
  onJumpToPage: (page: number) => void;
  onDeleteHighlight: (id: number) => void;
  onUpdateNotes: (notes: string) => void;
  onSelectEntry: (id: number) => void;
}

type TabLabelKey =
  | "detail.tab.info"
  | "detail.tab.highlights"
  | "detail.tab.notes"
  | "detail.tab.related";

const TABS: { id: MetaTabId; labelKey: TabLabelKey }[] = [
  { id: "info",       labelKey: "detail.tab.info" },
  { id: "highlights", labelKey: "detail.tab.highlights" },
  { id: "notes",      labelKey: "detail.tab.notes" },
  { id: "related",    labelKey: "detail.tab.related" },
];

export function MetaPanel({
  entry, tab, onTabChange,
  highlights, onJumpToPage, onDeleteHighlight,
  onUpdateNotes, onSelectEntry,
}: MetaPanelProps) {
  const { t } = useTranslation();
  return (
    <aside style={{
      width: 340, flexShrink: 0, height: "100%",
      borderLeft: "1px solid var(--border)",
      background: "var(--surface)",
      display: "flex", flexDirection: "column", overflow: "hidden",
    }}>
      <div style={{
        display: "flex", borderBottom: "1px solid var(--border)",
        padding: "0 8px", flexShrink: 0,
      }}>
        {TABS.map(({ id, labelKey }) => {
          const active = tab === id;
          return (
            <button
              key={id}
              onClick={() => onTabChange(id)}
              style={{
                flex: 1, padding: "9px 0", border: "none", background: "transparent",
                fontSize: 12, fontWeight: active ? 600 : 500,
                color: active ? "var(--text)" : "var(--text-mute)",
                borderBottom: active ? "2px solid var(--accent-strong)" : "2px solid transparent",
                marginBottom: -1, cursor: "pointer",
              }}
            >{t(labelKey)}</button>
          );
        })}
      </div>
      <div style={{ flex: 1, overflow: "auto", padding: "16px 18px" }}>
        {tab === "info" && <InfoTab entry={entry} />}
        {tab === "highlights" && (
          <HighlightsTab
            highlights={highlights}
            onJumpToPage={onJumpToPage}
            onDelete={onDeleteHighlight}
          />
        )}
        {tab === "notes" && <NotesTab entry={entry} onUpdate={onUpdateNotes} />}
        {tab === "related" && <RelatedTab entry={entry} onSelectEntry={onSelectEntry} />}
      </div>
    </aside>
  );
}
