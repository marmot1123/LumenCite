//! LCIR の取り込み（ingestion）。実験フラグ判定・添付ごとの LCIR 構築（pdfium）・
//! 派生 FTS 再生成・read 面の組み立て。既存 `fulltext` 経路は触らず、LCIR は追加の side-build。

pub mod figures;
pub mod graph;
pub mod pdf;
pub mod structure;
pub mod symbols;
pub mod tex;

use crate::db::document_nodes::NewDocumentNode;
use crate::db::document_nodes_fts::NodeFtsInput;
use crate::db::document_versions::NewDocumentVersion;
use crate::db::source_fragments::NewSourceFragment;
use crate::db::{
    document_nodes, document_nodes_fts, document_versions, fulltext, math_expressions,
    node_relations, settings, source_fragments,
};
use crate::document_ir::{self, CoordinateSpace, ExtractionStatus, FragmentType, NodeKind, Origin};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;

/// arXiv TeX ソース添付の mime（`download_arxiv_source` が登録する唯一の値）。
/// **build ディスパッチとバッチ対象クエリはこの値を同一述語として共有する**（Phase 4）。
pub const TEX_SOURCE_MIME: &str = "application/gzip";

/// 実験フラグ。OFF の間は LCIR 経路を一切実行しない（既存挙動 byte-for-byte 不変）。
pub async fn lcir_enabled(pool: &SqlitePool) -> bool {
    settings::get_setting(pool, settings::LCIR_ENABLED_KEY)
        .await
        .ok()
        .flatten()
        .as_deref()
        == Some("1")
}

/// **読める LCIR が実在するか**（フラグ ON かつ完了済みの版が 1 本以上ある）。
///
/// `lcir_enabled` は「これから構築してよいか」を表すフラグで、ON にしただけでは何も
/// 構築されない（PDF の build は設定→データの手動ボタン）。read 面の露出可否
/// （チャットのツール一覧・Phase 10b）は「実際に読めるか」で決めないと、
/// `has_lcir:false` しか返さないツールの定義でコンテキストを食うことになる。
///
/// 逆にフラグを OFF に戻したときは、既に構築済みの版があっても隠す
/// （ユーザーが「使わない」と言った以上、外部モデルへ渡す面からは下ろす）。
pub async fn lcir_readable(pool: &SqlitePool) -> bool {
    if !lcir_enabled(pool).await {
        return false;
    }
    sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(
            SELECT 1 FROM document_versions
            WHERE extraction_status IN ('completed', 'completed_with_warnings')
         )",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0)
        == 1
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

/// 添付 1 件の LCIR を構築する。抽出器は添付の **mime だけ**で選ぶ（バッチ対象クエリと
/// 同一述語・`docs/LCIR_design_overview.md` Phase 4）: `%pdf%` → pdfium / `application/gzip`
/// （arXiv TeX ソース） → `lumencite-tex`。それ以外はエラー。
///
/// content_key で冪等: この添付に同一 content_key の completed があれば再抽出せず reuse。
/// **SHA-256 → reuse 判定 → 抽出**の順にし、rebuild バッチが対象を広めに拾っても
/// フル抽出は走らない。新版を採用したら同一添付の旧 completed を superseded にし、
/// `parent_version_id` で連結する（添付単位なので PDF 版と TeX 版が互いを supersede
/// することはない）。フラグ OFF なら何もせず `enabled: false` を返す（DB に一切書かない）。
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

    // 添付の相対パス / mime / entry_id を取得。
    let row: Option<(String, String, i64)> =
        sqlx::query_as("SELECT file_path, mime_type, entry_id FROM attachments WHERE id = ?")
            .bind(attachment_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    let (file_path, mime_type, entry_id) =
        row.ok_or_else(|| format!("attachment {attachment_id} not found"))?;
    let abs_path = app_data_dir.join(&file_path);

    let is_tex = mime_type == TEX_SOURCE_MIME;
    let is_pdf = mime_type.to_ascii_lowercase().contains("pdf");
    if !is_pdf && !is_tex {
        return Err(format!("unsupported attachment type for LCIR: {mime_type}"));
    }
    let (extractor_name, extractor_version) = if is_tex {
        (
            document_ir::schema::TEX_EXTRACTOR_NAME,
            document_ir::schema::TEX_EXTRACTOR_VERSION,
        )
    } else {
        (
            document_ir::schema::EXTRACTOR_NAME,
            document_ir::schema::EXTRACTOR_VERSION,
        )
    };

    // まず SHA-256 だけ計算する（IO なので blocking スレッドへ）。
    let abs2 = abs_path.clone();
    let source_sha256 = tokio::task::spawn_blocking(move || document_ir::sha256_file(&abs2))
        .await
        .map_err(|e| format!("sha256 task panicked: {e}"))?
        .map_err(|e| format!("sha256 failed: {e}"))?;

    let config_hash = "";
    let ckey =
        document_ir::content_key(&source_sha256, extractor_name, extractor_version, config_hash);

    // 冪等: 既存 completed があれば reuse（抽出そのものを省く）。
    if let Some(existing) = document_versions::find_completed(pool, attachment_id, &ckey)
        .await
        .map_err(|e| e.to_string())?
    {
        let page_count = document_nodes::page_nodes_for_version(pool, existing.id)
            .await
            .map_err(|e| e.to_string())?
            .len() as i64;
        // 派生 node-FTS を冪等に確認（既にあれば張り直すだけ・無ければ補う）。
        if let Err(e) = regenerate_node_fts_from_lcir(pool, attachment_id).await {
            eprintln!("LCIR: node-FTS regeneration failed for attachment {attachment_id}: {e}");
        }
        // アセットファイルの self-heal（Phase 8a・best-effort）: DB 行が指すファイルが消えて
        // いたら再抽出して書き直す（reuse は抽出を省くため、ここで補わないと恒久欠損になる）。
        if !is_tex {
            if let Err(e) = heal_missing_assets(pool, app_data_dir, existing.id, &abs_path).await {
                eprintln!("LCIR: asset self-heal failed for attachment {attachment_id}: {e}");
            }
        }
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

    // 新版の親 = 現在の最新 completed（supersede チェーン・添付単位 = 同一抽出器系列）。
    let parent_version_id = document_versions::latest_completed_for_attachment(pool, attachment_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|v| v.id);

    let (version_id, page_count, message) = if is_tex {
        let (vid, blocks) = build_tex_version(
            pool,
            attachment_id,
            &abs_path,
            &mime_type,
            &source_sha256,
            &ckey,
            parent_version_id,
        )
        .await?;
        (vid, 0, format!("built LCIR from TeX source: {blocks} block(s)"))
    } else {
        // アセットの相対ディレクトリ（Phase 8a）。attachments.file_path の親（'/' 区切り保証済み）
        // に '/' 文字列連結で組む（OS パス演算から逆算しない — Windows で '\\' を DB に入れない）。
        let asset_parent = file_path
            .rsplit_once('/')
            .map(|(parent, _)| parent.to_string())
            .unwrap_or_else(|| format!("attachments/{entry_id}"));
        let key16 = &ckey[..16];
        let asset_rel_dir = format!("{asset_parent}/.lcir/{attachment_id}/{key16}");
        let ctx = PdfBuildCtx {
            attachment_id,
            abs_path: &abs_path,
            mime_type: &mime_type,
            source_sha256: &source_sha256,
            ckey: &ckey,
            parent_version_id,
            app_data_dir,
            asset_rel_dir: &asset_rel_dir,
        };
        let (vid, pages) = build_pdf_version(pool, &ctx).await?;
        (vid, pages, format!("built LCIR: {pages} page(s)"))
    };

    // 派生の node-FTS を張り直す（best-effort。失敗しても LCIR 本体は確定済みなので build は
    // 成功扱い）。TeX 版は内部ガードで索引対象外になる。
    if let Err(e) = regenerate_node_fts_from_lcir(pool, attachment_id).await {
        eprintln!("LCIR: node-FTS regeneration failed for attachment {attachment_id}: {e}");
    }

    Ok(LcirBuildResult {
        enabled: true,
        built: true,
        reused: false,
        version_id: Some(version_id),
        content_key: Some(ckey),
        page_count,
        message,
    })
}

/// `build_pdf_version` の入力一式（Phase 8a でアセット書き出し先が増えたためまとめた）。
struct PdfBuildCtx<'a> {
    attachment_id: i64,
    abs_path: &'a Path,
    mime_type: &'a str,
    source_sha256: &'a str,
    ckey: &'a str,
    parent_version_id: Option<i64>,
    app_data_dir: &'a Path,
    /// アセットディレクトリ（app data dir 相対・'/' 区切り・content_key 先頭 16hex まで含む）。
    asset_rel_dir: &'a str,
}

/// pdfium 抽出で version + `document > page > block > line` の木を作る（Phase 1-3 の経路）。
/// Phase 8a: 図領域の `figure` ノード + crop PNG アセット + `caption_of` 辺も作る。
/// 返り値は (version_id, page_count)。
async fn build_pdf_version(pool: &SqlitePool, ctx: &PdfBuildCtx<'_>) -> Result<(i64, i64), String> {
    let abs_asset_dir = ctx.app_data_dir.join(ctx.asset_rel_dir);

    // pdfium 抽出は CPU/native 依存なので blocking スレッドへ。crop PNG は抽出中に
    // asset_dir へ書き出す（tx の外・メモリに貯めない）。
    let abs2 = ctx.abs_path.to_path_buf();
    let asset_dir2 = abs_asset_dir.clone();
    let extracted =
        tokio::task::spawn_blocking(move || pdf::extract_document(&abs2, Some(&asset_dir2))).await;
    let extracted_doc = match extracted {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            // 抽出失敗: 書きかけのアセットを best-effort で除去（同一 content_key の完了版は
            // 存在しない = reuse 経路に乗らなかったので、このディレクトリは今回のもの）。
            let _ = std::fs::remove_dir_all(&abs_asset_dir);
            return Err(e);
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&abs_asset_dir);
            return Err(format!("extraction task panicked: {e}"));
        }
    };

    let result = insert_pdf_version_tx(pool, ctx, &extracted_doc).await;
    match result {
        Ok(ok) => {
            // 旧 content_key ディレクトリ（superseded 版のアセット）を回収する（best-effort・
            // DB の assets 行は provenance として残す = 読み出しは latest completed のみ）。
            gc_stale_asset_dirs(ctx.app_data_dir, &abs_asset_dir);
            Ok(ok)
        }
        Err(e) => {
            // tx 失敗: 今回書いたアセットの孤児を best-effort で除去。
            let _ = std::fs::remove_dir_all(&abs_asset_dir);
            Err(e)
        }
    }
}

/// reuse 経路のアセット self-heal（Phase 8a・best-effort）。この版の assets 行が指すファイルが
/// 1 つでも欠けていたら再抽出して同一パスへ書き直す（同一 content_key = 抽出は決定的なので
/// ファイル名・領域は再現する）。DB は sha256/寸法/サイズだけ更新し、version 行・ノード・辺は
/// 不変。バックアップ復元後の部分欠損や手動削除からの回復経路（fulltext FTS5 self-heal と同型）。
async fn heal_missing_assets(
    pool: &SqlitePool,
    app_data_dir: &Path,
    version_id: i64,
    abs_path: &Path,
) -> Result<(), String> {
    let rows = crate::db::assets::assets_for_version(pool, version_id)
        .await
        .map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(());
    }
    if rows
        .iter()
        .all(|a| app_data_dir.join(&a.relative_path).is_file())
    {
        return Ok(());
    }
    // アセットディレクトリは行の relative_path（'/' 区切り）から復元する。
    let Some((rel_dir, _)) = rows[0].relative_path.rsplit_once('/') else {
        return Ok(());
    };
    let rel_dir = rel_dir.to_string();
    let abs2 = abs_path.to_path_buf();
    let dir2 = app_data_dir.join(&rel_dir);
    let extracted = tokio::task::spawn_blocking(move || pdf::extract_document(&abs2, Some(&dir2)))
        .await
        .map_err(|e| format!("asset heal task panicked: {e}"))?
        .map_err(|e| format!("asset heal extraction failed: {e}"))?;
    let mut refreshed = 0u64;
    for page in &extracted.pages {
        for region in &page.image_regions {
            if let Some(file) = &region.file {
                let rel = format!("{rel_dir}/{}", file.file_name);
                refreshed += crate::db::assets::refresh_asset_file(
                    pool,
                    version_id,
                    &rel,
                    &file.sha256,
                    (file.width_px as i64, file.height_px as i64),
                    file.size_bytes as i64,
                )
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }
    eprintln!(
        "LCIR: asset self-heal re-rendered files for version {version_id} ({refreshed} row(s) refreshed)"
    );
    Ok(())
}

/// 旧 content_key ディレクトリを「前回の残骸」とみなすまでの猶予（debt-15）。
///
/// `pnpm tauri dev` の debug ビルドと配布版は identifier が同じで、同一の app data dir と
/// 実 DB を共有する。GUI ロックは `try_lock` なので 2 個目の起動も止まらない。抽出器版の
/// 違う 2 つのインスタンスが同じ添付を build すると、猶予が無ければ互いの crop PNG を
/// trash へ送り合う定常ループになる（8c の alt text は crop の sha256 で carry するので、
/// 消し合いは再レンダリング費用だけでなく carry の当たり判定にも効く）。
///
/// 値と理由は `backup::WORK_FILE_STALE_SECS` に揃えてある。
const STALE_ASSET_DIR_SECS: u64 = 60 * 60;

/// `.lcir/<attachment_id>/` 直下の「現 content_key 以外」のサブディレクトリを trash へ。
/// ただし猶予（[`STALE_ASSET_DIR_SECS`]）内に書かれたものは別インスタンスが今まさに
/// 使っている可能性があるので残す。残しても次回の build で回収されるだけで、
/// 消し違えると別インスタンスの成果物が消える ＝ 非対称なので「疑わしきは残す」。
fn gc_stale_asset_dirs(app_data_dir: &Path, abs_asset_dir: &Path) {
    let (Some(parent), Some(current)) = (abs_asset_dir.parent(), abs_asset_dir.file_name()) else {
        return;
    };
    let Ok(rd) = std::fs::read_dir(parent) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() || entry.file_name() == current {
            continue;
        }
        if !is_stale_asset_dir(&p, now) {
            eprintln!(
                "LCIR: keeping recently written asset dir (another instance may own it): {}",
                p.display()
            );
            continue;
        }
        let _ = crate::attachment_trash::move_to_trash(app_data_dir, &p);
    }
}

/// アセットディレクトリが猶予を過ぎたか。
///
/// 見るのは**中のファイルの mtime の最大値**。ここを最古（`min`）にすると 1 添付の抽出が
/// 最長 75 分（att37・527 頁）かかるせいで、**最初に書いた crop が build 完了時点で既に
/// 猶予を超えている**＝長い build ほど猶予が実質ゼロになり、猶予を入れた意味が消える。
/// 最も高価な添付でだけ守られない、という最悪の壊れ方をするので `max` でなければならない。
///
/// ディレクトリ自身の mtime ではなくファイルを見るのは、ファイルの mtime のほうが
/// 「いつ書かれたか」の直接の証拠だから。crop の書き出しは `write_atomic` の tmp + rename
/// なので今はディレクトリの mtime も動くが、同名 in-place 上書きに変えた途端に動かなくなる。
/// ファイルが 1 つも無いときだけディレクトリ自身の mtime にフォールバックする
/// （書き始める直前の別インスタンスを守るため）。crop ディレクトリは平坦なので 1 階層で足りる。
fn is_stale_asset_dir(dir: &Path, now: std::time::SystemTime) -> bool {
    let newest = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().and_then(|m| m.modified()).ok())
        .max();
    let mtime = match newest.or_else(|| std::fs::metadata(dir).and_then(|m| m.modified()).ok()) {
        Some(m) => m,
        // mtime が読めないなら判断材料が無い。回収せず次回に回す。
        None => return false,
    };
    // mtime が未来（時計のずれ・別マシンからの復元）なら `duration_since` は Err。
    // その場合も回収しない。
    now.duration_since(mtime)
        .is_ok_and(|age| age.as_secs() >= STALE_ASSET_DIR_SECS)
}

