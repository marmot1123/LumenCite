//! URL からの PDF ダウンロード + 添付（Web クリッパー用）。
//!
//! 全体をメモリに読み切ってから（上限あり）検証・保存するので、失敗時に
//! 中途半端なファイルが残らない。`%PDF-` マジックバイトで検証するため、
//! ペイウォールが返す HTML やエラーページは添付されない。

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;

use crate::models::Attachment;

/// ダウンロードの上限。Content-Length は詐称できるため実読で判定する。
#[derive(Debug, Clone, Copy)]
pub struct DownloadCaps {
    pub max_bytes: u64,
    pub timeout: Duration,
}

impl Default for DownloadCaps {
    fn default() -> Self {
        DownloadCaps { max_bytes: 50 * 1024 * 1024, timeout: Duration::from_secs(30) }
    }
}

/// PDF をメモリへダウンロードして検証する。
/// 返り値は `(バイト列, Content-Disposition のファイル名)`。
pub async fn fetch_pdf(url: &str, caps: DownloadCaps) -> Result<(Vec<u8>, Option<String>), String> {
    let client = reqwest::Client::builder()
        .user_agent("LumenCite/0.1 (mailto:support@lumencite.app)")
        .timeout(caps.timeout)
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download failed: {e}"))?;

    let cd_name = resp
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(content_disposition_filename);

    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp;
    loop {
        let chunk = stream.chunk().await.map_err(|e| format!("download failed: {e}"))?;
        let Some(chunk) = chunk else { break };
        if bytes.len() as u64 + chunk.len() as u64 > caps.max_bytes {
            return Err(format!("PDF exceeds the {} MB limit", caps.max_bytes / 1024 / 1024));
        }
        bytes.extend_from_slice(&chunk);
        // 先頭が揃った時点で PDF でなければ以降を読まずに打ち切る
        if bytes.len() >= 5 && !bytes.starts_with(b"%PDF-") {
            return Err("response is not a PDF".to_string());
        }
    }
    if bytes.len() < 5 || !bytes.starts_with(b"%PDF-") {
        return Err("response is not a PDF".to_string());
    }

    Ok((bytes, cd_name))
}

/// PDF を `<app_data_dir>/attachments/<entry_id>/` に保存して DB に登録する。
/// DB 登録に失敗したら保存したファイルは削除する（`add_attachment` コマンドと同じ方針）。
pub async fn download_and_attach(
    pool: &SqlitePool,
    app_data_dir: &Path,
    entry_id: i64,
    url: &str,
    caps: DownloadCaps,
) -> Result<Attachment, String> {
    let (bytes, cd_name) = fetch_pdf(url, caps).await?;

    let entry_dir = app_data_dir.join("attachments").join(entry_id.to_string());
    std::fs::create_dir_all(&entry_dir).map_err(|e| e.to_string())?;

    let file_name = suggested_file_name(url, cd_name.as_deref());
    let dest = unique_dest(&entry_dir, &file_name);
    std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;

    let dest_name = dest.file_name().unwrap_or_default().to_string_lossy().to_string();
    let rel_path = format!("attachments/{}/{}", entry_id, dest_name);

    match crate::db::attachments::add_attachment(
        pool,
        entry_id,
        &rel_path,
        &dest_name,
        "application/pdf",
    )
    .await
    {
        Ok(att) => Ok(att),
        Err(e) => {
            let _ = std::fs::remove_file(&dest);
            Err(e.to_string())
        }
    }
}

