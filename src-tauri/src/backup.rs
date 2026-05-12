//! DB バックアップ。
//! - SQLite の `VACUUM INTO` を使って読み取り中でもロックを取らずにコピーを作る。
//! - 保管先は `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.db`。
//! - 直近 `keep` 世代のみ残し、それより古いものは削除する。

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Local;
use sqlx::SqlitePool;

#[derive(Debug, serde::Serialize)]
pub struct BackupInfo {
    pub path: String,
    pub file_name: String,
    pub created_at: String,
    pub size_bytes: u64,
}

pub async fn run_backup(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
) -> Result<PathBuf, String> {
    let backups_dir = app_dir.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let file_name = format!("lumencite-{}.db", timestamp);
    let target = backups_dir.join(&file_name);

    // VACUUM INTO は通常のクエリと違ってトランザクション内で実行できないので
    // SQL リテラルとしてパスを直接埋め込む。シングルクォートをエスケープしておく。
    let target_str = target.to_string_lossy().replace('\'', "''");
    let sql = format!("VACUUM INTO '{}'", target_str);
    sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| format!("VACUUM INTO failed: {}", e))?;

    prune_old_backups(&backups_dir, keep).map_err(|e| e.to_string())?;

    Ok(target)
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
            if !name.starts_with("lumencite-") || !name.ends_with(".db") {
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
            if !name.starts_with("lumencite-") || !name.ends_with(".db") {
                return None;
            }
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), modified))
        })
        .collect();

    // 新しい順にソートし、keep 件を超えたものを削除
    paths.sort_by(|a, b| b.1.cmp(&a.1));
    for (path, _) in paths.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}
