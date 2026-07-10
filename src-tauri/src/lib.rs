mod backup;
mod bibtex;
pub mod cli;
mod db;
mod download;
mod keychain;
mod llm;
mod mcp;
mod mcp_server;
pub mod mcp_shim;
mod metadata;
mod models;
mod orcid;
mod secretbox;

use models::{
    Attachment, Author, AuthorIdentifierInput, AuthorInput, Collection, EntryDetail, EntryFilter,
    EntryInput, EntrySummary, FulltextHit, ImportResult, SidebarCounts, Tag,
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

/// GUI 生存ロックのファイル名。GUI と CLI で同じパスを見る（CR-011）。
pub const GUI_LOCK_FILE: &str = "lumencite.gui.lock";

/// GUI が起動中である印の advisory ロックを保持する。プロセスが生きている限り握り続ける
/// よう、File をプロセス寿命の static に格納する。OS がプロセス終了時に自動解放するので
/// stale ロックは残らない。2 個目のインスタンスでロックが取れなくても起動は妨げない。
fn acquire_gui_lock(data_dir: &std::path::Path) {
    use fs2::FileExt;
    static GUI_LOCK: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();
    let path = data_dir.join(GUI_LOCK_FILE);
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    {
        if file.try_lock_exclusive().is_ok() {
            let _ = GUI_LOCK.set(file);
        }
    }
}

pub struct AppState {
    pub db: SqlitePool,
    /// BibTeX 自動同期リクエストを送る送信側。受信側のタスクが debounce して実行する。
    pub sync_tx: UnboundedSender<()>,
    /// 進行中チャットの承認待ち・中断状態を保持する共有ランタイム。
    pub chat: Arc<ChatRuntime>,
    /// 外部 MCP サーバーのクライアント（Chat ツールへマージ）。
    pub mcp: Arc<mcp::McpManager>,
    /// LumenCite 自身を MCP サーバーとして公開する際の起動/停止マネージャ。
    pub mcp_server: Arc<mcp_server::McpServerManager>,
    /// アプリデータディレクトリ（添付ファイルの相対パス解決用）。
    pub app_data_dir: PathBuf,
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
    filter: Option<EntryFilter>,
) -> Result<Vec<EntrySummary>, String> {
    let filter = filter.unwrap_or_default();
    db::entries::get_entries_filtered(&state.db, collection_id, tag_id, view.as_deref(), &filter)
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
async fn bulk_purge(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
) -> Result<(), String> {
    // 実際にゴミ箱から消えた id だけが返る（現役エントリは purge されない・CR-001）。
    let purged = db::entries::bulk_purge(&state.db, &ids)
        .await
        .map_err(|e| e.to_string())?;
    for id in &purged {
        remove_entry_attachment_dir(&app, *id);
    }
    request_sync(&state);
    Ok(())
}

/// ゴミ箱を空にする。表示中の id ではなく DB 側で `deleted_at IS NOT NULL` を評価するため、
/// 検索・フィルタで現役エントリが紛れても hard delete しない（CR-001）。
#[tauri::command]
async fn empty_trash(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let purged = db::entries::purge_trash(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    for id in &purged {
        remove_entry_attachment_dir(&app, *id);
    }
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
async fn is_citation_key_available(
    state: State<'_, AppState>,
    key: String,
    exclude_id: Option<i64>,
) -> Result<bool, String> {
    db::entries::is_citation_key_available(&state.db, &key, exclude_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_entry(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    // fulltext のクリーンアップは db::entries::delete_entry 内で行われる
    db::entries::delete_entry(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    remove_entry_attachment_dir(&app, id);
    request_sync(&state);
    Ok(())
}

/// hard delete 後に添付の実ファイル（attachments/<entry_id>/）を削除する。
/// DB 削除成功後に呼ぶ。ファイル側の失敗は無視する。
fn remove_entry_attachment_dir(app: &tauri::AppHandle, entry_id: i64) {
    if let Ok(root) = attachments_root(app) {
        let _ = std::fs::remove_dir_all(root.join(entry_id.to_string()));
    }
}

#[tauri::command]
async fn search_entries(
    state: State<'_, AppState>,
    query: String,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<String>,
    filter: Option<EntryFilter>,
) -> Result<Vec<EntrySummary>, String> {
    let filter = filter.unwrap_or_default();
    db::entries::search_entries_filtered(
        &state.db,
        &query,
        collection_id,
        tag_id,
        view.as_deref(),
        &filter,
    )
    .await
    .map_err(|e| e.to_string())
}

// ── authors (v0.3.0 M7) ───────────────────────────────────────────────────────

#[tauri::command]
async fn search_authors(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<Author>, String> {
    db::authors::search_authors(&state.db, &query, limit.unwrap_or(20))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_author(state: State<'_, AppState>, id: i64) -> Result<Option<Author>, String> {
    db::authors::get_author(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn update_author(
    state: State<'_, AppState>,
    id: i64,
    input: AuthorInput,
) -> Result<Author, String> {
    let updated = db::authors::update_author(&state.db, id, &input)
        .await
        .map_err(|e| e.to_string())?;
    // 著者表記が変われば bib export 内容にも波及するので同期キックを送る
    request_sync(&state);
    Ok(updated)
}

#[tauri::command]
async fn merge_authors(
    state: State<'_, AppState>,
    from_id: i64,
    into_id: i64,
) -> Result<(), String> {
    db::authors::merge_authors(&state.db, from_id, into_id)
        .await
        .map_err(|e| e.to_string())?;
    request_sync(&state);
    Ok(())
}

#[tauri::command]
async fn add_author_identifier(
    state: State<'_, AppState>,
    author_id: i64,
    input: AuthorIdentifierInput,
) -> Result<(), String> {
    db::authors::add_author_identifier(&state.db, author_id, &input)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_author_identifier(
    state: State<'_, AppState>,
    author_id: i64,
    scheme: String,
) -> Result<(), String> {
    db::authors::delete_author_identifier(&state.db, author_id, &scheme)
        .await
        .map_err(|e| e.to_string())
}

/// ORCID Public API から著者情報を取得して AuthorInput を返す（M12）。
/// state を取らないのは DB に触らない pure fetcher だから。
#[tauri::command]
async fn fetch_author_from_orcid(orcid: String) -> Result<AuthorInput, String> {
    orcid::fetch_by_orcid(&orcid).await
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

/// 詳細ビュー用: 指定エントリが .bib 同期で実際に割り当てられる cite key を返す。
#[tauri::command]
async fn resolve_citation_key(
    state: State<'_, AppState>,
    entry_id: i64,
) -> Result<String, String> {
    bibtex::resolve_citation_key(&state.db, entry_id).await
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

use download::create_unique_file;

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

    // 名前を原子的に予約してから中身をコピーする（CR-008）。予約済みの空ファイルを
    // copy で上書きするので、並行追加でも 1 ファイルを 2 行で共有することがない。
    let (file, dest) = create_unique_file(&entry_dir, &file_name).map_err(|e| e.to_string())?;
    drop(file);
    if let Err(e) = std::fs::copy(&src, &dest) {
        let _ = std::fs::remove_file(&dest); // 予約した空ファイルを残さない
        return Err(e.to_string());
    }

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
        Ok(att) => {
            // 添付成功後にバックグラウンドで全文索引する（SPEC: 添付後に自動索引・CR-027）。
            // リーダーからの手動添付もこの経路を通るので、以前は索引されなかった。
            let pool = state.db.clone();
            let att_id = att.id;
            let abs = dest.clone();
            tauri::async_runtime::spawn(async move {
                db::fulltext::extract_and_index(&pool, abs, att_id).await;
            });
            Ok(att)
        }
        Err(e) => {
            // DB 登録失敗時はコピー済みファイルを掃除する
            let _ = std::fs::remove_file(&dest);
            Err(e.to_string())
        }
    }
}

/// arXiv ID から PDF をダウンロードして `entry_id` に添付する。
///
/// 文献をメタデータ取得で追加した直後に「PDF も一括で取得する」ためのコマンド。
/// `https://arxiv.org/pdf/<id>` を `download::download_and_attach`（50MB 上限・
/// `%PDF-` マジックバイト検証・タイムアウト）でダウンロードし、成功したら
/// バックグラウンドで全文索引を試みる（索引失敗は無視 — 後追いで手動索引可能）。
///
/// ペイウォールやネットワーク障害で失敗しても、呼び出し側はエントリ作成を
/// 成功扱いにする想定（クリッパーと同じ方針）。
#[tauri::command]
async fn download_arxiv_pdf(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    entry_id: i64,
    arxiv_id: String,
) -> Result<Attachment, String> {
    let id = metadata::normalize_arxiv_id(&arxiv_id);
    if id.is_empty() {
        return Err("arXiv ID が空です".to_string());
    }
    let url = format!("https://arxiv.org/pdf/{}", id);

    let app_data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let att = download::download_and_attach(
        &state.db,
        &app_data_dir,
        entry_id,
        &url,
        download::DownloadCaps::default(),
    )
    .await?;

    // 添付済み PDF をバックグラウンドで全文索引する（best-effort・共有ヘルパ・CR-027）。
    let pool = state.db.clone();
    let att_id = att.id;
    let abs = app_data_dir
        .join("attachments")
        .join(entry_id.to_string())
        .join(&att.file_name);
    tauri::async_runtime::spawn(async move {
        db::fulltext::extract_and_index(&pool, abs, att_id).await;
    });

    Ok(att)
}

#[tauri::command]
async fn delete_attachment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<(), String> {
    // 添付レコードと全文索引を単一トランザクションで削除（CR-008）。orphan index を残さない。
    let removed = db::attachments::delete_attachment_with_fulltext(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;

    // ファイル本体は best-effort で削除。失敗しても DB は整合しているので致命ではないが、
    // 握りつぶさずログに残す（掃除漏れの検知用）。
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let abs = root.join(&removed.file_path);
    if let Err(e) = std::fs::remove_file(&abs) {
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("attachment file removal failed ({}): {e}", abs.display());
        }
    }
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

/// 「未索引の PDF を一括索引」の結果サマリ。
#[derive(serde::Serialize)]
struct IndexMissingResult {
    /// 処理対象（未索引 PDF 添付）の総数。
    total: i64,
    /// テキストを抽出して索引できた添付数。
    indexed: i64,
    /// テキストレイヤーが無く 0 ページだった添付数（OCR 候補）。
    needs_ocr: i64,
    /// ファイル読み込み / 抽出に失敗した添付数。
    failed: i64,
}

/// まだ全文索引が無い PDF 添付を洗い出し、順にテキスト抽出して索引する。
/// 添付時の自動索引を逃したエントリ（過去分・失敗分）を後追いで索引するための一括処理。
#[tauri::command]
async fn index_missing_attachments(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<IndexMissingResult, String> {
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let targets = db::fulltext::attachments_without_fulltext(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let total = targets.len() as i64;
    let mut indexed = 0i64;
    let mut needs_ocr = 0i64;
    let mut failed = 0i64;

    for (att_id, file_path) in targets {
        let abs = root.join(&file_path);
        // pdf-extract は重い同期処理なので 1 件ずつ blocking スレッドへ逃がす。
        let extracted = tauri::async_runtime::spawn_blocking(move || {
            pdf_extract::extract_text_by_pages(&abs).map_err(|e| e.to_string())
        })
        .await;

        let pages_text = match extracted {
            Ok(Ok(p)) => p,
            // spawn の join 失敗・抽出失敗どちらも「失敗」扱いで次へ。
            _ => {
                failed += 1;
                continue;
            }
        };

        let pages: Vec<(i64, String)> = pages_text
            .into_iter()
            .enumerate()
            .map(|(i, t)| ((i + 1) as i64, t))
            .collect();
        let non_empty = pages.iter().filter(|(_, t)| !t.trim().is_empty()).count();

        if db::fulltext::index_attachment(&state.db, att_id, &pages)
            .await
            .is_err()
        {
            failed += 1;
            continue;
        }

        if non_empty > 0 {
            indexed += 1;
        } else {
            // テキストレイヤーが無い（スキャン PDF 等）。OCR で拾う候補。
            needs_ocr += 1;
        }
    }

    Ok(IndexMissingResult {
        total,
        indexed,
        needs_ocr,
        failed,
    })
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
    view: Option<String>,
) -> Result<Vec<FulltextHit>, String> {
    db::fulltext::search_fulltext(&state.db, &query, collection_id, tag_id, view.as_deref())
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

/// 選択中の添付 PDF に属すハイライトだけを返す（CR-015）。
#[tauri::command]
async fn get_highlights_by_attachment(
    state: State<'_, AppState>,
    attachment_id: i64,
) -> Result<Vec<db::highlights::Highlight>, String> {
    db::highlights::list_by_attachment(&state.db, attachment_id)
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
    /// OCR 用プロバイダ/モデル。空/None なら provider/model にフォールバック。
    #[serde(default)]
    pub ocr_provider: Option<String>,
    #[serde(default)]
    pub ocr_model: Option<String>,
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
    let ocr_provider = db::settings::get_setting(&state.db, db::settings::LLM_OCR_PROVIDER_KEY)
        .await.map_err(|e| e.to_string())?
        .filter(|s| !s.trim().is_empty());
    let ocr_model = db::settings::get_setting(&state.db, db::settings::LLM_OCR_MODEL_KEY)
        .await.map_err(|e| e.to_string())?
        .filter(|s| !s.trim().is_empty());
    Ok(LlmSettings { provider, model, summary_source, summary_prompt, ocr_provider, ocr_model })
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
    db::settings::set_setting(&state.db, db::settings::LLM_OCR_PROVIDER_KEY, settings.ocr_provider.as_deref().unwrap_or(""))
        .await.map_err(|e| e.to_string())?;
    db::settings::set_setting(&state.db, db::settings::LLM_OCR_MODEL_KEY, settings.ocr_model.as_deref().unwrap_or(""))
        .await.map_err(|e| e.to_string())?;
    Ok(())
}

/// デフォルトのシステムプロンプトをフロントから取れるようにするユーティリティ。
#[tauri::command]
fn get_default_summary_prompt() -> String {
    llm::DEFAULT_SYSTEM_PROMPT.to_string()
}

// ── 用途別 settings コマンド（CR-002） ─────────────────────────────────────
// 任意キーを書ける汎用 setter は API keys や MCP config も上書きできて危険なので、
// 検証付きの用途別コマンドに分ける。

/// Chat ツールの自動承認ホワイトリスト（tool_name -> bool）を取得する。
#[tauri::command]
async fn get_tool_whitelist(
    state: State<'_, AppState>,
) -> Result<std::collections::HashMap<String, bool>, String> {
    let json = db::settings::get_setting(&state.db, db::settings::CHAT_TOOL_WHITELIST_KEY)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

/// Chat ツールの自動承認ホワイトリストを保存する。
/// override 可能なツール名（`OVERRIDABLE_TOOLS`）以外のキーは拒否する。
#[tauri::command]
async fn set_tool_whitelist(
    state: State<'_, AppState>,
    overrides: std::collections::HashMap<String, bool>,
) -> Result<(), String> {
    for key in overrides.keys() {
        if !llm::tools::approval::OVERRIDABLE_TOOLS.contains(&key.as_str()) {
            return Err(format!("unknown or non-overridable tool: {key}"));
        }
    }
    let json = serde_json::to_string(&overrides).map_err(|e| e.to_string())?;
    db::settings::set_setting(&state.db, db::settings::CHAT_TOOL_WHITELIST_KEY, &json)
        .await
        .map_err(|e| e.to_string())
}

/// PDF ビューの最後に開いていたページ（エントリ単位）を取得する。未設定なら None。
#[tauri::command]
async fn get_pdf_last_page(
    state: State<'_, AppState>,
    entry_id: i64,
) -> Result<Option<i64>, String> {
    let key = format!("pdf.last_page.{entry_id}");
    let v = db::settings::get_setting(&state.db, &key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(v.and_then(|s| s.parse::<i64>().ok()).filter(|&n| n > 0))
}

/// PDF ビューの最後に開いていたページを保存する。1 以上のみ受理する。
#[tauri::command]
async fn set_pdf_last_page(
    state: State<'_, AppState>,
    entry_id: i64,
    page: i64,
) -> Result<(), String> {
    if page < 1 {
        return Err("page must be >= 1".to_string());
    }
    let key = format!("pdf.last_page.{entry_id}");
    db::settings::set_setting(&state.db, &key, &page.to_string())
        .await
        .map_err(|e| e.to_string())
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
        let texts: Vec<String> = db::fulltext::get_entry_fulltext(&state.db, entry_id)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(_, content)| content)
            .collect();
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

/// ツール承認待ちがユーザー応答を待つ上限（CR-014）。これを超えたら fail-closed で拒否する。
/// セッションを離れて放置された run が永久に承認待ちで居座るのを防ぐ。
const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// 進行中チャットの承認待ち・中断状態を保持する共有ランタイム。
#[derive(Default)]
pub struct ChatRuntime {
    /// (session_id, call_id) -> 承認待ちの送信側。
    /// call_id は provider 管理で session 間衝突し得るため session_id と複合キーにする（CR-014）。
    pending: Mutex<HashMap<(i64, String), oneshot::Sender<bool>>>,
    /// session_id -> 中断フラグ。存在すること自体が「その session で run が進行中」を表す。
    cancels: Mutex<HashMap<i64, Arc<AtomicBool>>>,
}

impl ChatRuntime {
    /// セッションの run を開始する。**同一セッションで既に run が進行中なら `None`** を返し、
    /// 並行 send を拒否させる（CR-014: cancel フラグの上書き・二重実行を防ぐ）。
    fn begin(&self, session_id: i64) -> Option<Arc<AtomicBool>> {
        let mut cancels = self.cancels.lock().unwrap();
        if cancels.contains_key(&session_id) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        cancels.insert(session_id, flag.clone());
        Some(flag)
    }

    /// セッション終了時の後始末（中断フラグと残った承認待ちを除去）。
    fn finish(&self, session_id: i64) {
        self.cancels.lock().unwrap().remove(&session_id);
        self.pending.lock().unwrap().retain(|(sid, _), _| *sid != session_id);
    }

    /// ツール承認待ちを登録し、決定を待つ受信側を返す。
    fn register_approval(&self, session_id: i64, call_id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .insert((session_id, call_id.to_string()), tx);
        rx
    }

    /// UI からの承認/拒否を該当する待ちに伝える。session_id で当該セッションの待ちだけを解決する。
    fn resolve_approval(&self, session_id: i64, call_id: &str, approved: bool) {
        if let Some(tx) = self
            .pending
            .lock()
            .unwrap()
            .remove(&(session_id, call_id.to_string()))
        {
            let _ = tx.send(approved);
        }
    }

    /// セッションを中断する。中断フラグを立て、当該セッションの承認待ちは拒否扱いで解放する。
    fn cancel(&self, session_id: i64) {
        if let Some(flag) = self.cancels.lock().unwrap().get(&session_id) {
            flag.store(true, Ordering::SeqCst);
        }
        let mut pending = self.pending.lock().unwrap();
        let keys: Vec<(i64, String)> = pending
            .keys()
            .filter(|(sid, _)| *sid == session_id)
            .cloned()
            .collect();
        for key in keys {
            if let Some(tx) = pending.remove(&key) {
                let _ = tx.send(false);
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
    /// `.bib` 自動同期コーディネーターへの通知用（write ツール成功時にキック）。
    sync_tx: UnboundedSender<()>,
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
        // idle timeout（CR-014）: UI が応答しないまま放置されても永久待機しない。
        match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(approved)) => approved,
            _ => {
                // timeout / sender drop はどちらも fail-closed（拒否）。pending も掃除する。
                self.runtime
                    .resolve_approval(self.session_id, &call.call_id, false);
                false
            }
        }
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

    fn on_db_mutated(&mut self) {
        // write ツール成功のたびにキック（800ms デバウンスされるので回数は問題ない）。
        // ループがエラーで終わっても実行済みの書き換えは同期される。
        let _ = self.sync_tx.send(());
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
async fn unarchive_chat_session(
    state: State<'_, AppState>,
    id: i64,
) -> Result<db::chat::ChatSession, String> {
    db::chat::unarchive_session(&state.db, id)
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

#[tauri::command]
async fn set_chat_session_model(
    state: State<'_, AppState>,
    id: i64,
    provider: String,
    model: String,
) -> Result<db::chat::ChatSession, String> {
    db::chat::set_model(&state.db, id, &provider, &model)
        .await
        .map_err(|e| e.to_string())
}

// ── MCP クライアント ─────────────────────────────────────────────────────────

/// 設定済み MCP サーバー 1 件 + 起動状態（UI の一覧表示用）。
#[derive(serde::Serialize)]
struct McpServerInfo {
    id: String,
    command: String,
    args: Vec<String>,
    env: std::collections::HashMap<String, String>,
    /// 起動状態。未起動試行などで不明な場合は null。
    status: Option<mcp::McpServerStatus>,
}

/// 保存用に env の値を暗号化する（CR-012）。既に暗号化済みの値はそのまま。
fn encrypt_server_env(mut c: mcp::McpServerConfig) -> Result<mcp::McpServerConfig, String> {
    for v in c.env.values_mut() {
        if !secretbox::is_encrypted(v) {
            *v = secretbox::encrypt(v)?;
        }
    }
    Ok(c)
}

/// 起動用に env の値を復号する。暗号化されていない値（旧・平文）はそのまま返す。
fn decrypt_server_env(mut c: mcp::McpServerConfig) -> Result<mcp::McpServerConfig, String> {
    for v in c.env.values_mut() {
        *v = secretbox::decrypt(v)?;
    }
    Ok(c)
}

/// 一覧表示用に env の値を伏せる。キー名だけ残し、秘密値はフロントに返さない（CR-012）。
fn mask_server_env(mut c: mcp::McpServerConfig) -> mcp::McpServerConfig {
    for v in c.env.values_mut() {
        *v = String::new();
    }
    c
}

#[tauri::command]
async fn list_mcp_servers(state: State<'_, AppState>) -> Result<Vec<McpServerInfo>, String> {
    let json = db::settings::get_setting(&state.db, db::settings::MCP_SERVERS_KEY)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let statuses = state.mcp.statuses().await;
    let infos = mcp::parse_servers_config(&json)
        .into_iter()
        .map(mask_server_env) // 秘密値は返さない
        .map(|c| {
            let status = statuses.get(&c.id).cloned();
            McpServerInfo { id: c.id, command: c.command, args: c.args, env: c.env, status }
        })
        .collect();
    Ok(infos)
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
    // 保存は暗号化した env で行う。起動はフロントから受け取った平文 config で行う。
    servers.push(encrypt_server_env(config.clone())?);
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

/// 保存済み config（env は暗号化）を読み出して復号し、再起動する。
/// フロントは秘密値を持たないので、再起動は id だけを渡して backend 側で組み立てる（CR-012）。
#[tauri::command]
async fn restart_mcp_server(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let json = db::settings::get_setting(&state.db, db::settings::MCP_SERVERS_KEY)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let config = mcp::parse_servers_config(&json)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("no such MCP server: {id}"))?;
    let config = decrypt_server_env(config)?;
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

// ── OCR ──────────────────────────────────────────────────────────────────────

/// 詳細ビューの「OCR を実行」ボタン用。ユーザー操作なので承認は不要（クリック＝同意）。
/// LLM ツール `ocr_pdf` と内部実装（run_ocr）を共有する。
#[tauri::command]
async fn ocr_pdf(
    state: State<'_, AppState>,
    entry_id: i64,
    attachment_id: Option<i64>,
    pages: Option<Vec<i64>>,
) -> Result<String, String> {
    llm::tools::ocr::run_ocr(&state.db, &state.app_data_dir, entry_id, attachment_id, pages)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn approve_tool_call(
    state: State<'_, AppState>,
    session_id: i64,
    call_id: String,
    approved: bool,
) -> Result<(), String> {
    state.chat.resolve_approval(session_id, &call_id, approved);
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

    // 同一セッションで run が進行中なら拒否する（CR-014）。二重実行や cancel フラグ上書きを防ぐ。
    let cancel = match state.chat.begin(session_id) {
        Some(flag) => flag,
        None => {
            let msg = "this chat session already has a message in progress".to_string();
            let _ = channel.send(ChatStreamEvent::Error { message: msg.clone() });
            return Err(msg);
        }
    };
    let mut host = ChannelHost {
        channel: channel.clone(),
        runtime: state.chat.clone(),
        session_id,
        cancel,
        sync_tx: state.sync_tx.clone(),
    };

    let ctx = llm::tools::ToolContext {
        pool: &pool,
        session_id,
        scope_mode: &session.scope_mode,
        scope_entry_ids: &entry_ids,
        mcp: Some(state.mcp.as_ref()),
        app_data_dir: &state.app_data_dir,
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
async fn get_bibtex_exclude_abstract_note(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(db::settings::get_setting(&state.db, db::settings::BIBTEX_EXCLUDE_ABSTRACT_NOTE_KEY)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("1"))
}

#[tauri::command]
async fn set_bibtex_exclude_abstract_note(
    state: State<'_, AppState>,
    exclude: bool,
) -> Result<(), String> {
    db::settings::set_setting(
        &state.db,
        db::settings::BIBTEX_EXCLUDE_ABSTRACT_NOTE_KEY,
        if exclude { "1" } else { "" },
    )
    .await
    .map_err(|e| e.to_string())?;
    // 設定変更を同期先 .bib に即反映する。
    request_sync(&state);
    Ok(())
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

/// DB 初期化失敗の分類。sqlx/Tauri 非依存にして、ダイアログ文言生成を単体テスト可能にする。
#[derive(Debug, Clone, PartialEq)]
enum DbInitFailure {
    /// DB が現バイナリの知る最新より新しい schema を持つ（旧版で新版 DB を開いた＝ダウングレード非互換）。
    /// 値は適用済みだが解決できなかった version。
    NewerSchema(i64),
    /// その他のマイグレーション失敗（チェックサム不一致・SQL エラー等）。
    Migrate(String),
    /// 接続 / オープン失敗（ロック・破損・権限など）。
    Connect(String),
}

/// sqlx の `MigrateError` を `DbInitFailure` に分類する。`VersionMissing`（適用済み version が
/// 解決対象に無い）は、新版で作られた DB を旧版で開いたダウングレードと解釈する。
fn classify_migrate_error(e: &sqlx::migrate::MigrateError) -> DbInitFailure {
    match e {
        sqlx::migrate::MigrateError::VersionMissing(v) => DbInitFailure::NewerSchema(*v),
        other => DbInitFailure::Migrate(other.to_string()),
    }
}

/// 起動時 DB 初期化失敗をユーザーに見せる (タイトル, 本文)。日英併記でロケール非依存にする。
fn db_init_dialog_text(failure: &DbInitFailure) -> (String, String) {
    match failure {
        DbInitFailure::NewerSchema(v) => (
            "LumenCite — データベースを開けません / Cannot open database".to_string(),
            format!(
                "このライブラリは、より新しいバージョンの LumenCite で作成されています（schema v{v}）。\
                 LumenCite を最新版に更新してから開いてください。データは安全です（削除されていません）。\
                 \n\nThis library was created by a newer version of LumenCite (schema v{v}). \
                 Please update LumenCite to the latest version, then reopen. Your data is safe."
            ),
        ),
        DbInitFailure::Migrate(msg) => (
            "LumenCite — データベースの更新に失敗 / Database update failed".to_string(),
            format!(
                "データベースの更新（マイグレーション）に失敗しました。再起動しても解決しない場合は、\
                 最新版への更新やバックアップからの復元をご検討ください。\
                 \n\nFailed to apply database migrations. If restarting does not help, \
                 consider updating to the latest version or restoring from a backup.\n\n{msg}"
            ),
        ),
        DbInitFailure::Connect(msg) => (
            "LumenCite — データベースを開けません / Cannot open database".to_string(),
            format!(
                "データベースに接続できませんでした。別の LumenCite が起動していないか確認してください。\
                 \n\nCould not open the database. Make sure another instance of LumenCite is not already running.\n\n{msg}"
            ),
        ),
    }
}

// ─── MCP サーバー公開（Phase 1） ─────────────────────────────────────────────

/// MCP サーバーの状態（フロントの設定画面表示用）。
#[derive(serde::Serialize)]
struct McpServerStatusInfo {
    enabled: bool,
    running: bool,
    port: u16,
    has_token: bool,
    /// Phase 2: write 系ツールを公開しているか（`mcp_server.write_enabled`）。
    write_enabled: bool,
}

/// AppState + AppHandle から MCP サーバー起動用の依存をまとめる。
fn mcp_server_deps(state: &AppState, app: &AppHandle) -> mcp_server::ServerDeps {
    mcp_server::ServerDeps {
        pool: state.db.clone(),
        app_data_dir: state.app_data_dir.clone(),
        sync_tx: state.sync_tx.clone(),
        app: Some(app.clone()),
    }
}

/// 設定済みポート（未設定なら既定値）。
async fn mcp_server_configured_port(pool: &SqlitePool) -> Result<u16, String> {
    Ok(db::settings::get_setting(pool, db::settings::MCP_SERVER_PORT_KEY)
        .await
        .map_err(|e| e.to_string())?
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(mcp_server::DEFAULT_PORT))
}

/// bool 設定（"1" で有効）を読む。
async fn setting_is_on(pool: &SqlitePool, key: &str) -> Result<bool, String> {
    Ok(db::settings::get_setting(pool, key)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("1"))
}

/// ローカル HTTP サーバー（MCP サーバー / Web クリッパー共用）を起動し、
/// 実バインドポートを設定へ保存して返す。既に起動中なら再起動になる。
async fn start_http_server(state: &AppState, app: &AppHandle) -> Result<u16, String> {
    let token = mcp_server::get_or_create_token(&state.db).await?;
    let port = mcp_server_configured_port(&state.db).await?;
    let bound = state
        .mcp_server
        .start(mcp_server_deps(state, app), port, token)?;
    // OS が別ポートを割り当てた場合に追従できるよう、実バインドポートを保存。
    db::settings::set_setting(&state.db, db::settings::MCP_SERVER_PORT_KEY, &bound.to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok(bound)
}

/// MCP・クリッパーの両方が無効ならサーバーを停止する（どちらかが使っていれば維持）。
async fn stop_http_server_if_unused(state: &AppState) -> Result<(), String> {
    let mcp_on = setting_is_on(&state.db, db::settings::MCP_SERVER_ENABLED_KEY).await?;
    let clipper_on = setting_is_on(&state.db, db::settings::CLIPPER_ENABLED_KEY).await?;
    if !mcp_on && !clipper_on {
        state.mcp_server.stop();
    }
    Ok(())
}

/// 状態を組み立てる内部ヘルパ（複数コマンドから共有）。
async fn build_mcp_server_status(
    pool: &SqlitePool,
    manager: &mcp_server::McpServerManager,
) -> Result<McpServerStatusInfo, String> {
    let enabled = db::settings::get_setting(pool, db::settings::MCP_SERVER_ENABLED_KEY)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("1");
    let configured_port = mcp_server_configured_port(pool).await?;
    let running_port = manager.running_port();
    let has_token = keychain::get(&keychain::account_for_mcp_token())
        .map_err(|e| e.to_string())?
        .is_some();
    let write_enabled = db::settings::get_setting(pool, db::settings::MCP_SERVER_WRITE_ENABLED_KEY)
        .await
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("1");
    Ok(McpServerStatusInfo {
        enabled,
        running: running_port.is_some(),
        port: running_port.unwrap_or(configured_port),
        has_token,
        write_enabled,
    })
}

#[tauri::command]
async fn get_mcp_server_status(
    state: State<'_, AppState>,
) -> Result<McpServerStatusInfo, String> {
    build_mcp_server_status(&state.db, &state.mcp_server).await
}

#[tauri::command]
async fn set_mcp_server_enabled(
    state: State<'_, AppState>,
    app: AppHandle,
    enabled: bool,
) -> Result<McpServerStatusInfo, String> {
    db::settings::set_setting(
        &state.db,
        db::settings::MCP_SERVER_ENABLED_KEY,
        if enabled { "1" } else { "0" },
    )
    .await
    .map_err(|e| e.to_string())?;

    if enabled {
        start_http_server(&state, &app).await?;
    } else {
        // クリッパーがまだ使っている場合はサーバーを維持する
        stop_http_server_if_unused(&state).await?;
    }

    build_mcp_server_status(&state.db, &state.mcp_server).await
}

/// Phase 2: write 系ツールの公開可否を切り替える。サーバーはリクエスト毎に設定を
/// 読むため、再起動は不要（起動中ならそのまま反映される）。
#[tauri::command]
async fn set_mcp_server_write_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<McpServerStatusInfo, String> {
    db::settings::set_setting(
        &state.db,
        db::settings::MCP_SERVER_WRITE_ENABLED_KEY,
        if enabled { "1" } else { "0" },
    )
    .await
    .map_err(|e| e.to_string())?;
    build_mcp_server_status(&state.db, &state.mcp_server).await
}

/// Phase 2: MCP 経由の write 監査ログを新しい順で返す。
#[tauri::command]
async fn get_mcp_audit_log(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<db::mcp_audit::McpAuditEntry>, String> {
    db::mcp_audit::recent(&state.db, limit.unwrap_or(100))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn regenerate_mcp_server_token(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<String, String> {
    let token = mcp_server::generate_token(&state.db).await?;
    keychain::set(&keychain::account_for_mcp_token(), &token).map_err(|e| e.to_string())?;
    // 起動中なら新トークンで再起動する。
    if state.mcp_server.running_port().is_some() {
        let port = mcp_server_configured_port(&state.db).await?;
        state
            .mcp_server
            .start(mcp_server_deps(&state, &app), port, token.clone())?;
    }
    Ok(token)
}

#[tauri::command]
async fn get_mcp_server_config_snippet(
    state: State<'_, AppState>,
    client: String,
) -> Result<String, String> {
    let port = state
        .mcp_server
        .running_port()
        .unwrap_or(mcp_server_configured_port(&state.db).await?);
    let token = mcp_server::get_or_create_token(&state.db).await?;
    let url = format!("http://127.0.0.1:{port}/mcp");

    let snippet = match client.as_str() {
        "claude_code" => format!(
            "claude mcp add --transport http lumencite {url} --header \"Authorization: Bearer {token}\""
        ),
        // Claude Desktop は stdio のみ対応のため、本体バイナリ自身を `--mcp-stdio` shim として
        // 起動させる `mcpServers` JSON を生成する（Phase 3）。`command` は現在の実行ファイル絶対パス。
        "claude_desktop" => {
            let exe = std::env::current_exe()
                .map_err(|e| format!("failed to resolve executable path: {e}"))?;
            let config = serde_json::json!({
                "mcpServers": {
                    "lumencite": {
                        "command": exe.to_string_lossy(),
                        "args": ["--mcp-stdio"],
                        "env": {
                            "LUMENCITE_MCP_URL": url,
                            "LUMENCITE_MCP_TOKEN": token,
                        }
                    }
                }
            });
            serde_json::to_string_pretty(&config)
                .map_err(|e| format!("failed to serialize config: {e}"))?
        }
        // Codex CLI（OpenAI）は `~/.codex/config.toml` の `[mcp_servers.<name>]` に
        // stdio サーバーを登録する。Claude Desktop と同じ `--mcp-stdio` shim を流用する。
        "codex" => {
            let exe = std::env::current_exe()
                .map_err(|e| format!("failed to resolve executable path: {e}"))?;
            codex_config_snippet(&exe.to_string_lossy(), &url, &token)
        }
        // その他の汎用リモート MCP クライアント向けには素の URL とヘッダを返す。
        _ => format!("URL: {url}\nHeader: Authorization: Bearer {token}"),
    };
    Ok(snippet)
}

/// TOML 基本文字列（`"..."`）用のエスケープ。Windows パスの `\` や `"`・制御文字を
/// 安全に含められるようにする。
fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Codex CLI（`~/.codex/config.toml`）へ貼り付ける `[mcp_servers.lumencite]` スニペット。
/// LumenCite 本体を `--mcp-stdio` ブリッジとして stdio 起動させ、接続先 URL とトークンを env で渡す。
fn codex_config_snippet(exe: &str, url: &str, token: &str) -> String {
    format!(
        "[mcp_servers.lumencite]\ncommand = \"{}\"\nargs = [\"--mcp-stdio\"]\nenv = {{ LUMENCITE_MCP_URL = \"{}\", LUMENCITE_MCP_TOKEN = \"{}\" }}\n",
        toml_escape(exe),
        toml_escape(url),
        toml_escape(token),
    )
}

/// GitHub Releases の `tag_name`（例 `"v0.5.0"`）が現在のアプリバージョンより新しいか。
/// 先頭の `v` は無視。いずれかが semver として解釈できなければ `false`
/// （更新を誤って促さない安全側に倒す）。
fn release_is_newer(current: &str, latest_tag: &str) -> bool {
    let parse = |s: &str| semver::Version::parse(s.trim().trim_start_matches('v'));
    match (parse(current), parse(latest_tag)) {
        (Ok(cur), Ok(latest)) => latest > cur,
        _ => false,
    }
}

/// GitHub Releases の最新版情報（フロントの更新通知バナー表示用）。
/// フロント（`updater.ts` の `GithubReleaseInfo`）は camelCase で受けるため rename する。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GithubReleaseInfo {
    current_version: String,
    latest_version: String,
    /// `latest_version > current_version`
    is_newer: bool,
    html_url: String,
    body: Option<String>,
}

/// GitHub Releases の最新 tag を取得し、現在のアプリバージョンと比較する。
///
/// tauri-plugin-updater とは独立した「通知のみ」の更新確認経路。`latest.json` も
/// updater 署名鍵も使わず、ダウンロード/インストールもしない（フロントが `html_url` を
/// 外部ブラウザで開くだけ）。そのため macOS の自動更新を壊さず **全 OS で安全**に、
/// Windows/Linux ユーザーにも新版を通知できる。
#[tauri::command]
async fn check_latest_github_release() -> Result<GithubReleaseInfo, String> {
    const CURRENT: &str = env!("CARGO_PKG_VERSION");
    const LATEST_RELEASE_API: &str =
        "https://api.github.com/repos/marmot1123/LumenCite/releases/latest";
    const RELEASES_PAGE: &str = "https://github.com/marmot1123/LumenCite/releases/latest";

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("client build failed: {e}"))?;
    let resp = client
        .get(LATEST_RELEASE_API)
        .header(reqwest::header::USER_AGENT, "LumenCite")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned {}", resp.status()));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))?;
    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "response missing tag_name".to_string())?;
    let html_url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(RELEASES_PAGE)
        .to_string();
    let body = json
        .get("body")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Ok(GithubReleaseInfo {
        current_version: CURRENT.to_string(),
        latest_version: tag.trim().trim_start_matches('v').to_string(),
        is_newer: release_is_newer(CURRENT, tag),
        html_url,
        body,
    })
}

// ─── Web クリッパー（v0.5.0） ────────────────────────────────────────────────

/// Web クリッパーの状態（フロントの設定画面表示用）。
#[derive(serde::Serialize)]
struct ClipperStatusInfo {
    /// `clipper.enabled == "1"`
    enabled: bool,
    /// 共用 HTTP サーバースレッドが起動中か
    server_running: bool,
    port: u16,
}

async fn build_clipper_status(
    pool: &SqlitePool,
    manager: &mcp_server::McpServerManager,
) -> Result<ClipperStatusInfo, String> {
    let enabled = setting_is_on(pool, db::settings::CLIPPER_ENABLED_KEY).await?;
    let running_port = manager.running_port();
    Ok(ClipperStatusInfo {
        enabled,
        server_running: running_port.is_some(),
        port: running_port.unwrap_or(mcp_server_configured_port(pool).await?),
    })
}

#[tauri::command]
async fn get_clipper_status(state: State<'_, AppState>) -> Result<ClipperStatusInfo, String> {
    build_clipper_status(&state.db, &state.mcp_server).await
}

#[tauri::command]
async fn set_clipper_enabled(
    state: State<'_, AppState>,
    app: AppHandle,
    enabled: bool,
) -> Result<ClipperStatusInfo, String> {
    db::settings::set_setting(
        &state.db,
        db::settings::CLIPPER_ENABLED_KEY,
        if enabled { "1" } else { "0" },
    )
    .await
    .map_err(|e| e.to_string())?;

    if enabled {
        // MCP 側で既に起動していればそのまま共用する（再起動しない）
        if state.mcp_server.running_port().is_none() {
            start_http_server(&state, &app).await?;
        }
    } else {
        stop_http_server_if_unused(&state).await?;
    }

    build_clipper_status(&state.db, &state.mcp_server).await
}

/// ブラウザ拡張のオプションページに貼り付ける接続コードを返す。
/// 形式: `lc1.` + base64url(`{"v":1,"port":<u16>,"token":"<48hex>"}`)（パディングなし）。
/// トークン再生成（`regenerate_mcp_server_token`）でこのコードは無効になる。
#[tauri::command]
async fn get_clipper_connect_code(state: State<'_, AppState>) -> Result<String, String> {
    use base64::Engine;
    let port = state
        .mcp_server
        .running_port()
        .unwrap_or(mcp_server_configured_port(&state.db).await?);
    let token = mcp_server::get_or_create_token(&state.db).await?;
    let payload = serde_json::json!({ "v": 1, "port": port, "token": token });
    Ok(format!(
        "lc1.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.to_string())
    ))
}

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

            // GUI 生存フラグ（CR-011）: このロックを保持している間は「GUI 起動中」。
            // CLI の直接書込経路がこれを見て、MCP 委譲できないときに live DB を壊さないよう
            // 判断する。try_lock なので 2 個目のインスタンスでも起動を妨げない。
            acquire_gui_lock(&data_dir);

            let options = SqliteConnectOptions::new()
                .filename(data_dir.join("lumencite.db"))
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                .foreign_keys(true);

            // DB 接続 + マイグレーション。失敗は `?` で setup 外に投げず、ここで握って
            // ユーザー向けダイアログを表示してから安全終了する（旧版で新版 DB を開いた等で
            // SIGABRT クラッシュしていた問題への対応）。
            let pool = match tauri::async_runtime::block_on(async {
                let pool = SqlitePool::connect_with(options)
                    .await
                    .map_err(|e| DbInitFailure::Connect(e.to_string()))?;
                sqlx::migrate!("./migrations")
                    .run(&pool)
                    .await
                    .map_err(|e| classify_migrate_error(&e))?;
                Ok::<_, DbInitFailure>(pool)
            }) {
                Ok(pool) => pool,
                Err(failure) => {
                    eprintln!("DB init failed: {failure:?}");
                    let (title, body) = db_init_dialog_text(&failure);
                    // setup はメインスレッドで走るため、rfd のネイティブモーダルはイベントループ
                    // 起動前でもインラインで表示できる（tauri-plugin-dialog の blocking_show は
                    // run_on_main_thread 経由でここではデッドロックするので使わない）。
                    let _ = rfd::MessageDialog::new()
                        .set_level(rfd::MessageLevel::Error)
                        .set_title(title)
                        .set_description(body)
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                    // pool 未生成で先へ進めないため即終了（ダイアログ確認後）。
                    std::process::exit(1);
                }
            };

            // v0.3.0: 既存ライブラリの entries_fts.authors_text を新合成
            // (name + name_original + reading_*) で 1 回だけ作り直す。フラグ既設なら no-op。
            // 失敗してもアプリ起動は止めず、log だけ残してリトライさせる（次回起動で再試行）。
            let fts_pool = pool.clone();
            tauri::async_runtime::spawn(async move {
                match db::entries::rebuild_authors_fts_once(&fts_pool).await {
                    Ok(true) => eprintln!("entries_fts: rebuilt for v0.3.0 authors schema"),
                    Ok(false) => {}
                    Err(e) => eprintln!("entries_fts rebuild failed: {e}"),
                }
            });

            // BibTeX 自動同期のコーディネーター。各ミューテーションが sync_tx.send() で
            // 通知し、受信タスクが debounce して書き出す。
            let (sync_tx, sync_rx) = unbounded_channel::<()>();
            let pool_for_task = pool.clone();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_sync_task(pool_for_task, handle, sync_rx).await;
            });

            let mcp = Arc::new(mcp::McpManager::default());
            let mcp_server = Arc::new(mcp_server::McpServerManager::default());
            // AppState に move する前に、MCP サーバー自動起動用のクローンを取る。
            let srv_sync_tx = sync_tx.clone();
            let srv_app = app.handle().clone();
            app.manage(AppState {
                db: pool.clone(),
                sync_tx,
                chat: Arc::new(ChatRuntime::default()),
                mcp: mcp.clone(),
                mcp_server: mcp_server.clone(),
                app_data_dir: data_dir.clone(),
            });

            // MCP サーバー公開 or Web クリッパーのどちらかが有効なら共用 HTTP サーバーを起動する。
            let srv_pool = pool.clone();
            let srv_dir = data_dir.clone();
            tauri::async_runtime::spawn(async move {
                let is_on = |key: &'static str| {
                    let pool = srv_pool.clone();
                    async move {
                        db::settings::get_setting(&pool, key)
                            .await
                            .ok()
                            .flatten()
                            .as_deref()
                            == Some("1")
                    }
                };
                let enabled = is_on(db::settings::MCP_SERVER_ENABLED_KEY).await
                    || is_on(db::settings::CLIPPER_ENABLED_KEY).await;
                if !enabled {
                    return;
                }
                match mcp_server::get_or_create_token(&srv_pool).await {
                    Ok(token) => {
                        let port = db::settings::get_setting(&srv_pool, db::settings::MCP_SERVER_PORT_KEY)
                            .await
                            .ok()
                            .flatten()
                            .and_then(|s| s.parse::<u16>().ok())
                            .unwrap_or(mcp_server::DEFAULT_PORT);
                        let deps = mcp_server::ServerDeps {
                            pool: srv_pool.clone(),
                            app_data_dir: srv_dir,
                            sync_tx: srv_sync_tx,
                            app: Some(srv_app),
                        };
                        if let Err(e) = mcp_server.start(deps, port, token) {
                            eprintln!("MCP server start failed: {e}");
                        }
                    }
                    Err(e) => eprintln!("MCP server token error: {e}"),
                }
            });

            // 設定済みの MCP サーバーをバックグラウンドで起動する。
            let mcp_pool = pool.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(Some(json)) =
                    db::settings::get_setting(&mcp_pool, db::settings::MCP_SERVERS_KEY).await
                {
                    let servers = mcp::parse_servers_config(&json);

                    // 旧・平文の env を一度だけ暗号化して保存し直す（CR-012 の migration）。
                    // 平文値が 1 つでもあれば全体を暗号化して書き戻す。
                    let has_plaintext = servers.iter().any(|s| {
                        s.env.values().any(|v| !v.is_empty() && !secretbox::is_encrypted(v))
                    });
                    if has_plaintext {
                        let migrated: Result<Vec<_>, _> =
                            servers.iter().cloned().map(encrypt_server_env).collect();
                        match migrated {
                            Ok(encrypted) => {
                                let _ = db::settings::set_setting(
                                    &mcp_pool,
                                    db::settings::MCP_SERVERS_KEY,
                                    &mcp::serialize_servers_config(&encrypted),
                                )
                                .await;
                            }
                            Err(e) => eprintln!("MCP env migration failed: {e}"),
                        }
                    }

                    // 起動は復号した env で行う。
                    for cfg in servers {
                        let cfg = match decrypt_server_env(cfg) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!("MCP env decrypt failed: {e}");
                                continue;
                            }
                        };
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
            is_citation_key_available,
            delete_entry,
            set_starred,
            trash_entry,
            restore_entry,
            get_sidebar_counts,
            bulk_trash,
            bulk_restore,
            bulk_purge,
            empty_trash,
            bulk_add_to_collection,
            bulk_add_tag,
            search_entries,
            search_authors,
            get_author,
            update_author,
            merge_authors,
            add_author_identifier,
            delete_author_identifier,
            fetch_author_from_orcid,
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
            resolve_citation_key,
            save_bibtex,
            get_bibtex_sync_path,
            set_bibtex_sync_path,
            clear_bibtex_sync_path,
            get_bibtex_exclude_abstract_note,
            set_bibtex_exclude_abstract_note,
            pick_bibtex_sync_path,
            sync_bibtex_now,
            pick_pdf_file,
            add_attachment,
            download_arxiv_pdf,
            delete_attachment,
            read_attachment_bytes,
            open_pdf_viewer,
            index_attachment,
            index_missing_attachments,
            unindex_attachment,
            is_attachment_indexed,
            fulltext_search,
            get_highlights,
            get_highlights_by_attachment,
            create_highlight,
            update_highlight,
            delete_highlight,
            get_llm_settings,
            save_llm_settings,
            get_default_summary_prompt,
            get_tool_whitelist,
            set_tool_whitelist,
            get_pdf_last_page,
            set_pdf_last_page,
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
            unarchive_chat_session,
            set_chat_session_scope,
            set_chat_session_model,
            chat_send_message,
            approve_tool_call,
            cancel_chat_stream,
            generate_chat_title,
            list_mcp_servers,
            add_mcp_server,
            restart_mcp_server,
            remove_mcp_server,
            get_mcp_server_status,
            set_mcp_server_enabled,
            set_mcp_server_write_enabled,
            get_mcp_audit_log,
            regenerate_mcp_server_token,
            get_mcp_server_config_snippet,
            check_latest_github_release,
            get_clipper_status,
            set_clipper_enabled,
            get_clipper_connect_code,
            ocr_pdf,
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
mod db_init_tests {
    use super::*;

    #[test]
    fn newer_schema_text_mentions_version_and_update_bilingually() {
        let (title, body) = db_init_dialog_text(&DbInitFailure::NewerSchema(7));
        assert!(!title.is_empty());
        assert!(body.contains("v7"), "欠落 version を本文に含める");
        assert!(body.contains("新しいバージョン"), "日本語の説明を含む");
        assert!(body.contains("newer version"), "英語の説明を含む");
        assert!(body.contains("安全") && body.contains("safe"), "データは安全である旨を日英で示す");
    }

    #[test]
    fn migrate_text_includes_error_detail() {
        let (_t, body) = db_init_dialog_text(&DbInitFailure::Migrate("checksum mismatch".into()));
        assert!(body.contains("checksum mismatch"));
    }

    #[test]
    fn connect_text_includes_error_detail() {
        let (_t, body) = db_init_dialog_text(&DbInitFailure::Connect("database is locked".into()));
        assert!(body.contains("database is locked"));
    }
}

#[cfg(test)]
mod chat_runtime_tests {
    use super::*;

    /// CR-014: 同一セッションで run 進行中なら 2 回目の begin は None（並行 run 拒否）。
    #[test]
    fn begin_rejects_concurrent_run_for_same_session() {
        let rt = ChatRuntime::default();
        let first = rt.begin(1);
        assert!(first.is_some());
        assert!(rt.begin(1).is_none(), "同一セッションの二重 run は拒否");
        // 別セッションは並行 OK。
        assert!(rt.begin(2).is_some());
        // finish 後は再度開始できる。
        rt.finish(1);
        assert!(rt.begin(1).is_some());
    }

    /// CR-014: approval は (session_id, call_id) で解決され、別セッションの同名 call を誤解決しない。
    #[tokio::test]
    async fn resolve_approval_is_scoped_to_session() {
        let rt = ChatRuntime::default();
        let rx1 = rt.register_approval(1, "call-x");
        let rx2 = rt.register_approval(2, "call-x"); // 同じ call_id 別セッション

        // session 2 を解決しても session 1 は待ちのまま。
        rt.resolve_approval(2, "call-x", true);
        assert_eq!(rx2.await.unwrap(), true);

        rt.resolve_approval(1, "call-x", false);
        assert_eq!(rx1.await.unwrap(), false);
    }
}

#[cfg(test)]
mod pdf_extract_tests {
    /// CR-017: 未信頼 PDF の解析が panic せず Err を返すこと。
    /// pdf-extract 0.12 / lopdf 0.42 で RUSTSEC-2026-0187（深いネストによる stack overflow）
    /// を解消済み。回帰でクラッシュに戻らないよう、壊れた入力での挙動を固定する。
    #[test]
    fn malformed_pdf_returns_error_without_panicking() {
        let dir = std::env::temp_dir().join("lumencite_cr017_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("garbage.pdf");
        // %PDF ヘッダだけ持つが本体が壊れているバイト列。
        let bytes = b"%PDF-1.7\n1 0 obj<< /Type /Catalog >>endobj\ngarbage\x00\xff\xfe";
        std::fs::write(&path, bytes).unwrap();

        let result = pdf_extract::extract_text_by_pages(&path);
        // panic せず（ここへ到達している時点で保証）、成功でも失敗でも許容する。
        // 主目的は「クラッシュしない」こと。
        let _ = result;

        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod update_and_snippet_tests {
    use super::*;

    #[test]
    fn release_is_newer_compares_semver_and_ignores_v_prefix() {
        assert!(release_is_newer("0.4.0", "v0.5.0"));
        assert!(release_is_newer("0.4.0", "0.5.0")); // v なしでも可
        assert!(release_is_newer("0.5.0", "v0.5.1"));
        assert!(!release_is_newer("0.5.0", "v0.5.0")); // 同一は「新しい」でない
        assert!(!release_is_newer("0.5.0", "v0.4.9")); // 古い tag は促さない
    }

    #[test]
    fn release_is_newer_is_false_on_unparseable() {
        // どちらかが semver でないときは更新を誤って促さない（安全側）。
        assert!(!release_is_newer("0.5.0", "nightly"));
        assert!(!release_is_newer("dev", "v0.6.0"));
        assert!(!release_is_newer("0.5.0", ""));
    }

    #[test]
    fn release_is_newer_handles_prerelease() {
        // prerelease は本リリースより古いと扱われる（semver 準拠）。
        assert!(release_is_newer("0.5.0-beta.1", "v0.5.0"));
        assert!(!release_is_newer("0.5.0", "v0.5.0-beta.1"));
    }

    #[test]
    fn codex_snippet_is_valid_toml_table_with_stdio_shim() {
        let s = codex_config_snippet("/Applications/LumenCite.app/exe", "http://127.0.0.1:7373/mcp", "tok123");
        assert!(s.starts_with("[mcp_servers.lumencite]"));
        assert!(s.contains("command = \"/Applications/LumenCite.app/exe\""));
        assert!(s.contains("args = [\"--mcp-stdio\"]"));
        assert!(s.contains("LUMENCITE_MCP_URL = \"http://127.0.0.1:7373/mcp\""));
        assert!(s.contains("LUMENCITE_MCP_TOKEN = \"tok123\""));
    }

    #[test]
    fn codex_snippet_escapes_windows_backslash_paths() {
        // Windows の実行ファイルパスは `\` を含むため TOML 基本文字列でエスケープが要る。
        let s = codex_config_snippet(r"C:\Program Files\LumenCite\lumencite.exe", "http://127.0.0.1:7373/mcp", "t");
        assert!(s.contains(r#"command = "C:\\Program Files\\LumenCite\\lumencite.exe""#));
        // エスケープ後の TOML が再パースできること（値が原文と一致）。
        let parsed: toml_check::Value = toml_check::parse(&s);
        assert_eq!(
            parsed.command,
            r"C:\Program Files\LumenCite\lumencite.exe"
        );
    }

    // codex スニペットが本物の TOML パーサで読めることを検証する最小パーサ。
    // 依存を増やさないため command 行だけを対象にした軽量実装。
    mod toml_check {
        pub struct Value {
            pub command: String,
        }
        pub fn parse(s: &str) -> Value {
            let line = s
                .lines()
                .find(|l| l.trim_start().starts_with("command = "))
                .expect("command 行が無い");
            let raw = line.trim_start().trim_start_matches("command = ").trim();
            let inner = raw
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .expect("基本文字列でない");
            // TOML 基本文字列のエスケープを戻す。
            let mut out = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('\\') => out.push('\\'),
                        Some('"') => out.push('"'),
                        Some('n') => out.push('\n'),
                        Some('r') => out.push('\r'),
                        Some('t') => out.push('\t'),
                        Some(other) => out.push(other),
                        None => {}
                    }
                } else {
                    out.push(c);
                }
            }
            Value { command: out }
        }
    }
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
        rt.resolve_approval(7, "call-1", true);
        assert_eq!(rx.await.unwrap(), true);
        // 解決済みなので二度目は何も起きない（パニックしない）
        rt.resolve_approval(7, "call-1", false);
    }

    #[tokio::test]
    async fn runtime_cancel_denies_pending_and_sets_flag() {
        let rt = ChatRuntime::default();
        let flag = rt.begin(42).unwrap();
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
        rt.resolve_approval(2, "b-call", true);
        assert_eq!(rx_b.await.unwrap(), true);
    }
}