/// Content-Disposition の `filename="..."` / `filename=...` を取り出す（簡易版）。
fn content_disposition_filename(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let idx = lower.find("filename=")?;
    let rest = &value[idx + "filename=".len()..];
    let name = rest
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// 保存ファイル名を決める: Content-Disposition → URL 末尾セグメント → "download.pdf"。
/// サニタイズし、拡張子 .pdf を保証する。
fn suggested_file_name(url: &str, cd_name: Option<&str>) -> String {
    let from_url = || {
        url.split(['?', '#'])
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let raw = cd_name
        .map(str::to_string)
        .or_else(from_url)
        .unwrap_or_else(|| "download".to_string());

    let mut name = sanitize_file_name(&raw);
    if name.is_empty() {
        name = "download".to_string();
    }
    if !name.to_ascii_lowercase().ends_with(".pdf") {
        name.push_str(".pdf");
    }
    name
}

/// パス区切り・制御文字・先頭ドットを除去し、長すぎる名前を丸める。
fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.');
    cleaned.chars().take(120).collect()
}

/// `dir` 内で未使用のファイル名を返す（既存なら `_1`, `_2`… を付ける）。
pub(crate) fn unique_dest(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (file_name.to_string(), String::new()),
    };
    for i in 1..1000 {
        let next = dir.join(format!("{stem}_{i}{ext}"));
        if !next.exists() {
            return next;
        }
    }
    dir.join(format!(
        "{stem}_{}{ext}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ))
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;
    use sqlx::SqlitePool;

    // ── pure ──────────────────────────────────────────────────────────────

    #[test]
    fn file_name_derivation_table() {
        assert_eq!(suggested_file_name("https://arxiv.org/pdf/2301.00001", None), "2301.00001.pdf");
        assert_eq!(
            suggested_file_name("https://ex.com/a/paper.pdf?download=1", None),
            "paper.pdf"
        );
        assert_eq!(
            suggested_file_name("https://ex.com/x", Some("Nice Paper.PDF")),
            "Nice Paper.PDF"
        );
        assert_eq!(suggested_file_name("https://ex.com/", None), "download.pdf");
        // パス区切り・制御文字・先頭ドットは除去される
        assert_eq!(suggested_file_name("https://ex.com/x", Some("../../etc/passwd")), "etcpasswd.pdf");
        assert_eq!(suggested_file_name("https://ex.com/x", Some(".hidden")), "hidden.pdf");
    }

    #[test]
    fn content_disposition_parsing() {
        assert_eq!(
            content_disposition_filename(r#"attachment; filename="paper.pdf""#).as_deref(),
            Some("paper.pdf")
        );
        assert_eq!(
            content_disposition_filename("attachment; filename=paper.pdf; size=1").as_deref(),
            Some("paper.pdf")
        );
        assert_eq!(content_disposition_filename("inline"), None);
    }

    // ── 統合（ローカル fixture サーバー） ──────────────────────────────────

    /// 1 リクエストだけ応答して終了する fixture HTTP サーバーを立て、URL を返す。
    fn serve_once(body: Vec<u8>) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            if let Ok(req) = server.recv() {
                let _ = req.respond(tiny_http::Response::from_data(body));
            }
        });
        format!("http://127.0.0.1:{port}/fixture.pdf")
    }

    async fn make_entry(pool: &SqlitePool) -> i64 {
        create_entry(pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        })
        .await
        .unwrap()
        .id
    }

    fn temp_app_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lc-dl-test-{}-{tag}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn download_and_attach_happy_path(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let url = serve_once(b"%PDF-1.4 fake pdf body".to_vec());
        let dir = temp_app_dir("ok");

        let att = download_and_attach(&pool, &dir, entry_id, &url, DownloadCaps::default())
            .await
            .unwrap();

        assert_eq!(att.file_name, "fixture.pdf");
        let stored = dir.join("attachments").join(entry_id.to_string()).join(&att.file_name);
        assert!(stored.exists(), "file exists under attachments/<id>/");
        assert!(std::fs::read(&stored).unwrap().starts_with(b"%PDF-"));
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE entry_id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn download_rejects_non_pdf_and_oversize(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let dir = temp_app_dir("bad");

        // HTML（ペイウォール等）→ マジックバイトで拒否
        let url = serve_once(b"<!doctype html><html>login please</html>".to_vec());
        let err = download_and_attach(&pool, &dir, entry_id, &url, DownloadCaps::default())
            .await
            .unwrap_err();
        assert!(err.contains("not a PDF"), "{err}");

        // 上限超過 → 拒否
        let url = serve_once([b"%PDF-".to_vec(), vec![0u8; 4096]].concat());
        let caps = DownloadCaps { max_bytes: 1024, ..Default::default() };
        let err = download_and_attach(&pool, &dir, entry_id, &url, caps).await.unwrap_err();
        assert!(err.contains("limit"), "{err}");

        // ファイルも DB 行も残っていない
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE entry_id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
        assert!(!dir.join("attachments").join(entry_id.to_string()).exists()
            || std::fs::read_dir(dir.join("attachments").join(entry_id.to_string()))
                .map(|mut d| d.next().is_none())
                .unwrap_or(true));

        std::fs::remove_dir_all(&dir).ok();
    }
}
