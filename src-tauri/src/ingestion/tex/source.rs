//! Phase 4: arXiv TeX ソースのコンテナ層。
//!
//! arXiv e-print は「gzip された tar」「gzip された単一 .tex」「（PDF-only 投稿では）PDF」の
//! いずれかで届く。ここでは内容スニッフィングで形式を判定し、**メモリ内でのみ**展開して
//! `.tex` / `.bbl` だけを取り出す。ディスクへ一切書かないため、tar のパストラバーサルや
//! シンボリックリンクの問題は構造的に起きない。展開量は上限でガードする
//! （decompression bomb 対策）。純関数で CI テスト可能。

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// 展開後の合計バイト数の上限（decompression bomb ガード）。
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// 1 ファイルの展開後バイト数の上限。
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// 読み込むファイル数の上限。
const MAX_FILES: usize = 2048;

/// 展開結果。パスは正規化済み（`\` → `/`・先頭 `./` 除去）。値は UTF-8（不正なら latin-1 解釈）。
#[derive(Debug)]
pub struct TexSourceFiles {
    /// 正規化パス → 中身。`.tex` / `.bbl` のみ（図・スタイル等は読まない）。
    pub files: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

/// 添付ファイル（e-print の gzip / tar / 生 .tex）を読み、`.tex`/`.bbl` をメモリへ展開する。
pub fn load_tex_source(path: &Path) -> Result<TexSourceFiles, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("failed to open source: {e}"))?;
    let mut raw = Vec::new();
    // 添付そのもの（圧縮済み）の読み込みにも上限をかける。
    file.take(MAX_TOTAL_BYTES + 1)
        .read_to_end(&mut raw)
        .map_err(|e| format!("failed to read source: {e}"))?;
    if raw.len() as u64 > MAX_TOTAL_BYTES {
        return Err(format!(
            "source file exceeds the {} MiB limit",
            MAX_TOTAL_BYTES / 1024 / 1024
        ));
    }
    let fallback_name = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".to_string());
    load_tex_source_bytes(&raw, &fallback_name)
}

/// バイト列版（テスト・内容スニッフィングの本体）。
pub fn load_tex_source_bytes(raw: &[u8], fallback_name: &str) -> Result<TexSourceFiles, String> {
    if raw.starts_with(b"%PDF-") {
        return Err(
            "this arXiv submission is PDF-only (no TeX source is published); \
             use the PDF attachment instead"
                .to_string(),
        );
    }
    // gzip magic (1f 8b) → 展開してから内側を再判定（tar か単一 .tex か）。
    if raw.len() >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(raw);
        let mut inner = Vec::new();
        // 展開の途中で上限を超えたら打ち切る（bomb ガードは展開「しながら」効かせる）。
        decoder
            .by_ref()
            .take(MAX_TOTAL_BYTES + 1)
            .read_to_end(&mut inner)
            .map_err(|e| format!("failed to decompress gzip: {e}"))?;
        if inner.len() as u64 > MAX_TOTAL_BYTES {
            return Err(format!(
                "decompressed source exceeds the {} MiB limit",
                MAX_TOTAL_BYTES / 1024 / 1024
            ));
        }
        return load_uncompressed(&inner, fallback_name);
    }
    load_uncompressed(raw, fallback_name)
}

/// 非圧縮のバイト列を tar / 単一 .tex として読む。
fn load_uncompressed(data: &[u8], fallback_name: &str) -> Result<TexSourceFiles, String> {
    if is_tar(data) {
        return load_tar(data);
    }
    // 単一ファイル（arXiv の単一 .tex 投稿・手動添付の .tex）。名前は情報用途のみ。
    let name = if fallback_name.to_ascii_lowercase().ends_with(".tex") {
        fallback_name.to_string()
    } else {
        format!("{fallback_name}.tex")
    };
    let mut files = BTreeMap::new();
    files.insert(normalize_path(&name), decode_text(data));
    Ok(TexSourceFiles {
        files,
        warnings: Vec::new(),
    })
}

/// tar 判定（POSIX ustar / GNU tar のマジック。offset 257 に "ustar"）。
fn is_tar(data: &[u8]) -> bool {
    data.len() > 262 && &data[257..262] == b"ustar"
}

