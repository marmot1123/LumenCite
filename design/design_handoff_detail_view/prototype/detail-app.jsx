// LumenCite — Detail screen with PDF viewer
const { useState, useRef, useEffect } = React;

const Icon = ({ name, size = 14, color = "currentColor", sw = 1.5 }) => {
  const s = size, c = color;
  const p = {
    back: <path d="M9 3L4 8l5 5M4 8h9" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    star: <path d="M8 2l1.8 3.6 4 .6-2.9 2.8.7 4L8 11.2 4.4 13l.7-4L2.2 6.2l4-.6L8 2z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
    starF: <path d="M8 2l1.8 3.6 4 .6-2.9 2.8.7 4L8 11.2 4.4 13l.7-4L2.2 6.2l4-.6L8 2z" fill={c}/>,
    download: <path d="M8 2v8M5 7l3 3 3-3M3 13h10" stroke={c} strokeWidth={sw} strokeLinecap="round" fill="none"/>,
    sparkle: <path d="M8 2l1.2 3.8L13 7l-3.8 1.2L8 12l-1.2-3.8L3 7l3.8-1.2L8 2z" stroke={c} strokeWidth={sw} fill="none"/>,
    more: <><circle cx="3.5" cy="8" r="1" fill={c}/><circle cx="8" cy="8" r="1" fill={c}/><circle cx="12.5" cy="8" r="1" fill={c}/></>,
    search: <><circle cx="7" cy="7" r="4.5" stroke={c} strokeWidth={sw}/><path d="M10.5 10.5l3 3" stroke={c} strokeWidth={sw} strokeLinecap="round"/></>,
    plus: <path d="M8 3v10M3 8h10" stroke={c} strokeWidth={sw} strokeLinecap="round"/>,
    minus: <path d="M3 8h10" stroke={c} strokeWidth={sw} strokeLinecap="round"/>,
    fit: <path d="M3 5V3h2M11 3h2v2M13 11v2h-2M5 13H3v-2" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    chevL: <path d="M9.5 4l-3.5 4 3.5 4" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    chevR: <path d="M6.5 4l3.5 4-3.5 4" stroke={c} strokeWidth={sw} strokeLinecap="round" strokeLinejoin="round" fill="none"/>,
    sidebar: <><rect x="2" y="3" width="12" height="10" rx="1.5" stroke={c} strokeWidth={sw}/><path d="M6 3v10" stroke={c} strokeWidth={sw}/></>,
    sidebarR: <><rect x="2" y="3" width="12" height="10" rx="1.5" stroke={c} strokeWidth={sw}/><path d="M10 3v10" stroke={c} strokeWidth={sw}/></>,
    pen: <path d="M11 3l2 2-7.5 7.5L3 13l.5-2.5L11 3z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
    highlight: <path d="M3 13h10M5 11l4-7 3 1.5-4 7-3-1.5z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
    note: <path d="M3 3h7l3 3v7H3V3zM10 3v3h3" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
    quote: <path d="M4 4h3v4H4V4zM3 8h4v3l-2 2-2-1V8zM9 4h3v4H9V4zM8 8h4v3l-2 2-2-1V8z" fill={c}/>,
    link: <path d="M9 7l-2 2M6.5 4.5l1-1a2.5 2.5 0 0 1 3.5 3.5l-1 1M9.5 11.5l-1 1a2.5 2.5 0 0 1-3.5-3.5l1-1" stroke={c} strokeWidth={sw} strokeLinecap="round" fill="none"/>,
    folder: <path d="M2.5 4.5a1 1 0 0 1 1-1H6l1.5 1.5h5a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1V4.5z" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
    cite: <path d="M3 3h7l3 3v7H3V3zM6 8h4M6 10h3" stroke={c} strokeWidth={sw} strokeLinejoin="round" fill="none"/>,
  };
  return <svg width={s} height={s} viewBox="0 0 16 16" fill="none">{p[name]}</svg>;
};

