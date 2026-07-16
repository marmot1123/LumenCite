//! バックアップアーカイブ（`.zip`）からの復元（CR-018 の残タスク）。
//!
//! 復元はライブ DB を差し替えるため、稼働中の `SqlitePool` を握ったまま
//! `lumencite.db` を上書きするのは危険（特に Windows はオープン中ファイルを置換できない）。
//! そこで **2 フェーズ**に分ける:
//!
//! 1. **staging（アプリ稼働中）** — [`stage_restore`]:
//!    `.zip` を検証（`db.sqlite` の存在・`PRAGMA integrity_check`・スキーマ版が
//!    このアプリより新しくないか）し、**復元前に現行状態を自動フルバックアップ**して
//!    安全網を張ってから、内容を `<app_dir>/pending-restore/` へ展開してマーカーを置く。
//!    その後フロントがアプリを再起動する。
//! 2. **apply（次回起動時・pool を開く前）** — [`apply_pending_restore`]:
//!    現行 `lumencite.db`（＋ `-wal`/`-shm`）と `attachments/` を `<app_dir>/pre-restore/`
//!    へ退避し、staged の内容を所定位置へ移す。途中失敗時は退避物から巻き戻す。
//!    pool を開く前に実行するので、オープン中ファイル置換の問題を避けられる。
//!
//! `RESTORE_LOCK` で staging を直列化する。zip-slip（`..` やアーカイブ外への書き出し）も防ぐ。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Row, SqlitePool};

/// ライブ DB のファイル名（`lib.rs` の `SqliteConnectOptions::filename` と一致させる）。
const DB_FILE: &str = "lumencite.db";
/// staging 済み復元内容を置くディレクトリ名。
const PENDING_DIR: &str = "pending-restore";
/// 適用時に退避した現行データを置くディレクトリ名（ロールバック用・1 世代保持）。
const PRE_RESTORE_DIR: &str = "pre-restore";
/// staging 完了を示すマーカーファイル名。これが揃って初めて起動時 apply が動く。
const MARKER: &str = ".ready";
/// アーカイブ内の DB エントリ名（`backup::write_archive` と一致）。
const ARCHIVE_DB: &str = "db.sqlite";
/// アーカイブ内の添付プレフィックス（`backup::write_archive` と一致）。
const ARCHIVE_ATTACH_PREFIX: &str = "attachments/";

/// 復元 staging を直列化するプロセス全体で共有のロック。
/// 復元中の再入や、自動バックアップ・別の復元操作との競合を防ぐ。
static RESTORE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug)]
pub struct StageInfo {
    /// 復元直前に取った安全網バックアップ（`.zip`）のパス。
    pub safety_backup: PathBuf,
}

#[derive(Debug)]
pub struct ApplyInfo {
    /// 退避した旧データの置き場所（`<app_dir>/pre-restore`）。
    pub pre_restore: PathBuf,
}

/// このアプリにコンパイル済み migration の最大バージョン。
/// 復元対象 DB がこれより新しい migration を持つ場合、古いアプリで新しい DB を開くことになり
/// クラッシュや破損を招くため拒否する。
fn embedded_max_migration_version() -> i64 {
    sqlx::migrate!("./migrations")
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0)
}

