//! LCIR Phase 10a: 文脈バンドル（`get_node_context`）。
//!
//! 「この定理の主張・前提定義・証明・参照数式・参照図表を 1 回で寄越せ」に答える。
//! **新表も新推定器も要らない** — `LcirDocument`（= 既存 7 表の派生ビュー）だけを入力に取る
//! **DB 非依存の決定的純関数**で、`export/` と同じ立ち位置（fs/DB に触らない・pdfium 不要・
//! `#[test]` で CI 完結）。DB からの読み出しは `ingestion::load_node_lcir` が担う。
//!
//! ## なぜ「ノード 1 個の周りを組む」ことが必要か（実データの非対称）
//!
//! - **PDF 版の定理ノードは主張を持っていない。** 木は `document > page > block > line` の
//!   平坦木で、ブロックの切れ目は行間ギャップだけで決まる。実測で `theorem` ノードの
//!   plain_text は平均 168 文字（TeX 版は 975 文字）で、主張の続き（式・"where …"）は
//!   **theorem ノードの子ではなく page 直下の兄弟**に落ちている。したがって「読み順で次の
//!   構造境界まで」を server-side で連結しないと、定理を読んだことにならない。
//! - **その連結はページをまたぐ。** 実測で proof の 53% / theorem の 33% が次の境界まで
//!   ページ境界を越える。`page` を親に持つ平坦木を `(page.ordinal, block.ordinal)` で
//!   つなぐ ＝ 木の pre-order で並べる、がロードマップ完了条件「ページ境界で文脈が
//!   切れない」の実体。
//! - **TeX 版は逆に環境本文が丸ごと 1 ノード**（内側の display 数式も生 LaTeX のまま本文に
//!   残る）。同じ規則を適用すると `continuation` は「定理の続き」ではなく「定理の後に
//!   続く地の文」になる。どちらも文脈として有用だが**意味が違う**ので、出力の
//!   `continuation` は「読み順で次の構造境界まで」とだけ定義し、解釈は呼び出し側に渡す。
//!
//! ## 集める根拠（誤検出より欠損・§16）
//!
//! 「前提定義」を運ぶ辺は実質存在しない（実測: 定理系ノード 2,242 件のうち `definition` を
//! 指す出辺を持つのは 31 件 = 1.4%）。`defines_symbol` / `depends_on` は語彙だけで生成経路が
//! 無い。そこで [`Premise::via`] に**導出経路を必ず併記**して 3 通りを別物として返す:
//!
//! | via | 根拠 | 版 |
//! |-----|------|----|
//! | `reference` | `refers_to_theorem` 辺の指し先が `definition` ノード（原文の `\ref{def:..}` / "Definition 3.1"） | 両方 |
//! | `occurrence` | `symbol_occurrences` に記録された出現（display 数式内の表層一致・Phase 6b） | TeX |
//! | `symbol` | 記号の `surface_form` が本文に `$X$` の形で現れる（**この関数内の表層照合**・非永続） | TeX |
//!
//! `symbol` だけがこの関数の照合だが、**新しい事実を推定して保存するのではなく**、既に
//! `symbols` にある定義文を読み手に見せるための絞り込みにすぎない。加えて「定義が焦点より
//! 読み順で前にあること」を必須にし、記号ごとに 1 件へ畳む。
//!
//! ## 2 ホップしない
//!
//! 定理のバンドルに「その証明が参照している数式」まで畳み込むと、深さが呼び出し側から
//! 見えなくなりバンドル長が予測できなくなる。`proofs` には proof の **node_id** を載せるので、
//! 必要なら同じツールをその id で呼べばよい（合成は呼び出し側の仕事）。

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::document_ir::{LcirDocument, LcirNode};

/// 焦点ノードの本文をそのまま返す上限（文字）。これを超えると末尾を切る。
const FOCUS_TEXT_CAP: usize = 4000;
/// `before` の各ノードの本文上限（向き付け用なので短くてよい）。
const BEFORE_TEXT_CAP: usize = 600;
/// 関係辺の相手ノードのスニペット上限。
const RELATED_TEXT_CAP: usize = 240;
/// 前提定義（定義文ノード）のスニペット上限。
const PREMISE_TEXT_CAP: usize = 400;

/// バンドルの大きさ（呼び出し側が調整できる上限）。既定値は「1 定理を読むのに足りて、
/// LLM のコンテキストを食い潰さない」ところに置いた。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextOptions {
    /// 焦点の前に何ブロック載せるか（向き付け用）。最初の構造境界を含めたら打ち切る。
    pub max_before: usize,
    /// 焦点の後に何ブロック載せるか（次の構造境界の**手前**まで）。
    pub max_continuation: usize,
    /// `continuation` 全体の文字数上限。
    pub max_continuation_chars: usize,
    /// 関係リスト（equations / figures / citations / references / proofs）1 本あたりの件数上限。
    pub max_related: usize,
    /// 前提定義の件数上限。
    pub max_premises: usize,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            max_before: 2,
            // 実ライブラリ実測（latest pdfium 版・定理系 1,375 件の「次の構造境界まで」の
            // ブロック数）: 8 で 78.3% / 12 で 84.7% / **16 で 88.9%** / 24 で 92.3%。
            // 16 以降は伸びが鈍る。数式の続きは 1 ブロックが数十文字と短いので、実際の
            // 大きさは `max_continuation_chars` の方で抑える。
            max_continuation: 16,
            max_continuation_chars: 6000,
            max_related: 12,
            max_premises: 12,
        }
    }
}

/// バンドルに載せるノード 1 件。`page`/`bbox` は PDF 版のみ（TeX 版に座標は存在しない）。
/// `origin` / `confidence` を必ず透過するので、呼び出し側は原文由来（`tex_source` /
/// `pdf_text_layer`）と推定（`layout_model` / `llm_inference`）を区別できる。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextNode {
    pub node_id: i64,
    pub kind: String,
    /// 本文。`figure` ノードのように本文を持たないノードでは省略される。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `text` が上限で切られたか。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub text_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<i64>,
    /// `[x, y, width, height]`（PDF user space・左下原点・pt）。既存 MCP ツールと同じ 4 要素配列。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// `payload_json` から拾う識別子（`theorem_number` / `section_number` / `note` / `labels` /
    /// `figure_number` / `caption_number`）。無いキーは載らない。
    #[serde(skip_serializing_if = "serde_json::Map::is_empty")]
    pub identifiers: serde_json::Map<String, serde_json::Value>,
    /// 数式表現（`display_math` ノードのみ）。PDF 版は `latex` を持たず `normalized_text` だけ。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub math: Option<crate::document_ir::LcirMath>,
    /// 図の代替テキスト（Phase 8c・**LLM Vision の生成物**または手編集）。`origin` を必ず見ること。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<crate::document_ir::LcirAltText>,
    /// 図の crop PNG 等。`relative_path` はメタデータ参照で実体の存在は保証しない。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<crate::document_ir::LcirAsset>,
}

/// 関係辺 1 本と、その相手ノード。`from_node_id` は「焦点そのもの」とは限らない
/// （PDF では定理の続きブロックが参照を持つため `continuation` のノードからも辺を拾う）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Related {
    pub relation_type: String,
    /// `outgoing` = 焦点（またはその続き）から出る辺 / `incoming` = 焦点に入る辺。
    pub direction: &'static str,
    pub from_node_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub node: ContextNode,
}

/// 図・表への参照。**辺の指し先は実体とは限らない**（PDF は図領域が検出できていれば `figure`
/// ノード、できなければ `figure_caption` ノードを指す）ので、`caption_of` を 1 ホップ辿って
/// 実体と caption の両方を解決した結果を併記する。
///
/// - `node` … 辺が指しているノードそのもの（無加工）。
/// - `figure` … 領域（bbox）・crop アセット・alt text の持ち主。到達できないことがある
///   （実 DB の実測は `figure_caption` 1,021 件中 262 件 = 25.7% だが、これは**再構築前**の値。
///   コード側は 8d-8 / 8d-2 で 662 件まで増えている ＝ 再構築後は多数派が逆転する）。
/// - `caption` … 原文のキャプション文。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FloatRef {
    pub relation_type: String,
    pub from_node_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// `node`（実体に解決）/ `caption`（caption に落ちた）。PDF の `refers_to_figure` /
    /// `refers_to_table` にのみ付く（TeX 経路と `caption_of` には無い）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_via: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub node: ContextNode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub figure: Option<ContextNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<ContextNode>,
}

/// 焦点が依拠する定義。`via` が導出経路（[`crate::context`] のモジュール doc の表）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Premise {
    pub via: &'static str,
    /// 定義文のノード。**`definition` ノードとは限らない** — 実測で `symbols.defined_at_node_id`
    /// の過半は `paragraph`（185/356）で、`definition` は 26 件しかない。
    pub node: ContextNode,
    /// 記号経由（`occurrence` / `symbol`）のときの記号。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<PremiseSymbol>,
    /// 辺経由（`reference`）のときの辺。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relation: Option<Related>,
}

/// 前提定義に添える記号（`symbols` の抜粋）。`confidence` は「**この文がこの記号を定義して
/// いる**という対応づけ」の確からしさで、意味の正しさではない。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PremiseSymbol {
    pub surface_form: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_form: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// バンドルに載らなかった/信用を割り引くべき事実の機械可読な安定 ID。