const TAG_COLORS = {
  amber:  { bg: "oklch(0.95 0.05 75)",  fg: "oklch(0.42 0.12 65)",  dot: "oklch(0.7 0.13 70)" },
  blue:   { bg: "oklch(0.95 0.04 240)", fg: "oklch(0.42 0.12 245)", dot: "oklch(0.6 0.13 245)" },
  rose:   { bg: "oklch(0.95 0.04 15)",  fg: "oklch(0.45 0.13 15)",  dot: "oklch(0.65 0.15 15)" },
};

function TagPill({ name, color = "amber" }) {
  const c = TAG_COLORS[color] || TAG_COLORS.amber;
  return <span style={{
    display: "inline-flex", alignItems: "center", gap: 4,
    padding: "1px 7px 1px 6px", borderRadius: 999,
    background: c.bg, color: c.fg, fontSize: 10.5, fontWeight: 500,
  }}>
    <span style={{ width: 5, height: 5, borderRadius: 999, background: c.dot }} />
    {name}
  </span>;
}

// ─── Top header ──────────────────────────────────────────────
function Header({ entry, onBack }) {
  return <header style={{
    flexShrink: 0, height: 50, padding: "0 14px",
    borderBottom: "1px solid var(--border)", background: "var(--surface)",
    display: "flex", alignItems: "center", gap: 12,
  }}>
    <button onClick={onBack} style={btn()}>
      <Icon name="back" size={13} color="var(--text-mute)" />
      <span>ライブラリ</span>
    </button>
    <div style={{ width: 1, height: 18, background: "var(--border)" }} />
    <span style={{ fontSize: 10.5, fontWeight: 600, padding: "1px 6px",
      borderRadius: 4, background: "var(--surface-2)", color: "var(--text-mute)",
      letterSpacing: "0.04em", textTransform: "uppercase" }}>article</span>
    <h1 style={{
      margin: 0, fontSize: 14, fontWeight: 600, color: "var(--text)",
      flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
      letterSpacing: "-0.005em",
    }}>{entry.title}</h1>
    <button style={btn()}>
      <Icon name="starF" size={13} color="oklch(0.72 0.14 70)" />
    </button>
    <button style={btn()}>
      <Icon name="cite" size={13} color="var(--text-mute)" />
      <span>引用</span>
    </button>
    <button style={btn()}>
      <Icon name="sparkle" size={13} color="var(--text-mute)" />
      <span>要約</span>
    </button>
    <button style={btn()}>
      <Icon name="download" size={13} color="var(--text-mute)" />
    </button>
    <button style={btn()}>
      <Icon name="more" size={13} color="var(--text-mute)" />
    </button>
  </header>;
}

function btn() { return {
  display: "inline-flex", alignItems: "center", gap: 5,
  padding: "5px 9px", border: "1px solid transparent",
  borderRadius: 5, background: "transparent",
  fontSize: 12, fontWeight: 500, color: "var(--text)",
  cursor: "pointer",
}; }

// ─── PDF Thumbnails (left rail) ──────────────────────────────
function Thumbnails({ pages, current, onSelect }) {
  return <aside style={{
    width: 96, flexShrink: 0, height: "100%",
    background: "var(--sidebar)", borderRight: "1px solid var(--border)",
    overflow: "auto", padding: "10px 0",
  }}>
    {Array.from({ length: pages }).map((_, i) => (
      <div key={i} onClick={() => onSelect(i + 1)} style={{
        margin: "0 12px 8px", cursor: "pointer", textAlign: "center",
      }}>
        <div style={{
          width: 72, height: 92, background: "white",
          border: current === i + 1 ? "2px solid var(--accent-strong)" : "1px solid var(--border)",
          borderRadius: 2, position: "relative",
          boxShadow: "0 1px 2px rgba(0,0,0,0.04)",
          padding: "8px 6px", overflow: "hidden",
        }}>
          {/* mini page preview */}
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {i === 0 && <div style={{ height: 4, margin: "2px 8px", background: "#444" }} />}
            {i === 0 && <div style={{ height: 2, margin: "0 12px 4px", background: "#888" }} />}
            {Array.from({ length: 12 }).map((_, j) => (
              <div key={j} style={{
                height: 1.5, background: "#aaa",
                width: j % 4 === 3 ? "60%" : "100%",
              }} />
            ))}
          </div>
        </div>
        <div style={{
          marginTop: 3, fontSize: 10, color: current === i + 1 ? "var(--accent-strong)" : "var(--text-faint)",
          fontFamily: "var(--mono)", fontWeight: current === i + 1 ? 600 : 400,
        }}>{i + 1}</div>
      </div>
    ))}
  </aside>;
}