/// 展開済み `db.sqlite` を検証する。
/// - `PRAGMA integrity_check` が `ok` を返すこと。
/// - `_sqlx_migrations` が存在し（＝LumenCite の DB であること）、
///   その最大バージョンがこのアプリの最大 migration 以下であること。
async fn validate_db_file(db_path: &Path) -> Result<(), String> {
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false)
        .read_only(true);
    let pool = SqlitePool::connect_with(options)
        .await
        .map_err(|e| format!("バックアップ内の DB を開けません: {e}"))?;

    // integrity_check は健全なら 1 行 "ok" を返す。
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(&pool)
        .await
        .map_err(|e| format!("整合性チェックに失敗しました: {e}"))?;
    if integrity != "ok" {
        pool.close().await;
        return Err(format!("バックアップ内の DB が壊れています: {integrity}"));
    }

    // _sqlx_migrations が無ければ LumenCite のバックアップではない。
    let has_migrations: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'")
            .fetch_optional(&pool)
            .await
            .map_err(|e| e.to_string())?;
    if has_migrations.is_none() {
        pool.close().await;
        return Err("LumenCite のバックアップではありません（migration 情報がありません）".into());
    }

    let backup_version: i64 = sqlx::query("SELECT COALESCE(MAX(version), 0) AS v FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?
        .try_get("v")
        .map_err(|e| e.to_string())?;
    pool.close().await;

    let app_version = embedded_max_migration_version();
    if backup_version > app_version {
        return Err(format!(
            "このバックアップは新しいバージョンの LumenCite で作成されています（DB schema {backup_version} > 対応 {app_version}）。\
             アプリを更新してから復元してください。"
        ));
    }
    Ok(())
}

/// アーカイブのエントリ名を安全な相対パスへ正規化する（zip-slip 対策）。
/// - 絶対パス、`..`、ドライブ接頭辞を含むものは拒否。
/// - `db.sqlite` そのもの、または `attachments/` 配下のみ許可する。
fn safe_archive_path(name: &str) -> Option<PathBuf> {
    // ディレクトリエントリ（末尾 `/`）は無視。
    if name.ends_with('/') {
        return None;
    }
    // 許可リスト: db.sqlite か attachments/ 配下だけ。
    let is_db = name == ARCHIVE_DB;
    let is_attach = name.starts_with(ARCHIVE_ATTACH_PREFIX);
    if !is_db && !is_attach {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in name.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            return None; // トラバーサル拒否
        }
        // Windows のドライブ/UNC を弾く。
        if comp.contains('\\') || comp.contains(':') {
            return None;
        }
        out.push(comp);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// `.zip` を `dest` 以下へ展開する（db.sqlite ＋ attachments/ のみ・zip-slip 防止）。
/// `db.sqlite` を 1 つ以上含まなければエラー。
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| format!("アーカイブを開けません: {e}"))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| format!("アーカイブを読めません（.zip ですか？）: {e}"))?;

    let mut saw_db = false;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        let Some(rel) = safe_archive_path(&name) else {
            continue; // 対象外・危険なエントリはスキップ
        };
        if rel == Path::new(ARCHIVE_DB) {
            saw_db = true;
        }
        let out_path = dest.join(&rel);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = fs::File::create(&out_path).map_err(|e| e.to_string())?;
        io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }

    if !saw_db {
        return Err("バックアップに db.sqlite が含まれていません".into());
    }
    Ok(())
}

/// 復元をステージングする（アプリ稼働中に呼ぶ）。
/// 成功後、呼び出し側はアプリを再起動する。実際の差し替えは次回起動時の
/// [`apply_pending_restore`] が行う。
pub async fn stage_restore(
    pool: &SqlitePool,
    app_dir: &Path,
    archive: &Path,
) -> Result<StageInfo, String> {
    let _guard = RESTORE_LOCK.lock().await;

    if !archive.is_file() {
        return Err("指定されたバックアップファイルが見つかりません".into());
    }

    let pending = app_dir.join(PENDING_DIR);
    // 前回の未完了 staging が残っていれば作り直す。
    let _ = fs::remove_dir_all(&pending);
    fs::create_dir_all(&pending).map_err(|e| e.to_string())?;

    // 失敗時は staging ディレクトリごと片付ける。
    let staged = (|| -> Result<PathBuf, String> {
        extract_archive(archive, &pending)?;
        Ok(pending.join(ARCHIVE_DB))
    })();
    let staged_db = match staged {
        Ok(p) => p,
        Err(e) => {
            let _ = fs::remove_dir_all(&pending);
            return Err(e);
        }
    };

    // 展開した DB を検証（整合性・スキーマ版）。
    if let Err(e) = validate_db_file(&staged_db).await {
        let _ = fs::remove_dir_all(&pending);
        return Err(e);
    }

    // 安全網: 復元前に現行状態を完全バックアップする。ここが失敗するなら復元も中止する。
    let safety_backup = match crate::backup::run_backup(pool, app_dir, 14).await {
        Ok(p) => p,
        Err(e) => {
            let _ = fs::remove_dir_all(&pending);
            return Err(format!("復元前の自動バックアップに失敗したため中止しました: {e}"));
        }
    };

    // 全部揃ってからマーカーを置く。マーカーがある時だけ起動時 apply が動く。
    fs::write(pending.join(MARKER), b"ready").map_err(|e| e.to_string())?;

    Ok(StageInfo { safety_backup })
}