/// tar をメモリ内で走査し `.tex` / `.bbl` の通常ファイルだけを取り出す。
/// symlink・ディレクトリ・その他の特殊エントリは読まない（ディスクへ書かないので
/// 脱出系の攻撃は成立しないが、通常ファイル以外を読む理由も無い）。
fn load_tar(data: &[u8]) -> Result<TexSourceFiles, String> {
    let mut archive = tar::Archive::new(data);
    let mut files = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut total: u64 = 0;
    let mut count: usize = 0;

    let entries = archive
        .entries()
        .map_err(|e| format!("failed to read tar: {e}"))?;
    for entry in entries {
        let mut entry = match entry {
            Ok(e) => e,
            Err(e) => {
                // tar は壊れたヘッダの先を読み進められない（イテレータはここで終了する）。
                // 「1 エントリだけスキップした」と偽って部分文書を completed で保存するより、
                // 全体をエラーにして失敗を可視化する。
                return Err(format!("corrupt tar archive: {e}"));
            }
        };
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let path = match entry.path() {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(_) => continue,
        };
        let lower = path.to_ascii_lowercase();
        if !(lower.ends_with(".tex") || lower.ends_with(".bbl") || lower.ends_with(".ltx")) {
            continue;
        }
        count += 1;
        if count > MAX_FILES {
            return Err(format!("source contains more than {MAX_FILES} files"));
        }
        let mut content = Vec::new();
        // ヘッダ申告サイズは信用せず、実読で上限を効かせる。
        if let Err(e) = entry
            .by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut content)
        {
            warnings.push(format!("failed to read '{path}': {e}"));
            continue;
        }
        if content.len() as u64 > MAX_FILE_BYTES {
            warnings.push(format!(
                "skipped '{path}': larger than {} MiB",
                MAX_FILE_BYTES / 1024 / 1024
            ));
            continue;
        }
        total += content.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(format!(
                "extracted source exceeds the {} MiB limit",
                MAX_TOTAL_BYTES / 1024 / 1024
            ));
        }
        // 同名重複（悪意ある/壊れた tar）は最初のエントリを採用し、警告に残す。
        match files.entry(normalize_path(&path)) {
            std::collections::btree_map::Entry::Occupied(e) => warnings.push(format!(
                "duplicate path in archive: '{}' (kept the first)",
                e.key()
            )),
            std::collections::btree_map::Entry::Vacant(e) => {
                e.insert(decode_text(&content));
            }
        }
    }

    if files.is_empty() {
        return Err("no .tex file found in the source archive".to_string());
    }
    Ok(TexSourceFiles { files, warnings })
}

/// パス正規化: `\` → `/`、先頭の `./` を除去。照合（`\input` 解決）を安定させる。
pub(crate) fn normalize_path(p: &str) -> String {
    let p = p.replace('\\', "/");
    p.strip_prefix("./").unwrap_or(&p).to_string()
}

