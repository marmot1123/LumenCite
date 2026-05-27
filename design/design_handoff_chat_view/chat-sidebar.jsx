// LumenCite Chat — Session list (left pane)

const { useState: useStateCS } = React;
const IconCS = window.ChatIcon;

function ProviderBadge({ provider, model }) {
  // Tiny inline label, e.g. "Claude · sonnet"
  const label =
    provider === "anthropic" ? "Claude" :
    provider === "openai" ? "GPT" : provider;
  const short =
    model.replace(/^claude-/, "").replace(/-?4\.5$/, "").replace(/^gpt-/, "") || model;
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 3,
      fontSize: 10, color: "var(--text-faint)",
      fontFamily: "var(--mono)",
    }}>
      <span style={{
        width: 5, height: 5, borderRadius: "50%",
        background: provider === "anthropic"
          ? "oklch(0.62 0.14 35)"
          : "oklch(0.55 0.13 165)",
        flexShrink: 0,
      }} />
      <span>{label}·{short}</span>
    </span>
  );
}

function ScopeChip({ session, dense }) {
  const isAll = session.scope_mode === "all";
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 3,
      fontSize: 10, padding: dense ? "0 5px" : "1px 6px",
      borderRadius: 3,
      background: isAll ? "var(--surface-2)" : "color-mix(in oklch, var(--accent-strong) 8%, transparent)",
      color: isAll ? "var(--text-mute)" : "var(--accent-strong)",
      border: "1px solid " + (isAll ? "var(--border)" : "color-mix(in oklch, var(--accent-strong) 25%, transparent)"),
      fontWeight: 500,
      fontVariantNumeric: "tabular-nums",
      flexShrink: 0,
    }}>
      {isAll ? "all" : session.entry_count + " papers"}
    </span>
  );
}

function SessionRow({ session, active, dense, onClick }) {
  const [hover, setHover] = useStateCS(false);
  const [menuHover, setMenuHover] = useStateCS(false);
  const padV = dense === "compact" ? 7 : dense === "comfortable" ? 12 : 9;
  return (
    <div
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => { setHover(false); setMenuHover(false); }}
      style={{
        position: "relative",
        padding: padV + "px 12px " + padV + "px 12px",
        margin: "0 6px",
        borderRadius: 6, cursor: "pointer",
        background: active
          ? "color-mix(in oklch, var(--accent-strong) 8%, var(--sidebar))"
          : hover ? "var(--hover)" : "transparent",
        outline: active ? "1px solid color-mix(in oklch, var(--accent-strong) 30%, transparent)" : "1px solid transparent",
        outlineOffset: -1,
        transition: "background 80ms ease",
        display: "flex", flexDirection: "column", gap: 4,
      }}
    >
      {/* Active accent bar */}
      {active && (
        <span style={{
          position: "absolute", left: 0, top: 8, bottom: 8, width: 2,
          borderRadius: 2, background: "var(--accent-strong)",
        }} />
      )}

      <div style={{
        fontSize: 12.5,
        fontWeight: active ? 600 : 500,
        color: active ? "var(--text)" : "var(--text)",
        lineHeight: 1.35,
        letterSpacing: "-0.005em",
        display: "-webkit-box",
        WebkitBoxOrient: "vertical",
        WebkitLineClamp: 2,
        overflow: "hidden",
        paddingRight: hover ? 20 : 0,
        transition: "padding 80ms ease",
      }}>{session.title}</div>

      <div style={{
        display: "flex", alignItems: "center", gap: 6, marginTop: 1,
        minWidth: 0,
      }}>
        <ScopeChip session={session} dense={dense === "compact"} />
        <span style={{
          fontSize: 10.5, color: "var(--text-faint)",
          fontVariantNumeric: "tabular-nums",
          whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis",
        }}>{session.updated}</span>
        <span style={{ flex: 1 }} />
        <span style={{
          fontSize: 10, color: "var(--text-faint)",
          fontFamily: "var(--mono)",
        }}>{session.messages}</span>
      </div>

      {/* Per-row menu (visible on hover) */}
      {hover && (
        <button
          onClick={(e) => { e.stopPropagation(); }}
          onMouseEnter={() => setMenuHover(true)}
          onMouseLeave={() => setMenuHover(false)}
          style={{
            position: "absolute", top: 6, right: 6,
            width: 18, height: 18, padding: 0, border: "none",
            background: menuHover ? "var(--surface)" : "transparent",
            borderRadius: 4, cursor: "pointer",
            display: "inline-flex", alignItems: "center", justifyContent: "center",
            color: "var(--text-mute)",
            boxShadow: menuHover ? "0 0 0 1px var(--border)" : "none",
          }}
          title="セッションメニュー"
        >
          <IconCS name="more" size={11} color="var(--text-mute)" />
        </button>
      )}
    </div>
  );
}

