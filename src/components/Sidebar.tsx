import { useState, useEffect, useRef } from "react";
import { Icon } from "./icons";
import { tagColorForName } from "./TagPill";
import type { Collection, SidebarCounts, Tag } from "../types";

interface SidebarProps {
  selectedView: string;
  onSelectView: (view: string) => void;
  collections: Collection[];
  tags: Tag[];
  counts: SidebarCounts;
  onCreateCollection: (name: string, parentId?: number) => void;
  onRenameCollection: (id: number, name: string) => void;
  onDeleteCollection: (id: number) => void;
  onCreateTag: (name: string) => void;
  onDeleteTag: (id: number) => void;
  onExportCollection: (collectionId: number, collectionName: string) => void;
  onDropEntry: (entryId: number, collectionId: number) => void;
  draggingId: number | null;
  bibtexSyncPath: string | null;
  bibtexLastSynced: string | null;
  bibtexLastError: string | null;
  onOpenBibtexSync: () => void;
}

const iconBtn: React.CSSProperties = {
  width: 24, height: 24, padding: 0, border: "none", background: "transparent",
  borderRadius: 5, cursor: "pointer", display: "inline-flex",
  alignItems: "center", justifyContent: "center",
};

function SidebarSection({ title, children, action }: {
  title: string;
  children: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div style={{ marginBottom: 18 }}>
      <div style={{
        display: "flex", alignItems: "center", justifyContent: "space-between",
        padding: "0 14px 6px",
        fontSize: 11, fontWeight: 600, letterSpacing: "0.06em",
        color: "var(--text-faint)", textTransform: "uppercase",
      }}>
        <span>{title}</span>
        {action}
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 1 }}>{children}</div>
    </div>
  );
}

function NavRow({ icon, label, count, active, onClick, indent = 0, expandable, expanded, onToggle, onContextMenu, dropTarget, onMouseEnter, onMouseLeave, onMouseUp }: {
  icon: React.ReactNode | string;
  label: string;
  count?: number;
  active: boolean;
  onClick: () => void;
  indent?: number;
  expandable?: boolean;
  expanded?: boolean;
  onToggle?: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  dropTarget?: boolean;
  onMouseEnter?: () => void;
  onMouseLeave?: () => void;
  onMouseUp?: () => void;
}) {
  const [hover, setHover] = useState(false);
  return (
    <div
      onClick={onClick}
      onContextMenu={onContextMenu}
      onMouseEnter={() => { setHover(true); onMouseEnter?.(); }}
      onMouseLeave={() => { setHover(false); onMouseLeave?.(); }}
      onMouseUp={onMouseUp}
      style={{
        display: "flex", alignItems: "center", gap: 8,
        padding: `5px 14px 5px ${10 + indent * 14}px`,
        margin: "0 6px", borderRadius: 6, cursor: "pointer",
        background: dropTarget ? "var(--accent-soft)" : active ? "var(--accent-soft)" : hover ? "var(--hover)" : "transparent",
        color: active || dropTarget ? "var(--accent-strong)" : "var(--text)",
        fontSize: 13, fontWeight: active ? 550 : 450,
        outline: dropTarget ? "2px solid var(--accent-strong)" : "none",
        outlineOffset: -2,
        transition: "background 80ms ease",
      }}
    >
      {expandable ? (
        <span
          onClick={(e) => { e.stopPropagation(); onToggle?.(); }}
          style={{
            display: "inline-flex", alignItems: "center", justifyContent: "center",
            width: 12, height: 12, marginLeft: -4, color: "var(--text-mute)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
            transition: "transform 100ms ease",
          }}
        >
          <Icon name="chevronRight" size={10} />
        </span>
      ) : <span style={{ width: 8 }} />}
      <span style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        color: active ? "var(--accent-strong)" : "var(--text-mute)",
        flexShrink: 0,
      }}>
        {typeof icon === "string" ? <Icon name={icon as Parameters<typeof Icon>[0]["name"]} size={14} /> : icon}
      </span>
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {label}
      </span>
      {count != null && (
        <span style={{
          fontSize: 11, color: active ? "var(--accent-strong)" : "var(--text-faint)",
          fontVariantNumeric: "tabular-nums",
        }}>{count}</span>
      )}
    </div>
  );
}