/// version 行 + ノード木 + 数式 + 図アセット + 関係辺を 1 トランザクションで挿入する。
async fn insert_pdf_version_tx(
    pool: &SqlitePool,
    ctx: &PdfBuildCtx<'_>,
    extracted_doc: &pdf::ExtractedDocument,
) -> Result<(i64, i64), String> {
    let attachment_id = ctx.attachment_id;
    let (figure_total, asset_total) = extracted_doc
        .pages
        .iter()
        .flat_map(|p| p.image_regions.iter())
        .fold((0usize, 0usize), |(f, a), r| {
            (f + 1, a + usize::from(r.file.is_some()))
        });
    let metadata = serde_json::json!({
        "coordinate_space": CoordinateSpace::default(),
        "page_count": extracted_doc.pages.len(),
        "pdfium_render": "0.8",
        "figure_count": figure_total,
        "asset_count": asset_total,
        "render_target_width": pdf::RENDER_TARGET_WIDTH,
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

    // Phase 8c: 版跨ぎの alt text 引き継ぎ材料。crop PNG の SHA-256（= バイト同一画像の指紋）を
    // キーに、同一添付の過去の全版から生成済み alt text を引く。抽出器版を上げるたびに Vision を
    // 再課金しないための carry（tx の外で読むだけ・アセットが無い抽出では引かない）。
    let carry_alt_texts = if asset_total > 0 {
        crate::db::node_alt_texts::alt_texts_by_asset_sha256(pool, attachment_id)
            .await
            .map_err(|e| e.to_string())?
    } else {
        HashMap::new()
    };

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let version_id = document_versions::insert_version(
        &mut *tx,
        &NewDocumentVersion {
            attachment_id,
            content_key: ctx.ckey,
            schema_version: document_ir::schema::SCHEMA_VERSION,
            source_sha256: ctx.source_sha256,
            source_mime_type: ctx.mime_type,
            extractor_name: document_ir::schema::EXTRACTOR_NAME,
            extractor_version: document_ir::schema::EXTRACTOR_VERSION,
            config_hash: "",
            parent_version_id: ctx.parent_version_id,
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

    // Phase 2: pdfium セグメントを論理構造（段落・見出し・caption 等）に認識し、
    // document > page > block > line の木にする。recognizer 状態はページをまたいで継続する
    // （abstract/参考文献モードが複数ページに渡るため）。
    let mut page_count = 0i64;
    let mut recognizer = structure::RecognizerState::new();
    // 参照グラフ用に block ノードの軽量ビューを読み順（ページ跨ぎの通し番号）で集める（Phase 6a）。
    let mut graph_nodes: Vec<graph::GraphNode> = Vec::new();
    let mut reading_index = 0i64;
    // 図の文書通し番号（Phase 8a・1 始まり）。
    let mut figure_index = 0i64;
    for (pi, page) in extracted_doc.pages.iter().enumerate() {
        let payload = serde_json::json!({
            "page_width_pt": page.width_pt,
            "page_height_pt": page.height_pt,
            "rotation_deg": page.rotation_deg,
        })
        .to_string();
        // **保存の前に C0 制御文字を落とす**（debt-22）。索引側（p1）だけで正規化すると、
        // 9a の JSON export（`export_lcir_json`・UI から実呼出）と `get_node_context` の
        // page-focus に生値が残り、以後の `LcirDocument` の読み手全員に正規化義務が分散する。
        // 改行は保つ（[`structure::clean_page_text`] の doc コメント参照）。
        let cleaned_page_text = structure::clean_page_text(&page.plain_text);
        let page_text = if cleaned_page_text.trim().is_empty() {
            None
        } else {
            Some(cleaned_page_text.as_str())
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

        // 各 page には常にページ全面（MediaBox）の fragment を付与（構造分割が失敗しても
        // page 粒度に degrade して情報を失わない）。
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

        // 論理ブロック + その行。ブロック型は推定なので origin=layout_model + confidence を必ず持たせ、
        // 行テキストは PDF レイヤー由来なので pdf_text_layer にする（原文由来と推定を区別）。
        let blocks = structure::recognize_page(page, &mut recognizer);
        // 図領域ペアリング候補の caption（Phase 8a）。Algorithm/Listing は FigureCaption だが
        // 画像領域の caption ではないのでラベル語で除外する。
        let mut page_captions: Vec<(i64, document_ir::BBox, Option<String>)> = Vec::new();
        for (bi, sblock) in blocks.iter().enumerate() {
            let payload_json = block_payload_json(sblock);
            let block_node_id = document_nodes::insert_node(
                &mut *tx,
                &NewDocumentNode {
                    document_version_id: version_id,
                    parent_id: Some(page_node_id),
                    node_kind: sblock.kind.as_str(),
                    ordinal: bi as i64,
                    plain_text: Some(sblock.text.as_str()),
                    language: None,
                    confidence: Some(sblock.confidence),
                    origin: Some(Origin::LayoutModel.as_str()),
                    payload_json: payload_json.as_deref(),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
            // 参照グラフ用ビュー（PDF は label/cite_key を持たず、番号一致で解決する）。
            graph_nodes.push(graph::GraphNode {
                id: block_node_id,
                kind: sblock.kind,
                reading_index,
                plain_text: sblock.text.clone(),
                labels: Vec::new(),
                equation_label: sblock.equation_label.clone(),
                theorem_number: sblock.theorem_number.clone(),
                cite_key: None,
                caption_label: sblock.caption_label.clone(),
                caption_number: sblock.caption_number.clone(),
            });
            reading_index += 1;
            if sblock.kind == NodeKind::FigureCaption
                && structure::is_figure_caption_label(sblock.caption_label.as_deref())
            {
                page_captions.push((
                    block_node_id,
                    sblock.bbox,
                    sblock.caption_number.clone(),
                ));
            }
            source_fragments::insert_fragment(
                &mut *tx,
                &NewSourceFragment {
                    node_id: block_node_id,
                    page_number: page.page_number,
                    x: sblock.bbox.x,
                    y: sblock.bbox.y,
                    width: sblock.bbox.width,
                    height: sblock.bbox.height,
                    rotation: page.rotation_deg,
                    reading_order: Some(bi as i64),
                    fragment_type: Some(FragmentType::Block.as_str()),
                },
            )
            .await
            .map_err(|e| e.to_string())?;

            // display 数式は表層表現 1 行を作る（Phase 3）。PDF 由来なので LaTeX/MathML は未確定で
            // normalized_text（= クリーンな表層文字列 = block の plain_text）だけを埋め、
            // semantic_status='surface_only'・origin='pdf_text_layer'。意味は Phase 7 で。
            if sblock.kind == NodeKind::DisplayMath {
                math_expressions::insert_math(
                    &mut *tx,
                    &math_expressions::NewMathExpression {
                        node_id: block_node_id,
                        display_mode: document_ir::MathDisplayMode::Display.as_str(),
                        equation_label: sblock.equation_label.as_deref(),
                        latex: None,
                        presentation_mathml: None,
                        content_mathml: None,
                        openmath_json: None,
                        normalized_text: Some(sblock.text.as_str()),
                        ast_json: None,
                        semantic_status: document_ir::MathSemanticStatus::SurfaceOnly.as_str(),
                        confidence: Some(sblock.confidence),
                        origin: Some(Origin::PdfTextLayer.as_str()),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
            }

            for (li, line) in sblock.lines.iter().enumerate() {
                let line_node_id = document_nodes::insert_node(
                    &mut *tx,
                    &NewDocumentNode {
                        document_version_id: version_id,
                        parent_id: Some(block_node_id),
                        node_kind: NodeKind::Line.as_str(),
                        ordinal: li as i64,
                        plain_text: Some(line.text.as_str()),
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
                        node_id: line_node_id,
                        page_number: page.page_number,
                        x: line.bbox.x,
                        y: line.bbox.y,
                        width: line.bbox.width,
                        height: line.bbox.height,
                        rotation: page.rotation_deg,
                        reading_order: Some(line.reading_order),
                        fragment_type: Some(FragmentType::Line.as_str()),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
            }
        }

        // Phase 8a: 図領域 → figure ノード + crop アセット + caption_of 辺。
        // 領域はレイアウト推定なので origin=layout_model + confidence。caption とのペアは
        // 幾何（相互最近のみ）で決め、番号は caption の "Figure N" から引き継ぐ。
        if !page.image_regions.is_empty() {
            let cap_bboxes: Vec<document_ir::BBox> =
                page_captions.iter().map(|(_, b, _)| *b).collect();
            // Phase 8d-2: **ラスタ図を先に caption と結び、ベクター図は余った caption とだけ結ぶ**。
            // 1 段でまとめて `pair_captions` に掛けると、相互最近がページ全体の大域計算なので
            // ベクター領域を 1 個足しただけで既存ラスタ図の `caption_of` 辺が奪われうる。
            let split = |want: figures::RegionSource| -> (Vec<usize>, Vec<document_ir::BBox>) {
                page.image_regions
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.source == want)
                    .map(|(i, r)| (i, r.bbox))
                    .unzip()
            };
            let (raster_at, raster_bboxes) = split(figures::RegionSource::Raster);
            let (vector_at, vector_bboxes) = split(figures::RegionSource::Vector);
            let pair_map: std::collections::HashMap<usize, usize> =
                figures::pair_captions_two_stage(&raster_bboxes, &vector_bboxes, &cap_bboxes)
                    .into_iter()
                    .map(|(fi, ci)| {
                        // 戻り値は `raster ++ vector` の連結順なので、元の並びへ引き直す。
                        let at = if fi < raster_bboxes.len() {
                            raster_at[fi]
                        } else {
                            vector_at[fi - raster_bboxes.len()]
                        };
                        (at, ci)
                    })
                    .collect();
            for (ri, region) in page.image_regions.iter().enumerate() {
                figure_index += 1;
                let paired = pair_map.get(&ri).copied();
                let figure_number =
                    paired.and_then(|ci| page_captions[ci].2.as_deref());
                let mut payload = serde_json::Map::new();
                payload.insert(
                    "figure_index".to_string(),
                    serde_json::Value::from(figure_index),
                );
                if let Some(n) = figure_number {
                    payload.insert(
                        "figure_number".to_string(),
                        serde_json::Value::from(n.to_string()),
                    );
                }
                // Phase 8d-2: ベクター図だけ由来を記録する。**ラスタ図の payload が完全に
                // 不変なわけではない** ── `figure_index` は文書通しの連番なので、あるページで
                // ベクター図が 1 個採られると**それ以降のページのラスタ図の番号がずれる**。
                // 不変なのは `region_index` / `ordinal` / crop のファイル名 / caption ペアの方で、
                // alt text の carry は crop の sha256 キーなので `figure_index` のずれでは壊れない。
                if region.source == figures::RegionSource::Vector {
                    payload.insert(
                        "region_source".to_string(),
                        serde_json::Value::from("vector"),
                    );
                }
                let payload_json = serde_json::Value::Object(payload).to_string();
                let figure_node_id = document_nodes::insert_node(
                    &mut *tx,
                    &NewDocumentNode {
                        document_version_id: version_id,
                        parent_id: Some(page_node_id),
                        node_kind: NodeKind::Figure.as_str(),
                        ordinal: (blocks.len() + ri) as i64,
                        plain_text: None,
                        language: None,
                        confidence: Some(region.source.confidence()),
                        origin: Some(Origin::LayoutModel.as_str()),
                        payload_json: Some(&payload_json),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
                // 参照グラフ用ビュー（Phase 8d-7）。本文の "Figure 3" が指す**実体**の
                // ターゲットになる。plain_text は無いのでこの行から参照が出ることはない。
                //
                // **ベクター図は番号を渡さない**（Phase 8d-2）。`graph::FloatTargets` は同じ番号が
                // 2 つのノードに付くとその番号ごと墓標（`None`）にするので、渡すと
                // 「同一版に同番号の図 caption が 2 つあり、片方だけがラスタ図と結ばれている」版
                // （実 DB に 18 版・27 組）で、余っていた方にベクター図が付いた瞬間に
                // **既存の `refers_to_figure` 辺が消える**。caption 側は同番号の時点で既に墓標なので
                // フォールバックも効かない。番号は payload には載せる（`get_figures` の表示用）が、
                // 参照解決の実体索引には入れない ── 既存の辺を守る方を採る。
                // 恒久解は `FloatTargets::insert` を由来つきにしてラスタを勝たせることだが、
                // それは 8d-7 の契約を変えるので 8d-2 のスコープ外にした。
                let graph_caption_number = match region.source {
                    figures::RegionSource::Raster => figure_number.map(|s| s.to_string()),
                    figures::RegionSource::Vector => None,
                };
                graph_nodes.push(graph::GraphNode {
                    id: figure_node_id,
                    kind: NodeKind::Figure,
                    reading_index,
                    plain_text: String::new(),
                    labels: Vec::new(),
                    equation_label: None,
                    theorem_number: None,
                    cite_key: None,
                    caption_label: None,
                    caption_number: graph_caption_number,
                });
                reading_index += 1;
                source_fragments::insert_fragment(
                    &mut *tx,
                    &NewSourceFragment {
                        node_id: figure_node_id,
                        page_number: page.page_number,
                        x: region.bbox.x,
                        y: region.bbox.y,
                        width: region.bbox.width,
                        height: region.bbox.height,
                        rotation: page.rotation_deg,
                        reading_order: None,
                        fragment_type: Some(FragmentType::Block.as_str()),
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
                if let Some(file) = &region.file {
                    let relative_path = format!("{}/{}", ctx.asset_rel_dir, file.file_name);
                    let asset_meta = serde_json::json!({
                        "page": page.page_number,
                        "region_index": ri,
                        "render_target_width": pdf::RENDER_TARGET_WIDTH,
                    })
                    .to_string();
                    let asset_id = crate::db::assets::insert_asset(
                        &mut *tx,
                        &crate::db::assets::NewAsset {
                            document_version_id: version_id,
                            sha256: &file.sha256,
                            mime_type: "image/png",
                            relative_path: &relative_path,
                            width: Some(file.width_px as i64),
                            height: Some(file.height_px as i64),
                            size_bytes: Some(file.size_bytes as i64),
                            metadata_json: Some(&asset_meta),
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    crate::db::assets::insert_node_asset(
                        &mut *tx,
                        &crate::db::assets::NewNodeAsset {
                            node_id: figure_node_id,
                            asset_id,
                        },
                        "page_crop",
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                    // Phase 8c: 同じ crop（バイト同一画像）を説明済みなら、その alt text を
                    // 新版のこのノードへ引き継ぐ（Vision の再課金を避ける）。由来版を
                    // carried_from_version_id に残し、生成物であることは origin/model が示す。
                    if let Some(prev) = carry_alt_texts.get(&file.sha256) {
                        crate::db::node_alt_texts::insert_alt_text(
                            &mut *tx,
                            &crate::db::node_alt_texts::NewAltText {
                                node_id: figure_node_id,
                                document_version_id: version_id,
                                source_asset_sha256: &file.sha256,
                                text: &prev.text,
                                origin: &prev.origin,
                                confidence: prev.confidence,
                                model: prev.model.as_deref(),
                                // 既に引き継がれた行を再度引き継ぐときも「最初の生成版」を指す。
                                carried_from_version_id: Some(
                                    prev.carried_from_version_id
                                        .unwrap_or(prev.document_version_id),
                                ),
                            },
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    }
                }
                if let Some(ci) = paired {
                    node_relations::insert_relation(
                        &mut *tx,
                        &node_relations::NewNodeRelation {
                            document_version_id: version_id,
                            from_node_id: page_captions[ci].0,
                            relation_type: document_ir::RelationType::CaptionOf.as_str(),
                            to_node_id: figure_node_id,
                            // 辺の確からしさは結んだ図の確からしさを超えない（8d-2）。
                            confidence: Some(region.source.confidence()),
                            origin: Some(Origin::LayoutModel.as_str()),
                            metadata_json: None,
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // 参照グラフ（本文→数式/定理、proof→theorem）を解決して張る（Phase 6a・PDF は番号一致）。
    insert_relations_for_version(&mut tx, version_id, &graph_nodes, graph::RefStrategy::Pdf).await?;

    // Phase 8c: carry と同じ tx で旧版の生成 alt text を刈る（crop PNG は commit 後の GC で
    // trash されるので、行だけ残しても参照されず肥大化するだけ）。**新版にアセットが 1 つも
    // 無いときは刈らない** — 抽出の一時的な不調で 0 件になったときに過去の生成物を永久に
    // 失わないため。手編集（user_edited）はアクセサ側で対象外。
    if asset_total > 0 {
        let pruned =
            crate::db::node_alt_texts::prune_carried_alt_texts(&mut *tx, attachment_id, version_id)
                .await
                .map_err(|e| e.to_string())?;
        if pruned > 0 {
            eprintln!(
                "LCIR: pruned {pruned} superseded alt text row(s) for attachment {attachment_id}"
            );
        }
    }

    // 新版採用: 同一添付の旧 completed を superseded に。
    document_versions::mark_superseded_for_attachment(&mut *tx, attachment_id, version_id)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok((version_id, page_count))
}

/// TeX 抽出（`ingestion::tex`）で version + `document > block` のフラット木を作る（Phase 4）。
/// page/line ノードと `source_fragments` は作らない（TeX に PDF 座標は無い）。display 数式は
/// **原文 LaTeX** を `math_expressions.latex` に `semantic_status='source_provided'` で保存する。
/// 返り値は (version_id, block_count)。
async fn build_tex_version(
    pool: &SqlitePool,
    attachment_id: i64,
    abs_path: &Path,
    mime_type: &str,
    source_sha256: &str,
    ckey: &str,
    parent_version_id: Option<i64>,
) -> Result<(i64, i64), String> {
    // gzip/tar 展開 + 解析は CPU/IO 依存なので blocking スレッドへ。
    let abs2 = abs_path.to_path_buf();
    let extracted = tokio::task::spawn_blocking(move || tex::extract_document(&abs2)).await;
    let doc = match extracted {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(format!("extraction task panicked: {e}")),
    };

    // TeX には座標が無いので coordinate_space は記録しない。
    let table_count = doc
        .blocks
        .iter()
        .filter(|b| b.kind == NodeKind::Table)
        .count();
    let metadata = serde_json::json!({
        "main_file": doc.main_file,
        "source_file_count": doc.source_file_count,
        "block_count": doc.blocks.len(),
        "table_count": table_count,
    })
    .to_string();
    let warnings_json = if doc.warnings.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&doc.warnings).unwrap_or_default())
    };
    let status = if doc.warnings.is_empty() {
        ExtractionStatus::Completed
    } else {
        ExtractionStatus::CompletedWithWarnings
    };

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    let version_id = document_versions::insert_version(
        &mut *tx,
        &NewDocumentVersion {
            attachment_id,
            content_key: ckey,
            schema_version: document_ir::schema::SCHEMA_VERSION,
            source_sha256,
            source_mime_type: mime_type,
            extractor_name: document_ir::schema::TEX_EXTRACTOR_NAME,
            extractor_version: document_ir::schema::TEX_EXTRACTOR_VERSION,
            config_hash: "",
            parent_version_id,
            status,
            warnings_json: warnings_json.as_deref(),
            metadata_json: Some(&metadata),
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    // document ルート（原文由来なので origin=tex_source）。
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
            origin: Some(Origin::TexSource.as_str()),
            payload_json: None,
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    let block_count = doc.blocks.len() as i64;
    // 参照グラフ / 記号系用に block ノードの軽量ビューを集める（TeX はフラットなので ordinal = 読み順）。
    let mut graph_nodes: Vec<graph::GraphNode> = Vec::new();
    let mut symbol_nodes: Vec<symbols::SymbolNode> = Vec::new();
    // Phase 8b: 同一 table 環境由来の (table_caption, table) を env_group で結んで caption_of 辺に
    // する（PDF 8a の幾何ペアリングと違い構造的事実なので高信頼）。BTreeMap で挿入順を決定的に。
    let mut caption_by_group: std::collections::BTreeMap<u32, i64> = std::collections::BTreeMap::new();
    let mut table_by_group: std::collections::BTreeMap<u32, i64> = std::collections::BTreeMap::new();
    for (bi, block) in doc.blocks.iter().enumerate() {
        let payload_json = tex_block_payload_json(block);
        let node_id = document_nodes::insert_node(
            &mut *tx,
            &NewDocumentNode {
                document_version_id: version_id,
                parent_id: Some(doc_node_id),
                node_kind: block.kind.as_str(),
                ordinal: bi as i64,
                plain_text: Some(block.text.as_str()),
                language: None,
                confidence: Some(block.confidence),
                origin: Some(Origin::TexSource.as_str()),
                payload_json: payload_json.as_deref(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        // 参照グラフ用ビュー（TeX は \label/cite_key を原資料から持つ）。
        graph_nodes.push(graph::GraphNode {
            id: node_id,
            kind: block.kind,
            reading_index: bi as i64,
            plain_text: block.text.clone(),
            labels: block.labels.clone(),
            equation_label: block.equation_label.clone(),
            theorem_number: None,
            cite_key: block.cite_key.clone(),
            // TeX は caption のラベル語も float 番号（コンパイル時に決まる）も持たない。
            caption_label: None,
            caption_number: None,
        });
        symbol_nodes.push(symbols::SymbolNode {
            id: node_id,
            kind: block.kind,
            reading_index: bi as i64,
            plain_text: block.text.clone(),
            latex: block.latex.clone(),
        });
        if let Some(g) = block.env_group {
            match block.kind {
                NodeKind::TableCaption => {
                    caption_by_group.insert(g, node_id);
                }
                NodeKind::Table => {
                    table_by_group.insert(g, node_id);
                }
                _ => {}
            }
        }

        if block.kind == NodeKind::DisplayMath {
            math_expressions::insert_math(
                &mut *tx,
                &math_expressions::NewMathExpression {
                    node_id,
                    display_mode: document_ir::MathDisplayMode::Display.as_str(),
                    equation_label: block.equation_label.as_deref(),
                    latex: block.latex.as_deref(),
                    presentation_mathml: None,
                    content_mathml: None,
                    openmath_json: None,
                    normalized_text: Some(block.text.as_str()),
                    ast_json: None,
                    semantic_status: document_ir::MathSemanticStatus::SourceProvided.as_str(),
                    confidence: Some(block.confidence),
                    origin: Some(Origin::TexSource.as_str()),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        }
    }

    // Phase 8b: caption_of 辺（from=caption → to=table・同一環境由来の構造的事実・原文由来）。
    for (g, table_node_id) in &table_by_group {
        if let Some(caption_node_id) = caption_by_group.get(g) {
            node_relations::insert_relation(
                &mut *tx,
                &node_relations::NewNodeRelation {
                    document_version_id: version_id,
                    from_node_id: *caption_node_id,
                    relation_type: document_ir::RelationType::CaptionOf.as_str(),
                    to_node_id: *table_node_id,
                    confidence: Some(0.95),
                    origin: Some(Origin::TexSource.as_str()),
                    metadata_json: None,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        }
    }

    // 参照グラフ（\ref/\eqref/\cite・proof→theorem）を解決して張る（Phase 6a・TeX は label 一致）。
    insert_relations_for_version(&mut tx, version_id, &graph_nodes, graph::RefStrategy::Tex).await?;
    // 記号定義（"let $U$ be ...", "$H := ...$"）を抽出（Phase 6b・TeX のみ）。
    insert_symbols_for_version(&mut tx, version_id, &symbol_nodes).await?;

    document_versions::mark_superseded_for_attachment(&mut *tx, attachment_id, version_id)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    Ok((version_id, block_count))
}

/// TeX ブロックの型固有属性（見出し階層・節番号・\label 名・cite key）を payload_json にする。
fn tex_block_payload_json(b: &tex::TexBlock) -> Option<String> {
    let mut map = serde_json::Map::new();
    if let Some(level) = b.heading_level {
        map.insert("heading_level".to_string(), serde_json::Value::from(level));
    }
    if let Some(ref number) = b.section_number {
        map.insert(
            "section_number".to_string(),
            serde_json::Value::from(number.clone()),
        );
    }
    if !b.labels.is_empty() {
        map.insert(
            "labels".to_string(),
            serde_json::Value::from(b.labels.clone()),
        );
    }
    if let Some(ref key) = b.cite_key {
        map.insert("cite_key".to_string(), serde_json::Value::from(key.clone()));
    }
    if let Some(ref note) = b.note {
        map.insert("note".to_string(), serde_json::Value::from(note.clone()));
    }
    // Phase 8b: table ノードのセル構造。追加式スキーマ（colspan/rowspan/rule_above/alignments/
    // latex_source は値があるときだけ出す）。下流（markdown・MCP）は column_spec を再パースせず
    // alignments を見る。
    if let Some(ref t) = b.table {
        map.insert(
            "column_spec".to_string(),
            serde_json::Value::from(t.column_spec.clone()),
        );
        map.insert("n_columns".to_string(), serde_json::Value::from(t.n_columns));
        map.insert(
            "n_rows".to_string(),
            serde_json::Value::from(t.rows.len() as i64),
        );
        if let Some(ref aligns) = t.alignments {
            map.insert(
                "alignments".to_string(),
                serde_json::Value::from(aligns.clone()),
            );
        }
        let rows: Vec<serde_json::Value> = t
            .rows
            .iter()
            .map(|r| {
                let cells: Vec<serde_json::Value> = r
                    .cells
                    .iter()
                    .map(|c| {
                        let mut m = serde_json::Map::new();
                        m.insert("text".to_string(), serde_json::Value::from(c.text.clone()));
                        if let Some(n) = c.colspan {
                            m.insert("colspan".to_string(), serde_json::Value::from(n));
                        }
                        if let Some(n) = c.rowspan {
                            m.insert("rowspan".to_string(), serde_json::Value::from(n));
                        }
                        serde_json::Value::Object(m)
                    })
                    .collect();
                let mut m = serde_json::Map::new();
                m.insert("cells".to_string(), serde_json::Value::from(cells));
                if r.rule_above {
                    m.insert("rule_above".to_string(), serde_json::Value::from(true));
                }
                serde_json::Value::Object(m)
            })
            .collect();
        map.insert("rows".to_string(), serde_json::Value::from(rows));
        if let Some(ref src) = t.latex_source {
            map.insert(
                "latex_source".to_string(),
                serde_json::Value::from(src.clone()),
            );
        }
    }
    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map).to_string())
    }
}

/// 収集した block ノードのビューから参照グラフ（Phase 6a）を解決し、`node_relations` に挿入する。
/// build のトランザクション内で（全ノード挿入後・commit 前に）呼ぶ。抽出は純関数
/// （`graph::resolve_relations`）で、原文由来（TeX）と推定（PDF）を strategy で切り替える。
async fn insert_relations_for_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    version_id: i64,
    graph_nodes: &[graph::GraphNode],
    strategy: graph::RefStrategy,
) -> Result<(), String> {
    for edge in graph::resolve_relations(graph_nodes, strategy) {
        node_relations::insert_relation(
            &mut **tx,
            &node_relations::NewNodeRelation {
                document_version_id: version_id,
                from_node_id: edge.from_node_id,
                relation_type: edge.relation_type.as_str(),
                to_node_id: edge.to_node_id,
                confidence: Some(edge.confidence),
                origin: Some(edge.origin.as_str()),
                metadata_json: edge.metadata_json.as_deref(),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 収集した block ノードのビューから記号定義（Phase 6b・TeX のみ）を抽出し、`symbols` /
/// `symbol_occurrences` に挿入する。build のトランザクション内で（全ノード挿入後・commit 前に）
/// 呼ぶ。抽出は純関数（`symbols::extract_symbols`）。origin=tex_source（表層は原文由来・対応づけ
/// はヒューリスティックなので confidence で区別）。
async fn insert_symbols_for_version(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    version_id: i64,
    symbol_nodes: &[symbols::SymbolNode],
) -> Result<(), String> {
    let (extracted, occurrences) = symbols::extract_symbols(symbol_nodes);
    let mut symbol_ids: Vec<i64> = Vec::with_capacity(extracted.len());
    for s in &extracted {
        let id = crate::db::symbols::insert_symbol(
            &mut **tx,
            &crate::db::symbols::NewSymbol {
                document_version_id: version_id,
                surface_form: &s.surface_form,
                normalized_form: s.normalized_form.as_deref(),
                description: s.description.as_deref(),
                symbol_type: s.symbol_type.map(|t| t.as_str()),
                defined_at_node_id: Some(s.defined_at_node_id),
                scope_node_id: s.scope_node_id,
                semantic_json: None,
                confidence: Some(s.confidence),
                origin: Some(Origin::TexSource.as_str()),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        symbol_ids.push(id);
    }
    for o in &occurrences {
        crate::db::symbols::insert_occurrence(
            &mut **tx,
            &crate::db::symbols::NewSymbolOccurrence {
                symbol_id: symbol_ids[o.symbol_index],
                node_id: o.node_id,
                local_offset_json: None,
                surface_form: &o.surface_form,
                confidence: Some(o.confidence),
                origin: Some(Origin::TexSource.as_str()),
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// ブロックの型固有属性（見出し階層・節番号・定理番号・付記名）を payload_json にする。無ければ None。
fn block_payload_json(b: &structure::StructuredBlock) -> Option<String> {
    let mut map = serde_json::Map::new();
    if let Some(level) = b.heading_level {
        map.insert("heading_level".to_string(), serde_json::Value::from(level));
    }
    if let Some(ref number) = b.section_number {
        map.insert(
            "section_number".to_string(),
            serde_json::Value::from(number.clone()),
        );
    }
    if let Some(ref number) = b.theorem_number {
        map.insert(
            "theorem_number".to_string(),
            serde_json::Value::from(number.clone()),
        );
    }
    if let Some(ref note) = b.note {
        map.insert("note".to_string(), serde_json::Value::from(note.clone()));
    }
    if let Some(ref label) = b.caption_label {
        map.insert(
            "caption_label".to_string(),
            serde_json::Value::from(label.clone()),
        );
    }
    if let Some(ref number) = b.caption_number {
        map.insert(
            "caption_number".to_string(),
            serde_json::Value::from(number.clone()),
        );
    }
    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map).to_string())
    }
}

/// 完了 LCIR がまだ無い PDF 添付を洗い出し、順に構築する（過去分・失敗分の後追い）。
/// フラグ OFF なら `enabled: false` で即返す。既存 `index_missing_attachments` の LCIR 版。
pub async fn build_missing_lcir<F: Fn(i64, i64)>(
    pool: &SqlitePool,
    app_data_dir: &Path,
    on_progress: F,
) -> Result<LcirBatchResult, String> {
    if !lcir_enabled(pool).await {
        return Ok(disabled_batch());
    }
    let targets = document_versions::attachments_without_completed_lcir(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(run_build_batch(pool, app_data_dir, targets, on_progress).await)
}

/// 現行より古い抽出器版（例 Phase 1 の 0.1.0）で作られた LCIR を、現行版へ再構築する。
/// 抽出ロジックを上げた後、既存コーパスに新しい構造認識を行き渡らせるためのバッチ。
///
/// **抽出器と mime フィルタは必ずペアで渡す**（Phase 4）: 「outdated」は同一抽出器系列の
/// 中でだけ意味を持つ。pdfium 版で構築済みの PDF を「TeX 版が無いから outdated」と誤判定
/// して全コーパス再抽出する事故を防ぐ。フラグ OFF なら `enabled: false` で即返す。
pub async fn rebuild_outdated_lcir<F: Fn(i64, i64)>(
    pool: &SqlitePool,
    app_data_dir: &Path,
    on_progress: F,
) -> Result<LcirBatchResult, String> {
    if !lcir_enabled(pool).await {
        return Ok(disabled_batch());
    }
    let mut targets = document_versions::attachments_with_outdated_lcir(
        pool,
        document_ir::schema::EXTRACTOR_NAME,
        document_ir::schema::EXTRACTOR_VERSION,
        "%pdf%",
    )
    .await
    .map_err(|e| e.to_string())?;
    targets.extend(
        document_versions::attachments_with_outdated_lcir(
            pool,
            document_ir::schema::TEX_EXTRACTOR_NAME,
            document_ir::schema::TEX_EXTRACTOR_VERSION,
            TEX_SOURCE_MIME,
        )
        .await
        .map_err(|e| e.to_string())?,
    );
    Ok(run_build_batch(pool, app_data_dir, targets, on_progress).await)
}

fn disabled_batch() -> LcirBatchResult {
    LcirBatchResult {
        enabled: false,
        total: 0,
        built: 0,
        reused: 0,
        failed: 0,
    }
}

/// 対象添付を順に build して集計する。`build_missing_lcir` / `rebuild_outdated_lcir` が共有。
/// `on_progress(done, total)` は 1 添付ぶん処理するたびに呼ぶ。**数十分かかりうる**バッチ
/// （既存コーパスの再構築は PDF 1 本ごとに pdfium 抽出 + ページレンダ + crop 書き出し）なので、
/// 呼び出し側が進捗イベントに変換して「固まって見える」のを避けられるようにする。
async fn run_build_batch<F: Fn(i64, i64)>(
    pool: &SqlitePool,
    app_data_dir: &Path,
    targets: Vec<(i64, String)>,
    on_progress: F,
) -> LcirBatchResult {
    let total = targets.len() as i64;
    let (mut built, mut reused, mut failed) = (0i64, 0i64, 0i64);
    for (i, (att_id, _path)) in targets.into_iter().enumerate() {
        match build_lcir_for_attachment(pool, app_data_dir, att_id).await {
            Ok(r) if r.built => built += 1,
            Ok(r) if r.reused => reused += 1,
            Ok(_) => {}
            Err(e) => {
                eprintln!("LCIR: batch build failed for attachment {att_id}: {e}");
                failed += 1;
            }
        }
        on_progress(i as i64 + 1, total);
    }
    LcirBatchResult {
        enabled: true,
        total,
        built,
        reused,
        failed,
    }
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
    // ページ FTS は pdfium 版のみ（TeX 版は page ノードを持たず、`fulltext` はページ粒度の
    // PDF 検索インデックスなので触らない）。
    if version.extractor_name != document_ir::schema::EXTRACTOR_NAME {
        return Ok(0);
    }
    let pages = document_nodes::page_nodes_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?;
    // page ノードの ordinal は 0 始まり。fulltext.page は 1 始まり（= ordinal + 1）。
    // **保険でここでも C0 を落とす**（debt-22）。保存側（`insert_pdf_version_tx`）で既に
    // 落としているので通常は恒等だが、**#7 の再構築より前に p1 を回すと実 DB には
    // 抽出器 0.13.0 以前の汚れた page が残っている**ため、索引にだけは持ち込まない。
    let rows: Vec<(i64, String)> = pages
        .into_iter()
        .filter_map(|p| {
            p.plain_text
                .map(|t| (p.ordinal + 1, structure::clean_page_text(&t)))
        })
        .collect();
    let n = rows.len() as i64;
    fulltext::index_attachment(pool, attachment_id, &rows)
        .await
        .map_err(|e| e.to_string())?;
    Ok(n)
}

/// LCIR の block ノード（段落・見出し・caption 等）から `document_nodes_fts`（ノード単位 FTS・
/// Phase 2）を再生成する。`regenerate_page_fts_from_lcir` のノード粒度版。
///
/// これは追加の派生索引（既存 `fulltext` のページ検索とは別物）で、build 後に呼んで検索可能に
/// する。LCIR が無ければ node-FTS をクリアして 0 を返す。反映したノード数を返す。
pub async fn regenerate_node_fts_from_lcir(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<i64, String> {
    let version = match document_versions::latest_completed_for_attachment(pool, attachment_id)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(v) => v,
        None => {
            // LCIR が無い添付は node-FTS も空にする（古い索引が残らないよう掃除）。
            document_nodes_fts::unindex_attachment(pool, attachment_id)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(0);
        }
    };
    // TeX 版（Phase 4）は node-FTS に載せない: 同一エントリの PDF 版と本文が重複ヒットし、
    // bbox も持たないため（検索 = PDF 版 / 読み出し = TeX 優先の分担・design overview §8）。
    if version.extractor_name != document_ir::schema::EXTRACTOR_NAME {
        document_nodes_fts::unindex_attachment(pool, attachment_id)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(0);
    }
    let rows = document_nodes::indexable_nodes_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?;
    let inputs: Vec<NodeFtsInput> = rows
        .into_iter()
        .map(|(node_id, node_kind, content, page)| NodeFtsInput {
            node_id,
            page,
            node_kind,
            content,
        })
        .collect();
    let n = inputs.len() as i64;
    document_nodes_fts::index_nodes(pool, attachment_id, &inputs)
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
    load_lcir_document_for_version(pool, version).await.map(Some)
}

/// 版を指定して LCIR 派生ビューを組む。`load_lcir_document` の実体で、node 起点の read 面
/// （Phase 10a）は「呼び出し側が持っているノードが載っている版」をこちらで直接読む
/// （添付の最新版に読み替えると、superseded 版のノード id が引けなくなるため）。
pub async fn load_lcir_document_for_version(
    pool: &SqlitePool,
    version: crate::models::DocumentVersion,
) -> Result<document_ir::LcirDocument, String> {
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

    // 数式ノードには表層表現（math_expressions）を紐づける（Phase 3）。
    let mut math_by_node: HashMap<i64, document_ir::LcirMath> = HashMap::new();
    for m in math_expressions::math_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?
    {
        math_by_node.insert(
            m.node_id,
            document_ir::LcirMath {
                display_mode: m.display_mode,
                equation_label: m.equation_label,
                latex: m.latex,
                presentation_mathml: m.presentation_mathml,
                content_mathml: m.content_mathml,
                openmath: m.openmath_json,
                normalized_text: m.normalized_text,
                semantic_status: m.semantic_status,
                confidence: m.confidence,
                origin: m.origin,
            },
        );
    }

    // 図表アセット（Phase 8a）を node_assets 経由でノードに紐づける。
    // relative_path はメタデータ参照でファイルの存在は保証しない（欠損許容）。
    let asset_rows: HashMap<i64, crate::models::Asset> =
        crate::db::assets::assets_for_version(pool, version.id)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| (a.id, a))
            .collect();
    let mut assets_by_node: HashMap<i64, Vec<document_ir::LcirAsset>> = HashMap::new();
    for link in crate::db::assets::node_assets_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?
    {
        if let Some(a) = asset_rows.get(&link.asset_id) {
            assets_by_node
                .entry(link.node_id)
                .or_default()
                .push(document_ir::LcirAsset {
                    role: link.role,
                    mime_type: a.mime_type.clone(),
                    relative_path: a.relative_path.clone(),
                    width: a.width,
                    height: a.height,
                    size_bytes: a.size_bytes,
                    sha256: a.sha256.clone(),
                    metadata: a
                        .metadata_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str(s).ok()),
                });
        }
    }

    // 代替テキスト（Phase 8c）をノードに紐づける。1 ノードに生成 1 件 + 手編集 1 件までなので、
    // **手編集を優先**して 1 つだけ載せる（AI 生成が人の記述を隠さない）。
    let mut alt_text_by_node: HashMap<i64, crate::models::NodeAltText> = HashMap::new();
    for a in crate::db::node_alt_texts::alt_texts_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?
    {
        match alt_text_by_node.get(&a.node_id) {
            Some(prev) if prev.origin == document_ir::Origin::UserEdited.as_str() => {}
            _ => {
                alt_text_by_node.insert(a.node_id, a);
            }
        }
    }

    let lcir_nodes = nodes
        .into_iter()
        .map(|n| document_ir::LcirNode {
            source_fragments: by_node.remove(&n.id).unwrap_or_default(),
            math: math_by_node.remove(&n.id),
            assets: assets_by_node.remove(&n.id).unwrap_or_default(),
            alt_text: alt_text_by_node
                .remove(&n.id)
                .map(|a| document_ir::LcirAltText {
                    text: a.text,
                    origin: a.origin,
                    confidence: a.confidence,
                    model: a.model,
                    source_asset_sha256: a.source_asset_sha256,
                    carried_from_version_id: a.carried_from_version_id,
                }),
            payload: n
                .payload_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            id: n.id,
            kind: n.node_kind,
            ordinal: n.ordinal,
            parent_id: n.parent_id,
            plain_text: n.plain_text,
            origin: n.origin,
            confidence: n.confidence,
        })
        .collect();

    // ノード間の型付き関係（Phase 6a・参照グラフ）を版単位で載せる。
    let relations = node_relations::relations_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| document_ir::LcirRelation {
            from_node_id: r.from_node_id,
            relation_type: r.relation_type,
            to_node_id: r.to_node_id,
            confidence: r.confidence,
            origin: r.origin,
            metadata: r
                .metadata_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
        })
        .collect();

    // 記号定義とその出現（Phase 6b・記号系）を版単位で載せる。出現は symbol_id でまとめる。
    let mut occ_by_symbol: HashMap<i64, Vec<document_ir::LcirSymbolOccurrence>> = HashMap::new();
    for o in crate::db::symbols::occurrences_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?
    {
        occ_by_symbol
            .entry(o.symbol_id)
            .or_default()
            .push(document_ir::LcirSymbolOccurrence {
                node_id: o.node_id,
                surface_form: o.surface_form,
                confidence: o.confidence,
                origin: o.origin,
            });
    }
    let symbol_list = crate::db::symbols::symbols_for_version(pool, version.id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|s| document_ir::LcirSymbol {
            occurrences: occ_by_symbol.remove(&s.id).unwrap_or_default(),
            id: s.id,
            surface_form: s.surface_form,
            normalized_form: s.normalized_form,
            description: s.description,
            symbol_type: s.symbol_type,
            defined_at_node_id: s.defined_at_node_id,
            scope_node_id: s.scope_node_id,
            confidence: s.confidence,
            origin: s.origin,
        })
        .collect();

    // TeX 版（Phase 4）は PDF 座標を持たないので coordinate_space を主張しない。
    let coordinate_space = if version.extractor_name == document_ir::schema::EXTRACTOR_NAME {
        Some(CoordinateSpace::default())
    } else {
        None
    };
    Ok(document_ir::LcirDocument {
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
        coordinate_space,
        nodes: lcir_nodes,
        relations,
        symbols: symbol_list,
    })
}

/// ノード id 起点で LCIR を読む（Phase 10a の入口）。返すのは `(version, entry_id, doc)`
/// （`attachment_id` は `version` が持つ）。ノードが存在しなければ `None`。
///
/// `load_entry_lcir`（エントリ起点・tex > pdf 優先）と対になる関数だが、**優先度で版を
/// 選ばない** — ノード id がどの版の話かをすでに決めているため。`source` 引数も要らない。
pub async fn load_node_lcir(
    pool: &SqlitePool,
    node_id: i64,
) -> Result<Option<(crate::models::DocumentVersion, i64, document_ir::LcirDocument)>, String> {
    let Some(version_id) = document_nodes::version_id_for_node(pool, node_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let Some(version) = document_versions::find_by_id(pool, version_id)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    let attachment_id = version.attachment_id;
    let entry_id = crate::db::attachments::get_attachment(pool, attachment_id)
        .await
        .map_err(|e| e.to_string())?
        .entry_id;
    let doc = load_lcir_document_for_version(pool, version.clone()).await?;
    Ok(Some((version, entry_id, doc)))
}

// ---- エントリ→版解決（Phase 4 の read 優先度。MCP / エクスポート / CLI で共有） ----

/// エントリの「添付ごとの最新 completed 版」を全部返す。並びは **read 優先度降順**
/// （`extractor_priority`: tex > pdfium）→ attachment_id 昇順。併存する複数表現の列挙と
/// 既定選択の単一ソース。
pub async fn entry_lcir_versions(
    pool: &SqlitePool,
    entry_id: i64,
) -> Result<Vec<crate::models::DocumentVersion>, sqlx::Error> {
    let mut versions = sqlx::query_as::<_, crate::models::DocumentVersion>(
        "SELECT dv.* FROM document_versions dv
         JOIN attachments a ON a.id = dv.attachment_id
         WHERE a.entry_id = ?
           AND dv.extraction_status IN ('completed', 'completed_with_warnings')
           AND dv.id = (
               SELECT MAX(dv2.id) FROM document_versions dv2
               WHERE dv2.attachment_id = dv.attachment_id
                 AND dv2.extraction_status IN ('completed', 'completed_with_warnings')
           )
         ORDER BY dv.attachment_id",
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?;
    versions.sort_by(|a, b| {
        document_ir::schema::extractor_priority(&b.extractor_name)
            .cmp(&document_ir::schema::extractor_priority(&a.extractor_name))
            .then(a.attachment_id.cmp(&b.attachment_id))
    });
    Ok(versions)
}

/// `source` 引数（"tex"/"pdf"）→ extractor_name。エラーはそのままユーザーに見せる文言。
pub fn source_to_extractor(source: &str) -> Result<&'static str, String> {
    match source {
        "tex" => Ok(document_ir::schema::TEX_EXTRACTOR_NAME),
        "pdf" => Ok(document_ir::schema::EXTRACTOR_NAME),
        other => Err(format!("unknown source '{other}' (use \"tex\" or \"pdf\")")),
    }
}

/// extractor_name → 短い source 名（"tex"/"pdf"）。未知の抽出器名はそのまま返す。
pub fn short_source_name(extractor_name: &str) -> &str {
    match extractor_name {
        document_ir::schema::TEX_EXTRACTOR_NAME => "tex",
        document_ir::schema::EXTRACTOR_NAME => "pdf",
        other => other,
    }
}

/// エントリの LCIR を読む。`wanted_extractor`（extractor_name・`source_to_extractor` で
/// 解決済み）指定時はその抽出器の版に限定し、未指定なら優先度順（tex > pdfium）で最初に
/// 読めた版を返す。読めた/読めないに関わらず併存する版の一覧も返す — 「無かったとき」の
/// 案内文を実在する表現に基づいて組み立てるため。
#[allow(clippy::type_complexity)]
pub async fn load_entry_lcir(
    pool: &SqlitePool,
    entry_id: i64,
    wanted_extractor: Option<&str>,
) -> Result<
    (
        Option<(i64, document_ir::LcirDocument)>,
        Vec<crate::models::DocumentVersion>,
    ),
    String,
> {
    let versions = entry_lcir_versions(pool, entry_id)
        .await
        .map_err(|e| e.to_string())?;
    for v in &versions {
        if let Some(name) = wanted_extractor {
            if v.extractor_name != name {
                continue;
            }
        }
        if let Some(doc) = load_lcir_document(pool, v.attachment_id).await? {
            return Ok((Some((v.attachment_id, doc)), versions));
        }
    }
    Ok((None, versions))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;
    use std::path::PathBuf;

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

    /// 一括バッチは 1 添付ぶん処理するたびに進捗コールバックを呼ぶ（存在しない添付でも
    /// 失敗として数えて前進する = 1 件の失敗でバッチ全体を止めない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn build_batch_reports_progress_and_survives_failures(pool: SqlitePool) {
        settings::set_setting(&pool, settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
        let seen = std::sync::Mutex::new(Vec::<(i64, i64)>::new());
        let res = run_build_batch(
            &pool,
            Path::new("/nonexistent"),
            vec![(9001, "a.pdf".to_string()), (9002, "b.pdf".to_string())],
            |done, total| seen.lock().unwrap().push((done, total)),
        )
        .await;
        assert_eq!(res.total, 2);
        assert_eq!(res.failed, 2, "存在しない添付は失敗として数える");
        assert_eq!(res.built, 0);
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(1, 2), (2, 2)],
            "1 添付ごとに (done, total) が通知される"
        );
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
        let r = build_missing_lcir(&pool, Path::new("/nonexistent"), |_, _| {})
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
        let r = build_missing_lcir(&pool, Path::new("/nonexistent"), |_, _| {})
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

    /// debt-22 の保険: **抽出器 0.13.0 以前で保存された汚れた page** を索引に持ち込まない。
    ///
    /// p1（#8）は #7 の再構築より**後**に入るが、それまでの間に手で回されうるし、
    /// 実 DB には 0.6.0 時点の page が 138 版ぶん残っている。索引側にも同じ正規化を重ねて、
    /// **`fulltext` の汚染率が 13.3% → 78.8% に跳ねる**のを防ぐ。
    #[sqlx::test(migrations = "./migrations")]
    async fn page_fts_regeneration_strips_c0_from_legacy_page_nodes(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = document_versions::insert_version(
            &pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-legacy",
                schema_version: document_ir::schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: document_ir::schema::EXTRACTOR_NAME,
                extractor_version: "0.6.0", // 実 DB に残っている版
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
        // 語の内側に C0 が刺さった保存値（実データの形）。
        document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("The result is consis\u{2}tent\r\nwith theory."),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(regenerate_page_fts_from_lcir(&pool, att).await.unwrap(), 1);
        let content: String =
            sqlx::query_scalar("SELECT content FROM fulltext WHERE attachment_id = ?")
                .bind(att)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(content, "The result is consistent\nwith theory.");
        // 割れていた語が引けるようになる（trigram 索引に C0 が入ると落ちる）。
        let hits = fulltext::search_fulltext(&pool, "consistent", None, None, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "C0 が残っていると語が割れて引けない");
    }

    /// 手組みの LCIR（page > block > line）から node-FTS を再生成でき、block だけが索引され
    /// （page/line/document は除外）、ヒットに node_kind と bbox が付く。Phase 2 の実証（CI 可能）。
    #[sqlx::test(migrations = "./migrations")]
    async fn regenerate_node_fts_indexes_blocks_not_skeleton(pool: SqlitePool) {
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
        let page = document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("full page text with transformer somewhere"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let para = document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(page),
                node_kind: NodeKind::Paragraph.as_str(),
                ordinal: 0,
                plain_text: Some("Transformer architecture is explained here"),
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        // 段落の block fragment（ハイライト領域）。
        source_fragments::insert_fragment(
            &pool,
            &NewSourceFragment {
                node_id: para,
                page_number: 1,
                x: 72.0,
                y: 600.0,
                width: 400.0,
                height: 24.0,
                rotation: 0.0,
                reading_order: Some(0),
                fragment_type: Some(FragmentType::Block.as_str()),
            },
        )
        .await
        .unwrap();
        // 行ノード（索引対象外であることの確認用）。
        document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(para),
                node_kind: NodeKind::Line.as_str(),
                ordinal: 0,
                plain_text: Some("Transformer architecture is explained here"),
                language: None,
                confidence: None,
                origin: Some("pdf_text_layer"),
                payload_json: None,
            },
        )
        .await
        .unwrap();

        let n = regenerate_node_fts_from_lcir(&pool, att).await.unwrap();
        assert_eq!(n, 1, "block(paragraph) だけ索引・page/line/document は除外");

        let hits = document_nodes_fts::search_nodes(&pool, "transformer", None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].node_id, para);
        assert_eq!(hits[0].node_kind, "paragraph");
        let bbox = hits[0].bbox.as_ref().expect("block fragment → bbox");
        assert_eq!(bbox.y, 600.0);
    }

    /// LCIR が無い添付では node-FTS が空になり、既存の索引もクリアされる。
    #[sqlx::test(migrations = "./migrations")]
    async fn regenerate_node_fts_clears_when_no_lcir(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        // 先に手動で 1 行入れておく（古い索引が残っているケース）。
        document_nodes_fts::index_nodes(
            &pool,
            att,
            &[NodeFtsInput {
                node_id: 1,
                page: 1,
                node_kind: "paragraph".to_string(),
                content: "stale leftover row".to_string(),
            }],
        )
        .await
        .unwrap();

        let n = regenerate_node_fts_from_lcir(&pool, att).await.unwrap();
        assert_eq!(n, 0);
        assert!(document_nodes_fts::search_nodes(&pool, "stale", None, None, None, None, None)
            .await
            .unwrap()
            .is_empty());
    }

    /// TeX ソース添付（application/gzip）の build 一式（CI 実行可能・pdfium 不要）:
    /// mime ディスパッチ → gzip 展開 → フラット木（page/line/fragment 無し）→ 原文 LaTeX の
    /// math_expressions → node-FTS 非索引 → 冪等 reuse。
    #[sqlx::test(migrations = "./migrations")]
    async fn build_tex_attachment_end_to_end(pool: SqlitePool) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        settings::set_setting(&pool, settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Tex Paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let file_name = "arxiv-2301.00001-source.gz";
        let rel = format!("attachments/{}/{}", entry.id, file_name);
        let att = add_attachment(&pool, entry.id, &rel, file_name, TEX_SOURCE_MIME)
            .await
            .unwrap()
            .id;

        // gzip した単一 .tex をテンポラリ app_data_dir に配置。
        let root = std::env::temp_dir().join(format!("lcir-tex-e2e-{}", std::process::id()));
        let dir = root.join("attachments").join(entry.id.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let tex = "\\documentclass{article}\\title{Tex Paper}\\begin{document}\n\
                   \\begin{abstract}\nAbout transformers.\n\\end{abstract}\n\
                   \\section{Intro}\nBody text here. Let $E$ be the total energy of the system.\n\
                   \\begin{equation}\\label{eq:e}E=mc^2\\end{equation}\n\
                   \\begin{table}\\caption{Masses.}\\label{tab:m}\n\
                   \\begin{tabular}{lc}\\hline Particle & Mass \\\\ \\hline e & 0.511 \\\\ \\hline\\end{tabular}\n\
                   \\end{table}\n\
                   \\end{document}";
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(tex.as_bytes()).unwrap();
        std::fs::write(dir.join(file_name), enc.finish().unwrap()).unwrap();

        let res = build_lcir_for_attachment(&pool, &root, att).await.unwrap();
        assert!(res.enabled && res.built && !res.reused, "{res:?}");
        assert_eq!(res.page_count, 0, "TeX 版に page は無い");
        assert!(res.message.contains("block(s)"), "{}", res.message);

        let doc = load_lcir_document(&pool, att).await.unwrap().unwrap();
        assert_eq!(doc.source.extractor_name, document_ir::schema::TEX_EXTRACTOR_NAME);
        assert!(doc.coordinate_space.is_none(), "TeX 版は座標系を主張しない");
        assert!(doc.nodes.iter().all(|n| n.kind != "page" && n.kind != "line"));
        assert!(doc.nodes.iter().all(|n| n.source_fragments.is_empty()));
        assert!(doc.nodes.iter().any(|n| n.kind == "front_matter"));
        assert!(doc.nodes.iter().any(|n| n.kind == "abstract"));
        assert!(doc.nodes.iter().any(|n| n.kind == "section"));
        let math = doc.nodes.iter().find(|n| n.kind == "display_math").unwrap();
        let m = math.math.as_ref().expect("math row for display_math");
        assert!(m.latex.as_deref().unwrap().contains("E=mc^2"));
        assert_eq!(m.semantic_status, "source_provided");
        assert_eq!(m.origin.as_deref(), Some("tex_source"));
        assert_eq!(math.origin.as_deref(), Some("tex_source"));
        assert!(document_ir::validation::validate(&doc).is_ok());

        // Phase 6b: 定義文 "Let $E$ be ..." から記号 E を抽出し、数式 $E=mc^2$ に出現を張る。
        let sym = doc
            .symbols
            .iter()
            .find(|s| s.surface_form == "E")
            .expect("symbol E extracted from definition sentence");
        assert_eq!(sym.description.as_deref(), Some("the total energy of the system"));
        assert_eq!(sym.origin.as_deref(), Some("tex_source"));
        assert!(sym.defined_at_node_id.is_some(), "定義ノードが紐づく");
        assert!(
            sym.occurrences.iter().any(|o| o.node_id == math.id),
            "記号 E は display 数式 E=mc^2 に出現する"
        );

        // Phase 8b: tabular がセル構造つき table ノードになり、caption_of 辺で結ばれる。
        let table = doc
            .nodes
            .iter()
            .find(|n| n.kind == "table")
            .expect("table node from tabular");
        assert_eq!(table.origin.as_deref(), Some("tex_source"));
        assert_eq!(table.plain_text.as_deref(), Some("Particle | Mass\ne | 0.511"));
        let payload = table.payload.as_ref().expect("table payload");
        assert_eq!(payload.get("n_columns").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(payload.get("n_rows").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(
            payload.get("column_spec").and_then(|v| v.as_str()),
            Some("lc")
        );
        let rows = payload.get("rows").and_then(|v| v.as_array()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1]["cells"][1].get("text").and_then(|v| v.as_str()),
            Some("0.511")
        );
        assert_eq!(rows[1].get("rule_above").and_then(|v| v.as_bool()), Some(true));
        assert!(payload
            .get("latex_source")
            .and_then(|v| v.as_str())
            .unwrap()
            .starts_with("\\begin{tabular}"));
        let caption = doc
            .nodes
            .iter()
            .find(|n| n.kind == "table_caption")
            .expect("table caption");
        let cap_edge = doc
            .relations
            .iter()
            .find(|r| r.relation_type == "caption_of" && r.to_node_id == table.id)
            .expect("caption_of edge for table");
        assert_eq!(cap_edge.from_node_id, caption.id);
        assert_eq!(cap_edge.origin.as_deref(), Some("tex_source"));
        // labels は caption 側 → \ref{tab:m} は refers_to_table として解決される想定
        //（この文書には \ref が無いので辺は張られない）。metadata に table_count が載る。

        // TeX 版は node-FTS に載らない。
        let fts_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM document_nodes_fts WHERE attachment_id = ?")
                .bind(att)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(fts_rows, 0, "TeX 版は node-FTS 非索引");

        // 冪等: 同一バイトの再 build は reuse（抽出も走らない）。
        let again = build_lcir_for_attachment(&pool, &root, att).await.unwrap();
        assert!(again.reused, "{again:?}");
        assert_eq!(again.content_key, res.content_key);

        std::fs::remove_dir_all(&root).ok();
    }

    /// LCIR 対象外の mime は明示エラー（バッチ対象クエリと同一述語のディスパッチ）。
    #[sqlx::test(migrations = "./migrations")]
    async fn build_rejects_unsupported_mime(pool: SqlitePool) {
        settings::set_setting(&pool, settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "P".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = add_attachment(&pool, entry.id, "attachments/x/notes.txt", "notes.txt", "text/plain")
            .await
            .unwrap()
            .id;
        let err = build_lcir_for_attachment(&pool, Path::new("/nonexistent"), att)
            .await
            .unwrap_err();
        assert!(err.contains("unsupported"), "{err}");
    }

    /// 最新版が TeX（非 pdfium）の添付では node-FTS を張らず、古い索引もクリアする（Phase 4）。
    #[sqlx::test(migrations = "./migrations")]
    async fn regenerate_node_fts_skips_tex_versions(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = document_versions::insert_version(
            &pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: "ck-tex",
                schema_version: document_ir::schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: TEX_SOURCE_MIME,
                extractor_name: document_ir::schema::TEX_EXTRACTOR_NAME,
                extractor_version: document_ir::schema::TEX_EXTRACTOR_VERSION,
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
                origin: Some("tex_source"),
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
                node_kind: NodeKind::Paragraph.as_str(),
                ordinal: 0,
                plain_text: Some("tex paragraph text"),
                language: None,
                confidence: Some(0.9),
                origin: Some("tex_source"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        // 古い索引が残っているケースを模す。
        document_nodes_fts::index_nodes(
            &pool,
            att,
            &[NodeFtsInput {
                node_id: 999,
                page: 1,
                node_kind: "paragraph".to_string(),
                content: "stale pdf row".to_string(),
            }],
        )
        .await
        .unwrap();

        let n = regenerate_node_fts_from_lcir(&pool, att).await.unwrap();
        assert_eq!(n, 0, "TeX 版は索引しない");
        assert!(document_nodes_fts::search_nodes(&pool, "stale", None, None, None, None, None)
            .await
            .unwrap()
            .is_empty());
        assert!(document_nodes_fts::search_nodes(&pool, "tex paragraph", None, None, None, None, None)
            .await
            .unwrap()
            .is_empty());
        // ページ FTS 側も TeX 版では何もしない。
        assert_eq!(regenerate_page_fts_from_lcir(&pool, att).await.unwrap(), 0);
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

    /// display_math ノードに紐づく math_expressions が、read 面（LcirNode.math）へ組み上がる
    /// （Phase 3 の表層表現・PDF 由来は semantic_status='surface_only'）。
    #[sqlx::test(migrations = "./migrations")]
    async fn load_lcir_document_includes_math(pool: SqlitePool) {
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
        let eq = document_nodes::insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::DisplayMath.as_str(),
                ordinal: 0,
                plain_text: Some("U = S2 C2 S1 C1"),
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        math_expressions::insert_math(
            &pool,
            &math_expressions::NewMathExpression {
                node_id: eq,
                display_mode: "display",
                equation_label: Some("(2.1)"),
                latex: None,
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("U = S2 C2 S1 C1"),
                ast_json: None,
                semantic_status: document_ir::MathSemanticStatus::SurfaceOnly.as_str(),
                confidence: Some(0.6),
                origin: Some("pdf_text_layer"),
            },
        )
        .await
        .unwrap();

        let doc = load_lcir_document(&pool, att).await.unwrap().unwrap();
        let math_node = doc.nodes.iter().find(|n| n.kind == "display_math").unwrap();
        let math = math_node.math.as_ref().expect("display_math ノードは math を持つ");
        assert_eq!(math.display_mode, "display");
        assert_eq!(math.equation_label.as_deref(), Some("(2.1)"));
        assert_eq!(math.semantic_status, "surface_only");
        assert_eq!(math.normalized_text.as_deref(), Some("U = S2 C2 S1 C1"));
        assert!(math.latex.is_none(), "PDF 由来では LaTeX 未確定");
        // 非数式ノードには math が付かない。
        assert!(doc.nodes.iter().find(|n| n.kind == "document").unwrap().math.is_none());
    }

    /// Phase 8d-7: PDF 本文の "Figure 3" が図表番号と照合され、`node_relations` に永続化されて
    /// read 面（`LcirDocument.relations`）まで出ること。純関数側（`graph.rs`）で解決規則は
    /// 押さえてあるので、ここは **DB 経路の配線**（figure ノードが索引に載る / metadata が
    /// 往復する / caption の自己言及が辺にならない）を守るのが役目。
    #[sqlx::test(migrations = "./migrations")]
    async fn insert_relations_persists_pdf_figure_reference(pool: SqlitePool) {
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
        let new_node = |kind: NodeKind, ordinal: i64, text: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                document_nodes::insert_node(
                    &pool,
                    &NewDocumentNode {
                        document_version_id: vid,
                        parent_id: None,
                        node_kind: kind.as_str(),
                        ordinal,
                        plain_text: text,
                        language: None,
                        confidence: Some(0.6),
                        origin: Some("layout_model"),
                        payload_json: None,
                    },
                )
                .await
                .unwrap()
            }
        };
        let cap = new_node(NodeKind::FigureCaption, 0, Some("Figure 3: Overview.")).await;
        let fig = new_node(NodeKind::Figure, 1, None).await;
        let para = new_node(NodeKind::Paragraph, 2, Some("The pipeline is shown in Figure 3.")).await;

        let graph_nodes = vec![
            graph::GraphNode {
                id: cap,
                kind: NodeKind::FigureCaption,
                reading_index: 0,
                plain_text: "Figure 3: Overview.".to_string(),
                labels: Vec::new(),
                equation_label: None,
                theorem_number: None,
                cite_key: None,
                caption_label: Some("Figure".to_string()),
                caption_number: Some("3".to_string()),
            },
            graph::GraphNode {
                id: fig,
                kind: NodeKind::Figure,
                reading_index: 1,
                plain_text: String::new(),
                labels: Vec::new(),
                equation_label: None,
                theorem_number: None,
                cite_key: None,
                caption_label: None,
                caption_number: Some("3".to_string()),
            },
            graph::GraphNode {
                id: para,
                kind: NodeKind::Paragraph,
                reading_index: 2,
                plain_text: "The pipeline is shown in Figure 3.".to_string(),
                labels: Vec::new(),
                equation_label: None,
                theorem_number: None,
                cite_key: None,
                caption_label: None,
                caption_number: None,
            },
        ];
        let mut tx = pool.begin().await.unwrap();
        insert_relations_for_version(&mut tx, vid, &graph_nodes, graph::RefStrategy::Pdf)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let doc = load_lcir_document(&pool, att).await.unwrap().unwrap();
        let figure_refs: Vec<_> = doc
            .relations
            .iter()
            .filter(|r| r.relation_type == "refers_to_figure")
            .collect();
        assert_eq!(figure_refs.len(), 1, "本文からの図参照 1 本だけ: {figure_refs:?}");
        let e = figure_refs[0];
        assert_eq!(e.from_node_id, para);
        assert_eq!(e.to_node_id, fig, "番号を持つ figure ノードに解決する");
        assert_eq!(e.origin.as_deref(), Some("layout_model"));
        assert_eq!(
            e.metadata.as_ref().and_then(|m| m.get("resolved_via")),
            Some(&serde_json::Value::from("node")),
            "metadata が DB を往復する"
        );
    }

    /// 手動 pdfium 実機確認: 実 DB コピー + 実 PDF に対して build → load → 冪等 build を走らせる。
    /// native lib（`src-tauri/pdfium/libpdfium.dylib`）が要るため `#[ignore]`。env 未設定なら skip。
    ///
    /// **LCIR_SMOKE_APPDIR は「コピー元」**（実 appdir・読むだけ）。テストは一時ディレクトリに
    /// 対象添付だけをコピーしてそこへ build するので、実 appdir に Phase 8a のアセットや
    /// trash が書き込まれることはない。
    /// 例:
    /// `LCIR_SMOKE_DB=/path/copy.db LCIR_SMOKE_APPDIR="$HOME/Library/Application Support/com.lumencite.app" \
    ///  LCIR_SMOKE_ATT=8 cargo test --lib lcir_build_real_pdf -- --ignored --nocapture`
    /// （`cargo test --ignored ...` は `unexpected argument` で落ちる。`--ignored` は `--` の後ろ）
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

        // 一時 appdir: 対象添付だけを実 appdir からコピーして再現する（Phase 8a のアセット
        // 書き込み・GC・trash を実 appdir から隔離する）。
        let (file_path,): (String,) =
            sqlx::query_as("SELECT file_path FROM attachments WHERE id = ?")
                .bind(att)
                .fetch_one(&pool)
                .await
                .unwrap();
        let build_root = std::env::temp_dir().join(format!(
            "lumencite-lcir-smoke-{att}-{}",
            std::process::id()
        ));
        let dest = build_root.join(&file_path);
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::copy(Path::new(&appdir).join(&file_path), &dest).unwrap();
        eprintln!("build appdir = {}", build_root.display());

        let res = build_lcir_for_attachment(&pool, &build_root, att)
            .await
            .unwrap();
        eprintln!("build result: {res:?}");
        assert!(res.enabled);
        assert!(res.built || res.reused);
        assert!(res.page_count > 0, "should extract at least one page");

        let doc = load_lcir_document(&pool, att).await.unwrap().unwrap();
        let pages = doc.nodes.iter().filter(|n| n.kind == "page").count();
        let lines = doc.nodes.iter().filter(|n| n.kind == "line").count();
        let count = |k: &str| doc.nodes.iter().filter(|n| n.kind == k).count();
        eprintln!(
            "content_key={} pages={pages} lines={lines}\n  \
             section={} subsection={} heading={} paragraph={} abstract={} \
             figure_caption={} table_caption={} display_math={} bibliography_entry={} unknown_block={}",
            doc.content_key,
            count("section"),
            count("subsection"),
            count("heading"),
            count("paragraph"),
            count("abstract"),
            count("figure_caption"),
            count("table_caption"),
            count("display_math"),
            count("bibliography_entry"),
            count("unknown_block"),
        );
        // Phase 5: 定理系ノードの内訳と数点のサンプル（番号・付記名 payload）。
        eprintln!(
            "  theorem={} lemma={} proposition={} corollary={} definition={} remark={} example={} proof={}",
            count("theorem"),
            count("lemma"),
            count("proposition"),
            count("corollary"),
            count("definition"),
            count("remark"),
            count("example"),
            count("proof"),
        );
        for n in doc
            .nodes
            .iter()
            .filter(|n| {
                matches!(
                    n.kind.as_str(),
                    "theorem" | "lemma" | "proposition" | "corollary" | "definition" | "remark"
                        | "example" | "proof"
                )
            })
            .take(8)
        {
            let bbox = n
                .source_fragments
                .first()
                .map(|f| format!("p{} ({:.0},{:.0})", f.page, f.bbox.x, f.bbox.y));
            eprintln!(
                "  [{}] conf={:?} payload={:?} {:?} {}",
                n.kind,
                n.confidence,
                n.payload,
                bbox,
                n.plain_text.as_deref().unwrap_or("").chars().take(70).collect::<String>(),
            );
        }
        // 検出した数式（表層）を数点表示: 制御文字が除かれ normalized_text が埋まること。
        for n in doc.nodes.iter().filter(|n| n.kind == "display_math").take(5) {
            let m = n.math.as_ref();
            eprintln!(
                "  [display_math] label={:?} status={:?} conf={:?} {:?}",
                m.and_then(|m| m.equation_label.clone()),
                m.map(|m| m.semantic_status.clone()),
                n.confidence,
                n.plain_text.as_deref().unwrap_or("").chars().take(60).collect::<String>(),
            );
        }
        // このビルド（= 読み込んだ最新版）にスコープする。実 DB には他添付・TeX 版・superseded 版の
        // math 行も溜まっているため、グローバル COUNT(*) は単一版の display_math 数と一致しない。
        let math_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM math_expressions me
             JOIN document_nodes dn ON dn.id = me.node_id
             WHERE dn.document_version_id = ?",
        )
        .bind(doc.version_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        eprintln!("math_expressions rows (this version) = {math_rows}");
        assert_eq!(
            math_rows as usize,
            count("display_math"),
            "display_math ノード数と math_expressions 行数が一致する"
        );
        // 見出しの節番号（payload）とブロック領域（bbox）を数点表示。
        for n in doc
            .nodes
            .iter()
            .filter(|n| matches!(n.kind.as_str(), "section" | "subsection" | "heading"))
            .take(6)
        {
            let bbox = n.source_fragments.first().map(|f| {
                format!("p{} ({:.0},{:.0})", f.page, f.bbox.x, f.bbox.y)
            });
            eprintln!(
                "  [{}] {:?} conf={:?} payload={:?} {:?}",
                n.kind,
                n.plain_text.as_deref().unwrap_or("").chars().take(50).collect::<String>(),
                n.confidence,
                n.payload,
                bbox,
            );
        }
        assert!(pages > 0);
        assert!(lines > 0, "Phase 2 は line ノードを作る");

        // node-FTS が張られ、ブロック粒度で検索でき、ヒットに bbox が付く。
        let node_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM document_nodes_fts WHERE attachment_id = ?")
                .bind(att)
                .fetch_one(&pool)
                .await
                .unwrap();
        eprintln!("document_nodes_fts rows = {node_count}");
        assert!(node_count > 0, "build 後は node-FTS が張られる");

        // Phase 6a: 参照グラフ。type 別カウントと数点のサンプルを表示（PDF は番号一致・layout_model）。
        let mut rel_by_type: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for r in &doc.relations {
            *rel_by_type.entry(r.relation_type.clone()).or_insert(0) += 1;
        }
        eprintln!(
            "node_relations (this version) = {} | {rel_by_type:?}",
            doc.relations.len()
        );
        for r in doc.relations.iter().take(8) {
            eprintln!(
                "  [{}] {}→{} conf={:?} origin={:?} meta={:?}",
                r.relation_type, r.from_node_id, r.to_node_id, r.confidence, r.origin, r.metadata,
            );
        }
        // Phase 8d-7: 図表参照の解決先の内訳（実体 figure/table か caption fallback か）。
        // 期待は caption 優勢（figure ノードが番号を持つのは caption とペアリングできたときだけ）。
        // `from` が figure_caption で `to` がその図、という組が出ていたら自己言及の抑止漏れ。
        let kind_by_id: std::collections::HashMap<i64, &str> = doc
            .nodes
            .iter()
            .map(|n| (n.id, n.kind.as_str()))
            .collect();
        let mut via_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        for r in doc.relations.iter().filter(|r| {
            matches!(
                r.relation_type.as_str(),
                "refers_to_figure" | "refers_to_table"
            )
        }) {
            let via = r
                .metadata
                .as_ref()
                .and_then(|m| m.get("resolved_via"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let from_kind = kind_by_id.get(&r.from_node_id).copied().unwrap_or("?");
            let to_kind = kind_by_id.get(&r.to_node_id).copied().unwrap_or("?");
            *via_counts
                .entry(format!("{from_kind}→{via}/{to_kind}"))
                .or_insert(0) += 1;
        }
        eprintln!("  [8d-7] float refs by from_kind→resolved_via/to_kind = {via_counts:?}");
        let rel_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM node_relations WHERE document_version_id = ?")
                .bind(doc.version_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            rel_rows as usize,
            doc.relations.len(),
            "node_relations 行数と派生ビューが一致"
        );

        // Phase 8a: 図領域・アセット・caption_of。ベクター図（tikz）主体の論文では 0 件が
        // 正当なのでハードアサートしない（カウントと実ファイル整合のみ検証）。
        let fig_nodes: Vec<_> = doc.nodes.iter().filter(|n| n.kind == "figure").collect();
        let caption_of = doc
            .relations
            .iter()
            .filter(|r| r.relation_type == "caption_of")
            .count();
        let asset_rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE document_version_id = ?")
                .bind(doc.version_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        eprintln!(
            "figures={} caption_of={} asset rows={}",
            fig_nodes.len(),
            caption_of,
            asset_rows
        );
        let mut first_asset_abs: Option<std::path::PathBuf> = None;
        for n in fig_nodes.iter().take(8) {
            let bbox = n
                .source_fragments
                .first()
                .map(|f| format!("p{} ({:.0},{:.0} {:.0}x{:.0})", f.page, f.bbox.x, f.bbox.y, f.bbox.width, f.bbox.height));
            eprintln!("  [figure] payload={:?} {:?}", n.payload, bbox);
            for a in &n.assets {
                let abs = build_root.join(&a.relative_path);
                let on_disk = abs.is_file();
                eprintln!(
                    "    asset role={} {}x{:?} {}B {} exists={}",
                    a.role,
                    a.width.unwrap_or(0),
                    a.height,
                    a.size_bytes.unwrap_or(0),
                    a.relative_path,
                    on_disk,
                );
                assert!(on_disk, "アセット行が指すファイルが実在する");
                assert_eq!(
                    document_ir::sha256_file(&abs).unwrap(),
                    a.sha256,
                    "ファイル内容と sha256 が一致する"
                );
                first_asset_abs.get_or_insert(abs);
            }
        }

        // Phase 8c: 代替テキスト。**Vision 呼び出し（課金）は smoke では行わない** — 代わりに
        // バッチ対象クエリが実データで何を拾うか（crop 付きの図がすべて対象・生成済みは除外）を
        // 検証する。carry / prune は DB テストで網羅済み。
        let all_crops = crate::db::node_alt_texts::AltTextTargetFilter {
            min_crop_px: 0,
            ..Default::default()
        };
        let targets = crate::db::node_alt_texts::figures_missing_alt_text(&pool, all_crops)
            .await
            .unwrap();
        // 既定しきい値（短辺 200px）で実際に課金対象になる件数も見せる（小片が落ちる）。
        let default_targets = crate::db::node_alt_texts::figures_missing_alt_text(
            &pool,
            crate::db::node_alt_texts::AltTextTargetFilter::default(),
        )
        .await
        .unwrap();
        eprintln!(
            "[phase8c] default filter (min 200px) targets: this version={}",
            default_targets
                .iter()
                .filter(|t| t.document_version_id == doc.version_id)
                .count()
        );
        let mine: Vec<_> = targets
            .iter()
            .filter(|t| t.document_version_id == doc.version_id)
            .collect();
        eprintln!(
            "[phase8c] alt text targets: this version={} / library-wide={}",
            mine.len(),
            targets.len()
        );
        assert_eq!(
            mine.len(),
            asset_rows as usize,
            "crop を持つ図はすべて生成バッチの対象になる（まだ alt text が無い）"
        );
        for t in mine.iter().take(3) {
            eprintln!(
                "  [target] node={} sha={}… {}",
                t.node_id,
                t.asset_sha256.chars().take(8).collect::<String>(),
                t.relative_path
            );
        }
        // 生成済みを 1 件だけ模擬 → 同じ図は二度と対象にならない（再実行で再課金しない）。
        if let Some(t) = mine.first() {
            crate::db::node_alt_texts::insert_alt_text(
                &pool,
                &crate::db::node_alt_texts::NewAltText {
                    node_id: t.node_id,
                    document_version_id: t.document_version_id,
                    source_asset_sha256: &t.asset_sha256,
                    text: "(smoke) simulated description",
                    origin: document_ir::Origin::LlmInference.as_str(),
                    confidence: Some(0.5),
                    model: Some("smoke"),
                    carried_from_version_id: None,
                },
            )
            .await
            .unwrap();
            let after = crate::db::node_alt_texts::figures_missing_alt_text(&pool, all_crops)
                .await
                .unwrap();
            assert_eq!(
                after
                    .iter()
                    .filter(|x| x.document_version_id == doc.version_id)
                    .count(),
                mine.len() - 1,
                "生成済みの図はバッチ対象から外れる"
            );
            // read 面: 最新版の figure ノードに alt_text が載る（origin/model つき）。
            let reread = load_lcir_document(&pool, att).await.unwrap().unwrap();
            let with_alt = reread
                .nodes
                .iter()
                .find(|n| n.alt_text.is_some())
                .expect("alt_text を持つノードがある");
            let alt = with_alt.alt_text.as_ref().unwrap();
            eprintln!(
                "  [alt_text] node={} origin={} model={:?} text={:?}",
                with_alt.id, alt.origin, alt.model, alt.text
            );
            assert_eq!(with_alt.kind, "figure");
            assert_eq!(alt.origin, "llm_inference");
        }

        // 冪等性: 同一 PDF を再 build → 再抽出せず reuse（同一 content_key）。
        let again = build_lcir_for_attachment(&pool, &build_root, att)
            .await
            .unwrap();
        eprintln!(
            "second build: built={} reused={}",
            again.built, again.reused
        );
        assert!(again.reused, "same PDF should reuse via content_key");
        assert_eq!(again.content_key, res.content_key);

        // Phase 8a: reuse 経路の self-heal — アセットファイルを消して再 build すると復活する。
        if let Some(abs) = first_asset_abs {
            std::fs::remove_file(&abs).unwrap();
            let healed = build_lcir_for_attachment(&pool, &build_root, att)
                .await
                .unwrap();
            assert!(healed.reused);
            assert!(
                abs.is_file(),
                "self-heal が欠損アセットを再レンダリングする: {}",
                abs.display()
            );
            eprintln!("self-heal ok: {}", abs.display());
        }

        // 一時 appdir を後片付け（best-effort）。LCIR_SMOKE_KEEP=1 なら crop PNG の目視確認用に残す。
        if std::env::var("LCIR_SMOKE_KEEP").as_deref() != Ok("1") {
            let _ = std::fs::remove_dir_all(&build_root);
        }
    }

    // ---- エントリ→版解決（Phase 9a で共有化。MCP / エクスポート / CLI の単一ソース） ----

    /// completed 版を直接 INSERT する（build を通さない軽量セットアップ）。
    async fn insert_completed_version(
        pool: &SqlitePool,
        attachment_id: i64,
        extractor_name: &str,
        content_key: &str,
    ) -> i64 {
        sqlx::query(
            "INSERT INTO document_versions
                (attachment_id, content_key, schema_version, source_sha256, source_mime_type,
                 extractor_name, extractor_version, config_hash, extraction_status)
             VALUES (?, ?, '0.1.0', 'sha', 'application/octet-stream', ?, '0.0.1', '', 'completed')",
        )
        .bind(attachment_id)
        .bind(content_key)
        .bind(extractor_name)
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid()
    }

    async fn insert_root_node(pool: &SqlitePool, version_id: i64) {
        sqlx::query(
            "INSERT INTO document_nodes (document_version_id, parent_id, node_kind, ordinal)
             VALUES (?, NULL, 'document', 0)",
        )
        .bind(version_id)
        .execute(pool)
        .await
        .unwrap();
    }

    /// 2 添付（PDF 版 + TeX 版）を持つエントリを作り、(entry_id, pdf_att, tex_att) を返す。
    async fn setup_two_source_entry(pool: &SqlitePool) -> (i64, i64, i64) {
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "Two Sources".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let pdf_att = add_attachment(pool, entry.id, "attachments/x/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap()
            .id;
        let tex_att = add_attachment(pool, entry.id, "attachments/x/s.gz", "s.gz", TEX_SOURCE_MIME)
            .await
            .unwrap()
            .id;
        let pdf_ver =
            insert_completed_version(pool, pdf_att, document_ir::schema::EXTRACTOR_NAME, "ck-pdf")
                .await;
        let tex_ver = insert_completed_version(
            pool,
            tex_att,
            document_ir::schema::TEX_EXTRACTOR_NAME,
            "ck-tex",
        )
        .await;
        insert_root_node(pool, pdf_ver).await;
        insert_root_node(pool, tex_ver).await;
        (entry.id, pdf_att, tex_att)
    }

    /// Phase 8a: load_lcir_document が figure ノードへ assets（node_assets 経由）を紐づけ、
    /// caption_of 辺も relations に載ること（build を通さない DB 直挿入・pdfium 不要）。
    #[sqlx::test(migrations = "./migrations")]
    async fn load_lcir_document_attaches_assets_to_nodes(pool: SqlitePool) {
        let (_entry_id, pdf_att, _tex_att) = setup_two_source_entry(&pool).await;
        let ver = document_versions::latest_completed_for_attachment(&pool, pdf_att)
            .await
            .unwrap()
            .unwrap()
            .id;
        let figure: i64 = sqlx::query_scalar(
            "INSERT INTO document_nodes
                (document_version_id, parent_id, node_kind, ordinal, confidence, origin, payload_json)
             VALUES (?, NULL, 'figure', 1, 0.6, 'layout_model', '{\"figure_index\":1,\"figure_number\":\"2\"}')
             RETURNING id",
        )
        .bind(ver)
        .fetch_one(&pool)
        .await
        .unwrap();
        let asset_id = crate::db::assets::insert_asset(
            &pool,
            &crate::db::assets::NewAsset {
                document_version_id: ver,
                sha256: "abc",
                mime_type: "image/png",
                relative_path: "attachments/x/.lcir/1/deadbeef/fig-p001-00.png",
                width: Some(800),
                height: Some(600),
                size_bytes: Some(4321),
                metadata_json: Some(r#"{"page":1,"region_index":0}"#),
            },
        )
        .await
        .unwrap();
        crate::db::assets::insert_node_asset(
            &pool,
            &crate::db::assets::NewNodeAsset {
                node_id: figure,
                asset_id,
            },
            "page_crop",
        )
        .await
        .unwrap();

        let doc = load_lcir_document(&pool, pdf_att).await.unwrap().unwrap();
        let fig_node = doc.nodes.iter().find(|n| n.kind == "figure").unwrap();
        assert_eq!(fig_node.assets.len(), 1);
        let a = &fig_node.assets[0];
        assert_eq!(a.role, "page_crop");
        assert_eq!(a.mime_type, "image/png");
        assert_eq!(
            a.relative_path,
            "attachments/x/.lcir/1/deadbeef/fig-p001-00.png"
        );
        assert_eq!(a.width, Some(800));
        assert_eq!(a.size_bytes, Some(4321));
        assert_eq!(a.sha256, "abc");
        assert_eq!(a.metadata.as_ref().unwrap()["page"], 1);
        // figure 以外のノード（root）には assets が付かない。
        let root = doc.nodes.iter().find(|n| n.kind == "document").unwrap();
        assert!(root.assets.is_empty());
    }

    // ---- Phase 8c: 版跨ぎの alt text carry / prune（pdfium 不要・抽出結果を手組みして tx を回す） ----

    /// crop PNG 1 枚を持つ 1 ページの抽出結果を作る。`sha` を変えると「別の絵」になる。
    fn extracted_with_crop(sha: &str) -> pdf::ExtractedDocument {
        pdf::ExtractedDocument {
            pages: vec![pdf::ExtractedPage {
                page_number: 1,
                width_pt: 595.0,
                height_pt: 842.0,
                box_left: 0.0,
                box_bottom: 0.0,
                rotation_deg: 0.0,
                plain_text: "page text".to_string(),
                blocks: Vec::new(),
                image_regions: vec![pdf::ExtractedImageRegion {
                    bbox: document_ir::BBox::new(100.0, 400.0, 300.0, 200.0),
                    source: figures::RegionSource::Raster,
                    file: Some(pdf::ExtractedAssetFile {
                        file_name: "fig-p001-00.png".to_string(),
                        width_px: 800,
                        height_px: 534,
                        sha256: sha.to_string(),
                        size_bytes: 4321,
                    }),
                }],
            }],
            warnings: Vec::new(),
        }
    }

    /// debt-22: **保存の時点で** page の `plain_text` から C0 制御文字が落ちていること。
    ///
    /// 索引側（p1）だけで正規化すると、9a の JSON export と `get_node_context` の
    /// page-focus に生値が残る。ここは「読み手を名指しできない正規化義務を分散させない」ための
    /// 保存点の配線テストで、純関数側は `structure::tests::clean_page_text_*` が固定する。
    #[sqlx::test(migrations = "./migrations")]
    async fn page_text_is_stored_without_c0_control_characters(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let mut doc = extracted_with_crop("sha-c0");
        doc.pages[0].plain_text = "Quantum\u{2} walks\r\nare consis\u{2}tent\r\n".to_string();
        let vid = insert_version_from(&pool, att, "ck-c0", None, &doc).await;

        let text: Option<String> = sqlx::query_scalar(
            "SELECT plain_text FROM document_nodes
             WHERE document_version_id = ? AND node_kind = 'page'",
        )
        .bind(vid)
        .fetch_one(&pool)
        .await
        .unwrap();
        let text = text.expect("page に本文がある");
        assert_eq!(
            text, "Quantum walks\nare consistent\n",
            "C0 は落ちるが改行は残る"
        );
        assert!(!text.chars().any(|c| (c as u32) < 0x20 && c != '\n' && c != '\t'));
    }

    /// 中身が制御文字だけのページは `plain_text = NULL` になる（掃除の結果、空になる）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_page_made_only_of_control_characters_is_stored_as_null(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let mut doc = extracted_with_crop("sha-empty");
        doc.pages[0].plain_text = "\u{2}\u{15}\u{c}".to_string();
        let vid = insert_version_from(&pool, att, "ck-empty", None, &doc).await;

        let text: Option<String> = sqlx::query_scalar(
            "SELECT plain_text FROM document_nodes
             WHERE document_version_id = ? AND node_kind = 'page'",
        )
        .bind(vid)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(text, None);
    }

    /// 抽出結果を 1 版として挿入する（`build_pdf_version` のうち pdfium と FS を除いた部分）。
    async fn insert_version_from(
        pool: &SqlitePool,
        attachment_id: i64,
        ckey: &str,
        parent: Option<i64>,
        doc: &pdf::ExtractedDocument,
    ) -> i64 {
        let ctx = PdfBuildCtx {
            attachment_id,
            abs_path: Path::new("/nonexistent/p.pdf"),
            mime_type: "application/pdf",
            source_sha256: "sha",
            ckey,
            parent_version_id: parent,
            app_data_dir: Path::new("/nonexistent"),
            asset_rel_dir: &format!("attachments/x/.lcir/{attachment_id}/{}", &ckey[..4]),
        };
        insert_pdf_version_tx(pool, &ctx, doc).await.unwrap().0
    }

    /// Phase 8d-2: ラスタ図 1 個 + ベクター図 1 個 + caption 2 つの 1 ページ。
    /// caption は**下側がラスタ図に、上側がベクター図に**結ばれる配置にしてある。
    fn extracted_with_raster_and_vector() -> pdf::ExtractedDocument {
        let block = |text: &str, y: f64| pdf::ExtractedBlock {
            text: text.to_string(),
            bbox: document_ir::BBox::new(100.0, y, 300.0, 10.0),
            reading_order: (800.0 - y) as i64,
        };
        pdf::ExtractedDocument {
            pages: vec![pdf::ExtractedPage {
                page_number: 1,
                width_pt: 595.0,
                height_pt: 842.0,
                box_left: 0.0,
                box_bottom: 0.0,
                rotation_deg: 0.0,
                plain_text: "page".to_string(),
                // caption 2 つの間に本文 3 行を挟む。**行間の中央値で段落を割る**ので、
                // 2 行だけだと中央値 = そのギャップになって 1 ブロックに畳まれてしまう。
                blocks: vec![
                    block("Figure 1: raster.", 560.0),
                    block("Body text of the paper here.", 500.0),
                    block("Second line of the same paragraph.", 488.0),
                    block("Third line of the same paragraph.", 476.0),
                    block("Figure 2: vector.", 260.0),
                ],
                image_regions: vec![
                    pdf::ExtractedImageRegion {
                        bbox: document_ir::BBox::new(100.0, 600.0, 300.0, 120.0),
                        source: figures::RegionSource::Raster,
                        file: Some(pdf::ExtractedAssetFile {
                            file_name: "fig-p001-00.png".to_string(),
                            width_px: 800,
                            height_px: 320,
                            sha256: "rastersha".to_string(),
                            size_bytes: 100,
                        }),
                    },
                    pdf::ExtractedImageRegion {
                        bbox: document_ir::BBox::new(100.0, 300.0, 300.0, 120.0),
                        source: figures::RegionSource::Vector,
                        file: Some(pdf::ExtractedAssetFile {
                            file_name: "vec-p001-00.png".to_string(),
                            width_px: 800,
                            height_px: 320,
                            sha256: "vectorsha".to_string(),
                            size_bytes: 200,
                        }),
                    },
                ],
            }],
            warnings: Vec::new(),
        }
    }

    /// 同じ図番号の caption が 2 つあり、片方だけがラスタ図と結ばれている版
    /// （実 DB に 18 版・27 組）。ベクター図に番号を渡すとこの形で既存の辺が消える。
    fn extracted_with_two_figure_ones() -> pdf::ExtractedDocument {
        let mut doc = extracted_with_raster_and_vector();
        let page = &mut doc.pages[0];
        page.blocks[0].text = "Figure 1: raster.".to_string();
        page.blocks[1].text = "As shown in Figure 1, the setup works.".to_string();
        page.blocks[4].text = "Figure 1: vector.".to_string();
        doc
    }

    /// **ベクター図は参照グラフの番号索引に入れない**（ゲート ②a の変異 S6）。
    ///
    /// `graph::FloatTargets` は同じ番号が 2 つのノードに付くとその番号ごと墓標（`None`）に
    /// するので、ベクター図にも番号を渡すと「同番号の図 caption が 2 つあり片方だけが
    /// ラスタ図と結ばれている」版で**既存の `refers_to_figure` 辺が消える**。
    /// 変異（ベクターにも番号を渡す）は 1,051 本すべて緑のまま通っていた ── 既存テストは
    /// payload しか見ておらず、参照グラフを 1 行も assert していなかった。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_vector_figure_does_not_tombstone_a_number_a_raster_figure_owns(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = insert_version_from(&pool, att, "ck-dup", None, &extracted_with_two_figure_ones())
            .await;

        let figures: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, COALESCE(payload_json,'') FROM document_nodes
             WHERE document_version_id = ? AND node_kind = 'figure' ORDER BY ordinal",
        )
        .bind(vid)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(figures.len(), 2, "ラスタ 1 + ベクター 1");
        let (raster_id, raster_payload) = &figures[0];
        // 番号自体は両方の payload に載る（`get_figures` の表示用）。索引に入れないだけ。
        assert!(raster_payload.contains("\"figure_number\":\"1\""), "{raster_payload}");
        assert!(figures[1].1.contains("\"figure_number\":\"1\""), "{:?}", figures[1].1);

        let doc = load_lcir_document(&pool, att).await.unwrap().unwrap();
        let refs: Vec<_> = doc
            .relations
            .iter()
            .filter(|r| r.relation_type == "refers_to_figure")
            .collect();
        assert_eq!(
            refs.len(),
            1,
            "本文の \"Figure 1\" はラスタ図に解決する（ベクターに番号を渡すと墓標で 0 本になる）: {refs:?}"
        );
        assert_eq!(refs[0].to_node_id, *raster_id);
    }

    /// 2 段ペアリングの戻り値は `raster ++ vector` の連結順なので、**元の並びへ引き直す**
    /// 必要がある（ゲート ②a の変異 S8）。
    ///
    /// 本番の `compose_figure_regions` は必ずラスタを先に置くため引き直しは現状恒等写像で、
    /// `let at = fi` に潰しても 1,051 本が全緑だった＝この関数の正しさが別ファイルの
    /// 並び規約だけに支えられていた。ここでは**ベクターが先に並んだ入力**を与えて、
    /// 引き直し自体を留める（並び規約の方は
    /// `figures::tests::composed_regions_always_list_raster_before_vector` で別に留める）。
    #[sqlx::test(migrations = "./migrations")]
    async fn caption_pairs_survive_a_vector_first_region_order(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let mut doc = extracted_with_raster_and_vector();
        doc.pages[0].image_regions.reverse(); // [vector, raster]
        let vid = insert_version_from(&pool, att, "ck-vecfirst", None, &doc).await;

        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT COALESCE(payload_json,'') FROM document_nodes
             WHERE document_version_id = ? AND node_kind = 'figure' ORDER BY ordinal",
        )
        .bind(vid)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("\"region_source\":\"vector\""), "{:?}", rows[0]);
        assert!(
            rows[0].contains("\"figure_number\":\"2\""),
            "先頭のベクター領域は下側の caption と結ぶ: {:?}",
            rows[0]
        );
        assert!(!rows[1].contains("region_source"), "{:?}", rows[1]);
        assert!(
            rows[1].contains("\"figure_number\":\"1\""),
            "後ろのラスタ領域は上側の caption と結ぶ: {:?}",
            rows[1]
        );
    }

    /// Phase 8d-2 の build 面: 2 段ペアリングの添字の引き直し・由来ごとの confidence・
    /// `region_source` payload・**ベクター図を参照グラフの番号索引に入れないこと**を通す。
    #[sqlx::test(migrations = "./migrations")]
    async fn vector_regions_get_their_own_confidence_and_stay_out_of_the_number_index(
        pool: SqlitePool,
    ) {
        let att = setup_attachment(&pool).await;
        let vid = insert_version_from(
            &pool,
            att,
            "ck-vector",
            None,
            &extracted_with_raster_and_vector(),
        )
        .await;

        let rows: Vec<(i64, f64, String)> = sqlx::query_as(
            "SELECT id, confidence, COALESCE(payload_json,'') FROM document_nodes
             WHERE document_version_id = ? AND node_kind = 'figure' ORDER BY ordinal",
        )
        .bind(vid)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2, "ラスタ 1 + ベクター 1");
        let (raster_id, raster_conf, raster_payload) = &rows[0];
        let (vector_id, vector_conf, vector_payload) = &rows[1];
        assert_eq!(*raster_conf, figures::RASTER_REGION_CONFIDENCE);
        assert_eq!(*vector_conf, figures::VECTOR_REGION_CONFIDENCE);
        assert!(!raster_payload.contains("region_source"), "ラスタの payload は 8a のまま");
        assert!(vector_payload.contains("\"region_source\":\"vector\""));
        // 番号は両方 payload に載る（表示用）。
        assert!(raster_payload.contains("\"figure_number\":\"1\""), "{raster_payload}");
        assert!(vector_payload.contains("\"figure_number\":\"2\""), "{vector_payload}");

        // caption_of は 2 本。**辺の confidence は結んだ図に揃う**。
        let edges: Vec<(i64, f64)> = sqlx::query_as(
            "SELECT to_node_id, confidence FROM node_relations
             WHERE document_version_id = ? AND relation_type = 'caption_of' ORDER BY to_node_id",
        )
        .bind(vid)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(edges.len(), 2, "2 段ペアリングで両方が結ばれる: {edges:?}");
        let conf_of = |id: &i64| edges.iter().find(|e| e.0 == *id).map(|e| e.1);
        assert_eq!(conf_of(raster_id), Some(figures::RASTER_REGION_CONFIDENCE));
        assert_eq!(conf_of(vector_id), Some(figures::VECTOR_REGION_CONFIDENCE));

        // アセットは由来ごとに別のファイル名で入る。
        let paths: Vec<String> = sqlx::query_scalar(
            "SELECT relative_path FROM assets WHERE document_version_id = ? ORDER BY relative_path",
        )
        .bind(vid)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(paths[0].ends_with("fig-p001-00.png"), "{paths:?}");
        assert!(paths[1].ends_with("vec-p001-00.png"), "{paths:?}");
    }

    async fn figure_node_of(pool: &SqlitePool, version_id: i64) -> i64 {
        sqlx::query_scalar(
            "SELECT id FROM document_nodes WHERE document_version_id = ? AND node_kind = 'figure'",
        )
        .bind(version_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn add_generated_alt_text(pool: &SqlitePool, node_id: i64, version_id: i64, sha: &str) {
        crate::db::node_alt_texts::insert_alt_text(
            pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id,
                document_version_id: version_id,
                source_asset_sha256: sha,
                text: "Diagram of two coupled cavities.",
                origin: document_ir::Origin::LlmInference.as_str(),
                confidence: Some(0.5),
                model: Some("gpt-4o-mini"),
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();
    }

    /// 抽出器版を上げて再構築しても、crop が**バイト同一**なら alt text を引き継ぎ（再課金しない）、
    /// 旧版の生成行は刈られること。由来は `carried_from_version_id` に残る。
    #[sqlx::test(migrations = "./migrations")]
    async fn alt_text_is_carried_when_crop_fingerprint_matches(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let doc = extracted_with_crop("cropsha-same");
        let v1 = insert_version_from(&pool, att, "ck1-aaaa", None, &doc).await;
        let fig1 = figure_node_of(&pool, v1).await;
        add_generated_alt_text(&pool, fig1, v1, "cropsha-same").await;

        // 同じ絵のまま再構築（別 content_key = 抽出器版を上げた想定）。
        let v2 = insert_version_from(&pool, att, "ck2-bbbb", Some(v1), &doc).await;
        let fig2 = figure_node_of(&pool, v2).await;
        assert_ne!(fig1, fig2, "新版のノードは別 id");

        let carried = crate::db::node_alt_texts::alt_texts_for_version(&pool, v2)
            .await
            .unwrap();
        assert_eq!(carried.len(), 1, "新版へ 1 件引き継がれる");
        assert_eq!(carried[0].node_id, fig2);
        assert_eq!(carried[0].text, "Diagram of two coupled cavities.");
        assert_eq!(carried[0].origin, "llm_inference");
        assert_eq!(carried[0].model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(carried[0].carried_from_version_id, Some(v1), "由来版が残る");
        assert!(
            crate::db::node_alt_texts::alt_texts_for_version(&pool, v1)
                .await
                .unwrap()
                .is_empty(),
            "旧版の生成行は刈られる"
        );

        // read 面: 最新版の figure ノードに alt_text が載る。
        let lcir = load_lcir_document(&pool, att).await.unwrap().unwrap();
        let fig = lcir.nodes.iter().find(|n| n.kind == "figure").unwrap();
        let alt = fig.alt_text.as_ref().expect("alt_text が載る");
        assert_eq!(alt.origin, "llm_inference");
        assert_eq!(alt.source_asset_sha256, "cropsha-same");

        // さらに再構築しても「最初の生成版」を指し続ける（carry の連鎖で由来を失わない）。
        let v3 = insert_version_from(&pool, att, "ck3-cccc", Some(v2), &doc).await;
        let again = crate::db::node_alt_texts::alt_texts_for_version(&pool, v3)
            .await
            .unwrap();
        assert_eq!(again[0].carried_from_version_id, Some(v1));
    }

    /// 絵が変わった（crop の sha256 が違う）図には引き継がない — 別の絵に古い説明を付けない。
    #[sqlx::test(migrations = "./migrations")]
    async fn alt_text_is_not_carried_when_crop_changes(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let v1 = insert_version_from(&pool, att, "ck1-aaaa", None, &extracted_with_crop("old-sha"))
            .await;
        let fig1 = figure_node_of(&pool, v1).await;
        add_generated_alt_text(&pool, fig1, v1, "old-sha").await;

        let v2 = insert_version_from(
            &pool,
            att,
            "ck2-bbbb",
            Some(v1),
            &extracted_with_crop("new-sha"),
        )
        .await;
        assert!(
            crate::db::node_alt_texts::alt_texts_for_version(&pool, v2)
                .await
                .unwrap()
                .is_empty(),
            "別の絵には引き継がない"
        );
        let lcir = load_lcir_document(&pool, att).await.unwrap().unwrap();
        let fig = lcir.nodes.iter().find(|n| n.kind == "figure").unwrap();
        assert!(fig.alt_text.is_none());
    }

    /// 手編集（user_edited）は carry の対象外だが、刈られもしない（人の記述を勝手に消さない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn user_edited_alt_text_survives_rebuild_without_being_carried(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let doc = extracted_with_crop("cropsha-same");
        let v1 = insert_version_from(&pool, att, "ck1-aaaa", None, &doc).await;
        let fig1 = figure_node_of(&pool, v1).await;
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: fig1,
                document_version_id: v1,
                source_asset_sha256: "cropsha-same",
                text: "Hand written description.",
                origin: document_ir::Origin::UserEdited.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        let v2 = insert_version_from(&pool, att, "ck2-bbbb", Some(v1), &doc).await;
        assert!(
            crate::db::node_alt_texts::alt_texts_for_version(&pool, v2)
                .await
                .unwrap()
                .is_empty(),
            "手編集は carry しない"
        );
        let old = crate::db::node_alt_texts::alt_texts_for_version(&pool, v1)
            .await
            .unwrap();
        assert_eq!(old.len(), 1, "手編集は刈られない");
        assert_eq!(old[0].origin, "user_edited");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn entry_lcir_versions_sorts_tex_first(pool: SqlitePool) {
        let (entry_id, _pdf_att, tex_att) = setup_two_source_entry(&pool).await;
        let versions = entry_lcir_versions(&pool, entry_id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(
            versions[0].extractor_name,
            document_ir::schema::TEX_EXTRACTOR_NAME,
            "read 優先度は tex > pdfium"
        );
        assert_eq!(versions[0].attachment_id, tex_att);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_entry_lcir_prefers_tex_and_honors_wanted(pool: SqlitePool) {
        let (entry_id, pdf_att, tex_att) = setup_two_source_entry(&pool).await;

        // 未指定 → tex 版。
        let (found, versions) = load_entry_lcir(&pool, entry_id, None).await.unwrap();
        let (att, doc) = found.expect("tex 版が読める");
        assert_eq!(att, tex_att);
        assert_eq!(doc.source.extractor_name, document_ir::schema::TEX_EXTRACTOR_NAME);
        assert_eq!(versions.len(), 2);

        // pdf 指定 → pdfium 版に限定。
        let wanted = source_to_extractor("pdf").unwrap();
        let (found, _) = load_entry_lcir(&pool, entry_id, Some(wanted)).await.unwrap();
        let (att, doc) = found.expect("pdf 版が読める");
        assert_eq!(att, pdf_att);
        assert_eq!(doc.source.extractor_name, document_ir::schema::EXTRACTOR_NAME);

        // 未知 source はエラー文言を返す。
        assert!(source_to_extractor("html").is_err());
        assert_eq!(short_source_name(document_ir::schema::TEX_EXTRACTOR_NAME), "tex");
        assert_eq!(short_source_name(document_ir::schema::EXTRACTOR_NAME), "pdf");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_entry_lcir_returns_versions_even_when_wanted_missing(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Tex Only".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let tex_att = add_attachment(&pool, entry.id, "attachments/y/s.gz", "s.gz", TEX_SOURCE_MIME)
            .await
            .unwrap()
            .id;
        let ver = insert_completed_version(
            &pool,
            tex_att,
            document_ir::schema::TEX_EXTRACTOR_NAME,
            "ck-tex-only",
        )
        .await;
        insert_root_node(&pool, ver).await;

        let wanted = source_to_extractor("pdf").unwrap();
        let (found, versions) = load_entry_lcir(&pool, entry.id, Some(wanted)).await.unwrap();
        assert!(found.is_none(), "pdf 版は無い");
        assert_eq!(versions.len(), 1, "案内文用に併存一覧は返る");
        assert_eq!(short_source_name(&versions[0].extractor_name), "tex");
    }

    // ---- 旧 content_key ディレクトリの GC（debt-15） ----

    /// `<root>/attachments/1/.lcir/7/<key>` を作り、中に crop PNG を 1 枚置く。
    fn make_asset_dir(root: &Path, key: &str) -> PathBuf {
        let dir = root
            .join("attachments")
            .join("1")
            .join(".lcir")
            .join("7")
            .join(key);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fig-p001-00.png"), b"png").unwrap();
        dir
    }

    /// ファイル 1 枚の mtime を `secs` 秒前に戻す。
    fn age_file(path: &Path, secs: u64) {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    /// ディレクトリ内の全ファイルの mtime を `secs` 秒前に戻す。
    fn age_files(dir: &Path, secs: u64) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            age_file(&e.path(), secs);
        }
    }

    fn gc_tmp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lcir-gc-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    /// 猶予を過ぎた旧 content_key ディレクトリは trash へ回収する（従来どおり）。
    #[test]
    fn gc_collects_stale_asset_dir() {
        let root = gc_tmp_root("stale");
        let current = make_asset_dir(&root, "aaaaaaaaaaaaaaaa");
        let old = make_asset_dir(&root, "bbbbbbbbbbbbbbbb");
        std::fs::write(old.join("fig-p002-00.png"), b"png").unwrap();
        age_files(&old, 2 * 60 * 60);

        gc_stale_asset_dirs(&root, &current);

        assert!(!old.exists(), "猶予を過ぎた旧ディレクトリは回収される");
        assert!(current.is_dir(), "現 content_key は残る");
        // 直接消すのではなく trash（永続 retry queue）へ入る。
        let trashed: Vec<_> = std::fs::read_dir(crate::attachment_trash::trash_dir(&root))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(trashed, vec!["bbbbbbbbbbbbbbbb".to_string()]);
        std::fs::remove_dir_all(&root).ok();
    }

    /// **最古ではなく最新**のファイル mtime で判定する。1 添付の抽出は最長 75 分
    /// （att37・527 頁）かかるので、最古で判定すると長い build ほど猶予が実質ゼロになり、
    /// 別インスタンスが書き終えた直後の crop を消してしまう。
    #[test]
    fn gc_judges_by_newest_file_not_oldest() {
        let root = gc_tmp_root("newest");
        let current = make_asset_dir(&root, "aaaaaaaaaaaaaaaa");
        let other = make_asset_dir(&root, "eeeeeeeeeeeeeeee");
        // 長い build を模す: 1 枚目は 2 時間前、2 枚目は今しがた書かれた。
        age_file(&other.join("fig-p001-00.png"), 2 * 60 * 60);
        std::fs::write(other.join("fig-p002-00.png"), b"png").unwrap();

        gc_stale_asset_dirs(&root, &current);

        assert!(
            other.is_dir(),
            "最新ファイルが猶予内なら残す（最古で判定すると消えてしまう）"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 現 content_key は**名前の一致**だけで守られている（猶予では守られない）。
    /// 中身が猶予を超えていても回収してはならない。
    #[test]
    fn gc_never_collects_current_dir_even_when_aged() {
        let root = gc_tmp_root("agedcurrent");
        let current = make_asset_dir(&root, "aaaaaaaaaaaaaaaa");
        let old = make_asset_dir(&root, "bbbbbbbbbbbbbbbb");
        age_files(&current, 2 * 60 * 60);
        age_files(&old, 2 * 60 * 60);

        // current の中身は猶予を超えている = 猶予では守られない状態。
        assert!(is_stale_asset_dir(&current, std::time::SystemTime::now()));

        gc_stale_asset_dirs(&root, &current);

        assert!(current.is_dir(), "名前の一致ガードだけが current を守っている");
        assert!(!old.exists(), "旧は回収される");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 書き始める直前（ファイルが 1 枚も無い）のディレクトリは、ディレクトリ自身の
    /// mtime にフォールバックして守る。別インスタンスが今まさに作った直後を消さない。
    #[test]
    fn gc_keeps_freshly_created_empty_dir() {
        let root = gc_tmp_root("empty");
        let current = make_asset_dir(&root, "aaaaaaaaaaaaaaaa");
        let empty = root
            .join("attachments")
            .join("1")
            .join(".lcir")
            .join("7")
            .join("ffffffffffffffff");
        std::fs::create_dir_all(&empty).unwrap();

        gc_stale_asset_dirs(&root, &current);

        assert!(empty.is_dir(), "作られたばかりの空ディレクトリは残す");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 最近書かれた旧 content_key ディレクトリは残す。dev ビルドと配布版は同じ
    /// app data dir を共有するので、猶予が無いと互いの crop を消し合う（debt-15）。
    #[test]
    fn gc_keeps_recently_written_asset_dir() {
        let root = gc_tmp_root("fresh");
        let current = make_asset_dir(&root, "aaaaaaaaaaaaaaaa");
        let other = make_asset_dir(&root, "cccccccccccccccc");

        gc_stale_asset_dirs(&root, &current);

        assert!(other.is_dir(), "別インスタンスが書いたばかりのディレクトリは残す");
        assert!(
            other.join("fig-p001-00.png").is_file(),
            "中の crop PNG も残る"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// `heal_missing_assets` が再レンダリングした直後のディレクトリは残す。
    /// 全ファイルが猶予超過でも、1 枚書き直されていれば「使われている」証拠になる。
    #[test]
    fn gc_keeps_dir_whose_files_were_rewritten_in_place() {
        let root = gc_tmp_root("rewritten");
        let current = make_asset_dir(&root, "aaaaaaaaaaaaaaaa");
        let other = make_asset_dir(&root, "dddddddddddddddd");
        std::fs::write(other.join("fig-p002-00.png"), b"png").unwrap();
        age_files(&other, 2 * 60 * 60);
        // heal が 1 枚だけ書き直した状態。
        std::fs::write(other.join("fig-p001-00.png"), b"png2").unwrap();

        gc_stale_asset_dirs(&root, &current);

        assert!(other.is_dir(), "書き直された直後のディレクトリは残す");
        std::fs::remove_dir_all(&root).ok();
    }
}
