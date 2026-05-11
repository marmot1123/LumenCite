mod bibtex;
mod db;
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
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

pub struct AppState {
    pub db: SqlitePool,
    /// BibTeX 自動同期リクエストを送る送信側。受信側のタスクが debounce して実行する。
    pub sync_tx: UnboundedSender<()>,
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
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let content = bibtex::export_bibtex(&state.db, entry_ids).await?;
    let default_name = default_name.unwrap_or_else(|| "lumencite.bib".to_string());

    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .set_file_name(&default_name)
            .add_filter("BibTeX", &["bib"])
            .blocking_save_file()
        else {
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

// ── bibtex sync settings ─────────────────────────────────────────────────────

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

// ── app setup ─────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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

            app.manage(AppState { db: pool, sync_tx });
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
            import_bibtex,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
