//! DB バックアップ（CR-018: 添付本体込みの完全バックアップ）。
//! - SQLite の `VACUUM INTO` を使って読み取り中でもロックを取らずに DB のクリーンコピーを作り、
//!   添付本体（`<app_data_dir>/attachments/`）とあわせて単一の `.zip` に束ねる。
//! - 保管先は `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.zip`。
//!   アーカイブ内レイアウトは `db.sqlite` ＋ `attachments/<entry_id>/<file_name>`。
//! - 直近 `keep` 世代のみ残し、それより古いものは削除する（旧 `.db` バックアップも対象）。
//!
//! 作業ファイルは全て「完成前は拾われない名前」で書く:
//! - VACUUM INTO の中間 DB … `.vacuum-<stem>.db.tmp`
//! - 書き込み中のアーカイブ … `<stem>.zip.partial`（完成時に `<stem>.zip` へ rename）
//!
//! これらはプロセスが途中で殺されると残るため、起動時に [`sweep_backup_workdir`] で回収する
//! （中間 DB は DB と同サイズ = 数百 MB あり、放置するとディスクを食い潰す）。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use sqlx::SqlitePool;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// 既定の保持世代数。起動時 / 24h 自動バックアップと手動実行で共有する。
pub const DEFAULT_KEEP: usize = 14;

/// 自動バックアップの最小間隔（秒）。前回成功からこれ未満なら起動時バックアップは走らない。
/// 起動のたびにフルバックアップ（VACUUM + zip）を走らせると、ライブラリが育つほど
/// 起動直後の数分間ディスクと CPU を占有し、MCP サーバーの起動や初期描画を巻き添えにする。
pub const AUTO_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// バックアップ対象ファイルか判定する。
/// 完全バックアップは `.zip`。旧世代の DB-only バックアップ（`.db`）も
/// 一覧表示・世代管理の対象に含める（放置すると prune されず溜まり続けるため）。
/// 書きかけの `.zip.partial` は「バックアップ」ではないので含めない。
fn is_backup_file(name: &str) -> bool {
    name.starts_with("lumencite-") && (name.ends_with(".zip") || name.ends_with(".db"))
}

/// 途中終了で残った作業ファイルか判定する（[`sweep_backup_workdir`] の対象）。
fn is_leftover_work_file(name: &str) -> bool {
    // VACUUM INTO の中間 DB とその journal（`.vacuum-<stem>.db.tmp[-journal]`）
    (name.starts_with(".vacuum-") && name.contains(".db.tmp"))
        // 書き込み途中で死んだアーカイブ
        || (name.starts_with("lumencite-") && name.ends_with(".zip.partial"))
        // 旧 .db バックアップの journal。本体が prune された後もこれだけ取り残される
        // （`is_backup_file` に一致しないので世代管理からも漏れる）。
        || (name.starts_with("lumencite-") && name.ends_with(".db-journal"))
}

#[derive(Debug, serde::Serialize)]
pub struct BackupInfo {
    pub path: String,
    pub file_name: String,
    pub created_at: String,
    pub size_bytes: u64,
}

/// バックアップを直列化するプロセス全体で共有のロック（CR-022）。
/// 自動バックアップ（起動時 + 24h タイマー）と手動実行（`run_backup_now`）が重なると、
/// ①同一秒のファイル名選択が TOCTOU で衝突（VACUUM INTO が「already exists」で失敗）、
/// ②`prune_old_backups` が別実行の作成中ファイルを消す、といった競合が起きる。
/// DB は 1 つなのでモジュール static で足りる。
static BACKUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// バックアップを実行する（手動実行 = 常に走る）。
pub async fn run_backup(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
) -> Result<PathBuf, String> {
    // ファイル名選択 → VACUUM INTO → zip → prune を他のバックアップと直列化する（CR-022）。
    let _guard = BACKUP_LOCK.lock().await;
    run_backup_inner(pool, app_dir, keep).await
}

