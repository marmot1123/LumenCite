// LumenCite Chat — Session header, Composer, ScopePicker, NewSessionDialog

const { useState: useStateCC } = React;
const IconCC = window.ChatIcon;
const { SCOPE_LIB_ENTRIES } = window.LUMEN_CHAT;

// ─── SessionHeader ──────────────────────────────────────────────────
function SessionHeader({ session, onScopeOpen, scopeOpen, onToggleRightPanel, rightPanelOpen }) {
  const [editing, setEditing] = useStateCC(false);
  const [title, setTitle] = useStateCC(session.title);

  return (
    <header style={{
      flexShrink: 0,
      borderBottom: "1px solid var(--border)",
      background: "var(--surface)",
      padding: "10px 18px 11px 22px",
      display: "flex", alignItems: "center", gap: 14,
    }}>
      <div style={{ flex: 1, minWidth: 0, display: "flex", alignItems: "center", gap: 10 }}>
        {editing ? (
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            onBlur={() => setEditing(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === "Escape") setEditing(false);
            }}
            autoFocus
            style={{
              fontSize: 14.5, fontWeight: 600, color: "var(--text)",
              letterSpacing: "-0.01em",
              border: "1px solid var(--accent-strong)",
              outline: "none",
              borderRadius: 5, padding: "3px 8px",
              background: "var(--surface)",
              flex: 1, minWidth: 0,
              fontFamily: "inherit",
            }}
          />
        ) : (
          <h1
            onClick={() => setEditing(true)}
            style={{
              margin: 0, fontSize: 14.5, fontWeight: 600, color: "var(--text)",
              letterSpacing: "-0.01em",
              cursor: "text",
              padding: "3px 6px", borderRadius: 5,
              transition: "background 100ms ease",
              whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis",
              maxWidth: 480,
            }}
            onMouseEnter={(e) => e.currentTarget.style.background = "var(--row-hover)"}
            onMouseLeave={(e) => e.currentTarget.style.background = "transparent"}
            title={title}
          >{title}</h1>
        )}

        {/* Scope chip — opens popover */}
        <button
          onClick={onScopeOpen}
          style={{
            display: "inline-flex", alignItems: "center", gap: 5,
            padding: "3px 8px 3px 7px",
            borderRadius: 999, border: "1px solid var(--border-strong)",
            background: scopeOpen ? "var(--accent-soft)" : "var(--surface-2)",
            color: scopeOpen ? "var(--accent-strong)" : "var(--text)",
            fontSize: 11.5, fontWeight: 500, cursor: "pointer",
            transition: "all 100ms ease",
          }}
        >
          <span style={{
            fontSize: 9.5, fontFamily: "var(--mono)",
            color: scopeOpen ? "var(--accent-strong)" : "var(--text-faint)",
            letterSpacing: "0.06em", textTransform: "uppercase",
          }}>scope:</span>
          {session.scope_mode === "all"
            ? <span>ライブラリ全体</span>
            : <span>{session.entry_count} papers</span>}
          <IconCC name="chevronDown" size={9}
            color={scopeOpen ? "var(--accent-strong)" : "var(--text-mute)"} />
        </button>

        {/* Provider/model */}
        <div style={{
          display: "inline-flex", alignItems: "center", gap: 5,
          padding: "2px 7px", borderRadius: 4,
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          fontSize: 10.5, color: "var(--text-mute)",
          fontFamily: "var(--mono)",
        }}>
          <span style={{
            width: 5, height: 5, borderRadius: "50%",
            background: session.provider === "anthropic"
              ? "oklch(0.62 0.14 35)" : "oklch(0.55 0.13 165)",
          }} />
          {session.provider === "anthropic" ? "Claude" : "GPT"}
          <span style={{ opacity: 0.5 }}>·</span>
          {session.model.replace(/^claude-/, "").replace(/^gpt-/, "")}
        </div>
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        <HeaderBtn icon="search" title="このセッション内を検索" />
        <HeaderBtn icon="archive" title="アーカイブ" />
        <HeaderBtn
          icon="panel"
          title="右パネル"
          active={rightPanelOpen}
          onClick={onToggleRightPanel}
        />
        <HeaderBtn icon="more" title="その他" />
      </div>
    </header>
  );
}

