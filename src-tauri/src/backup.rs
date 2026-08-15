//! DB バックアップ（CR-018: 添付本体込みの完全バックアップ）。
//! - SQLite の `VACUUM INTO` を使って読み取り中でもロックを取らずに DB のクリーンコピーを作り、
//!   添付本体（`<app_data_dir>/attachments/`）とあわせて単一の `.zip` に束ねる。
//! - 保管先は `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.zip`。
//!   アーカイブ内レイアウトは `db.sqlite` ＋ `attachments/<entry_id>/<file_name>`
//!   ＋（走査中に消えたエントリがあったときだけ）`SKIPPED.txt`。
//! - 直近 `keep` 世代のみ残し、それより古いものは削除する（旧 `.db` バックアップも対象）。
//!
//! 作業ファイルは全て「完成前は拾われない名前」で書く:
//! - VACUUM INTO の中間 DB … `.vacuum-<stem>.db.tmp`
//! - 書き込み中のアーカイブ … `<stem>.zip.partial`（完成時に `<stem>.zip` へ rename）
//!
//! これらはプロセスが途中で殺されると残るため、起動時に [`sweep_backup_workdir`] で回収する
//! （中間 DB は DB と同サイズ = 数百 MB あり、放置するとディスクを食い潰す）。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::Local;
use sqlx::SqlitePool;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// 既定の保持世代数。起動時 / 24h 自動バックアップと手動実行で共有する。
pub const DEFAULT_KEEP: usize = 14;

/// 自動バックアップの最小間隔（秒）。前回成功からこれ未満なら起動時バックアップは走らない。
/// 起動のたびにフルバックアップ（VACUUM + zip）を走らせると、ライブラリが育つほど
/// 起動直後の数分間ディスクと CPU を占有し、MCP サーバーの起動や初期描画を巻き添えにする。
pub const AUTO_INTERVAL_SECS: i64 = 24 * 60 * 60;

/// バックアップ対象ファイルか判定する。
/// 完全バックアップは `.zip`。旧世代の DB-only バックアップ（`.db`）も
/// 一覧表示・世代管理の対象に含める（放置すると prune されず溜まり続けるため）。
/// 書きかけの `.zip.partial` は「バックアップ」ではないので含めない。
fn is_backup_file(name: &str) -> bool {
    name.starts_with("lumencite-") && (name.ends_with(".zip") || name.ends_with(".db"))
}

/// 途中終了で残った作業ファイルか判定する（[`sweep_backup_workdir`] の対象）。
fn is_leftover_work_file(name: &str) -> bool {
    // VACUUM INTO の中間 DB とその journal（`.vacuum-<stem>.db.tmp[-journal]`）
    (name.starts_with(".vacuum-") && name.contains(".db.tmp"))
        // 書き込み途中で死んだアーカイブ
        || (name.starts_with("lumencite-") && name.ends_with(".zip.partial"))
        // 旧 .db バックアップの journal。本体が prune された後もこれだけ取り残される
        // （`is_backup_file` に一致しないので世代管理からも漏れる）。
        || (name.starts_with("lumencite-") && name.ends_with(".db-journal"))
}

#[derive(Debug, serde::Serialize)]
pub struct BackupInfo {
    pub path: String,
    pub file_name: String,
    pub created_at: String,
    pub size_bytes: u64,
}

/// バックアップを直列化するプロセス全体で共有のロック（CR-022）。
/// 自動バックアップ（起動時 + 24h タイマー）と手動実行（`run_backup_now`）が重なると、
/// ①同一秒のファイル名選択が TOCTOU で衝突（VACUUM INTO が「already exists」で失敗）、
/// ②`prune_old_backups` が別実行の作成中ファイルを消す、といった競合が起きる。
/// DB は 1 つなのでモジュール static で足りる。
static BACKUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// バックアップが今走っているか。
///
/// 読み手は 2 つ:
/// 1. v1.0.0-p2 の LCIR 起動時バックフィル（`lib.rs`・`should_stop`）
/// 2. `ingestion::gc_stale_asset_dirs`（②b の W1-5 で配線・**crop を消す側**）
///
/// フル zip は `attachments/` を丸ごと束ねて実測 9 分かかる。その最中に LCIR build が完了して
/// `gc_stale_asset_dirs` が旧 content_key ディレクトリを trash へ送ると、**アーカイブは
/// 「`assets` 行はあるがファイルが無い」状態で固まる** ── 復元すると `heal_missing_assets` が
/// 全ページ再抽出に化ける。
///
/// ⚠ **この節は以前「課金済み alt text の carry が無言で外れる」と書いていたが、それは
/// debt-16 を解消する前（`b69863d` 以前）の話。** 今は `db::assets::refresh_asset_file` が
/// 指紋の付け替えを同一 tx で行うので、説明が黙って外れることは無い。残るのは
/// **再抽出のコスト**と、領域数がずれたときに説明が別の絵に付く危険（debt-20）。
///
/// **状態を二重に持たない。** 実行中フラグを別に立てると「立て忘れ」という壊し方が生まれ、
/// しかもそれを単体テストで検出できない（実行中の一瞬を外から観測する必要があるため）。
/// [`BACKUP_LOCK`] を握っていること**が**実行中の定義なので、そのまま覗く。
/// `try_lock` は待ち行列に並ばないので、待っている実行から permit を奪うことはない。
pub fn is_running() -> bool {
    BACKUP_LOCK.try_lock().is_err()
}

