// LumenCite — Main App (toolbar + composition)

const { useState: useStateMain, useMemo: useMemoMain, useEffect: useEffectMain } = React;
const { Icon: IconM } = window.LumenCommon;
const { ENTRIES: ALL_ENTRIES, COLLECTIONS, TAGS_USED } = window.LUMEN_DATA;
const Sidebar = window.LumenSidebar;
const EntriesTable = window.LumenTable;
const DetailPanel = window.LumenDetail;

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "light",
  "accent": "amber",
  "density": "default",
  "showDetail": true,
  "showSidebar": true
}/*EDITMODE-END*/;

const ACCENTS = {
  amber:  { strong: "oklch(0.62 0.14 65)",  soft: "oklch(0.95 0.04 70)",  ring: "oklch(0.7 0.13 65 / 0.25)" },
  indigo: { strong: "oklch(0.52 0.16 270)", soft: "oklch(0.95 0.04 270)", ring: "oklch(0.6 0.15 270 / 0.25)" },
  teal:   { strong: "oklch(0.55 0.10 195)", soft: "oklch(0.95 0.04 200)", ring: "oklch(0.62 0.10 200 / 0.25)" },
  rose:   { strong: "oklch(0.58 0.16 15)",  soft: "oklch(0.95 0.04 15)",  ring: "oklch(0.65 0.15 15 / 0.25)" },
};

function applyTheme(theme, accentName) {
  const dark = theme === "dark";
  const a = ACCENTS[accentName] || ACCENTS.amber;
  const v = (k, val) => document.documentElement.style.setProperty(k, val);
  if (dark) {
    // Softer, gray-based dark — lower contrast, less pure black
    v("--bg", "oklch(0.27 0.004 80)");
    v("--surface", "oklch(0.31 0.004 80)");
    v("--surface-2", "oklch(0.29 0.004 80)");
    v("--sidebar", "oklch(0.285 0.004 80)");
    v("--border", "oklch(0.38 0.004 80)");
    v("--border-subtle", "oklch(0.34 0.004 80)");
    v("--border-strong", "oklch(0.44 0.004 80)");
    v("--text", "oklch(0.86 0.004 80)");
    v("--text-mute", "oklch(0.66 0.004 80)");
    v("--text-faint", "oklch(0.52 0.004 80)");
    v("--row-hover", "oklch(0.34 0.004 80)");
    v("--row-selected", "oklch(0.38 0.018 70)");
    v("--hover", "oklch(0.34 0.004 80)");
    v("--accent-strong", "oklch(0.74 0.12 65)");
    v("--accent-soft", "oklch(0.36 0.05 65)");
    v("--accent-ring", a.ring);
  } else {
    v("--bg", "oklch(0.985 0.003 80)");
    v("--surface", "#ffffff");
    v("--surface-2", "oklch(0.975 0.004 80)");
    v("--sidebar", "oklch(0.972 0.004 80)");
    v("--border", "oklch(0.92 0.005 80)");
    v("--border-subtle", "oklch(0.95 0.004 80)");
    v("--border-strong", "oklch(0.86 0.006 80)");
    v("--text", "oklch(0.22 0.01 70)");
    v("--text-mute", "oklch(0.5 0.008 70)");
    v("--text-faint", "oklch(0.65 0.005 70)");
    v("--row-hover", "oklch(0.965 0.005 80)");
    v("--row-selected", "oklch(0.955 0.02 70)");
    v("--hover", "oklch(0.95 0.005 80)");
    v("--accent-strong", a.strong);
    v("--accent-soft", a.soft);
    v("--accent-ring", a.ring);
  }
  v("--mono", '"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, monospace');
}

