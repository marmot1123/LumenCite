//! URL からの PDF ダウンロード + 添付（Web クリッパー用）。
//!
//! 全体をメモリに読み切ってから（上限あり）検証・保存するので、失敗時に
//! 中途半端なファイルが残らない。`%PDF-` マジックバイトで検証するため、
//! ペイウォールが返す HTML やエラーページは添付されない。

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;

use crate::models::Attachment;

/// リダイレクトを手動で追う際の上限。
const MAX_REDIRECTS: usize = 5;

/// SSRF 対策（CR-003）: このアドレスへは接続させない。
/// loopback / private / link-local / unspecified / CGNAT / multicast などを弾く。
fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                // CGNAT 100.64.0.0/10（is_shared は unstable なので手動判定）
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 0x40)
                // 0.0.0.0/8
                || v4.octets()[0] == 0
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return true;
            }
            // IPv4-mapped (::ffff:a.b.c.d) は埋め込み IPv4 で再判定する。
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            // unique local fc00::/7
            (seg[0] & 0xfe00) == 0xfc00
                // link-local fe80::/10
                || (seg[0] & 0xffc0) == 0xfe80
        }
    }
}

/// ホスト名を解決し、公開 IP に紐づく単一の `SocketAddr` を返す。
/// 解決結果に禁止アドレスが 1 つでも含まれれば拒否する（split-horizon / rebinding 対策）。
/// 返した addr へ接続を固定（`.resolve()` でピン留め）することで TOCTOU も避ける。
async fn resolve_public_addr(
    host: &str,
    port: u16,
    allow_private: bool,
) -> Result<SocketAddr, String> {
    let host_owned = host.to_string();
    let addrs: Vec<SocketAddr> = tokio::task::spawn_blocking(move || {
        (host_owned.as_str(), port)
            .to_socket_addrs()
            .map(|it| it.collect::<Vec<_>>())
    })
    .await
    .map_err(|e| format!("dns resolution failed: {e}"))?
    .map_err(|e| format!("dns resolution failed: {e}"))?;

    if addrs.is_empty() {
        return Err("could not resolve host".to_string());
    }
    if !allow_private {
        if let Some(bad) = addrs.iter().find(|a| is_forbidden_ip(a.ip())) {
            return Err(format!("refusing to fetch from non-public address {}", bad.ip()));
        }
    }
    Ok(addrs[0])
}

/// ダウンロードの上限。Content-Length は詐称できるため実読で判定する。
#[derive(Debug, Clone, Copy)]
pub struct DownloadCaps {
    pub max_bytes: u64,
    pub timeout: Duration,
    /// SSRF ガード（CR-003）を無効化して private/loopback へも接続を許すか。
    /// 本番は必ず false。ローカル fixture サーバーを使うテストでのみ true にする。
    pub allow_private_hosts: bool,
}

impl Default for DownloadCaps {
    fn default() -> Self {
        DownloadCaps {
            max_bytes: 50 * 1024 * 1024,
            timeout: Duration::from_secs(30),
            allow_private_hosts: false,
        }
    }
}

