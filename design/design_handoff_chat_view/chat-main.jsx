// LumenCite Chat — Main app

const { useState: useStateMC, useEffect: useEffectMC, useMemo: useMemoMC } = React;
const { CHAT_SESSIONS, CHAT_MESSAGES } = window.LUMEN_CHAT;
const IconMC = window.ChatIcon;
const Sidebar = window.ChatSidebar;
const SessionHeader = window.ChatSessionHeader;
const MessageList = window.ChatMessageList;
const Composer = window.ChatComposer;
const ScopePicker = window.ChatScopePicker;
const NewSessionDialog = window.ChatNewSessionDialog;

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "light",
  "accent": "amber",
  "density": "default",
  "showRightPanel": true,
  "showSidebar": true,
  "showScopePicker": false,
  "showNewSession": false,
  "streaming": false,
  "approvalState": "pending",
  "showEmptySidebar": false
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
    v("--accent-strong", a.strong.replace(/0\.(5\d|6\d)/, "0.74"));
    v("--accent-soft", "oklch(0.36 0.05 65)");
    v("--accent-ring", a.ring);
    // Tool card colors for dark
    v("--tc-read-bg",    "oklch(0.33 0.004 80)");
    v("--tc-read-bd",    "oklch(0.40 0.004 80)");
    v("--tc-read-fg",    "oklch(0.72 0.005 80)");
    v("--tc-write-bg",   "oklch(0.34 0.025 170)");
    v("--tc-write-bd",   "oklch(0.45 0.04 170)");
    v("--tc-write-fg",   "oklch(0.78 0.08 170)");
    v("--tc-approve-bg", "oklch(0.36 0.04 75)");
    v("--tc-approve-bd", "oklch(0.55 0.10 75)");
    v("--tc-approve-fg", "oklch(0.82 0.12 75)");
    v("--tc-delete-bg",  "oklch(0.34 0.04 20)");
    v("--tc-delete-bd",  "oklch(0.50 0.10 20)");
    v("--tc-delete-fg",  "oklch(0.80 0.14 20)");
    v("--tc-mcp-bg",     "oklch(0.33 0.03 285)");
    v("--tc-mcp-bd",     "oklch(0.46 0.06 285)");
    v("--tc-mcp-fg",     "oklch(0.80 0.10 285)");
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
    // Tool card colors for light
    v("--tc-read-bg",    "oklch(0.975 0.004 80)");
    v("--tc-read-bd",    "oklch(0.91 0.005 80)");
    v("--tc-read-fg",    "oklch(0.45 0.01 70)");
    v("--tc-write-bg",   "oklch(0.97 0.025 170)");
    v("--tc-write-bd",   "oklch(0.86 0.04 170)");
    v("--tc-write-fg",   "oklch(0.42 0.08 170)");
    v("--tc-approve-bg", "oklch(0.97 0.06 75)");
    v("--tc-approve-bd", "oklch(0.78 0.13 75)");
    v("--tc-approve-fg", "oklch(0.42 0.12 65)");
    v("--tc-delete-bg",  "oklch(0.97 0.04 20)");
    v("--tc-delete-bd",  "oklch(0.78 0.14 20)");
    v("--tc-delete-fg",  "oklch(0.46 0.16 20)");
    v("--tc-mcp-bg",     "oklch(0.96 0.04 285)");
    v("--tc-mcp-bd",     "oklch(0.82 0.08 285)");
    v("--tc-mcp-fg",     "oklch(0.42 0.13 285)");
  }
  v("--mono", '"IBM Plex Mono", ui-monospace, SFMono-Regular, Menlo, monospace');
}

