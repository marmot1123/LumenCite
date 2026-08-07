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

/// **このプロセスで pdfium が壊れていると分かった**印（v1.0.0-p2）。
///
/// 立つのは 2 つの生成点だけで、判定を文字列一致に頼らない（エラー文言が変われば無言で
/// 効かなくなり、それを守るテストも無い）:
///
/// 1. [`bind_pdfium`] がライブラリを見つけられなかった ── Windows / Linux で同梱に失敗した配布物。
/// 2. **pdfium 抽出タスクが panic した**（[`note_extraction_panic`]）── pdfium-render の
///    `PDFIUM_THREAD_MARSHALL` は panic すると毒され、以後 `PdfiumThreadMarshall::lock()` は
///    `Err` を返さず **panic する**（`thread_safe.rs:68-79`）。この状態では bind は成功し
///    続けるので、1. だけでは印が永久に立たず、残りの対象を全部 panic で焼き切る。
///
/// **読むのは自動経路だけ**（起動時バックフィル・一括バッチ・添付時の自動 build）。
/// ユーザーが名指しで押した 1 件 build は必ず実際に bind を試みて生のエラーを返す ──
/// 印はプロセス寿命なので、自動経路と同じように黙らせると pdfium を入れ直しても
/// 「押しても何も起きない」状態から再起動なしには抜けられなくなる。
static PDFIUM_BROKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// このプロセスで pdfium が使えないと分かっているか（自動経路だけが読む）。
pub fn bind_is_known_broken() -> bool {
    PDFIUM_BROKEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// pdfium 抽出タスクが panic したことを記録する（生成点 2）。
pub fn note_extraction_panic() {
    PDFIUM_BROKEN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// pdfium 動的ライブラリを複数の候補から探してバインドする。
/// 候補は [`library_search_dirs`] を参照。見つからなければ最後にシステムライブラリ。
///
/// 失敗したら [`PDFIUM_BROKEN`] を立てる（生成点 1）。**印は自動経路だけが読む**ので、
/// ここで立てても名指しの 1 件 build は次回も実際に bind を試みる。
pub fn bind_pdfium() -> Result<Box<dyn PdfiumLibraryBindings>, String> {
    let exe = std::env::current_exe().ok();
    for dir in library_search_dirs(exe.as_deref()) {
        let name = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(b) = Pdfium::bind_to_library(&name) {
            return Ok(b);
        }
    }
    Pdfium::bind_to_system_library().map_err(|e| {
        PDFIUM_BROKEN.store(true, std::sync::atomic::Ordering::Relaxed);
        format!("pdfium library not found: {e}")
    })
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
