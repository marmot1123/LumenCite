//! 実バンドルの配置から `bind_pdfium()` がライブラリを見つけるかを実機で確かめるプローブ
//! （v1.0.0-p0 の残作業・`docs/LCIR_REMAINING_PHASES.md` §2.6-5 の (ii)）。
//!
//! **なぜ本体バイナリで測らないか**: 本体は webview を立ち上げてユーザーが PDF を触るまで
//! pdfium に接触しないので、ヘッドレスな CI では「見つかったか」を問える瞬間が来ない。
//! [`bind_pdfium`] の探索は `std::env::current_exe()` の**置き場所だけ**に依存する純粋な
//! パス計算なので、バンドルの中の本体と同じパスへこのプローブを置けば、本体が辿るのと
//! 同じ候補列を同じ順で辿る。
//!
//! **「見つかった」だけでは同梱ぶんを使った証拠にならない** — `bind_pdfium()` は候補を
//! 全部外すと最後に `bind_to_system_library()` へ落ちるので、システムに libpdfium が
//! 入っている機械では候補が 1 つも当たらなくても成功しうる。そこで成功時は Linux の
//! `/proc/self/maps` から**実際に dlopen されたパス**を出し、呼び出し側（CI）が
//! それがバンドル配下かどうかを検査できるようにする。
//!
//! 出力は 1 行 1 事実の機械可読形式で、終了コードは bind の成否と一致する。
//!
//! ```text
//! cargo build --release --example pdfium_probe
//! ```

use std::process::ExitCode;

/// dlopen 済みの pdfium の実パスを `/proc/self/maps` から拾う（Linux のみ・重複は畳む）。
///
/// bind に使ったバインディングが生きている間に読むこと（`Box<dyn PdfiumLibraryBindings>` を
/// drop するとライブラリが閉じてマップから消えうる）。
fn loaded_pdfium_paths() -> Vec<String> {
    let maps = match std::fs::read_to_string("/proc/self/maps") {
        Ok(s) => s,
        Err(_) => return Vec::new(), // Linux 以外 / 読めない環境では黙って空
    };
    let mut out: Vec<String> = Vec::new();
    for line in maps.lines() {
        // 形式: `addr perms offset dev inode  path`（path は空のこともある）
        let Some(idx) = line.find('/') else { continue };
        let path = line[idx..].trim();
        if !path.contains("libpdfium") {
            continue;
        }
        if !out.iter().any(|p| p == path) {
            out.push(path.to_string());
        }
    }
    out
}

fn main() -> ExitCode {
    // 診断のために「どこから」「どこで」走ったかを先に出す。exe の位置が探索候補を決め、
    // cwd は dev 用フォールバック候補（`pdfium` と `.`）の意味を決める。
    println!(
        "probe: current_exe={}",
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| format!("<unavailable: {e}>"))
    );
    println!(
        "probe: cwd={}",
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| format!("<unavailable: {e}>"))
    );

    match lumencite_lib::ingestion::pdf::pdfium::bind_pdfium() {
        Ok(_bindings) => {
            println!("probe: bind=ok");
            let paths = loaded_pdfium_paths();
            if paths.is_empty() {
                // bind は成功したのに実パスが取れない（`/proc` の無い OS など）。
                // CI は `loaded=` が 1 行も無いことを「検証できなかった」として扱う。
                println!("probe: loaded=<unknown>");
            }
            for p in paths {
                println!("probe: loaded={p}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("probe: bind=failed error={e}");
            ExitCode::FAILURE
        }
    }
}
