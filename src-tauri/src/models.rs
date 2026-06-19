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

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub parent_id: Option<i64>,
    pub children: Vec<Collection>,
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