// ─── PDF Viewer ──────────────────────────────────────────────
function PDFToolbar({ page, pages, setPage, zoom, setZoom, search, setSearch, mode, setMode, leftOpen, setLeftOpen, rightOpen, setRightOpen }) {
  return <div style={{
    flexShrink: 0, height: 38, padding: "0 12px",
    borderBottom: "1px solid var(--border)", background: "var(--surface)",
    display: "flex", alignItems: "center", gap: 8,
  }}>
    <button onClick={() => setLeftOpen(!leftOpen)} style={iconBtn(leftOpen)}>
      <Icon name="sidebar" size={13} color={leftOpen ? "var(--text)" : "var(--text-mute)"} />
    </button>

    <div style={{ width: 1, height: 18, background: "var(--border)" }} />

    {/* page nav */}
    <button onClick={() => setPage(Math.max(1, page - 1))} style={iconBtn()}>
      <Icon name="chevL" size={12} color="var(--text-mute)" />
    </button>
    <div style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 12, color: "var(--text)" }}>
      <input value={page} onChange={(e) => setPage(Math.max(1, Math.min(pages, +e.target.value || 1)))}
        style={{
          width: 36, padding: "3px 6px", borderRadius: 4,
          border: "1px solid var(--border-strong)",
          background: "var(--surface-2)", textAlign: "center",
          fontFamily: "var(--mono)", fontSize: 12, color: "var(--text)",
          outline: "none",
        }} />
      <span style={{ color: "var(--text-faint)", fontSize: 11.5, fontFamily: "var(--mono)" }}>/ {pages}</span>
    </div>
    <button onClick={() => setPage(Math.min(pages, page + 1))} style={iconBtn()}>
      <Icon name="chevR" size={12} color="var(--text-mute)" />
    </button>

    <div style={{ width: 1, height: 18, background: "var(--border)" }} />

    {/* zoom */}
    <button onClick={() => setZoom(Math.max(50, zoom - 10))} style={iconBtn()}>
      <Icon name="minus" size={12} color="var(--text-mute)" />
    </button>
    <div style={{
      fontSize: 11.5, color: "var(--text)", minWidth: 42, textAlign: "center",
      fontFamily: "var(--mono)", fontVariantNumeric: "tabular-nums",
    }}>{zoom}%</div>
    <button onClick={() => setZoom(Math.min(200, zoom + 10))} style={iconBtn()}>
      <Icon name="plus" size={12} color="var(--text-mute)" />
    </button>
    <button onClick={() => setZoom(100)} style={iconBtn()} title="フィット">
      <Icon name="fit" size={12} color="var(--text-mute)" />
    </button>

    <div style={{ width: 1, height: 18, background: "var(--border)" }} />

    {/* annotation tools */}
    <div style={{ display: "inline-flex", padding: 1, gap: 1,
      background: "var(--surface-2)", borderRadius: 5,
      border: "1px solid var(--border)" }}>
      {[["select", null], ["highlight", "highlight"], ["note", "note"], ["pen", "pen"]].map(([m, ic]) => (
        <button key={m} onClick={() => setMode(m)} style={{
          width: 26, height: 22, padding: 0, border: "none",
          background: mode === m ? "var(--surface)" : "transparent",
          boxShadow: mode === m ? "0 0 0 1px var(--border) inset" : "none",
          borderRadius: 3, cursor: "pointer",
          display: "inline-flex", alignItems: "center", justifyContent: "center",
        }}>
          {ic ? <Icon name={ic} size={12} color={mode === m ? "var(--text)" : "var(--text-mute)"} />
            : <span style={{ fontSize: 10, color: mode === m ? "var(--text)" : "var(--text-mute)" }}>↖</span>}
        </button>
      ))}
    </div>

    <div style={{ flex: 1 }} />

    {/* search */}
    <div style={{
      display: "flex", alignItems: "center", gap: 5,
      width: 200, height: 26, padding: "0 8px",
      background: "var(--surface-2)", border: "1px solid var(--border)",
      borderRadius: 5,
    }}>
      <Icon name="search" size={11} color="var(--text-faint)" />
      <input value={search} onChange={(e) => setSearch(e.target.value)}
        placeholder="本文を検索…"
        style={{ flex: 1, border: "none", background: "transparent", outline: "none",
          fontSize: 12, color: "var(--text)" }} />
      {search && <span style={{ fontSize: 10.5, color: "var(--text-faint)",
        fontFamily: "var(--mono)" }}>3 / 12</span>}
    </div>

    <button onClick={() => setRightOpen(!rightOpen)} style={iconBtn(rightOpen)}>
      <Icon name="sidebarR" size={13} color={rightOpen ? "var(--text)" : "var(--text-mute)"} />
    </button>
  </div>;
}