// ──────────────────────────────────────────────────────────
// Toolbar
// ──────────────────────────────────────────────────────────
// View tabs (Table / Covers / Timeline / Graph) — switchable view modes
function ViewTabs({ viewMode, setViewMode }) {
  const tabs = [
    { id: "table", label: "表", icon: "list", enabled: true },
    { id: "covers", label: "カバー", icon: "grid", enabled: true },
    { id: "timeline", label: "タイムライン", icon: "clock", enabled: false },
    { id: "graph", label: "引用グラフ", icon: "sync", enabled: false },
  ];
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 2,
      padding: "0 12px", height: 34, flexShrink: 0,
      borderBottom: "1px solid var(--border)",
      background: "var(--surface)",
    }}>
      {tabs.map((t) => {
        const active = viewMode === t.id;
        return (
          <button key={t.id}
            onClick={() => t.enabled && setViewMode(t.id)}
            disabled={!t.enabled}
            style={{
              display: "inline-flex", alignItems: "center", gap: 5,
              padding: "0 10px", height: 34,
              border: "none", background: "transparent",
              fontSize: 12, fontWeight: active ? 600 : 500,
              color: !t.enabled ? "var(--text-faint)"
                : active ? "var(--text)" : "var(--text-mute)",
              cursor: t.enabled ? "pointer" : "not-allowed",
              opacity: t.enabled ? 1 : 0.55,
              borderBottom: active ? "2px solid var(--accent-strong)" : "2px solid transparent",
              marginBottom: -1,
              letterSpacing: "0.01em",
            }}>
            <IconM name={t.icon} size={12} color={active ? "var(--text)" : "var(--text-mute)"} />
            {t.label}
            {!t.enabled && (
              <span style={{
                fontSize: 9.5, padding: "1px 4px", borderRadius: 3,
                background: "var(--surface-2)", color: "var(--text-faint)",
                marginLeft: 2, fontWeight: 500,
              }}>soon</span>
            )}
          </button>
        );
      })}
      <div style={{ flex: 1 }} />
      <span style={{ fontSize: 11, color: "var(--text-faint)" }}>
        {viewMode === "table" ? "メタデータ重視" :
         viewMode === "covers" ? "PDFサムネイル" : ""}
      </span>
    </div>
  );
}

// PDF Cover Grid
function CoverCard({ entry, selected, onClick }) {
  const [hover, setHover] = useStateMain(false);
  // generate per-entry color seed
  const hue = (entry.id * 47) % 360;
  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "flex", flexDirection: "column", gap: 8,
        padding: 10, borderRadius: 8, cursor: "pointer",
        background: selected ? "var(--row-selected)"
          : hover ? "var(--row-hover)" : "transparent",
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
        {/* striped placeholder */}
        <svg width="100%" height="100%" style={{ position: "absolute", inset: 0, opacity: 0.18 }}>
          <defs>
            <pattern id={`stripe-${entry.id}`} width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <line x1="0" y1="0" x2="0" y2="6" stroke={`oklch(0.4 0.05 ${hue})`} strokeWidth="1"/>
            </pattern>
          </defs>
          <rect width="100%" height="100%" fill={`url(#stripe-${entry.id})`}/>
        </svg>
        {/* type chip */}
        <div style={{
          position: "absolute", top: 8, left: 8,
          padding: "1px 6px", borderRadius: 3,
          background: "rgba(255,255,255,0.85)",
          fontSize: 9, fontWeight: 600, letterSpacing: "0.04em",
          color: `oklch(0.35 0.08 ${hue})`,
          textTransform: "uppercase",
          backdropFilter: "blur(4px)",
        }}>{entry.type}</div>
        {/* paper-style text simulacrum */}
        <div style={{
          position: "absolute", left: 12, right: 12, top: "32%",
          fontFamily: "'IBM Plex Serif', Georgia, serif",
          fontSize: 8.5, fontWeight: 600,
          color: `oklch(0.30 0.06 ${hue})`,
          lineHeight: 1.25, letterSpacing: "-0.01em",
          display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: 4, overflow: "hidden",
        }}>{entry.title}</div>
        <div style={{
          position: "absolute", left: 12, right: 12, bottom: 14,
          fontSize: 7, color: `oklch(0.40 0.05 ${hue})`,
          fontStyle: "italic",
        }}>
          {(entry.authors || [""])[0]}{entry.authors && entry.authors.length > 1 ? " et al." : ""}
        </div>
        {entry.year && <div style={{
          position: "absolute", right: 10, bottom: 6,
          fontSize: 7.5, fontFamily: "var(--mono)", color: `oklch(0.40 0.05 ${hue})`,
        }}>{entry.year}</div>}
        {/* attached corner */}
        {entry.attached && <div style={{
          position: "absolute", top: 8, right: 8,
          width: 16, height: 16, borderRadius: "50%",
          background: "rgba(255,255,255,0.85)",
          display: "flex", alignItems: "center", justifyContent: "center",
        }}><IconM name="paperclip" size={9} color={`oklch(0.35 0.08 ${hue})`} /></div>}
      </div>
      {/* meta */}
      <div style={{ minHeight: 38 }}>
        <div style={{
          fontSize: 11.5, fontWeight: 550, color: "var(--text)",
          lineHeight: 1.3, letterSpacing: "-0.005em",
          display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: 2, overflow: "hidden",
        }}>{entry.title}</div>
        <div style={{
          marginTop: 3, fontSize: 10.5, color: "var(--text-faint)",
          overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
        }}>
          {(entry.authors || [""])[0]}{entry.authors && entry.authors.length > 1 ? " et al." : ""} · {entry.year || ""}
        </div>
      </div>
    </div>
  );
}

