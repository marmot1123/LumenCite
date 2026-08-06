//! OCR ツール: スキャン PDF をページ画像化（pdfium）→ LLM Vision で文字起こし →
//! `fulltext` に保存して全文検索可能にする。ツール経由（LLM）と手動コマンドの両方から使う。

use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use serde_json::json;

use super::{ToolContext, ToolError};
use crate::keychain;
use crate::llm::{ocr, ToolCallSpec, ToolSpec};

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "ocr_pdf".to_string(),
        description: "OCR a scanned PDF attachment that has NO text layer: rasterize its pages, \
            transcribe them with the vision model, and index the text for full-text search. \
            This costs money and REPLACES the attachment's existing index, so it is a last resort: \
            first call get_fulltext, and only OCR when it answers `indexed: false`. An entry whose \
            text is already indexed will refuse a full OCR. Note that fulltext_search finding \
            nothing only means those words are absent — it does not mean the PDF is unindexed."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "The entry whose PDF to OCR." },
                "attachment_id": {
                    "type": "integer",
                    "description": "Optional specific PDF attachment to OCR. Omit to use the entry's first PDF."
                },
                "pages": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "1-based page numbers to OCR. Omit to OCR all pages."
                }
            },
            "required": ["entry_id"]
        }),
        needs_approval: true,
    }]
}

pub async fn try_execute(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Option<Result<String, ToolError>> {
    if call.tool_name != "ocr_pdf" {
        return None;
    }
    let entry_id = match call.arguments.get("entry_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => {
            return Some(Err(ToolError::InvalidArguments(
                "entry_id is required".into(),
            )))
        }
    };
    let pages = call
        .arguments
        .get("pages")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect::<Vec<i64>>());
    // チャットのスコープ外 entry は OCR させない（CR-024）。
    if let Err(e) = ctx.ensure_entry_in_scope(entry_id) {
        return Some(Err(e));
    }
    let attachment_id = call.arguments.get("attachment_id").and_then(|v| v.as_i64());

    // 索引済みの PDF を丸ごと OCR し直させない（issue #42）。全ページ OCR は
    // `index_attachment` で添付の索引を**置き換える**ので、pdfium が抜いたテキスト層が
    // Vision の出力で上書きされる。課金もかかる。既に読めるなら `get_fulltext` へ誘導する。
    // **ユーザーが UI から明示的に押す経路（`run_ocr` 直呼び）はこの制限を受けない** —
    // テキスト層が壊れている PDF を OCR し直すのは正当な操作なので。
    if pages.is_none() {
        let indexed = crate::db::fulltext::entry_fulltext_page_count(ctx.pool, entry_id)
            .await
            .unwrap_or(0);
        if indexed > 0 {
            return Some(Ok(format!(
                "entry {entry_id} already has {indexed} indexed page(s) of full text — call \
                 get_fulltext to read it. OCR is only for scanned PDFs with no text layer, and \
                 re-running it would replace the existing text. To re-transcribe specific pages \
                 anyway, pass `pages`."
            )));
        }
    }

    Some(run_ocr(ctx.pool, ctx.app_data_dir, entry_id, attachment_id, pages).await)
}

/// entry の PDF 添付を OCR して `fulltext` に保存する。
pub async fn run_ocr(
    pool: &sqlx::SqlitePool,
    app_data_dir: &Path,
    entry_id: i64,
    attachment_id: Option<i64>,
    pages: Option<Vec<i64>>,
) -> Result<String, ToolError> {
    // 1. 対象 PDF 添付。attachment_id 指定があればその添付を、無ければ最初の PDF を使う（CR-027）。
    //    複数 PDF のとき「常に先頭」を OCR してしまわないよう、UI からは選択中の添付 id を渡す。
    let row: Option<(i64, String)> = match attachment_id {
        Some(att_id) => {
            sqlx::query_as(
                "SELECT id, file_path FROM attachments
                 WHERE id = ? AND entry_id = ? AND mime_type = 'application/pdf'",
            )
            .bind(att_id)
            .bind(entry_id)
            .fetch_optional(pool)
            .await?
        }
        None => {
            sqlx::query_as(
                "SELECT id, file_path FROM attachments
                 WHERE entry_id = ? AND mime_type = 'application/pdf' ORDER BY id LIMIT 1",
            )
            .bind(entry_id)
            .fetch_optional(pool)
            .await?
        }
    };
    let (attachment_id, file_path) = row
        .ok_or_else(|| ToolError::Execution(format!("entry {entry_id} has no matching PDF attachment")))?;
    let abs_path = app_data_dir.join(&file_path);

    // 2. OCR プロバイダ/モデル（未設定なら chat 用にフォールバック）+ API キー
    let (provider, model) = resolve_ocr_provider(pool).await?;
    let account = keychain::account_for_api_key(&provider);
    let api_key = keychain::get(&account)
        .map_err(|e| ToolError::Execution(e.to_string()))?
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| ToolError::Execution(format!("API key for {provider} is not configured")))?;

    // 3. ラスタライズ（pdfium・同期）
    let images = rasterize(&abs_path, pages.as_deref())?;
    if images.is_empty() {
        return Err(ToolError::Execution("no pages to OCR".into()));
    }

    // 4. 全ページの Vision 結果を集めてから保存する。途中の API エラーで
    //    既存インデックスが失われないよう、削除はここでは行わない。
    let page_count = images.len();
    let mut total_chars = 0usize;
    let mut results: Vec<(i64, String)> = Vec::with_capacity(page_count);
    for (page_no, b64) in images {
        let text = ocr::ocr_image(&provider, &model, &api_key, "image/png", &b64)
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))?;
        total_chars += text.chars().count();
        results.push((page_no, text));
    }

    // 5. トランザクションで置き換え。全ページ OCR なら丸ごと、部分 OCR なら
    //    該当ページのみ差し替え（従来は部分 OCR でも全ページ消していた）。
    save_ocr_pages(pool, attachment_id, &results, pages.is_some()).await?;

    Ok(format!(
        "OCR'd {page_count} page(s); {total_chars} characters indexed for entry {entry_id}."
    ))
}