function InlineInput({
  initialValue = "",
  placeholder = "コレクション名…",
  indent = 0,
  onSubmit,
  onCancel,
}: {
  initialValue?: string;
  placeholder?: string;
  indent?: number;
  onSubmit: (value: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initialValue);
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => {
    ref.current?.focus();
    if (initialValue) ref.current?.select();
  }, []);

  return (
    <div style={{ padding: `3px 14px 3px ${18 + indent * 14}px`, margin: "0 6px" }}>
      <input
        ref={ref}
        value={value}
        onChange={e => setValue(e.target.value)}
        placeholder={placeholder}
        onKeyDown={e => {
          if (e.key === "Enter") { if (value.trim()) onSubmit(value.trim()); else onCancel(); e.preventDefault(); }
          if (e.key === "Escape") { onCancel(); e.preventDefault(); }
          e.stopPropagation();
        }}
        onBlur={() => { if (value.trim()) onSubmit(value.trim()); else onCancel(); }}
        style={{
          width: "100%", padding: "3px 7px",
          border: "1.5px solid var(--accent-strong)",
          borderRadius: 4, fontSize: 12.5,
          background: "var(--surface)", color: "var(--text)",
          outline: "none", boxSizing: "border-box",
        }}
      />
    </div>
  );
}

function TagDot({ name }: { name: string }) {
  const c = tagColorForName(name);
  return <span style={{ width: 8, height: 8, borderRadius: "50%", background: c.dot, display: "inline-block" }} />;
}

function CollectionRow({
  col, selectedView, onSelectView, depth = 0,
  renamingId, addingChildOf,
  onContextMenu, onRenameSubmit, onRenameCancel,
  onAddChildSubmit, onAddChildCancel, onDropEntry, draggingId,
  collectionCounts,
}: {
  col: Collection;
  selectedView: string;
  onSelectView: (v: string) => void;
  depth?: number;
  renamingId: number | null;
  addingChildOf: number | null;
  onContextMenu: (e: React.MouseEvent, col: Collection) => void;
  onRenameSubmit: (id: number, name: string) => void;
  onRenameCancel: () => void;
  onAddChildSubmit: (name: string, parentId: number) => void;
  onAddChildCancel: () => void;
  onDropEntry: (entryId: number, collectionId: number) => void;
  draggingId: number | null;
  collectionCounts: Record<string, number>;
}) {
  const [expanded, setExpanded] = useState(false);
  const [isDragHovering, setIsDragHovering] = useState(false);
  const isAddingChild = addingChildOf === col.id;
  const hasChildren = col.children.length > 0 || isAddingChild;

  useEffect(() => {
    if (isAddingChild) setExpanded(true);
  }, [isAddingChild]);

  // Clear hover when drag ends
  useEffect(() => {
    if (draggingId === null) setIsDragHovering(false);
  }, [draggingId]);

  const sharedChildProps = {
    selectedView, onSelectView,
    renamingId, addingChildOf,
    onContextMenu, onRenameSubmit, onRenameCancel,
    onAddChildSubmit, onAddChildCancel, onDropEntry, draggingId,
    collectionCounts,
  };

  if (renamingId === col.id) {
    return (
      <InlineInput
        initialValue={col.name}
        indent={depth}
        onSubmit={(name) => onRenameSubmit(col.id, name)}
        onCancel={onRenameCancel}
      />
    );
  }

  return (
    <>
      <NavRow
        icon="folder" label={col.name}
        count={collectionCounts[String(col.id)] ?? 0}
        active={selectedView === `col:${col.id}`}
        onClick={() => onSelectView(`col:${col.id}`)}
        indent={depth}
        expandable={hasChildren} expanded={expanded}
        onToggle={() => setExpanded(e => !e)}
        onContextMenu={(e) => { e.preventDefault(); onContextMenu(e, col); }}
        dropTarget={isDragHovering && draggingId !== null}
        onMouseEnter={() => { if (draggingId !== null) setIsDragHovering(true); }}
        onMouseLeave={() => setIsDragHovering(false)}
        onMouseUp={() => { if (isDragHovering && draggingId !== null) onDropEntry(draggingId, col.id); }}
      />
      {expanded && (
        <>
          {col.children.map(c => (
            <CollectionRow key={c.id} col={c} depth={depth + 1} {...sharedChildProps} />
          ))}
          {isAddingChild && (
            <InlineInput
              indent={depth + 1}
              onSubmit={(name) => onAddChildSubmit(name, col.id)}
              onCancel={onAddChildCancel}
            />
          )}
        </>
      )}
    </>
  );
}

