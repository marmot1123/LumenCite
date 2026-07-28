//! pdfium 動的ライブラリのバインド（LCIR 抽出と OCR で共用する単一ソース）。

use pdfium_render::prelude::*;
use std::path::{Path, PathBuf};

/// `tauri.conf.json` の `productName`。Linux では Tauri が `bundle.resources` を
/// `<exe>/../lib/<productName>/` へ置く（tauri-utils の `resource_dir_from` と同じ規則）。
const PRODUCT_NAME: &str = "LumenCite";

/// pdfium を探す候補ディレクトリを実行ファイルのパスから組み立てる（IO しない純関数）。
///
/// 探索順:
/// 1. 実行ファイル隣 — 通常のバイナリ隣、macOS の `Contents/MacOS`、
///    **Windows のリソースディレクトリ**（Tauri は Windows では exe と同じ場所に置く）
/// 2. `../Frameworks` / `../Resources` — macOS `.app` バンドル
/// 3. `../lib/<productName>` / `../lib/<crate 名>` / `../lib/<実行ファイル名>` —
///    **Linux のリソースディレクトリ**（deb/rpm は `/usr/bin/x` → `/usr/lib/<name>`、
///    AppImage は `$APPDIR/usr/bin` → `$APPDIR/usr/lib/<name>`）。
///    `<name>` が productName・crate 名・バイナリ名のどれになるかは配布形態と bundler 版に依存するため、
///    3 つとも見る（存在しないディレクトリは `bind_to_library` が失敗して次の候補へ進むだけ）。
/// 4. `pdfium` / `.` — dev（`src-tauri/pdfium/`）とカレント
fn library_search_dirs(exe: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(exe) = exe {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            dirs.push(dir.join("../Frameworks"));
            dirs.push(dir.join("../Resources"));

            let lib = dir.join("../lib");
            let mut push_unique = |p: PathBuf| {
                if !dirs.contains(&p) {
                    dirs.push(p);
                }
            };
            push_unique(lib.join(PRODUCT_NAME));
            push_unique(lib.join(env!("CARGO_PKG_NAME")));
            if let Some(stem) = exe.file_stem() {
                push_unique(lib.join(stem));
            }
        }
    }
    dirs.push(PathBuf::from("pdfium")); // dev: src-tauri/pdfium
    dirs.push(PathBuf::from("."));
    dirs
}

/// pdfium 動的ライブラリを複数の候補から探してバインドする。
/// 候補は [`library_search_dirs`] を参照。見つからなければ最後にシステムライブラリ。
pub fn bind_pdfium() -> Result<Box<dyn PdfiumLibraryBindings>, String> {
    let exe = std::env::current_exe().ok();
    for dir in library_search_dirs(exe.as_deref()) {
        let name = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(b) = Pdfium::bind_to_library(&name) {
            return Ok(b);
        }
    }
    Pdfium::bind_to_system_library().map_err(|e| format!("pdfium library not found: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains(dirs: &[PathBuf], suffix: &str) -> bool {
        dirs.iter().any(|d| d.ends_with(suffix))
    }

    #[test]
    fn search_dirs_include_exe_dir_first() {
        // Windows のリソースディレクトリ = exe と同じ場所。先頭で当たる必要がある。
        let dirs = library_search_dirs(Some(Path::new("/opt/app/LumenCite.exe")));
        assert_eq!(dirs[0], PathBuf::from("/opt/app"));
    }

    #[test]
    fn search_dirs_include_macos_bundle_locations() {
        let dirs = library_search_dirs(Some(Path::new(
            "/Applications/LumenCite.app/Contents/MacOS/LumenCite",
        )));
        assert!(contains(&dirs, "Contents/MacOS/../Frameworks"), "{dirs:?}");
        assert!(contains(&dirs, "Contents/MacOS/../Resources"), "{dirs:?}");
    }

    #[test]
    fn search_dirs_include_linux_resource_dir_for_deb_layout() {
        // deb/rpm: /usr/bin/lumencite → リソースは /usr/lib/LumenCite/
        let dirs = library_search_dirs(Some(Path::new("/usr/bin/lumencite")));
        assert!(contains(&dirs, "../lib/LumenCite"), "{dirs:?}");
        // バイナリ名が productName と異なる配布形態向けのフォールバック
        assert!(contains(&dirs, "../lib/lumencite"), "{dirs:?}");
    }

    #[test]
    fn search_dirs_include_linux_resource_dir_for_appimage_layout() {
        let dirs = library_search_dirs(Some(Path::new("/tmp/.mount_x/usr/bin/LumenCite")));
        assert!(contains(&dirs, "usr/bin/../lib/LumenCite"), "{dirs:?}");
    }

    #[test]
    fn search_dirs_do_not_duplicate_when_binary_matches_product_name() {
        let dirs = library_search_dirs(Some(Path::new("/usr/bin/LumenCite")));
        let hits = dirs
            .iter()
            .filter(|d| d.ends_with("../lib/LumenCite"))
            .count();
        assert_eq!(hits, 1, "{dirs:?}");
        // バイナリ名が productName でも crate 名側の候補は残す
        // （Linux のリソース dir 名が productName / crate 名のどちらになるか配布形態依存のため）
        assert!(contains(&dirs, "../lib/lumencite"), "{dirs:?}");
    }

    #[test]
    fn search_dirs_fall_back_to_dev_paths_without_exe() {
        let dirs = library_search_dirs(None);
        assert_eq!(dirs, vec![PathBuf::from("pdfium"), PathBuf::from(".")]);
    }
}