function HeaderBtn({ icon, title, active, onClick }) {
  const [hover, setHover] = useStateCC(false);
  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      title={title}
      style={{
        width: 26, height: 26, padding: 0,
        border: "1px solid " + (active ? "var(--border-strong)" : "transparent"),
        background: active ? "var(--surface-2)" : hover ? "var(--hover)" : "transparent",
        borderRadius: 5, cursor: "pointer",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
        color: "var(--text-mute)",
      }}
    >
      <IconCC name={icon} size={12} color={active ? "var(--text)" : "var(--text-mute)"} />
    </button>
  );
}

// ─── ScopePicker (popover) ──────────────────────────────────────────
function ScopePicker({ session, onClose, onSetScope }) {
  const [mode, setMode] = useStateCC(session.scope_mode);
  const [picked, setPicked] = useStateCC(
    new Set(SCOPE_LIB_ENTRIES.filter((e) => e.picked).map((e) => e.id))
  );
  const [q, setQ] = useStateCC("");

  const toggle = (id) => {
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id); else n.add(id);
      return n;
    });
  };

  const filtered = SCOPE_LIB_ENTRIES.filter((e) =>
    !q.trim() || e.full.toLowerCase().includes(q.toLowerCase()) || e.short.toLowerCase().includes(q.toLowerCase())
  );

  return (
    <div
      onClick={onClose}
      style={{
        position: "absolute", inset: 0, zIndex: 30,
        background: "rgba(20, 18, 14, 0.05)",
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          position: "absolute",
          top: 60, left: 280,
          width: 420,
          background: "var(--surface)",
          border: "1px solid var(--border-strong)",
          borderRadius: 9,
          boxShadow: "0 20px 50px rgba(0,0,0,0.18), 0 1px 0 rgba(0,0,0,0.05)",
          overflow: "hidden",
        }}
      >
        <div style={{ padding: "12px 14px 10px", borderBottom: "1px solid var(--border)" }}>
          <div style={{
            fontSize: 12, fontWeight: 600, color: "var(--text)",
          }}>このセッションの検索対象</div>
          <div style={{
            fontSize: 11, color: "var(--text-faint)", marginTop: 2,
          }}>scope を切り替えると以降の質問に反映されます</div>
        </div>

        {/* Mode tabs */}
        <div style={{ display: "flex", padding: "10px 12px 4px", gap: 6 }}>
          <ScopeMode
            label="ライブラリ全体"
            sub="all entries"
            active={mode === "all"}
            onClick={() => setMode("all")}
          />
          <ScopeMode
            label="特定の文献"
            sub={picked.size + " selected"}
            active={mode === "entries"}
            onClick={() => setMode("entries")}
          />
        </div>

        {mode === "entries" && (
          <>
            <div style={{ padding: "8px 12px 4px" }}>
              <div style={{
                display: "flex", alignItems: "center", gap: 6,
                padding: "5px 9px",
                background: "var(--surface-2)",
                border: "1px solid var(--border)",
                borderRadius: 5, height: 26,
              }}>
                <IconCC name="search" size={11} color="var(--text-faint)" />
                <input
                  value={q} onChange={(e) => setQ(e.target.value)}
                  placeholder="ライブラリから検索…"
                  style={{
                    flex: 1, border: "none", outline: "none", background: "transparent",
                    fontSize: 12, color: "var(--text)",
                  }}
                />
              </div>
            </div>
            <div style={{
              maxHeight: 240, overflow: "auto",
              padding: "4px 8px 6px",
            }}>
              {filtered.map((e) => {
                const on = picked.has(e.id);
                return (
                  <div key={e.id}
                    onClick={() => toggle(e.id)}
                    style={{
                      display: "flex", alignItems: "center", gap: 9,
                      padding: "6px 8px", margin: "1px 0",
                      borderRadius: 5, cursor: "pointer",
                      background: on ? "var(--accent-soft)" : "transparent",
                      transition: "background 80ms ease",
                    }}
                    onMouseEnter={(el) => { if (!on) el.currentTarget.style.background = "var(--hover)"; }}
                    onMouseLeave={(el) => { if (!on) el.currentTarget.style.background = "transparent"; }}
                  >
                    <span style={{
                      width: 14, height: 14, borderRadius: 3,
                      border: on ? "none" : "1.2px solid var(--border-strong)",
                      background: on ? "var(--accent-strong)" : "var(--surface)",
                      display: "inline-flex", alignItems: "center", justifyContent: "center",
                      flexShrink: 0,
                    }}>
                      {on && <IconCC name="check" size={9} color="white" strokeWidth={2.4} />}
                    </span>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{
                        fontSize: 12, color: "var(--text)", fontWeight: 500,
                        whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis",
                      }}>{e.full}</div>
                      <div style={{
                        fontSize: 10.5, color: "var(--text-faint)",
                        fontFamily: "var(--mono)",
                      }}>{e.short} · {e.year}</div>
                    </div>
                  </div>
                );
              })}
            </div>
          </>
        )}

        {mode === "all" && (
          <div style={{
            padding: "20px 16px 22px", textAlign: "center",
            color: "var(--text-mute)", fontSize: 12, lineHeight: 1.6,
          }}>
            <IconCC name="library" size={20} color="var(--text-mute)" />
            <div style={{ marginTop: 6 }}>
              <strong>{25}</strong> 件すべての文献を対象にします
            </div>
            <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 3 }}>
              fulltext_search はインデックス全体にかかります
            </div>
          </div>
        )}

        {/* Footer */}
        <div style={{
          padding: "10px 12px", borderTop: "1px solid var(--border)",
          background: "var(--surface-2)",
          display: "flex", justifyContent: "flex-end", gap: 8,
        }}>
          <button onClick={onClose} style={{
            padding: "5px 12px", borderRadius: 5,
            border: "1px solid var(--border-strong)",
            background: "var(--surface)", color: "var(--text)",
            fontSize: 12, cursor: "pointer",
          }}>キャンセル</button>
          <button onClick={() => onSetScope(mode, [...picked])} style={{
            padding: "5px 14px", borderRadius: 5, border: "none",
            background: "var(--accent-strong)", color: "white",
            fontSize: 12, fontWeight: 500, cursor: "pointer",
          }}>適用</button>
        </div>
      </div>
    </div>
  );
}