///
/// `export::ExportWarningCode`（「この**出力形式**では運べない」）とは別物なので型を共有
/// しない — こちらは形式の話ではなく**このバンドルの中身**の話。命名規約は同じで、
/// 一度出したら綴りを変えない。`export::warning` と同じ「狼少年にしない 3 規約」に従う:
/// ①このバンドルで実際に起きたことだけ報告する ②どの文書でも必ず真になる一般論
/// （「複数形の参照には辺が無い」等）は載せない — それはツール説明の仕事 ③1 つの事実を
/// 1 コードで報告する。
///
/// 規約②で**落としたもの**（過去に入れていたが外した）: 「TeX 版に座標が無い」
/// 「PDF 版に記号が無い」。どちらも `source` から機械的に決まる表現ごとの恒真命題で、
/// PDF 版バンドルの 100% / TeX 版バンドルの 100% に必ず付く ＝ 注記として情報量が無い。
/// 事実自体はツール説明に書いてある。`notes` は「**今回**届かなかったもの」に保つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextNoteCode {
    /// 焦点が本文ブロックではない（`document`/`page`/`line`）ので前後の文脈を組めない。
    FocusIsNotAContentBlock,
    /// `continuation` を上限（件数または文字数）で打ち切った。続きは残っている。
    ContinuationTruncated,
    /// 図表参照のうち実体（`figure`/`table` ノード）へ到達できなかったものがある。
    /// その参照では領域・crop・alt text が取れない。
    FloatEntityUnreachable,
    /// `proves` の相手が定理・補題・命題・系ではない（`remark`/`example`/`definition`）。
    /// 隣接フォールバックの副作用で起きる。
    ProvesTargetIsNotATheorem,
    /// 件数上限で関係リストを切った。
    RelatedTruncated,
    /// 件数上限で `premises` を切った。
    PremisesTruncated,
}

impl ContextNoteCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextNoteCode::FocusIsNotAContentBlock => "focus_is_not_a_content_block",
            ContextNoteCode::ContinuationTruncated => "continuation_truncated",
            ContextNoteCode::FloatEntityUnreachable => "float_entity_unreachable",
            ContextNoteCode::ProvesTargetIsNotATheorem => "proves_target_is_not_a_theorem",
            ContextNoteCode::RelatedTruncated => "related_truncated",
            ContextNoteCode::PremisesTruncated => "premises_truncated",
        }
    }

    /// 呼び出し側（LLM）にそのまま見せる 1 文。
    pub fn message(self) -> &'static str {
        match self {
            ContextNoteCode::FocusIsNotAContentBlock => {
                "the focus node is a skeleton node (document/page/line), so no surrounding blocks \
                 were assembled; pass a content block id from search_document_nodes or \
                 get_document_blocks"
            }
            ContextNoteCode::ContinuationTruncated => {
                "the continuation was cut at a size limit before reaching the next structural \
                 boundary; more blocks follow"
            }
            ContextNoteCode::FloatEntityUnreachable => {
                "at least one figure/table reference resolved only to a caption block, with no \
                 caption_of edge reaching the region itself, so its bbox, crop asset and alt text \
                 are unavailable; this is the norm on the tex representation (which builds no \
                 figure nodes) and happens on the pdf representation when neither a raster image \
                 nor a vector path cluster was detected next to the caption"
            }
            ContextNoteCode::ProvesTargetIsNotATheorem => {
                "a proves edge points at a remark/example/definition rather than a \
                 theorem/lemma/proposition/corollary; on the pdf representation most proves edges \
                 come from reading-order adjacency, so treat it as a hint"
            }
            ContextNoteCode::RelatedTruncated => {
                "at least one relation list was cut at max_related"
            }
            ContextNoteCode::PremisesTruncated => {
                "premises were cut at max_premises; more definitions back this block"
            }
        }
    }
}

/// `continuation` がどこで止まったか。**空/短いことの意味**を呼び出し側が読めるようにする。
///
/// - `boundary` — 次の論理単位が始まった（`node_id`/`kind` がその境界）。`kind` が
///   `figure_caption`/`table_caption` なら、フロートに割り込まれただけで主張はまだ続く
///   可能性がある（その caption の node_id で呼び直せる）。
/// - `max_continuation` / `max_continuation_chars` — 上限で切った。続きは残っている。
/// - `end_of_document` — 文書の終わり。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContinuationStop {
    pub reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

impl ContinuationStop {
    fn limit(reason: &'static str) -> Self {
        Self {
            reason,
            node_id: None,
            kind: None,
        }
    }
    fn truncated(&self) -> bool {
        matches!(self.reason, "max_continuation" | "max_continuation_chars")
    }
}

/// 注記 1 件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextNote {
    pub code: ContextNoteCode,
    pub message: &'static str,
}

/// 文脈バンドル。`entry_id` 等の書誌側は DB を引く呼び出し側（`mcp_server` / `cli`）が付ける。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeContext {
    pub node_id: i64,
    pub version_id: i64,
    /// 焦点ノード（本文は `FOCUS_TEXT_CAP` まで無加工）。
    pub focus: ContextNode,
    /// 焦点を囲む節（root 側から順）。平坦木なので「読み順で前にある見出し」を後ろ向きに拾う。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub section_path: Vec<ContextNode>,
    /// 焦点の直前のブロック（読み順・最初の構造境界を含めて打ち切り）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<ContextNode>,
    /// 焦点に続くブロック（読み順・次の構造境界の手前まで・ページをまたぐ）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub continuation: Vec<ContextNode>,
    /// `continuation` を**なぜそこで止めたか**。焦点が本文ブロックでなければ省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub continuation_stopped_at: Option<ContinuationStop>,
    /// この定理を証明する `proof`（`proves` の入辺）。1 定理に複数付きうる。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub proofs: Vec<Related>,
    /// この証明が証明する対象（`proves` の出辺）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub proves: Vec<Related>,
    /// 焦点が依拠する定義。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub premises: Vec<Premise>,
    /// `refers_to_equation` の指し先。**数式ノードとは限らない**（TeX の `\eqref` は参照先の
    /// 種別を見ないため）ので `node.kind` を必ず確認すること。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub equations: Vec<Related>,
    /// 図・表への参照（`refers_to_figure` / `refers_to_table` / 焦点が caption なら `caption_of`）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub figures: Vec<FloatRef>,
    /// `cites` の指し先（`bibliography_entry`）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<Related>,
    /// 上記以外の相互参照（`refers_to_theorem` / `refers_to_section` / `refers_to` …）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Related>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<ContextNote>,
}

// ── ノード種別の分類（純関数） ──────────────────────────────────────────────

/// 本文つき論理ブロック（骨格の `document`/`page`/`line` は除く）。
/// `llm::tools::document::is_content_block` と同じ集合（読み順の並びを両者でずらさない）。
fn is_content_block(kind: &str) -> bool {
    !matches!(kind, "document" | "page" | "line")
}

/// 読み順スパンを打ち切る「構造の切れ目」。ここに当たったら別の論理単位が始まっている。
///
/// **`heading` はレベル宣言のあるものだけ**を境界にする。実ライブラリの PDF 版 `heading`
/// 2,810 件のうち 2,640 件（94%）は `heading_level` を持たない confidence 0.55 の推定で、
/// 中身は数式から切り出された断片（`"AiAj ="` / `"fλ = n!"` / `"π P = π,"`）や柱
/// （`"204 SOME IMPORTANT DISTRIBUTIONS AND PROCESSES"`）である。これを境界に数えると
/// **定理の主張そのものの直前で continuation が止まる**（実測: 定理系スパン 2,526 件のうち
/// 240 件がレベル無し heading で停止し、うち 43 件は continuation 0 ブロック。例: 焦点
/// "Lemma 3.4 (Linearization formula)…" が `"AiAj ="` ＝その線形化公式で止まる）。
/// 見出しかどうか確信が持てないブロックで本文を捨てるより、節をまたいで数ブロック
/// 余分に載る方がまし — 載った側はテキストが見えるので読み手が判断できるが、
/// 捨てた側は見えない（§16「誤検出より欠損」は**構造の主張**に掛かる原則であって、
/// 「推定した境界で本文を落とす」ことの正当化にはならない）。
fn is_structural_boundary(n: &LcirNode) -> bool {
    if n.kind == "heading" {
        return heading_level_of(n).is_some();
    }
    matches!(
        n.kind.as_str(),
        "definition"
            | "theorem"
            | "lemma"
            | "proposition"
            | "corollary"
            | "remark"
            | "example"
            | "proof"
            | "section"
            | "subsection"
            | "abstract"
            | "front_matter"
            | "bibliography"
            | "bibliography_entry"
            | "figure_caption"
            | "table_caption"
    )
}

/// 見出し系（`section_path` を組む対象）の階層レベル。
///
/// **`heading_level` を宣言しているノードだけ**を見出しとして扱う。実ライブラリでは
/// `section`（1,758 件）と `subsection`（1,948 件）は必ず宣言している一方、`heading` は
/// 2,676 件が宣言なしで、その中身は数式から切り出された断片（`"bound"` / `"K nT nT"` 等）が
/// 多い。レベルなし `heading` を既定値で拾うと、そういう断片を「この定理を囲む節」として
/// LLM に見せることになる（誤検出より欠損・§16）。`abstract`/`front_matter` は宣言を
/// 持たないので対象外 — 前書きは後続ノードを「囲んで」いない。
fn heading_level_of(n: &LcirNode) -> Option<i64> {
    if !matches!(n.kind.as_str(), "section" | "subsection" | "heading") {
        return None;
    }
    n.payload
        .as_ref()
        .and_then(|p| p.get("heading_level"))
        .and_then(|v| v.as_i64())
}

