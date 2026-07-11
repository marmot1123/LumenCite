//! 添付ファイル/ディレクトリの削除を rename-to-trash + 永続 retry queue で堅牢化する（CR-008）。
//!
//! 直接 `remove_file` / `remove_dir_all` すると、ファイルがロック中（Windows で閲覧中の
//! PDF 等）や一時的な I/O エラーで失敗し、孤立ファイルがディスクに残る。代わりに:
//!
//! 1. 削除対象を `<app_data_dir>/.attachment-trash/` へ **rename** する。同一 FS なので
//!    原子的かつ高速で、ロック中でも rename は成功しやすく、元パスは即座に解放されるので
//!    同名の新規添付を妨げない。
//! 2. trash ディレクトリ自体を永続的な retry queue とみなし、起動時と各削除後に sweep して
//!    消せるものを消す。消せなかったもの（まだロック中等）は残し次回再試行する。
//!    ディレクトリが queue なのでプロセス再起動を跨いで永続する（追加の DB テーブル不要）。

use std::path::{Path, PathBuf};

/// trash ディレクトリ（`<app_data_dir>/.attachment-trash`）。
/// 実添付の `attachments/` とは兄弟なので sweep が実添付に触れることはない。
pub fn trash_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join(".attachment-trash")
}

/// `abs`（ファイル or ディレクトリ）を trash へ退避する。
/// - 存在しなければ何もしない（`Ok`）。
/// - rename できない（cross-device 等）場合は best-effort な直接削除にフォールバックする。
pub fn move_to_trash(app_data_dir: &Path, abs: &Path) -> std::io::Result<()> {
    if !abs.exists() {
        return Ok(());
    }
    let trash = trash_dir(app_data_dir);
    std::fs::create_dir_all(&trash)?;
    let label = abs
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "item".to_string());
    let dest = unique_path(&trash, &label);
    match std::fs::rename(abs, &dest) {
        Ok(()) => Ok(()),
        // rename 不可（別デバイス等）は直接削除に切り替える。
        Err(_) => {
            if abs.is_dir() {
                std::fs::remove_dir_all(abs)
            } else {
                std::fs::remove_file(abs)
            }
        }
    }
}

/// trash 内の一意な退避先を決める（basename 衝突は連番で回避）。
fn unique_path(trash: &Path, label: &str) -> PathBuf {
    let mut dest = trash.join(label);
    let mut n = 1u32;
    while dest.exists() {
        dest = trash.join(format!("{label}.{n}"));
        n += 1;
    }
    dest
}

/// trash を sweep して消せるものを消す（永続 retry queue の処理本体）。
/// ロック中等で消せなかったものは残し、次回の sweep で再試行する。消せた件数を返す。
pub fn sweep_trash(app_data_dir: &Path) -> usize {
    let trash = trash_dir(app_data_dir);
    let Ok(rd) = std::fs::read_dir(&trash) else {
        return 0;
    };
    let mut removed = 0;
    for entry in rd.flatten() {
        let p = entry.path();
        let ok = if p.is_dir() {
            std::fs::remove_dir_all(&p).is_ok()
        } else {
            std::fs::remove_file(&p).is_ok()
        };
        if ok {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "lc-trash-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&base).ok();
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn move_and_sweep_removes_file() {
        let root = tmp();
        let att = root.join("attachments").join("5");
        std::fs::create_dir_all(&att).unwrap();
        let f = att.join("paper.pdf");
        std::fs::write(&f, b"data").unwrap();

        move_to_trash(&root, &f).unwrap();
        assert!(!f.exists(), "元パスは解放される");
        // trash に 1 件退避されている。
        assert_eq!(std::fs::read_dir(trash_dir(&root)).unwrap().count(), 1);

        let removed = sweep_trash(&root);
        assert_eq!(removed, 1);
        assert_eq!(std::fs::read_dir(trash_dir(&root)).unwrap().count(), 0);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn move_dir_and_missing_source_are_ok() {
        let root = tmp();
        let dir = root.join("attachments").join("9");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("a.pdf"), b"x").unwrap();

        move_to_trash(&root, &dir).unwrap();
        assert!(!dir.exists(), "ディレクトリごと退避する");
        // 存在しないパスは no-op。
        move_to_trash(&root, &dir).unwrap();

        assert_eq!(sweep_trash(&root), 1, "ディレクトリも消える");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn same_basename_does_not_collide() {
        let root = tmp();
        for e in ["5", "8"] {
            let att = root.join("attachments").join(e);
            std::fs::create_dir_all(&att).unwrap();
            let f = att.join("paper.pdf");
            std::fs::write(&f, b"data").unwrap();
            move_to_trash(&root, &f).unwrap();
        }
        // 同名 basename でも連番で 2 件退避される。
        assert_eq!(std::fs::read_dir(trash_dir(&root)).unwrap().count(), 2);
        std::fs::remove_dir_all(&root).ok();
    }
}