/// テスト専用: 「バックアップ実行中」を本物の [`BACKUP_LOCK`] で再現する。
///
/// 別モジュールのテスト（`ingestion` の GC 配線）が `is_running()` の **true 側**を
/// 観測するために使う。⚠ **false 側はどのテストの支配下にも無い**（同じ static を
/// 他のバックアップテストが並列に取りうる）ので、握っている間だけを assert すること。
#[cfg(test)]
pub(crate) async fn hold_lock_for_test() -> tokio::sync::MutexGuard<'static, ()> {
    BACKUP_LOCK.lock().await
}

/// バックアップを実行する（手動実行 = 常に走る）。
pub async fn run_backup(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
) -> Result<PathBuf, String> {
    // ファイル名選択 → VACUUM INTO → zip → prune を他のバックアップと直列化する（CR-022）。
    let _guard = BACKUP_LOCK.lock().await;
    run_backup_inner(pool, app_dir, keep).await
}

/// 前回成功から `min_interval_secs` 以上経っているときだけバックアップする（自動実行用）。
/// 間引かれた場合は `Ok(None)`。
///
/// 判定はバックアップ本体と同じロックの内側で行う。外で判定すると、
/// 同時に走った 2 本が両方「due」と判断してフルバックアップを二重に走らせてしまう。
pub async fn run_backup_if_due(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
    min_interval_secs: i64,
) -> Result<Option<PathBuf>, String> {
    let _guard = BACKUP_LOCK.lock().await;
    if !is_backup_due(pool, min_interval_secs).await {
        return Ok(None);
    }
    run_backup_inner(pool, app_dir, keep).await.map(Some)
}

/// 前回成功時刻の記録を見て、自動バックアップを走らせるべきか判定する。
/// 記録が無い（初回・旧版からの更新直後）／読めない場合は「走らせる」側に倒す。
async fn is_backup_due(pool: &SqlitePool, min_interval_secs: i64) -> bool {
    let last = crate::db::settings::get_setting(pool, crate::db::settings::BACKUP_LAST_RUN_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|raw| chrono::DateTime::parse_from_rfc3339(&raw).ok());
    let Some(last) = last else {
        return true;
    };
    let elapsed = (Local::now() - last.with_timezone(&Local)).num_seconds();
    // 未来の時刻が記録されている（時計のずれ・タイムゾーン変更）ときも走らせる。
    // そうしないと記録が過去に戻るまでバックアップが永久に止まる。
    elapsed >= min_interval_secs || elapsed < 0
}

