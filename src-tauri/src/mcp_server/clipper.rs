//! Web クリッパー: ブラウザ拡張からの `POST /clipper` を処理する。
//!
//! 拡張はページから識別子（DOI / arXiv / ISBN）とタイトル等を抽出して送るだけで、
//! メタデータ解決（`metadata::fetch_by_*`）・重複判定（`find_duplicate_entry`）・
//! エントリ作成はすべてこちら側で行う。識別子が無い・解決に失敗した場合は
//! `webpage` エントリへフォールバックし、クリップ自体は失敗させない。
//!
//! DB ロジック（[`apply_clip`]）はネットワーク層（[`resolve_entry_input`]）と
//! 分離してあり、`#[sqlx::test]` で単体テストできる（`handle_rpc` と同じ方針）。

use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::models::EntryInput;

/// メタデータ API（CrossRef / arXiv / OpenLibrary）の応答待ち上限。
/// serve loop は単一スレッドなので、遅い外部 API で他のリクエストを塞がない。
const METADATA_TIMEOUT: Duration = Duration::from_secs(10);

/// `POST /clipper` のリクエストボディ。
#[derive(Debug, Default, Deserialize)]
pub struct ClipRequest {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub arxiv_id: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
    /// citation_pdf_url。無ければ arxiv_id から導出する（[`derive_pdf_url`]）。
    #[serde(default)]
    pub pdf_url: Option<String>,
    /// webpage フォールバック用: ページの公開日（ISO 8601 等。先頭 4 桁を year に使う）。
    #[serde(default)]
    pub published_date: Option<String>,
    /// webpage フォールバック用: サイト名（og:site_name）。
    #[serde(default)]
    pub site_name: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub collection_id: Option<i64>,
}

/// [`apply_clip`] の結果。
#[derive(Debug, PartialEq)]
pub enum ClipResult {
    Created { entry_id: i64, title: String },
    Duplicate { entry_id: i64, title: String },
}

/// 応答後に spawn する PDF ダウンロードジョブ（M3 で実行側を実装）。
#[derive(Debug, Clone)]
pub struct PdfJob {
    pub entry_id: i64,
    pub url: String,
}

/// [`handle_clip`] の結果。HTTP 層はこれを見て応答・副作用（sync/イベント/PDF）を行う。
pub struct ClipOutcome {
    pub response: Value,
    pub status: u16,
    pub mutated: bool,
    pub pdf_job: Option<PdfJob>,
}

/// `clipper.enabled` の現在値（リクエスト毎に評価し、トグル変更を即反映）。
pub async fn clipper_enabled(pool: &SqlitePool) -> bool {
    crate::db::settings::get_setting(pool, crate::db::settings::CLIPPER_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
}

/// 識別子なし・メタデータ解決失敗時のフォールバック入力を組み立てる（pure）。
/// 識別子は素通しで保持し、後からのクリップでも重複検出が効くようにする。
fn build_webpage_input(req: &ClipRequest) -> EntryInput {
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(&req.url)
        .to_string();

    let year = req
        .published_date
        .as_deref()
        .and_then(|d| d.get(..4))
        .and_then(|y| y.parse::<i64>().ok())
        .filter(|y| (1000..=9999).contains(y));

    let mut extra_fields = std::collections::HashMap::new();
    if let Some(site) = req.site_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        extra_fields.insert("organization".to_string(), site.to_string());
    }

    EntryInput {
        title,
        entry_type: "webpage".to_string(),
        url: Some(req.url.clone()),
        doi: req.doi.clone(),
        arxiv_id: req.arxiv_id.clone(),
        isbn: req.isbn.clone(),
        year,
        extra_fields,
        ..Default::default()
    }
}