/// 図・表の**実体**ノード（領域・アセット・alt text の持ち主）。
fn is_float_entity(kind: &str) -> bool {
    matches!(kind, "figure" | "table")
}

/// 「証明されうるもの」のうち定理・補題・命題・系（`graph::is_theorem_family` は
/// `remark`/`example`/`definition` も含むので、注記の判定にはこちらの狭い集合を使う）。
fn is_strict_theorem(kind: &str) -> bool {
    matches!(kind, "theorem" | "lemma" | "proposition" | "corollary")
}

// ── 本体 ────────────────────────────────────────────────────────────────────

/// 文脈バンドルを組む。`focus_node_id` がこの文書に無ければ `None`。
///
/// 決定的: 同じ `(doc, focus_node_id, opts)` からは常に同じ結果になる（すべての一覧を
/// 明示ソートし、`HashMap` の反復順に依存しない）。
pub fn build_node_context(
    doc: &LcirDocument,
    focus_node_id: i64,
    opts: &ContextOptions,
) -> Option<NodeContext> {
    let by_id: HashMap<i64, &LcirNode> = doc.nodes.iter().map(|n| (n.id, n)).collect();
    let focus = *by_id.get(&focus_node_id)?;

    let mut notes: NoteSet = NoteSet::default();

    // 読み順（木の pre-order）。PDF の `document > page > block` も TeX の `document > block`
    // も同じ規則で 1 本の列になる ＝ ページ境界が列の上で消える。
    let order = reading_order(&doc.nodes);
    let seq: Vec<i64> = order
        .iter()
        .copied()
        .filter(|id| by_id.get(id).is_some_and(|n| is_content_block(&n.kind)))
        .collect();
    let pos: HashMap<i64, usize> = seq.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    let focus_pos = pos.get(&focus_node_id).copied();
    if focus_pos.is_none() {
        notes.add(ContextNoteCode::FocusIsNotAContentBlock);
    }

    let before = focus_pos
        .map(|p| collect_before(&seq, &by_id, p, opts.max_before))
        .unwrap_or_default();
    let stop = focus_pos.map(|p| collect_continuation(&seq, &by_id, p, opts));
    let continuation: Vec<ContextNode> =
        stop.as_ref().map(|(c, _)| c.clone()).unwrap_or_default();
    let continuation_stopped_at = stop.map(|(_, s)| s);
    if continuation_stopped_at.as_ref().is_some_and(|s| s.truncated()) {
        notes.add(ContextNoteCode::ContinuationTruncated);
    }
    let section_path = focus_pos
        .map(|p| collect_section_path(&seq, &by_id, p))
        .unwrap_or_default();

    // 辺の起点は「焦点 + その続き」。PDF では定理の主張が複数ブロックに割れており、
    // 参照（"by Lemma 2.1", "Eq. (3)"）は続きのブロックに乗っているため。
    let mut origin_ids: Vec<i64> = vec![focus_node_id];
    origin_ids.extend(continuation.iter().map(|c| c.node_id));
    let origin_set: HashSet<i64> = origin_ids.iter().copied().collect();

    // `caption_of` の 1 ホップ索引（PDF/TeX とも from=caption → to=figure/table・実測で 1:1）。
    let mut caption_to_entity: HashMap<i64, i64> = HashMap::new();
    let mut entity_to_caption: HashMap<i64, i64> = HashMap::new();
    for r in &doc.relations {
        if r.relation_type == "caption_of" {
            caption_to_entity.entry(r.from_node_id).or_insert(r.to_node_id);
            entity_to_caption.entry(r.to_node_id).or_insert(r.from_node_id);
        }
    }

    let mut proofs: Vec<Related> = Vec::new();
    let mut proves: Vec<Related> = Vec::new();
    let mut equations: Vec<Related> = Vec::new();
    let mut figures: Vec<FloatRef> = Vec::new();
    let mut citations: Vec<Related> = Vec::new();
    let mut references: Vec<Related> = Vec::new();

    for r in &doc.relations {
        // 入辺で拾うのは 2 種類だけ（それ以外の被参照は get_node_relations(node_id=…) の
        // 担当 — バンドルを無制限に太らせない）。
        //
        // ① この定理を証明する proof。
        if r.relation_type == "proves" && r.to_node_id == focus_node_id {
            if let Some(n) = by_id.get(&r.from_node_id) {
                proofs.push(related(r, "incoming", r.to_node_id, n, RELATED_TEXT_CAP));
            }
            continue;
        }
        // ② 焦点（または続き）が図表**実体**のときの caption。`caption_of` は
        //    from=caption → to=figure の一方向でしか保存されないので、実体を焦点に
        //    すると出辺が 1 本も無く、原文キャプション（引用時に最も要る文）へ到達
        //    できなくなる。`get_figures` は figure ノードの id を返すので、ツール説明
        //    どおりに使うとこの経路に入る。
        if r.relation_type == "caption_of" && origin_set.contains(&r.to_node_id) {
            if let (Some(entity), Some(_caption)) =
                (by_id.get(&r.to_node_id), by_id.get(&r.from_node_id))
            {
                figures.push(float_ref(
                    r,
                    entity,
                    &by_id,
                    &caption_to_entity,
                    &entity_to_caption,
                ));
            }
            continue;
        }
        if !origin_set.contains(&r.from_node_id) {
            continue;
        }
        let Some(target) = by_id.get(&r.to_node_id) else {
            continue;
        };
        match r.relation_type.as_str() {
            "proves" => {
                if !is_strict_theorem(&target.kind) {
                    notes.add(ContextNoteCode::ProvesTargetIsNotATheorem);
                }
                proves.push(related(r, "outgoing", r.from_node_id, target, RELATED_TEXT_CAP));
            }
            "refers_to_equation" => {
                equations.push(related(r, "outgoing", r.from_node_id, target, RELATED_TEXT_CAP))
            }
            "cites" => {
                citations.push(related(r, "outgoing", r.from_node_id, target, RELATED_TEXT_CAP))
            }
            "refers_to_figure" | "refers_to_table" | "caption_of" => {
                let f = float_ref(r, target, &by_id, &caption_to_entity, &entity_to_caption);
                if f.figure.is_none() {
                    notes.add(ContextNoteCode::FloatEntityUnreachable);
                }
                figures.push(f);
            }
            _ => {
                references.push(related(r, "outgoing", r.from_node_id, target, RELATED_TEXT_CAP))
            }
        }
    }

    // 前提定義の「明示参照」経路は**打ち切り前**の references から作る。`max_related` は
    // 参照一覧の見やすさのための上限であって、最も確かな前提定義がそれで落ちるのは筋が悪い。
    let mut reference_premise_targets: Vec<Related> = references
        .iter()
        .filter(|r| r.node.kind == "definition")
        .cloned()
        .collect();
    reference_premise_targets.sort_by_key(|r| (r.from_node_id, r.node.node_id));

    let mut cut = false;
    cut |= cap_related(&mut proofs, opts.max_related);
    cut |= cap_related(&mut proves, opts.max_related);
    cut |= cap_related(&mut equations, opts.max_related);
    cut |= cap_related(&mut citations, opts.max_related);
    cut |= cap_related(&mut references, opts.max_related);
    figures.sort_by_key(|a| (a.from_node_id, a.node.node_id));
    figures.dedup_by_key(|a| (a.from_node_id, a.node.node_id));
    if figures.len() > opts.max_related {
        figures.truncate(opts.max_related);
        cut = true;
    }
    if cut {
        notes.add(ContextNoteCode::RelatedTruncated);
    }

    let (premises, premises_cut) = collect_premises(
        doc,
        &by_id,
        &pos,
        focus_pos,
        &origin_ids,
        &equations,
        &reference_premise_targets,
        opts,
    );
    if premises_cut {
        notes.add(ContextNoteCode::PremisesTruncated);
    }

    Some(NodeContext {
        node_id: focus_node_id,
        version_id: doc.version_id,
        focus: context_node(focus, FOCUS_TEXT_CAP),
        section_path,
        before,
        continuation,
        continuation_stopped_at,
        proofs,
        proves,
        premises,
        equations,
        figures,
        citations,
        references,
        notes: notes.finish(),
    })
}

// ── 読み順 ──────────────────────────────────────────────────────────────────

/// ノード木の pre-order（子は `(ordinal, id)` 昇順）。親が複数回現れる/循環する壊れた木でも
/// 停止するよう訪問済みを持つ。木に繋がっていない孤児は末尾に `(parent_id, ordinal, id)` 順で足す
/// （落とすと焦点が列から消えるため）。
fn reading_order(nodes: &[LcirNode]) -> Vec<i64> {
    let mut children: HashMap<Option<i64>, Vec<(i64, i64)>> = HashMap::new();
    let ids: HashSet<i64> = nodes.iter().map(|n| n.id).collect();
    for n in nodes {
        // 親が同じ版に無い場合はルート扱い（版跨ぎの parent_id は実データに無いが、防御）。
        let parent = n.parent_id.filter(|p| ids.contains(p));
        children.entry(parent).or_default().push((n.ordinal, n.id));
    }
    for v in children.values_mut() {
        v.sort_unstable();
    }

    let mut out = Vec::with_capacity(nodes.len());
    let mut visited: HashSet<i64> = HashSet::new();
    let mut stack: Vec<i64> = children
        .get(&None)
        .map(|v| v.iter().rev().map(|(_, id)| *id).collect())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        out.push(id);
        if let Some(cs) = children.get(&Some(id)) {
            for (_, cid) in cs.iter().rev() {
                stack.push(*cid);
            }
        }
    }
    if out.len() < nodes.len() {
        let mut orphans: Vec<(i64, i64, i64)> = nodes
            .iter()
            .filter(|n| !visited.contains(&n.id))
            .map(|n| (n.parent_id.unwrap_or(0), n.ordinal, n.id))
            .collect();
        orphans.sort_unstable();
        out.extend(orphans.into_iter().map(|(_, _, id)| id));
    }
    out
}

