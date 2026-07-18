use std::collections::HashMap;

/// 著者マスタ。v0.3.0 で多言語名・読み仮名・団体著者・CSL 互換フィールドへ拡張。
/// `identifiers` は別テーブル `author_identifiers` を JOIN して詰める用の付属フィールドで、
/// 列としては存在しないため `#[sqlx(default)]` で FromRow から除外する（M3 で組み立て）。
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct Author {
    pub id: i64,
    pub name: String,
    pub given_name: Option<String>,
    pub middle_name: Option<String>,
    pub family_name: Option<String>,
    pub suffix: Option<String>,
    pub name_particle: Option<String>,

    pub name_original: Option<String>,
    pub given_name_original: Option<String>,
    pub family_name_original: Option<String>,
    pub original_script: Option<String>,

    pub reading_family: Option<String>,
    pub reading_given: Option<String>,

    pub is_organization: bool,

    pub email: Option<String>,
    pub homepage_url: Option<String>,
    pub notes: Option<String>,

    pub orcid: Option<String>,
    pub updated_at: Option<String>,

    #[sqlx(skip)]
    pub identifiers: Vec<AuthorIdentifier>,
}

/// 著者の外部識別子（ORCID 以外は `author_identifiers` に正規化保持）。
/// scheme は 'orcid' / 'scopus' / 'dblp' / 'semantic_scholar' / 'wikidata' /
/// 'isni' / 'viaf' / 'researcher_id' / 'google_scholar' 等。
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone, PartialEq, Eq)]
pub struct AuthorIdentifier {
    pub author_id: i64,
    pub scheme: String,
    pub value: String,
    pub url: Option<String>,
}

/// 著者の新規作成 / 更新時に受け取る入力型。
/// `get_or_create_author`（M3 で配線済み） / `update_author`（M7） の引数として使う。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AuthorInput {
    pub name: String,
    #[serde(default)]
    pub given_name: Option<String>,
    #[serde(default)]
    pub middle_name: Option<String>,
    #[serde(default)]
    pub family_name: Option<String>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub name_particle: Option<String>,

    #[serde(default)]
    pub name_original: Option<String>,
    #[serde(default)]
    pub given_name_original: Option<String>,
    #[serde(default)]
    pub family_name_original: Option<String>,
    #[serde(default)]
    pub original_script: Option<String>,

    #[serde(default)]
    pub reading_family: Option<String>,
    #[serde(default)]
    pub reading_given: Option<String>,

    #[serde(default)]
    pub is_organization: bool,

    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub homepage_url: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,

    #[serde(default)]
    pub orcid: Option<String>,

    #[serde(default)]
    pub identifiers: Vec<AuthorIdentifierInput>,
}