/// SSRF ガード付きで URL を取得し、最終応答（非リダイレクト・成功ステータス）を返す共有コア。
///
/// リダイレクトを自動追尾せず、各ホップで scheme（http/https のみ）と解決先 IP
/// （公開アドレスのみ）を検証してから接続する（CR-003）。`fetch_pdf` /
/// `fetch_arxiv_source` が共有し、ペイロード検証は呼び出し側が行う。
async fn fetch_response(url: &str, caps: DownloadCaps) -> Result<reqwest::Response, String> {
    let mut current = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;

    for _ in 0..=MAX_REDIRECTS {
        if !matches!(current.scheme(), "http" | "https") {
            return Err("only http and https URLs are allowed".to_string());
        }
        let host = current
            .host_str()
            .ok_or_else(|| "URL has no host".to_string())?
            .to_string();
        let port = current
            .port_or_known_default()
            .ok_or_else(|| "URL has no port".to_string())?;
        // 解決先を検証し、その addr に接続を固定する。
        let addr = resolve_public_addr(&host, port, caps.allow_private_hosts).await?;

        let client = reqwest::Client::builder()
            .user_agent("LumenCite/0.1 (mailto:support@lumencite.app)")
            .timeout(caps.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .resolve(&host, addr)
            .build()
            .map_err(|e| e.to_string())?;

        let resp = client
            .get(current.clone())
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;

        if resp.status().is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| "redirect without Location".to_string())?;
            // 相対 Location を現在 URL 基準で解決し、次ホップとして再検証する。
            current = current
                .join(location)
                .map_err(|e| format!("invalid redirect target: {e}"))?;
            continue;
        }

        return resp
            .error_for_status()
            .map_err(|e| format!("download failed: {e}"));
    }
    Err("too many redirects".to_string())
}

/// Content-Disposition のファイル名を取り出す（共有）。
fn response_cd_name(resp: &reqwest::Response) -> Option<String> {
    resp.headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(content_disposition_filename)
}

/// PDF をメモリへダウンロードして検証する。
/// 返り値は `(バイト列, Content-Disposition のファイル名)`。
pub async fn fetch_pdf(url: &str, caps: DownloadCaps) -> Result<(Vec<u8>, Option<String>), String> {
    let resp = fetch_response(url, caps).await?;
    let cd_name = response_cd_name(&resp);

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

/// arXiv e-print（TeX ソース）をメモリへダウンロードして検証する（LCIR Phase 4）。
///
/// 正常応答は gzip（tar か単一 .tex）。PDF-only 投稿は arXiv が PDF を返すので、
/// 「TeX ソースが公開されていない」ことをユーザーに分かる形で弾く。HTML（エラーページ等）も
/// 弾く。それ以外は受理し、詳細な形式判定は LCIR ビルド側の内容スニッフィングに任せる。
pub async fn fetch_arxiv_source(
    url: &str,
    caps: DownloadCaps,
) -> Result<Vec<u8>, String> {
    let resp = fetch_response(url, caps).await?;

    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = resp;
    loop {
        let chunk = stream.chunk().await.map_err(|e| format!("download failed: {e}"))?;
        let Some(chunk) = chunk else { break };
        if bytes.len() as u64 + chunk.len() as u64 > caps.max_bytes {
            return Err(format!(
                "source exceeds the {} MB limit",
                caps.max_bytes / 1024 / 1024
            ));
        }
        bytes.extend_from_slice(&chunk);
        // PDF-only 投稿は先頭 5 バイトで確定するので以降を読まない。
        if bytes.len() >= 5 && bytes.starts_with(b"%PDF-") {
            return Err(
                "this arXiv submission is PDF-only (no TeX source is published)".to_string()
            );
        }
    }
    if bytes.is_empty() {
        return Err("empty response".to_string());
    }
    // HTML（エラーページ・ペイウォール等）: gzip / tar / TeX が '<' で始まることはない。
    if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<') {
        return Err("response is not a TeX source archive".to_string());
    }
    Ok(bytes)
}