/// 前回成功から `min_interval_secs` 以上経っているときだけバックアップする（自動実行用）。
/// 間引かれた場合は `Ok(None)`。
///
/// 判定はバックアップ本体と同じロックの内側で行う。外で判定すると、
/// 同時に走った 2 本が両方「due」と判断してフルバックアップを二重に走らせてしまう。
pub async fn run_backup_if_due(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
    min_interval_secs: i64,
) -> Result<Option<PathBuf>, String> {
    let _guard = BACKUP_LOCK.lock().await;
    if !is_backup_due(pool, min_interval_secs).await {
        return Ok(None);
    }
    run_backup_inner(pool, app_dir, keep).await.map(Some)
}

/// 前回成功時刻の記録を見て、自動バックアップを走らせるべきか判定する。
/// 記録が無い（初回・旧版からの更新直後）／読めない場合は「走らせる」側に倒す。
async fn is_backup_due(pool: &SqlitePool, min_interval_secs: i64) -> bool {
    let last = crate::db::settings::get_setting(pool, crate::db::settings::BACKUP_LAST_RUN_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(&raw).ok());
    let Some(last) = last else {
        return true;
    };
    let elapsed = (Local::now() - last.with_timezone(&Local)).num_seconds();
    // 未来の時刻が記録されている（時計のずれ・タイムゾーン変更）ときも走らせる。
    // そうしないと記録が過去に戻るまでバックアップが永久に止まる。
    elapsed >= min_interval_secs || elapsed < 0
}

async fn run_backup_inner(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
) -> Result<PathBuf, String> {
    let backups_dir = app_dir.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let mut stem = format!("lumencite-{}", timestamp);
    let mut target = backups_dir.join(format!("{}.zip", stem));
    let mut partial = backups_dir.join(format!("{}.zip.partial", stem));
    // タイムスタンプは秒精度なので、同一秒内の連続実行ではファイル名が衝突する。
    // 接尾辞で一意化する（アーカイブ本体・書きかけ・VACUUM 一時ファイルの全てに使う）。
    let mut n = 1usize;
    while target.exists() || partial.exists() {
        stem = format!("lumencite-{}-{}", timestamp, n);
        target = backups_dir.join(format!("{}.zip", stem));
        partial = backups_dir.join(format!("{}.zip.partial", stem));
        n += 1;
    }

    // VACUUM INTO は既存ファイルへは書けないので、まず一時 DB ファイルに吐き出してから
    // zip に格納する。一時ファイルは `lumencite-` 前缀を避け、is_backup_file に拾われないようにする。
    let tmp_db = backups_dir.join(format!(".vacuum-{}.db.tmp", stem));
    let _ = fs::remove_file(&tmp_db); // 前回異常終了の残骸があれば掃除

    let build = async {
        // VACUUM INTO は通常のクエリと違ってトランザクション内で実行できないので
        // SQL リテラルとしてパスを直接埋め込む。シングルクォートをエスケープしておく。
        let tmp_str = tmp_db.to_string_lossy().replace('\'', "''");
        let sql = format!("VACUUM INTO '{}'", tmp_str);
        sqlx::query(&sql)
            .execute(pool)
            .await
            .map_err(|e| format!("VACUUM INTO failed: {}", e))?;

        // zip 書き出しは同期 I/O ＋ CPU バウンド（deflate）で、ライブラリの規模によっては
        // 分単位かかる。async タスク内で直接回すと tokio のワーカーを丸ごと占有するので
        // blocking プールへ逃がす。
        let (dst, src, att) = (
            partial.clone(),
            tmp_db.clone(),
            app_dir.join("attachments"),
        );
        tokio::task::spawn_blocking(move || write_archive(&dst, &src, &att))
            .await
            .map_err(|e| format!("archive task failed: {}", e))?
            .map_err(|e| format!("archive write failed: {}", e))?;

        // 完成してから正式名に付け替える。途中で殺されても `.zip.partial` が残るだけで、
        // 中身の欠けたアーカイブが「バックアップ」として一覧・世代管理に混ざらない。
        fs::rename(&partial, &target).map_err(|e| format!("archive rename failed: {}", e))?;
        Ok::<(), String>(())
    };

    let result = build.await;
    // 一時ファイルは成功・失敗どちらでも掃除する。
    let _ = fs::remove_file(&tmp_db);
    if let Err(e) = result {
        // 途中失敗した壊れかけのアーカイブを残さない。
        let _ = fs::remove_file(&partial);
        return Err(e);
    }

    // 次回の自動バックアップの間引き判定に使う。成功時のみ記録するので、
    // 失敗し続ける限りは毎回リトライされる。
    if let Err(e) = crate::db::settings::set_setting(
        pool,
        crate::db::settings::BACKUP_LAST_RUN_KEY,
        &Local::now().to_rfc3339(),
    )
    .await
    {
        eprintln!("backup: failed to record last run time: {e}");
    }

    prune_old_backups(&backups_dir, keep).map_err(|e| e.to_string())?;

    Ok(target)
}

