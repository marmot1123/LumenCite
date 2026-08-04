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
/// - `0.7.0`: Phase 8d-7。本文の "Figure 3"/"Fig. 3"/"Table 2" を図表番号と照合して
///   `refers_to_figure`/`refers_to_table` 辺を張る（実体 `figure` ノード優先・無ければ
///   `figure_caption`/`table_caption` に解決し `metadata.resolved_via` で区別）。
///   出力（派生の関係辺）が増えるので、既存コーパスは `rebuild_outdated_lcir` で
///   張り直せるよう版を上げる（**再構築は必須ではない。辺は次回再構築時に付く**）。
/// - `0.8.0`: debt-12。全大文字ラベル + ローマ数字の caption（"TABLE III." 形）を
///   `table_caption`/`figure_caption` に分類する。実ライブラリで 48 ブロック / 12 版が
///   `paragraph` 等に落ちていた。分類（`node_kind`）が変わるので既存コーパスは
///   `rebuild_outdated_lcir` で張り直せるよう版を上げる。
/// - `0.9.0`: debt-14。図領域のクランプ範囲を `[0, 幅] × [0, 高さ]` からページ境界 box
///   （`CropBox ∩ MediaBox`）へ直し、その原点を生 CropBox ではなく交差から取る。
///   原点が非ゼロの PDF で図が切り落とされ / 丸ごと消え / 逆に box 外の画像を拾っていた。
///   図領域（`figure` ノードの bbox と crop PNG）が変わるので旧版は supersede される。
/// - `0.10.0`: debt-18。走り柱（ランニングヘッダ/フッタ）判定の帯を、ページ寸法の 10%/90% から
///   **ページ境界 box の原点 + 10%/90%** に直す。原点が非ゼロの PDF で帯がずれ、本文の短い行が
///   `paragraph` → `unknown_block` に降格し、逆に box 下端すぐ上の走り柱が段落として残っていた。
///   分類（`node_kind`）が変わるので旧版は supersede される。
/// - `0.11.0`: Phase 8d-8。**XObjectForm 内の Image** も図領域の候補にする。`\includegraphics` の
///   図が form に包まれる PDF では、トップレベル列挙だけだと図が 1 枚も見つからない。
///   子の bbox は form のコンテンツ空間で返るので、座標空間を仮説で決めず form ごとに
///   自己校正して（そのまま / 合成行列を当てた の 2 通りを form 自身の bounds への包含率で比較）
///   ページ空間へ移す。図領域が増えるので旧版は supersede される
///   （実測: 生存 138 版で 1,202 → 1,248 領域・既存領域の移動と消滅は 0）。
/// - `0.12.0`: Phase 8d-2。**ベクター図（tikz/pgf）の path クラスタ**も図領域の候補にする。
///   探索するのは「同一ページに図 caption があり、そのうちラスタ図と結ばれなかったものが残る」
///   ページだけで、採るのはその余った caption と相互最近でペアになったクラスタだけ
///   （caption を持たないベクター図は取り逃す ── 本文の罫線クラスタと区別する手段が無い）。
///   ラスタ側の領域列・crop ファイル名・caption ペアには一切触らない（ベクターは後ろに足し、
///   crop は `vec-` プレフィクスで別採番し、caption は 2 段でペアリングする）。
///   図領域が増えるので旧版は supersede される（実測: 生存 138 版で 1,248 → 1,628 領域・
///   図 caption とのペア 281 → 661・既存領域の移動と消滅は 0）。
/// - `0.13.0`: ゲート ②a の指摘。**XObjectForm の子 path にもクリップを掛ける**。8d-2 は
///   トップレベル path にだけクリップを交差させており、`\includegraphics{*.pdf}` の内側にある
///   axes クリップ付きの巨大 path が生 bbox のまま可視インクの hull に入っていた
///   （実測: 純ベクター form 171 個・クリップ付きの子 2,872 件。hull が縮む form が 73 個あり、
///   最悪は 43,462×18,575pt → 216.7×119.3pt ＝ クランプの 50% ルールで**図が丸ごと消えていた**）。
///   ベクター図領域が変わるので旧版は supersede される。
/// - `0.14.0`: debt-22。**page ノードの `plain_text` から C0 制御文字を落として保存する**
///   （`\n` / `\t` 以外の U+0000..U+001F を除去し `\r\n` / `\r` を `\n` に寄せる）。
///   pdfium はマップできない数式グリフを `\u{2}` 等で吐き、それが**語の内側に刺さる**ため
///   FTS5 の trigram 索引で語が割れ、9a の JSON export と `get_node_context` の page-focus では
///   生のまま外に出ていた（実測: 非空 5,803 ページの 78.8% に C0・`\r` は 5,786 ページ）。
///   保存値が変わるので旧版は supersede される。**改行は潰さない**
///   （`normalize_ws` を流用すると `get_fulltext` の本文が 1 行の塊になる）。
pub const EXTRACTOR_VERSION: &str = "0.14.0";

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
/// - `0.5.0`: Phase 8b。`tabular`/`tabular*`/`tabularx` をセル構造化して `table` ノード
///   （payload に rows/alignments/原文スニペット）+ table_caption との `caption_of` 辺を作る。
///   出力（ノード・辺）が増えるので旧版は `rebuild_outdated_lcir` で張り直せるよう版を上げる。
pub const TEX_EXTRACTOR_VERSION: &str = "0.5.0";

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