function iconBtn(active) { return {
  width: 26, height: 26, padding: 0, border: "none",
  background: active ? "var(--surface-2)" : "transparent",
  boxShadow: active ? "0 0 0 1px var(--border) inset" : "none",
  borderRadius: 5, cursor: "pointer",
  display: "inline-flex", alignItems: "center", justifyContent: "center",
}; }

function PDFPage1({ zoom }) {
  return <div className="pdf-page" style={{ transform: `scale(${zoom / 100})`, transformOrigin: "top center" }}>
    <h1>Attention Is All You Need</h1>
    <div className="authors">
      Ashish Vaswani∗ &nbsp; Noam Shazeer∗ &nbsp; Niki Parmar∗ &nbsp; Jakob Uszkoreit∗<br/>
      Llion Jones∗ &nbsp; Aidan N. Gomez∗† &nbsp; Łukasz Kaiser∗ &nbsp; Illia Polosukhin∗‡
    </div>
    <div className="affil">Google Brain &nbsp;·&nbsp; Google Research &nbsp;·&nbsp; University of Toronto</div>

    <h2 style={{ textAlign: "center" }}>Abstract</h2>
    <p style={{ padding: "0 28px" }}>
      The dominant sequence transduction models are based on complex recurrent or convolutional neural networks
      that include an encoder and a decoder. The best performing models also connect the encoder and decoder
      through an attention mechanism. <span className="hl-y">We propose a new simple network architecture, the
      Transformer, based solely on attention mechanisms, dispensing with recurrence and convolutions entirely.</span>
      Experiments on two machine translation tasks show these models to be superior in quality while being more
      parallelizable and requiring significantly less time to train.
    </p>

    <div className="columns">
      <h2>1 &nbsp; Introduction</h2>
      <p>Recurrent neural networks, long short-term memory and gated recurrent neural networks in particular,
        have been firmly established as state of the art approaches in sequence modeling and transduction
        problems such as language modeling and machine translation.</p>
      <p>Recurrent models typically factor computation along the symbol positions of the input and output sequences.
        <span className="hl-g"> This inherently sequential nature precludes parallelization within training examples</span>,
        which becomes critical at longer sequence lengths.</p>
      <p>Attention mechanisms have become an integral part of compelling sequence modeling and transduction models
        in various tasks, allowing modeling of dependencies without regard to their distance in the input or output
        sequences.</p>

      <h2>2 &nbsp; Background</h2>
      <p>The goal of reducing sequential computation also forms the foundation of the Extended Neural GPU,
        ByteNet and ConvS2S, all of which use convolutional neural networks as basic building block.</p>
      <p>Self-attention, sometimes called intra-attention, is an attention mechanism relating different positions
        of a single sequence in order to compute a representation of the sequence.</p>

      <div className="figure">[ Figure 1: The Transformer model architecture ]</div>
      <div className="caption">Figure 1: The Transformer — model architecture.</div>

      <h2>3 &nbsp; Model Architecture</h2>
      <p>Most competitive neural sequence transduction models have an encoder-decoder structure. Here, the encoder
        maps an input sequence of symbol representations (x₁, ..., xₙ) to a sequence of continuous representations
        z = (z₁, ..., zₙ).</p>
      <div className="equation">Attention(Q, K, V) = softmax(QKᵀ / √dₖ) V</div>
      <p><span className="hl-b">The two most commonly used attention functions are additive attention and dot-product
        (multiplicative) attention.</span> Dot-product attention is identical to our algorithm, except for the
        scaling factor of 1/√dₖ.</p>
      <p>While for small values of dₖ the two mechanisms perform similarly, additive attention outperforms dot
        product attention without scaling for larger values of dₖ.</p>
    </div>
    <div className="pageno">1</div>
  </div>;
}