function ScopeMode({ label, sub, active, onClick }) {
  return (
    <button
      onClick={onClick}
      style={{
        flex: 1, textAlign: "left",
        padding: "8px 10px", borderRadius: 6,
        border: "1px solid " + (active ? "var(--accent-strong)" : "var(--border)"),
        background: active ? "var(--accent-soft)" : "var(--surface)",
        color: active ? "var(--accent-strong)" : "var(--text)",
        cursor: "pointer",
      }}
    >
      <div style={{ fontSize: 12, fontWeight: 600 }}>{label}</div>
      <div style={{
        fontSize: 10.5, marginTop: 2,
        color: active ? "var(--accent-strong)" : "var(--text-faint)",
        fontFamily: "var(--mono)", opacity: 0.85,
      }}>{sub}</div>
    </button>
  );
}

// ─── Composer ───────────────────────────────────────────────────────
function Composer({ session, streaming, blocked, onSend, onStop }) {
  const [v, setV] = useStateCC("");
  const [focused, setFocused] = useStateCC(false);

  const send = () => {
    if (!v.trim() || blocked) return;
    onSend(v);
    setV("");
  };

  return (
    <div style={{
      flexShrink: 0, padding: "10px 40px 16px",
      background: "var(--surface)",
      borderTop: "1px solid var(--border-subtle)",
    }}>
      <div style={{ maxWidth: 820, margin: "0 auto" }}>
        {/* Blocked banner */}
        {blocked && (
          <div style={{
            display: "flex", alignItems: "center", gap: 8,
            padding: "7px 11px 7px 9px",
            marginBottom: 8,
            borderRadius: 6,
            background: "var(--tc-approve-bg)",
            border: "1px solid var(--tc-approve-bd)",
            fontSize: 11.5, color: "var(--tc-approve-fg)",
            lineHeight: 1.5,
          }}>
            <IconCC name="warn" size={12} color="var(--tc-approve-fg)" />
            <span>承認待ちのツール呼び出しがあります。上のカードで <strong>許可</strong> または <strong>拒否</strong> を選んでください。</span>
          </div>
        )}

        <div
          style={{
            display: "flex", flexDirection: "column",
            border: "1px solid " + (focused ? "var(--accent-strong)" : "var(--border-strong)"),
            borderRadius: 10,
            background: blocked ? "var(--surface-2)" : "var(--surface)",
            boxShadow: focused
              ? "0 0 0 3px var(--accent-ring), 0 1px 0 rgba(0,0,0,0.02)"
              : "0 1px 0 rgba(0,0,0,0.03)",
            opacity: blocked ? 0.65 : 1,
            transition: "all 100ms ease",
          }}
        >
          <textarea
            value={v}
            disabled={blocked}
            onChange={(e) => setV(e.target.value)}
            onFocus={() => setFocused(true)}
            onBlur={() => setFocused(false)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) { e.preventDefault(); send(); }
            }}
            placeholder={blocked
              ? "承認待ち — まずツール呼び出しを許可または拒否してください"
              : "Message LumenCite…  ⌘↩ で送信"}
            rows={2}
            style={{
              width: "100%", border: "none", outline: "none",
              background: "transparent", resize: "none",
              padding: "11px 14px 4px",
              fontSize: 13.5, lineHeight: 1.55, color: "var(--text)",
              fontFamily: "inherit",
              minHeight: 56,
            }}
          />
          {/* Composer toolbar */}
          <div style={{
            display: "flex", alignItems: "center", gap: 8,
            padding: "4px 8px 8px 10px",
          }}>
            <ComposerBtn icon="paperclip" title="添付" />
            <ComposerBtn icon="library" title="文献を引用" />
            <div style={{
              fontSize: 10.5, color: "var(--text-faint)",
              display: "flex", alignItems: "center", gap: 6,
              fontFamily: "var(--mono)",
            }}>
              <span style={{
                display: "inline-flex", alignItems: "center", gap: 3,
                color: session.scope_mode === "all"
                  ? "var(--text-faint)" : "var(--accent-strong)",
              }}>
                scope: {session.scope_mode === "all" ? "all" : session.entry_count + " papers"}
              </span>
            </div>
            <div style={{ flex: 1 }} />

            {streaming ? (
              <button
                onClick={onStop}
                style={{
                  display: "inline-flex", alignItems: "center", gap: 6,
                  padding: "5px 12px 5px 10px", borderRadius: 6,
                  border: "1px solid var(--border-strong)",
                  background: "var(--surface)", color: "var(--text)",
                  fontSize: 12, fontWeight: 500, cursor: "pointer",
                }}
              >
                <IconCC name="stop" size={10} color="var(--text)" />
                中断
              </button>
            ) : (
              <button
                onClick={send}
                disabled={blocked || !v.trim()}
                style={{
                  display: "inline-flex", alignItems: "center", gap: 6,
                  padding: "5px 12px 5px 11px", borderRadius: 6,
                  border: "none",
                  background: (blocked || !v.trim())
                    ? "color-mix(in oklch, var(--accent-strong) 50%, var(--text-faint))"
                    : "var(--accent-strong)",
                  color: "white",
                  fontSize: 12, fontWeight: 600,
                  cursor: (blocked || !v.trim()) ? "not-allowed" : "pointer",
                  opacity: (blocked || !v.trim()) ? 0.55 : 1,
                  boxShadow: "0 1px 0 oklch(0.4 0.12 60 / 0.2)",
                  transition: "all 100ms ease",
                }}
              >
                送信
                <IconCC name="enter" size={10} color="white" strokeWidth={2} />
              </button>
            )}
          </div>
        </div>

        {/* Tiny hint line */}
        <div style={{
          marginTop: 6, padding: "0 4px",
          display: "flex", alignItems: "center", gap: 10,
          fontSize: 10.5, color: "var(--text-faint)",
        }}>
          <span>LumenCite はライブラリ内の検索結果を引用します。</span>
          <span style={{ flex: 1 }} />
          <span style={{ fontFamily: "var(--mono)" }}>{v.length} chars</span>
        </div>
      </div>
    </div>
  );
}