/// 焦点の直前のブロック（向き付け用）。本文を持たないノードは**飛ばす** — 8a の `figure`
/// ノードはページ内の全ブロックより後ろの ordinal で入る（`ingestion/mod.rs` の
/// `ordinal: blocks.len() + ri`）ので、ページ先頭のブロックを焦点にすると `before` が
/// 本文ゼロの figure だけで埋まり、向き付けの役に立たない。図そのものは辺（`figures`）で
/// 到達できる。
fn collect_before(
    seq: &[i64],
    by_id: &HashMap<i64, &LcirNode>,
    focus_pos: usize,
    max: usize,
) -> Vec<ContextNode> {
    let mut out = Vec::new();
    let mut i = focus_pos;
    while i > 0 && out.len() < max {
        i -= 1;
        let Some(n) = by_id.get(&seq[i]) else { continue };
        if n.plain_text.as_ref().is_none_or(|t| t.trim().is_empty()) {
            continue;
        }
        let boundary = is_structural_boundary(n);
        out.push(context_node(n, BEFORE_TEXT_CAP));
        if boundary {
            // 直前の見出し/定理まで載せたら、そこから先は別の論理単位。
            break;
        }
    }
    out.reverse();
    out
}

/// 焦点に続くブロックを次の構造境界の**手前**まで。**なぜそこで止めたか**も返す —
/// 「主張が終わった」と「フロートのキャプションに割り込まれた」と「上限で切った」を
/// 呼び出し側が区別できないと、`continuation` が空/短いことの意味が読めない
/// （実測: 定理系 2,345 件のうち 63 件は次の境界が figure/table の caption で、
/// フロートはページ先頭・末尾に置かれるので、まさにページ跨ぎの連結中に起きる）。
fn collect_continuation(
    seq: &[i64],
    by_id: &HashMap<i64, &LcirNode>,
    focus_pos: usize,
    opts: &ContextOptions,
) -> (Vec<ContextNode>, ContinuationStop) {
    let mut out = Vec::new();
    let mut spent = 0usize;
    let mut i = focus_pos + 1;
    while i < seq.len() {
        let Some(n) = by_id.get(&seq[i]) else {
            i += 1;
            continue;
        };
        if is_structural_boundary(n) {
            return (
                out,
                ContinuationStop {
                    reason: "boundary",
                    node_id: Some(n.id),
                    kind: Some(n.kind.clone()),
                },
            );
        }
        if out.len() >= opts.max_continuation {
            return (out, ContinuationStop::limit("max_continuation"));
        }
        let remaining = opts.max_continuation_chars.saturating_sub(spent);
        if remaining == 0 {
            return (out, ContinuationStop::limit("max_continuation_chars"));
        }
        let node = context_node(n, remaining);
        // 予算には LaTeX 原文・alt text も数える。TeX 版の display 数式は plain_text とは
        // 別に `math.latex` を持つので、本文長だけで測ると応答が入力依存で膨らむ。
        spent += node_weight(&node);
        let cut = node.text_truncated;
        out.push(node);
        if cut {
            return (out, ContinuationStop::limit("max_continuation_chars"));
        }
        i += 1;
    }
    (out, ContinuationStop::limit("end_of_document"))
}

/// 応答サイズに効く文字数（本文 + 原文 LaTeX + 代替テキスト）。
fn node_weight(n: &ContextNode) -> usize {
    let text = n.text.as_ref().map_or(0, |t| t.chars().count());
    let math = n.math.as_ref().map_or(0, |m| {
        m.latex.as_ref().map_or(0, |l| l.chars().count())
            + m.normalized_text.as_ref().map_or(0, |t| t.chars().count())
    });
    let alt = n.alt_text.as_ref().map_or(0, |a| a.text.chars().count());
    text + math + alt
}

/// 焦点を囲む節。木が平坦（PDF は page 直下・TeX は document 直下）で `section` は兄弟なので、
/// 読み順を後ろ向きに走査して**見出しレベルが厳密に小さくなるもの**だけを拾う。
/// `ingestion::symbols` の `current_section`（`symbols.scope_node_id` の決め方）と**似ているが
/// 同じではない**: あちらは `section`/`subsection` を読み順で「最後に見たもの」1 件に上書きし
/// レベルを見ない。こちらはレベル宣言のあるものだけを対象に、階層として最大 4 件積む。
fn collect_section_path(
    seq: &[i64],
    by_id: &HashMap<i64, &LcirNode>,
    focus_pos: usize,
) -> Vec<ContextNode> {
    let mut out = Vec::new();
    let mut min_level = i64::MAX;
    let mut i = focus_pos;
    while i > 0 && out.len() < 4 {
        i -= 1;
        let Some(n) = by_id.get(&seq[i]) else { continue };
        let Some(level) = heading_level_of(n) else {
            continue;
        };
        if level < min_level {
            min_level = level;
            out.push(context_node(n, RELATED_TEXT_CAP));
        }
    }
    out.reverse();
    out
}

// ── 前提定義 ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn collect_premises(
    doc: &LcirDocument,
    by_id: &HashMap<i64, &LcirNode>,
    pos: &HashMap<i64, usize>,
    focus_pos: Option<usize>,
    origin_ids: &[i64],
    equations: &[Related],
    reference_targets: &[Related],
    opts: &ContextOptions,
) -> (Vec<Premise>, bool) {
    let mut out: Vec<Premise> = Vec::new();

    // 経路 1: 明示参照（辺の指し先が definition ノード）。最も確かだが実データでは希少。
    // **定義ノード単位で 1 件に畳む** — 焦点と続きブロックの両方が同じ "Definition 3.1" に
    // 言及すると `push_edge` の重複排除（from を含む三つ組）を素通りして 2 本残るので、
    // そのまま流すと同じ定義が 2 度並び `max_premises` の枠も 2 消費する。
    let mut seen_reference: HashSet<i64> = HashSet::new();
    for r in reference_targets {
        if r.node.kind != "definition" || !seen_reference.insert(r.node.node_id) {
            continue;
        }
        if let Some(n) = by_id.get(&r.node.node_id) {
            out.push(Premise {
                via: "reference",
                node: context_node(n, PREMISE_TEXT_CAP),
                symbol: None,
                relation: Some(r.clone()),
            });
        }
    }

    // 経路 2/3: 記号（TeX 版のみ・`symbols` が空なら何も出ない）。
    if !doc.symbols.is_empty() {
        // 記号の出現が記録されているノード（display 数式）と、本文照合の対象テキスト。
        let mut occurrence_scope: HashSet<i64> = origin_ids.iter().copied().collect();
        occurrence_scope.extend(equations.iter().map(|e| e.node.node_id));
        let mut haystack = String::new();
        for id in origin_ids {
            if let Some(n) = by_id.get(id) {
                if let Some(t) = &n.plain_text {
                    haystack.push_str(t);
                    haystack.push('\n');
                }
            }
        }
        // (via_rank, description あり, 定義位置) が大きいものを surface ごとに 1 件だけ残す。
        let mut best: HashMap<&str, (u8, bool, usize, Premise)> = HashMap::new();
        for s in &doc.symbols {
            let Some(def_id) = s.defined_at_node_id else {
                continue;
            };
            let Some(&def_pos) = pos.get(&def_id) else {
                continue;
            };
            // 前提は焦点より読み順で前にあること（後ろの定義は前提ではない）。
            if focus_pos.is_none_or(|fp| def_pos >= fp) {
                continue;
            }
            let Some(def_node) = by_id.get(&def_id) else {
                continue;
            };
            let via = if s
                .occurrences
                .iter()
                .any(|o| occurrence_scope.contains(&o.node_id))
            {
                Some(("occurrence", 2u8))
            } else if !s.surface_form.is_empty()
                && haystack.contains(&format!("${}$", s.surface_form))
            {
                Some(("symbol", 1u8))
            } else {
                None
            };
            let Some((via, rank)) = via else { continue };
            let has_desc = s.description.is_some();
            let key = s.surface_form.as_str();
            let candidate = (
                rank,
                has_desc,
                def_pos,
                Premise {
                    via,
                    node: context_node(def_node, PREMISE_TEXT_CAP),
                    symbol: Some(PremiseSymbol {
                        surface_form: s.surface_form.clone(),
                        normalized_form: s.normalized_form.clone(),
                        description: s.description.clone(),
                        symbol_type: s.symbol_type.clone(),
                        confidence: s.confidence,
                        origin: s.origin.clone(),
                    }),
                    relation: None,
                },
            );
            match best.get(key) {
                Some(prev) if (prev.0, prev.1, prev.2) >= (candidate.0, candidate.1, candidate.2) => {}
                _ => {
                    best.insert(key, candidate);
                }
            }
        }
        let mut symbol_premises: Vec<Premise> = best.into_values().map(|(_, _, _, p)| p).collect();
        symbol_premises.sort_by(|a, b| {
            let (sa, sb) = (
                a.symbol.as_ref().map(|s| s.surface_form.as_str()).unwrap_or(""),
                b.symbol.as_ref().map(|s| s.surface_form.as_str()).unwrap_or(""),
            );
            sa.cmp(sb).then(a.node.node_id.cmp(&b.node.node_id))
        });
        out.extend(symbol_premises);
    }

    let cut = out.len() > opts.max_premises;
    out.truncate(opts.max_premises);
    (out, cut)
}