function PDFViewer({ leftOpen, setLeftOpen, rightOpen, setRightOpen }) {
  const [page, setPage] = useState(1);
  const [zoom, setZoom] = useState(100);
  const [search, setSearch] = useState("");
  const [mode, setMode] = useState("select");
  const pages = 15;

  return <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0,
    background: "oklch(0.94 0.005 80)" }}>
    <PDFToolbar
      page={page} pages={pages} setPage={setPage}
      zoom={zoom} setZoom={setZoom}
      search={search} setSearch={setSearch}
      mode={mode} setMode={setMode}
      leftOpen={leftOpen} setLeftOpen={setLeftOpen}
      rightOpen={rightOpen} setRightOpen={setRightOpen}
    />
    <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
      {leftOpen && <Thumbnails pages={pages} current={page} onSelect={setPage} />}
      <div style={{ flex: 1, overflow: "auto" }}>
        <PDFPage1 zoom={zoom} />
      </div>
    </div>
  </div>;
}

// ─── Right metadata sidebar ───────────────────────────────────
function MetaTabs({ tab, setTab }) {
  const tabs = [["info", "情報"], ["highlights", "ハイライト"], ["notes", "ノート"], ["related", "関連"]];
  return <div style={{
    display: "flex", borderBottom: "1px solid var(--border)",
    padding: "0 8px", flexShrink: 0,
  }}>
    {tabs.map(([k, l]) => (
      <button key={k} onClick={() => setTab(k)} style={{
        flex: 1, padding: "9px 0", border: "none", background: "transparent",
        fontSize: 12, fontWeight: tab === k ? 600 : 500,
        color: tab === k ? "var(--text)" : "var(--text-mute)",
        borderBottom: tab === k ? "2px solid var(--accent-strong)" : "2px solid transparent",
        marginBottom: -1, cursor: "pointer",
      }}>{l}</button>
    ))}
  </div>;
}

function Field({ label, value, mono }) {
  if (!value) return null;
  return <div style={{ marginBottom: 12 }}>
    <div style={{ fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
      textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 3 }}>{label}</div>
    <div style={{ fontSize: 12.5, color: "var(--text)",
      fontFamily: mono ? "var(--mono)" : "inherit", lineHeight: 1.45,
      wordBreak: "break-word" }}>{value}</div>
  </div>;
}

