-- LCIR Phase 3（数式表層）: 数式の複数表現を保存する。inline_math/display_math ノード 1 個に
-- つき 1 行。数式は単一形式に統一しない方針で、用途の異なる表現を列で併存させる。
--
-- PDF 由来ではまず表層（normalized_text = 正規化した Unicode 線形文字列）だけを持ち
-- semantic_status='surface_only' にする。LaTeX は Phase 4（TeX 取込）、Content MathML/OpenMath/
-- AST は Phase 7（意味）で埋める。**原文由来と推定を区別**するため origin と confidence を持たせ、
-- AI 推定の意味は必ず不確実なものとして扱う。
--
-- roadmap §5.4 の TEXT-UUID DDL を LumenCite 規約（INTEGER PK・FK ON DELETE CASCADE・
-- datetime('now')）に適応。ノード削除（=バージョン/添付削除のカスケード）で数式行も消える。
-- 実験フラグ lcir.enabled が OFF の間は空のまま。
CREATE TABLE math_expressions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id             INTEGER NOT NULL REFERENCES document_nodes(id) ON DELETE CASCADE,
    display_mode        TEXT    NOT NULL,      -- 'inline' | 'display'
    equation_label      TEXT,                  -- 数式番号 "(2.1)" 等（あれば）
    latex               TEXT,                  -- Phase 4（TeX 取込）で埋める
    presentation_mathml TEXT,                  -- 表示・構文構造（将来）
    content_mathml      TEXT,                  -- 意味（Phase 7）
    openmath_json       TEXT,                  -- 意味（Phase 7）
    normalized_text     TEXT,                  -- 検索用の正規化線形文字列（PDF 表層）
    ast_json            TEXT,                  -- 数式構文木（Phase 7）
    semantic_status     TEXT    NOT NULL,      -- not_attempted/surface_only/inferred/verified/source_provided
    confidence          REAL,                  -- 表層検出の確からしさ（意味の確からしさではない）
    origin              TEXT,                  -- pdf_text_layer/math_recognition/tex_source/...
    created_at          TEXT    NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_math_expressions_node ON math_expressions(node_id);