// ── 変換ヘルパ ──────────────────────────────────────────────────────────────

fn context_node(n: &LcirNode, text_cap: usize) -> ContextNode {
    let (text, text_truncated) = match &n.plain_text {
        Some(t) if !t.is_empty() => {
            let (s, cut) = clip(t, text_cap);
            (Some(s), cut)
        }
        _ => (None, false),
    };
    let frag = n.source_fragments.first();
    let mut identifiers = serde_json::Map::new();
    if let Some(p) = n.payload.as_ref() {
        // `cite_key` は「読んで引用する」ツールの中核（TeX の bibliography_entry が持つ）。
        // `figure_index` は内部通番なので出さない。
        for key in [
            "theorem_number",
            "section_number",
            "note",
            "labels",
            "cite_key",
            "figure_number",
            "caption_number",
            "caption_label",
        ] {
            if let Some(v) = p.get(key) {
                identifiers.insert(key.to_string(), v.clone());
            }
        }
    }
    ContextNode {
        node_id: n.id,
        kind: n.kind.clone(),
        text,
        text_truncated,
        page: frag.map(|f| f.page),
        bbox: frag.map(|f| [f.bbox.x, f.bbox.y, f.bbox.width, f.bbox.height]),
        origin: n.origin.clone(),
        confidence: n.confidence,
        identifiers,
        math: n.math.clone(),
        alt_text: n.alt_text.clone(),
        assets: n.assets.clone(),
    }
}

/// char 単位で安全に切る（`llm::tools::document::relation_snippet` と同じ作法）。切ったら `true`。
fn clip(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    (s, true)
}

fn related(
    r: &crate::document_ir::LcirRelation,
    direction: &'static str,
    from_node_id: i64,
    node: &LcirNode,
    cap: usize,
) -> Related {
    Related {
        relation_type: r.relation_type.clone(),
        direction,
        from_node_id,
        confidence: r.confidence,
        origin: r.origin.clone(),
        metadata: r.metadata.clone(),
        node: context_node(node, cap),
    }
}