function NewChatButton({ onClick }) {
  const [hover, setHover] = useStateCS(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        display: "flex", alignItems: "center", gap: 8,
        width: "100%", padding: "8px 10px",
        border: "1px solid " + (hover ? "var(--accent-strong)" : "var(--border-strong)"),
        background: hover
          ? "color-mix(in oklch, var(--accent-strong) 6%, var(--surface))"
          : "var(--surface)",
        color: hover ? "var(--accent-strong)" : "var(--text)",
        borderRadius: 6, cursor: "pointer",
        fontSize: 12.5, fontWeight: 500,
        boxShadow: "0 1px 0 rgba(0,0,0,0.02)",
        transition: "all 100ms ease",
      }}
    >
      <span style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 16, height: 16, borderRadius: 4,
        background: "var(--accent-strong)", color: "white",
      }}>
        <IconCS name="plus" size={10} color="white" strokeWidth={2} />
      </span>
      <span style={{ flex: 1, textAlign: "left" }}>新しい Chat</span>
      <span style={{
        fontSize: 10, color: hover ? "var(--accent-strong)" : "var(--text-faint)",
        padding: "1px 5px", border: "1px solid currentColor",
        borderRadius: 3, fontFamily: "var(--mono)", opacity: 0.7,
      }}>⌘ N</span>
    </button>
  );
}

function SidebarSearch() {
  const [v, setV] = useStateCS("");
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 6,
      padding: "5px 9px",
      background: "var(--surface-2)",
      border: "1px solid var(--border)",
      borderRadius: 6, height: 26,
    }}>
      <IconCS name="search" size={11} color="var(--text-faint)" />
      <input
        value={v} onChange={(e) => setV(e.target.value)}
        placeholder="セッションを検索…"
        style={{
          flex: 1, border: "none", outline: "none", background: "transparent",
          fontSize: 12, color: "var(--text)", minWidth: 0,
        }}
      />
    </div>
  );
}

