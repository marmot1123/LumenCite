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
        description: "OCR a scanned PDF attachment that has no text layer: rasterize its pages, \
            transcribe them with the vision model, and index the text for full-text search. \
            Use this when fulltext_search returns nothing for an entry that has a PDF attachment."
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
    let attachment_id = call.arguments.get("attachment_id").and_then(|v| v.as_i64());
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
    match pages {
        None => crate::db::fulltext::index_attachment(pool, attachment_id, &results).await?,
        Some(_) => {
            crate::db::fulltext::update_attachment_pages(pool, attachment_id, &results).await?
        }
    }

    Ok(format!(
        "OCR'd {page_count} page(s); {total_chars} characters indexed for entry {entry_id}."
    ))
}

async fn resolve_ocr_provider(pool: &sqlx::SqlitePool) -> Result<(String, String), ToolError> {
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
    let bindings = bind_pdfium().map_err(ToolError::Execution)?;
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

/// pdfium 動的ライブラリを複数の候補から探してバインドする。
/// 候補: 実行ファイル隣 / macOS バンドルの Contents/Frameworks / Resources /
/// `pdfium`（dev では src-tauri/pdfium） / カレント → 最後にシステムライブラリ。
fn bind_pdfium() -> Result<
    std::boxed::Box<dyn pdfium_render::prelude::PdfiumLibraryBindings>,
    String,
> {
    use pdfium_render::prelude::*;
    use std::path::PathBuf;

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf()); // Contents/MacOS, または通常のバイナリ隣
            dirs.push(dir.join("../Frameworks")); // macOS .app バンドル同梱先
            dirs.push(dir.join("../Resources"));
        }
    }
    dirs.push(PathBuf::from("pdfium")); // dev: src-tauri/pdfium
    dirs.push(PathBuf::from("."));

    for dir in dirs {
        let name = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(b) = Pdfium::bind_to_library(&name) {
            return Ok(b);
        }
    }
    Pdfium::bind_to_system_library().map_err(|e| format!("pdfium library not found: {e}"))
}
