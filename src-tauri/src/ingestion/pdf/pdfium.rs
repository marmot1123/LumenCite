//! pdfium 動的ライブラリのバインド（LCIR 抽出と OCR で共用する単一ソース）。

use pdfium_render::prelude::*;
use std::path::PathBuf;

/// pdfium 動的ライブラリを複数の候補から探してバインドする。
/// 候補: 実行ファイル隣 / macOS バンドルの Contents/Frameworks / Resources /
/// `pdfium`（dev では src-tauri/pdfium） / カレント → 最後にシステムライブラリ。
pub fn bind_pdfium() -> Result<Box<dyn PdfiumLibraryBindings>, String> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf()); // Contents/MacOS, または通常のバイナリ隣
            dirs.push(dir.join("../Frameworks")); // macOS .app バンドル同梱先
            dirs.push(dir.join("../Resources"));
        }
    }
    dirs.push(PathBuf::from("pdfium")); // dev: src-tauri/pdfium
    dirs.push(PathBuf::from("."));

    for dir in dirs {
        let name = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(b) = Pdfium::bind_to_library(&name) {
            return Ok(b);
        }
    }
    Pdfium::bind_to_system_library().map_err(|e| format!("pdfium library not found: {e}"))
}
