// LumenCite Chat — MessageList + ToolCallCard

const { useState: useStateCM } = React;
const IconCM = window.ChatIcon;

// ─── ToolCallCard ───────────────────────────────────────────────────
// Five kinds × multiple states. All share the same outer chrome,
// but kind drives color and icon.

const TOOL_KINDS = {
  read: {
    color: "var(--tc-read-fg)",
    bg:    "var(--tc-read-bg)",
    bd:    "var(--tc-read-bd)",
    label: "read",
    iconBg: "transparent",
    glyph: "search",
  },
  write: {
    color: "var(--tc-write-fg)",
    bg:    "var(--tc-write-bg)",
    bd:    "var(--tc-write-bd)",
    label: "write",
    iconBg: "color-mix(in oklch, var(--tc-write-fg) 12%, transparent)",
    glyph: "pencil",
  },
  approve: {
    color: "var(--tc-approve-fg)",
    bg:    "var(--tc-approve-bg)",
    bd:    "var(--tc-approve-bd)",
    label: "approval required",
    iconBg: "color-mix(in oklch, var(--tc-approve-fg) 15%, transparent)",
    glyph: "warn",
  },
  delete: {
    color: "var(--tc-delete-fg)",
    bg:    "var(--tc-delete-bg)",
    bd:    "var(--tc-delete-bd)",
    label: "destructive",
    iconBg: "color-mix(in oklch, var(--tc-delete-fg) 12%, transparent)",
    glyph: "trash",
  },
  mcp: {
    color: "var(--tc-mcp-fg)",
    bg:    "var(--tc-mcp-bg)",
    bd:    "var(--tc-mcp-bd)",
    label: "mcp",
    iconBg: "color-mix(in oklch, var(--tc-mcp-fg) 12%, transparent)",
    glyph: "plug",
  },
};

function ToolKindBadge({ kind, server }) {
  const k = TOOL_KINDS[kind];
  return (
    <span style={{
      display: "inline-flex", alignItems: "center", gap: 4,
      fontSize: 9.5, fontWeight: 600, letterSpacing: "0.06em",
      textTransform: "uppercase",
      color: k.color, padding: "1px 6px",
      borderRadius: 3,
      background: k.iconBg,
      border: "1px solid color-mix(in oklch, " + k.color + " 25%, transparent)",
      fontFamily: "var(--mono)",
    }}>
      {kind === "mcp" && server && (
        <span style={{ opacity: 0.7 }}>{server}</span>
      )}
      {kind !== "mcp" && k.label}
      {kind === "mcp" && <span>mcp</span>}
    </span>
  );
}

function JsonPreview({ value, color }) {
  // Compact, syntax-tinted JSON for the args panel.
  const render = (v, depth = 0) => {
    if (v === null) return <span style={{ color: "var(--text-faint)" }}>null</span>;
    if (typeof v === "string") return <span style={{ color: "oklch(0.42 0.10 145)" }}>"{v}"</span>;
    if (typeof v === "number" || typeof v === "boolean")
      return <span style={{ color: "oklch(0.5 0.13 270)" }}>{String(v)}</span>;
    if (Array.isArray(v)) return (
      <>
        <span>[</span>
        {v.map((x, i) => (
          <React.Fragment key={i}>
            {render(x, depth + 1)}
            {i < v.length - 1 ? <span>, </span> : null}
          </React.Fragment>
        ))}
        <span>]</span>
      </>
    );
    if (typeof v === "object") {
      const keys = Object.keys(v);
      return (
        <>
          <span>{"{"}</span>
          <div style={{ paddingLeft: 12 }}>
            {keys.map((k, i) => (
              <div key={k}>
                <span style={{ color: "oklch(0.48 0.04 30)" }}>{k}</span>
                <span style={{ color: "var(--text-faint)" }}>: </span>
                {render(v[k], depth + 1)}
                {i < keys.length - 1 ? <span>,</span> : null}
              </div>
            ))}
          </div>
          <span>{"}"}</span>
        </>
      );
    }
    return <span>{String(v)}</span>;
  };

  return (
    <div style={{
      fontFamily: "var(--mono)", fontSize: 11, lineHeight: 1.55,
      color: "var(--text)",
      padding: "8px 10px",
      background: "var(--surface)",
      border: "1px solid var(--border)",
      borderRadius: 5,
      overflowX: "auto", whiteSpace: "pre-wrap",
    }}>
      {render(value)}
    </div>
  );
}