/// 作業ファイルを「前回の残骸」とみなすまでの猶予。
/// GUI ロックは try_lock で 2 個目の起動を止めない（dev ビルドと配布版の併用など）ので、
/// 別インスタンスが今まさに書いている作業ファイルを消さないよう、更新から間を置く。
const WORK_FILE_STALE_SECS: u64 = 60 * 60;

/// 前回の異常終了で残った作業ファイルを回収し、世代数も整える（起動時に 1 回）。
/// 戻り値は削除したファイル数。
///
/// `prune_old_backups` はバックアップ成功時にしか走らないため、途中終了が続くと
/// 世代が `keep` を超えたまま溜まる。ここでも 1 回かけておく。
pub fn sweep_backup_workdir(app_dir: &Path, keep: usize) -> usize {
    let backups_dir = app_dir.join("backups");
    let Ok(rd) = fs::read_dir(&backups_dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut removed = 0;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_leftover_work_file(&name) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age.as_secs() >= WORK_FILE_STALE_SECS);
        if stale && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    let _ = prune_old_backups(&backups_dir, keep);
    removed
}

/// `db.sqlite` ＋ `attachments/…` を単一 zip に書き出す。
fn write_archive(target: &Path, db_file: &Path, attachments_dir: &Path) -> io::Result<()> {
    // ZipWriter は圧縮のたびに小さく write するので、素の File だと syscall が支配的になる。
    let file = io::BufWriter::with_capacity(1 << 20, fs::File::create(target)?);
    let mut zip = zip::ZipWriter::new(file);
    // 添付は PDF / PNG で「既に圧縮済みだから deflate は無駄」と思いがちだが、実ライブラリで
    // 測ると PNG（LCIR の図 crop）が 64%、PDF が 86% まで縮む。無圧縮にすると 1 世代あたり
    // 200MB 以上・14 世代で 3GB 以上増えるので、全体を deflate で統一する。
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("db.sqlite", opts)?;
    let mut db = fs::File::open(db_file)?;
    io::copy(&mut db, &mut zip)?;

    if attachments_dir.is_dir() {
        add_dir_recursive(&mut zip, attachments_dir, "attachments", opts)?;
    }

    zip.finish()?.flush()?;
    Ok(())
}

/// `dir` 以下を再帰的に zip へ追加する。アーカイブ内パスは `prefix` からの `/` 区切り。
fn add_dir_recursive<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    opts: SimpleFileOptions,
) -> io::Result<()> {
    // 決定的な順序で走査する（テスト容易性と差分の安定のため）。
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let zip_path = format!("{}/{}", prefix, name);
        if path.is_dir() {
            add_dir_recursive(zip, &path, &zip_path, opts)?;
        } else if path.is_file() {
            zip.start_file(&zip_path, opts)?;
            let mut f = fs::File::open(&path)?;
            io::copy(&mut f, zip)?;
        }
    }
    Ok(())
}

pub fn list_backups(app_dir: &Path) -> Result<Vec<BackupInfo>, String> {
    let backups_dir = app_dir.join("backups");
    if !backups_dir.exists() {
        return Ok(vec![]);
    }

    let mut entries: Vec<BackupInfo> = fs::read_dir(&backups_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !is_backup_file(&name) {
                return None;
            }
            let meta = e.metadata().ok()?;
            let modified = meta.modified().ok()?;
            let dt: chrono::DateTime<Local> = modified.into();
            Some(BackupInfo {
                path: e.path().to_string_lossy().to_string(),
                file_name: name,
                created_at: dt.format("%Y-%m-%d %H:%M:%S").to_string(),
                size_bytes: meta.len(),
            })
        })
        .collect();

    // 新しい順
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(entries)
}

