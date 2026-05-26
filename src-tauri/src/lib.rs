mod backup;
mod bibtex;
mod db;
mod keychain;
mod llm;
mod mcp;
mod metadata;
mod models;

use models::{
    Attachment, Collection, EntryDetail, EntryInput, EntrySummary, FulltextHit, ImportResult,
    SidebarCounts, Tag,
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
    SqlitePool,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{
    ipc::Channel,
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::oneshot;

pub struct AppState {
    pub db: SqlitePool,
    /// BibTeX 自動同期リクエストを送る送信側。受信側のタスクが debounce して実行する。
    pub sync_tx: UnboundedSender<()>,
    /// 進行中チャットの承認待ち・中断状態を保持する共有ランタイム。
    pub chat: Arc<ChatRuntime>,
    /// 外部 MCP サーバーのクライアント（Chat ツールへマージ）。
    pub mcp: Arc<mcp::McpManager>,
}

/// BibTeX 同期結果を UI に通知するイベントペイロード。
#[derive(Clone, serde::Serialize)]
struct BibtexSyncEvent {
    path: String,
    synced_at: Option<String>,
    error: Option<String>,
}

/// 設定された同期先があれば書き込み、結果をイベントで通知する。
async fn perform_bibtex_sync(pool: &SqlitePool, app: &AppHandle) {
    let path_str = match db::settings::get_setting(pool, db::settings::BIBTEX_SYNC_PATH_KEY).await {
        Ok(Some(p)) if !p.trim().is_empty() => p,
        _ => return, // 未設定・空のときは no-op
    };
    let path = std::path::PathBuf::from(&path_str);
    match bibtex::sync_bibtex(pool, &path).await {
        Ok(()) => {
            let now = chrono_now_iso();
            let _ = app.emit(
                "bibtex-synced",
                BibtexSyncEvent { path: path_str, synced_at: Some(now), error: None },
            );
        }
        Err(e) => {
            let _ = app.emit(
                "bibtex-synced",
                BibtexSyncEvent { path: path_str, synced_at: None, error: Some(e) },
            );
        }
    }
}

fn chrono_now_iso() -> String {
    // chrono を増やさず std だけで ISO 8601 風の文字列を作る。秒精度・ローカル TZ なしで十分。
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 単なる epoch 秒を文字列に。フロントで Date(parseInt * 1000) する想定にしてもよいが、
    // ここでは扱いやすさを優先して "2026-05-11T03:04:05Z" 風には作らず epoch 秒の文字列にする。
    format!("{}", secs)
}

/// 同期リクエスト受信タスク。trailing-edge debounce で、最後のリクエストから
/// 静かな期間が続いたら同期を発火する。
async fn run_sync_task(
    pool: SqlitePool,
    app: AppHandle,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    let debounce = Duration::from_millis(800);
    while rx.recv().await.is_some() {
        // 最初のリクエストを受け取った。debounce 窓内に追加リクエストが来たら待ち直す。
        loop {
            match tokio::time::timeout(debounce, rx.recv()).await {
                Ok(Some(())) => continue, // 追加リクエスト → 待ち直し
                Ok(None) => return,        // 送信側が全部 drop された
                Err(_) => break,           // 静かになった → 発火
            }
        }
        perform_bibtex_sync(&pool, &app).await;
    }
}

// ── entries ──────────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_entries(
    state: State<'_, AppState>,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<String>,
) -> Result<Vec<EntrySummary>, String> {
    db::entries::get_entries(&state.db, collection_id, tag_id, view.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_starred(
    state: State<'_, AppState>,
    id: i64,
    starred: bool,
) -> Result<(), String> {
    db::entries::set_starred(&state.db, id, starred)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn trash_entry(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::entries::trash_entry(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn restore_entry(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::entries::restore_entry(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn get_sidebar_counts(state: State<'_, AppState>) -> Result<SidebarCounts, String> {
    db::entries::get_sidebar_counts(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn bulk_trash(state: State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    db::entries::bulk_trash(&state.db, &ids)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn bulk_restore(state: State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    db::entries::bulk_restore(&state.db, &ids)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn bulk_purge(state: State<'_, AppState>, ids: Vec<i64>) -> Result<(), String> {
    db::entries::bulk_purge(&state.db, &ids)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn bulk_add_to_collection(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    collection_id: i64,
) -> Result<(), String> {
    db::entries::bulk_add_to_collection(&state.db, &ids, collection_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn bulk_add_tag(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    tag_id: i64,
) -> Result<(), String> {
    db::entries::bulk_add_tag(&state.db, &ids, tag_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_entry(state: State<'_, AppState>, id: i64) -> Result<EntryDetail, String> {
    db::entries::get_entry(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

/// state.sync_tx に best-effort で送る。受信側が drop されていても無視する。
fn request_sync(state: &State<'_, AppState>) {
    let _ = state.sync_tx.send(());
}

#[tauri::command]
async fn create_entry(
    state: State<'_, AppState>,
    input: EntryInput,
) -> Result<EntryDetail, String> {
    let r = db::entries::create_entry(&state.db, &input)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(r)
}

#[tauri::command]
async fn update_entry(
    state: State<'_, AppState>,
    id: i64,
    input: EntryInput,
) -> Result<EntryDetail, String> {
    let r = db::entries::update_entry(&state.db, id, &input)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(r)
}

#[tauri::command]
async fn delete_entry(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    // attachments の cascade では fulltext は消えないので先に消す
    let _ = db::fulltext::unindex_entry(&state.db, id).await;
    db::entries::delete_entry(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn search_entries(
    state: State<'_, AppState>,
    query: String,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
) -> Result<Vec<EntrySummary>, String> {
    db::entries::search_entries(&state.db, &query, collection_id, tag_id)
        .await
        .map_err(|e| e.to_string())
}

// ── tags ──────────────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_tags(state: State<'_, AppState>) -> Result<Vec<Tag>, String> {
    db::tags::get_tags(&state.db).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_tag(state: State<'_, AppState>, name: String) -> Result<Tag, String> {
    db::tags::create_tag(&state.db, &name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_tag(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::tags::delete_tag(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_tag_to_entry(
    state: State<'_, AppState>,
    entry_id: i64,
    tag_id: i64,
) -> Result<(), String> {
    db::tags::add_tag_to_entry(&state.db, entry_id, tag_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remove_tag_from_entry(
    state: State<'_, AppState>,
    entry_id: i64,
    tag_id: i64,
) -> Result<(), String> {
    db::tags::remove_tag_from_entry(&state.db, entry_id, tag_id)
        .await
        .map_err(|e| e.to_string())
}

// ── collections ───────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_collections(state: State<'_, AppState>) -> Result<Vec<Collection>, String> {
    db::collections::get_collections(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_collection(
    state: State<'_, AppState>,
    name: String,
    parent_id: Option<i64>,
) -> Result<Collection, String> {
    db::collections::create_collection(&state.db, &name, parent_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_collection(
    state: State<'_, AppState>,
    id: i64,
    name: String,
) -> Result<Collection, String> {
    db::collections::update_collection(&state.db, id, &name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_collection(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::collections::delete_collection(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_entry_to_collection(
    state: State<'_, AppState>,
    entry_id: i64,
    collection_id: i64,
) -> Result<(), String> {
    db::collections::add_entry_to_collection(&state.db, entry_id, collection_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remove_entry_from_collection(
    state: State<'_, AppState>,
    entry_id: i64,
    collection_id: i64,
) -> Result<(), String> {
    db::collections::remove_entry_from_collection(&state.db, entry_id, collection_id)
        .await
        .map_err(|e| e.to_string())
}

// ── bibtex ────────────────────────────────────────────────────────────────────

#[tauri::command]
async fn import_bibtex(
    state: State<'_, AppState>,
    content: String,
) -> Result<ImportResult, String> {
    let r = bibtex::import_bibtex(&state.db, &content).await?;
    request_sync(&state);
    Ok(r)
}

/// .bib ファイル選択ダイアログを開いてパスを返す。
#[tauri::command]
async fn pick_bibtex_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let path = app
            .dialog()
            .file()
            .add_filter("BibTeX", &["bib", "bibtex"])
            .blocking_pick_file();
        Ok::<Option<String>, String>(
            path.and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().to_string()),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 指定パスの .bib ファイルを読み込んでインポートする。
#[tauri::command]
async fn import_bibtex_file(
    state: State<'_, AppState>,
    path: String,
) -> Result<ImportResult, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let r = bibtex::import_bibtex(&state.db, &content).await?;
    request_sync(&state);
    Ok(r)
}

#[tauri::command]
async fn export_bibtex(
    state: State<'_, AppState>,
    entry_ids: Option<Vec<i64>>,
) -> Result<String, String> {
    bibtex::export_bibtex(&state.db, entry_ids).await
}

#[tauri::command]
async fn save_bibtex(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    entry_ids: Option<Vec<i64>>,
    default_name: Option<String>,
    default_directory: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let content = bibtex::export_bibtex(&state.db, entry_ids).await?;
    let default_name = default_name.unwrap_or_else(|| "lumencite.bib".to_string());

    // 同期パスのような既存ファイル絶対パスが渡された場合は親ディレクトリを抽出する。
    // 既にディレクトリならそのまま使う。存在しない・空文字なら指定なしと同等。
    let initial_dir = default_directory
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| {
            let p = PathBuf::from(&s);
            if p.is_dir() {
                Some(p)
            } else {
                p.parent().filter(|d| d.is_dir()).map(|d| d.to_path_buf())
            }
        });

    tauri::async_runtime::spawn_blocking(move || {
        let mut builder = app
            .dialog()
            .file()
            .set_file_name(&default_name)
            .add_filter("BibTeX", &["bib"]);
        if let Some(dir) = initial_dir {
            builder = builder.set_directory(dir);
        }
        let Some(path) = builder.blocking_save_file() else {
            return Ok(None);
        };
        let path_buf = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path_buf, content).map_err(|e| e.to_string())?;
        Ok(Some(path_buf.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── attachments ───────────────────────────────────────────────────────────────

fn attachments_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("attachments"))
}

fn unique_dest(dir: &std::path::Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (file_name.to_string(), String::new()),
    };
    for i in 1..1000 {
        let next = dir.join(format!("{stem}_{i}{ext}"));
        if !next.exists() {
            return next;
        }
    }
    dir.join(format!(
        "{stem}_{}{ext}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ))
}

#[tauri::command]
async fn pick_pdf_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let path = app
            .dialog()
            .file()
            .add_filter("PDF", &["pdf"])
            .blocking_pick_file();
        let Some(p) = path else { return Ok(None) };
        let buf = p.into_path().map_err(|e| e.to_string())?;
        Ok(Some(buf.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn add_attachment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    entry_id: i64,
    source_path: String,
) -> Result<Attachment, String> {
    let src = PathBuf::from(&source_path);
    if !src.exists() {
        return Err(format!("ファイルが見つかりません: {source_path}"));
    }
    let file_name = src
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| "ファイル名を取得できません".to_string())?;

    let root = attachments_root(&app)?;
    let entry_dir = root.join(entry_id.to_string());
    std::fs::create_dir_all(&entry_dir).map_err(|e| e.to_string())?;

    let dest = unique_dest(&entry_dir, &file_name);
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;

    let rel_path = format!(
        "attachments/{}/{}",
        entry_id,
        dest.file_name().unwrap().to_string_lossy()
    );
    let dest_name = dest.file_name().unwrap().to_string_lossy().to_string();

    let result = db::attachments::add_attachment(
        &state.db,
        entry_id,
        &rel_path,
        &dest_name,
        "application/pdf",
    )
    .await;

    match result {
        Ok(att) => Ok(att),
        Err(e) => {
            // DB 登録失敗時はコピー済みファイルを掃除する
            let _ = std::fs::remove_file(&dest);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn delete_attachment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    // fulltext は FK が無いので明示的に消す（attachments の cascade では拾えない）
    let _ = db::fulltext::unindex_attachment(&state.db, id).await;

    let removed = db::attachments::delete_attachment(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;

    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let abs = root.join(&removed.file_path);
    let _ = std::fs::remove_file(&abs);
    Ok(())
}

#[tauri::command]
async fn read_attachment_bytes(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<u8>, String> {
    let att = db::attachments::get_attachment_with_path(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;

    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let abs = root.join(&att.file_path);
    std::fs::read(&abs).map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_pdf_viewer(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
    page: Option<i64>,
) -> Result<(), String> {
    let att = db::attachments::get_attachment_with_path(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;

    let label = format!("pdf-viewer-{id}");

    // 既に同じ添付のウィンドウが開いていればフォーカスし、page 指定があれば送る
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.set_focus();
        if let Some(p) = page {
            let _ = win.emit("jump-to-page", p);
        }
        return Ok(());
    }

    let mut url_str = format!("pdf-viewer.html?id={id}");
    if let Some(p) = page {
        url_str.push_str(&format!("&page={p}"));
    }
    let url = WebviewUrl::App(url_str.into());
    WebviewWindowBuilder::new(&app, label, url)
        .title(&att.file_name)
        .inner_size(1100.0, 900.0)
        .min_inner_size(600.0, 500.0)
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
async fn index_attachment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<i64, String> {
    let att = db::attachments::get_attachment_with_path(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;

    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let abs = root.join(&att.file_path);

    // pdf-extract は同期で重い CPU 処理なので blocking スレッドへ逃がす
    let pages_text = tauri::async_runtime::spawn_blocking(move || {
        pdf_extract::extract_text_by_pages(&abs).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    let pages: Vec<(i64, String)> = pages_text
        .into_iter()
        .enumerate()
        .map(|(i, t)| ((i + 1) as i64, t))
        .collect();
    let indexed_pages = pages.iter().filter(|(_, t)| !t.trim().is_empty()).count() as i64;

    db::fulltext::index_attachment(&state.db, id, &pages)
        .await
        .map_err(|e| e.to_string())?;

    Ok(indexed_pages)
}

#[tauri::command]
async fn unindex_attachment(
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    db::fulltext::unindex_attachment(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn is_attachment_indexed(state: State<'_, AppState>, id: i64) -> Result<bool, String> {
    db::fulltext::is_indexed(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn fulltext_search(
    state: State<'_, AppState>,
    query: String,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
) -> Result<Vec<FulltextHit>, String> {
    db::fulltext::search_fulltext(&state.db, &query, collection_id, tag_id)
        .await
        .map_err(|e| e.to_string())
}

// ── highlights ──────────────────────────────────────────────────────────────

#[tauri::command]
async fn get_highlights(
    state: State<'_, AppState>,
    entry_id: i64,
) -> Result<Vec<db::highlights::Highlight>, String> {
    db::highlights::list_by_entry(&state.db, entry_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_highlight(
    state: State<'_, AppState>,
    input: db::highlights::HighlightInput,
) -> Result<db::highlights::Highlight, String> {
    db::highlights::create(&state.db, &input)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_highlight(
    state: State<'_, AppState>,
    id: i64,
    patch: db::highlights::HighlightUpdate,
) -> Result<db::highlights::Highlight, String> {
    db::highlights::update(&state.db, id, &patch)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_highlight(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::highlights::delete(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

// ── LLM 設定 / 要約 ─────────────────────────────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct LlmSettings {
    pub provider: String,         // "openai" | "anthropic"
    pub model: String,
    pub summary_source: String,   // "abstract" | "fulltext"
    pub summary_prompt: String,   // 空文字なら llm::DEFAULT_SYSTEM_PROMPT
}

#[tauri::command]
async fn get_llm_settings(state: State<'_, AppState>) -> Result<LlmSettings, String> {
    let provider = db::settings::get_setting(&state.db, db::settings::LLM_PROVIDER_KEY)
        .await.map_err(|e| e.to_string())?
        .unwrap_or_else(|| "openai".to_string());
    let model = db::settings::get_setting(&state.db, db::settings::LLM_MODEL_KEY)
        .await.map_err(|e| e.to_string())?
        .unwrap_or_else(|| {
            match provider.as_str() {
                "anthropic" => "claude-haiku-4-5-20251001".to_string(),
                _ => "gpt-4o-mini".to_string(),
            }
        });
    let summary_source = db::settings::get_setting(&state.db, db::settings::LLM_SUMMARY_SOURCE_KEY)
        .await.map_err(|e| e.to_string())?
        .unwrap_or_else(|| "abstract".to_string());
    let summary_prompt = db::settings::get_setting(&state.db, db::settings::LLM_SUMMARY_PROMPT_KEY)
        .await.map_err(|e| e.to_string())?
        .unwrap_or_default();
    Ok(LlmSettings { provider, model, summary_source, summary_prompt })
}

#[tauri::command]
async fn save_llm_settings(state: State<'_, AppState>, settings: LlmSettings) -> Result<(), String> {
    db::settings::set_setting(&state.db, db::settings::LLM_PROVIDER_KEY, &settings.provider)
        .await.map_err(|e| e.to_string())?;
    db::settings::set_setting(&state.db, db::settings::LLM_MODEL_KEY, &settings.model)
        .await.map_err(|e| e.to_string())?;
    db::settings::set_setting(&state.db, db::settings::LLM_SUMMARY_SOURCE_KEY, &settings.summary_source)
        .await.map_err(|e| e.to_string())?;
    db::settings::set_setting(&state.db, db::settings::LLM_SUMMARY_PROMPT_KEY, &settings.summary_prompt)
        .await.map_err(|e| e.to_string())?;
    Ok(())
}

/// デフォルトのシステムプロンプトをフロントから取れるようにするユーティリティ。
#[tauri::command]
fn get_default_summary_prompt() -> String {
    llm::DEFAULT_SYSTEM_PROMPT.to_string()
}

#[tauri::command]
async fn set_api_key(provider: String, key: String) -> Result<(), String> {
    let account = keychain::account_for_api_key(&provider);
    if key.trim().is_empty() {
        keychain::delete(&account).map_err(|e| e.to_string())
    } else {
        keychain::set(&account, key.trim()).map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn delete_api_key(provider: String) -> Result<(), String> {
    let account = keychain::account_for_api_key(&provider);
    keychain::delete(&account).map_err(|e| e.to_string())
}

/// API キーの有無のみを返す（実値はフロントに返さない）。
#[tauri::command]
async fn has_api_key(provider: String) -> Result<bool, String> {
    let account = keychain::account_for_api_key(&provider);
    let v = keychain::get(&account).map_err(|e| e.to_string())?;
    Ok(v.map(|s| !s.trim().is_empty()).unwrap_or(false))
}

#[tauri::command]
async fn test_llm_connection(provider: String, model: String) -> Result<(), String> {
    let account = keychain::account_for_api_key(&provider);
    let key = keychain::get(&account).map_err(|e| e.to_string())?
        .ok_or_else(|| "API key is not configured".to_string())?;
    llm::test_connection(&provider, &model, &key).await.map_err(|e| e.to_string())
}

#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SummaryStreamEvent {
    Start { model: String },
    Delta { text: String },
    Done { full_text: String },
    Error { message: String },
}

#[tauri::command]
async fn generate_summary(
    state: State<'_, AppState>,
    entry_id: i64,
    source: String,
    channel: Channel<SummaryStreamEvent>,
) -> Result<(), String> {
    // エントリと LLM 設定を読み込む
    let entry = db::entries::get_entry(&state.db, entry_id)
        .await.map_err(|e| e.to_string())?;
    let settings = get_llm_settings(state.clone()).await?;
    let account = keychain::account_for_api_key(&settings.provider);
    let api_key = keychain::get(&account).map_err(|e| e.to_string())?
        .ok_or_else(|| "API key is not configured".to_string())?;

    // 要約対象テキストを決める
    let body = if source == "fulltext" {
        let texts: Vec<String> = sqlx::query_scalar(
            "SELECT content FROM fulltext WHERE attachment_id IN
             (SELECT id FROM attachments WHERE entry_id = ?) ORDER BY page",
        )
        .bind(entry_id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
        if texts.is_empty() {
            entry.abstract_.clone().unwrap_or_default()
        } else {
            // 1 リクエストで送れる範囲に切り詰める（おおむね 24K 文字）
            let mut joined = texts.join("\n\n");
            const MAX_CHARS: usize = 24_000;
            if joined.chars().count() > MAX_CHARS {
                joined = joined.chars().take(MAX_CHARS).collect();
            }
            joined
        }
    } else {
        entry.abstract_.clone().unwrap_or_default()
    };

    if body.trim().is_empty() {
        let _ = channel.send(SummaryStreamEvent::Error {
            message: "no content to summarize".to_string(),
        });
        return Err("no content to summarize".to_string());
    }

    let _ = channel.send(SummaryStreamEvent::Start { model: settings.model.clone() });

    let ch_for_delta = channel.clone();
    let result = llm::generate_summary(
        &settings.provider,
        &settings.model,
        &api_key,
        &settings.summary_prompt,
        &entry.title,
        &body,
        move |delta| {
            let _ = ch_for_delta.send(SummaryStreamEvent::Delta { text: delta.to_string() });
        },
    )
    .await;

    match result {
        Ok(full_text) => {
            let _ = channel.send(SummaryStreamEvent::Done { full_text });
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = channel.send(SummaryStreamEvent::Error { message: msg.clone() });
            Err(msg)
        }
    }
}

#[tauri::command]
async fn save_entry_summary(
    state: State<'_, AppState>,
    id: i64,
    summary: String,
    model: String,
) -> Result<(), String> {
    db::entries::set_summary(&state.db, id, &summary, &model)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

// ── chat（agentic LLM チャット）─────────────────────────────────────────────

/// agentic ループの進行をフロントへ流す Tauri レベルのストリーミングイベント。
#[derive(Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChatStreamEvent {
    SessionStarted {
        session_id: i64,
    },
    Delta {
        text: String,
    },
    ToolCallProposed {
        call_id: String,
        tool_name: String,
        args_preview: String,
        needs_approval: bool,
    },
    ToolCallExecuted {
        call_id: String,
        result_summary: String,
    },
    MessagePersisted {
        message_id: i64,
        role: String,
    },
    Done,
    Error {
        message: String,
    },
}

/// 進行中チャットの承認待ち・中断状態を保持する共有ランタイム。
#[derive(Default)]
pub struct ChatRuntime {
    /// call_id -> 承認待ちの送信側
    pending: Mutex<HashMap<String, PendingApproval>>,
    /// session_id -> 中断フラグ
    cancels: Mutex<HashMap<i64, Arc<AtomicBool>>>,
}

struct PendingApproval {
    session_id: i64,
    tx: oneshot::Sender<bool>,
}

impl ChatRuntime {
    /// セッションの中断フラグを作成・登録して返す。
    fn begin(&self, session_id: i64) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels.lock().unwrap().insert(session_id, flag.clone());
        flag
    }

    /// セッション終了時の後始末（中断フラグと残った承認待ちを除去）。
    fn finish(&self, session_id: i64) {
        self.cancels.lock().unwrap().remove(&session_id);
        self.pending.lock().unwrap().retain(|_, p| p.session_id != session_id);
    }

    /// ツール承認待ちを登録し、決定を待つ受信側を返す。
    fn register_approval(&self, session_id: i64, call_id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert(call_id.to_string(), PendingApproval { session_id, tx });
        rx
    }

    /// UI からの承認/拒否を該当する待ちに伝える。
    fn resolve_approval(&self, call_id: &str, approved: bool) {
        if let Some(p) = self.pending.lock().unwrap().remove(call_id) {
            let _ = p.tx.send(approved);
        }
    }

    /// セッションを中断する。中断フラグを立て、当該セッションの承認待ちは拒否扱いで解放する。
    fn cancel(&self, session_id: i64) {
        if let Some(flag) = self.cancels.lock().unwrap().get(&session_id) {
            flag.store(true, Ordering::SeqCst);
        }
        let mut pending = self.pending.lock().unwrap();
        let ids: Vec<String> = pending
            .iter()
            .filter(|(_, p)| p.session_id == session_id)
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            if let Some(p) = pending.remove(&id) {
                let _ = p.tx.send(false);
            }
        }
    }
}

/// `ChatLoopHost` を Tauri Channel + ChatRuntime に橋渡しする実装。
struct ChannelHost {
    channel: Channel<ChatStreamEvent>,
    runtime: Arc<ChatRuntime>,
    session_id: i64,
    cancel: Arc<AtomicBool>,
}

fn role_label(role: llm::Role) -> String {
    match role {
        llm::Role::User => "user",
        llm::Role::Assistant => "assistant",
        llm::Role::Tool => "tool",
    }
    .to_string()
}

/// UI 表示用に文字列を最大 `max` 文字へ丸める。
fn clip_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

#[async_trait::async_trait]
impl llm::chat::ChatLoopHost for ChannelHost {
    fn on_delta(&mut self, text: &str) {
        let _ = self
            .channel
            .send(ChatStreamEvent::Delta { text: text.to_string() });
    }

    async fn on_tool_proposed(&mut self, call: &llm::ToolCallSpec, needs_approval: bool) {
        let _ = self.channel.send(ChatStreamEvent::ToolCallProposed {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            args_preview: clip_text(&call.arguments.to_string(), 200),
            needs_approval,
        });
    }

    async fn request_approval(&mut self, call: &llm::ToolCallSpec) -> bool {
        let rx = self.runtime.register_approval(self.session_id, &call.call_id);
        rx.await.unwrap_or(false)
    }

    async fn on_tool_executed(&mut self, call_id: &str, result_summary: &str) {
        let _ = self.channel.send(ChatStreamEvent::ToolCallExecuted {
            call_id: call_id.to_string(),
            result_summary: clip_text(result_summary, 500),
        });
    }

    async fn on_message_persisted(&mut self, message_id: i64, role: llm::Role) {
        let _ = self.channel.send(ChatStreamEvent::MessagePersisted {
            message_id,
            role: role_label(role),
        });
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// DB の chat_messages 行を、プロバイダに渡す `llm::ChatMessage` 列へ変換する。
fn db_messages_to_chat(rows: &[db::chat::ChatMessage]) -> Vec<llm::ChatMessage> {
    rows.iter()
        .map(|r| match r.role.as_str() {
            "assistant" => {
                let tool_calls = r
                    .tool_calls
                    .as_ref()
                    .and_then(|s| serde_json::from_str::<Vec<llm::ToolCallSpec>>(s).ok())
                    .filter(|v| !v.is_empty());
                llm::ChatMessage {
                    role: llm::Role::Assistant,
                    content: vec![llm::ContentBlock::text(r.content.clone())],
                    tool_calls,
                    tool_call_id: None,
                }
            }
            "tool" => llm::ChatMessage::tool_result(
                r.tool_call_id.clone().unwrap_or_default(),
                r.content.clone(),
            ),
            _ => llm::ChatMessage::user_text(r.content.clone()),
        })
        .collect()
}

#[tauri::command]
async fn list_chat_sessions(
    state: State<'_, AppState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<db::chat::ChatSession>, String> {
    db::chat::list_sessions(&state.db, limit.unwrap_or(100), offset.unwrap_or(0))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn create_chat_session(
    state: State<'_, AppState>,
    title: String,
    provider: String,
    model: String,
    scope_mode: String,
    entry_ids: Vec<i64>,
) -> Result<db::chat::ChatSession, String> {
    db::chat::create_session(
        &state.db,
        &db::chat::NewChatSession {
            title,
            provider,
            model,
            system_prompt: None,
            scope_mode,
            entry_ids,
        },
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_chat_session(
    state: State<'_, AppState>,
    id: i64,
) -> Result<db::chat::SessionWithMessages, String> {
    db::chat::get_session_with_messages(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_chat_session_title(
    state: State<'_, AppState>,
    id: i64,
    title: String,
) -> Result<db::chat::ChatSession, String> {
    db::chat::update_title(&state.db, id, &title)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn archive_chat_session(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    db::chat::archive_session(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_chat_session_scope(
    state: State<'_, AppState>,
    id: i64,
    scope_mode: String,
    entry_ids: Vec<i64>,
) -> Result<db::chat::ChatSession, String> {
    db::chat::set_scope(&state.db, id, &scope_mode, &entry_ids)
        .await
        .map_err(|e| e.to_string())
}

// ── MCP クライアント ─────────────────────────────────────────────────────────

#[tauri::command]
async fn list_mcp_servers(state: State<'_, AppState>) -> Result<Vec<mcp::McpServerConfig>, String> {
    let json = db::settings::get_setting(&state.db, db::settings::MCP_SERVERS_KEY)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    Ok(mcp::parse_servers_config(&json))
}

#[tauri::command]
async fn add_mcp_server(
    state: State<'_, AppState>,
    config: mcp::McpServerConfig,
) -> Result<(), String> {
    let json = db::settings::get_setting(&state.db, db::settings::MCP_SERVERS_KEY)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let mut servers = mcp::parse_servers_config(&json);
    servers.retain(|s| s.id != config.id);
    servers.push(config.clone());
    db::settings::set_setting(
        &state.db,
        db::settings::MCP_SERVERS_KEY,
        &mcp::serialize_servers_config(&servers),
    )
    .await
    .map_err(|e| e.to_string())?;
    state.mcp.start(config).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn remove_mcp_server(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let json = db::settings::get_setting(&state.db, db::settings::MCP_SERVERS_KEY)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let mut servers = mcp::parse_servers_config(&json);
    servers.retain(|s| s.id != id);
    db::settings::set_setting(
        &state.db,
        db::settings::MCP_SERVERS_KEY,
        &mcp::serialize_servers_config(&servers),
    )
    .await
    .map_err(|e| e.to_string())?;
    state.mcp.stop(&id).await;
    Ok(())
}

#[tauri::command]
async fn approve_tool_call(
    state: State<'_, AppState>,
    call_id: String,
    approved: bool,
) -> Result<(), String> {
    state.chat.resolve_approval(&call_id, approved);
    Ok(())
}

#[tauri::command]
async fn cancel_chat_stream(state: State<'_, AppState>, session_id: i64) -> Result<(), String> {
    state.chat.cancel(session_id);
    Ok(())
}

/// agentic ループのエントリポイント。user メッセージを永続化し、会話履歴を読み込み、
/// ループを実行して進行を `channel` で配信する。
#[tauri::command]
async fn chat_send_message(
    state: State<'_, AppState>,
    session_id: i64,
    user_text: String,
    channel: Channel<ChatStreamEvent>,
) -> Result<(), String> {
    let pool = state.db.clone();
    let session = db::chat::get_session(&pool, session_id)
        .await
        .map_err(|e| e.to_string())?;

    let account = keychain::account_for_api_key(&session.provider);
    let api_key = match keychain::get(&account) {
        Ok(Some(k)) if !k.trim().is_empty() => k,
        _ => {
            let _ = channel.send(ChatStreamEvent::Error {
                message: "API key is not configured".to_string(),
            });
            return Err("API key is not configured".to_string());
        }
    };

    let _ = channel.send(ChatStreamEvent::SessionStarted { session_id });

    // user メッセージを永続化
    let user_row = db::chat::append_message(
        &pool,
        &db::chat::NewChatMessage {
            session_id,
            role: "user".to_string(),
            content: user_text,
            tool_calls: None,
            tool_call_id: None,
        },
    )
    .await
    .map_err(|e| e.to_string())?;
    let _ = channel.send(ChatStreamEvent::MessagePersisted {
        message_id: user_row.id,
        role: "user".to_string(),
    });

    // 会話履歴・スコープ・ホワイトリストを読み込む
    let swm = db::chat::get_session_with_messages(&pool, session_id)
        .await
        .map_err(|e| e.to_string())?;
    let messages = db_messages_to_chat(&swm.messages);
    let entry_ids = swm.entry_ids;
    let whitelist = db::settings::get_setting(&pool, db::settings::CHAT_TOOL_WHITELIST_KEY)
        .await
        .ok()
        .flatten();
    let system = session
        .system_prompt
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| llm::chat::DEFAULT_CHAT_SYSTEM_PROMPT.to_string());

    let provider = match llm::provider_for(&session.provider) {
        Ok(p) => p,
        Err(e) => {
            let _ = channel.send(ChatStreamEvent::Error { message: e.to_string() });
            return Err(e.to_string());
        }
    };
    let mut tools = llm::tools::all_tool_specs();
    tools.extend(state.mcp.tool_specs().await);

    let cancel = state.chat.begin(session_id);
    let mut host = ChannelHost {
        channel: channel.clone(),
        runtime: state.chat.clone(),
        session_id,
        cancel,
    };

    let ctx = llm::tools::ToolContext {
        pool: &pool,
        session_id,
        scope_mode: &session.scope_mode,
        scope_entry_ids: &entry_ids,
        mcp: Some(state.mcp.as_ref()),
    };
    let params = llm::chat::ChatLoopParams {
        api_key: &api_key,
        model: &session.model,
        system: &system,
        whitelist: whitelist.as_deref(),
        max_turns: llm::chat::DEFAULT_MAX_TURNS,
    };

    let result =
        llm::chat::run_chat_loop(provider.as_ref(), &ctx, &tools, messages, &params, &mut host)
            .await;
    state.chat.finish(session_id);

    match result {
        Ok(()) => {
            let _ = channel.send(ChatStreamEvent::Done);
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = channel.send(ChatStreamEvent::Error { message: msg.clone() });
            Err(msg)
        }
    }
}

/// セッションの最初の数メッセージから簡潔なタイトルを LLM 生成し、保存して返す。
#[tauri::command]
async fn generate_chat_title(state: State<'_, AppState>, session_id: i64) -> Result<String, String> {
    let session = db::chat::get_session(&state.db, session_id)
        .await
        .map_err(|e| e.to_string())?;
    let swm = db::chat::get_session_with_messages(&state.db, session_id)
        .await
        .map_err(|e| e.to_string())?;

    let mut body = String::new();
    for m in swm.messages.iter().take(4) {
        if m.role == "tool" {
            continue;
        }
        body.push_str(&format!("{}: {}\n", m.role, m.content));
    }
    if body.trim().is_empty() {
        return Ok(session.title);
    }

    let account = keychain::account_for_api_key(&session.provider);
    let api_key = keychain::get(&account)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "API key is not configured".to_string())?;

    let raw = llm::generate_summary(
        &session.provider,
        &session.model,
        &api_key,
        "Generate a concise 3-6 word title summarizing this chat. \
         Reply with only the title — no quotes and no trailing punctuation.",
        "Conversation",
        &body,
        |_| {},
    )
    .await
    .map_err(|e| e.to_string())?;

    let title = raw.trim().trim_matches('"').trim().to_string();
    let title = if title.is_empty() { session.title } else { title };
    db::chat::update_title(&state.db, session_id, &title)
        .await
        .map_err(|e| e.to_string())?;
    Ok(title)
}

// ── バックアップ / エクスポート ─────────────────────────────────────────────

#[tauri::command]
async fn run_backup_now(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let path = backup::run_backup(&state.db, &dir, 14).await?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn list_backups(app: tauri::AppHandle) -> Result<Vec<backup::BackupInfo>, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    backup::list_backups(&dir)
}

#[tauri::command]
fn open_backup_folder(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let backups_dir = dir.join("backups");
    std::fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(backups_dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn export_database_json(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    // 全エントリ ID を取得して、それぞれ詳細を読み込む
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM entries ORDER BY id")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let mut all: Vec<models::EntryDetail> = Vec::with_capacity(ids.len());
    for id in ids {
        let detail = db::entries::get_entry(&state.db, id).await.map_err(|e| e.to_string())?;
        all.push(detail);
    }
    let json = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;

    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .set_file_name("lumencite-export.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path_buf = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path_buf, json).map_err(|e| e.to_string())?;
        Ok(Some(path_buf.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn export_database_markdown(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use std::fmt::Write;
    use tauri_plugin_dialog::DialogExt;

    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM entries WHERE deleted_at IS NULL ORDER BY title")
        .fetch_all(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let mut md = String::new();
    md.push_str("# LumenCite Export\n\n");
    for id in ids {
        let detail = db::entries::get_entry(&state.db, id).await.map_err(|e| e.to_string())?;
        writeln!(md, "## {}\n", detail.title).ok();
        if !detail.authors.is_empty() {
            let authors: Vec<String> = detail.authors.iter().map(|a| a.name.clone()).collect();
            writeln!(md, "**Authors:** {}\n", authors.join(", ")).ok();
        }
        if let Some(y) = detail.year {
            writeln!(md, "**Year:** {}\n", y).ok();
        }
        if let Some(doi) = &detail.doi {
            writeln!(md, "**DOI:** {}\n", doi).ok();
        }
        if let Some(arxiv) = &detail.arxiv_id {
            writeln!(md, "**arXiv:** {}\n", arxiv).ok();
        }
        if let Some(abstract_) = &detail.abstract_ {
            if !abstract_.trim().is_empty() {
                writeln!(md, "### Abstract\n\n{}\n", abstract_).ok();
            }
        }
        if let Some(notes) = &detail.notes {
            if !notes.trim().is_empty() {
                writeln!(md, "### Notes\n\n{}\n", notes).ok();
            }
        }
        if let Some(summary) = &detail.summary {
            if !summary.trim().is_empty() {
                writeln!(md, "### Summary\n\n{}\n", summary).ok();
            }
        }
        md.push_str("\n---\n\n");
    }

    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .set_file_name("lumencite-export.md")
            .add_filter("Markdown", &["md"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path_buf = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path_buf, md).map_err(|e| e.to_string())?;
        Ok(Some(path_buf.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── bibtex sync settings ─────────────────────────────────────────────────────

/// アプリ名サブメニューに「Settings…」を入れた標準メニューを構築して設定する。
/// `Menu::default` 相当の構造を踏襲しつつ、アプリメニューだけ独自にする。
fn install_app_menu(app: &AppHandle) -> tauri::Result<()> {
    let pkg = app.package_info();
    let config = app.config();
    let about = AboutMetadata {
        name: Some(pkg.name.clone()),
        version: Some(pkg.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    let settings_item = MenuItem::with_id(
        app,
        "open-settings",
        "Settings…",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    // OS 標準の About ダイアログは使わず、アプリ内の About タブを開くカスタム項目にする。
    let about_item = MenuItem::with_id(
        app,
        "open-about",
        format!("About {}", pkg.name),
        true,
        None::<&str>,
    )?;
    let _ = &about; // about metadata は OS 標準ダイアログ用だったので未使用化

    #[cfg(target_os = "macos")]
    let app_submenu = Submenu::with_items(
        app,
        &pkg.name,
        true,
        &[
            &about_item,
            &PredefinedMenuItem::separator(app)?,
            &settings_item,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    let edit_submenu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;

    #[cfg(target_os = "macos")]
    let view_submenu = Submenu::with_items(
        app,
        "View",
        true,
        &[&PredefinedMenuItem::fullscreen(app, None)?],
    )?;

    let window_submenu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            #[cfg(target_os = "macos")]
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;

    #[cfg(not(target_os = "macos"))]
    let file_submenu = Submenu::with_items(
        app,
        "File",
        true,
        &[
            &settings_item,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, None)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;

    // macOS では Help にも About を入れない（アプリメニューの About に集約）。
    // Windows/Linux では Help メニューに About を入れる（macOS のアプリメニューが無いため）。
    #[cfg(target_os = "macos")]
    let help_items: Vec<&dyn tauri::menu::IsMenuItem<_>> = vec![];
    #[cfg(not(target_os = "macos"))]
    let help_items: Vec<&dyn tauri::menu::IsMenuItem<_>> = vec![&about_item];
    let help_submenu = Submenu::with_items(app, "Help", true, &help_items)?;

    #[cfg(target_os = "macos")]
    let menu = Menu::with_items(
        app,
        &[&app_submenu, &edit_submenu, &view_submenu, &window_submenu, &help_submenu],
    )?;
    #[cfg(not(target_os = "macos"))]
    let menu = Menu::with_items(
        app,
        &[&file_submenu, &edit_submenu, &window_submenu, &help_submenu],
    )?;

    app.set_menu(menu)?;
    app.on_menu_event(|app_handle, event| {
        match event.id().as_ref() {
            "open-settings" => { let _ = app_handle.emit("open-settings", ()); }
            "open-about"    => { let _ = app_handle.emit("open-about", ()); }
            _ => {}
        }
    });
    Ok(())
}

#[tauri::command]
async fn get_bibtex_sync_path(state: State<'_, AppState>) -> Result<Option<String>, String> {
    db::settings::get_setting(&state.db, db::settings::BIBTEX_SYNC_PATH_KEY)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_bibtex_sync_path(
    state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    db::settings::set_setting(&state.db, db::settings::BIBTEX_SYNC_PATH_KEY, &path)
        .await
        .map_err(|e| e.to_string())?;
    // 設定変更直後に一度同期しておく（debounce を待たせない）
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn clear_bibtex_sync_path(state: State<'_, AppState>) -> Result<(), String> {
    db::settings::delete_setting(&state.db, db::settings::BIBTEX_SYNC_PATH_KEY)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn pick_bibtex_sync_path(
    app: tauri::AppHandle,
    default_name: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let default_name = default_name.unwrap_or_else(|| "references.bib".to_string());
    tauri::async_runtime::spawn_blocking(move || {
        let path = app
            .dialog()
            .file()
            .set_file_name(&default_name)
            .add_filter("BibTeX", &["bib"])
            .blocking_save_file();
        let Some(p) = path else { return Ok(None) };
        let buf = p.into_path().map_err(|e| e.to_string())?;
        Ok(Some(buf.to_string_lossy().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn sync_bibtex_now(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    // debounce をバイパスして即時同期。設定未設定なら no-op（emit もしない）。
    perform_bibtex_sync(&state.db, &app).await;
    Ok(())
}

// ── metadata fetch ────────────────────────────────────────────────────────────

#[tauri::command]
async fn fetch_metadata_by_doi(doi: String) -> Result<EntryInput, String> {
    metadata::fetch_by_doi(&doi).await
}

#[tauri::command]
async fn fetch_metadata_by_arxiv(arxiv_id: String) -> Result<EntryInput, String> {
    metadata::fetch_by_arxiv(&arxiv_id).await
}

#[tauri::command]
async fn fetch_metadata_by_isbn(isbn: String) -> Result<EntryInput, String> {
    metadata::fetch_by_isbn(&isbn).await
}

#[tauri::command]
async fn find_duplicate_entry(
    state: State<'_, AppState>,
    doi: Option<String>,
    arxiv_id: Option<String>,
    isbn: Option<String>,
) -> Result<Option<i64>, String> {
    db::entries::find_duplicate_entry(&state.db, doi.as_deref(), arxiv_id.as_deref(), isbn.as_deref())
        .await
        .map_err(|e| e.to_string())
}

// ── app setup ─────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            let options = SqliteConnectOptions::new()
                .filename(data_dir.join("lumencite.db"))
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .foreign_keys(true);

            let pool = tauri::async_runtime::block_on(async {
                let pool = SqlitePool::connect_with(options).await?;
                sqlx::migrate!("./migrations").run(&pool).await?;
                Ok::<_, Box<dyn std::error::Error>>(pool)
            })?;

            // BibTeX 自動同期のコーディネーター。各ミューテーションが sync_tx.send() で
            // 通知し、受信タスクが debounce して書き出す。
            let (sync_tx, sync_rx) = unbounded_channel::<()>();
            let pool_for_task = pool.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_sync_task(pool_for_task, handle, sync_rx).await;
            });

            let mcp = Arc::new(mcp::McpManager::default());
            app.manage(AppState {
                db: pool.clone(),
                sync_tx,
                chat: Arc::new(ChatRuntime::default()),
                mcp: mcp.clone(),
            });

            // 設定済みの MCP サーバーをバックグラウンドで起動する。
            let mcp_pool = pool.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(Some(json)) =
                    db::settings::get_setting(&mcp_pool, db::settings::MCP_SERVERS_KEY).await
                {
                    for cfg in mcp::parse_servers_config(&json) {
                        if let Err(e) = mcp.start(cfg).await {
                            eprintln!("MCP server start failed: {e}");
                        }
                    }
                }
            });

            // メニューバー: アプリ名サブメニューに「Settings…」を追加（macOS / Windows / Linux）。
            // 標準的なショートカット ⌘+, (macOS) / Ctrl+, (他 OS) を割り当てる。
            install_app_menu(app.handle())?;

            // バックアップ: 起動時に 1 回 + 24h 間隔で実行。
            // エラーは log のみで握り潰し、本体ループは止めない。
            let backup_pool = pool.clone();
            let backup_dir = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                const RETENTION: usize = 14;
                if let Err(e) = backup::run_backup(&backup_pool, &backup_dir, RETENTION).await {
                    eprintln!("startup backup failed: {}", e);
                }
                let mut interval = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
                interval.tick().await; // 起動直後の重複 tick を消費
                loop {
                    interval.tick().await;
                    if let Err(e) = backup::run_backup(&backup_pool, &backup_dir, RETENTION).await {
                        eprintln!("scheduled backup failed: {}", e);
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_entries,
            get_entry,
            create_entry,
            update_entry,
            delete_entry,
            set_starred,
            trash_entry,
            restore_entry,
            get_sidebar_counts,
            bulk_trash,
            bulk_restore,
            bulk_purge,
            bulk_add_to_collection,
            bulk_add_tag,
            search_entries,
            get_tags,
            create_tag,
            delete_tag,
            add_tag_to_entry,
            remove_tag_from_entry,
            get_collections,
            create_collection,
            update_collection,
            delete_collection,
            add_entry_to_collection,
            remove_entry_from_collection,
            fetch_metadata_by_doi,
            fetch_metadata_by_arxiv,
            fetch_metadata_by_isbn,
            find_duplicate_entry,
            import_bibtex,
            pick_bibtex_file,
            import_bibtex_file,
            export_bibtex,
            save_bibtex,
            get_bibtex_sync_path,
            set_bibtex_sync_path,
            clear_bibtex_sync_path,
            pick_bibtex_sync_path,
            sync_bibtex_now,
            pick_pdf_file,
            add_attachment,
            delete_attachment,
            read_attachment_bytes,
            open_pdf_viewer,
            index_attachment,
            unindex_attachment,
            is_attachment_indexed,
            fulltext_search,
            get_highlights,
            create_highlight,
            update_highlight,
            delete_highlight,
            get_llm_settings,
            save_llm_settings,
            get_default_summary_prompt,
            set_api_key,
            delete_api_key,
            has_api_key,
            test_llm_connection,
            generate_summary,
            save_entry_summary,
            list_chat_sessions,
            create_chat_session,
            get_chat_session,
            update_chat_session_title,
            archive_chat_session,
            set_chat_session_scope,
            chat_send_message,
            approve_tool_call,
            cancel_chat_stream,
            generate_chat_title,
            list_mcp_servers,
            add_mcp_server,
            remove_mcp_server,
            run_backup_now,
            list_backups,
            open_backup_folder,
            export_database_json,
            export_database_markdown,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod chat_command_tests {
    use super::*;

    fn row(role: &str, content: &str, tool_calls: Option<&str>, tool_call_id: Option<&str>) -> db::chat::ChatMessage {
        db::chat::ChatMessage {
            id: 1,
            session_id: 1,
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: tool_calls.map(|s| s.to_string()),
            tool_call_id: tool_call_id.map(|s| s.to_string()),
            created_at: String::new(),
            position: 0,
        }
    }

    #[test]
    fn db_messages_to_chat_maps_roles_and_tool_calls() {
        let tc_json = r#"[{"call_id":"c1","tool_name":"list_tags","arguments":{}}]"#;
        let rows = vec![
            row("user", "hi", None, None),
            row("assistant", "let me look", Some(tc_json), None),
            row("tool", "no tags", None, Some("c1")),
        ];
        let msgs = db_messages_to_chat(&rows);
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, llm::Role::User);
        assert!(msgs[0].tool_calls.is_none());

        assert_eq!(msgs[1].role, llm::Role::Assistant);
        let calls = msgs[1].tool_calls.as_ref().expect("assistant tool_calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "list_tags");

        assert_eq!(msgs[2].role, llm::Role::Tool);
        assert_eq!(msgs[2].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn db_messages_ignores_blank_tool_calls() {
        // 空配列 JSON は None に潰す
        let rows = vec![row("assistant", "hello", Some("[]"), None)];
        let msgs = db_messages_to_chat(&rows);
        assert!(msgs[0].tool_calls.is_none());
    }

    #[test]
    fn clip_text_truncates_with_ellipsis() {
        assert_eq!(clip_text("short", 10), "short");
        assert_eq!(clip_text("abcdefghij", 5), "abcde…");
    }

    #[tokio::test]
    async fn runtime_resolve_approval_delivers_decision() {
        let rt = ChatRuntime::default();
        let rx = rt.register_approval(7, "call-1");
        rt.resolve_approval("call-1", true);
        assert_eq!(rx.await.unwrap(), true);
        // 解決済みなので二度目は何も起きない（パニックしない）
        rt.resolve_approval("call-1", false);
    }

    #[tokio::test]
    async fn runtime_cancel_denies_pending_and_sets_flag() {
        let rt = ChatRuntime::default();
        let flag = rt.begin(42);
        let rx = rt.register_approval(42, "call-x");
        rt.cancel(42);
        assert!(flag.load(Ordering::SeqCst), "cancel flag should be set");
        assert_eq!(rx.await.unwrap(), false, "pending approval should be denied");
        rt.finish(42);
    }

    #[tokio::test]
    async fn runtime_cancel_only_affects_its_own_session() {
        let rt = ChatRuntime::default();
        let _flag_a = rt.begin(1);
        let _flag_b = rt.begin(2);
        let rx_b = rt.register_approval(2, "b-call");
        rt.cancel(1); // 別セッションを中断
        // セッション 2 の承認待ちは残っている → 明示的に許可できる
        rt.resolve_approval("b-call", true);
        assert_eq!(rx_b.await.unwrap(), true);
    }
}
