import { useState, useMemo, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./index.css";

import { useTheme } from "./hooks/useTheme";
import { Sidebar } from "./components/Sidebar";
import { Toolbar, ViewTabs } from "./components/Toolbar";
import { EntriesTable } from "./components/EntriesTable";
import { CoversGrid } from "./components/CoversGrid";
import { DetailPanel } from "./components/DetailPanel";
import { BulkActionsPanel } from "./components/BulkActionsPanel";
import { AddSheet } from "./components/AddSheet";
import { EditSheet } from "./components/EditSheet";
import { StatusBar } from "./components/StatusBar";
import { FulltextResults } from "./components/FulltextResults";
import { BibtexSyncSheet } from "./components/BibtexSyncSheet";

import type { EntrySummary, EntryDetail, Collection, Tag, ViewMode, SearchScope, FulltextHit, SidebarCounts } from "./types";

const EMPTY_COUNTS: SidebarCounts = {
  total: 0, starred: 0, unfiled: 0, trash: 0, collections: {}, tags: {},
};

type LoadEntriesArgs = {
  collectionId?: number | null;
  tagId?: number | null;
  view?: string | null;
};

// 特殊ビュー（starred / unfiled / trash）のときに backend へ渡す view パラメータ。
// コレクション・タグビューでは null（collection/tag フィルタが優先）。
function viewParam(selectedView: string): string | null {
  if (selectedView === "starred" || selectedView === "unfiled" || selectedView === "trash") {
    return selectedView;
  }
  return null;
}

function PlaceholderView({ title, body }: { title: string; body: string }) {
  return (
    <div style={{
      flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
      flexDirection: "column", gap: 6, color: "var(--text-faint)",
      background: "var(--surface)",
    }}>
      <div style={{ fontSize: 14, color: "var(--text-mute)", fontWeight: 500 }}>{title}</div>
      <div style={{ fontSize: 12 }}>{body}</div>
    </div>
  );
}

function viewLabel(selectedView: string, collections: Collection[]): { title: string; subtitle?: string } {
  if (selectedView === "all")      return { title: "すべての文献", subtitle: "Library" };
  if (selectedView === "starred")  return { title: "お気に入り", subtitle: "Starred" };
  if (selectedView === "recent")   return { title: "最近追加", subtitle: "直近8件" };
  if (selectedView === "unfiled")  return { title: "未整理", subtitle: "コレクション未割当" };
  if (selectedView === "trash")    return { title: "ゴミ箱", subtitle: "Trash" };
  if (selectedView.startsWith("col:")) {
    const id = Number(selectedView.slice(4));
    const find = (cs: Collection[]): Collection | undefined => {
      for (const c of cs) {
        if (c.id === id) return c;
        const found = find(c.children);
        if (found) return found;
      }
    };
    const col = find(collections);
    return { title: col?.name ?? "コレクション", subtitle: "コレクション" };
  }
  if (selectedView.startsWith("tag:")) {
    return { title: `#${selectedView.slice(4)}`, subtitle: "タグ" };
  }
  return { title: "文献" };
}

export default function App() {
  const { density } = useTheme();

  const [entries, setEntries] = useState<EntrySummary[]>([]);
  const [collections, setCollections] = useState<Collection[]>([]);
  const [tags, setTags] = useState<Tag[]>([]);
  const [counts, setCounts] = useState<SidebarCounts>(EMPTY_COUNTS);

  const reloadCounts = () =>
    invoke<SidebarCounts>("get_sidebar_counts")
      .then(setCounts)
      .catch(() => setCounts(EMPTY_COUNTS));
  const [selectedView, setSelectedView] = useState("all");

  // 複数選択。size===1 のときが「単独選択」、size>1 が「一括モード」、size===0 で未選択。
  // selectedId は派生値で、単独選択時のみ非 null。
  const [selectedIds, setSelectedIds] = useState<Set<number>>(() => new Set());
  // Shift+Click の範囲選択用アンカー（最後にプレーン/Cmd+Click した行）
  const [anchorId, setAnchorId] = useState<number | null>(null);
  const selectedId = selectedIds.size === 1 ? [...selectedIds][0] : null;
  const isBulk = selectedIds.size > 1;

  const clearSelection = () => {
    setSelectedIds(new Set());
    setAnchorId(null);
  };

  const selectSingle = (id: number) => {
    setSelectedIds(new Set([id]));
    setAnchorId(id);
  };

  const [detail, setDetail] = useState<EntryDetail | null>(null);
  const [sort, setSort] = useState<{ key: string; dir: "asc" | "desc" }>({ key: "added", dir: "desc" });
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [searchScope, setSearchScope] = useState<SearchScope>("meta");
  const [fulltextHits, setFulltextHits] = useState<FulltextHit[]>([]);
  const [indexingCount, setIndexingCount] = useState(0);
  const [viewMode, setViewMode] = useState<ViewMode>("table");
  const [showAdd, setShowAdd] = useState(false);
  const [showEdit, setShowEdit] = useState(false);
  const [showBibtexSync, setShowBibtexSync] = useState(false);

  // BibTeX 同期の状態。path は起動時に取得、lastSynced/lastError は backend からのイベントで更新。
  const [bibtexSyncPath, setBibtexSyncPath] = useState<string | null>(null);
  const [bibtexLastSynced, setBibtexLastSynced] = useState<string | null>(null);
  const [bibtexLastError, setBibtexLastError] = useState<string | null>(null);

  useEffect(() => {
    invoke<string | null>("get_bibtex_sync_path")
      .then(p => setBibtexSyncPath(p && p.trim() ? p : null))
      .catch(() => setBibtexSyncPath(null));

    const unlistenPromise = listen<{ path: string; synced_at: string | null; error: string | null }>(
      "bibtex-synced",
      (e) => {
        setBibtexSyncPath(e.payload.path && e.payload.path.trim() ? e.payload.path : null);
        if (e.payload.error) {
          setBibtexLastError(e.payload.error);
        } else {
          setBibtexLastError(null);
          setBibtexLastSynced(e.payload.synced_at);
        }
      },
    );
    return () => { unlistenPromise.then(u => u()); };
  }, []);

  useEffect(() => {
    const t = setTimeout(() => setDebouncedSearch(search), 200);
    return () => clearTimeout(t);
  }, [search]);

  // ── pointer-based drag state ──────────────────────────────────────────────
  const [dragEntryId, setDragEntryId] = useState<number | null>(null);
  const [dragPos, setDragPos] = useState({ x: 0, y: 0 });
  const dragStartRef = useRef<{ id: number; x: number; y: number } | null>(null);
  const isDraggingRef = useRef(false);

  useEffect(() => {
    const THRESHOLD = 5;
    const onMove = (e: MouseEvent) => {
      if (!dragStartRef.current) return;
      const dx = e.clientX - dragStartRef.current.x;
      const dy = e.clientY - dragStartRef.current.y;
      if (!isDraggingRef.current && Math.hypot(dx, dy) > THRESHOLD) {
        isDraggingRef.current = true;
        document.body.style.userSelect = "none";
        setDragEntryId(dragStartRef.current.id);
      }
      if (isDraggingRef.current) setDragPos({ x: e.clientX, y: e.clientY });
    };
    const onUp = () => {
      document.body.style.userSelect = "";
      isDraggingRef.current = false;
      dragStartRef.current = null;
      setDragEntryId(null);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, []);

  const handleStartDrag = (id: number, e: React.MouseEvent) => {
    dragStartRef.current = { id, x: e.clientX, y: e.clientY };
    isDraggingRef.current = false;
    document.body.style.userSelect = "none";
  };

  const loadEntries = (view = selectedView, query = debouncedSearch, scope = searchScope) => {
    // サイドバー件数は entries とは独立した集計値。loadEntries が呼ばれるたびに
    // refresh しておくことで、view 切替直後にも最新の件数が表示される。
    reloadCounts();

    const collectionId = view.startsWith("col:") ? Number(view.slice(4)) : null;
    const tagId = view.startsWith("tag:") ? Number(view.slice(4)) : null;
    const viewName = viewParam(view);
    const trimmed = query.trim();

    if (scope === "fulltext") {
      // 全文モード時は entries テーブルは現在のビューで埋めておき、結果リストは fulltextHits に持つ
      const args: LoadEntriesArgs = { collectionId, tagId, view: viewName };
      invoke<EntrySummary[]>("get_entries", args).then(setEntries).catch(console.error);
      if (trimmed) {
        invoke<FulltextHit[]>("fulltext_search", { query: trimmed, collectionId, tagId })
          .then(setFulltextHits)
          .catch((e) => { console.error(e); setFulltextHits([]); });
      } else {
        setFulltextHits([]);
      }
      return;
    }

    if (trimmed) {
      invoke<EntrySummary[]>("search_entries", { query: trimmed, collectionId, tagId })
        .then(setEntries)
        .catch(console.error);
    } else {
      const args: LoadEntriesArgs = { collectionId, tagId, view: viewName };
      invoke<EntrySummary[]>("get_entries", args).then(setEntries).catch(console.error);
    }
  };

  // load entries when view, debounced search, or scope changes
  useEffect(() => { loadEntries(); }, [selectedView, debouncedSearch, searchScope]);

  // entries が変わったら、表示されていない id を選択集合から外す。
  // これにより view/search 切替で「見えない選択」が残らない。
  useEffect(() => {
    setSelectedIds(prev => {
      if (prev.size === 0) return prev;
      const visible = new Set(entries.map(e => e.id));
      let removed = false;
      const next = new Set<number>();
      for (const id of prev) {
        if (visible.has(id)) next.add(id);
        else removed = true;
      }
      return removed ? next : prev;
    });
    setAnchorId(prev => (prev != null && !entries.some(e => e.id === prev) ? null : prev));
  }, [entries]);

  // load collections and tags once
  useEffect(() => {
    invoke<Collection[]>("get_collections")
      .then(setCollections)
      .catch(() => setCollections([]));

    invoke<Tag[]>("get_tags")
      .then(setTags)
      .catch(() => setTags([]));
  }, []);

  // load detail when single selection changes (bulk mode shows BulkActionsPanel, no detail needed)
  useEffect(() => {
    if (selectedId == null) { setDetail(null); return; }
    invoke<EntryDetail>("get_entry", { id: selectedId })
      .then(setDetail)
      .catch(() => setDetail(null));
  }, [selectedId]);

  // ── multi-select handler (called from EntriesTable rows) ──────────────────
  const handleTableSelect = (id: number, mods: { meta?: boolean; shift?: boolean }) => {
    if (mods.shift && anchorId != null) {
      // 範囲選択: anchor から id まで（現在の表示順）を選択集合に置き換える
      const ids = filteredEntries.map(e => e.id);
      const aIdx = ids.indexOf(anchorId);
      const bIdx = ids.indexOf(id);
      if (aIdx === -1 || bIdx === -1) { selectSingle(id); return; }
      const [lo, hi] = aIdx < bIdx ? [aIdx, bIdx] : [bIdx, aIdx];
      setSelectedIds(new Set(ids.slice(lo, hi + 1)));
      // shift+click ではアンカーを動かさない（範囲を広げ縮めできるように）
    } else if (mods.meta) {
      setSelectedIds(prev => {
        const next = new Set(prev);
        if (next.has(id)) next.delete(id); else next.add(id);
        return next;
      });
      setAnchorId(id);
    } else {
      selectSingle(id);
    }
  };

  // ESC で選択解除
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && selectedIds.size > 0) {
        // 編集モーダルなど他のオーバーレイが開いているときは触らない
        if (!showAdd && !showEdit) clearSelection();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedIds.size, showAdd, showEdit]);

  const filteredEntries = useMemo(() => {
    let list = [...entries];

    // client-side filters for special views
    if (selectedView === "recent") {
      list = list.slice(0, 8);
    }

    const dir = sort.dir === "asc" ? 1 : -1;
    list.sort((a, b) => {
      let av: string | number = "", bv: string | number = "";
      if (sort.key === "title")   { av = a.title;                        bv = b.title; }
      if (sort.key === "authors") { av = a.authors[0]?.name ?? "";       bv = b.authors[0]?.name ?? ""; }
      if (sort.key === "year")    { av = a.year ?? 0;                    bv = b.year ?? 0; }
      return av < bv ? -dir : av > bv ? dir : 0;
    });

    return list;
  }, [entries, selectedView, sort]);

  // 通常ビューからの「削除」はゴミ箱へ移動（ソフト削除）。単独・複数選択どちらも bulk API。
  const handleTrash = () => {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    invoke("bulk_trash", { ids })
      .then(() => { clearSelection(); setDetail(null); loadEntries(); })
      .catch(console.error);
  };

  // ゴミ箱ビューからの「永久削除」はハード削除。
  const handlePurge = () => {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    invoke("bulk_purge", { ids })
      .then(() => { clearSelection(); setDetail(null); loadEntries(); })
      .catch(console.error);
  };

  const handleRestore = () => {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    invoke("bulk_restore", { ids })
      .then(() => { clearSelection(); setDetail(null); loadEntries(); })
      .catch(console.error);
  };

  const handleToggleStar = (id: number, starred: boolean) => {
    invoke("set_starred", { id, starred })
      .then(() => {
        setEntries(prev => prev.map(e => e.id === id ? { ...e, starred } : e));
        if (detail && detail.id === id) setDetail({ ...detail, starred });
      })
      .catch(console.error);
  };

  const reloadCollections = () =>
    invoke<Collection[]>("get_collections").then(setCollections).catch(console.error);

  const reloadTags = () =>
    invoke<Tag[]>("get_tags").then(setTags).catch(console.error);

  const handleCreateCollection = (name: string, parentId?: number) => {
    invoke("create_collection", { name, parentId: parentId ?? null })
      .then(reloadCollections)
      .catch(console.error);
  };

  const handleRenameCollection = (id: number, name: string) => {
    invoke("update_collection", { id, name })
      .then(reloadCollections)
      .catch(console.error);
  };

  const handleDeleteCollection = (id: number) => {
    invoke("delete_collection", { id })
      .then(() => {
        reloadCollections();
        if (selectedView === `col:${id}`) setSelectedView("all");
      })
      .catch(console.error);
  };

  const reloadDetail = (id: number) =>
    invoke<EntryDetail>("get_entry", { id })
      .then(setDetail)
      .catch(console.error);

  const handleAddToCollection = (collectionId: number) => {
    if (selectedId == null) return;
    invoke("add_entry_to_collection", { entryId: selectedId, collectionId })
      .then(() => reloadDetail(selectedId))
      .catch(console.error);
  };

  const handleRemoveFromCollection = (collectionId: number) => {
    if (selectedId == null) return;
    invoke("remove_entry_from_collection", { entryId: selectedId, collectionId })
      .then(() => { reloadDetail(selectedId); loadEntries(); })
      .catch(console.error);
  };

  const handleDropEntry = (entryId: number, collectionId: number) => {
    // ドラッグ中の項目が現在の選択集合に含まれていれば、選択全体をまとめてドロップする。
    const ids = selectedIds.has(entryId) && selectedIds.size > 1
      ? [...selectedIds]
      : [entryId];
    invoke("bulk_add_to_collection", { ids, collectionId })
      .then(() => {
        if (selectedId != null && ids.includes(selectedId)) reloadDetail(selectedId);
        loadEntries();
      })
      .catch(console.error);
  };

  // 複数選択時のコレクション一括追加（BulkActionsPanel から呼ばれる）
  const handleBulkAddToCollection = (collectionId: number) => {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    invoke("bulk_add_to_collection", { ids, collectionId })
      .then(() => { loadEntries(); })
      .catch(console.error);
  };

  // 複数選択時のタグ一括追加（名前指定。存在しなければ作成）
  const handleBulkAddTag = async (name: string) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    try {
      let tag = tags.find(t => t.name === trimmed);
      if (!tag) {
        tag = await invoke<Tag>("create_tag", { name: trimmed });
        await reloadTags();
      }
      await invoke("bulk_add_tag", { ids, tagId: tag.id });
      loadEntries();
    } catch (e) {
      console.error(e);
    }
  };

  // 複数選択時の BibTeX 書き出し
  const handleBulkExport = () => {
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    handleExportBibtex(ids, `lumencite-${ids.length}件`);
  };

  const handleCreateTag = (name: string) =>
    invoke<Tag>("create_tag", { name })
      .then(reloadTags)
      .catch(console.error);

  const handleDeleteTag = (id: number) =>
    invoke("delete_tag", { id })
      .then(() => {
        reloadTags();
        loadEntries();
        if (selectedView === `tag:${id}`) setSelectedView("all");
        if (selectedId != null) reloadDetail(selectedId);
      })
      .catch(console.error);

  const handleAddTagToEntry = async (name: string) => {
    if (selectedId == null) return;
    const trimmed = name.trim();
    if (!trimmed) return;
    try {
      // 既存タグを名前で探し、なければ作成
      let tag = tags.find(t => t.name === trimmed);
      if (!tag) {
        tag = await invoke<Tag>("create_tag", { name: trimmed });
        await reloadTags();
      }
      await invoke("add_tag_to_entry", { entryId: selectedId, tagId: tag.id });
      await reloadDetail(selectedId);
      loadEntries();
    } catch (e) {
      console.error(e);
    }
  };

  const handleRemoveTagFromEntry = (tagId: number) => {
    if (selectedId == null) return;
    invoke("remove_tag_from_entry", { entryId: selectedId, tagId })
      .then(() => {
        reloadDetail(selectedId);
        loadEntries();
      })
      .catch(console.error);
  };

  const safeFileName = (s: string) =>
    s.replace(/[\/\\:*?"<>|]/g, "_").trim() || "lumencite";

  const handleExportBibtex = (entryIds?: number[], baseName?: string) => {
    const ids = entryIds ?? filteredEntries.map(e => e.id);
    if (ids.length === 0) return;
    const defaultName = `${safeFileName(baseName ?? "lumencite")}.bib`;
    invoke<string | null>("save_bibtex", { entryIds: ids, defaultName }).catch(console.error);
  };

  const handleExportCollection = async (collectionId: number, collectionName: string) => {
    try {
      const colEntries = await invoke<EntrySummary[]>("get_entries", { collectionId, tagId: null });
      handleExportBibtex(colEntries.map(e => e.id), collectionName);
    } catch (e) {
      console.error(e);
    }
  };

  const handleSort = (key: string) => {
    setSort(s => s.key === key
      ? { key, dir: s.dir === "asc" ? "desc" : "asc" }
      : { key, dir: key === "title" || key === "authors" ? "asc" : "desc" });
  };

  const label = viewLabel(selectedView, collections);

  return (
    <div style={{
      width: "100%", height: "100%",
      background: "var(--bg)", color: "var(--text)",
      display: "flex", overflow: "hidden",
    }}>
      <Sidebar
        selectedView={selectedView}
        onSelectView={setSelectedView}
        collections={collections}
        tags={tags}
        counts={counts}
        onCreateCollection={handleCreateCollection}
        onRenameCollection={handleRenameCollection}
        onDeleteCollection={handleDeleteCollection}
        onCreateTag={handleCreateTag}
        onDeleteTag={handleDeleteTag}
        onExportCollection={handleExportCollection}
        onDropEntry={handleDropEntry}
        draggingId={dragEntryId}
        bibtexSyncPath={bibtexSyncPath}
        bibtexLastSynced={bibtexLastSynced}
        bibtexLastError={bibtexLastError}
        onOpenBibtexSync={() => setShowBibtexSync(true)}
      />

      <main style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
        <Toolbar
          title={label.title}
          subtitle={label.subtitle}
          count={searchScope === "fulltext" ? fulltextHits.length : filteredEntries.length}
          search={search}
          onSearchChange={setSearch}
          searchScope={searchScope}
          onSearchScopeChange={setSearchScope}
          onAddOpen={() => setShowAdd(true)}
          onExportBibtex={() => handleExportBibtex(undefined, label.title)}
          exportDisabled={filteredEntries.length === 0}
        />
        {searchScope === "meta" && (
          <ViewTabs viewMode={viewMode} onViewModeChange={setViewMode} />
        )}

        <div style={{ flex: 1, minHeight: 0, position: "relative", display: "flex", flexDirection: "column" }}>
          {searchScope === "fulltext" && (
            <FulltextResults
              hits={fulltextHits}
              query={debouncedSearch}
              selectedId={selectedId}
              onSelect={selectSingle}
            />
          )}
          {searchScope === "meta" && viewMode === "table" && (
            <EntriesTable
              entries={filteredEntries}
              selectedIds={selectedIds}
              onSelect={handleTableSelect}
              sort={sort}
              onSort={handleSort}
              density={density}
              draggingId={dragEntryId}
              onStartDrag={handleStartDrag}
              onToggleStar={handleToggleStar}
            />
          )}
          {searchScope === "meta" && viewMode === "covers" && (
            <CoversGrid
              entries={filteredEntries}
              selectedId={selectedId}
              onSelect={selectSingle}
            />
          )}
          {searchScope === "meta" && viewMode === "timeline" && (
            <PlaceholderView title="タイムラインビュー" body="出版年・追加日軸で並べるビューを今後追加します。" />
          )}
          {searchScope === "meta" && viewMode === "graph" && (
            <PlaceholderView title="引用グラフビュー" body="文献間の引用関係をネットワーク表示します。" />
          )}
          {showAdd && (
            <AddSheet
              onClose={() => setShowAdd(false)}
              onCreated={(entry) => {
                setShowAdd(false);
                loadEntries();
                selectSingle(entry.id);
              }}
              onImported={() => {
                loadEntries();
              }}
            />
          )}
          {showEdit && detail && (
            <EditSheet
              entry={detail}
              onClose={() => setShowEdit(false)}
              onSaved={(updated) => {
                setShowEdit(false);
                setDetail(updated);
                loadEntries();
              }}
            />
          )}
        </div>

        <StatusBar
          total={entries.length}
          filtered={searchScope === "fulltext" ? fulltextHits.length : filteredEntries.length}
          selectedId={selectedId}
          indexingCount={indexingCount}
        />
      </main>

      {isBulk ? (
        <BulkActionsPanel
          width={320}
          count={selectedIds.size}
          inTrash={selectedView === "trash"}
          allCollections={collections}
          onClearSelection={clearSelection}
          onTrash={handleTrash}
          onRestore={handleRestore}
          onPurge={handlePurge}
          onAddToCollection={handleBulkAddToCollection}
          onAddTag={handleBulkAddTag}
          onExportBibtex={handleBulkExport}
        />
      ) : (
        <DetailPanel
          entry={detail} width={320}
          inTrash={selectedView === "trash"}
          onEdit={() => setShowEdit(true)}
          onDelete={selectedView === "trash" ? handlePurge : handleTrash}
          onRestore={handleRestore}
          onToggleStar={() => detail && handleToggleStar(detail.id, !detail.starred)}
          allCollections={collections}
          onAddToCollection={handleAddToCollection}
          onRemoveFromCollection={handleRemoveFromCollection}
          allTags={tags}
          onAddTag={handleAddTagToEntry}
          onRemoveTag={handleRemoveTagFromEntry}
          onAttachmentsChanged={() => {
            if (selectedId != null) reloadDetail(selectedId);
            loadEntries();
          }}
          onAttachmentAdded={(attachmentId) => {
            setIndexingCount(c => c + 1);
            invoke("index_attachment", { id: attachmentId })
              .catch(console.error)
              .finally(() => setIndexingCount(c => Math.max(0, c - 1)));
          }}
        />
      )}

      {showBibtexSync && (
        <BibtexSyncSheet
          initialPath={bibtexSyncPath}
          lastSynced={bibtexLastSynced}
          lastError={bibtexLastError}
          onPathChanged={(p) => setBibtexSyncPath(p)}
          onClose={() => setShowBibtexSync(false)}
        />
      )}

      {dragEntryId !== null && (
        <div style={{
          position: "fixed",
          left: dragPos.x + 14, top: dragPos.y - 10,
          pointerEvents: "none", zIndex: 9999,
          background: "var(--surface)",
          border: "1px solid var(--border-strong)",
          borderRadius: 6, padding: "4px 10px",
          fontSize: 12, color: "var(--text-mute)",
          boxShadow: "0 4px 14px rgba(0,0,0,0.18)",
          maxWidth: 220, overflow: "hidden",
          textOverflow: "ellipsis", whiteSpace: "nowrap",
        }}>
          {selectedIds.has(dragEntryId) && selectedIds.size > 1
            ? `${selectedIds.size} 件の文献`
            : (entries.find(e => e.id === dragEntryId)?.title ?? "文献")}
        </div>
      )}
    </div>
  );
}