/// `<app_dir>/pending-restore` にステージ済みの復元があるかを返す。
pub fn has_pending_restore(app_dir: &Path) -> bool {
    app_dir.join(PENDING_DIR).join(MARKER).is_file()
}

/// 起動時、pool を開く前に呼ぶ。ステージ済みの復元があれば適用する。
/// - 適用成功: 旧データを `pre-restore/` に残したまま `Ok(Some(_))`。
/// - 何もなし: `Ok(None)`。
/// - 途中失敗: 退避物から巻き戻したうえで `Err(_)`（呼び出し側は旧 DB のまま続行できる）。
pub fn apply_pending_restore(app_dir: &Path) -> Result<Option<ApplyInfo>, String> {
    let pending = app_dir.join(PENDING_DIR);
    let marker = pending.join(MARKER);
    if !marker.is_file() {
        // マーカー無し = 未完了 staging の残骸。あれば掃除して no-op。
        if pending.exists() {
            let _ = fs::remove_dir_all(&pending);
        }
        return Ok(None);
    }

    let staged_db = pending.join(ARCHIVE_DB);
    if !staged_db.is_file() {
        // マーカーはあるが本体が無い異常状態。安全側で破棄。
        let _ = fs::remove_dir_all(&pending);
        return Err("ステージされた復元に db.sqlite がありません。復元を中止しました。".into());
    }

    let pre = app_dir.join(PRE_RESTORE_DIR);
    // 直前世代の退避が残っていれば作り直す。
    let _ = fs::remove_dir_all(&pre);
    fs::create_dir_all(&pre).map_err(|e| e.to_string())?;

    let staged_att = pending.join("attachments");

    // 現行 → 退避 → staged を所定位置へ、を 1 つでも失敗したら巻き戻す。
    let swap = (|| -> Result<(), String> {
        move_live_aside(app_dir, &pre)?;
        move_path(&staged_db, &app_dir.join(DB_FILE))?;
        if staged_att.is_dir() {
            move_path(&staged_att, &app_dir.join("attachments"))?;
        }
        Ok(())
    })();

    match swap {
        Ok(()) => {
            // マーカーと残骸を消す（再起動ループ防止）。pre-restore は 1 世代残す。
            let _ = fs::remove_dir_all(&pending);
            Ok(Some(ApplyInfo { pre_restore: pre }))
        }
        Err(e) => {
            // 退避物から元の状態へ巻き戻す。
            rollback_from_pre(app_dir, &pre);
            let _ = fs::remove_dir_all(&pending);
            let _ = fs::remove_dir_all(&pre);
            Err(format!("復元に失敗し、元の状態へ巻き戻しました: {e}"))
        }
    }
}

/// 現行のライブ DB（＋ WAL/SHM）と `attachments/` を `pre` へ退避する。
fn move_live_aside(app_dir: &Path, pre: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let name = format!("{DB_FILE}{suffix}");
        let live = app_dir.join(&name);
        if live.exists() {
            move_path(&live, &pre.join(&name))?;
        }
    }
    let live_att = app_dir.join("attachments");
    if live_att.is_dir() {
        move_path(&live_att, &pre.join("attachments"))?;
    }
    Ok(())
}

