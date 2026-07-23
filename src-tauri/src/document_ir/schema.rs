//! LCIR スキーマ定数。抽出ロジックの identity と再現性の基準になる。

/// LCIR JSON の schema URI（export/交換用の識別子）。
pub const SCHEMA_URI: &str = "https://lumencite.dev/schema/document-ir/0.1";

/// LCIR スキーマバージョン（破壊的変更で上げる）。
pub const SCHEMA_VERSION: &str = "0.1.0";

/// PDF 抽出器の名前（provenance の extractor_name）。
pub const EXTRACTOR_NAME: &str = "lumencite-pdfium";

/// PDF 抽出**ロジック**の semver。pdfium クレート版とは別に、抽出結果を左右する我々の
/// ロジックが変わったら手で上げる。content_key と supersede 判定の基準になる。
///
/// - `0.1.0`: Phase 1。page + text_block(セグメント) の平坦木。
/// - `0.2.0`: Phase 2。論理構造認識で `page > block(段落/見出し/caption 等) > line` の木にする
///   （`ingestion::structure`）。出力が変わるので旧 0.1.0 版は再構築時に supersede される。
/// - `0.3.0`: Phase 3。display 数式を認識して `display_math` ノード + `math_expressions`(表層)を
///   作り、制御文字を除去する。出力が変わるので旧版は再構築時に supersede される。
/// - `0.4.0`: Phase 5。行頭キーワードから定理・定義・証明ブロック
///   （`theorem`/`lemma`/`proposition`/`corollary`/`definition`/`remark`/`example`/`proof`）を
///   信頼度付きで認識し、番号・付記名を payload に載せる。出力が変わるので旧版は supersede される。
/// - `0.5.0`: Phase 6a。本文の "Theorem 2.3"/"Eq. (2.1)" を定理番号/数式番号と照合して参照グラフ
///   （`node_relations`・refers_to_*・proof→theorem の proves）を張る。抽出出力（派生の関係辺）が
///   増えるので、既存コーパスは `rebuild_outdated_lcir` で張り直せるよう版を上げる。
/// - `0.6.0`: Phase 8a。埋込画像（トップレベル Image オブジェクト）から図領域を検出して
///   `figure` ノード + ページ crop PNG アセット（`assets`/`node_assets`）+ `caption_of` 辺を
///   作り、caption の payload にラベル語・番号を載せる。出力が変わるので旧版は supersede される。
pub const EXTRACTOR_VERSION: &str = "0.6.0";

/// TeX 抽出器の名前（Phase 4・arXiv TeX ソース）。pdfium 版と**別 `document_version` として併存**
/// する（ADR #8）。supersede・rebuild 判定は抽出器ごとに独立。
pub const TEX_EXTRACTOR_NAME: &str = "lumencite-tex";

/// TeX 抽出**ロジック**の semver（pdfium 側とは独立採番）。
///
/// - `0.1.0`: Phase 4。gzip/tar のメモリ内展開・`\input` 解決・構造認識
///   （front_matter/abstract/節/段落/display 数式=生 LaTeX/caption/list/code/thebibliography）。
/// - `0.2.0`: Phase 5。定理系環境（標準名 + preamble の `\newtheorem` 宣言）と `proof` を型付き
///   ノードにし、`[note]`・`\label` を捕捉する。出力が変わるので旧版は再構築時に supersede される。
/// - `0.3.0`: Phase 6a。本文に原文のまま残る `\ref`/`\eqref`/`\cite` を `\label`/cite key と照合して
///   参照グラフ（`node_relations`）を張る（proof→theorem の proves も）。出力（関係辺）が増えるので
///   旧版は `rebuild_outdated_lcir` で張り直せるよう版を上げる。
/// - `0.4.0`: Phase 6b。定義文（"let $U$ be ...", "$H := ...$"）からインライン数式を記号として抽出し
///   `symbols`/`symbol_occurrences` を作る。出力（記号）が増えるので旧版は張り直せるよう版を上げる。
pub const TEX_EXTRACTOR_VERSION: &str = "0.4.0";

/// read 面で複数表現からどれを既定採用するかの優先度（大きいほど優先）。
/// 原資料に近い TeX（生 LaTeX・原文構造）を PDF 抽出（推定構造・表層数式）より優先する。
/// 未知の抽出器は 0（併存はするが既定では選ばれない）。
pub fn extractor_priority(name: &str) -> i64 {
    match name {
        TEX_EXTRACTOR_NAME => 2,
        EXTRACTOR_NAME => 1,
        _ => 0,
    }
}