fn prune_old_backups(backups_dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut paths: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(backups_dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if !is_backup_file(&name) {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), modified))
        })
        .collect();

    // 新しい順にソートし、keep 件を超えたものを削除
    paths.sort_by_key(|p| std::cmp::Reverse(p.1));
    for (path, _) in paths.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// zip アーカイブ内のエントリ名一覧を返すテストヘルパ。
    fn archive_names(path: &Path) -> Vec<String> {
        let file = fs::File::open(path).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn consecutive_backups_in_same_second_all_succeed(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-test-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        // 3 連続実行はほぼ確実に同一秒に収まる。全て成功し、別ファイルになること。
        let p1 = run_backup(&pool, &dir, 14).await.unwrap();
        let p2 = run_backup(&pool, &dir, 14).await.unwrap();
        let p3 = run_backup(&pool, &dir, 14).await.unwrap();

        assert_ne!(p1, p2);
        assert_ne!(p2, p3);
        assert!(p1.exists() && p2.exists() && p3.exists());
        // 完全バックアップは .zip
        assert!(p1.extension().is_some_and(|e| e == "zip"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// CR-018: バックアップは DB（db.sqlite）と添付本体を同一 zip に含む。
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_bundles_db_and_attachments(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-attach-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        // 添付レイアウトを模す: <app_dir>/attachments/<entry_id>/<file_name>
        let att = dir.join("attachments").join("42");
        std::fs::create_dir_all(&att).unwrap();
        std::fs::write(att.join("paper.pdf"), b"%PDF-1.7 fake pdf bytes").unwrap();
        // ネストしたサブディレクトリも再帰的に含まれること
        let nested = dir.join("attachments").join("7").join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("note.txt"), b"hello").unwrap();

        let archive = run_backup(&pool, &dir, 14).await.unwrap();
        let names = archive_names(&archive);

        assert!(names.iter().any(|n| n == "db.sqlite"), "names={names:?}");
        assert!(
            names.iter().any(|n| n == "attachments/42/paper.pdf"),
            "names={names:?}"
        );
        assert!(
            names.iter().any(|n| n == "attachments/7/sub/note.txt"),
            "names={names:?}"
        );

        // 添付本体のバイト列がそのまま格納されていること
        let file = fs::File::open(&archive).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut content = Vec::new();
        zip.by_name("attachments/42/paper.pdf")
            .unwrap()
            .read_to_end(&mut content)
            .unwrap();
        assert_eq!(content, b"%PDF-1.7 fake pdf bytes");

        // VACUUM 一時ファイルが残っていないこと
        let leftovers: Vec<_> = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".vacuum-"))
            .collect();
        assert!(leftovers.is_empty(), "temp vacuum files left: {leftovers:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 添付ディレクトリが無くても DB だけで成功する。
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_without_attachments_dir_succeeds(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-noattach-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let archive = run_backup(&pool, &dir, 14).await.unwrap();
        let names = archive_names(&archive);
        assert_eq!(names, vec!["db.sqlite".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 自動バックアップは前回成功から間隔が空いていなければ間引かれる。
    /// 手動実行（`run_backup`）は間隔に関係なく常に走る。
    #[sqlx::test(migrations = "./migrations")]
    async fn auto_backup_is_skipped_until_interval_elapses(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-due-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        // 記録が無い初回は走る。
        let first = run_backup_if_due(&pool, &dir, DEFAULT_KEEP, AUTO_INTERVAL_SECS)
            .await
            .unwrap();
        assert!(first.is_some(), "初回は実行される");

        // 直後の 2 回目は間引かれる。
        let second = run_backup_if_due(&pool, &dir, DEFAULT_KEEP, AUTO_INTERVAL_SECS)
            .await
            .unwrap();
        assert!(second.is_none(), "24h 未満なので間引かれる");

        // 手動実行は間引かれない。
        assert!(run_backup(&pool, &dir, DEFAULT_KEEP).await.is_ok());

        // 間隔 0 なら常に due。
        assert!(run_backup_if_due(&pool, &dir, DEFAULT_KEEP, 0)
            .await
            .unwrap()
            .is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 前回成功時刻が未来（時計のずれ）でも止まらない。
    #[sqlx::test(migrations = "./migrations")]
    async fn future_last_run_does_not_block_backups(pool: SqlitePool) {
        let future = (Local::now() + chrono::Duration::days(3)).to_rfc3339();
        crate::db::settings::set_setting(
            &pool,
            crate::db::settings::BACKUP_LAST_RUN_KEY,
            &future,
        )
        .await
        .unwrap();

        assert!(is_backup_due(&pool, AUTO_INTERVAL_SECS).await);
    }

    /// 途中終了で残った作業ファイルだけを回収し、本物のバックアップは消さない。
    #[test]
    fn sweep_removes_stale_work_files_only() {
        let dir = std::env::temp_dir().join(format!("lc-backup-sweep-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();

        let files = [
            ".vacuum-lumencite-20260101-000000.db.tmp",
            ".vacuum-lumencite-20260101-000000.db.tmp-journal",
            "lumencite-20260101-000000.zip.partial",
            "lumencite-20260101-000002.db-journal", // 本体が prune 済みの旧 journal
            "lumencite-20260101-000000.zip",        // 本物
            "lumencite-20260101-000001.db",         // 旧世代の DB-only バックアップ
        ];
        for f in files {
            std::fs::write(backups.join(f), b"x").unwrap();
        }
        // 猶予（WORK_FILE_STALE_SECS）を過ぎた古い残骸として mtime を戻す。
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        for f in &files[..4] {
            let file = std::fs::File::options()
                .write(true)
                .open(backups.join(f))
                .unwrap();
            file.set_modified(old).unwrap();
        }

        let removed = sweep_backup_workdir(&dir, DEFAULT_KEEP);
        assert_eq!(removed, 4, "作業ファイル 4 件だけ消える");

        let mut left: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "lumencite-20260101-000000.zip".to_string(),
                "lumencite-20260101-000001.db".to_string(),
            ]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 書き込み中の作業ファイル（新しい mtime）は別インスタンスのものかもしれないので消さない。
    #[test]
    fn sweep_keeps_fresh_work_files() {
        let dir = std::env::temp_dir().join(format!("lc-backup-sweep-fresh-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join(".vacuum-lumencite-20260101-000000.db.tmp"), b"x").unwrap();

        assert_eq!(sweep_backup_workdir(&dir, DEFAULT_KEEP), 0);
        assert!(backups.join(".vacuum-lumencite-20260101-000000.db.tmp").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 成功したバックアップは `.zip.partial` を残さない（rename されている）。
    #[sqlx::test(migrations = "./migrations")]
    async fn successful_backup_leaves_no_partial(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-partial-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let archive = run_backup(&pool, &dir, DEFAULT_KEEP).await.unwrap();
        assert!(archive.exists());

        let partials: Vec<_> = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
            .collect();
        assert!(partials.is_empty(), "partial left: {partials:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// CR-022: 同時実行でもロックで直列化され、全て成功して別ファイルになる。
    #[sqlx::test(migrations = "./migrations")]
    async fn concurrent_backups_all_succeed_with_distinct_files(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-conc-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        // 4 本を同時に投げる。ロックが無ければ同一秒のファイル名衝突で失敗し得る。
        let (r1, r2, r3, r4) = tokio::join!(
            run_backup(&pool, &dir, 14),
            run_backup(&pool, &dir, 14),
            run_backup(&pool, &dir, 14),
            run_backup(&pool, &dir, 14),
        );
        let paths = [r1.unwrap(), r2.unwrap(), r3.unwrap(), r4.unwrap()];
        for p in &paths {
            assert!(p.exists(), "{p:?} should exist");
        }
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), 4, "全て別ファイル: {paths:?}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
