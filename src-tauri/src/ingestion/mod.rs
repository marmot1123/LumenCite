//! LCIR の取り込み（ingestion）。実験フラグ判定・添付ごとの LCIR 構築（pdfium）・
//! 派生 FTS 再生成・read 面の組み立て。既存 `fulltext` 経路は触らず、LCIR は追加の side-build。

pub mod pdf;

use crate::db::document_nodes::NewDocumentNode;
use crate::db::document_versions::NewDocumentVersion;
use crate::db::source_fragments::NewSourceFragment;
use crate::db::{document_nodes, document_versions, fulltext, settings, source_fragments};
use crate::document_ir::{self, CoordinateSpace, ExtractionStatus, FragmentType, NodeKind, Origin};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;

/// 実験フラグ。OFF の間は LCIR 経路を一切実行しない（既存挙動 byte-for-byte 不変）。
pub async fn lcir_enabled(pool: &SqlitePool) -> bool {
    settings::get_setting(pool, settings::LCIR_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
}

/// `build_lcir_for_attachment` の結果サマリ。
#[derive(Debug, serde::Serialize)]
pub struct LcirBuildResult {
    pub enabled: bool,
    pub built: bool,
    pub reused: bool,
    pub version_id: Option<i64>,
    pub content_key: Option<String>,
    pub page_count: i64,
    pub message: String,
}

/// `build_missing_lcir`（一括バックフィル）の結果サマリ。
#[derive(Debug, serde::Serialize)]
pub struct LcirBatchResult {
    pub enabled: bool,
    pub total: i64,
    pub built: i64,
    pub reused: i64,
    pub failed: i64,
}

/// 添付 1 件の LCIR を pdfium 抽出で構築する。
///
/// content_key で冪等: この添付に同一 content_key の completed があれば再抽出せず reuse。
/// 新版を採用したら同一添付の旧 completed を superseded にし、`parent_version_id` で連結する。
/// フラグ OFF なら何もせず `enabled: false` を返す（DB に一切書かない）。
pub async fn build_lcir_for_attachment(
    pool: &SqlitePool,
    app_data_dir: &Path,
    attachment_id: i64,
) -> Result<LcirBuildResult, String> {
    if !lcir_enabled(pool).await {
        return Ok(LcirBuildResult {
            enabled: false,
            built: false,
            reused: false,
            version_id: None,
            content_key: None,
            page_count: 0,
            message: "LCIR is disabled (settings 'lcir.enabled')".to_string(),
        });
    }

    // 添付の相対パス / mime を取得。
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT file_path, mime_type FROM attachments WHERE id = ?")
            .bind(attachment_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    let (file_path, mime_type) =
        row.ok_or_else(|| format!("attachment {attachment_id} not found"))?;
    let abs_path = app_data_dir.join(&file_path);

    // SHA-256 と pdfium 抽出は CPU/IO/native 依存なので blocking スレッドへ。
    let abs2 = abs_path.clone();
    let extracted = tokio::task::spawn_blocking(move || {
        let sha = document_ir::sha256_file(&abs2).map_err(|e| format!("sha256 failed: {e}"))?;
        let doc = pdf::extract_document(&abs2)?;
        Ok::<_, String>((sha, doc))
    })
    .await;
    let (source_sha256, extracted_doc) = match extracted {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(format!("extraction task panicked: {e}")),
    };

    let config_hash = "";
    let ckey = document_ir::content_key(
        &source_sha256,
        document_ir::schema::EXTRACTOR_NAME,
        document_ir::schema::EXTRACTOR_VERSION,
        config_hash,
    );

    // 冪等: 既存 completed があれば reuse（再抽出しない）。
    if let Some(existing) = document_versions::find_completed(pool, attachment_id, &ckey)
        .await
        .map_err(|e| e.to_string())?
    {
        let page_count = document_nodes::page_nodes_for_version(pool, existing.id)
            .await
            .map_err(|e| e.to_string())?
            .len() as i64;
        return Ok(LcirBuildResult {
            enabled: true,
            built: false,
            reused: true,
            version_id: Some(existing.id),
            content_key: Some(ckey),
            page_count,
            message: "reused existing LCIR version (same content_key)".to_string(),
        });
    }

    // 新版の親 = 現在の最新 completed（supersede チェーン）。
    let parent_version_id = document_versions::latest_completed_for_attachment(pool, attachment_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|v| v.id);

    let metadata = serde_json::json!({
        "coordinate_space": CoordinateSpace::default(),
        "page_count": extracted_doc.pages.len(),
        "pdfium_render": "0.8",
    })
    .to_string();
    let warnings_json = if extracted_doc.warnings.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&extracted_doc.warnings).unwrap_or_default())
    };
    let status = if extracted_doc.warnings.is_empty() {
        ExtractionStatus::Completed
    } else {
        ExtractionStatus::CompletedWithWarnings
    };

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let version_id = document_versions::insert_version(
        &mut *tx,
        &NewDocumentVersion {
            attachment_id,
            content_key: &ckey,
            schema_version: document_ir::schema::SCHEMA_VERSION,
            source_sha256: &source_sha256,
            source_mime_type: &mime_type,
            extractor_name: document_ir::schema::EXTRACTOR_NAME,
            extractor_version: document_ir::schema::EXTRACTOR_VERSION,
            config_hash,
            parent_version_id,
            status,
            warnings_json: warnings_json.as_deref(),
            metadata_json: Some(&metadata),
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    // document ルートノード。
    let doc_node_id = document_nodes::insert_node(
        &mut *tx,
        &NewDocumentNode {
            document_version_id: version_id,
            parent_id: None,
            node_kind: NodeKind::Document.as_str(),
            ordinal: 0,
            plain_text: None,
            language: None,
            confidence: None,
            origin: Some(Origin::PdfTextLayer.as_str()),
            payload_json: None,
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    let mut page_count = 0i64;
    for (pi, page) in extracted_doc.pages.iter().enumerate() {
        let payload = serde_json::json!({
            "page_width_pt": page.width_pt,
            "page_height_pt": page.height_pt,
            "rotation_deg": page.rotation_deg,
        })
        .to_string();
        let page_text = if page.plain_text.trim().is_empty() {
            None
        } else {
            Some(page.plain_text.as_str())
        };
        let page_node_id = document_nodes::insert_node(
            &mut *tx,
            &NewDocumentNode {
                document_version_id: version_id,
                parent_id: Some(doc_node_id),
                node_kind: NodeKind::Page.as_str(),
                ordinal: pi as i64,
                plain_text: page_text,
                language: None,
                confidence: None,
                origin: Some(Origin::PdfTextLayer.as_str()),
                payload_json: Some(&payload),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        page_count += 1;

        // 各 page には常にページ全面（MediaBox）の fragment を付与（分割失敗時も page 粒度に degrade）。
        source_fragments::insert_fragment(
            &mut *tx,
            &NewSourceFragment {
                node_id: page_node_id,
                page_number: page.page_number,
                x: 0.0,
                y: 0.0,
                width: page.width_pt,
                height: page.height_pt,
                rotation: page.rotation_deg,
                reading_order: Some(0),
                fragment_type: Some(FragmentType::Page.as_str()),
            },
        )
        .await
        .map_err(|e| e.to_string())?;

        // text_block ノード + bbox fragment。
        for block in &page.blocks {
            let tb_id = document_nodes::insert_node(
                &mut *tx,
                &NewDocumentNode {
                    document_version_id: version_id,
                    parent_id: Some(page_node_id),
                    node_kind: NodeKind::TextBlock.as_str(),
                    ordinal: block.reading_order,
                    plain_text: Some(block.text.as_str()),
                    language: None,
                    confidence: None,
                    origin: Some(Origin::PdfTextLayer.as_str()),
                    payload_json: None,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            source_fragments::insert_fragment(
                &mut *tx,
                &NewSourceFragment {
                    node_id: tb_id,
                    page_number: page.page_number,
                    x: block.bbox.x,
                    y: block.bbox.y,
                    width: block.bbox.width,
                    height: block.bbox.height,
                    rotation: page.rotation_deg,
                    reading_order: Some(block.reading_order),
                    fragment_type: Some(FragmentType::TextBlock.as_str()),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        }
    }

    // 新版採用: 同一添付の旧 completed を superseded に。
    document_versions::mark_superseded_for_attachment(&mut *tx, attachment_id, version_id)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(LcirBuildResult {
        enabled: true,
        built: true,
        reused: false,
        version_id: Some(version_id),
        content_key: Some(ckey),
        page_count,
        message: format!("built LCIR: {page_count} page(s)"),
    })
}

/// 完了 LCIR がまだ無い PDF 添付を洗い出し、順に構築する（過去分・失敗分の後追い）。
/// フラグ OFF なら `enabled: false` で即返す。既存 `index_missing_attachments` の LCIR 版。
pub async fn build_missing_lcir(
    pool: &SqlitePool,
    app_data_dir: &Path,
) -> Result<LcirBatchResult, String> {
    if !lcir_enabled(pool).await {
        return Ok(LcirBatchResult {
            enabled: false,
            total: 0,
            built: 0,
            reused: 0,
            failed: 0,
        });
    }
    let targets = document_versions::attachments_without_completed_lcir(pool)
        .await
        .map_err(|e| e.to_string())?;
    let total = targets.len() as i64;
    let (mut built, mut reused, mut failed) = (0i64, 0i64, 0i64);
    for (att_id, _path) in targets {
        match build_lcir_for_attachment(pool, app_data_dir, att_id).await {
            Ok(r) if r.built => built += 1,
            Ok(r) if r.reused => reused += 1,
            Ok(_) => {}
            Err(_) => failed += 1,
        }
    }
    Ok(LcirBatchResult {
        enabled: true,
        total,
        built,
        reused,
        failed,
    })
}

/// LCIR の page ノードの `plain_text` から `fulltext`(FTS5) を再生成する。
///
/// Phase 1「FTS5 を削除しても LCIR から再構築できる」の実証。既存の post-attach 索引は
/// pdf-extract 由来のまま並走させ、これは**まだ既定の索引ソースにはしない**（seam）。
/// 反映したページ数を返す。LCIR が無ければ 0。
pub async fn regenerate_page_fts_from_lcir(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<i64, String> {
    let version = match document_versions::latest_completed_for_attachment(pool, attachment_id)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(v) => v,
        None => return Ok(0),
    };
    let pages = document_nodes::page_nodes_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?;
    // page ノードの ordinal は 0 始まり。fulltext.page は 1 始まり（= ordinal + 1）。
    let rows: Vec<(i64, String)> = pages
        .into_iter()
        .filter_map(|p| p.plain_text.map(|t| (p.ordinal + 1, t)))
        .collect();
    let n = rows.len() as i64;
    fulltext::index_attachment(pool, attachment_id, &rows)
        .await
        .map_err(|e| e.to_string())?;
    Ok(n)
}

/// 添付の最新 LCIR を JSON 派生ビュー（`LcirDocument`）に組み立てる（read 面）。
pub async fn load_lcir_document(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<Option<document_ir::LcirDocument>, String> {
    let version = match document_versions::latest_completed_for_attachment(pool, attachment_id)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(v) => v,
        None => return Ok(None),
    };
    let nodes = document_nodes::nodes_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?;
    let frags = source_fragments::fragments_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?;

    let mut by_node: HashMap<i64, Vec<document_ir::LcirFragment>> = HashMap::new();
    for f in frags {
        by_node
            .entry(f.node_id)
            .or_default()
            .push(document_ir::LcirFragment {
                page: f.page_number,
                bbox: document_ir::BBox::new(f.x, f.y, f.width, f.height),
                fragment_type: f.fragment_type,
            });
    }

    let lcir_nodes = nodes
        .into_iter()
        .map(|n| document_ir::LcirNode {
            source_fragments: by_node.remove(&n.id).unwrap_or_default(),
            id: n.id,
            kind: n.node_kind,
            ordinal: n.ordinal,
            parent_id: n.parent_id,
            plain_text: n.plain_text,
            origin: n.origin,
            confidence: n.confidence,
        })
        .collect();

    Ok(Some(document_ir::LcirDocument {
        schema: document_ir::schema::SCHEMA_URI.to_string(),
        schema_version: version.schema_version,
        version_id: version.id,
        content_key: version.content_key,
        source: document_ir::LcirSource {
            sha256: version.source_sha256,
            mime_type: version.source_mime_type,
            extractor_name: version.extractor_name,
            extractor_version: version.extractor_version,
        },
        coordinate_space: CoordinateSpace::default(),
        nodes: lcir_nodes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    async fn setup_attachment(pool: &SqlitePool) -> i64 {
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "P".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        add_attachment(pool, entry.id, "attachments/1/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap()
            .id
    }

    /// フラグ未設定時、build は何もせず（DB に 0 行）`enabled: false` を返す。
    /// pdfium も触らないので添付ファイルが実在しなくても OK。
    #[sqlx::test(migrations = "./migrations")]
    async fn build_is_noop_when_flag_off(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let res = build_lcir_for_attachment(&pool, Path::new("/nonexistent"), att)
            .await
            .unwrap();
        assert!(!res.enabled);
        assert!(!res.built);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM document_versions")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "flag OFF は LCIR 表に一切書かない");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn build_missing_is_disabled_when_flag_off(pool: SqlitePool) {
        setup_attachment(&pool).await;
        let r = build_missing_lcir(&pool, Path::new("/nonexistent"))
            .await
            .unwrap();
        assert!(!r.enabled);
        assert_eq!(r.total, 0);
    }

    /// フラグ ON でも、完了 LCIR がある添付だけなら対象 0 で pdfium を呼ばない（CI 安全）。
    #[sqlx::test(migrations = "./migrations")]
    async fn build_missing_skips_already_built(pool: SqlitePool) {
        settings::set_setting(&pool, settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
        let att = setup_attachment(&pool).await;
        document_versions::insert_version(
            &pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: document_ir::schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: document_ir::schema::EXTRACTOR_NAME,
                extractor_version: document_ir::schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let r = build_missing_lcir(&pool, Path::new("/nonexistent"))
            .await
            .unwrap();
        assert!(r.enabled);
        assert_eq!(r.total, 0, "完了済み添付のみなら対象 0（抽出は走らない）");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn lcir_enabled_reflects_setting(pool: SqlitePool) {
        assert!(!lcir_enabled(&pool).await);
        settings::set_setting(&pool, settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
        assert!(lcir_enabled(&pool).await);
        settings::set_setting(&pool, settings::LCIR_ENABLED_KEY, "0")
            .await
            .unwrap();
        assert!(!lcir_enabled(&pool).await);
    }

    /// 手組みの LCIR（version + page ノード）から fulltext を再生成でき、検索でヒットする。
    /// Phase 1「FTS5 削除 → LCIR から再構築」の実証（pdfium 不要で CI 実行可能）。
    #[sqlx::test(migrations = "./migrations")]
    async fn regenerate_fts_from_manual_lcir(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = document_versions::insert_version(
            &pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: document_ir::schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: document_ir::schema::EXTRACTOR_NAME,
                extractor_version: document_ir::schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: None,
                payload_json: None,
            },
        )
        .await
        .unwrap();
        document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("Transformer architecture is described here."),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();

        let n = regenerate_page_fts_from_lcir(&pool, att).await.unwrap();
        assert_eq!(n, 1);
        let hits = fulltext::search_fulltext(&pool, "transformer", None, None, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attachment_id, att);
        assert_eq!(hits[0].page, 1, "page ノードの ordinal+1 が fulltext.page になる");
    }

    /// 手組みの LCIR を read 面（LcirDocument）に組み立て、fragment がノードに紐づき、
    /// validation を通ること。
    #[sqlx::test(migrations = "./migrations")]
    async fn load_lcir_document_assembles_tree(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = document_versions::insert_version(
            &pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: document_ir::schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: document_ir::schema::EXTRACTOR_NAME,
                extractor_version: document_ir::schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let root = document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("hello"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        source_fragments::insert_fragment(
            &pool,
            &NewSourceFragment {
                node_id: page,
                page_number: 1,
                x: 0.0,
                y: 0.0,
                width: 595.0,
                height: 842.0,
                rotation: 0.0,
                reading_order: Some(0),
                fragment_type: Some("page"),
            },
        )
        .await
        .unwrap();

        let doc = load_lcir_document(&pool, att).await.unwrap().unwrap();
        assert_eq!(doc.version_id, vid);
        assert_eq!(doc.content_key, "ck");
        assert_eq!(doc.nodes.len(), 2);
        let page_node = doc.nodes.iter().find(|n| n.kind == "page").unwrap();
        assert_eq!(page_node.source_fragments.len(), 1);
        assert_eq!(page_node.source_fragments[0].page, 1);
        assert!(document_ir::validation::validate(&doc).is_ok());
    }

    /// 手動 pdfium 実機確認: 実 DB コピー + 実 PDF に対して build → load → 冪等 build を走らせる。
    /// native lib（`src-tauri/pdfium/libpdfium.dylib`）が要るため `#[ignore]`。env 未設定なら skip。
    /// 例:
    /// `LCIR_SMOKE_DB=/path/copy.db LCIR_SMOKE_APPDIR="$HOME/Library/Application Support/com.lumencite.app" \
    ///  LCIR_SMOKE_ATT=8 cargo test --ignored lcir_build_real_pdf -- --nocapture`
    #[tokio::test]
    #[ignore = "manual pdfium smoke test; needs LCIR_SMOKE_* env + libpdfium"]
    async fn lcir_build_real_pdf() {
        let (db, appdir, att) = match (
            std::env::var("LCIR_SMOKE_DB"),
            std::env::var("LCIR_SMOKE_APPDIR"),
            std::env::var("LCIR_SMOKE_ATT"),
        ) {
            (Ok(d), Ok(a), Ok(t)) => (d, a, t.parse::<i64>().expect("LCIR_SMOKE_ATT must be int")),
            _ => {
                eprintln!("skip: set LCIR_SMOKE_DB / LCIR_SMOKE_APPDIR / LCIR_SMOKE_ATT");
                return;
            }
        };
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        settings::set_setting(&pool, settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();

        let res = build_lcir_for_attachment(&pool, Path::new(&appdir), att)
            .await
            .unwrap();
        eprintln!("build result: {res:?}");
        assert!(res.enabled);
        assert!(res.built || res.reused);
        assert!(res.page_count > 0, "should extract at least one page");

        let doc = load_lcir_document(&pool, att).await.unwrap().unwrap();
        let pages = doc.nodes.iter().filter(|n| n.kind == "page").count();
        let blocks = doc.nodes.iter().filter(|n| n.kind == "text_block").count();
        let frags: usize = doc.nodes.iter().map(|n| n.source_fragments.len()).sum();
        eprintln!(
            "content_key={} pages={pages} text_blocks={blocks} fragments={frags}",
            doc.content_key
        );
        for n in doc.nodes.iter().filter(|n| n.kind == "text_block").take(4) {
            if let Some(f) = n.source_fragments.first() {
                let snippet: String =
                    n.plain_text.as_deref().unwrap_or("").chars().take(50).collect();
                eprintln!(
                    "  p{} bbox=({:.1},{:.1},{:.1}x{:.1}) | {snippet:?}",
                    f.page, f.bbox.x, f.bbox.y, f.bbox.width, f.bbox.height
                );
            }
        }
        assert!(pages > 0);

        // 冪等性: 同一 PDF を再 build → 再抽出せず reuse（同一 content_key）。
        let again = build_lcir_for_attachment(&pool, Path::new(&appdir), att)
            .await
            .unwrap();
        eprintln!(
            "second build: built={} reused={}",
            again.built, again.reused
        );
        assert!(again.reused, "same PDF should reuse via content_key");
        assert_eq!(again.content_key, res.content_key);
    }
}