// ─── Right context panel ────────────────────────────────────────────
function ContextPanel({ session }) {
  return (
    <aside style={{
      width: 280, flexShrink: 0, height: "100%",
      background: "var(--surface)",
      borderLeft: "1px solid var(--border)",
      display: "flex", flexDirection: "column",
      overflow: "hidden",
    }}>
      <div style={{
        padding: "12px 16px 11px",
        borderBottom: "1px solid var(--border)",
        display: "flex", alignItems: "center", gap: 8,
      }}>
        <div style={{
          fontSize: 12, fontWeight: 600, color: "var(--text)",
          display: "flex", alignItems: "center", gap: 6,
        }}>
          コンテキスト
          <span style={{
            fontSize: 10.5, padding: "1px 6px", borderRadius: 999,
            background: "var(--surface-2)",
            color: "var(--text-faint)",
            fontVariantNumeric: "tabular-nums",
          }}>{session.entries?.length || 0}</span>
        </div>
        <div style={{ flex: 1 }} />
        <button style={{
          width: 22, height: 22, padding: 0, border: "1px dashed var(--border-strong)",
          background: "transparent", borderRadius: 5, cursor: "pointer",
          display: "inline-flex", alignItems: "center", justifyContent: "center",
        }}>
          <IconMC name="plus" size={10} color="var(--text-mute)" />
        </button>
      </div>

      <div style={{ flex: 1, overflow: "auto", padding: "10px 12px 14px" }}>
        <SectionLabel>選択中の文献</SectionLabel>
        <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 14 }}>
          {(session.entries || []).map((e, i) => (
            <EntryRow key={e.id} entry={e} index={i + 1} />
          ))}
        </div>

        <SectionLabel>このターンの引用</SectionLabel>
        <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 14 }}>
          <CitationChip num={1} entry="Kempe 2003" loc="p.4 §2.1" />
          <CitationChip num={2} entry="Aharonov 1993" loc="p.2 abstract" />
          <CitationChip num={3} entry="Childs 2010" loc="p.8 §3" />
        </div>

        <SectionLabel>このセッションで使われたツール</SectionLabel>
        <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
          <ToolCount kind="read" count={4} label="fulltext_search · get_entry" />
          <ToolCount kind="write" count={2} label="add_tag · update_notes" />
          <ToolCount kind="approve" count={1} label="update_entry" pending />
          <ToolCount kind="mcp" count={1} label="mcp_obsidian_create_note" pending />
        </div>

        <div style={{
          marginTop: 16, padding: "10px 12px",
          borderRadius: 7,
          background: "var(--surface-2)",
          border: "1px dashed var(--border)",
          fontSize: 11, color: "var(--text-mute)", lineHeight: 1.55,
        }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
            <IconMC name="info" size={11} color="var(--text-mute)" />
            <span style={{ fontWeight: 600, color: "var(--text)" }}>承認ポリシー</span>
          </div>
          読み取り系は自動、<code style={{ fontFamily: "var(--mono)", fontSize: 10.5 }}>create</code> /
          <code style={{ fontFamily: "var(--mono)", fontSize: 10.5 }}> update</code> / MCP write は都度承認、
          <code style={{ fontFamily: "var(--mono)", fontSize: 10.5 }}>delete</code> は常時承認です。
        </div>
      </div>
    </aside>
  );
}

function SectionLabel({ children }) {
  return (
    <div style={{
      fontSize: 9.5, fontWeight: 600, letterSpacing: "0.08em",
      color: "var(--text-faint)", textTransform: "uppercase",
      margin: "2px 4px 6px",
    }}>{children}</div>
  );
}

function EntryRow({ entry, index }) {
  return (
    <div style={{
      padding: "7px 9px", borderRadius: 6,
      background: "var(--surface-2)",
      border: "1px solid var(--border)",
      display: "flex", gap: 8, alignItems: "flex-start",
    }}>
      <div style={{
        width: 18, height: 18, borderRadius: 4,
        background: "var(--accent-soft)",
        color: "var(--accent-strong)",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        fontSize: 10, fontWeight: 600, fontFamily: "var(--mono)",
        flexShrink: 0, marginTop: 1,
      }}>{index}</div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{
          fontSize: 11.5, fontWeight: 550, color: "var(--text)",
          lineHeight: 1.4, letterSpacing: "-0.005em",
        }}>{entry.full}</div>
        <div style={{
          fontSize: 10, color: "var(--text-faint)",
          fontFamily: "var(--mono)", marginTop: 2,
        }}>entry #{entry.id}</div>
      </div>
    </div>
  );
}