function CoversGrid({ entries, selectedId, onSelect }) {
  return (
    <div style={{
      flex: 1, overflow: "auto", padding: "16px 18px",
      background: "var(--surface)",
    }}>
      <div style={{
        display: "grid", gap: 10,
        gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))",
      }}>
        {entries.map((e) => (
          <CoverCard key={e.id} entry={e}
            selected={selectedId === e.id}
            onClick={() => onSelect(e.id)} />
        ))}
      </div>
    </div>
  );
}

function PlaceholderView({ title, body }) {
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

function ToolbarBtn({ icon, label, onClick, active, primary }) {
  const [hover, setHover] = useStateMain(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "inline-flex", alignItems: "center", gap: 5,
        padding: label ? "5px 9px 5px 8px" : "5px 6px",
        borderRadius: 6,
        border: "1px solid " + (active ? "var(--border-strong)" : "transparent"),
        background: primary ? "var(--accent-strong)"
          : active ? "var(--surface-2)"
          : hover ? "var(--hover)" : "transparent",
        color: primary ? "white" : "var(--text)",
        fontSize: 12, fontWeight: 500, cursor: "pointer",
        transition: "background 80ms ease",
      }}
    >
      <IconM name={icon} size={13} color={primary ? "white" : "var(--text-mute)"} />
      {label && <span>{label}</span>}
    </button>
  );
}

function SegBtn({ icon, active, onClick, title }) {
  return (
    <button
      onClick={onClick} title={title}
      style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 26, height: 24, padding: 0,
        border: "none", borderRadius: 4,
        background: active ? "var(--surface)" : "transparent",
        boxShadow: active ? "0 0 0 1px var(--border) inset, 0 1px 0 rgba(0,0,0,0.03)" : "none",
        color: active ? "var(--text)" : "var(--text-mute)",
        cursor: "pointer",
      }}>
      <IconM name={icon} size={12} color={active ? "var(--text)" : "var(--text-mute)"} />
    </button>
  );
}

function Toolbar({ title, subtitle, count, search, setSearch, view, setView, onAddOpen }) {
  return (
    <header style={{
      flexShrink: 0,
      borderBottom: "1px solid var(--border)",
      background: "var(--surface)",
    }}>
      {/* row 1 — context + actions */}
      <div style={{
        display: "flex", alignItems: "center", gap: 12,
        padding: "10px 16px 10px 14px", height: 50,
      }}>
        <div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0 }}>
          <h1 style={{
            margin: 0, fontSize: 15, fontWeight: 600, color: "var(--text)",
            letterSpacing: "-0.01em",
            display: "flex", alignItems: "center", gap: 8,
          }}>
            {title}
            <span style={{
              fontSize: 11, fontWeight: 500, color: "var(--text-faint)",
              padding: "1px 7px", borderRadius: 999,
              background: "var(--surface-2)",
              fontVariantNumeric: "tabular-nums",
            }}>{count}</span>
          </h1>
          {subtitle && (
            <div style={{ fontSize: 11.5, color: "var(--text-faint)", marginTop: 2 }}>
              {subtitle}
            </div>
          )}
        </div>

        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <ToolbarBtn icon="upload" label="インポート" />
          <ToolbarBtn icon="plus" label="文献を追加" primary onClick={onAddOpen} />
        </div>
      </div>

      {/* row 2 — search + view controls */}
      <div style={{
        display: "flex", alignItems: "center", gap: 10,
        padding: "0 16px 10px",
      }}>
        {/* search */}
        <div style={{
          display: "flex", alignItems: "center", gap: 6,
          flex: 1, maxWidth: 460,
          padding: "5px 10px",
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          borderRadius: 6,
          height: 28,
        }}>
          <IconM name="search" size={12} color="var(--text-faint)" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="タイトル・著者・DOI・本文で検索…"
            style={{
              flex: 1, border: "none", outline: "none", background: "transparent",
              fontSize: 12.5, color: "var(--text)",
            }}
          />
          <span style={{
            fontSize: 10, color: "var(--text-faint)",
            padding: "1px 5px", border: "1px solid var(--border-strong)",
            borderRadius: 3, fontFamily: "var(--mono)",
          }}>⌘ K</span>
        </div>

        <ToolbarBtn icon="filter" label="フィルタ" />
        <div style={{ width: 1, height: 18, background: "var(--border)" }} />
        <ToolbarBtn icon="columns" label="列" />
        <div style={{ flex: 1 }} />
        <ToolbarBtn icon="sortAsc" label="並び替え" />
      </div>
    </header>
  );
}

