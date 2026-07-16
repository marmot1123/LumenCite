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
pub const EXTRACTOR_VERSION: &str = "0.2.0";