/// テキスト化 + BOM 除去。UTF-8 でなければ latin-1 として解釈する（古い arXiv ソースは
/// latin-1 が多く、lossy 置換だとまさに救いたいバイトが `U+FFFD` に潰れるため）。
fn decode_text(data: &[u8]) -> String {
    let s = match std::str::from_utf8(data) {
        Ok(s) => s.to_string(),
        // latin-1 は各バイトが同値の Unicode コードポイントに 1:1 対応する。
        Err(_) => data.iter().map(|&b| b as char).collect(),
    };
    s.strip_prefix('\u{feff}').unwrap_or(&s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    /// テスト用: (パス, 中身) 群から tar バイト列を作る。
    fn make_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn single_gzipped_tex_loads_with_fallback_name() {
        let gz = gzip(b"\\documentclass{article}\\begin{document}hi\\end{document}");
        let src = load_tex_source_bytes(&gz, "arxiv-2301.00001-source").unwrap();
        assert_eq!(src.files.len(), 1);
        let (name, body) = src.files.iter().next().unwrap();
        assert_eq!(name, "arxiv-2301.00001-source.tex");
        assert!(body.contains("\\documentclass"));
    }

    #[test]
    fn gzipped_tar_keeps_only_tex_and_bbl() {
        let tar = make_tar(&[
            ("main.tex", b"\\documentclass{article}".as_slice()),
            ("./sections/intro.tex", b"Intro".as_slice()),
            ("refs.bbl", b"\\begin{thebibliography}{9}\\end{thebibliography}".as_slice()),
            ("fig1.pdf", b"%PDF- binary".as_slice()),
            ("style.sty", b"\\ProvidesPackage{style}".as_slice()),
        ]);
        let src = load_tex_source_bytes(&gzip(&tar), "x").unwrap();
        let names: Vec<&str> = src.files.keys().map(|s| s.as_str()).collect();
        assert_eq!(names, vec!["main.tex", "refs.bbl", "sections/intro.tex"]);
    }

    /// gzip を介さない生 tar も受ける（経路上で透過展開された場合の保険）。
    #[test]
    fn plain_tar_is_accepted() {
        let tar = make_tar(&[("a.tex", b"A".as_slice())]);
        let src = load_tex_source_bytes(&tar, "x").unwrap();
        assert_eq!(src.files.len(), 1);
        assert_eq!(src.files["a.tex"], "A");
    }

    #[test]
    fn plain_tex_is_accepted_as_single_file() {
        let src = load_tex_source_bytes(b"\\documentclass{article}", "notes.tex").unwrap();
        assert_eq!(src.files.len(), 1);
        assert!(src.files.contains_key("notes.tex"));
    }

    #[test]
    fn pdf_only_submission_is_rejected_with_clear_message() {
        let err = load_tex_source_bytes(b"%PDF-1.5 ...", "x").unwrap_err();
        assert!(err.contains("PDF-only"), "{err}");
    }

    /// gzip bomb: 展開後が上限を超えたらエラー（展開しながら打ち切る）。
    #[test]
    fn decompression_bomb_is_rejected() {
        let huge = vec![0u8; (MAX_TOTAL_BYTES + 1024 * 1024) as usize];
        let gz = gzip(&huge);
        assert!(gz.len() < 1024 * 1024, "test premise: bomb compresses small");
        let err = load_tex_source_bytes(&gz, "x").unwrap_err();
        assert!(err.contains("limit"), "{err}");
    }

    /// symlink エントリは読まない（tar 内に .tex への symlink があっても中身は取らない）。
    #[test]
    fn symlink_entries_are_ignored() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder
            .append_link(&mut header, "evil.tex", "/etc/passwd")
            .unwrap();
        let mut h2 = tar::Header::new_gnu();
        h2.set_size(4);
        h2.set_mode(0o644);
        h2.set_cksum();
        builder.append_data(&mut h2, "ok.tex", b"ok!!".as_slice()).unwrap();
        let tar = builder.into_inner().unwrap();

        let src = load_tex_source_bytes(&tar, "x").unwrap();
        assert_eq!(src.files.len(), 1);
        assert!(src.files.contains_key("ok.tex"));
    }

    #[test]
    fn non_utf8_content_decodes_as_latin1() {
        // latin-1 の "café" (0xE9)。lossy 置換ではなく latin-1 として復元する。
        let tar = make_tar(&[("a.tex", b"caf\xe9".as_slice())]);
        let src = load_tex_source_bytes(&tar, "x").unwrap();
        assert_eq!(src.files["a.tex"], "café");
    }

    #[test]
    fn duplicate_paths_keep_first_entry_with_warning() {
        let tar = make_tar(&[
            ("a.tex", b"first".as_slice()),
            ("a.tex", b"second".as_slice()),
        ]);
        let src = load_tex_source_bytes(&tar, "x").unwrap();
        assert_eq!(src.files["a.tex"], "first");
        assert!(src.warnings.iter().any(|w| w.contains("duplicate")), "{:?}", src.warnings);
    }

    #[test]
    fn ltx_extension_is_accepted() {
        let tar = make_tar(&[("old.ltx", b"\\documentstyle{article}".as_slice())]);
        let src = load_tex_source_bytes(&tar, "x").unwrap();
        assert!(src.files.contains_key("old.ltx"));
    }

    /// レビュー回帰: 壊れた tar ヘッダは「スキップ」できない（イテレータが止まり後続が
    /// 静かに欠落する）ため、全体をエラーにする。
    #[test]
    fn corrupt_tar_entry_is_fatal_not_silent() {
        let mut tar = make_tar(&[("a.tex", b"AAA".as_slice())]);
        // 終端マーカー（ゼロブロック）を壊れたヘッダに差し替える。
        let len = tar.len();
        tar[len - 1024..len - 512].fill(0xFF);
        let err = load_tex_source_bytes(&tar, "x").unwrap_err();
        assert!(err.contains("corrupt tar"), "{err}");
    }
}