/// 退避物（`pre`）から現行位置へ戻す（apply 途中失敗時のロールバック）。best-effort。
fn rollback_from_pre(app_dir: &Path, pre: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let name = format!("{DB_FILE}{suffix}");
        let live = app_dir.join(&name);
        // 差し替え途中で作られた新ファイルを消してから戻す。
        let _ = fs::remove_file(&live);
        let saved = pre.join(&name);
        if saved.exists() {
            let _ = move_path(&saved, &live);
        }
    }
    let live_att = app_dir.join("attachments");
    let _ = fs::remove_dir_all(&live_att);
    let saved_att = pre.join("attachments");
    if saved_att.is_dir() {
        let _ = move_path(&saved_att, &live_att);
    }
}

/// `from` を `to` へ移動する。まず `rename`、跨デバイスで失敗したら copy+delete で代替する。
fn move_path(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            // 別ファイルシステム跨ぎ等で rename 不可なとき。
            if from.is_dir() {
                copy_dir_all(from, to).map_err(|e| e.to_string())?;
                fs::remove_dir_all(from).map_err(|e| e.to_string())?;
            } else {
                fs::copy(from, to).map_err(|e| e.to_string())?;
                fs::remove_file(from).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
    }
}

fn copy_dir_all(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_all(&src, &dst)?;
        } else {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// テスト用: db.sqlite ＋ 添付を含む有効なアーカイブを、稼働 DB から作る。
    async fn make_archive(pool: &SqlitePool, work: &Path) -> PathBuf {
        // 添付レイアウトを用意
        let att = work.join("attachments").join("42");
        fs::create_dir_all(&att).unwrap();
        fs::write(att.join("paper.pdf"), b"%PDF-1.7 restore-test").unwrap();
        crate::backup::run_backup(pool, work, 14).await.unwrap()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn stage_then_apply_restores_db_and_attachments(pool: SqlitePool) {
        let src = std::env::temp_dir().join(format!("lc-restore-src-{}", std::process::id()));
        fs::remove_dir_all(&src).ok();
        fs::create_dir_all(&src).unwrap();
        let archive = make_archive(&pool, &src).await;

        // 復元先の app_dir を用意（別の中身のライブ DB と添付を置く）。
        let app = std::env::temp_dir().join(format!("lc-restore-app-{}", std::process::id()));
        fs::remove_dir_all(&app).ok();
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join(DB_FILE), b"OLD-DB-CONTENT").unwrap();
        fs::write(app.join(format!("{DB_FILE}-wal")), b"OLD-WAL").unwrap();
        let old_att = app.join("attachments").join("99");
        fs::create_dir_all(&old_att).unwrap();
        fs::write(old_att.join("old.pdf"), b"OLD-ATTACH").unwrap();

        // staging
        let info = stage_restore(&pool, &app, &archive).await.unwrap();
        assert!(info.safety_backup.exists(), "安全網バックアップが作られる");
        assert!(has_pending_restore(&app));

        // apply（起動時相当）
        let applied = apply_pending_restore(&app).unwrap();
        assert!(applied.is_some(), "適用された");

        // ライブ DB がアーカイブ由来（sqlite ヘッダ）に置き換わっている。
        let mut header = [0u8; 16];
        {
            let mut f = fs::File::open(app.join(DB_FILE)).unwrap();
            f.read_exact(&mut header).unwrap();
        }
        assert_eq!(&header[..15], b"SQLite format 3", "復元 DB は本物の SQLite");

        // 復元した添付が存在し、旧 WAL は消えている。
        assert!(app.join("attachments").join("42").join("paper.pdf").is_file());
        assert!(!app.join(format!("{DB_FILE}-wal")).exists(), "旧 WAL は退避済み");
        // 旧データは pre-restore に退避されている。
        assert!(app.join(PRE_RESTORE_DIR).join(DB_FILE).is_file());
        // pending は消えている（再起動ループ防止）。
        assert!(!has_pending_restore(&app));

        fs::remove_dir_all(&src).ok();
        fs::remove_dir_all(&app).ok();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn apply_is_noop_without_marker(_pool: SqlitePool) {
        let app = std::env::temp_dir().join(format!("lc-restore-noop-{}", std::process::id()));
        fs::remove_dir_all(&app).ok();
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join(DB_FILE), b"LIVE").unwrap();

        let applied = apply_pending_restore(&app).unwrap();
        assert!(applied.is_none());
        // ライブ DB は無傷。
        assert_eq!(fs::read(app.join(DB_FILE)).unwrap(), b"LIVE");

        fs::remove_dir_all(&app).ok();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn stage_rejects_non_archive(pool: SqlitePool) {
        let app = std::env::temp_dir().join(format!("lc-restore-bad-{}", std::process::id()));
        fs::remove_dir_all(&app).ok();
        fs::create_dir_all(&app).unwrap();
        let bad = app.join("not-a-zip.zip");
        fs::write(&bad, b"this is not a zip file").unwrap();

        let err = stage_restore(&pool, &app, &bad).await.unwrap_err();
        assert!(err.contains("アーカイブ"), "err={err}");
        // 失敗時に staging を残さない。
        assert!(!has_pending_restore(&app));

        fs::remove_dir_all(&app).ok();
    }

    /// zip 内 db.sqlite のスキーマ版がアプリより新しい場合は拒否する。
    #[sqlx::test(migrations = "./migrations")]
    async fn stage_rejects_newer_schema(pool: SqlitePool) {
        let app = std::env::temp_dir().join(format!("lc-restore-newer-{}", std::process::id()));
        fs::remove_dir_all(&app).ok();
        fs::create_dir_all(&app).unwrap();

        // 未来バージョンの migration 行を持つ DB を作る。
        let future = embedded_max_migration_version() + 100;
        let db_path = app.join("future.sqlite");
        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let p = SqlitePool::connect_with(opts).await.unwrap();
            sqlx::query(
                "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT, \
                 installed_on TIMESTAMP, success BOOLEAN, checksum BLOB, execution_time BIGINT)",
            )
            .execute(&p)
            .await
            .unwrap();
            sqlx::query("INSERT INTO _sqlx_migrations (version, description, success) VALUES (?, 'future', 1)")
                .bind(future)
                .execute(&p)
                .await
                .unwrap();
            p.close().await;
        }
        // db.sqlite として zip に固める。
        let archive = app.join("future-backup.zip");
        {
            let f = fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file(ARCHIVE_DB, opts).unwrap();
            let mut dbf = fs::File::open(&db_path).unwrap();
            let mut buf = Vec::new();
            dbf.read_to_end(&mut buf).unwrap();
            use std::io::Write;
            zip.write_all(&buf).unwrap();
            zip.finish().unwrap();
        }

        let err = stage_restore(&pool, &app, &archive).await.unwrap_err();
        assert!(err.contains("新しいバージョン"), "err={err}");
        assert!(!has_pending_restore(&app));

        fs::remove_dir_all(&app).ok();
    }

    #[test]
    fn safe_archive_path_blocks_traversal() {
        assert!(safe_archive_path("db.sqlite").is_some());
        assert!(safe_archive_path("attachments/42/p.pdf").is_some());
        assert!(safe_archive_path("attachments/../../etc/passwd").is_none());
        assert!(safe_archive_path("../secret").is_none());
        assert!(safe_archive_path("/etc/passwd").is_none());
        assert!(safe_archive_path("other.txt").is_none());
        assert!(safe_archive_path("attachments/").is_none());
    }
}