/// OCR 結果を保存する。全ページ OCR なら添付ごと置き換え、部分 OCR なら該当ページのみ差し替え。
///
/// あわせて**この添付の索引は OCR 由来**だと記録する（p1）。記録しないと、LCIR からの派生や
/// 添付経路の pdf_extract が後から上書きしてしまう。ページ単位の部分 OCR でも添付ごと
/// 保護されるのは承知の上での保守側への倒し込み（§2.6-1）── ユーザーが OCR を回した添付は
/// 「この PDF のテキスト層は信用できない」と明示的に宣言した添付だから。
pub(crate) async fn save_ocr_pages(
    pool: &sqlx::SqlitePool,
    attachment_id: i64,
    results: &[(i64, String)],
    partial: bool,
) -> Result<(), sqlx::Error> {
    if partial {
        crate::db::fulltext::update_attachment_pages(pool, attachment_id, results).await?;
    } else {
        crate::db::fulltext::index_attachment(pool, attachment_id, results).await?;
    }
    // **中身を 1 行も残せなかった OCR では記録しない。** 空文字のページは索引に入らないので
    // （`replace_pages` / `update_attachment_pages` は空をスキップする）、記録だけ立てると
    // 「中身 0 行の索引を守り続ける」状態になり、その添付は再索引もできなくなる。
    if crate::db::fulltext::indexed_page_count(pool, attachment_id).await? == 0 {
        return crate::db::fulltext::clear_fulltext_source(pool, attachment_id).await;
    }
    crate::db::fulltext::set_fulltext_source(
        pool,
        attachment_id,
        crate::db::fulltext::FulltextSource::Ocr,
    )
    .await
}

/// OCR / Vision 用のプロバイダとモデル（未設定なら chat 用にフォールバック）。
/// Phase 8c の alt text 生成バッチも同じ設定を共有する（Vision 用の設定面を増やさない）。
pub(crate) async fn resolve_ocr_provider(
    pool: &sqlx::SqlitePool,
) -> Result<(String, String), ToolError> {
    use crate::db::settings::{
        get_setting, LLM_MODEL_KEY, LLM_OCR_MODEL_KEY, LLM_OCR_PROVIDER_KEY, LLM_PROVIDER_KEY,
    };
    let provider = match get_setting(pool, LLM_OCR_PROVIDER_KEY).await? {
        Some(p) if !p.trim().is_empty() => p,
        _ => get_setting(pool, LLM_PROVIDER_KEY)
            .await?
            .unwrap_or_else(|| "openai".to_string()),
    };
    let model = match get_setting(pool, LLM_OCR_MODEL_KEY).await? {
        Some(m) if !m.trim().is_empty() => m,
        _ => get_setting(pool, LLM_MODEL_KEY).await?.unwrap_or_else(|| {
            match provider.as_str() {
                "anthropic" => "claude-haiku-4-5-20251001".to_string(),
                _ => "gpt-4o-mini".to_string(),
            }
        }),
    };
    Ok((provider, model))
}

