//! LCIR スキーマ定数。抽出ロジックの identity と再現性の基準になる。

/// LCIR JSON の schema URI（export/交換用の識別子）。
pub const SCHEMA_URI: &str = "https://lumencite.dev/schema/document-ir/0.1";

/// LCIR スキーマバージョン（破壊的変更で上げる）。
pub const SCHEMA_VERSION: &str = "0.1.0";

/// PDF 抽出器の名前（provenance の extractor_name）。
pub const EXTRACTOR_NAME: &str = "lumencite-pdfium";

/// PDF 抽出**ロジック**の semver。pdfium クレート版とは別に、抽出結果を左右する我々の
/// ロジックが変わったら手で上げる。content_key と supersede 判定の基準になる。
pub const EXTRACTOR_VERSION: &str = "0.1.0";