/// 識別子からメタデータを解決する（優先順 DOI → arXiv → ISBN、各 10 秒タイムアウト）。
/// 失敗・識別子なしは webpage フォールバック。クリップ自体は失敗させない。
async fn resolve_entry_input(req: &ClipRequest) -> EntryInput {
    let fetched: Option<EntryInput> = if let Some(doi) = non_empty(req.doi.as_deref()) {
        with_timeout(crate::metadata::fetch_by_doi(doi)).await
    } else if let Some(arxiv) = non_empty(req.arxiv_id.as_deref()) {
        with_timeout(crate::metadata::fetch_by_arxiv(arxiv)).await
    } else if let Some(isbn) = non_empty(req.isbn.as_deref()) {
        with_timeout(crate::metadata::fetch_by_isbn(isbn)).await
    } else {
        None
    };

    match fetched {
        Some(mut input) => {
            // メタデータ側に URL が無ければクリップ元ページの URL を採用する
            if input.url.as_deref().map_or(true, |u| u.trim().is_empty()) {
                input.url = Some(req.url.clone());
            }
            input
        }
        None => build_webpage_input(req),
    }
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

async fn with_timeout(
    fut: impl std::future::Future<Output = Result<EntryInput, String>>,
) -> Option<EntryInput> {
    tokio::time::timeout(METADATA_TIMEOUT, fut).await.ok()?.ok()
}

/// 明示 `pdf_url` を優先し、無ければ arXiv ID から PDF URL を導出する（pure）。
pub fn derive_pdf_url(req: &ClipRequest) -> Option<String> {
    if let Some(url) = non_empty(req.pdf_url.as_deref()) {
        return Some(url.to_string());
    }
    non_empty(req.arxiv_id.as_deref())
        .map(|id| format!("https://arxiv.org/pdf/{}", crate::metadata::normalize_arxiv_id(id)))
}

/// DB のみを触るクリップ本体（テスト対象）: 重複判定 → タグ get-or-create →
/// `create_entry` → コレクション追加。重複時は何も作らず既存 id を返す。
pub async fn apply_clip(
    pool: &SqlitePool,
    mut input: EntryInput,
    req: &ClipRequest,
) -> Result<ClipResult, sqlx::Error> {
    // 重複判定はリクエストの識別子と解決済み入力の識別子の両方を見る
    // （metadata 解決で DOI が正規化されるケースに備える）。
    let dup = crate::db::entries::find_duplicate_entry(
        pool,
        non_empty(input.doi.as_deref().or(req.doi.as_deref())),
        non_empty(input.arxiv_id.as_deref().or(req.arxiv_id.as_deref())),
        non_empty(input.isbn.as_deref().or(req.isbn.as_deref())),
    )
    .await?;

    if let Some(entry_id) = dup {
        let title: String = sqlx::query_scalar("SELECT title FROM entries WHERE id = ?")
            .bind(entry_id)
            .fetch_one(pool)
            .await?;
        return Ok(ClipResult::Duplicate { entry_id, title });
    }

    if let Some(tags) = &req.tags {
        for name in tags {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let tag = crate::db::tags::get_or_create_tag(pool, name).await?;
            if !input.tag_ids.contains(&tag.id) {
                input.tag_ids.push(tag.id);
            }
        }
    }

    let entry = crate::db::entries::create_entry(pool, &input).await?;

    if let Some(collection_id) = req.collection_id {
        // 存在しないコレクション id は無視（クリップ自体は成功扱い）
        let _ =
            crate::db::collections::add_entry_to_collection(pool, entry.id, collection_id).await;
    }

    Ok(ClipResult::Created { entry_id: entry.id, title: entry.title })
}

/// クリップ 1 件の処理: メタデータ解決 → DB 適用 → 応答 JSON の組み立て。
pub async fn handle_clip(pool: &SqlitePool, req: &ClipRequest) -> ClipOutcome {
    let input = resolve_entry_input(req).await;

    match apply_clip(pool, input, req).await {
        Ok(ClipResult::Created { entry_id, title }) => {
            let pdf_job = derive_pdf_url(req).map(|url| PdfJob { entry_id, url });
            let mut response = json!({
                "status": "created",
                "entry_id": entry_id,
                "title": title,
            });
            if pdf_job.is_some() {
                response["pdf"] = json!("downloading");
            }
            ClipOutcome { response, status: 200, mutated: true, pdf_job }
        }
        Ok(ClipResult::Duplicate { entry_id, title }) => ClipOutcome {
            response: json!({
                "status": "duplicate",
                "entry_id": entry_id,
                "title": title,
            }),
            status: 200,
            mutated: false,
            pdf_job: None,
        },
        Err(e) => ClipOutcome {
            response: json!({ "status": "error", "code": "db_error", "message": e.to_string() }),
            status: 500,
            mutated: false,
            pdf_job: None,
        },
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;

    fn clip_req(url: &str) -> ClipRequest {
        ClipRequest { url: url.to_string(), ..Default::default() }
    }

    // ── pure ──────────────────────────────────────────────────────────────

    #[test]
    fn build_webpage_input_maps_fields() {
        let req = ClipRequest {
            url: "https://example.com/post".to_string(),
            title: Some("A Blog Post".to_string()),
            published_date: Some("2024-03-01T00:00:00Z".to_string()),
            site_name: Some("Example Blog".to_string()),
            doi: Some("10.1234/x".to_string()),
            ..Default::default()
        };
        let input = build_webpage_input(&req);
        assert_eq!(input.entry_type, "webpage");
        assert_eq!(input.title, "A Blog Post");
        assert_eq!(input.url.as_deref(), Some("https://example.com/post"));
        assert_eq!(input.year, Some(2024));
        assert_eq!(input.extra_fields.get("organization").map(String::as_str), Some("Example Blog"));
        // 識別子は素通しされる（後からの重複検出のため）
        assert_eq!(input.doi.as_deref(), Some("10.1234/x"));
    }

    #[test]
    fn build_webpage_input_falls_back_to_url_title() {
        let input = build_webpage_input(&clip_req("https://example.com"));
        assert_eq!(input.title, "https://example.com");
        assert_eq!(input.year, None);
    }

    #[test]
    fn derive_pdf_url_prefers_explicit_over_arxiv() {
        let mut req = clip_req("https://arxiv.org/abs/2301.00001v2");
        req.arxiv_id = Some("2301.00001v2".to_string());
        assert_eq!(
            derive_pdf_url(&req).as_deref(),
            Some("https://arxiv.org/pdf/2301.00001v2")
        );

        req.pdf_url = Some("https://example.com/paper.pdf".to_string());
        assert_eq!(derive_pdf_url(&req).as_deref(), Some("https://example.com/paper.pdf"));

        assert_eq!(derive_pdf_url(&clip_req("https://example.com")), None);
    }

    // ── DB ────────────────────────────────────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn clipper_enabled_defaults_to_false(pool: SqlitePool) {
        assert!(!clipper_enabled(&pool).await);
        crate::db::settings::set_setting(&pool, crate::db::settings::CLIPPER_ENABLED_KEY, "1")
            .await
            .unwrap();
        assert!(clipper_enabled(&pool).await);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn apply_clip_creates_webpage_entry_with_tags_and_collection(pool: SqlitePool) {
        let col = crate::db::collections::create_collection(&pool, "Inbox", None)
            .await
            .unwrap();
        let mut req = clip_req("https://example.com/post");
        req.title = Some("Post".to_string());
        req.tags = Some(vec!["ml".to_string(), "ml".to_string(), " ".to_string()]);
        req.collection_id = Some(col.id);
        let input = build_webpage_input(&req);

        let result = apply_clip(&pool, input, &req).await.unwrap();
        let ClipResult::Created { entry_id, title } = result else {
            panic!("expected Created, got {result:?}");
        };
        assert_eq!(title, "Post");

        let tag_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM entry_tags WHERE entry_id = ?")
                .bind(entry_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(tag_count, 1, "空白タグ・重複タグは除外して 1 件だけ付与");

        let in_col: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_collections WHERE entry_id = ? AND collection_id = ?",
        )
        .bind(entry_id)
        .bind(col.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(in_col, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn apply_clip_returns_duplicate_without_creating(pool: SqlitePool) {
        let existing = create_entry(&pool, &EntryInput {
            title: "Existing Paper".to_string(),
            entry_type: "article".to_string(),
            doi: Some("10.1234/example".to_string()),
            ..Default::default()
        }).await.unwrap();

        let mut req = clip_req("https://doi.org/10.1234/EXAMPLE");
        req.doi = Some("10.1234/EXAMPLE".to_string());
        let input = build_webpage_input(&req);

        let result = apply_clip(&pool, input, &req).await.unwrap();
        assert_eq!(
            result,
            ClipResult::Duplicate { entry_id: existing.id, title: "Existing Paper".to_string() }
        );

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "重複時は新規作成しない");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn handle_clip_fallback_creates_and_reports(pool: SqlitePool) {
        // 識別子なし → ネットワークに出ずに webpage フォールバックで作成される
        let mut req = clip_req("https://example.com/article");
        req.title = Some("No Identifier Page".to_string());

        let outcome = handle_clip(&pool, &req).await;
        assert_eq!(outcome.status, 200);
        assert!(outcome.mutated);
        assert!(outcome.pdf_job.is_none());
        assert_eq!(outcome.response["status"], "created");
        assert_eq!(outcome.response["title"], "No Identifier Page");
        assert!(outcome.response.get("pdf").is_none());

        // 同じページを再クリップ → URL では重複判定しないので新規になる…ではなく、
        // 識別子なしページは重複判定対象外のため 2 件目が作られる（v1 仕様）。
        let outcome2 = handle_clip(&pool, &req).await;
        assert_eq!(outcome2.response["status"], "created");
    }
}
