use std::collections::HashMap;

#[derive(Debug, serde::Serialize, serde::Deserialize, sqlx::FromRow, Clone)]
pub struct Author {
    pub id: i64,
    pub name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub orcid: Option<String>,
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
    #[serde(default)]
    pub tag_ids: Vec<i64>,
}