/// PDF をページ画像（PNG base64）に。`pages` は 1 始まり、None で全ページ。
fn rasterize(path: &Path, pages: Option<&[i64]>) -> Result<Vec<(i64, String)>, ToolError> {
    use pdfium_render::prelude::*;
    let bindings = crate::ingestion::pdf::pdfium::bind_pdfium().map_err(ToolError::Execution)?;
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| ToolError::Execution(format!("failed to open PDF: {e}")))?;
    let config = PdfRenderConfig::new().set_target_width(1600);
    let mut out = Vec::new();
    for (idx, page) in doc.pages().iter().enumerate() {
        let page_no = idx as i64 + 1;
        if let Some(ps) = pages {
            if !ps.contains(&page_no) {
                continue;
            }
        }
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| ToolError::Execution(format!("render failed on page {page_no}: {e}")))?;
        let dynimg = bitmap.as_image();
        let mut buf: Vec<u8> = Vec::new();
        dynimg
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| ToolError::Execution(e.to_string()))?;
        out.push((page_no, base64::engine::general_purpose::STANDARD.encode(&buf)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::db::fulltext::index_attachment;
    use crate::models::EntryInput;
    use sqlx::SqlitePool;

    /// p1: OCR の保存は**全ページ / 部分ページのどちらでも**「この添付は OCR 由来」を
    /// 記録する。記録しないと LCIR 派生や添付経路の pdf_extract が後から上書きし、
    /// ユーザーが課金して起こした転写が消える（§2.6-1）。
    #[sqlx::test(migrations = "./migrations")]
    async fn saving_ocr_marks_the_attachment_as_ocr_sourced(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Scanned book".to_string(),
                entry_type: "book".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let full = add_attachment(&pool, entry.id, "a/f.pdf", "f.pdf", "application/pdf")
            .await
            .unwrap();
        let partial = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();

        save_ocr_pages(&pool, full.id, &[(1, "full transcript".to_string())], false)
            .await
            .unwrap();
        save_ocr_pages(&pool, partial.id, &[(3, "page 3 only".to_string())], true)
            .await
            .unwrap();

        for att in [full.id, partial.id] {
            assert_eq!(
                crate::db::fulltext::get_fulltext_source(&pool, att)
                    .await
                    .unwrap(),
                Some(crate::db::fulltext::FulltextSource::Ocr),
                "attachment {att} が OCR 由来として記録されていない"
            );
        }
    }

    /// **中身を 1 行も残せなかった OCR は「OCR 由来」と記録しない。**
    /// 記録だけ立てると、中身 0 行の索引を守り続けて、その添付は再索引もできなくなる。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_ocr_that_transcribed_nothing_does_not_mark_the_attachment(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Blank scan".to_string(),
                entry_type: "book".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = add_attachment(&pool, entry.id, "a/b.pdf", "b.pdf", "application/pdf")
            .await
            .unwrap();

        save_ocr_pages(&pool, att.id, &[(1, "   ".to_string())], false)
            .await
            .unwrap();

        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att.id)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att.id)
                .await
                .unwrap(),
            None,
            "守る中身が無いのに記録を立ててはいけない"
        );
    }

    /// issue #42: 索引済みの PDF を LLM に丸ごと OCR し直させない。
    /// 全ページ OCR は添付の索引を置き換えるので、テキスト層が Vision 出力で消える。
    #[sqlx::test(migrations = "./migrations")]
    async fn refuses_full_ocr_of_an_already_indexed_entry(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Indexed paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, att.id, &[(1, "existing text layer".to_string())])
            .await
            .unwrap();

        let ctx = ToolContext {
            pool: &pool,
            session_id: 1,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        };
        let call = ToolCallSpec {
            call_id: "c1".to_string(),
            tool_name: "ocr_pdf".to_string(),
            arguments: json!({ "entry_id": entry.id }),
        };
        let out = try_execute(&ctx, &call).await.unwrap().unwrap();
        assert!(out.contains("already has"), "{out}");
        assert!(out.contains("get_fulltext"), "must point at the cheap path: {out}");

        // 索引は無傷。
        let pages = crate::db::fulltext::get_entry_fulltext(&pool, entry.id).await.unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].1, "existing text layer");
    }

    /// ページ指定の部分 OCR はこの制限を受けない（差し替えなので既存を消さない）。
    /// ここでは API キーが無いので「キー未設定」まで進めば分岐を抜けたことになる。
    #[sqlx::test(migrations = "./migrations")]
    async fn partial_ocr_is_not_blocked_by_the_indexed_guard(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Indexed paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, att.id, &[(1, "existing".to_string())])
            .await
            .unwrap();

        let ctx = ToolContext {
            pool: &pool,
            session_id: 1,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        };
        let call = ToolCallSpec {
            call_id: "c1".to_string(),
            tool_name: "ocr_pdf".to_string(),
            arguments: json!({ "entry_id": entry.id, "pages": [2] }),
        };
        let out = try_execute(&ctx, &call).await.unwrap();
        match out {
            Ok(s) => assert!(!s.contains("already has"), "partial OCR must not be refused: {s}"),
            Err(e) => {
                let m = e.to_string();
                assert!(!m.contains("already has"), "partial OCR must not be refused: {m}");
            }
        }
    }
}
