//! DB バックアップ（CR-018: 添付本体込みの完全バックアップ）。
//! - SQLite の `VACUUM INTO` を使って読み取り中でもロックを取らずに DB のクリーンコピーを作り、
//!   添付本体（`<app_data_dir>/attachments/`）とあわせて単一の `.zip` に束ねる。
//! - 保管先は `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.zip`。
//!   アーカイブ内レイアウトは `db.sqlite` ＋ `attachments/<entry_id>/<file_name>`。
//! - 直近 `keep` 世代のみ残し、それより古いものは削除する（旧 `.db` バックアップも対象）。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use sqlx::SqlitePool;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// バックアップ対象ファイルか判定する。
/// 完全バックアップは `.zip`。旧世代の DB-only バックアップ（`.db`）も
/// 一覧表示・世代管理の対象に含める（放置すると prune されず溜まり続けるため）。
fn is_backup_file(name: &str) -> bool {
    name.starts_with("lumencite-") && (name.ends_with(".zip") || name.ends_with(".db"))
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

pub async fn run_backup(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
) -> Result<PathBuf, String> {
    // ファイル名選択 → VACUUM INTO → zip → prune を他のバックアップと直列化する（CR-022）。
    let _guard = BACKUP_LOCK.lock().await;

    let backups_dir = app_dir.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let mut stem = format!("lumencite-{}", timestamp);
    let mut target = backups_dir.join(format!("{}.zip", stem));
    // タイムスタンプは秒精度なので、同一秒内の連続実行ではファイル名が衝突する。
    // 接尾辞で一意化する（アーカイブ本体と VACUUM 一時ファイルの双方に使う）。
    let mut n = 1usize;
    while target.exists() {
        stem = format!("lumencite-{}-{}", timestamp, n);
        target = backups_dir.join(format!("{}.zip", stem));
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

        write_archive(&target, &tmp_db, &app_dir.join("attachments"))
            .map_err(|e| format!("archive write failed: {}", e))?;
        Ok::<(), String>(())
    };

    let result = build.await;
    // 一時 DB は成功・失敗どちらでも掃除する。
    let _ = fs::remove_file(&tmp_db);
    if let Err(e) = result {
        // 途中失敗した壊れかけのアーカイブを残さない。
        let _ = fs::remove_file(&target);
        return Err(e);
    }

    prune_old_backups(&backups_dir, keep).map_err(|e| e.to_string())?;

    Ok(target)
}

/// `db.sqlite` ＋ `attachments/…` を単一 zip に書き出す。
fn write_archive(target: &Path, db_file: &Path, attachments_dir: &Path) -> io::Result<()> {
    let file = fs::File::create(target)?;
    let mut zip = zip::ZipWriter::new(file);
    // PDF は既に圧縮済みだが、DB は deflate がよく効く。全体を deflate で統一する。
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("db.sqlite", opts)?;
    let mut db = fs::File::open(db_file)?;
    io::copy(&mut db, &mut zip)?;

    if attachments_dir.is_dir() {
        add_dir_recursive(&mut zip, attachments_dir, "attachments", opts)?;
    }

    zip.finish()?;
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