function CitationChip({ num, entry, loc }) {
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 8,
      padding: "5px 8px", borderRadius: 5,
      border: "1px solid var(--border)",
      background: "var(--surface)",
      fontSize: 11,
    }}>
      <span style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 18, height: 18, borderRadius: 3,
        background: "var(--accent-soft)",
        color: "var(--accent-strong)",
        fontSize: 10, fontWeight: 600, fontFamily: "var(--mono)",
      }}>{num}</span>
      <span style={{ fontWeight: 550, color: "var(--text)" }}>{entry}</span>
      <span style={{ flex: 1 }} />
      <span style={{
        fontSize: 10, fontFamily: "var(--mono)",
        color: "var(--text-faint)",
      }}>{loc}</span>
    </div>
  );
}

function ToolCount({ kind, count, label, pending }) {
  const colors = {
    read:    "var(--tc-read-fg)",
    write:   "var(--tc-write-fg)",
    approve: "var(--tc-approve-fg)",
    mcp:     "var(--tc-mcp-fg)",
    delete:  "var(--tc-delete-fg)",
  };
  const glyph = {
    read: "search", write: "pencil", approve: "warn", mcp: "plug", delete: "trash",
  };
  return (
    <div style={{
      display: "flex", alignItems: "center", gap: 8,
      padding: "4px 8px", borderRadius: 5,
      fontSize: 11, color: "var(--text)",
    }}>
      <span style={{
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        width: 18, height: 18, borderRadius: 4,
        background: "color-mix(in oklch, " + colors[kind] + " 10%, transparent)",
        color: colors[kind],
      }}>
        <IconMC name={glyph[kind]} size={10} color={colors[kind]} />
      </span>
      <span style={{
        flex: 1, minWidth: 0,
        overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
        fontFamily: "var(--mono)", fontSize: 10.5,
        color: "var(--text-mute)",
      }}>{label}</span>
      <span style={{
        fontSize: 10, fontFamily: "var(--mono)",
        color: pending ? colors[kind] : "var(--text-faint)",
        fontWeight: pending ? 600 : 500,
      }}>
        {pending && "● "}{count}
      </span>
    </div>
  );
}

// ─── Empty conversation state ───────────────────────────────────────
function EmptyConversation() {
  return (
    <div style={{
      flex: 1, display: "flex", flexDirection: "column",
      alignItems: "center", justifyContent: "center",
      gap: 18, padding: 40,
      background: "var(--surface)",
    }}>
      <div style={{
        width: 56, height: 56, borderRadius: 14,
        background: "linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        boxShadow: "0 8px 24px oklch(0.5 0.15 60 / 0.25)",
      }}>
        <IconMC name="sparkle" size={28} color="white" strokeWidth={1.6} />
      </div>
      <div style={{ textAlign: "center" }}>
        <div style={{ fontSize: 17, fontWeight: 600, color: "var(--text)", letterSpacing: "-0.01em" }}>
          ライブラリを LLM に相談する
        </div>
        <div style={{ fontSize: 12.5, color: "var(--text-mute)", marginTop: 4, lineHeight: 1.6 }}>
          検索ツールでライブラリ全文を反復しながら、複数文献を横断して答えます。<br/>
          書き換え操作は承認制で実行されます。
        </div>
      </div>
      <div style={{
        display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8,
        width: "100%", maxWidth: 560,
      }}>
        <SuggestionCard title="この3本の共通点は？"
          sub="選択中の文献を横断要約" />
        <SuggestionCard title="量子ウォークについて教えて"
          sub="ライブラリ全体を検索" />
        <SuggestionCard title="未読のうち重要そうな3本は？"
          sub="メタデータと引用で評価" />
        <SuggestionCard title="この発見をノートに残して"
          sub="Obsidian MCP で書き出し" />
      </div>
    </div>
  );
}

