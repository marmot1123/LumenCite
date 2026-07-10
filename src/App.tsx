import { useState, useMemo, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
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
import { SettingsModal } from "./components/SettingsModal";
import { DetailView } from "./components/detail/DetailView";
import { SummarySheet } from "./components/detail/SummarySheet";
import { CommandPalette } from "./components/CommandPalette";
import { ChatScreen } from "./components/chat/ChatScreen";
import { useChatStore } from "./chat/store";

import type { EntrySummary, EntryDetail, EntryInput, Collection, Tag, ViewMode, SearchScope, FulltextHit, SidebarCounts, ImportResult, LlmSettings, EntryFilter } from "./types";
import { EMPTY_FILTER, isFilterActive } from "./types";

const EMPTY_COUNTS: SidebarCounts = {
  total: 0, starred: 0, unfiled: 0, trash: 0, collections: {}, tags: {},
};

type LoadEntriesArgs = {
  collectionId?: number | null;
  tagId?: number | null;
  view?: string | null;
  filter?: EntryFilter;
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

function viewLabel(
  selectedView: string,
  collections: Collection[],
  t: TFunction,
): { title: string; subtitle?: string } {
  if (selectedView === "all")      return { title: t("viewTitles.all"),      subtitle: t("viewTitles.allSub") };
  if (selectedView === "starred")  return { title: t("viewTitles.starred"),  subtitle: t("viewTitles.starredSub") };
  if (selectedView === "recent")   return { title: t("viewTitles.recent"),   subtitle: t("viewTitles.recentSub") };
  if (selectedView === "unfiled")  return { title: t("viewTitles.unfiled"),  subtitle: t("viewTitles.unfiledSub") };
  if (selectedView === "trash")    return { title: t("viewTitles.trash"),    subtitle: t("viewTitles.trashSub") };
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
    return { title: col?.name ?? t("viewTitles.collection"), subtitle: t("viewTitles.collection") };
  }
  if (selectedView.startsWith("tag:")) {
    return { title: `#${selectedView.slice(4)}`, subtitle: t("viewTitles.tag") };
  }
  return { title: t("viewTitles.entries") };
}

export default function App() {
  const { density } = useTheme();
  const { t } = useTranslation();

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
  // 一覧の複合フィルタ（v0.6.0）。ビュー切替をまたいで保持し、明示クリアするまで持続する。
  const [filter, setFilter] = useState<EntryFilter>(EMPTY_FILTER);
  const [fulltextHits, setFulltextHits] = useState<FulltextHit[]>([]);
  const [indexingCount, setIndexingCount] = useState(0);
  const [viewMode, setViewMode] = useState<ViewMode>("table");
  const [showAdd, setShowAdd] = useState(false);
  const [showEdit, setShowEdit] = useState(false);
  const [showBibtexSync, setShowBibtexSync] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [screen, setScreen] = useState<"library" | "detail" | "chat">("library");
  const [showSummary, setShowSummary] = useState(false);
  const [showPalette, setShowPalette] = useState(false);
  const [settingsInitialTab, setSettingsInitialTab] = useState<"appearance" | "llm" | "bibtex" | "updates" | "data" | "about" | undefined>(undefined);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  // 詳細ビューに入るときは、ライブラリのドラッグ操作で残った user-select: none をクリアして
  // PDF テキスト選択が祖先継承で阻害されないようにする。
  useEffect(() => {
    if (screen === "detail") {
      document.body.style.userSelect = "";
    }
  }, [screen]);

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

  // メニューバー「Settings…」(⌘+, / Ctrl+,) と「About …」から発火されるイベントを購読
  useEffect(() => {
    const unlistenSettings = listen("open-settings", () => {
      setSettingsInitialTab(undefined);
      setShowSettings(true);
    });
    const unlistenAbout = listen("open-about", () => {
      setSettingsInitialTab("about");
      setShowSettings(true);
    });
    return () => {
      unlistenSettings.then(u => u());
      unlistenAbout.then(u => u());
    };
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

  // 応答順序ガード: 遅れて解決した古い検索・一覧応答が新しい表示を上書きしないようにする
  const loadSeqRef = useRef(0);

  const loadEntries = (view = selectedView, query = debouncedSearch, scope = searchScope, flt = filter) => {
    // サイドバー件数は entries とは独立した集計値。loadEntries が呼ばれるたびに
    // refresh しておくことで、view 切替直後にも最新の件数が表示される。
    reloadCounts();

    const seq = ++loadSeqRef.current;
    const ifCurrent = <T,>(fn: (v: T) => void) => (v: T) => {
      if (seq === loadSeqRef.current) fn(v);
    };

    const collectionId = view.startsWith("col:") ? Number(view.slice(4)) : null;
    const tagId = view.startsWith("tag:") ? Number(view.slice(4)) : null;
    const viewName = viewParam(view);
    const trimmed = query.trim();
    // フィルタは無制約なら backend へ渡さない（undefined 扱い）ことで従来経路と完全一致させる。
    const filterArg = isFilterActive(flt) ? flt : undefined;

    if (scope === "fulltext") {
      // 全文モード時は entries テーブルは現在のビューで埋めておき、結果リストは fulltextHits に持つ。
      // 全文検索結果へのフィルタ適用は v0.6.0 スコープ外（Toolbar でフィルタ無効化済み）。
      const args: LoadEntriesArgs = { collectionId, tagId, view: viewName };
      invoke<EntrySummary[]>("get_entries", args).then(ifCurrent(setEntries)).catch(console.error);
      if (trimmed) {
        // view を渡してゴミ箱と現役を分離する（CR-001）。trash ビューでの検索が現役を返さない。
        invoke<FulltextHit[]>("fulltext_search", { query: trimmed, collectionId, tagId, view: viewName })
          .then(ifCurrent(setFulltextHits))
          .catch((e) => { console.error(e); ifCurrent(setFulltextHits)([]); });
      } else {
        setFulltextHits([]);
      }
      return;
    }

    if (trimmed) {
      // view を渡してゴミ箱と現役を分離する（CR-001）。
      invoke<EntrySummary[]>("search_entries", { query: trimmed, collectionId, tagId, view: viewName, filter: filterArg })
        .then(ifCurrent(setEntries))
        .catch(console.error);
    } else {
      const args: LoadEntriesArgs = { collectionId, tagId, view: viewName, filter: filterArg };
      invoke<EntrySummary[]>("get_entries", args).then(ifCurrent(setEntries)).catch(console.error);
    }
  };

  // load entries when view, debounced search, scope, or filter changes
  useEffect(() => { loadEntries(); }, [selectedView, debouncedSearch, searchScope, filter]);

  // チャット中に LLM が書き込みツールを実行すると dataVersion が進む。
  // チャット画面でも App 本体は mount されたままなので、ここで一覧を再読込しておけば
  // ライブラリへ戻る前からリアルタイムに最新化される。初期値 0 のときは何もしない。
  const chatDataVersion = useChatStore((s) => s.dataVersion);
  useEffect(() => {
    if (chatDataVersion > 0) loadEntries();
    // loadEntries は毎レンダー再生成されるが、ここは dataVersion の変化時のみ走らせたい
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chatDataVersion]);

  // MCP サーバー公開（Phase 2）経由で外部クライアント（Claude Code 等）が DB を
  // 書き換えると backend が "entries-changed" を emit する。常に最新の loadEntries を
  // 呼べるよう ref 経由にし、リスナー自体は mount 時に 1 回だけ張る。
  const loadEntriesRef = useRef(loadEntries);
  loadEntriesRef.current = loadEntries;
  useEffect(() => {
    const un = listen("entries-changed", () => { loadEntriesRef.current(); });
    return () => { un.then((u) => u()); };
  }, []);

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
    let cancelled = false; // 素早い選択切替で古い応答が勝たないように
    invoke<EntryDetail>("get_entry", { id: selectedId })
      .then((d) => { if (!cancelled) setDetail(d); })
      .catch(() => { if (!cancelled) setDetail(null); });
    return () => { cancelled = true; };
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
      // Added（登録日時）: created_at は ISO 風文字列なので辞書順比較でよい（CR-036）。
      // これが無いと "added" 列は常に同順（バックエンド順のまま）で asc/desc が効かなかった。
      if (sort.key === "added")   { av = a.created_at ?? "";             bv = b.created_at ?? ""; }
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

  // ゴミ箱ビュー全体に対する一括永久削除。
  // 表示中 id ではなく専用 empty_trash コマンド（DB 側で deleted_at IS NOT NULL を評価）を使う。
  // これにより、検索・フィルタで現役エントリが表示に混ざっても hard delete されない（CR-001）。
  const handleEmptyTrash = () => {
    const ids = entries.map(e => e.id);
    if (ids.length === 0) return;
    const ok = window.confirm(t("confirm.permanentDelete", { count: ids.length }));
    if (!ok) return;
    invoke("empty_trash")
      .then(() => { clearSelection(); setDetail(null); loadEntries(); })
      .catch(console.error);
  };

  // キーボードショートカット。
  // - Esc: 選択解除
  // - Cmd/Ctrl+F: 検索欄にフォーカス
  // - Cmd/Ctrl+N: 文献を追加
  // - ↑/↓: 行選択を上下に移動
  // - Delete/Backspace: 選択をゴミ箱へ（通常ビュー）
  // - Cmd/Ctrl+Backspace: 選択を永久削除（ゴミ箱ビューのみ）
  useEffect(() => {
    const isModalOpen = showAdd || showEdit || showBibtexSync || showSettings || showPalette || screen === "detail" || screen === "chat";
    const isEditableTarget = (t: EventTarget | null) => {
      const el = t as HTMLElement | null;
      if (!el) return false;
      const tag = el.tagName;
      return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
    };

    const onKey = (e: KeyboardEvent) => {
      // Esc: 選択解除（モーダルが開いているときは触らない）
      if (e.key === "Escape") {
        if (!isModalOpen && selectedIds.size > 0) clearSelection();
        return;
      }

      // Cmd/Ctrl+J: チャット画面の表示/非表示（暫定の入口。#18 でコマンドパレット等から正式化）
      if ((e.metaKey || e.ctrlKey) && (e.key === "j" || e.key === "J")) {
        if (showAdd || showEdit || showBibtexSync || showSettings) return;
        e.preventDefault();
        setScreen(s => (s === "chat" ? "library" : "chat"));
        return;
      }

      // Cmd/Ctrl+K: コマンドパレット切替（他モーダルが開いていない時のみ）
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        if (showAdd || showEdit || showBibtexSync || showSettings || showSummary) return;
        e.preventDefault();
        setShowPalette(p => !p);
        return;
      }

      // Cmd/Ctrl+F: 検索欄にフォーカス（入力中でもブラウザのページ内検索を上書きする）
      if ((e.metaKey || e.ctrlKey) && (e.key === "f" || e.key === "F")) {
        if (isModalOpen) return;
        e.preventDefault();
        const input = document.getElementById("toolbar-search") as HTMLInputElement | null;
        input?.focus();
        input?.select();
        return;
      }

      // 入力中・モーダル表示中は以下のショートカットを発火させない
      if (isModalOpen || isEditableTarget(e.target)) return;

      // Cmd/Ctrl+N: 文献を追加
      if ((e.metaKey || e.ctrlKey) && (e.key === "n" || e.key === "N")) {
        e.preventDefault();
        setShowAdd(true);
        return;
      }

      // ↑ / ↓: 行選択を移動
      if (e.key === "ArrowUp" || e.key === "ArrowDown") {
        const ids = filteredEntries.map(en => en.id);
        if (ids.length === 0) return;
        e.preventDefault();
        const cursor = anchorId ?? selectedId;
        const curIdx = cursor != null ? ids.indexOf(cursor) : -1;
        const nextIdx = curIdx === -1
          ? (e.key === "ArrowDown" ? 0 : ids.length - 1)
          : e.key === "ArrowDown"
            ? Math.min(curIdx + 1, ids.length - 1)
            : Math.max(curIdx - 1, 0);
        selectSingle(ids[nextIdx]);
        return;
      }

      // Cmd/Ctrl+Backspace: ゴミ箱ビューでの永久削除
      if ((e.metaKey || e.ctrlKey) && e.key === "Backspace") {
        if (selectedView !== "trash" || selectedIds.size === 0) return;
        e.preventDefault();
        const ok = window.confirm(t("confirm.permanentDelete", { count: selectedIds.size }));
        if (ok) handlePurge();
        return;
      }

      // Delete / Backspace: ゴミ箱送り（ゴミ箱ビューでは何もしない）
      if ((e.key === "Delete" || e.key === "Backspace") && !e.metaKey && !e.ctrlKey) {
        if (selectedIds.size === 0 || selectedView === "trash") return;
        e.preventDefault();
        handleTrash();
        return;
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedIds, anchorId, selectedId, filteredEntries, selectedView, showAdd, showEdit, showBibtexSync, showSettings, showSummary, showPalette, screen]);

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

  // タグへのドロップ（既存タグを複数件に一括付与）
  const handleDropToTag = (tagId: number, draggingId: number) => {
    const ids = selectedIds.has(draggingId) && selectedIds.size > 1
      ? [...selectedIds]
      : [draggingId];
    invoke("bulk_add_tag", { ids, tagId })
      .then(() => {
        if (selectedId != null && ids.includes(selectedId)) reloadDetail(selectedId);
        loadEntries();
      })
      .catch(console.error);
  };

  // 特殊ビュー（お気に入り / ゴミ箱）へのドロップ
  const handleDropToView = (view: "starred" | "trash", draggingId: number) => {
    const ids = selectedIds.has(draggingId) && selectedIds.size > 1
      ? [...selectedIds]
      : [draggingId];
    if (view === "trash") {
      invoke("bulk_trash", { ids })
        .then(() => { clearSelection(); setDetail(null); loadEntries(); })
        .catch(console.error);
    } else if (view === "starred") {
      Promise.all(ids.map(id => invoke("set_starred", { id, starred: true })))
        .then(() => {
          if (selectedId != null && ids.includes(selectedId)) reloadDetail(selectedId);
          loadEntries();
        })
        .catch(console.error);
    }
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
    handleExportBibtex(ids, t("export.bulkFilename", { count: ids.length }));
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

  // 抄録／ノートのインライン編集を保存する。update_entry は EntryInput を丸ごと
  // 要求するので、現在の detail から組み立てて該当フィールドだけ差し替える。
  const handleUpdateField = (field: "abstract_" | "notes", value: string) => {
    if (!detail) return;
    const targetId = detail.id;
    const trimmed = value.trim();
    const input: EntryInput = {
      title:        detail.title,
      year:         detail.year,
      entry_type:   detail.entry_type,
      doi:          detail.doi,
      isbn:         detail.isbn,
      arxiv_id:     detail.arxiv_id,
      url:          detail.url,
      abstract_:    detail.abstract_,
      notes:        detail.notes,
      extra_fields: detail.extra_fields,
      author_names: detail.authors.map(a => a.name),
      tag_ids:      detail.tags.map(t => t.id),
      [field]:      trimmed || undefined,
    };
    // Promise を返して呼び出し側が await / エラーハンドリングできるようにする（CR-034）。
    return invoke<EntryDetail>("update_entry", { id: targetId, input })
      .then(updated => {
        // 編集中に別の文献に切り替えていた場合は detail を上書きしない
        setDetail(prev => (prev && prev.id === targetId ? updated : prev));
      });
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
    // 同期パスが設定されていれば、その親ディレクトリを保存ダイアログの初期位置に使う。
    invoke<string | null>("save_bibtex", {
      entryIds: ids,
      defaultName,
      defaultDirectory: bibtexSyncPath ?? null,
    }).catch(console.error);
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

  const label = viewLabel(selectedView, collections, t);

  // 指定文献をスコープにした新規 Chat セッションを作って Chat 画面へ遷移する。
  // ids が空ならライブラリ全体スコープ。
  const startChatWithEntries = async (ids: number[]) => {
    try {
      const settings = await invoke<LlmSettings>("get_llm_settings");
      await useChatStore.getState().createSession({
        title: t("chat.newChat"),
        provider: settings.provider,
        model: settings.model,
        scopeMode: ids.length > 0 ? "entries" : "all",
        entryIds: ids,
      });
      setScreen("chat");
    } catch (e) {
      console.error("failed to start chat", e);
    }
  };

  // 画面に依存しない共通オーバーレイ（設定モーダル・コマンドパレット）。
  // library / detail / chat いずれの画面でも ⌘, やコマンドパレットが効くよう全分岐で描画する。
  const globalOverlays = (
    <>
      {showSettings && (
        <SettingsModal
          onClose={() => { setShowSettings(false); setSettingsInitialTab(undefined); }}
          onOpenBibtexSync={() => { setShowSettings(false); setShowBibtexSync(true); }}
          initialTab={settingsInitialTab}
        />
      )}
      <CommandPalette
        open={showPalette}
        onClose={() => setShowPalette(false)}
        entries={entries}
        onSelectEntry={selectSingle}
        onOpenDetail={(id) => { selectSingle(id); setScreen("detail"); }}
        onNewEntry={() => setShowAdd(true)}
        onOpenChat={() => setScreen("chat")}
        onOpenSettings={() => setShowSettings(true)}
        onOpenBibtexSync={() => setShowBibtexSync(true)}
        onSyncBibtexNow={() => { void invoke("sync_bibtex_now"); }}
        onSelectView={setSelectedView}
      />
    </>
  );

  if (screen === "chat") {
    return (
      <>
        <ChatScreen
          onBack={() => {
            // LLM がツール経由で DB を更新している可能性があるため、
            // ライブラリへ戻るタイミングで一覧を再読込して表示を最新化する。
            loadEntries();
            setScreen("library");
          }}
          onOpenSettings={() => setShowSettings(true)}
        />
        {globalOverlays}
      </>
    );
  }

  if (screen === "detail" && detail) {
    return (
      <>
        <DetailView
          key={detail.id} // エントリ切替時に page 等の state を持ち越さないよう再マウント
          entry={detail}
          onBack={() => setScreen("library")}
          onToggleStar={() => handleToggleStar(detail.id, !detail.starred)}
          onUpdateNotes={(notes) => handleUpdateField("notes", notes)}
          onSelectEntry={(id) => { selectSingle(id); }}
          onOpenInWindow={(attachmentId) => { void invoke("open_pdf_viewer", { id: attachmentId }); }}
          onSummarize={() => setShowSummary(true)}
          onChat={() => { void startChatWithEntries([detail.id]); }}
          onAuthorEdited={() => {
            // 著者表記が変われば一覧の表示・FTS インデックスにも波及するため両方再読込する
            reloadDetail(detail.id);
            loadEntries();
          }}
          onAttachmentsChanged={() => {
            // 添付追加で一覧の添付バッジ・件数も変わりうるため両方再読込する
            reloadDetail(detail.id);
            loadEntries();
          }}
        />
        {globalOverlays}
        {showSummary && (
          <SummarySheet
            entry={detail}
            onClose={() => setShowSummary(false)}
            onSavedToNotes={async (newNotes) => {
              // 保存を await し、失敗は SummarySheet 側で表示させる（成功時のみ閉じる・CR-034）。
              await handleUpdateField("notes", newNotes);
              setShowSummary(false);
            }}
            onOpenSettings={() => { setShowSummary(false); setShowSettings(true); }}
          />
        )}
      </>
    );
  }

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
        onDropToView={handleDropToView}
        onDropToTag={handleDropToTag}
        draggingId={dragEntryId}
        bibtexSyncPath={bibtexSyncPath}
        bibtexLastSynced={bibtexLastSynced}
        bibtexLastError={bibtexLastError}
        onOpenBibtexSync={() => setShowBibtexSync(true)}
        onOpenSettings={() => setShowSettings(true)}
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
          onImport={async () => {
            try {
              const path = await invoke<string | null>("pick_bibtex_file");
              if (!path) return;
              const result = await invoke<ImportResult>("import_bibtex_file", { path });
              setImportError(null);
              setImportResult(result);
              loadEntries();
            } catch (e) {
              setImportResult(null);
              setImportError(typeof e === "string" ? e : (e as Error)?.message ?? String(e));
            }
          }}
          onExportBibtex={() => handleExportBibtex(undefined, label.title)}
          exportDisabled={filteredEntries.length === 0}
          inTrash={selectedView === "trash"}
          onEmptyTrash={handleEmptyTrash}
          emptyTrashDisabled={entries.length === 0}
          filter={filter}
          onFilterChange={setFilter}
          onClearFilter={() => setFilter(EMPTY_FILTER)}
          tags={tags}
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
              onOpenDetail={(id) => { selectSingle(id); setScreen("detail"); }}
              sort={sort}
              onSort={handleSort}
              density={density}
              draggingId={dragEntryId}
              onStartDrag={handleStartDrag}
              onToggleStar={handleToggleStar}
              isEmptyLibrary={counts.total === 0 && !debouncedSearch.trim()}
              onAddEntry={() => setShowAdd(true)}
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
            <PlaceholderView title={t("placeholder.timelineTitle")} body={t("placeholder.timelineBody")} />
          )}
          {searchScope === "meta" && viewMode === "graph" && (
            <PlaceholderView title={t("placeholder.graphTitle")} body={t("placeholder.graphBody")} />
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
              onSelectExisting={(id) => {
                setShowAdd(false);
                setSelectedView("all");
                selectSingle(id);
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
          onChatWith={() => { void startChatWithEntries([...selectedIds]); }}
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
          onUpdateField={handleUpdateField}
          onSelectEntry={selectSingle}
          onSummarize={detail ? () => setShowSummary(true) : undefined}
          onOpenDetail={detail ? () => setScreen("detail") : undefined}
          onAuthorEdited={() => {
            if (selectedId != null) reloadDetail(selectedId);
            loadEntries();
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

      {globalOverlays}

      {showSummary && detail && (
        <SummarySheet
          entry={detail}
          onClose={() => setShowSummary(false)}
          onSavedToNotes={async (newNotes) => {
            // 保存を await し、失敗は SummarySheet 側で表示させる（成功時のみ閉じる・CR-034）。
            await handleUpdateField("notes", newNotes);
            setShowSummary(false);
          }}
          onOpenSettings={() => { setShowSummary(false); setShowSettings(true); }}
        />
      )}

      {(importResult || importError) && (
        <div
          onClick={() => { setImportResult(null); setImportError(null); }}
          style={{
            position: "fixed", inset: 0, background: "rgba(0,0,0,0.30)",
            display: "flex", alignItems: "center", justifyContent: "center", zIndex: 1100,
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              width: 460, maxWidth: "90vw",
              background: "var(--surface)",
              border: "1px solid var(--border-strong)",
              borderRadius: 10,
              boxShadow: "0 12px 32px rgba(0,0,0,0.18)",
              padding: "20px 22px 16px",
            }}
          >
            <div style={{ fontSize: 15, fontWeight: 600, color: "var(--text)", marginBottom: 10 }}>
              {t("toolbar.importResultTitle")}
            </div>
            {importError ? (
              <div style={{
                padding: "10px 12px", borderRadius: 6,
                background: "var(--danger-bg)", border: "1px solid var(--danger-border)",
                color: "var(--danger-text)", fontSize: 12, lineHeight: 1.55,
                wordBreak: "break-all",
              }}>
                {t("toolbar.importFailed", { error: importError })}
              </div>
            ) : importResult && (
              <>
                <div style={{ display: "flex", gap: 24, marginBottom: 8 }}>
                  <div>
                    <div style={{
                      fontSize: 26, fontWeight: 700, color: "var(--accent-strong)",
                      fontVariantNumeric: "tabular-nums",
                    }}>{importResult.imported}</div>
                    <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>
                      {t("toolbar.importedSuffix")}
                    </div>
                  </div>
                  {importResult.skipped > 0 && (
                    <div>
                      <div style={{
                        fontSize: 26, fontWeight: 700, color: "var(--text-mute)",
                        fontVariantNumeric: "tabular-nums",
                      }}>{importResult.skipped}</div>
                      <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>
                        {t("toolbar.skippedSuffix")}
                      </div>
                    </div>
                  )}
                </div>
                {importResult.errors.length > 0 && (
                  <div style={{ marginTop: 10, fontSize: 11, color: "var(--text-mute)" }}>
                    <div style={{ marginBottom: 4, fontWeight: 600 }}>{t("toolbar.importErrorsLabel")}</div>
                    <div style={{ maxHeight: 140, overflow: "auto" }}>
                      {importResult.errors.slice(0, 20).map((e, i) => (
                        <div key={i} style={{
                          fontFamily: "var(--mono)", color: "var(--text-faint)",
                          padding: "1px 0",
                        }}>{e}</div>
                      ))}
                    </div>
                  </div>
                )}
              </>
            )}
            <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 14 }}>
              <button
                onClick={() => { setImportResult(null); setImportError(null); }}
                style={{
                  padding: "5px 12px", borderRadius: 5,
                  border: "1px solid var(--border-strong)",
                  background: "var(--surface)", color: "var(--text)",
                  fontSize: 12, cursor: "pointer",
                }}
              >{t("common.close")}</button>
            </div>
          </div>
        </div>
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
            ? t("dnd.multiEntries", { count: selectedIds.size })
            : (entries.find(e => e.id === dragEntryId)?.title ?? t("dnd.fallback"))}
        </div>
      )}
    </div>
  );
}