function MetaPanel({ entry }) {
  const [tab, setTab] = useState("info");
  return <aside style={{
    width: 340, flexShrink: 0, height: "100%",
    borderLeft: "1px solid var(--border)", background: "var(--surface)",
    display: "flex", flexDirection: "column", overflow: "hidden",
  }}>
    <MetaTabs tab={tab} setTab={setTab} />
    <div style={{ flex: 1, overflow: "auto", padding: "16px 18px" }}>
      {tab === "info" && <>
        <h3 style={{ margin: 0, fontSize: 13.5, fontWeight: 600, lineHeight: 1.35,
          color: "var(--text)", letterSpacing: "-0.005em" }}>{entry.title}</h3>
        <div style={{ marginTop: 8, fontSize: 11.5, color: "var(--text-mute)", lineHeight: 1.55 }}>
          {entry.authors.join(", ")}
        </div>
        <div style={{ marginTop: 6, fontSize: 11, color: "var(--text-faint)", fontStyle: "italic" }}>
          {entry.venue} · {entry.year}
        </div>

        <div style={{ height: 14 }} />
        <Field label="DOI" value={entry.doi} mono />
        <Field label="arXiv" value={entry.arxiv} mono />
        <Field label="抄録" value={entry.abstract} />

        <div style={{ marginTop: 6, marginBottom: 12 }}>
          <div style={{ fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
            textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6 }}>タグ</div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
            <TagPill name="transformer" color="amber" />
            <TagPill name="attention" color="blue" />
            <TagPill name="seminal" color="rose" />
          </div>
        </div>

        <div style={{ marginBottom: 12 }}>
          <div style={{ fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
            textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6 }}>コレクション</div>
          <div style={{ display: "flex", flexDirection: "column", gap: 3 }}>
            <div style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 12 }}>
              <Icon name="folder" size={12} color="var(--text-mute)" /> Transformer 系
            </div>
            <div style={{ display: "inline-flex", alignItems: "center", gap: 6, fontSize: 12 }}>
              <Icon name="folder" size={12} color="var(--text-mute)" /> サーベイ
            </div>
          </div>
        </div>

        <div>
          <div style={{ fontSize: 10.5, fontWeight: 600, color: "var(--text-faint)",
            textTransform: "uppercase", letterSpacing: "0.06em", marginBottom: 6 }}>関連文献</div>
          <div style={{ fontSize: 12, color: "var(--text-mute)", display: "flex",
            flexDirection: "column", gap: 6 }}>
            <div style={{
              padding: 8, borderRadius: 5, border: "1px solid var(--border)",
              background: "var(--surface-2)", cursor: "pointer",
            }}>
              <div style={{ fontSize: 10, color: "var(--text-faint)", marginBottom: 2 }}>preprint of →</div>
              <div style={{ fontSize: 11.5, color: "var(--text)", fontWeight: 500 }}>
                Vaswani et al. (arXiv 1706.03762)
              </div>
            </div>
          </div>
        </div>
      </>}

      {tab === "highlights" && <>
        <div style={{ fontSize: 11, color: "var(--text-faint)", marginBottom: 10 }}>
          3 件のハイライト · ページ 1
        </div>
        {[
          { color: "yellow", page: 1, text: "We propose a new simple network architecture, the Transformer, based solely on attention mechanisms, dispensing with recurrence and convolutions entirely.", note: "Transformer提案文。引用候補。" },
          { color: "green", page: 1, text: "This inherently sequential nature precludes parallelization within training examples", note: "" },
          { color: "blue", page: 1, text: "The two most commonly used attention functions are additive attention and dot-product (multiplicative) attention.", note: "" },
        ].map((h, i) => (
          <div key={i} style={{
            marginBottom: 10, padding: "10px 11px",
            borderRadius: 6, border: "1px solid var(--border)",
            background: "var(--surface-2)",
          }}>
            <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 6 }}>
              <span style={{
                width: 4, height: 14, borderRadius: 2,
                background: h.color === "yellow" ? "oklch(0.85 0.15 95)"
                  : h.color === "green" ? "oklch(0.78 0.13 145)"
                  : "oklch(0.7 0.13 240)",
              }} />
              <span style={{ fontSize: 10.5, color: "var(--text-faint)",
                fontFamily: "var(--mono)" }}>p.{h.page}</span>
            </div>
            <div style={{ fontSize: 11.5, color: "var(--text)", lineHeight: 1.5,
              fontFamily: "'IBM Plex Serif', Georgia, serif" }}>"{h.text}"</div>
            {h.note && <div style={{
              marginTop: 7, paddingTop: 7, borderTop: "1px dashed var(--border)",
              fontSize: 11, color: "var(--text-mute)", lineHeight: 1.5,
            }}>📝 {h.note}</div>}
          </div>
        ))}
      </>}

      {tab === "notes" && <>
        <div style={{
          fontSize: 12.5, lineHeight: 1.65, color: "var(--text)",
          padding: "8px 0",
        }}>
          <p style={{ margin: "0 0 10px" }}><b>TransformerはBERT・GPT系列の基礎。</b></p>
          <p style={{ margin: "0 0 10px" }}>Multi-Head Attentionの定式化を確認した。Q·Kᵀ をスケーリングする理由（√dₖ）は、勾配が小さくなりすぎないようにするため。</p>
          <p style={{ margin: "0 0 10px" }}>修論§2.1 で引用予定。BERT (Devlin 2019) と一緒に置く。</p>
          <hr style={{ margin: "14px 0", border: "none", borderTop: "1px solid var(--border)" }}/>
          <p style={{ margin: 0, color: "var(--text-mute)", fontSize: 11.5 }}>
            🔗 <a href="#" style={{ color: "var(--accent-strong)", textDecoration: "none" }}>BERT: Pre-training of Deep Bidirectional Transformers</a>
          </p>
        </div>
        <button style={{
          marginTop: 12, padding: "5px 11px", borderRadius: 5,
          border: "1px solid var(--border-strong)", background: "var(--surface)",
          color: "var(--text)", fontSize: 11.5, cursor: "pointer",
          display: "inline-flex", alignItems: "center", gap: 5,
        }}>
          <Icon name="pen" size={11} color="var(--text-mute)" />
          編集
        </button>
      </>}

      {tab === "related" && <>
        <div style={{ fontSize: 11, color: "var(--text-faint)", marginBottom: 10 }}>引用関係 · 同コレクション</div>
        {[
          { rel: "preprint of", title: "Attention Is All You Need (arXiv)", year: 2017 },
          { rel: "cited by", title: "BERT: Pre-training of Deep Bidirectional Transformers", year: 2019 },
          { rel: "cited by", title: "An Image is Worth 16x16 Words", year: 2021 },
          { rel: "cited by", title: "LoRA: Low-Rank Adaptation of LLMs", year: 2022 },
        ].map((r, i) => (
          <div key={i} style={{
            padding: "8px 10px", marginBottom: 6,
            borderRadius: 5, border: "1px solid var(--border)",
            background: "var(--surface-2)", cursor: "pointer",
          }}>
            <div style={{ fontSize: 9.5, color: "var(--accent-strong)", fontWeight: 600,
              textTransform: "uppercase", letterSpacing: "0.04em", marginBottom: 3 }}>{r.rel}</div>
            <div style={{ fontSize: 11.5, color: "var(--text)", fontWeight: 500,
              lineHeight: 1.35 }}>{r.title}</div>
            <div style={{ fontSize: 10.5, color: "var(--text-faint)",
              marginTop: 2, fontFamily: "var(--mono)" }}>{r.year}</div>
          </div>
        ))}
      </>}
    </div>
  </aside>;
}