export function Sidebar({
  selectedView, onSelectView, collections, tags, counts,
  onCreateCollection, onRenameCollection, onDeleteCollection,
  onCreateTag, onDeleteTag, onExportCollection, onDropEntry, draggingId,
  bibtexSyncPath, bibtexLastSynced, bibtexLastError, onOpenBibtexSync,
}: SidebarProps) {
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [addingChildOf, setAddingChildOf] = useState<number | null>(null);
  const [addingRoot, setAddingRoot] = useState(false);
  const [addingTag, setAddingTag] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; col: Collection } | null>(null);
  const [tagContextMenu, setTagContextMenu] = useState<{ x: number; y: number; tag: Tag } | null>(null);

  useEffect(() => {
    if (!contextMenu && !tagContextMenu) return;
    const handler = () => { setContextMenu(null); setTagContextMenu(null); };
    window.addEventListener("click", handler);
    return () => window.removeEventListener("click", handler);
  }, [!!contextMenu, !!tagContextMenu]);

  const handleContextMenu = (e: React.MouseEvent, col: Collection) => {
    e.preventDefault();
    e.stopPropagation();
    setContextMenu({ x: e.clientX, y: e.clientY, col });
  };

  const handleTagContextMenu = (e: React.MouseEvent, tag: Tag) => {
    e.preventDefault();
    e.stopPropagation();
    setTagContextMenu({ x: e.clientX, y: e.clientY, tag });
  };

  const startAddRoot = () => {
    setAddingRoot(true);
    setAddingChildOf(null);
    setRenamingId(null);
  };

  return (
    <aside style={{
      width: 232, flexShrink: 0, height: "100%",
      borderRight: "1px solid var(--border)",
      background: "var(--sidebar)",
      display: "flex", flexDirection: "column",
      overflow: "hidden",
    }}>
      {/* brand header */}
      <div style={{
        padding: "14px 18px 16px",
        display: "flex", alignItems: "center", gap: 10,
        WebkitAppRegion: "drag",
      } as React.CSSProperties}>
        <div style={{
          width: 22, height: 22, borderRadius: 6,
          background: "linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))",
          display: "flex", alignItems: "center", justifyContent: "center",
          flexShrink: 0,
          boxShadow: "0 1px 2px rgba(120,80,20,0.25), inset 0 0.5px 0 rgba(255,255,255,0.5)",
        }}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
            <path d="M3 2v6a3 3 0 0 0 6 0V2" stroke="white" strokeWidth="1.4" strokeLinecap="round"/>
            <circle cx="6" cy="9.5" r="1" fill="white"/>
          </svg>
        </div>
        <div style={{ flex: 1 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text)", letterSpacing: "-0.01em" }}>
            LumenCite
          </div>
          <div style={{ fontSize: 10.5, color: "var(--text-faint)", marginTop: 1 }}>研究ライブラリ</div>
        </div>
        <button
          onClick={onOpenBibtexSync}
          title="BibTeX 自動同期"
          style={{ ...iconBtn, WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          <Icon name="sync" size={13} color="var(--text-mute)" />
        </button>
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: "4px 0 16px" }}>
        <SidebarSection title="ライブラリ">
          <NavRow icon="library" label="すべての文献" count={counts.total}
            active={selectedView === "all"} onClick={() => onSelectView("all")} />
          <NavRow icon="clock" label="最近追加" count={Math.min(counts.total, 8)}
            active={selectedView === "recent"} onClick={() => onSelectView("recent")} />
          <NavRow
            icon={<Icon name="starFill" size={12} color="oklch(0.7 0.13 70)" />}
            label="お気に入り" count={counts.starred}
            active={selectedView === "starred"} onClick={() => onSelectView("starred")} />
          <NavRow icon="inbox" label="未整理" count={counts.unfiled}
            active={selectedView === "unfiled"} onClick={() => onSelectView("unfiled")} />
          <NavRow icon="trash" label="ゴミ箱" count={counts.trash}
            active={selectedView === "trash"} onClick={() => onSelectView("trash")} />
        </SidebarSection>

        <SidebarSection title="コレクション" action={
          <button
            style={{ ...iconBtn, width: 18, height: 18 }}
            onMouseDown={e => e.stopPropagation()}
            onClick={startAddRoot}
          >
            <Icon name="plus" size={11} color="var(--text-mute)" />
          </button>
        }>
          {collections.map(col => (
            <CollectionRow
              key={col.id} col={col}
              selectedView={selectedView} onSelectView={onSelectView}
              renamingId={renamingId} addingChildOf={addingChildOf}
              onContextMenu={handleContextMenu}
              onRenameSubmit={(id, name) => { onRenameCollection(id, name); setRenamingId(null); }}
              onRenameCancel={() => setRenamingId(null)}
              onAddChildSubmit={(name, parentId) => { onCreateCollection(name, parentId); setAddingChildOf(null); }}
              onAddChildCancel={() => setAddingChildOf(null)}
              onDropEntry={onDropEntry}
              draggingId={draggingId}
              collectionCounts={counts.collections}
            />
          ))}
          {addingRoot && (
            <InlineInput
              onSubmit={(name) => { onCreateCollection(name); setAddingRoot(false); }}
              onCancel={() => setAddingRoot(false)}
            />
          )}
        </SidebarSection>

        <SidebarSection title="タグ" action={
          <button
            style={{ ...iconBtn, width: 18, height: 18 }}
            onMouseDown={e => e.stopPropagation()}
            onClick={() => setAddingTag(true)}
          >
            <Icon name="plus" size={11} color="var(--text-mute)" />
          </button>
        }>
          {tags.map(t => (
            <NavRow
              key={t.id}
              icon={<TagDot name={t.name} />}
              label={t.name}
              count={counts.tags[String(t.id)] ?? 0}
              active={selectedView === `tag:${t.id}`}
              onClick={() => onSelectView(`tag:${t.id}`)}
              onContextMenu={(e) => handleTagContextMenu(e, t)}
            />
          ))}
          {addingTag && (
            <InlineInput
              placeholder="タグ名…"
              onSubmit={(name) => { onCreateTag(name); setAddingTag(false); }}
              onCancel={() => setAddingTag(false)}
            />
          )}
        </SidebarSection>
      </div>

      {/* sync status */}
      <SyncStatus
        path={bibtexSyncPath}
        lastSynced={bibtexLastSynced}
        error={bibtexLastError}
        onClick={onOpenBibtexSync}
      />

      {/* Context menu */}
      {contextMenu && (
        <div
          style={{
            position: "fixed",
            left: contextMenu.x, top: contextMenu.y,
            background: "var(--surface)",
            border: "1px solid var(--border-strong)",
            borderRadius: 8,
            boxShadow: "0 4px 16px rgba(0,0,0,0.14)",
            padding: "4px 0",
            zIndex: 1000,
            minWidth: 168,
          }}
          onClick={e => e.stopPropagation()}
        >
          <ContextMenuItem label="名前を変更" onClick={() => {
            setRenamingId(contextMenu.col.id);
            setAddingRoot(false);
            setContextMenu(null);
          }} />
          <ContextMenuItem label="サブコレクションを追加" onClick={() => {
            setAddingChildOf(contextMenu.col.id);
            setAddingRoot(false);
            setContextMenu(null);
          }} />
          <ContextMenuItem label="BibTeX を書き出し" onClick={() => {
            onExportCollection(contextMenu.col.id, contextMenu.col.name);
            setContextMenu(null);
          }} />
          <div style={{ height: 1, background: "var(--border)", margin: "3px 0" }} />
          <ContextMenuItem label="削除" danger onClick={() => {
            onDeleteCollection(contextMenu.col.id);
            setContextMenu(null);
          }} />
        </div>
      )}

      {tagContextMenu && (
        <div
          style={{
            position: "fixed",
            left: tagContextMenu.x, top: tagContextMenu.y,
            background: "var(--surface)",
            border: "1px solid var(--border-strong)",
            borderRadius: 8,
            boxShadow: "0 4px 16px rgba(0,0,0,0.14)",
            padding: "4px 0",
            zIndex: 1000,
            minWidth: 168,
          }}
          onClick={e => e.stopPropagation()}
        >
          <ContextMenuItem label="削除" danger onClick={() => {
            onDeleteTag(tagContextMenu.tag.id);
            setTagContextMenu(null);
          }} />
        </div>
      )}
    </aside>
  );
}