function ChatSidebar({ width, sessions, activeId, onSelect, onNew, onBackToLibrary, density, showEmpty }) {
  const grouped = (() => {
    const today = [], yesterday = [], earlier = [];
    sessions.forEach((s) => {
      const u = s.updated;
      if (/h ago|min/.test(u)) today.push(s);
      else if (u === "yesterday") yesterday.push(s);
      else earlier.push(s);
    });
    return { today, yesterday, earlier };
  })();

  return (
    <aside style={{
      width, flexShrink: 0, height: "100%",
      borderRight: "1px solid var(--border)",
      background: "var(--sidebar)",
      display: "flex", flexDirection: "column",
      overflow: "hidden",
    }}>
      {/* Header — back to Library + brand */}
      <div style={{
        padding: "12px 14px 10px",
        display: "flex", alignItems: "center", gap: 8,
        WebkitAppRegion: "drag",
      }}>
        <button
          onClick={onBackToLibrary}
          style={{
            width: 24, height: 24, padding: 0, border: "1px solid var(--border)",
            background: "var(--surface)", borderRadius: 5, cursor: "pointer",
            display: "inline-flex", alignItems: "center", justifyContent: "center",
            color: "var(--text-mute)",
          }}
          title="ライブラリに戻る"
        >
          <IconCS name="arrowLeft" size={12} color="var(--text-mute)" />
        </button>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            fontSize: 12, fontWeight: 600, color: "var(--text)",
            letterSpacing: "-0.005em",
            display: "flex", alignItems: "center", gap: 6,
          }}>
            <IconCS name="sparkle" size={11} color="var(--accent-strong)" />
            LumenCite Chat
          </div>
          <div style={{
            fontSize: 10, color: "var(--text-faint)", marginTop: 1,
            fontFamily: "var(--mono)",
          }}>{sessions.length} sessions</div>
        </div>
      </div>

      <div style={{ padding: "0 12px 10px" }}>
        <NewChatButton onClick={onNew} />
      </div>

      <div style={{ padding: "0 12px 10px" }}>
        <SidebarSearch />
      </div>

      {/* Sessions */}
      <div style={{ flex: 1, overflow: "auto", paddingBottom: 16 }}>
        {showEmpty ? (
          <EmptyState />
        ) : (
          <>
            {grouped.today.length > 0 && (
              <GroupHeader label="今日" count={grouped.today.length} />
            )}
            {grouped.today.map((s) => (
              <SessionRow key={s.id} session={s} active={s.id === activeId}
                density={density} onClick={() => onSelect(s.id)} />
            ))}
            {grouped.yesterday.length > 0 && (
              <GroupHeader label="昨日" count={grouped.yesterday.length} />
            )}
            {grouped.yesterday.map((s) => (
              <SessionRow key={s.id} session={s} active={s.id === activeId}
                density={density} onClick={() => onSelect(s.id)} />
            ))}
            {grouped.earlier.length > 0 && (
              <GroupHeader label="それ以前" count={grouped.earlier.length} />
            )}
            {grouped.earlier.map((s) => (
              <SessionRow key={s.id} session={s} active={s.id === activeId}
                density={density} onClick={() => onSelect(s.id)} />
            ))}
          </>
        )}
      </div>

      {/* Footer: storage status */}
      <div style={{
        padding: "10px 14px 12px", borderTop: "1px solid var(--border)",
        fontSize: 10.5, color: "var(--text-faint)",
        display: "flex", alignItems: "center", gap: 7,
      }}>
        <span style={{
          width: 5, height: 5, borderRadius: "50%",
          background: "oklch(0.68 0.13 150)",
          boxShadow: "0 0 0 3px oklch(0.68 0.13 150 / 0.18)",
        }} />
        <span>すべてローカル保存</span>
        <span style={{ flex: 1 }} />
        <span style={{ fontFamily: "var(--mono)" }}>chat.db</span>
      </div>
    </aside>
  );
}

function GroupHeader({ label, count }) {
  return (
    <div style={{
      padding: "10px 14px 4px",
      display: "flex", alignItems: "center", gap: 6,
      fontSize: 10, fontWeight: 600, letterSpacing: "0.08em",
      color: "var(--text-faint)", textTransform: "uppercase",
    }}>
      <span>{label}</span>
      <span style={{ fontFamily: "var(--mono)", opacity: 0.7 }}>{count}</span>
    </div>
  );
}

function EmptyState() {
  return (
    <div style={{
      padding: "32px 18px 18px", textAlign: "center",
      color: "var(--text-faint)",
    }}>
      <div style={{
        width: 36, height: 36, borderRadius: 8,
        margin: "0 auto 10px",
        background: "var(--surface-2)",
        border: "1px solid var(--border)",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        color: "var(--accent-strong)",
      }}>
        <IconCS name="sparkle" size={16} color="var(--accent-strong)" />
      </div>
      <div style={{
        fontSize: 12.5, color: "var(--text)", fontWeight: 550,
        marginBottom: 4,
      }}>
        最初の Chat を始めよう
      </div>
      <div style={{ fontSize: 11, lineHeight: 1.55 }}>
        ライブラリ全体や、選んだ文献を<br/>
        対象に LLM と対話できます。
      </div>
    </div>
  );
}

window.ChatSidebar = ChatSidebar;
window.ChatProviderBadge = ProviderBadge;
window.ChatScopeChip = ScopeChip;