function ComposerBtn({ icon, title }) {
  const [hover, setHover] = useStateCC(false);
  return (
    <button
      title={title}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      style={{
        width: 24, height: 24, padding: 0,
        border: "1px solid " + (hover ? "var(--border-strong)" : "transparent"),
        background: hover ? "var(--surface-2)" : "transparent",
        borderRadius: 5, cursor: "pointer",
        display: "inline-flex", alignItems: "center", justifyContent: "center",
      }}
    >
      <IconCC name={icon} size={12} color="var(--text-mute)" />
    </button>
  );
}

// ─── NewSessionDialog ───────────────────────────────────────────────
function NewSessionDialog({ onClose, onCreate }) {
  const [provider, setProvider] = useStateCC("anthropic");
  const [model, setModel] = useStateCC("claude-sonnet-4.5");
  const [scope, setScope] = useStateCC("entries");
  const [picked, setPicked] = useStateCC(new Set([41, 88, 132]));

  const models = {
    anthropic: ["claude-sonnet-4.5", "claude-haiku-4.5", "claude-opus-4"],
    openai: ["gpt-5", "gpt-5-mini", "gpt-4.1"],
  };

  return (
    <div onClick={onClose} style={{
      position: "absolute", inset: 0, zIndex: 40,
      background: "rgba(20, 18, 14, 0.32)",
      backdropFilter: "blur(2px)",
      display: "flex", alignItems: "flex-start", justifyContent: "center",
      paddingTop: 100,
    }}>
      <div onClick={(e) => e.stopPropagation()} style={{
        width: 520, background: "var(--surface)",
        borderRadius: 10, border: "1px solid var(--border-strong)",
        boxShadow: "0 24px 60px rgba(0,0,0,0.22)",
        overflow: "hidden",
      }}>
        <div style={{
          padding: "14px 18px 12px", borderBottom: "1px solid var(--border)",
          display: "flex", alignItems: "center", gap: 10,
        }}>
          <div style={{
            width: 26, height: 26, borderRadius: 6,
            background: "color-mix(in oklch, var(--accent-strong) 14%, var(--surface))",
            display: "inline-flex", alignItems: "center", justifyContent: "center",
            color: "var(--accent-strong)",
          }}>
            <IconCC name="sparkle" size={14} color="var(--accent-strong)" />
          </div>
          <div>
            <div style={{ fontSize: 13.5, fontWeight: 600, color: "var(--text)" }}>新しい Chat を開始</div>
            <div style={{ fontSize: 11, color: "var(--text-faint)", marginTop: 2 }}>
              使用するモデルと初期スコープを選んでください
            </div>
          </div>
        </div>

        <div style={{ padding: "16px 18px 8px" }}>
          <FieldLabel>プロバイダ / モデル</FieldLabel>
          <div style={{ display: "flex", gap: 8 }}>
            <Pill
              icon={<span style={{ width: 7, height: 7, borderRadius: "50%", background: "oklch(0.62 0.14 35)" }} />}
              label="Anthropic" active={provider === "anthropic"}
              onClick={() => { setProvider("anthropic"); setModel("claude-sonnet-4.5"); }}
            />
            <Pill
              icon={<span style={{ width: 7, height: 7, borderRadius: "50%", background: "oklch(0.55 0.13 165)" }} />}
              label="OpenAI" active={provider === "openai"}
              onClick={() => { setProvider("openai"); setModel("gpt-5"); }}
            />
            <div style={{ flex: 1 }} />
            <select
              value={model} onChange={(e) => setModel(e.target.value)}
              style={{
                padding: "5px 10px", borderRadius: 5,
                border: "1px solid var(--border-strong)",
                background: "var(--surface)",
                fontSize: 12, fontFamily: "var(--mono)",
                color: "var(--text)", cursor: "pointer",
              }}
            >
              {models[provider].map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
          </div>
        </div>

        <div style={{ padding: "10px 18px 14px" }}>
          <FieldLabel>初期スコープ</FieldLabel>
          <div style={{ display: "flex", gap: 8 }}>
            <ScopeMode
              label="ライブラリ全体"
              sub="all entries"
              active={scope === "all"}
              onClick={() => setScope("all")}
            />
            <ScopeMode
              label="特定の文献"
              sub={picked.size + " 件選択中"}
              active={scope === "entries"}
              onClick={() => setScope("entries")}
            />
          </div>

          {scope === "entries" && (
            <div style={{
              marginTop: 10, padding: "8px 10px",
              background: "var(--surface-2)",
              border: "1px solid var(--border)",
              borderRadius: 6,
            }}>
              {[...picked].map((id) => {
                const e = SCOPE_LIB_ENTRIES.find((x) => x.id === id);
                if (!e) return null;
                return (
                  <div key={id} style={{
                    display: "flex", alignItems: "center", gap: 8,
                    padding: "3px 0",
                  }}>
                    <IconCC name="book" size={11} color="var(--text-mute)" />
                    <span style={{ fontSize: 11.5, color: "var(--text)", flex: 1 }}>{e.full}</span>
                    <span style={{
                      fontSize: 10, color: "var(--text-faint)",
                      fontFamily: "var(--mono)",
                    }}>{e.year}</span>
                    <button onClick={() => {
                      const n = new Set(picked); n.delete(id); setPicked(n);
                    }} style={{
                      width: 14, height: 14, padding: 0, border: "none",
                      background: "transparent", cursor: "pointer",
                      color: "var(--text-faint)",
                    }}>
                      <IconCC name="x" size={10} color="var(--text-faint)" />
                    </button>
                  </div>
                );
              })}
              <div style={{
                marginTop: 4, paddingTop: 4,
                borderTop: "1px dashed var(--border)",
                fontSize: 11, color: "var(--accent-strong)",
                cursor: "pointer", display: "inline-flex", alignItems: "center", gap: 4,
              }}>
                <IconCC name="plus" size={10} color="var(--accent-strong)" />
                文献を追加
              </div>
            </div>
          )}
        </div>

        <div style={{
          padding: "10px 16px", borderTop: "1px solid var(--border)",
          background: "var(--surface-2)",
          display: "flex", justifyContent: "flex-end", gap: 8,
        }}>
          <button onClick={onClose} style={{
            padding: "6px 14px", borderRadius: 5,
            border: "1px solid var(--border-strong)",
            background: "var(--surface)", color: "var(--text)",
            fontSize: 12, cursor: "pointer",
          }}>キャンセル</button>
          <button onClick={onCreate} style={{
            padding: "6px 16px", borderRadius: 5, border: "none",
            background: "var(--accent-strong)", color: "white",
            fontSize: 12, fontWeight: 600, cursor: "pointer",
          }}>開始</button>
        </div>
      </div>
    </div>
  );
}

function FieldLabel({ children }) {
  return (
    <div style={{
      fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
      textTransform: "uppercase", letterSpacing: "0.06em",
      marginBottom: 6,
    }}>{children}</div>
  );
}

function Pill({ icon, label, active, onClick }) {
  return (
    <button onClick={onClick} style={{
      display: "inline-flex", alignItems: "center", gap: 6,
      padding: "5px 12px", borderRadius: 999,
      border: "1px solid " + (active ? "var(--accent-strong)" : "var(--border-strong)"),
      background: active ? "var(--accent-soft)" : "var(--surface)",
      color: active ? "var(--accent-strong)" : "var(--text)",
      fontSize: 12, fontWeight: 500, cursor: "pointer",
    }}>
      {icon}
      {label}
    </button>
  );
}

window.ChatSessionHeader = SessionHeader;
window.ChatComposer = Composer;
window.ChatScopePicker = ScopePicker;
window.ChatNewSessionDialog = NewSessionDialog;