// ──────────────────────────────────────────────────────────
// Add Entry sheet (DOI / arXiv / ISBN auto-fetch)
// ──────────────────────────────────────────────────────────
function AddSheet({ onClose }) {
  const [tab, setTab] = useStateMain("doi");
  return (
    <div style={{
      position: "absolute", inset: 0, zIndex: 20,
      background: "rgba(20, 18, 14, 0.28)",
      backdropFilter: "blur(2px)",
      display: "flex", alignItems: "flex-start", justifyContent: "center",
      paddingTop: 90,
    }} onClick={onClose}>
      <div onClick={(e) => e.stopPropagation()} style={{
        width: 460, background: "var(--surface)",
        borderRadius: 10, border: "1px solid var(--border-strong)",
        boxShadow: "0 20px 50px rgba(0,0,0,0.18), 0 1px 0 rgba(0,0,0,0.05)",
        overflow: "hidden",
      }}>
        <div style={{
          padding: "14px 18px 12px", borderBottom: "1px solid var(--border)",
        }}>
          <div style={{ fontSize: 14, fontWeight: 600, color: "var(--text)" }}>文献を追加</div>
          <div style={{ fontSize: 11.5, color: "var(--text-faint)", marginTop: 3 }}>
            識別子から自動でメタデータを取得します
          </div>
        </div>
        <div style={{ display: "flex", padding: "0 14px", borderBottom: "1px solid var(--border)" }}>
          {[["doi", "DOI"], ["arxiv", "arXiv"], ["isbn", "ISBN"], ["bibtex", "BibTeX 貼付"], ["manual", "手動入力"]].map(([k, l]) => (
            <button key={k} onClick={() => setTab(k)} style={{
              padding: "9px 11px", border: "none", background: "transparent",
              fontSize: 12, fontWeight: tab === k ? 600 : 500,
              color: tab === k ? "var(--text)" : "var(--text-mute)",
              borderBottom: tab === k ? "1.5px solid var(--accent-strong)" : "1.5px solid transparent",
              marginBottom: -1, cursor: "pointer",
            }}>{l}</button>
          ))}
        </div>
        <div style={{ padding: 18 }}>
          <input
            placeholder={
              tab === "doi" ? "10.48550/arXiv.1706.03762" :
              tab === "arxiv" ? "1706.03762" :
              tab === "isbn" ? "978-0387310732" :
              tab === "bibtex" ? "@article{vaswani2017,..." : "タイトル"
            }
            style={{
              width: "100%", padding: "8px 10px", borderRadius: 6,
              border: "1px solid var(--border-strong)",
              fontFamily: tab === "manual" ? "inherit" : "var(--mono)",
              fontSize: 12.5, color: "var(--text)",
              background: "var(--surface-2)", outline: "none",
              boxSizing: "border-box",
            }}
          />
          <div style={{
            marginTop: 10, fontSize: 11, color: "var(--text-faint)",
            display: "flex", alignItems: "center", gap: 6,
          }}>
            <IconM name="info" size={11} color="var(--text-faint)" />
            CrossRef / arXiv / Google Books から取得します
          </div>
        </div>
        <div style={{
          padding: "10px 14px", borderTop: "1px solid var(--border)",
          background: "var(--surface-2)",
          display: "flex", justifyContent: "flex-end", gap: 8,
        }}>
          <button onClick={onClose} style={{
            padding: "5px 12px", borderRadius: 5,
            border: "1px solid var(--border-strong)",
            background: "var(--surface)", color: "var(--text)",
            fontSize: 12, cursor: "pointer",
          }}>キャンセル</button>
          <button style={{
            padding: "5px 14px", borderRadius: 5, border: "none",
            background: "var(--accent-strong)", color: "white",
            fontSize: 12, fontWeight: 500, cursor: "pointer",
          }}>取得</button>
        </div>
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────────────────
// Tweaks
// ──────────────────────────────────────────────────────────
function LumenTweaks({ tweaks, setTweak }) {
  const { TweaksPanel, TweakSection, TweakRadio, TweakToggle, TweakColor } = window;
  return (
    <TweaksPanel>
      <TweakSection title="外観">
        <TweakRadio label="テーマ" value={tweaks.theme}
          onChange={(v) => setTweak("theme", v)}
          options={[{ value: "light", label: "ライト" }, { value: "dark", label: "ダーク" }]} />
        <TweakRadio label="アクセント" value={tweaks.accent}
          onChange={(v) => setTweak("accent", v)}
          options={[
            { value: "amber", label: "Amber" },
            { value: "indigo", label: "Indigo" },
            { value: "teal", label: "Teal" },
            { value: "rose", label: "Rose" },
          ]} />
      </TweakSection>
      <TweakSection title="表示">
        <TweakRadio label="行の高さ" value={tweaks.density}
          onChange={(v) => setTweak("density", v)}
          options={[
            { value: "compact", label: "高密度" },
            { value: "default", label: "標準" },
            { value: "comfortable", label: "余裕" },
          ]} />
        <TweakToggle label="左サイドバー" value={tweaks.showSidebar} onChange={(v) => setTweak("showSidebar", v)} />
        <TweakToggle label="右パネル" value={tweaks.showDetail} onChange={(v) => setTweak("showDetail", v)} />
      </TweakSection>
    </TweaksPanel>
  );
}

// ──────────────────────────────────────────────────────────
// App
// ──────────────────────────────────────────────────────────
function App() {
  const useTweaks = window.useTweaks;
  const [tweaks, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [selectedView, setSelectedView] = useStateMain("all");
  const [selectedId, setSelectedId] = useStateMain(1);
  const [sort, setSort] = useStateMain({ key: "added", dir: "desc" });
  const [search, setSearch] = useStateMain("");
  const [view, setView] = useStateMain("list");
  const [viewMode, setViewMode] = useStateMain("table");
  const [showAdd, setShowAdd] = useStateMain(false);

  useEffectMain(() => {
    applyTheme(tweaks.theme, tweaks.accent);
  }, [tweaks.theme, tweaks.accent]);

  const filteredEntries = useMemoMain(() => {
    let list = ALL_ENTRIES;
    if (selectedView === "starred") list = list.filter((e) => e.starred);
    else if (selectedView === "recent") list = list.slice().sort((a, b) => (b.added > a.added ? 1 : -1)).slice(0, 8);
    else if (selectedView === "unfiled") list = list.filter((e) => !e.collections || e.collections.length === 0);
    else if (selectedView.startsWith("col:")) {
      const colName = (() => {
        const id = selectedView.slice(4);
        const find = (cs) => {
          for (const c of cs) {
            if (c.id === id) return c.name;
            if (c.children) { const x = find(c.children); if (x) return x; }
          }
          return null;
        };
        return find(COLLECTIONS);
      })();
      list = list.filter((e) => (e.collections || []).includes(colName));
    } else if (selectedView.startsWith("tag:")) {
      const t = selectedView.slice(4);
      list = list.filter((e) => (e.tags || []).includes(t));
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      list = list.filter((e) =>
        e.title.toLowerCase().includes(q) ||
        (e.authors || []).some((a) => a.toLowerCase().includes(q)) ||
        (e.venue || "").toLowerCase().includes(q) ||
        (e.tags || []).some((t) => t.includes(q)) ||
        (e.doi || "").toLowerCase().includes(q) ||
        (e.arxiv || "").toLowerCase().includes(q)
      );
    }
    const dir = sort.dir === "asc" ? 1 : -1;
    list = list.slice().sort((a, b) => {
      let av, bv;
      if (sort.key === "title") { av = a.title; bv = b.title; }
      else if (sort.key === "authors") { av = (a.authors || [""])[0]; bv = (b.authors || [""])[0]; }
      else if (sort.key === "year") { av = a.year || 0; bv = b.year || 0; }
      else { av = a.added || ""; bv = b.added || ""; }
      return av < bv ? -dir : av > bv ? dir : 0;
    });
    return list;
  }, [selectedView, search, sort]);

  const onSort = (key) => {
    setSort((s) => s.key === key
      ? { key, dir: s.dir === "asc" ? "desc" : "asc" }
      : { key, dir: key === "title" || key === "authors" ? "asc" : "desc" });
  };

  const selected = ALL_ENTRIES.find((e) => e.id === selectedId);

  // header label per view
  const viewLabel = (() => {
    if (selectedView === "all") return { title: "すべての文献", subtitle: "Library" };
    if (selectedView === "starred") return { title: "お気に入り", subtitle: null };
    if (selectedView === "recent") return { title: "最近追加", subtitle: "直近8件" };
    if (selectedView === "unfiled") return { title: "未整理", subtitle: "コレクション未割当" };
    if (selectedView === "trash") return { title: "ゴミ箱", subtitle: null };
    if (selectedView.startsWith("col:")) {
      const id = selectedView.slice(4);
      const find = (cs) => { for (const c of cs) { if (c.id === id) return c; if (c.children) { const x = find(c.children); if (x) return x; } } return null; };
      const c = find(COLLECTIONS);
      return { title: c?.name || "コレクション", subtitle: "コレクション" };
    }
    if (selectedView.startsWith("tag:")) return { title: "#" + selectedView.slice(4), subtitle: "タグ" };
    return { title: "文献", subtitle: null };
  })();

  return (
    <div style={{
      width: "100%", height: "100%",
      background: "var(--bg)", color: "var(--text)",
      display: "flex", overflow: "hidden",
      fontFamily: '"IBM Plex Sans", -apple-system, BlinkMacSystemFont, "Helvetica Neue", system-ui, sans-serif',
    }}>
      {tweaks.showSidebar && (
        <Sidebar
          width={232}
          selectedView={selectedView}
          onSelectView={setSelectedView}
          collections={COLLECTIONS}
          tags={TAGS_USED}
          totalCount={ALL_ENTRIES.length}
        />
      )}

      <main style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0 }}>
        <Toolbar
          title={viewLabel.title}
          subtitle={viewLabel.subtitle}
          count={filteredEntries.length}
          search={search}
          setSearch={setSearch}
          view={view}
          setView={setView}
          onAddOpen={() => setShowAdd(true)}
        />
        <ViewTabs viewMode={viewMode} setViewMode={setViewMode} />
        <div style={{ flex: 1, minHeight: 0, position: "relative", display: "flex", flexDirection: "column" }}>
          {viewMode === "table" && (
            <EntriesTable
              entries={filteredEntries}
              selectedId={selectedId}
              onSelect={setSelectedId}
              sort={sort}
              onSort={onSort}
              density={tweaks.density}
            />
          )}
          {viewMode === "covers" && (
            <CoversGrid
              entries={filteredEntries}
              selectedId={selectedId}
              onSelect={setSelectedId}
            />
          )}
          {viewMode === "timeline" && <PlaceholderView title="タイムラインビュー" body="出版年・追加日軸で並べるビューを今後追加します。" />}
          {viewMode === "graph" && <PlaceholderView title="引用グラフビュー" body="文献間の引用関係をネットワーク表示します。" />}
          {showAdd && <AddSheet onClose={() => setShowAdd(false)} />}
        </div>

        {/* status bar */}
        <div style={{
          flexShrink: 0, borderTop: "1px solid var(--border)",
          background: "var(--surface-2)",
          height: 24, padding: "0 14px",
          display: "flex", alignItems: "center", gap: 14,
          fontSize: 11, color: "var(--text-faint)",
        }}>
          <span style={{ fontVariantNumeric: "tabular-nums" }}>
            {filteredEntries.length} / {ALL_ENTRIES.length} 件
          </span>
          <span style={{ width: 1, height: 10, background: "var(--border)" }} />
          <span>選択中: {selected ? "1 件" : "なし"}</span>
          <div style={{ flex: 1 }} />
          <span>SQLite · {ALL_ENTRIES.length} entries · 28 authors · 14 tags</span>
        </div>
      </main>

      {tweaks.showDetail && <DetailPanel entry={selected} width={320} />}

      <LumenTweaks tweaks={tweaks} setTweak={setTweak} />
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