function SuggestionCard({ title, sub }) {
  const [hover, setHover] = useStateMC(false);
  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        padding: "11px 13px", borderRadius: 8, cursor: "pointer",
        border: "1px solid " + (hover ? "var(--accent-strong)" : "var(--border)"),
        background: hover ? "color-mix(in oklch, var(--accent-strong) 6%, var(--surface))" : "var(--surface)",
        transition: "all 100ms ease",
      }}
    >
      <div style={{
        fontSize: 12.5, fontWeight: 550, color: "var(--text)",
        letterSpacing: "-0.005em",
      }}>{title}</div>
      <div style={{
        fontSize: 11, color: "var(--text-faint)", marginTop: 3,
      }}>{sub}</div>
    </div>
  );
}

// ─── Tweaks panel ───────────────────────────────────────────────────
function ChatTweaks({ tweaks, setTweak }) {
  const { TweaksPanel, TweakSection, TweakRadio, TweakToggle, TweakSelect } = window;
  return (
    <TweaksPanel>
      <TweakSection title="外観">
        <TweakRadio label="テーマ" value={tweaks.theme}
          onChange={(v) => setTweak("theme", v)}
          options={[{ value: "light", label: "ライト" }, { value: "dark", label: "ダーク" }]} />
        <TweakSelect label="アクセント" value={tweaks.accent}
          onChange={(v) => setTweak("accent", v)}
          options={[
            { value: "amber", label: "Amber" },
            { value: "indigo", label: "Indigo" },
            { value: "teal", label: "Teal" },
            { value: "rose", label: "Rose" },
          ]} />
        <TweakRadio label="行密度" value={tweaks.density}
          onChange={(v) => setTweak("density", v)}
          options={[
            { value: "compact", label: "高密度" },
            { value: "default", label: "標準" },
            { value: "comfortable", label: "余裕" },
          ]} />
      </TweakSection>

      <TweakSection title="レイアウト">
        <TweakToggle label="セッション一覧" value={tweaks.showSidebar}
          onChange={(v) => setTweak("showSidebar", v)} />
        <TweakToggle label="右パネル（コンテキスト）" value={tweaks.showRightPanel}
          onChange={(v) => setTweak("showRightPanel", v)} />
      </TweakSection>

      <TweakSection title="状態の確認">
        <TweakSelect label="承認カード" value={tweaks.approvalState}
          onChange={(v) => setTweak("approvalState", v)}
          options={[
            { value: "pending", label: "承認待ち（既定）" },
            { value: "approved", label: "承認後の状態" },
            { value: "rejected", label: "拒否後の状態" },
          ]} />
        <TweakToggle label="ストリーミング中" value={tweaks.streaming}
          onChange={(v) => setTweak("streaming", v)} />
        <TweakToggle label="ScopePicker を開く" value={tweaks.showScopePicker}
          onChange={(v) => setTweak("showScopePicker", v)} />
        <TweakToggle label="新規セッション Dialog" value={tweaks.showNewSession}
          onChange={(v) => setTweak("showNewSession", v)} />
        <TweakToggle label="セッション一覧の空状態" value={tweaks.showEmptySidebar}
          onChange={(v) => setTweak("showEmptySidebar", v)} />
      </TweakSection>
    </TweaksPanel>
  );
}

// ─── Resolve messages based on approval state tweak ─────────────────
function resolveMessages(approvalState) {
  // Returns a possibly-mutated copy of CHAT_MESSAGES so the last
  // assistant tool calls reflect the requested state.
  const out = CHAT_MESSAGES.map((m) => ({ ...m, md: m.md ? [...m.md] : undefined }));
  const last = out[out.length - 1];
  if (!last || !last.md) return { messages: out, blocking: false };

  if (approvalState === "pending") {
    return { messages: out, blocking: true };
  }

  // Walk through and transform tool blocks
  const newMd = last.md.map((b) => {
    if (b.kind !== "tools") return b;
    const newCalls = b.calls.map((c) => {
      if (c.state !== "needs_approval") return c;
      if (approvalState === "approved") {
        return {
          ...c,
          state: "done_collapsed",
          kind: c.kind === "approve" ? "write" : c.kind,
          summary: c.kind === "mcp"
            ? "Obsidian: ノート作成完了 (1 ファイル)"
            : "タイトル更新を実行しました",
        };
      } else {
        return { ...c, state: "rejected" };
      }
    });
    return { ...b, calls: newCalls };
  });
  last.md = newMd;

  if (approvalState === "approved") {
    last.md = [
      ...newMd,
      { kind: "p", html: "承認された操作を反映しました。Childs 2010 のタイトルが更新され、Obsidian の <code>研究メモ/2026-05-26 量子ウォーク3本まとめ.md</code> が作成されています。" },
    ];
  } else {
    last.md = [
      ...newMd,
      { kind: "p", html: "了解しました。<span style=\"color: var(--text-mute)\">提案は取り下げます。</span>他の方針があれば指示してください。" },
    ];
  }

  return { messages: out, blocking: false };
}