function HitSnippet({ hit }) {
  return (
    <div style={{
      padding: "7px 10px", borderRadius: 5,
      background: "var(--surface)",
      border: "1px solid var(--border)",
      display: "flex", flexDirection: "column", gap: 3,
    }}>
      <div style={{
        display: "flex", alignItems: "center", gap: 8,
        fontSize: 10.5, color: "var(--text-mute)",
      }}>
        <span style={{
          fontWeight: 600, color: "var(--text)",
          fontFamily: "var(--mono)",
        }}>{hit.entry}</span>
        <span style={{ color: "var(--text-faint)" }}>·</span>
        <span style={{ fontFamily: "var(--mono)", color: "var(--text-faint)" }}>p.{hit.page}</span>
      </div>
      <div
        style={{ fontSize: 11.5, lineHeight: 1.5, color: "var(--text)" }}
        dangerouslySetInnerHTML={{ __html: hit.snippet }}
      />
    </div>
  );
}

function DiffBlock({ diff }) {
  return (
    <div style={{
      fontFamily: "var(--mono)", fontSize: 11.5, lineHeight: 1.55,
      borderRadius: 5, overflow: "hidden",
      border: "1px solid var(--border)",
    }}>
      {diff.map((d, i) => (
        <div key={i} style={{
          display: "flex", alignItems: "flex-start", gap: 8,
          padding: "5px 10px",
          background: d.op === "-"
            ? "color-mix(in oklch, var(--tc-delete-fg) 8%, var(--surface))"
            : "color-mix(in oklch, oklch(0.55 0.13 145) 9%, var(--surface))",
          borderTop: i > 0 ? "1px solid var(--border-subtle)" : "none",
        }}>
          <span style={{
            color: d.op === "-" ? "var(--tc-delete-fg)" : "oklch(0.42 0.12 145)",
            fontWeight: 600, width: 10, flexShrink: 0,
          }}>{d.op}</span>
          <span style={{ color: "var(--text)" }}>{d.text}</span>
        </div>
      ))}
    </div>
  );
}