// ─── App ──────────────────────────────────────────────────────
const ENTRY = {
  title: "Attention Is All You Need",
  authors: ["Vaswani, A.", "Shazeer, N.", "Parmar, N.", "Uszkoreit, J.", "Jones, L.", "Gomez, A. N.", "Kaiser, Ł.", "Polosukhin, I."],
  year: 2017, venue: "NeurIPS", doi: "10.48550/arXiv.1706.03762", arxiv: "1706.03762",
  abstract: "The dominant sequence transduction models are based on complex recurrent or convolutional neural networks. We propose a new simple network architecture, the Transformer, based solely on attention mechanisms, dispensing with recurrence and convolutions entirely.",
};

function App() {
  const [leftOpen, setLeftOpen] = useState(true);
  const [rightOpen, setRightOpen] = useState(true);

  return <div style={{
    width: "100%", height: "100%", background: "var(--bg)", color: "var(--text)",
    display: "flex", flexDirection: "column", overflow: "hidden",
    fontFamily: '"IBM Plex Sans", "Noto Sans JP", system-ui, sans-serif',
  }}>
    <Header entry={ENTRY} onBack={() => { window.location.href = "Library.html"; }} />
    <div style={{ flex: 1, display: "flex", minHeight: 0 }}>
      <PDFViewer
        leftOpen={leftOpen} setLeftOpen={setLeftOpen}
        rightOpen={rightOpen} setRightOpen={setRightOpen}
      />
      {rightOpen && <MetaPanel entry={ENTRY} />}
    </div>
  </div>;
}

ReactDOM.createRoot(document.getElementById("root")).render(<App />);