// ─── App ────────────────────────────────────────────────────────────
function App() {
  const useTweaks = window.useTweaks;
  const [tweaks, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [activeId, setActiveId] = useStateMC(1);
  const [localScopeOpen, setLocalScopeOpen] = useStateMC(false);
  const [localNewOpen, setLocalNewOpen] = useStateMC(false);

  useEffectMC(() => { applyTheme(tweaks.theme, tweaks.accent); }, [tweaks.theme, tweaks.accent]);

  const session = CHAT_SESSIONS.find((s) => s.id === activeId) || CHAT_SESSIONS[0];

  const { messages, blocking } = useMemoMC(
    () => resolveMessages(tweaks.approvalState),
    [tweaks.approvalState, activeId]
  );

  const scopeOpen = tweaks.showScopePicker || localScopeOpen;
  const newOpen = tweaks.showNewSession || localNewOpen;
  const empty = tweaks.showEmptySidebar;

  return (
    <div style={{
      width: "100%", height: "100%",
      background: "var(--bg)", color: "var(--text)",
      display: "flex", overflow: "hidden",
      fontFamily: '"IBM Plex Sans", "Noto Sans JP", -apple-system, BlinkMacSystemFont, system-ui, sans-serif',
    }}>
      {tweaks.showSidebar && (
        <Sidebar
          width={244}
          sessions={empty ? [] : CHAT_SESSIONS}
          activeId={activeId}
          onSelect={setActiveId}
          onNew={() => setLocalNewOpen(true)}
          onBackToLibrary={() => {}}
          density={tweaks.density}
          showEmpty={empty}
        />
      )}

      <main style={{
        flex: 1, display: "flex", flexDirection: "column",
        minWidth: 0, position: "relative",
      }}>
        {empty || !session ? (
          <EmptyConversation />
        ) : (
          <>
            <SessionHeader
              session={session}
              scopeOpen={scopeOpen}
              onScopeOpen={() => setLocalScopeOpen(!localScopeOpen)}
              rightPanelOpen={tweaks.showRightPanel}
              onToggleRightPanel={() => setTweak("showRightPanel", !tweaks.showRightPanel)}
            />
            <MessageList
              messages={messages}
              streaming={tweaks.streaming}
              onApprove={() => setTweak("approvalState", "approved")}
              onReject={() => setTweak("approvalState", "rejected")}
            />
            <Composer
              session={session}
              streaming={tweaks.streaming}
              blocked={blocking}
              onSend={() => {}}
              onStop={() => setTweak("streaming", false)}
            />
          </>
        )}

        {scopeOpen && (
          <ScopePicker
            session={session}
            onClose={() => { setLocalScopeOpen(false); setTweak("showScopePicker", false); }}
            onSetScope={() => { setLocalScopeOpen(false); setTweak("showScopePicker", false); }}
          />
        )}

        {newOpen && (
          <NewSessionDialog
            onClose={() => { setLocalNewOpen(false); setTweak("showNewSession", false); }}
            onCreate={() => { setLocalNewOpen(false); setTweak("showNewSession", false); }}
          />
        )}
      </main>

      {tweaks.showRightPanel && !empty && (
        <ContextPanel session={session} />
      )}

      <ChatTweaks tweaks={tweaks} setTweak={setTweak} />
    </div>
  );
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