function SyncStatus({ path, lastSynced, error, onClick }: {
  path: string | null;
  lastSynced: string | null;
  error: string | null;
  onClick: () => void;
}) {
  // path 未設定: グレー＋「未設定」 / error あり: 赤 / 通常: 緑＋ファイル名
  const fileName = path ? path.split("/").pop() ?? path : null;
  const dotColor = !path
    ? "oklch(0.7 0 0)"
    : error
    ? "oklch(0.55 0.18 15)"
    : "oklch(0.68 0.13 150)";
  const label = !path
    ? "BibTeX 同期: 未設定"
    : error
    ? "同期エラー"
    : fileName
    ? `${fileName} と同期`
    : "同期中";
  const human = lastSynced && !error
    ? new Date(parseInt(lastSynced, 10) * 1000).toLocaleTimeString()
    : null;

  return (
    <div
      onClick={onClick}
      title={path ?? "クリックして設定"}
      style={{
        padding: "10px 18px 12px", borderTop: "1px solid var(--border)",
        fontSize: 11, color: "var(--text-faint)",
        display: "flex", alignItems: "center", gap: 7, cursor: "pointer",
      }}
    >
      <span style={{
        width: 6, height: 6, borderRadius: "50%",
        background: dotColor,
        boxShadow: `0 0 0 3px ${dotColor} / 0.18`,
        flexShrink: 0,
      }} />
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {label}
      </span>
      {human && (
        <span style={{ color: "var(--text-faint)", fontVariantNumeric: "tabular-nums" }}>
          {human}
        </span>
      )}
    </div>
  );
}

function ContextMenuItem({ label, onClick, danger = false }: {
  label: string; onClick: () => void; danger?: boolean;
}) {
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "block", width: "100%", padding: "6px 14px",
        border: "none", textAlign: "left",
        fontSize: 12.5, cursor: "pointer",
        color: danger ? "oklch(0.52 0.18 15)" : "var(--text)",
        background: hover
          ? (danger ? "oklch(0.96 0.03 15)" : "var(--hover)")
          : "transparent",
      }}
    >{label}</button>
  );
}
