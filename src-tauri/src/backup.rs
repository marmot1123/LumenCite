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
    let mut target = backups_dir.join(format!("lumencite-{}.db", timestamp));
    // タイムスタンプは秒精度なので、同一秒内の連続実行では VACUUM INTO が
    // 「already exists」で失敗する。接尾辞で一意化する。
    let mut n = 1usize;
    while target.exists() {
        target = backups_dir.join(format!("lumencite-{}-{}.db", timestamp, n));
        n += 1;
    }

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
    paths.sort_by_key(|p| std::cmp::Reverse(p.1));
    for (path, _) in paths.into_iter().skip(keep) {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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

        std::fs::remove_dir_all(&dir).ok();
    }
}
