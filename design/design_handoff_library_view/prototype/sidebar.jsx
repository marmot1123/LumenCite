// LumenCite — Sidebar (Collections, Tags, Library)

const { useState: useStateSB } = React;
const { Icon: IconSB, tagColor: tagColorSB } = window.LumenCommon;

function SidebarSection({ title, children, action }) {
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

function NavRow({ icon, label, count, active, onClick, indent = 0, expandable, expanded, onToggle, accent }) {
  const [hover, setHover] = useStateSB(false);
  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "flex", alignItems: "center", gap: 8,
        padding: "5px 14px 5px " + (10 + indent * 14) + "px",
        margin: "0 6px", borderRadius: 6, cursor: "pointer",
        background: active ? "var(--accent-soft)" : hover ? "var(--hover)" : "transparent",
        color: active ? "var(--accent-strong)" : "var(--text)",
        fontSize: 13, fontWeight: active ? 550 : 450,
        transition: "background 80ms ease",
        position: "relative",
      }}
    >
      {expandable ? (
        <span
          onClick={(e) => { e.stopPropagation(); onToggle && onToggle(); }}
          style={{
            display: "inline-flex", alignItems: "center", justifyContent: "center",
            width: 12, height: 12, marginLeft: -4, color: "var(--text-mute)",
            transform: expanded ? "rotate(90deg)" : "rotate(0)",
            transition: "transform 100ms ease",
          }}
        >
          <IconSB name="chevronRight" size={10} />
        </span>
      ) : <span style={{ width: 8 }} />}
      <span style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        color: active ? "var(--accent-strong)" : accent || "var(--text-mute)",
        flexShrink: 0,
      }}>
        {typeof icon === "string" ? <IconSB name={icon} size={14} /> : icon}
      </span>
      <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {label}
      </span>
      {count != null && (
        <span style={{
          fontSize: 11, color: active ? "var(--accent-strong)" : "var(--text-faint)",
          fontVariantNumeric: "tabular-nums", opacity: active ? 0.85 : 0.85,
        }}>{count}</span>
      )}
    </div>
  );
}

function TagDot({ color }) {
  return <span style={{
    width: 8, height: 8, borderRadius: "50%",
    background: tagColorSB(color)?.dot || "var(--text-faint)",
    display: "inline-block",
  }} />;
}

function Sidebar({ width, selectedView, onSelectView, collections, tags, totalCount }) {
  const [expanded, setExpanded] = useStateSB({ c1: true });
  const toggle = (id) => setExpanded((e) => ({ ...e, [id]: !e[id] }));

  const renderCol = (col, depth = 0) => {
    const has = col.children && col.children.length;
    const open = expanded[col.id];
    return (
      <React.Fragment key={col.id}>
        <NavRow
          icon="folder" label={col.name} count={col.count}
          active={selectedView === "col:" + col.id}
          onClick={() => onSelectView("col:" + col.id)}
          indent={depth}
          expandable={has} expanded={open}
          onToggle={() => toggle(col.id)}
        />
        {has && open && col.children.map((c) => renderCol(c, depth + 1))}
      </React.Fragment>
    );
  };

  return (
    <aside style={{
      width, flexShrink: 0, height: "100%",
      borderRight: "1px solid var(--border)",
      background: "var(--sidebar)",
      display: "flex", flexDirection: "column",
      overflow: "hidden",
    }}>
      {/* Brand header */}
      <div style={{
        padding: "14px 18px 16px",
        display: "flex", alignItems: "center", gap: 10,
        WebkitAppRegion: "drag",
      }}>
        <div style={{
          width: 22, height: 22, borderRadius: 6,
          background: "linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))",
          display: "flex", alignItems: "center", justifyContent: "center",
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
          <div style={{ fontSize: 10.5, color: "var(--text-faint)", marginTop: 1 }}>
            研究ライブラリ
          </div>
        </div>
        <button style={iconBtn}>
          <IconSB name="sync" size={13} color="var(--text-mute)" />
        </button>
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: "4px 0 16px" }}>
        <SidebarSection title="ライブラリ">
          <NavRow icon="library" label="すべての文献" count={totalCount}
            active={selectedView === "all"} onClick={() => onSelectView("all")} />
          <NavRow icon="clock" label="最近追加" count={8}
            active={selectedView === "recent"} onClick={() => onSelectView("recent")} />
          <NavRow icon={<IconSB name="starFill" size={12} color="oklch(0.7 0.13 70)" />}
            label="お気に入り" count={ENTRIES.filter(e => e.starred).length}
            active={selectedView === "starred"} onClick={() => onSelectView("starred")} />
          <NavRow icon="inbox" label="未整理" count={ENTRIES.filter(e => !e.collections || e.collections.length === 0).length}
            active={selectedView === "unfiled"} onClick={() => onSelectView("unfiled")} />
          <NavRow icon="trash" label="ゴミ箱" count={2}
            active={selectedView === "trash"} onClick={() => onSelectView("trash")} />
        </SidebarSection>

        <SidebarSection title="コレクション" action={
          <button style={miniBtn} title="新規コレクション">
            <IconSB name="plus" size={11} color="var(--text-mute)" />
          </button>
        }>
          {collections.map((c) => renderCol(c))}
        </SidebarSection>

        <SidebarSection title="タグ">
          {tags.slice(0, 7).map((t) => (
            <NavRow
              key={t.name}
              icon={<TagDot color={t.color} />}
              label={t.name} count={t.count}
              active={selectedView === "tag:" + t.name}
              onClick={() => onSelectView("tag:" + t.name)}
            />
          ))}
        </SidebarSection>
      </div>

      {/* Bottom sync status */}
      <div style={{
        padding: "10px 18px 12px", borderTop: "1px solid var(--border)",
        fontSize: 11, color: "var(--text-faint)",
        display: "flex", alignItems: "center", gap: 7,
      }}>
        <span style={{
          width: 6, height: 6, borderRadius: "50%",
          background: "oklch(0.68 0.13 150)",
          boxShadow: "0 0 0 3px oklch(0.68 0.13 150 / 0.18)",
        }} />
        <span>references.bib と同期中</span>
      </div>
    </aside>
  );
}

const iconBtn = {
  width: 24, height: 24, padding: 0, border: "none", background: "transparent",
  borderRadius: 5, cursor: "pointer", display: "inline-flex",
  alignItems: "center", justifyContent: "center",
};
const miniBtn = { ...iconBtn, width: 18, height: 18 };

window.LumenSidebar = Sidebar;