async fn run_backup_inner(
    pool: &SqlitePool,
    app_dir: &Path,
    keep: usize,
) -> Result<PathBuf, String> {
    let backups_dir = app_dir.join("backups");
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let mut stem = format!("lumencite-{}", timestamp);
    let mut target = backups_dir.join(format!("{}.zip", stem));
    let mut partial = backups_dir.join(format!("{}.zip.partial", stem));
    // タイムスタンプは秒精度なので、同一秒内の連続実行ではファイル名が衝突する。
    // 接尾辞で一意化する（アーカイブ本体・書きかけ・VACUUM 一時ファイルの全てに使う）。
    let mut n = 1usize;
    while target.exists() || partial.exists() {
        stem = format!("lumencite-{}-{}", timestamp, n);
        target = backups_dir.join(format!("{}.zip", stem));
        partial = backups_dir.join(format!("{}.zip.partial", stem));
        n += 1;
    }

    // VACUUM INTO は既存ファイルへは書けないので、まず一時 DB ファイルに吐き出してから
    // zip に格納する。一時ファイルは `lumencite-` 前缀を避け、is_backup_file に拾われないようにする。
    let tmp_db = backups_dir.join(format!(".vacuum-{}.db.tmp", stem));
    let _ = fs::remove_file(&tmp_db); // 前回異常終了の残骸があれば掃除

    let build = async {
        // VACUUM INTO は通常のクエリと違ってトランザクション内で実行できないので
        // SQL リテラルとしてパスを直接埋め込む。シングルクォートをエスケープしておく。
        let tmp_str = tmp_db.to_string_lossy().replace('\'', "''");
        let sql = format!("VACUUM INTO '{}'", tmp_str);
        sqlx::query(&sql)
            .execute(pool)
            .await
            .map_err(|e| format!("VACUUM INTO failed: {}", e))?;

        // zip 書き出しは同期 I/O ＋ CPU バウンド（deflate）で、ライブラリの規模によっては
        // 分単位かかる。async タスク内で直接回すと tokio のワーカーを丸ごと占有するので
        // blocking プールへ逃がす。
        let (dst, src, att) = (
            partial.clone(),
            tmp_db.clone(),
            app_dir.join("attachments"),
        );
        let skipped = tokio::task::spawn_blocking(move || write_archive(&dst, &src, &att))
            .await
            .map_err(|e| format!("archive task failed: {}", e))?
            .map_err(|e| format!("archive write failed: {}", e))?;

        // 完成してから正式名に付け替える。途中で殺されても `.zip.partial` が残るだけで、
        // 中身の欠けたアーカイブが「バックアップ」として一覧・世代管理に混ざらない。
        fs::rename(&partial, &target).map_err(|e| format!("archive rename failed: {}", e))?;
        Ok::<SkippedEntries, String>(skipped)
    };

    let result = build.await;
    // 一時ファイルは成功・失敗どちらでも掃除する。
    let _ = fs::remove_file(&tmp_db);
    let skipped = match result {
        Ok(s) => s,
        Err(e) => {
            // 途中失敗した壊れかけのアーカイブを残さない。
            let _ = fs::remove_file(&partial);
            return Err(e);
        }
    };
    if skipped.count > 0 {
        // 「静かに欠けた成功」を成功と同じ見た目にしない。自動バックアップは UI に
        // 何も出さないので、ここのログとアーカイブ内の `SKIPPED.txt` が唯一の手掛かりになる。
        // 一覧そのものは `SKIPPED.txt` にあるので、ログには先頭数件だけ出す。
        eprintln!(
            "backup: {} entry/entries vanished during the archive walk and were skipped \
             (full list in {} inside {}): {}",
            skipped.count,
            SKIPPED_MANIFEST,
            target.display(),
            skipped
                .names
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // 次回の自動バックアップの間引き判定に使う。成功時のみ記録するので、
    // 失敗し続ける限りは毎回リトライされる。
    if let Err(e) = crate::db::settings::set_setting(
        pool,
        crate::db::settings::BACKUP_LAST_RUN_KEY,
        &Local::now().to_rfc3339(),
    )
    .await
    {
        eprintln!("backup: failed to record last run time: {e}");
    }

    prune_old_backups(&backups_dir, keep).map_err(|e| e.to_string())?;

    Ok(target)
}

/// 作業ファイルを「前回の残骸」とみなすまでの猶予。
/// ゲート②c C-01 以降、同一 app data dir の第2インスタンスは起動時に終了するが、
/// GUI ロックが使えない環境（flock 非対応 FS）では依然 2 個目が起動しうるので、
/// 別インスタンスが今まさに書いている作業ファイルを消さないよう、更新から間を置く。
const WORK_FILE_STALE_SECS: u64 = 60 * 60;

/// 前回の異常終了で残った作業ファイルを回収し、世代数も整える（起動時に 1 回）。
/// 戻り値は削除したファイル数。
///
/// `prune_old_backups` はバックアップ成功時にしか走らないため、途中終了が続くと
/// 世代が `keep` を超えたまま溜まる。ここでも 1 回かけておく。
pub fn sweep_backup_workdir(app_dir: &Path, keep: usize) -> usize {
    let backups_dir = app_dir.join("backups");
    let Ok(rd) = fs::read_dir(&backups_dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut removed = 0;
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_leftover_work_file(&name) {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age.as_secs() >= WORK_FILE_STALE_SECS);
        if stale && fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    let _ = prune_old_backups(&backups_dir, keep);
    removed
}

/// 走査中に消えていて archive に入れられなかったエントリの記録（②b W2-5）。
///
/// zip 書き出しは実ライブラリで **7〜9 分**かかり、その間に添付の削除・LCIR の
/// `gc_stale_asset_dirs`・GC の trash 送りが普通に走る。「あるはず」の先が消えているのは
/// **異常ではなく通常のレース**なので、バックアップ全体を失敗させない。
/// ただし黙って飛ばすと **「1 枚も落ちなかった成功」と「静かに欠けた成功」が同じ見た目**になる。
#[derive(Debug, Default)]
struct SkippedEntries {
    /// 飛ばした総数。
    count: usize,
    /// 記録に残すアーカイブ内パス（先頭 [`SKIPPED_SAMPLE_MAX`] 件）。
    names: Vec<String>,
}

/// 記録に残すパスの上限。走査中に添付ツリーごと消えると件数は数千に達しうるので、
/// 総数は数え続けたまま一覧だけ頭打ちにする。
const SKIPPED_SAMPLE_MAX: usize = 200;

/// 飛ばした一覧をアーカイブ内に残すエントリ名。
///
/// **復元は「`db.sqlite` か `attachments/` 配下」だけを許可する allowlist**（`restore.rs` の
/// `safe_archive_path`）なので、このエントリは復元時に無視される ＝ 展開先を汚さない。
/// ⚠ **自動で読むものは 1 つも無い。** 復元時に警告を出したいなら別途配線が要る（debt-45）。
pub(crate) const SKIPPED_MANIFEST: &str = "SKIPPED.txt";

/// 消えても報告しない作業ファイルか。
///
/// LCIR の crop は `write_atomic` が `<name>.png.tmp` を書いてから rename する
/// （`ingestion/pdf/mod.rs`）。つまり**バックアップ中に build が走れば `.tmp` は正常動作として
/// 消える**。これを数えると `SKIPPED.txt` が build のたびに出て、
/// **唯一の欠損シグナルが恒常的に狼少年になる**。消える前に見えていれば zip には入るので、
/// 「入らなかったのに黙っている」ことにはならない。
fn is_transient_work_file(zip_path: &str) -> bool {
    zip_path.ends_with(".tmp")
}

impl SkippedEntries {
    fn note(&mut self, zip_path: &str, err: &io::Error) {
        self.count += 1;
        if self.names.len() < SKIPPED_SAMPLE_MAX {
            self.names.push(format!("{zip_path}\t({err})"));
        }
    }

    /// アーカイブに同梱する報告本文。
    ///
    /// 「消えた（vanished）」と断定しない ── ディレクトリごと消えた場合も 1 件で数えるし、
    /// 壊れた symlink や特殊ファイルも同じ枝に落ちる。言えるのは
    /// **「アーカイブに入れられなかった」**ことだけ。
    fn report(&self) -> String {
        let mut s = format!(
            "LumenCite backup: {} entry/entries could not be archived (they were removed or \
             became unreadable while the archive was being written) and are NOT included \
             in this backup.\n\
             このバックアップには、書き出し中に読めなくなった {} 件のエントリが\
             含まれていません。\n\n",
            self.count, self.count
        );
        for n in &self.names {
            s.push_str(n);
            s.push('\n');
        }
        if self.count > self.names.len() {
            s.push_str(&format!("... and {} more\n", self.count - self.names.len()));
        }
        s
    }
}

/// 「走査中に消えていたら飛ばす、それ以外は失敗」を適用する**唯一の判定点**。
///
/// `NotFound` だけを飛ばす。ここを広げると、容量不足や権限エラーで**中身の欠けた
/// アーカイブが「成功」として一覧に並ぶ**。
///
/// ⚠ **`NotFound` が FS 由来であることは、この関数では保証していない。**
/// `ZipError` も `io::Error` へ変換されると `NotFound` になりうる
/// （`ZipError::FileNotFound` と、内側の kind をそのまま通す `ZipError::Io`）。
/// 今それが起きないのは変換の性質ではなく、**`FileNotFound` を返すのは `abort_file` と
/// `shallow_copy_file` / `deep_copy_file` だけで、このモジュールはどれも呼んでいない**から。
/// そのどれかを使うようになったら、ここの分類を必ず見直すこと。
fn skip_if_vanished(
    res: io::Result<()>,
    zip_path: &str,
    skipped: &mut SkippedEntries,
) -> io::Result<()> {
    match res {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if !is_transient_work_file(zip_path) {
                skipped.note(zip_path, &e);
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// `db.sqlite` ＋ `attachments/…` を単一 zip に書き出す。
/// 戻り値は走査中に消えて飛ばしたエントリの記録（[`SkippedEntries`]）。
fn write_archive(
    target: &Path,
    db_file: &Path,
    attachments_dir: &Path,
) -> io::Result<SkippedEntries> {
    // ZipWriter は圧縮のたびに小さく write するので、素の File だと syscall が支配的になる。
    let file = io::BufWriter::with_capacity(1 << 20, fs::File::create(target)?);
    let mut zip = zip::ZipWriter::new(file);
    // 添付は PDF / PNG で「既に圧縮済みだから deflate は無駄」と思いがちだが、実ライブラリで
    // 測ると PNG（LCIR の図 crop）が 64%、PDF が 86% まで縮む。無圧縮にすると 1 世代あたり
    // 200MB 以上・14 世代で 3GB 以上増えるので、全体を deflate で統一する。
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file("db.sqlite", opts)?;
    let mut db = fs::File::open(db_file)?;
    io::copy(&mut db, &mut zip)?;

    let mut skipped = SkippedEntries::default();
    // `attachments/` が最初から無いのは通常（添付ゼロのライブラリ）なので記録しない。
    // 「あると確かめてから読めなくなった」だけを飛ばした扱いにする。
    if attachments_dir.is_dir() {
        skip_if_vanished(
            add_dir_recursive(&mut zip, attachments_dir, "attachments", opts, &mut skipped),
            "attachments",
            &mut skipped,
        )?;
    }

    if skipped.count > 0 {
        // 報告はアーカイブの中に置く。stderr のログは復元する時点では残っていない。
        zip.start_file(SKIPPED_MANIFEST, opts)?;
        zip.write_all(skipped.report().as_bytes())?;
    }

    zip.finish()?.flush()?;
    Ok(skipped)
}

/// `dir` 以下を再帰的に zip へ追加する。アーカイブ内パスは `prefix` からの `/` 区切り。
fn add_dir_recursive<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    dir: &Path,
    prefix: &str,
    opts: SimpleFileOptions,
    skipped: &mut SkippedEntries,
) -> io::Result<()> {
    // 決定的な順序で走査する（テスト容易性と差分の安定のため）。
    let mut entries = Vec::new();
    for e in fs::read_dir(dir)? {
        match e {
            Ok(e) => entries.push(e),
            // 反復自体が失敗した ＝ 名前すら分からないエントリ。旧コードは `filter_map(ok)` で
            // 黙って落としていた。**判定点を増やさない**ため、名前をプレースホルダにして
            // 同じ `skip_if_vanished` へ通す（＝ NotFound 以外はここでも全体を失敗させる）。
            Err(err) => skip_if_vanished(
                Err(err),
                &format!("{prefix}/<unreadable directory entry>"),
                skipped,
            )?,
        }
    }
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let zip_path = format!("{}/{}", prefix, name);
        // 消えたかどうかの分類は [`skip_if_vanished`] 1 か所に集める。
        // ここで分岐ごとに判定を書くと、どれか 1 つを壊してもテストが落ちない冗長になる。
        let step = archive_one(zip, &path, &zip_path, opts, skipped);
        skip_if_vanished(step, &zip_path, skipped)?;
    }
    Ok(())
}

/// 1 エントリ（ディレクトリ or ファイル）を zip へ足す。**この関数は判定を持たない** ──
/// 消えていたかどうかの分類は呼び出し側（[`skip_if_vanished`]）が行う。
fn archive_one<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &Path,
    zip_path: &str,
    opts: SimpleFileOptions,
    skipped: &mut SkippedEntries,
) -> io::Result<()> {
    if path.is_dir() {
        return add_dir_recursive(zip, path, zip_path, opts, skipped);
    }
    if !path.is_file() {
        // ディレクトリでもファイルでもない。**なぜそう見えたのかを取り直す。**
        // `Path::is_dir()` / `is_file()` は **stat の失敗を全部 `false` に潰す**ので、
        // ここで合成した `NotFound` を返すと、親ディレクトリに `x` が無いだけの
        // （＝今もディスクに在る）ファイルを「消えました」と報告してしまう。
        // ⚠ `symlink_metadata` はリンク自身を見るので、**壊れた symlink は `Ok`** に落ちる
        // ── 走査中に消えたファイルと同じ扱い（どちらも中身を持てない）で正しい。
        return Err(match fs::symlink_metadata(path) {
            // stat は通ったのに dir でも file でもない = 壊れた symlink・FIFO・ソケット。
            // 呼び出し側に 1 種類の形で渡すため NotFound にする。
            Ok(_) => io::Error::new(
                io::ErrorKind::NotFound,
                "not a regular file or directory",
            ),
            // ENOENT はそのまま「消えた」。EACCES / EIO などは**本物の kind のまま**返して
            // バックアップ全体を失敗させる（親を辿れる `chmod 000` のファイルが
            // 既にそうなっているのと揃える）。
            Err(e) => e,
        });
    }
    // **開いた結果を渡す**（ここでは開くだけ・判定しない）。
    write_file_entry(zip, fs::File::open(path), zip_path, opts)
}

/// 開いたファイルを zip の 1 エントリとして書く。
///
/// **`opened` が成功してから `start_file` する。** 逆順にすると、開けなかったファイルの
/// 空エントリが zip に残り「0 バイトで存在する」という別種の嘘になる ── しかも
/// `SKIPPED.txt` には「含まれていません」と載るので、アーカイブが自分と矛盾する。
///
/// この順序が効くのは「`is_file()` が true を返した直後に消える」レースだけで、
/// FS では決定論的に組めない。**開いた結果を引数で受ける**ことで、その 1 点だけを
/// テストから撮れるようにしてある（[`is_transient_work_file`] と同じく、判定を
/// アダプタ層から引き剥がす形）。
fn write_file_entry<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    opened: io::Result<fs::File>,
    zip_path: &str,
    opts: SimpleFileOptions,
) -> io::Result<()> {
    let mut f = opened?;
    zip.start_file(zip_path, opts)?;
    io::copy(&mut f, zip)?;
    Ok(())
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
            if !is_backup_file(&name) {
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
            if !is_backup_file(&name) {
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
    use std::io::Read;

    /// `is_running` は**バックアップのロックそのもの**を見る（別フラグを持たない）。
    ///
    /// フラグを二重に持つと「立て忘れ」という壊し方が生まれ、しかも実行中の一瞬を外から
    /// 観測しないと検出できない ＝ 単体テストで守れない。ロックを定義そのものにすれば、
    /// `run_backup` / `run_backup_if_due` が取る限り自動的に真になる。
    /// これを読むのは v1.0.0-p2 の LCIR バックフィル（zip 中は譲る）。
    /// ⚠ **「走っていなければ false」は assert しない。** `BACKUP_LOCK` はプロセス全体で
    /// 共有の static で、同じファイルの他のバックアップテストが並列に取りうる。false 側は
    /// このテストの制御下に無く、assert すると CI で不定期に落ちる（実際に落とした）。
    /// 自分が握っている間だけが、このテストが観測を保証できる唯一の状態。
    #[tokio::test]
    async fn is_running_reflects_the_backup_lock() {
        let guard = BACKUP_LOCK.lock().await;
        assert!(is_running(), "ロックを握っている間は true");
        drop(guard);
    }

    /// **「飛ばす」か「バックアップごと失敗する」かの判定はここ 1 か所**（②b W2-5）。
    /// `NotFound` だけを飛ばす ── 広げると容量不足や権限エラーで中身の欠けた
    /// アーカイブが「成功」として一覧に並ぶ。
    #[test]
    fn only_a_vanished_entry_is_skipped() {
        let mut skipped = SkippedEntries::default();
        assert!(skip_if_vanished(Ok(()), "attachments/1/a.pdf", &mut skipped).is_ok());
        assert_eq!(skipped.count, 0, "成功は何も記録しない");

        let gone = io::Error::new(io::ErrorKind::NotFound, "gone");
        assert!(
            skip_if_vanished(Err(gone), "attachments/1/b.pdf", &mut skipped).is_ok(),
            "走査中に消えたエントリでバックアップ全体を落とさない"
        );
        assert_eq!(skipped.count, 1);
        assert!(skipped.names[0].starts_with("attachments/1/b.pdf"));

        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        assert!(
            skip_if_vanished(Err(denied), "attachments/1/c.pdf", &mut skipped).is_err(),
            "消えた以外の失敗は握り潰さない（欠けたまま成功させない）"
        );
        assert_eq!(skipped.count, 1, "失敗させた分は skip に数えない");
    }

    /// 走査の途中で消えたエントリは、`is_dir()` にも `is_file()` にも当たらない。
    /// その形を呼び出し側へ **`NotFound` として**返すのが `archive_one` の役目
    /// （分類は持たない）。
    #[test]
    fn archive_one_reports_a_missing_path_as_not_found() {
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut skipped = SkippedEntries::default();
        let missing = std::env::temp_dir().join("lc-backup-no-such-entry-xyzzy");

        let err = archive_one(&mut zip, &missing, "attachments/1/gone.pdf", opts, &mut skipped)
            .expect_err("消えたパスは Err で返る");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(skipped.count, 0, "記録は呼び出し側の仕事");
        // 入れられなかったエントリの**空の枠**を残さない（「0 バイトで存在する」は別種の嘘）。
        let archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        assert_eq!(archive.len(), 0);
    }

    /// **開けなかったファイルの空の枠を zip に残さない。**
    ///
    /// この順序が効くのは「`is_file()` が true の直後に消える」レースだけで FS では組めない。
    /// `write_file_entry` が**開いた結果を引数で受ける**ので、そこだけを撮れる。
    /// 逆順（`start_file` が先）に戻すと、0 バイトのエントリが**完成したアーカイブに残り**、
    /// 同じパスが `SKIPPED.txt` に「含まれていません」と載って自己矛盾する。
    #[test]
    fn a_file_that_could_not_be_opened_leaves_no_empty_frame() {
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

        let err = write_file_entry(
            &mut zip,
            Err(io::Error::new(io::ErrorKind::NotFound, "gone")),
            "attachments/1/gone.pdf",
            opts,
        )
        .expect_err("開けなければ Err");

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
        assert_eq!(archive.len(), 0, "空の枠を残さない");
    }

    /// **stat できなかったファイルを「消えた」と報告しない。**
    ///
    /// 親から `x` を落とすと `read_dir` は名前を返すのに子の `metadata()` が EACCES で落ち、
    /// `is_dir()` / `is_file()` が**どちらも false** になる（レビューで実測された形）。
    /// ここで合成 `NotFound` を返すと、今もディスクに在るファイルが `SKIPPED.txt` に
    /// 「含まれていません」と載る ＝ 唯一の欠損記録が嘘をつく。
    ///
    /// ⚠ **root で走らせると権限が効かない**ので、効いていることを確かめてから assert する
    /// （効いていなければこのテストは何も主張しない ── CI の `ubuntu-22.04` は非 root）。
    #[cfg(unix)]
    #[test]
    fn a_file_we_cannot_stat_is_not_reported_as_vanished() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("lc-backup-perm-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let dir = root.join("locked");
        std::fs::create_dir_all(&dir).unwrap();
        let victim = dir.join("paper.pdf");
        std::fs::write(&victim, b"%PDF-1.7 fake").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o600)).unwrap();

        let enforced = fs::symlink_metadata(&victim).is_err();
        if enforced {
            let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            let mut skipped = SkippedEntries::default();
            let err = archive_one(&mut zip, &victim, "attachments/1/paper.pdf", opts, &mut skipped)
                .expect_err("読めないファイルは Err");
            assert_eq!(
                err.kind(),
                io::ErrorKind::PermissionDenied,
                "stat の失敗を「消えた」に化けさせない（化けると欠けたまま成功する）"
            );
        } else {
            eprintln!("skipped: この環境では権限が効いていない（root?）");
        }

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_dir_all(&root).ok();
    }

    /// `write_atomic` の作業ファイル（`<name>.png.tmp`）が rename で消えるのは
    /// **LCIR build の正常動作**。これを数えると `SKIPPED.txt` が build のたびに出て、
    /// 唯一の欠損シグナルが狼少年になる。飛ばすが**記録はしない**。
    #[test]
    fn a_vanished_work_file_is_skipped_without_crying_wolf() {
        let mut skipped = SkippedEntries::default();
        let gone = io::Error::new(io::ErrorKind::NotFound, "renamed away");
        assert!(skip_if_vanished(
            Err(gone),
            "attachments/1/.lcir/7/abcd/fig-p001-00.png.tmp",
            &mut skipped
        )
        .is_ok());
        assert_eq!(skipped.count, 0, "作業ファイルの消滅は欠損ではない");

        // 本物の crop が消えたときは今までどおり記録する（規則を広げすぎない）。
        let gone = io::Error::new(io::ErrorKind::NotFound, "gone");
        assert!(skip_if_vanished(
            Err(gone),
            "attachments/1/.lcir/7/abcd/fig-p001-00.png",
            &mut skipped
        )
        .is_ok());
        assert_eq!(skipped.count, 1);
    }

    /// 報告本文は総数を落とさない（一覧だけ頭打ちにする）。
    #[test]
    fn the_report_keeps_the_total_when_the_list_is_capped() {
        let mut skipped = SkippedEntries::default();
        let n = SKIPPED_SAMPLE_MAX + 3;
        for i in 0..n {
            skipped.note(
                &format!("attachments/1/f{i}.pdf"),
                &io::Error::new(io::ErrorKind::NotFound, "gone"),
            );
        }
        assert_eq!(skipped.count, n);
        assert_eq!(skipped.names.len(), SKIPPED_SAMPLE_MAX);
        let report = skipped.report();
        assert!(report.contains(&n.to_string()), "総数が出る: {report}");
        assert!(report.contains("... and 3 more"), "省略が分かる: {report}");
    }

    /// zip アーカイブ内のエントリ名一覧を返すテストヘルパ。
    fn archive_names(path: &Path) -> Vec<String> {
        let file = fs::File::open(path).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().to_string())
            .collect()
    }

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
        // 完全バックアップは .zip
        assert!(p1.extension().is_some_and(|e| e == "zip"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// CR-018: バックアップは DB（db.sqlite）と添付本体を同一 zip に含む。
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_bundles_db_and_attachments(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-attach-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        // 添付レイアウトを模す: <app_dir>/attachments/<entry_id>/<file_name>
        let att = dir.join("attachments").join("42");
        std::fs::create_dir_all(&att).unwrap();
        std::fs::write(att.join("paper.pdf"), b"%PDF-1.7 fake pdf bytes").unwrap();
        // ネストしたサブディレクトリも再帰的に含まれること
        let nested = dir.join("attachments").join("7").join("sub");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("note.txt"), b"hello").unwrap();

        let archive = run_backup(&pool, &dir, 14).await.unwrap();
        let names = archive_names(&archive);

        assert!(names.iter().any(|n| n == "db.sqlite"), "names={names:?}");
        assert!(
            names.iter().any(|n| n == "attachments/42/paper.pdf"),
            "names={names:?}"
        );
        assert!(
            names.iter().any(|n| n == "attachments/7/sub/note.txt"),
            "names={names:?}"
        );

        // 添付本体のバイト列がそのまま格納されていること
        let file = fs::File::open(&archive).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut content = Vec::new();
        zip.by_name("attachments/42/paper.pdf")
            .unwrap()
            .read_to_end(&mut content)
            .unwrap();
        assert_eq!(content, b"%PDF-1.7 fake pdf bytes");

        // VACUUM 一時ファイルが残っていないこと
        let leftovers: Vec<_> = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".vacuum-"))
            .collect();
        assert!(leftovers.is_empty(), "temp vacuum files left: {leftovers:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// ②b W2-5: 走査中に読めなくなったエントリは**飛ばして続行**し、飛ばしたことを
    /// アーカイブ内の `SKIPPED.txt` に残す。
    ///
    /// 実際の失敗は「9 分の zip 書き出し中に添付や crop が消える」レースだが、
    /// レースは決定的に組めないので**同じ分岐**（`is_dir()` にも `is_file()` にも
    /// 当たらないパス）を壊れた symlink で作る。
    #[cfg(unix)]
    #[sqlx::test(migrations = "./migrations")]
    async fn a_vanished_entry_is_skipped_and_reported(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-skip-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let att = dir.join("attachments").join("42");
        std::fs::create_dir_all(&att).unwrap();
        std::fs::write(att.join("paper.pdf"), b"%PDF-1.7 fake").unwrap();
        // 走査が届いたときには実体が無いエントリ。
        std::os::unix::fs::symlink(att.join("gone.pdf"), att.join("dangling.pdf")).unwrap();

        let archive = run_backup(&pool, &dir, 14).await.unwrap();
        let names = archive_names(&archive);

        assert!(
            names.iter().any(|n| n == "attachments/42/paper.pdf"),
            "残っている添付は入る: {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "attachments/42/dangling.pdf"),
            "読めなかったエントリの空の枠を作らない: {names:?}"
        );
        assert!(
            names.iter().any(|n| n == SKIPPED_MANIFEST),
            "飛ばしたことが記録される: {names:?}"
        );

        let file = fs::File::open(&archive).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut report = String::new();
        zip.by_name(SKIPPED_MANIFEST)
            .unwrap()
            .read_to_string(&mut report)
            .unwrap();
        assert!(
            report.contains("attachments/42/dangling.pdf"),
            "どれが欠けたか分かる: {report}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 1 件も飛ばさなかったバックアップに報告は入らない。
    /// **「静かに欠けた成功」と「本当に完全な成功」を見分けられる**ことが目的なので、
    /// 常時 `SKIPPED.txt` を入れると意味が消える。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_complete_backup_carries_no_skip_manifest(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-noskip-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let att = dir.join("attachments").join("42");
        std::fs::create_dir_all(&att).unwrap();
        std::fs::write(att.join("paper.pdf"), b"%PDF-1.7 fake").unwrap();

        let archive = run_backup(&pool, &dir, 14).await.unwrap();
        let names = archive_names(&archive);

        assert!(!names.iter().any(|n| n == SKIPPED_MANIFEST), "{names:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 添付ディレクトリが無くても DB だけで成功する。
    #[sqlx::test(migrations = "./migrations")]
    async fn backup_without_attachments_dir_succeeds(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-noattach-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let archive = run_backup(&pool, &dir, 14).await.unwrap();
        let names = archive_names(&archive);
        assert_eq!(names, vec!["db.sqlite".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 自動バックアップは前回成功から間隔が空いていなければ間引かれる。
    /// 手動実行（`run_backup`）は間隔に関係なく常に走る。
    #[sqlx::test(migrations = "./migrations")]
    async fn auto_backup_is_skipped_until_interval_elapses(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-due-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        // 記録が無い初回は走る。
        let first = run_backup_if_due(&pool, &dir, DEFAULT_KEEP, AUTO_INTERVAL_SECS)
            .await
            .unwrap();
        assert!(first.is_some(), "初回は実行される");

        // 直後の 2 回目は間引かれる。
        let second = run_backup_if_due(&pool, &dir, DEFAULT_KEEP, AUTO_INTERVAL_SECS)
            .await
            .unwrap();
        assert!(second.is_none(), "24h 未満なので間引かれる");

        // 手動実行は間引かれない。
        assert!(run_backup(&pool, &dir, DEFAULT_KEEP).await.is_ok());

        // 間隔 0 なら常に due。
        assert!(run_backup_if_due(&pool, &dir, DEFAULT_KEEP, 0)
            .await
            .unwrap()
            .is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 前回成功時刻が未来（時計のずれ）でも止まらない。
    #[sqlx::test(migrations = "./migrations")]
    async fn future_last_run_does_not_block_backups(pool: SqlitePool) {
        let future = (Local::now() + chrono::Duration::days(3)).to_rfc3339();
        crate::db::settings::set_setting(
            &pool,
            crate::db::settings::BACKUP_LAST_RUN_KEY,
            &future,
        )
        .await
        .unwrap();

        assert!(is_backup_due(&pool, AUTO_INTERVAL_SECS).await);
    }

    /// 途中終了で残った作業ファイルだけを回収し、本物のバックアップは消さない。
    #[test]
    fn sweep_removes_stale_work_files_only() {
        let dir = std::env::temp_dir().join(format!("lc-backup-sweep-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();

        let files = [
            ".vacuum-lumencite-20260101-000000.db.tmp",
            ".vacuum-lumencite-20260101-000000.db.tmp-journal",
            "lumencite-20260101-000000.zip.partial",
            "lumencite-20260101-000002.db-journal", // 本体が prune 済みの旧 journal
            "lumencite-20260101-000000.zip",        // 本物
            "lumencite-20260101-000001.db",         // 旧世代の DB-only バックアップ
        ];
        for f in files {
            std::fs::write(backups.join(f), b"x").unwrap();
        }
        // 猶予（WORK_FILE_STALE_SECS）を過ぎた古い残骸として mtime を戻す。
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        for f in &files[..4] {
            let file = std::fs::File::options()
                .write(true)
                .open(backups.join(f))
                .unwrap();
            file.set_modified(old).unwrap();
        }

        let removed = sweep_backup_workdir(&dir, DEFAULT_KEEP);
        assert_eq!(removed, 4, "作業ファイル 4 件だけ消える");

        let mut left: Vec<String> = std::fs::read_dir(&backups)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                "lumencite-20260101-000000.zip".to_string(),
                "lumencite-20260101-000001.db".to_string(),
            ]
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 書き込み中の作業ファイル（新しい mtime）は別インスタンスのものかもしれないので消さない。
    #[test]
    fn sweep_keeps_fresh_work_files() {
        let dir = std::env::temp_dir().join(format!("lc-backup-sweep-fresh-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join(".vacuum-lumencite-20260101-000000.db.tmp"), b"x").unwrap();

        assert_eq!(sweep_backup_workdir(&dir, DEFAULT_KEEP), 0);
        assert!(backups.join(".vacuum-lumencite-20260101-000000.db.tmp").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 成功したバックアップは `.zip.partial` を残さない（rename されている）。
    #[sqlx::test(migrations = "./migrations")]
    async fn successful_backup_leaves_no_partial(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-partial-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        let archive = run_backup(&pool, &dir, DEFAULT_KEEP).await.unwrap();
        assert!(archive.exists());

        let partials: Vec<_> = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
            .collect();
        assert!(partials.is_empty(), "partial left: {partials:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// CR-022: 同時実行でもロックで直列化され、全て成功して別ファイルになる。
    #[sqlx::test(migrations = "./migrations")]
    async fn concurrent_backups_all_succeed_with_distinct_files(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-backup-conc-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        // 4 本を同時に投げる。ロックが無ければ同一秒のファイル名衝突で失敗し得る。
        let (r1, r2, r3, r4) = tokio::join!(
            run_backup(&pool, &dir, 14),
            run_backup(&pool, &dir, 14),
            run_backup(&pool, &dir, 14),
            run_backup(&pool, &dir, 14),
        );
        let paths = [r1.unwrap(), r2.unwrap(), r3.unwrap(), r4.unwrap()];
        for p in &paths {
            assert!(p.exists(), "{p:?} should exist");
        }
        let unique: std::collections::HashSet<_> = paths.iter().collect();
        assert_eq!(unique.len(), 4, "全て別ファイル: {paths:?}");

        std::fs::remove_dir_all(&dir).ok();
    }
}