/// arXiv TeX ソースを `arxiv-<id>-source.gz`・mime `application/gzip` として添付する。
/// 全文索引は行わない（PDF ではない）。LCIR ビルドは呼び出し側が添付成功後に明示実行する。
/// `url` は呼び出し側が組み立てる（本番は `https://arxiv.org/e-print/<id>`・テストは fixture）。
pub async fn download_and_attach_arxiv_source(
    pool: &SqlitePool,
    app_data_dir: &Path,
    entry_id: i64,
    arxiv_id: &str,
    url: &str,
    caps: DownloadCaps,
) -> Result<Attachment, String> {
    let bytes = fetch_arxiv_source(url, caps).await?;

    // 再取得（v1→v2 改訂など）は**既存の TeX ソース添付を上書き**する: 別添付を積むと
    // read 優先順位のタイブレークで古い方が選ばれ続けるうえ、添付 id が変わると
    // supersede チェーンも切れる。同一添付の中身が変われば sha256 → content_key が変わり、
    // 次の build が新版を作って旧版を supersede する（正しい版管理に自然に乗る）。
    let existing: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, file_path FROM attachments
         WHERE entry_id = ? AND mime_type = ? ORDER BY id LIMIT 1",
    )
    .bind(entry_id)
    .bind(crate::ingestion::TEX_SOURCE_MIME)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if let Some((att_id, rel_path)) = existing {
        let abs = app_data_dir.join(&rel_path);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&abs, &bytes).map_err(|e| e.to_string())?;
        return crate::db::attachments::get_attachment(pool, att_id)
            .await
            .map_err(|e| e.to_string());
    }

    let entry_dir = app_data_dir.join("attachments").join(entry_id.to_string());
    std::fs::create_dir_all(&entry_dir).map_err(|e| e.to_string())?;

    // 旧式 ID（hep-th/9901001）の '/' は sanitize で除去される。
    let mut stem = sanitize_file_name(&format!("arxiv-{arxiv_id}-source"));
    if stem.is_empty() {
        stem = "arxiv-source".to_string();
    }
    let (mut file, dest) =
        create_unique_file(&entry_dir, &format!("{stem}.gz")).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        if let Err(e) = file.write_all(&bytes) {
            drop(file);
            let _ = std::fs::remove_file(&dest);
            return Err(e.to_string());
        }
    }
    drop(file);

    let dest_name = dest.file_name().unwrap_or_default().to_string_lossy().to_string();
    let rel_path = format!("attachments/{entry_id}/{dest_name}");

    match crate::db::attachments::add_attachment(
        pool,
        entry_id,
        &rel_path,
        &dest_name,
        crate::ingestion::TEX_SOURCE_MIME,
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
    // 名前を原子的に予約してから書き込む（CR-008）。書き込み失敗時は予約したファイルを残さない。
    let (mut file, dest) = create_unique_file(&entry_dir, &file_name).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        if let Err(e) = file.write_all(&bytes) {
            drop(file);
            let _ = std::fs::remove_file(&dest);
            return Err(e.to_string());
        }
    }
    drop(file);

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
/// 手動添付（`add_attachment`）も通す: 先頭ドット除去により、LCIR アセット用の予約名
/// `.lcir` とユーザーファイルが衝突しないことを構造的に保証する（Phase 8a）。
pub(crate) fn sanitize_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':'))
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.');
    cleaned.chars().take(120).collect()
}