#[allow(dead_code)] // M3 で配線
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthorIdentifierInput {
    pub scheme: String,
    pub value: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct Attachment {
    pub id: i64,
    pub entry_id: i64,
    pub file_name: String,
    pub mime_type: String,
    pub created_at: String,
}

// ---- LCIR (LumenCite Document Intermediate Representation) の行 DTO（migration 0014） ----

/// 添付ごとの抽出/変換結果 1 回分（provenance の正本）。
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct DocumentVersion {
    pub id: i64,
    pub attachment_id: i64,
    pub content_key: String,
    pub schema_version: String,
    pub source_sha256: String,
    pub source_mime_type: String,
    pub extractor_name: String,
    pub extractor_version: String,
    pub config_hash: String,
    pub parent_version_id: Option<i64>,
    pub extraction_status: String,
    pub warnings_json: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

/// 文書の型付きノード。`node_kind` は `document_ir::NodeKind` の snake_case。
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct DocumentNode {
    pub id: i64,
    pub document_version_id: i64,
    pub parent_id: Option<i64>,
    pub node_kind: String,
    pub ordinal: i64,
    pub plain_text: Option<String>,
    pub language: Option<String>,
    pub confidence: Option<f64>,
    pub origin: Option<String>,
    pub payload_json: Option<String>,
    pub created_at: String,
}

/// ノード ↔ PDF 領域。座標は `highlights` と同一系（PDF pt・左下原点）。
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct SourceFragment {
    pub id: i64,
    pub node_id: i64,
    pub page_number: i64,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    pub reading_order: Option<i64>,
    pub fragment_type: Option<String>,
}

/// 数式の複数表現（migration 0016・Phase 3）。inline_math/display_math ノードに 1:1 で付く。
#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct MathExpression {
    pub id: i64,
    pub node_id: i64,
    pub display_mode: String,
    pub equation_label: Option<String>,
    pub latex: Option<String>,
    pub presentation_mathml: Option<String>,
    pub content_mathml: Option<String>,
    pub openmath_json: Option<String>,
    pub normalized_text: Option<String>,
    pub ast_json: Option<String>,
    pub semantic_status: String,
    pub confidence: Option<f64>,
    pub origin: Option<String>,
    pub created_at: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct EntrySummary {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub entry_type: String,
    pub authors: Vec<Author>,
    pub tags: Vec<Tag>,
    pub has_attachment: bool,
    pub created_at: String,
    pub journal: Option<String>,
    pub starred: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EntryRelation {
    pub entry: EntrySummary,
    pub relation_type: String,
    pub direction: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct EntryDetail {
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub entry_type: String,
    pub citation_key: Option<String>,
    pub doi: Option<String>,
    pub isbn: Option<String>,
    pub arxiv_id: Option<String>,
    pub url: Option<String>,
    pub abstract_: Option<String>,
    pub notes: Option<String>,
    pub summary: Option<String>,
    pub summary_model: Option<String>,
    pub summary_generated_at: Option<String>,
    pub authors: Vec<Author>,
    pub tags: Vec<Tag>,
    pub has_attachment: bool,
    pub created_at: String,
    pub starred: bool,
    pub deleted_at: Option<String>,
    pub extra_fields: HashMap<String, String>,
    pub attachments: Vec<Attachment>,
    pub relations: Vec<EntryRelation>,
    pub collections: Vec<Collection>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct ImportResult {
    pub imported: i64,
    pub skipped: i64,
    pub errors: Vec<String>,
}

/// サイドバー各行に表示する件数のスナップショット。
/// total / starred / unfiled はゴミ箱を除外した数。trash はゴミ箱内の件数。
/// `collections` / `tags` は id -> 件数（いずれもゴミ箱を除外）。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct SidebarCounts {
    pub total: i64,
    pub starred: i64,
    pub unfiled: i64,
    pub trash: i64,
    pub collections: HashMap<i64, i64>,
    pub tags: HashMap<i64, i64>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct FulltextHit {
    pub entry: EntrySummary,
    pub attachment_id: i64,
    pub page: i64,
    pub snippet: String,
}

/// LCIR ノード単位 FTS（Phase 2）のヒット。ページ粒度の `FulltextHit` と違い、段落・見出し・
/// caption 等の**ブロック粒度**で当たり、`node_kind` と（あれば）PDF 上の領域 `bbox` を返すので
/// 「検索ヒット → PDF 該当ブロックをハイライト」に直結できる。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct NodeFtsHit {
    pub entry: EntrySummary,
    pub attachment_id: i64,
    pub node_id: i64,
    pub page: i64,
    pub node_kind: String,
    pub snippet: String,
    /// ブロックの統合領域（PDF user space・左下原点・pt）。fragment が無ければ None。
    pub bbox: Option<crate::document_ir::BBox>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub children: Vec<Collection>,
}

/// 複合タグフィルタの結合方法。`Or` = いずれかのタグを含む / `And` = すべてのタグを含む。
/// serde は小文字（"or" / "and"）で受ける。既定は `Or`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagMatch {
    #[default]
    Or,
    And,
}

/// 一覧の複合フィルタ（v0.6.0）。scope（collection/tag/view）や検索クエリと AND で合成される。
/// 全フィールドが空（`is_empty()` が true）なら無制約で、従来の挙動と一致する。
/// ネストオブジェクトなのでキーは serde 定義どおり snake_case で受ける。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct EntryFilter {
    /// 種別。非空なら `entry_type IN (...)`（要素どうしは OR）。
    #[serde(default)]
    pub entry_types: Vec<String>,
    /// `year >= year_min`。
    #[serde(default)]
    pub year_min: Option<i64>,
    /// `year <= year_max`。
    #[serde(default)]
    pub year_max: Option<i64>,
    /// `Some(true)` = star 付きのみ / `Some(false)` = star なしのみ / `None` = 指定なし。
    #[serde(default)]
    pub starred: Option<bool>,
    /// `Some(true)` = 添付あり / `Some(false)` = 添付なし / `None` = 指定なし。
    #[serde(default)]
    pub has_attachment: Option<bool>,
    /// 複合タグ。非空なら `tag_match` で結合。scope の単一 tag_id とは独立に AND 合成。
    #[serde(default)]
    pub tag_ids: Vec<i64>,
    /// `tag_ids` の結合方法（既定 Or）。
    #[serde(default)]
    pub tag_match: TagMatch,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct EntryInput {
    pub title: String,
    pub year: Option<i64>,
    pub entry_type: String,
    #[serde(default)]
    pub citation_key: Option<String>,
    pub doi: Option<String>,
    pub isbn: Option<String>,
    pub arxiv_id: Option<String>,
    pub url: Option<String>,
    pub abstract_: Option<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub extra_fields: HashMap<String, String>,
    #[serde(default)]
    pub author_ids: Vec<i64>,
    #[serde(default)]
    pub author_names: Vec<String>,
    /// v0.3.0: 構造化された著者情報を渡すルート（bibtex literal / metadata の ORCID 等）。
    /// `Some` のとき `author_names` は無視され、ここに含まれる AuthorInput がそのまま
    /// `get_or_create_author` に渡される。フロント既存のペイロード互換のため `Option`。
    #[serde(default)]
    pub authors: Option<Vec<AuthorInput>>,
    #[serde(default)]
    pub tag_ids: Vec<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// フロント（types.ts の EntryFilter）が送る JSON 形状で正しくデシリアライズできること。
    /// キーは snake_case、tag_match は小文字、省略された任意フィールドは None/空になる。
    #[test]
    fn entry_filter_deserializes_frontend_shape() {
        let json = serde_json::json!({
            "entry_types": ["article", "book"],
            "year_min": 2020,
            "year_max": 2023,
            "starred": true,
            "has_attachment": false,
            "tag_ids": [1, 2],
            "tag_match": "and"
        });
        let f: EntryFilter = serde_json::from_value(json).unwrap();
        assert_eq!(f.entry_types, vec!["article", "book"]);
        assert_eq!(f.year_min, Some(2020));
        assert_eq!(f.year_max, Some(2023));
        assert_eq!(f.starred, Some(true));
        assert_eq!(f.has_attachment, Some(false));
        assert_eq!(f.tag_ids, vec![1, 2]);
        assert_eq!(f.tag_match, TagMatch::And);
    }

    /// 空フィルタ（EMPTY_FILTER 相当）と、全フィールド省略のどちらも既定値になる。
    #[test]
    fn entry_filter_defaults_are_permissive() {
        let empty: EntryFilter = serde_json::from_value(serde_json::json!({
            "entry_types": [], "tag_ids": [], "tag_match": "or"
        })).unwrap();
        assert!(empty.entry_types.is_empty());
        assert_eq!(empty.tag_match, TagMatch::Or);
        assert!(empty.starred.is_none());

        // 全省略でも default で成立する
        let bare: EntryFilter = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(bare.tag_match, TagMatch::Or);
        assert!(bare.tag_ids.is_empty());
    }
}