function ToolCallCard({ call, onApprove, onReject }) {
  const initialOpen = call.state === "done_expanded" || call.state === "needs_approval";
  const [open, setOpen] = useStateCM(initialOpen);
  const k = TOOL_KINDS[call.kind];

  const pending = call.state === "needs_approval";
  const rejected = call.state === "rejected";
  const running = call.state === "running";

  // Tinted background only for the "louder" states
  const louder = pending || call.kind === "delete";

  return (
    <div
      className={pending ? "pulse-approve" : ""}
      style={{
        borderRadius: 7,
        border: "1px solid " + (louder ? k.bd : "var(--border)"),
        background: louder ? k.bg : "var(--surface)",
        boxShadow: pending
          ? "0 1px 0 oklch(0 0 0 / 0.02)"
          : "0 1px 0 oklch(0 0 0 / 0.02)",
        overflow: "hidden",
        opacity: rejected ? 0.7 : 1,
        transition: "background 120ms ease, opacity 120ms ease",
      }}
    >
      {/* Summary header */}
      <button
        onClick={() => !pending && setOpen(!open)}
        style={{
          width: "100%", border: "none", background: "transparent",
          padding: "8px 10px 8px 10px",
          display: "flex", alignItems: "center", gap: 9,
          cursor: pending ? "default" : "pointer",
          textAlign: "left", color: "var(--text)",
        }}
      >
        {/* Glyph */}
        <span style={{
          display: "inline-flex", alignItems: "center", justifyContent: "center",
          width: 22, height: 22, borderRadius: 5,
          background: louder ? "var(--surface)" : k.iconBg,
          color: k.color,
          flexShrink: 0,
          border: louder ? "1px solid " + k.bd : "none",
        }}>
          <IconCM name={k.glyph} size={12} color={k.color} strokeWidth={1.6} />
        </span>

        {/* Tool name + args preview */}
        <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", gap: 1 }}>
          <div style={{
            display: "flex", alignItems: "center", gap: 6,
            fontSize: 11.5, fontFamily: "var(--mono)",
            color: "var(--text)", fontWeight: 500,
            minWidth: 0,
          }}>
            <span style={{
              color: k.color, fontWeight: 600,
            }}>{call.tool}</span>
            <span style={{ color: "var(--text-faint)" }}>(</span>
            <span style={{
              color: "var(--text-mute)", overflow: "hidden",
              textOverflow: "ellipsis", whiteSpace: "nowrap",
              minWidth: 0, flex: 1,
            }}>{formatArgs(call.args)}</span>
            <span style={{ color: "var(--text-faint)" }}>)</span>
          </div>
          <div style={{
            fontSize: 11, color: "var(--text-mute)",
            display: "flex", alignItems: "center", gap: 6,
          }}>
            {running && <span className="spinner" style={{ color: k.color }} />}
            {running ? "実行中…" : (
              rejected ? <span style={{ color: "var(--text-faint)" }}>拒否済み</span>
              : pending ? <span style={{ color: k.color, fontWeight: 500 }}>承認待ち</span>
              : call.summary
            )}
          </div>
        </div>

        {/* Right side: kind badge + chevron */}
        <ToolKindBadge kind={call.kind} server={call.server} />
        {!pending && !running && (
          <span style={{
            display: "inline-flex", color: "var(--text-faint)",
            transform: open ? "rotate(90deg)" : "rotate(0deg)",
            transition: "transform 120ms ease",
            marginLeft: 2,
          }}>
            <IconCM name="chevronRight" size={11} color="var(--text-faint)" />
          </span>
        )}
      </button>

      {/* Expanded body */}
      {open && !running && (
        <div style={{
          borderTop: "1px solid " + (louder ? k.bd : "var(--border)"),
          padding: "10px 12px 12px",
          background: louder ? "color-mix(in oklch, var(--surface) 60%, " + k.bg + ")" : "var(--surface-2)",
          display: "flex", flexDirection: "column", gap: 9,
        }}>
          {/* arguments */}
          <div>
            <div style={{
              fontSize: 9.5, fontWeight: 600, letterSpacing: "0.08em",
              color: "var(--text-faint)", textTransform: "uppercase",
              marginBottom: 4,
            }}>arguments</div>
            <JsonPreview value={call.args} />
          </div>

          {/* result preview, kind-specific */}
          {call.hits && (
            <div>
              <div style={{
                fontSize: 9.5, fontWeight: 600, letterSpacing: "0.08em",
                color: "var(--text-faint)", textTransform: "uppercase",
                marginBottom: 4,
              }}>hits ({call.hits.length})</div>
              <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                {call.hits.map((h, i) => <HitSnippet key={i} hit={h} />)}
              </div>
            </div>
          )}

          {call.diff && (
            <div>
              <div style={{
                fontSize: 9.5, fontWeight: 600, letterSpacing: "0.08em",
                color: "var(--text-faint)", textTransform: "uppercase",
                marginBottom: 4,
              }}>diff</div>
              <DiffBlock diff={call.diff} />
            </div>
          )}

          {/* Approval bar */}
          {pending && (
            <div style={{
              marginTop: 2,
              display: "flex", alignItems: "center", gap: 8,
              padding: "8px 10px", borderRadius: 6,
              background: "color-mix(in oklch, " + k.color + " 12%, var(--surface))",
              border: "1px solid " + k.bd,
            }}>
              <IconCM name="warn" size={13} color={k.color} />
              <span style={{
                flex: 1, fontSize: 11.5, color: "var(--text)", lineHeight: 1.4,
              }}>
                このツール呼び出しは <strong>承認が必要</strong>です。承認するまで会話は進みません。
              </span>
              <button
                onClick={onReject}
                style={{
                  padding: "5px 11px", borderRadius: 5,
                  border: "1px solid var(--border-strong)",
                  background: "var(--surface)", color: "var(--text)",
                  fontSize: 11.5, fontWeight: 500, cursor: "pointer",
                }}
              >拒否</button>
              <button
                onClick={onApprove}
                style={{
                  display: "inline-flex", alignItems: "center", gap: 5,
                  padding: "5px 13px", borderRadius: 5,
                  border: "none",
                  background: k.color, color: "white",
                  fontSize: 11.5, fontWeight: 600, cursor: "pointer",
                  boxShadow: "0 1px 0 oklch(0 0 0 / 0.1)",
                }}
              >
                <IconCM name="check" size={11} color="white" />
                許可
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function formatArgs(args) {
  // Single-line preview for the collapsed header
  const entries = Object.entries(args || {});
  return entries.map(([k, v]) => {
    let val;
    if (typeof v === "string") val = '"' + (v.length > 36 ? v.slice(0, 33) + "…" : v) + '"';
    else if (typeof v === "object" && v !== null) val = Array.isArray(v) ? "[…]" : "{…}";
    else val = String(v);
    return k + "=" + val;
  }).join(", ");
}

// ─── Markdown-ish renderer ──────────────────────────────────────────
// Renders the pre-tokenized "md" array on assistant messages.
function MdBody({ md, streaming, onApprove, onReject }) {
  return (
    <div className="md">
      {md.map((b, i) => {
        if (b.kind === "p") return (
          <p key={i} dangerouslySetInnerHTML={{ __html: b.html }} />
        );
        if (b.kind === "h") return <h3 key={i}>{b.text}</h3>;
        if (b.kind === "ul") return (
          <ul key={i}>
            {b.items.map((it, j) => (
              <li key={j} dangerouslySetInnerHTML={{ __html: it }} />
            ))}
          </ul>
        );
        if (b.kind === "math_display") return (
          <div key={i} className="math-display">
            <span>{b.render}</span>
            {b.tag && <span className="tag">{b.tag}</span>}
          </div>
        );
        if (b.kind === "tools") return (
          <div key={i} style={{
            display: "flex", flexDirection: "column", gap: 6,
            margin: "10px 0 14px",
          }}>
            {b.calls.map((c, j) => (
              <ToolCallCard key={j} call={c} onApprove={onApprove} onReject={onReject} />
            ))}
          </div>
        );
        return null;
      })}
      {streaming && <span className="caret" />}
    </div>
  );
}

// ─── Message bubbles ────────────────────────────────────────────────
function UserMessage({ content }) {
  return (
    <div style={{
      display: "flex", justifyContent: "flex-end", marginBottom: 18,
    }}>
      <div style={{
        maxWidth: "76%",
        padding: "10px 14px",
        borderRadius: 12,
        borderTopRightRadius: 4,
        background: "color-mix(in oklch, var(--accent-strong) 9%, var(--surface))",
        border: "1px solid color-mix(in oklch, var(--accent-strong) 22%, transparent)",
        color: "var(--text)",
        fontSize: 13.5, lineHeight: 1.6,
        boxShadow: "0 1px 0 oklch(0 0 0 / 0.02)",
        whiteSpace: "pre-wrap", textWrap: "pretty",
      }}>{content}</div>
    </div>
  );
}

function AssistantMessage({ message, streaming, onApprove, onReject }) {
  return (
    <div style={{
      display: "flex", gap: 12, marginBottom: 22,
      alignItems: "flex-start",
    }}>
      <div style={{
        flexShrink: 0,
        width: 26, height: 26, borderRadius: 7,
        background: "linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))",
        display: "flex", alignItems: "center", justifyContent: "center",
        boxShadow: "0 1px 2px rgba(120,80,20,0.20), inset 0 0.5px 0 rgba(255,255,255,0.5)",
        marginTop: 1,
      }}>
        <svg width="14" height="14" viewBox="0 0 14 14" fill="none">
          <path d="M3.5 2.5v6.5a3.5 3.5 0 0 0 7 0V2.5" stroke="white" strokeWidth="1.5" strokeLinecap="round"/>
          <circle cx="7" cy="11" r="1.1" fill="white"/>
        </svg>
      </div>
      <div style={{ flex: 1, minWidth: 0, paddingTop: 1 }}>
        <MdBody md={message.md} streaming={streaming}
          onApprove={onApprove} onReject={onReject} />
      </div>
    </div>
  );
}

// ─── MessageList ────────────────────────────────────────────────────
function MessageList({ messages, streaming, onApprove, onReject }) {
  return (
    <div style={{
      flex: 1, overflow: "auto",
      padding: "20px 40px 24px",
      background: "var(--surface)",
    }}>
      <div style={{ maxWidth: 820, margin: "0 auto" }}>
        {messages.map((m, idx) => {
          const isLast = idx === messages.length - 1;
          if (m.role === "user") return <UserMessage key={m.id} content={m.content} />;
          return (
            <AssistantMessage key={m.id} message={m}
              streaming={isLast && streaming}
              onApprove={onApprove} onReject={onReject} />
          );
        })}
      </div>
    </div>
  );
}

window.ChatMessageList = MessageList;
window.ChatToolCallCard = ToolCallCard;