/// 図表参照を「辺の指し先 + 実体 + caption」に展開する。
///
/// `caption_of` の向きは **from=caption → to=figure/table** なので、caption から実体へは
/// **順方向**の 1 ホップ、実体から caption へは逆引きになる（`exec_get_figures` と同じ）。
fn float_ref(
    r: &crate::document_ir::LcirRelation,
    target: &LcirNode,
    by_id: &HashMap<i64, &LcirNode>,
    caption_to_entity: &HashMap<i64, i64>,
    entity_to_caption: &HashMap<i64, i64>,
) -> FloatRef {
    let (entity_id, caption_id) = if is_float_entity(&target.kind) {
        (Some(target.id), entity_to_caption.get(&target.id).copied())
    } else {
        (caption_to_entity.get(&target.id).copied(), Some(target.id))
    };
    let pick = |id: Option<i64>| {
        id.and_then(|i| by_id.get(&i))
            .map(|n| context_node(n, RELATED_TEXT_CAP))
    };
    FloatRef {
        relation_type: r.relation_type.clone(),
        from_node_id: r.from_node_id,
        confidence: r.confidence,
        origin: r.origin.clone(),
        resolved_via: r
            .metadata
            .as_ref()
            .and_then(|m| m.get("resolved_via"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        metadata: r.metadata.clone(),
        node: context_node(target, RELATED_TEXT_CAP),
        figure: pick(entity_id),
        caption: pick(caption_id),
    }
}

/// 決定的な並びに整えてから上限で切る。切ったら `true`。
fn cap_related(v: &mut Vec<Related>, max: usize) -> bool {
    v.sort_by_key(|a| (a.from_node_id, a.node.node_id));
    if v.len() > max {
        v.truncate(max);
        return true;
    }
    false
}

/// 注記の重複排除 + 決定的な並び。
#[derive(Default)]
struct NoteSet(std::collections::BTreeSet<ContextNoteCode>);

impl NoteSet {
    fn add(&mut self, code: ContextNoteCode) {
        self.0.insert(code);
    }
    fn finish(self) -> Vec<ContextNote> {
        self.0
            .into_iter()
            .map(|code| ContextNote {
                code,
                message: code.message(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document_ir::{
        BBox, LcirAsset, LcirFragment, LcirMath, LcirRelation, LcirSource, LcirSymbol,
        LcirSymbolOccurrence,
    };

    // ── フィクスチャ ────────────────────────────────────────────────────────

    fn node(id: i64, kind: &str, ordinal: i64, parent: Option<i64>, text: Option<&str>) -> LcirNode {
        LcirNode {
            id,
            kind: kind.to_string(),
            ordinal,
            parent_id: parent,
            plain_text: text.map(|s| s.to_string()),
            origin: Some("layout_model".to_string()),
            confidence: Some(0.6),
            payload: None,
            math: None,
            source_fragments: Vec::new(),
            assets: Vec::new(),
            alt_text: None,
        }
    }

    /// ページ上の矩形を 1 つ持たせる（PDF 版のふり）。
    fn with_region(mut n: LcirNode, page: i64) -> LcirNode {
        n.source_fragments = vec![LcirFragment {
            page,
            bbox: BBox::new(72.0, 500.0, 300.0, 12.0),
            fragment_type: Some("block".to_string()),
        }];
        n
    }

    fn with_payload(mut n: LcirNode, payload: serde_json::Value) -> LcirNode {
        n.payload = Some(payload);
        n
    }

    fn rel(from: i64, ty: &str, to: i64) -> LcirRelation {
        LcirRelation {
            from_node_id: from,
            relation_type: ty.to_string(),
            to_node_id: to,
            confidence: Some(0.6),
            origin: Some("layout_model".to_string()),
            metadata: None,
        }
    }

    fn doc(nodes: Vec<LcirNode>, relations: Vec<LcirRelation>) -> LcirDocument {
        LcirDocument {
            schema: "s".to_string(),
            schema_version: "0.1.0".to_string(),
            version_id: 7,
            content_key: "ck".to_string(),
            source: LcirSource {
                sha256: "sha".to_string(),
                mime_type: "application/pdf".to_string(),
                extractor_name: "lumencite-pdfium".to_string(),
                extractor_version: "0.7.0".to_string(),
            },
            coordinate_space: None,
            nodes,
            relations,
            symbols: Vec::new(),
        }
    }

    /// PDF 版の実データと同じ形の木を組む: `document > page > block`。
    /// ページ 1 に定理（10）→ 数式（11）→ 段落（12）、ページ 2 に段落（20）→ 節見出し（21）。
    /// 定理の主張がページをまたいで続く典型（実測で theorem の 33% がこの形）。
    fn pdf_two_pages() -> LcirDocument {
        doc(
            vec![
                node(1, "document", 0, None, None),
                node(2, "page", 0, Some(1), None),
                node(3, "page", 1, Some(1), None),
                with_region(
                    with_payload(
                        node(10, "theorem", 0, Some(2), Some("Theorem 2 (1D limit). Let a, b.")),
                        serde_json::json!({"theorem_number": "2", "note": "1D limit"}),
                    ),
                    1,
                ),
                with_region(node(11, "display_math", 1, Some(2), Some("rho = w f, (19)")), 1),
                with_region(node(12, "paragraph", 2, Some(2), Some("where the weight w is")), 1),
                with_region(node(20, "paragraph", 0, Some(3), Some("defined in (E8).")), 2),
                with_region(
                    with_payload(
                        node(21, "section", 1, Some(3), Some("3 Numerical results")),
                        serde_json::json!({"heading_level": 1, "section_number": "3"}),
                    ),
                    2,
                ),
            ],
            Vec::new(),
        )
    }

    fn ids(v: &[ContextNode]) -> Vec<i64> {
        v.iter().map(|n| n.node_id).collect()
    }

    fn has_note(c: &NodeContext, code: ContextNoteCode) -> bool {
        c.notes.iter().any(|n| n.code == code)
    }

    // ── 読み順とスパン（完了条件「ページ境界で文脈が切れない」） ────────────

    /// 定理の続きは page ノードをまたいで連結され、次の構造境界（section）の手前で止まる。
    #[test]
    fn continuation_crosses_page_boundary_and_stops_at_next_boundary() {
        let c = build_node_context(&pdf_two_pages(), 10, &ContextOptions::default()).unwrap();
        assert_eq!(
            ids(&c.continuation),
            vec![11, 12, 20],
            "ページ 1 の続き 2 件 + ページ 2 の 1 件。section(21) は境界なので入らない"
        );
        assert_eq!(c.continuation[2].page, Some(2), "ページ 2 のノードが載っている");
        assert!(!has_note(&c, ContextNoteCode::ContinuationTruncated));
    }

    /// 次の定理は別の論理単位なので続きに飲み込まない。
    #[test]
    fn continuation_stops_before_the_next_theorem() {
        let mut d = pdf_two_pages();
        d.nodes[6] = with_region(node(20, "lemma", 0, Some(3), Some("Lemma 3.")), 2);
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c.continuation), vec![11, 12]);
    }

    #[test]
    fn continuation_is_capped_by_block_count_and_reports_it() {
        let opts = ContextOptions {
            max_continuation: 1,
            ..Default::default()
        };
        let c = build_node_context(&pdf_two_pages(), 10, &opts).unwrap();
        assert_eq!(ids(&c.continuation), vec![11]);
        assert!(has_note(&c, ContextNoteCode::ContinuationTruncated));
    }

    #[test]
    fn continuation_is_capped_by_char_budget_and_reports_it() {
        let opts = ContextOptions {
            max_continuation_chars: 5,
            ..Default::default()
        };
        let c = build_node_context(&pdf_two_pages(), 10, &opts).unwrap();
        assert_eq!(ids(&c.continuation), vec![11]);
        assert!(c.continuation[0].text_truncated);
        assert!(has_note(&c, ContextNoteCode::ContinuationTruncated));
    }

    /// 前方向は最初の境界を**含めて**打ち切る（直前の見出し/定理まで見せて向き付けする）。
    #[test]
    fn before_includes_the_first_boundary_then_stops() {
        let c = build_node_context(&pdf_two_pages(), 20, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c.before), vec![11, 12], "12 は段落・11 は数式（境界ではない）");

        let c2 = build_node_context(&pdf_two_pages(), 12, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c2.before), vec![10, 11], "theorem(10) を含めてそこで止まる");
    }

    #[test]
    fn section_path_takes_enclosing_headings_in_root_order() {
        let d = doc(
            vec![
                node(1, "document", 0, None, None),
                with_payload(
                    node(2, "section", 0, Some(1), Some("2 Setup")),
                    serde_json::json!({"heading_level": 1}),
                ),
                with_payload(
                    node(3, "subsection", 1, Some(1), Some("2.1 Notation")),
                    serde_json::json!({"heading_level": 2}),
                ),
                node(4, "paragraph", 2, Some(1), Some("We write ...")),
            ],
            Vec::new(),
        );
        let c = build_node_context(&d, 4, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c.section_path), vec![2, 3], "root 側から順");
    }

    /// `heading_level` を宣言していない `heading` は見出しとして扱わない。実データでは
    /// 数式から切り出された断片（"bound" 等）が `heading` に落ちており、拾うと
    /// 「この定理を囲む節」として嘘を見せることになる。
    #[test]
    fn section_path_ignores_headings_without_a_declared_level() {
        let d = doc(
            vec![
                node(1, "document", 0, None, None),
                with_payload(
                    node(2, "section", 0, Some(1), Some("2 Setup")),
                    serde_json::json!({"heading_level": 1}),
                ),
                node(3, "heading", 1, Some(1), Some("bound")),
                node(4, "abstract", 2, Some(1), Some("We study ...")),
                node(5, "paragraph", 3, Some(1), Some("We write ...")),
            ],
            Vec::new(),
        );
        let c = build_node_context(&d, 5, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c.section_path), vec![2], "レベル宣言のある section だけ");
    }

    /// **回帰（レビュー high）**: レベル宣言の無い `heading` は境界にしない。実データの
    /// PDF `heading` の 94% は数式断片（"AiAj =" 等）で、境界に数えると定理の主張が
    /// その式の直前で切れる（しかも「境界で止めた」ので黙って切れる）。
    #[test]
    fn a_heading_without_a_declared_level_does_not_cut_the_statement() {
        let mut d = pdf_two_pages();
        // 定理の続きの式が `heading` に誤分類されたケース（レベル宣言なし）。
        d.nodes[4] = with_region(node(11, "heading", 1, Some(2), Some("AiAj =")), 1);
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c.continuation), vec![11, 12, 20], "式で切らずに続きを連結する");

        // レベル宣言のある heading は本物の見出しなので境界のまま。
        d.nodes[4] = with_region(
            with_payload(
                node(11, "heading", 1, Some(2), Some("2.1 Setup")),
                serde_json::json!({"heading_level": 2}),
            ),
            1,
        );
        let c2 = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert!(c2.continuation.is_empty());
        assert_eq!(c2.continuation_stopped_at.as_ref().unwrap().kind.as_deref(), Some("heading"));
    }

    /// **回帰（レビュー medium）**: 止めた理由を必ず返す。フロートの caption に
    /// 割り込まれただけなのか主張が終わったのかを、呼び出し側が区別できるようにする。
    #[test]
    fn continuation_reports_where_and_why_it_stopped() {
        let mut d = pdf_two_pages();
        d.nodes[4] = with_region(
            node(11, "figure_caption", 1, Some(2), Some("FIG. 1. A contour.")),
            1,
        );
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        let stop = c.continuation_stopped_at.as_ref().unwrap();
        assert_eq!(stop.reason, "boundary");
        assert_eq!(stop.node_id, Some(11));
        assert_eq!(stop.kind.as_deref(), Some("figure_caption"));
        assert!(c.continuation.is_empty());
        // 境界は「打ち切り」ではないので truncated 注記は出さない（理由は別フィールドで返る）。
        assert!(!has_note(&c, ContextNoteCode::ContinuationTruncated));

        let last = build_node_context(&d, 20, &ContextOptions::default()).unwrap();
        assert_eq!(
            last.continuation_stopped_at.as_ref().unwrap().reason,
            "boundary",
            "20 の次は section(21)"
        );
    }

    /// **回帰（レビュー medium）**: 本文を持たないノード（8a の figure はページ末尾の
    /// ordinal で入る）で `before` が埋まると向き付けの役に立たない。
    #[test]
    fn before_skips_nodes_without_text() {
        let mut d = pdf_two_pages();
        // ページ 1 の末尾に図領域が 2 件（実データと同じくブロックより後ろの ordinal）。
        d.nodes.push(with_region(node(80, "figure", 10, Some(2), None), 1));
        d.nodes.push(with_region(node(81, "figure", 11, Some(2), None), 1));
        let c = build_node_context(&d, 20, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c.before), vec![11, 12], "figure ではなく地の文が入る");
    }

    #[test]
    fn missing_focus_node_returns_none() {
        assert!(build_node_context(&pdf_two_pages(), 999, &ContextOptions::default()).is_none());
    }

    #[test]
    fn skeleton_focus_is_reported_and_yields_no_neighbours() {
        let c = build_node_context(&pdf_two_pages(), 2, &ContextOptions::default()).unwrap();
        assert!(c.before.is_empty() && c.continuation.is_empty());
        assert!(has_note(&c, ContextNoteCode::FocusIsNotAContentBlock));
    }

    // ── 辺の収集 ────────────────────────────────────────────────────────────

    /// PDF では定理の主張が割れており参照は**続きのブロック**に乗る。そこからも辺を拾う。
    #[test]
    fn relations_are_collected_from_the_continuation_too() {
        let mut d = pdf_two_pages();
        d.nodes.push(with_region(
            node(30, "display_math", 5, Some(3), Some("E = mc^2")),
            2,
        ));
        d.relations = vec![rel(12, "refers_to_equation", 30)];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.equations.len(), 1);
        assert_eq!(c.equations[0].from_node_id, 12, "焦点ではなく続きのブロックが参照元");
        assert_eq!(c.equations[0].node.node_id, 30);
    }

    /// `\eqref` は参照先の種別を見ないので `refers_to_equation` の相手が数式とは限らない。
    /// バンドルは相手の kind を無加工で載せる（数式と決めつけない）。
    #[test]
    fn equation_reference_keeps_the_real_target_kind() {
        let mut d = pdf_two_pages();
        d.relations = vec![rel(10, "refers_to_equation", 20)];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.equations[0].node.kind, "paragraph");
    }

    #[test]
    fn proofs_come_in_as_incoming_proves_edges() {
        let mut d = pdf_two_pages();
        d.nodes
            .push(with_region(node(40, "proof", 3, Some(3), Some("Proof. ...")), 2));
        d.relations = vec![rel(40, "proves", 10)];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(ids(&c.proofs.iter().map(|r| r.node.clone()).collect::<Vec<_>>()), vec![40]);
        assert_eq!(c.proofs[0].direction, "incoming");
        assert!(c.proves.is_empty());

        // 逆向き: 証明を焦点にすると proves 出辺になる。
        let c2 = build_node_context(&d, 40, &ContextOptions::default()).unwrap();
        assert_eq!(c2.proves[0].node.node_id, 10);
        assert_eq!(c2.proves[0].direction, "outgoing");
        assert!(!has_note(&c2, ContextNoteCode::ProvesTargetIsNotATheorem));
    }

    /// PDF の proves は 96% が読み順の隣接フォールバックで、実測 9% は remark/example/
    /// definition を指す。定理と偽らず注記する。
    #[test]
    fn proves_pointing_at_a_remark_is_flagged() {
        let mut d = pdf_two_pages();
        d.nodes[3] = with_region(node(10, "remark", 0, Some(2), Some("Remark.")), 1);
        d.nodes
            .push(with_region(node(40, "proof", 3, Some(3), Some("Proof.")), 2));
        d.relations = vec![rel(40, "proves", 10)];
        let c = build_node_context(&d, 40, &ContextOptions::default()).unwrap();
        assert_eq!(c.proves[0].node.kind, "remark");
        assert!(has_note(&c, ContextNoteCode::ProvesTargetIsNotATheorem));
    }

    #[test]
    fn citations_and_other_references_are_separated() {
        let mut d = pdf_two_pages();
        d.nodes.push(node(50, "bibliography_entry", 9, Some(3), Some("[1] Smith")));
        d.nodes.push(with_region(
            node(51, "lemma", 8, Some(3), Some("Lemma 1.")),
            2,
        ));
        d.relations = vec![rel(10, "cites", 50), rel(10, "refers_to_theorem", 51)];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.citations[0].node.node_id, 50);
        assert_eq!(c.references[0].node.node_id, 51);
    }

    #[test]
    fn related_lists_are_capped_and_reported() {
        let mut d = pdf_two_pages();
        for i in 0..5 {
            d.nodes
                .push(node(100 + i, "bibliography_entry", 9 + i, Some(3), Some("[x]")));
            d.relations.push(rel(10, "cites", 100 + i));
        }
        let opts = ContextOptions {
            max_related: 2,
            ..Default::default()
        };
        let c = build_node_context(&d, 10, &opts).unwrap();
        assert_eq!(c.citations.len(), 2);
        assert!(has_note(&c, ContextNoteCode::RelatedTruncated));
    }

    // ── 図表参照（caption_of の 1 ホップ） ──────────────────────────────────

    fn figure_node(id: i64) -> LcirNode {
        let mut n = with_region(
            with_payload(
                node(id, "figure", 20, Some(3), None),
                serde_json::json!({"figure_index": 1, "figure_number": "3"}),
            ),
            2,
        );
        n.assets = vec![LcirAsset {
            role: "page_crop".to_string(),
            mime_type: "image/png".to_string(),
            relative_path: "attachments/1/.lcir/fig.png".to_string(),
            width: Some(800),
            height: Some(600),
            size_bytes: Some(1234),
            sha256: "abc".to_string(),
            metadata: None,
        }];
        n
    }

    /// 辺が caption を指していても、`caption_of`（from=caption → to=figure）を**順方向**に
    /// 1 ホップ辿って実体（bbox・crop・alt text の持ち主）に到達する。
    #[test]
    fn float_reference_resolves_from_caption_to_the_region() {
        let mut d = pdf_two_pages();
        d.nodes.push(with_region(
            with_payload(
                node(60, "figure_caption", 10, Some(3), Some("Figure 3: architecture")),
                serde_json::json!({"caption_label": "Figure", "caption_number": "3"}),
            ),
            2,
        ));
        d.nodes.push(figure_node(61));
        let mut r = rel(10, "refers_to_figure", 60);
        r.metadata = Some(serde_json::json!({"ref": "Figure 3", "number": "3", "resolved_via": "caption"}));
        d.relations = vec![r, rel(60, "caption_of", 61)];

        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.figures.len(), 1);
        let f = &c.figures[0];
        assert_eq!(f.resolved_via.as_deref(), Some("caption"));
        assert_eq!(f.node.node_id, 60, "辺の指し先は無加工で載せる");
        assert_eq!(f.caption.as_ref().unwrap().node_id, 60);
        assert_eq!(f.figure.as_ref().unwrap().node_id, 61, "1 ホップで実体へ");
        assert!(f.figure.as_ref().unwrap().bbox.is_some());
        assert_eq!(f.figure.as_ref().unwrap().assets.len(), 1);
        assert!(!has_note(&c, ContextNoteCode::FloatEntityUnreachable));
    }

    /// 実測では caption の 3/4 に `caption_of` が無い。到達できないことを黙らない。
    #[test]
    fn float_reference_without_caption_of_is_flagged_unreachable() {
        let mut d = pdf_two_pages();
        d.nodes.push(with_region(
            node(60, "figure_caption", 10, Some(3), Some("Figure 3: architecture")),
            2,
        ));
        d.relations = vec![rel(10, "refers_to_figure", 60)];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert!(c.figures[0].figure.is_none());
        assert_eq!(c.figures[0].caption.as_ref().unwrap().node_id, 60);
        assert!(has_note(&c, ContextNoteCode::FloatEntityUnreachable));
    }

    /// 辺が実体を直接指しているときは、caption を**逆引き**で添える（get_figures と同じ向き）。
    #[test]
    fn float_reference_to_the_region_attaches_the_caption_backwards() {
        let mut d = pdf_two_pages();
        d.nodes.push(with_region(
            node(60, "figure_caption", 10, Some(3), Some("Figure 3: architecture")),
            2,
        ));
        d.nodes.push(figure_node(61));
        let mut r = rel(10, "refers_to_figure", 61);
        r.metadata = Some(serde_json::json!({"resolved_via": "node"}));
        d.relations = vec![r, rel(60, "caption_of", 61)];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.figures[0].figure.as_ref().unwrap().node_id, 61);
        assert_eq!(c.figures[0].caption.as_ref().unwrap().node_id, 60);
    }

    /// **回帰（レビュー medium）**: `get_figures` が返す figure ノード id をそのまま渡す
    /// 経路（ツール説明が案内している）。`caption_of` は from=caption → to=figure の
    /// 一方向なので、出辺だけ見ていると原文キャプションに到達できない。
    #[test]
    fn a_figure_entity_as_focus_still_reaches_its_caption() {
        let mut d = pdf_two_pages();
        d.nodes.push(with_region(
            with_payload(
                node(60, "figure_caption", 10, Some(3), Some("Figure 3: architecture")),
                serde_json::json!({"caption_label": "Figure", "caption_number": "3"}),
            ),
            2,
        ));
        d.nodes.push(figure_node(61));
        d.relations = vec![rel(60, "caption_of", 61)];

        let c = build_node_context(&d, 61, &ContextOptions::default()).unwrap();
        assert_eq!(c.focus.kind, "figure");
        assert_eq!(c.figures.len(), 1, "{:?}", c.figures);
        assert_eq!(c.figures[0].caption.as_ref().unwrap().node_id, 60);
        assert_eq!(
            c.figures[0].caption.as_ref().unwrap().text.as_deref(),
            Some("Figure 3: architecture")
        );
        assert_eq!(c.figures[0].figure.as_ref().unwrap().node_id, 61);
    }

    // ── 前提定義 ────────────────────────────────────────────────────────────

    fn tex_doc_with_symbols(symbols: Vec<LcirSymbol>) -> LcirDocument {
        // TeX 版は完全にフラット（全ブロックが document 直下）で座標を持たない。
        let mut d = doc(
            vec![
                node(1, "document", 0, None, None),
                node(2, "definition", 0, Some(1), Some("Let $U$ be the evolution operator.")),
                node(3, "paragraph", 1, Some(1), Some("We now state the result.")),
                node(4, "theorem", 2, Some(1), Some("The operator $U$ is unitary.")),
                node(5, "definition", 3, Some(1), Some("Let $V$ be the shift.")),
            ],
            Vec::new(),
        );
        d.source.extractor_name = "lumencite-tex".to_string();
        d.symbols = symbols;
        d
    }

    fn symbol(id: i64, surface: &str, defined_at: i64, description: Option<&str>) -> LcirSymbol {
        LcirSymbol {
            id,
            surface_form: surface.to_string(),
            normalized_form: None,
            description: description.map(|s| s.to_string()),
            symbol_type: None,
            defined_at_node_id: Some(defined_at),
            scope_node_id: None,
            confidence: Some(0.6),
            origin: Some("tex_source".to_string()),
            occurrences: Vec::new(),
        }
    }

    /// 定理本文に `$U$` が出て、`U` の定義が読み順で**前**にある → 前提定義として拾う。
    /// 後ろで定義される `$V$` は前提ではないので拾わない。
    #[test]
    fn premise_via_symbol_requires_a_definition_earlier_in_reading_order() {
        let d = tex_doc_with_symbols(vec![
            symbol(1, "U", 2, Some("the evolution operator")),
            symbol(2, "V", 5, Some("the shift")),
        ]);
        let c = build_node_context(&d, 4, &ContextOptions::default()).unwrap();
        assert_eq!(c.premises.len(), 1, "{:?}", c.premises);
        assert_eq!(c.premises[0].via, "symbol");
        assert_eq!(c.premises[0].node.node_id, 2);
        assert_eq!(
            c.premises[0].symbol.as_ref().unwrap().description.as_deref(),
            Some("the evolution operator")
        );
    }

    /// 記録済みの出現（6b）は表層照合より確かなので優先し、`via` で区別できるようにする。
    #[test]
    fn premise_via_occurrence_wins_over_surface_matching() {
        let mut d = tex_doc_with_symbols(vec![symbol(1, "U", 2, None)]);
        d.symbols[0].occurrences = vec![LcirSymbolOccurrence {
            node_id: 4,
            surface_form: "U".to_string(),
            confidence: Some(0.5),
            origin: Some("tex_source".to_string()),
        }];
        let c = build_node_context(&d, 4, &ContextOptions::default()).unwrap();
        assert_eq!(c.premises[0].via, "occurrence");
    }

    /// 定理本文に現れない記号は載せない（記号一覧ではなく「この定理の前提」なので）。
    #[test]
    fn premise_skips_symbols_absent_from_the_text() {
        let d = tex_doc_with_symbols(vec![symbol(1, "W", 2, Some("unused"))]);
        let c = build_node_context(&d, 4, &ContextOptions::default()).unwrap();
        assert!(c.premises.is_empty(), "{:?}", c.premises);
    }

    /// 明示参照（辺の指し先が definition）は最も確かな経路。
    #[test]
    fn premise_via_explicit_reference_to_a_definition() {
        let mut d = pdf_two_pages();
        d.nodes.push(with_region(
            node(70, "definition", 7, Some(3), Some("Definition 1. A graph is ...")),
            2,
        ));
        d.relations = vec![rel(10, "refers_to_theorem", 70)];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.premises.len(), 1);
        assert_eq!(c.premises[0].via, "reference");
        assert_eq!(c.premises[0].node.node_id, 70);
        assert!(c.premises[0].relation.is_some());
        assert_eq!(c.references.len(), 1, "参照リストにも残る（重複ではなく別の見方）");
    }

    /// **回帰（レビュー low）**: 焦点と続きブロックの両方が同じ "Definition 3.1" に
    /// 言及すると `push_edge` の重複排除（from を含む三つ組）を素通りして辺が 2 本残る。
    /// 定義ノード単位で 1 件に畳む。
    #[test]
    fn the_same_definition_is_not_listed_twice_as_a_premise() {
        let mut d = pdf_two_pages();
        d.nodes.push(with_region(
            node(70, "definition", 7, Some(3), Some("Definition 1. A graph is ...")),
            2,
        ));
        // 焦点（10）と続き（12）の両方から同じ定義へ。
        d.relations = vec![
            rel(10, "refers_to_theorem", 70),
            rel(12, "refers_to_theorem", 70),
        ];
        let c = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.premises.len(), 1, "{:?}", c.premises);
        assert_eq!(c.references.len(), 2, "参照一覧の方は 2 本のまま（辺は 2 本ある）");
    }

    /// **回帰（レビュー low）**: 最も確かな明示参照の前提定義が、無関係な
    /// `max_related` の打ち切りで消えてはいけない。
    #[test]
    fn reference_premises_survive_the_related_cap() {
        let mut d = pdf_two_pages();
        for i in 0..4 {
            d.nodes.push(node(100 + i, "bibliography_entry", 20 + i, Some(3), Some("[x]")));
            d.relations.push(rel(10, "refers_to_theorem", 100 + i));
        }
        d.nodes.push(with_region(
            node(70, "definition", 30, Some(3), Some("Definition 1.")),
            2,
        ));
        d.relations.push(rel(10, "refers_to_theorem", 70));
        let opts = ContextOptions {
            max_related: 2,
            ..Default::default()
        };
        let c = build_node_context(&d, 10, &opts).unwrap();
        assert_eq!(c.references.len(), 2, "参照一覧は上限どおり");
        assert_eq!(c.premises.len(), 1, "定義は打ち切りに巻き込まれない: {:?}", c.premises);
        assert_eq!(c.premises[0].node.node_id, 70);
    }

    /// **回帰（レビュー medium）**: premises の打ち切りも注記する（他の 6 リストと対称）。
    #[test]
    fn truncating_premises_is_reported() {
        let d = tex_doc_with_symbols(vec![
            symbol(1, "U", 2, Some("the evolution operator")),
            symbol(2, "W", 2, Some("another")),
        ]);
        let mut d = d;
        d.nodes[3].plain_text = Some("The operators $U$ and $W$ are unitary.".to_string());
        let opts = ContextOptions {
            max_premises: 1,
            ..Default::default()
        };
        let c = build_node_context(&d, 4, &opts).unwrap();
        assert_eq!(c.premises.len(), 1);
        assert!(has_note(&c, ContextNoteCode::PremisesTruncated));
    }

    /// **回帰（レビュー medium）**: 予算は本文だけでなく原文 LaTeX も数える。
    /// TeX の display 数式は plain_text とは別に `math.latex` を持つので、本文長だけで
    /// 測ると応答が入力依存で膨らむ。
    #[test]
    fn the_continuation_budget_counts_latex_too() {
        let mut d = pdf_two_pages();
        d.nodes[4].plain_text = Some("x".to_string());
        d.nodes[4].math = Some(LcirMath {
            display_mode: "display".to_string(),
            equation_label: None,
            latex: Some("y".repeat(500)),
            presentation_mathml: None,
            content_mathml: None,
            openmath: None,
            normalized_text: None,
            semantic_status: "source_provided".to_string(),
            confidence: None,
            origin: Some("tex_source".to_string()),
        });
        let opts = ContextOptions {
            max_continuation_chars: 100,
            ..Default::default()
        };
        let c = build_node_context(&d, 10, &opts).unwrap();
        assert_eq!(ids(&c.continuation), vec![11], "LaTeX 500 字で予算を使い切る");
        assert!(has_note(&c, ContextNoteCode::ContinuationTruncated));
    }

    /// **回帰（レビュー low）**: cite key は「読んで引用する」ツールの中核。
    #[test]
    fn identifiers_carry_the_cite_key() {
        let d = doc(
            vec![
                node(1, "document", 0, None, None),
                with_payload(
                    node(2, "bibliography_entry", 0, Some(1), Some("[1] Smith 2020")),
                    serde_json::json!({"cite_key": "smith2020"}),
                ),
            ],
            Vec::new(),
        );
        let c = build_node_context(&d, 2, &ContextOptions::default()).unwrap();
        assert_eq!(c.focus.identifiers["cite_key"], "smith2020");
    }

    // ── provenance と注記 ───────────────────────────────────────────────────

    #[test]
    fn tex_bundle_has_no_regions() {
        let d = tex_doc_with_symbols(Vec::new());
        let c = build_node_context(&d, 4, &ContextOptions::default()).unwrap();
        assert!(c.focus.page.is_none() && c.focus.bbox.is_none());
    }

    /// 規約②: 表現ごとに必ず真になる事実（TeX に座標が無い / PDF に記号が無い）は
    /// notes に載せない。`notes` は「**今回**届かなかったもの」に保つ。
    #[test]
    fn notes_are_empty_when_nothing_actually_went_wrong() {
        let pdf = build_node_context(&pdf_two_pages(), 10, &ContextOptions::default()).unwrap();
        assert!(pdf.notes.is_empty(), "{:?}", pdf.notes);
        let tex = tex_doc_with_symbols(Vec::new());
        let tex = build_node_context(&tex, 4, &ContextOptions::default()).unwrap();
        assert!(tex.notes.is_empty(), "{:?}", tex.notes);
    }

    /// 完了条件「AI 推定部分を回答中で識別できる」— origin/confidence をノードごとに透過する。
    #[test]
    fn every_node_carries_origin_and_confidence() {
        let c = build_node_context(&pdf_two_pages(), 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.focus.origin.as_deref(), Some("layout_model"));
        assert_eq!(c.focus.confidence, Some(0.6));
        assert!(c.continuation.iter().all(|n| n.origin.is_some()));
    }

    #[test]
    fn focus_identifiers_come_from_payload() {
        let c = build_node_context(&pdf_two_pages(), 10, &ContextOptions::default()).unwrap();
        assert_eq!(c.focus.identifiers["theorem_number"], "2");
        assert_eq!(c.focus.identifiers["note"], "1D limit");
    }

    #[test]
    fn focus_math_is_passed_through_verbatim() {
        let mut d = pdf_two_pages();
        d.nodes[4].math = Some(LcirMath {
            display_mode: "display".to_string(),
            equation_label: Some("(19)".to_string()),
            latex: None,
            presentation_mathml: None,
            content_mathml: None,
            openmath: None,
            normalized_text: Some("rho = w f".to_string()),
            semantic_status: "surface_only".to_string(),
            confidence: Some(0.6),
            origin: Some("pdf_text_layer".to_string()),
        });
        let c = build_node_context(&d, 11, &ContextOptions::default()).unwrap();
        let m = c.focus.math.as_ref().unwrap();
        assert_eq!(m.semantic_status, "surface_only");
        assert!(m.latex.is_none(), "PDF 版に LaTeX は無い（表層のみ）");
        assert_eq!(m.normalized_text.as_deref(), Some("rho = w f"));
    }

    #[test]
    fn focus_text_is_clipped_at_the_cap() {
        let long = "x".repeat(FOCUS_TEXT_CAP + 50);
        let d = doc(
            vec![
                node(1, "document", 0, None, None),
                node(2, "paragraph", 0, Some(1), Some(&long)),
            ],
            Vec::new(),
        );
        let c = build_node_context(&d, 2, &ContextOptions::default()).unwrap();
        assert!(c.focus.text_truncated);
        assert_eq!(c.focus.text.as_ref().unwrap().chars().count(), FOCUS_TEXT_CAP + 1);
    }

    // ── 決定性・頑健性 ──────────────────────────────────────────────────────

    /// ノードや辺の入力順に結果が依存しない（HashMap の反復順を漏らさない）。
    #[test]
    fn output_is_independent_of_input_order() {
        let mut d = pdf_two_pages();
        d.nodes.push(node(50, "bibliography_entry", 9, Some(3), Some("[1]")));
        d.nodes.push(node(51, "bibliography_entry", 10, Some(3), Some("[2]")));
        d.relations = vec![rel(10, "cites", 51), rel(10, "cites", 50)];
        let a = build_node_context(&d, 10, &ContextOptions::default()).unwrap();

        d.nodes.reverse();
        d.relations.reverse();
        let b = build_node_context(&d, 10, &ContextOptions::default()).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            a.citations.iter().map(|r| r.node.node_id).collect::<Vec<_>>(),
            vec![50, 51]
        );
    }

    /// 壊れた木（親子の循環）でも停止し、焦点は列から消えない。
    #[test]
    fn cyclic_parent_links_do_not_hang() {
        let d = doc(
            vec![
                node(1, "paragraph", 0, Some(2), Some("a")),
                node(2, "paragraph", 0, Some(1), Some("b")),
                node(3, "paragraph", 1, None, Some("c")),
            ],
            Vec::new(),
        );
        let c = build_node_context(&d, 1, &ContextOptions::default()).unwrap();
        assert_eq!(c.focus.node_id, 1);
    }

    #[test]
    fn serialized_bundle_omits_empty_lists() {
        let d = doc(
            vec![
                node(1, "document", 0, None, None),
                node(2, "paragraph", 0, Some(1), Some("only block")),
            ],
            Vec::new(),
        );
        let c = build_node_context(&d, 2, &ContextOptions::default()).unwrap();
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("proofs").is_none() && v.get("equations").is_none());
        assert_eq!(v["focus"]["kind"], "paragraph");
        assert!(v["focus"].get("text_truncated").is_none(), "false は出さない");
    }
}