/// `dir` 内で未使用のファイル名を **原子的に予約**し、作成済みの空ファイルとパスを返す（CR-008）。
/// `create_new`（O_CREAT|O_EXCL）で名前を確保するため、並行追加で `exists()` チェックが
/// すり抜けて同名を上書き（1 ファイルを 2 行が共有 / 片方消失）することがない。
pub(crate) fn create_unique_file(
    dir: &Path,
    file_name: &str,
) -> std::io::Result<(std::fs::File, PathBuf)> {
    fn try_create(p: &Path) -> std::io::Result<Option<std::fs::File>> {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(p) {
            Ok(f) => Ok(Some(f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(e) => Err(e),
        }
    }

    let first = dir.join(file_name);
    if let Some(f) = try_create(&first)? {
        return Ok((f, first));
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (file_name.to_string(), String::new()),
    };
    for i in 1..10000 {
        let cand = dir.join(format!("{stem}_{i}{ext}"));
        if let Some(f) = try_create(&cand)? {
            return Ok((f, cand));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not find a free attachment file name",
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

    // ── SSRF ガード（CR-003） ──────────────────────────────────────────────

    #[test]
    fn forbidden_ip_classifies_private_and_public() {
        use std::net::{Ipv4Addr, Ipv6Addr};
        let bad = [
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),   // loopback
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),    // private
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), // private
            IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1)), // link-local
            IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),     // unspecified/0/8
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),  // CGNAT
            IpAddr::V6(Ipv6Addr::LOCALHOST),           // ::1
            IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)), // link-local
            IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1)), // unique local
        ];
        for ip in bad {
            assert!(is_forbidden_ip(ip), "{ip} should be forbidden");
        }
        let good = [
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
            IpAddr::V6(Ipv6Addr::new(0x2606, 0x2800, 0x220, 1, 0, 0, 0, 1)),
        ];
        for ip in good {
            assert!(!is_forbidden_ip(ip), "{ip} should be allowed");
        }
    }

    #[tokio::test]
    async fn fetch_pdf_rejects_non_http_scheme() {
        let err = fetch_pdf("file:///etc/passwd", DownloadCaps::default()).await.unwrap_err();
        assert!(err.contains("http"), "{err}");
        let err = fetch_pdf("ftp://example.com/x.pdf", DownloadCaps::default()).await.unwrap_err();
        assert!(err.contains("http"), "{err}");
    }

    #[tokio::test]
    async fn fetch_pdf_rejects_loopback_without_flag() {
        // ガード有効（既定）なら loopback への接続は解決段階で拒否される。
        let err = fetch_pdf("http://127.0.0.1:9/x.pdf", DownloadCaps::default()).await.unwrap_err();
        assert!(err.contains("non-public"), "{err}");
    }

    // ── 原子的な名前予約（CR-008） ────────────────────────────────────────

    #[test]
    fn create_unique_file_reserves_distinct_names() {
        let dir = temp_app_dir("uniq");
        std::fs::create_dir_all(&dir).unwrap();

        let (_f1, p1) = create_unique_file(&dir, "paper.pdf").unwrap();
        let (_f2, p2) = create_unique_file(&dir, "paper.pdf").unwrap();
        let (_f3, p3) = create_unique_file(&dir, "paper.pdf").unwrap();

        // 3 回とも別パスが予約され、全て実在する。
        assert_ne!(p1, p2);
        assert_ne!(p2, p3);
        assert_ne!(p1, p3);
        assert!(p1.exists() && p2.exists() && p3.exists());
        assert_eq!(p1.file_name().unwrap(), "paper.pdf");
        assert_eq!(p2.file_name().unwrap(), "paper_1.pdf");

        std::fs::remove_dir_all(&dir).ok();
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

    /// ローカル fixture サーバー（loopback）へ接続するため SSRF ガードを外した caps。
    fn test_caps() -> DownloadCaps {
        DownloadCaps { allow_private_hosts: true, ..Default::default() }
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

        let att = download_and_attach(&pool, &dir, entry_id, &url, test_caps())
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

    // ── arXiv TeX ソース（LCIR Phase 4） ──────────────────────────────────

    #[sqlx::test(migrations = "./migrations")]
    async fn arxiv_source_happy_path_attaches_gzip(pool: SqlitePool) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let entry_id = make_entry(&pool).await;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"\\documentclass{article}\\begin{document}x\\end{document}").unwrap();
        let url = serve_once(enc.finish().unwrap());
        let dir = temp_app_dir("texsrc");

        let att = download_and_attach_arxiv_source(
            &pool, &dir, entry_id, "2301.00001", &url, test_caps(),
        )
        .await
        .unwrap();

        assert_eq!(att.file_name, "arxiv-2301.00001-source.gz");
        assert_eq!(att.mime_type, crate::ingestion::TEX_SOURCE_MIME);
        let stored = dir.join("attachments").join(entry_id.to_string()).join(&att.file_name);
        assert!(stored.exists());
        assert!(std::fs::read(&stored).unwrap().starts_with(&[0x1f, 0x8b]));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// PDF-only 投稿（e-print が PDF を返す）は「TeX 未公開」の明示エラーで、ファイルも行も残らない。
    #[sqlx::test(migrations = "./migrations")]
    async fn arxiv_source_rejects_pdf_only_and_html(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let dir = temp_app_dir("texsrc-bad");

        let url = serve_once(b"%PDF-1.5 pdf-only submission".to_vec());
        let err = download_and_attach_arxiv_source(
            &pool, &dir, entry_id, "old/9901001", &url, test_caps(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("PDF-only"), "{err}");

        let url = serve_once(b"<!doctype html><html>error</html>".to_vec());
        let err = download_and_attach_arxiv_source(
            &pool, &dir, entry_id, "old/9901001", &url, test_caps(),
        )
        .await
        .unwrap_err();
        assert!(err.contains("not a TeX source"), "{err}");

        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments WHERE entry_id = ?")
            .bind(entry_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 再取得は別添付を積まず、既存の TeX ソース添付の中身を上書きする（レビュー回帰:
    /// 重複添付だと read 優先順位が古い方を選び続ける）。
    #[sqlx::test(migrations = "./migrations")]
    async fn arxiv_source_refetch_overwrites_existing_attachment(pool: SqlitePool) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let entry_id = make_entry(&pool).await;
        let dir = temp_app_dir("texsrc-refetch");
        let gz = |body: &[u8]| {
            let mut enc = GzEncoder::new(Vec::new(), Compression::default());
            enc.write_all(body).unwrap();
            enc.finish().unwrap()
        };

        let url1 = serve_once(gz(b"v1 content"));
        let first = download_and_attach_arxiv_source(
            &pool, &dir, entry_id, "2301.00001", &url1, test_caps(),
        )
        .await
        .unwrap();

        let url2 = serve_once(gz(b"v2 content revised"));
        let second = download_and_attach_arxiv_source(
            &pool, &dir, entry_id, "2301.00001", &url2, test_caps(),
        )
        .await
        .unwrap();

        assert_eq!(first.id, second.id, "同一添付行を再利用する");
        let rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM attachments WHERE entry_id = ? AND mime_type = ?",
        )
        .bind(entry_id)
        .bind(crate::ingestion::TEX_SOURCE_MIME)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 1, "TeX ソース添付は 1 行のまま");

        // 中身は v2 に置き換わっている。
        let stored = dir.join("attachments").join(entry_id.to_string()).join(&first.file_name);
        let mut dec = flate2::read::GzDecoder::new(std::fs::File::open(&stored).unwrap());
        let mut body = String::new();
        std::io::Read::read_to_string(&mut dec, &mut body).unwrap();
        assert_eq!(body, "v2 content revised");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 旧式 ID（hep-th/9901001）の '/' はファイル名から除去される。
    #[sqlx::test(migrations = "./migrations")]
    async fn arxiv_source_sanitizes_old_style_ids(pool: SqlitePool) {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let entry_id = make_entry(&pool).await;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"x").unwrap();
        let url = serve_once(enc.finish().unwrap());
        let dir = temp_app_dir("texsrc-old");

        let att = download_and_attach_arxiv_source(
            &pool, &dir, entry_id, "hep-th/9901001", &url, test_caps(),
        )
        .await
        .unwrap();
        assert_eq!(att.file_name, "arxiv-hep-th9901001-source.gz");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn download_rejects_non_pdf_and_oversize(pool: SqlitePool) {
        let entry_id = make_entry(&pool).await;
        let dir = temp_app_dir("bad");

        // HTML（ペイウォール等）→ マジックバイトで拒否
        let url = serve_once(b"<!doctype html><html>login please</html>".to_vec());
        let err = download_and_attach(&pool, &dir, entry_id, &url, test_caps())
            .await
            .unwrap_err();
        assert!(err.contains("not a PDF"), "{err}");

        // 上限超過 → 拒否
        let url = serve_once([b"%PDF-".to_vec(), vec![0u8; 4096]].concat());
        let caps = DownloadCaps { max_bytes: 1024, ..test_caps() };
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
