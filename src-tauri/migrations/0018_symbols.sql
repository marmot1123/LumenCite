-- LCIR Phase 6b（記号系）: 論文内の記号定義（`symbols`）と、その記号の出現（`symbol_occurrences`）
-- を保存する。定義文（"let $U$ be ...", "define $X$ as ...", "denote by $H$ ...", "$U := ...$"）
-- を認識し、記号の表層（surface_form）と説明（description）を取り出す（`ingestion::symbols`）。
--
-- **原資料由来と推定を区別**する: surface_form/description は TeX 本文の verbatim（origin='tex_source'）
-- だが、「この文がこの記号を定義している」という対応づけはヒューリスティック推定なので confidence を
-- 必ず持たせる（意味の確からしさではなく検出の確からしさ）。**PDF はインライン数式が区切り無しで
-- 潰れており記号を確実に切り出せないため対象外**（TeX 版のみ・検索/読み出しの分担と同型）。
--
-- roadmap §5.8-5.9 の TEXT-UUID DDL を LumenCite 規約（INTEGER PK・FK ON DELETE CASCADE・
-- datetime('now')）に適応。version 削除（=添付削除のカスケード）で記号・出現も消える。
-- 実験フラグ lcir.enabled が OFF の間は空のまま。
CREATE TABLE symbols (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    document_version_id INTEGER NOT NULL REFERENCES document_versions(id) ON DELETE CASCADE,
    surface_form        TEXT    NOT NULL,      -- 記号の表層（"U" / "\mathcal{H}" / "U_0"）
    normalized_form     TEXT,                   -- 装飾（\mathbf/\mathcal 等）を剥いた正規化・任意
    description         TEXT,                   -- 定義文から取り出した説明（"the time evolution operator"）
    symbol_type         TEXT,                   -- operator/matrix/set/graph/... の推定・任意
    defined_at_node_id  INTEGER REFERENCES document_nodes(id) ON DELETE CASCADE,
    scope_node_id       INTEGER REFERENCES document_nodes(id) ON DELETE SET NULL,  -- 定義を含む節（軽いスコープ）
    semantic_json       TEXT,                   -- 未モデル化の意味属性（後続フェーズ）
    confidence          REAL,                   -- **定義検出**の確からしさ（意味ではない）
    origin              TEXT,                   -- tex_source（TeX 本文由来）
    created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_symbols_version ON symbols(document_version_id);
CREATE INDEX idx_symbols_defined_at ON symbols(defined_at_node_id);

CREATE TABLE symbol_occurrences (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    symbol_id         INTEGER NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
    node_id           INTEGER NOT NULL REFERENCES document_nodes(id) ON DELETE CASCADE,
    local_offset_json TEXT,                     -- ノード内オフセット等（未使用・後続）
    surface_form      TEXT    NOT NULL,         -- 出現時の表層
    confidence        REAL,                     -- 出現照合の確からしさ（表層一致は近似）
    origin            TEXT,
    created_at        TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_symbol_occurrences_symbol ON symbol_occurrences(symbol_id);
CREATE INDEX idx_symbol_occurrences_node ON symbol_occurrences(node_id);
