// LumenCite — Detail Panel (right pane)

const { useState: useStateD } = React;
const { Icon: IconD, TypeIcon: TypeIconD, tagColor: tagColorD } = window.LumenCommon;
const { TagPill: TagPillD, formatAuthors: fmtAuthD, fmtDate: fmtDateD } = window.LumenTableHelpers;

function Field({ label, value, mono, copy }) {
  if (!value) return null;
  return (
    <div style={{ marginBottom: 12 }}>
      <div style={{
        fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
        textTransform: "uppercase", letterSpacing: "0.06em",
        marginBottom: 3,
      }}>{label}</div>
      <div style={{
        fontSize: 12.5, color: "var(--text)",
        fontFamily: mono ? "var(--mono)" : "inherit",
        wordBreak: "break-word", lineHeight: 1.45,
      }}>{value}</div>
    </div>
  );
}

function Tab({ label, active, onClick }) {
  return (
    <button onClick={onClick} style={{
      flex: 1, padding: "8px 0", border: "none", background: "transparent",
      fontSize: 12, fontWeight: active ? 600 : 500,
      color: active ? "var(--text)" : "var(--text-mute)",
      cursor: "pointer", position: "relative",
      borderBottom: active ? "1.5px solid var(--accent-strong)" : "1.5px solid transparent",
      marginBottom: -1,
      letterSpacing: "0.01em",
    }}>{label}</button>
  );
}

function ActionBtn({ icon, label, primary, onClick }) {
  return (
    <button onClick={onClick} style={{
      display: "inline-flex", alignItems: "center", gap: 5,
      padding: "5px 9px", borderRadius: 5,
      border: primary ? "none" : "1px solid var(--border-strong)",
      background: primary ? "var(--accent-strong)" : "var(--surface)",
      color: primary ? "white" : "var(--text)",
      fontSize: 11.5, fontWeight: 500, cursor: "pointer",
      boxShadow: primary ? "0 1px 0 oklch(0.4 0.12 60 / 0.2)" : "0 1px 0 rgba(0,0,0,0.02)",
    }}>
      {icon && <IconD name={icon} size={11} color={primary ? "white" : "var(--text-mute)"} />}
      {label}
    </button>
  );
}

function DetailPanel({ entry, width }) {
  const [tab, setTab] = useStateD("info");

  if (!entry) {
    return (
      <aside style={panelStyle(width)}>
        <div style={{
          flex: 1, display: "flex", alignItems: "center", justifyContent: "center",
          padding: 24, color: "var(--text-faint)",
          fontSize: 12.5, textAlign: "center", lineHeight: 1.6,
        }}>
          文献を選択すると<br/>詳細が表示されます
        </div>
      </aside>
    );
  }

  return (
    <aside style={panelStyle(width)}>
      {/* hero */}
      <div style={{ padding: "16px 18px 14px", borderBottom: "1px solid var(--border)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
          <span style={{
            display: "inline-flex", alignItems: "center", gap: 5,
            padding: "1px 7px 1px 5px", borderRadius: 4,
            background: "var(--surface-2)", color: "var(--text-mute)",
            fontSize: 10.5, fontWeight: 500, letterSpacing: "0.02em",
          }}>
            <TypeIconD type={entry.type} size={11} />
            {entry.type}
          </span>
          {entry.year && (
            <span style={{
              fontSize: 11, color: "var(--text-faint)",
              fontVariantNumeric: "tabular-nums",
            }}>{entry.year}</span>
          )}
          <div style={{ flex: 1 }} />
          <button style={iconBtnD} title="お気に入り">
            <IconD name={entry.starred ? "starFill" : "star"} size={13}
              color={entry.starred ? "oklch(0.72 0.14 70)" : "var(--text-mute)"} />
          </button>
        </div>
        <h2 style={{
          margin: 0, fontSize: 15.5, fontWeight: 600, lineHeight: 1.32,
          color: "var(--text)", letterSpacing: "-0.012em",
        }}>{entry.title}</h2>
        {entry.authors && (
          <div style={{
            marginTop: 8, fontSize: 12, color: "var(--text-mute)", lineHeight: 1.5,
          }}>{entry.authors.join(", ")}</div>
        )}
        {entry.venue && (
          <div style={{
            marginTop: 6, fontSize: 11.5, color: "var(--text-faint)",
            fontStyle: "italic",
          }}>{entry.venue}</div>
        )}
        {/* actions */}
        <div style={{ display: "flex", gap: 6, marginTop: 12, flexWrap: "wrap" }}>
          {entry.attached && <ActionBtn icon="ext" label="PDFを開く" primary />}
          <ActionBtn icon="sparkle" label="要約" />
          <ActionBtn icon="download" label="BibTeX" />
        </div>
      </div>

      {/* tabs */}
      <div style={{
        display: "flex", borderBottom: "1px solid var(--border)",
        padding: "0 14px", flexShrink: 0,
      }}>
        <Tab label="情報" active={tab === "info"} onClick={() => setTab("info")} />
        <Tab label="抄録" active={tab === "abstract"} onClick={() => setTab("abstract")} />
        <Tab label="ノート" active={tab === "notes"} onClick={() => setTab("notes")} />
        <Tab label="関連" active={tab === "related"} onClick={() => setTab("related")} />
      </div>

      {/* body */}
      <div style={{ flex: 1, overflow: "auto", padding: "16px 18px" }}>
        {tab === "info" && (
          <>
            <Field label="DOI" value={entry.doi} mono />
            <Field label="arXiv" value={entry.arxiv} mono />
            <Field label="ISBN" value={entry.isbn} mono />
            <Field label="URL" value={entry.url} mono />
            <Field label="掲載年" value={entry.year} />
            <Field label="出版" value={entry.venue} />
            <Field label="追加日" value={fmtDateD(entry.added)} />

            <div style={{ marginTop: 4 }}>
              <div style={{
                fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
                textTransform: "uppercase", letterSpacing: "0.06em",
                marginBottom: 6,
              }}>タグ</div>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                {(entry.tags || []).map((t) => <TagPillD key={t} name={t} />)}
                <button style={{
                  display: "inline-flex", alignItems: "center", gap: 3,
                  padding: "1px 7px", borderRadius: 999,
                  border: "1px dashed var(--border-strong)",
                  background: "transparent", color: "var(--text-faint)",
                  fontSize: 10.5, cursor: "pointer",
                }}>
                  <IconD name="plus" size={9} color="var(--text-faint)" />
                  追加
                </button>
              </div>
            </div>

            {entry.collections && entry.collections.length > 0 && (
              <div style={{ marginTop: 14 }}>
                <div style={{
                  fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
                  textTransform: "uppercase", letterSpacing: "0.06em",
                  marginBottom: 6,
                }}>コレクション</div>
                <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
                  {entry.collections.map((c) => (
                    <div key={c} style={{
                      display: "inline-flex", alignItems: "center", gap: 6,
                      fontSize: 12, color: "var(--text)",
                    }}>
                      <IconD name="folder" size={12} color="var(--text-mute)" />
                      {c}
                    </div>
                  ))}
                </div>
              </div>
            )}
          </>
        )}

        {tab === "abstract" && (
          <div style={{ fontSize: 12.5, lineHeight: 1.65, color: "var(--text)" }}>
            {entry.abstract || (
              <span style={{ color: "var(--text-faint)" }}>抄録は登録されていません。</span>
            )}
          </div>
        )}

        {tab === "notes" && (
          <div>
            {entry.notes ? (
              <div style={{
                fontSize: 12.5, lineHeight: 1.65, color: "var(--text)",
                whiteSpace: "pre-wrap",
              }}>{entry.notes}</div>
            ) : (
              <div style={{
                padding: "40px 0", textAlign: "center", color: "var(--text-faint)",
                fontSize: 12,
              }}>
                <div style={{ marginBottom: 8 }}>ノートはまだありません</div>
                <button style={{
                  padding: "5px 11px", borderRadius: 5,
                  border: "1px solid var(--border-strong)",
                  background: "var(--surface)", color: "var(--text)",
                  fontSize: 11.5, cursor: "pointer",
                }}>ノートを作成</button>
              </div>
            )}
          </div>
        )}

        {tab === "related" && (
          <div style={{
            fontSize: 12, color: "var(--text-faint)", lineHeight: 1.6,
          }}>
            arXiv プレプリント版や、引用関係にある文献がここに表示されます。
          </div>
        )}
      </div>
    </aside>
  );
}

function panelStyle(width) {
  return {
    width, flexShrink: 0, height: "100%",
    background: "var(--surface)",
    borderLeft: "1px solid var(--border)",
    display: "flex", flexDirection: "column",
    overflow: "hidden",
  };
}

const iconBtnD = {
  width: 26, height: 26, padding: 0, border: "none", background: "transparent",
  borderRadius: 5, cursor: "pointer", display: "inline-flex",
  alignItems: "center", justifyContent: "center",
};

window.LumenDetail = DetailPanel;
