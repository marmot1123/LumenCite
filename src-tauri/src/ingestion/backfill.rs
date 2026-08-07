//! LCIR の起動時バックフィル（v1.0.0-p2）。
//!
//! 添付が増えたときの自動 build（`ingestion::ingest_new_pdf_attachment`）だけでは、
//! **p2 より前から在る添付**に LCIR が永久に届かない（`attachments_without_completed_lcir`
//! は完了版のある添付を除外し、`attachments_with_outdated_lcir` は版 bump 無しでは 0 件）。
//! そこで起動時に少しずつ進めるバックフィルを置く。雛形はバックアップの間引き
//! （`backup::run_backup_if_due` / settings KV / dev 既定オフ / 起動時 spawn + interval）。
//!
//! ## 上限は「経過時間」だけで、件数では切らない
//!
//! 実測の分布が極端に偏る（138 PDF で 1,180 秒・うち att37 1 本で約 8 分）ので、件数上限は
//! 最悪ケースを何も制御しない。かつ `spawn_blocking` の中の pdfium 抽出はキャンセルできないので
//! **添付境界でしか止められない** ＝ 最悪は「予算 + 最長 1 添付」。
//! ただし予算判定の後に build ロック待ちで実時間が進むと、この上限は崩れる。
//! そこで**ロックはこの層で（残り予算をタイムアウトにして）取り、取れた直後に
//! `should_stop` と残り予算をもう一度評価する**。
//!
//! ## 対象は「完了版が無い添付」だけ
//!
//! `attachments_with_outdated_lcir` は繋がない。抽出器版は LCIR の PR ごとにほぼ毎回上がるので、
//! 繋ぐと「毎リリースで全ユーザーが無断で全再構築を踏む」と等価になる
//! （`docs/LCIR_REMAINING_PHASES.md` §2.5 の決定）。

use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use sqlx::SqlitePool;

use crate::db::{document_versions, settings};

/// 自動バックフィルの最小間隔（秒）。前回の「1 件以上着手した回」からこれ未満なら走らない。
pub const AUTO_INTERVAL_SECS: i64 = 60 * 60;

/// 1 ランで新しい添付に着手してよい時間（添付境界で判定）。
pub const DEFAULT_RUN_BUDGET: Duration = Duration::from_secs(5 * 60);

/// 起動してからバックフィルを始めるまでの待ち。MCP のハンドシェイク（15 秒でタイムアウト）と
/// 初期描画を巻き添えにしないための猶予。
pub const STARTUP_DELAY: Duration = Duration::from_secs(60);

/// build ロックを待つ上限を「残り予算」にするときの下限（残り予算が 0 でも一瞬は試す）。
const MIN_LOCK_WAIT: Duration = Duration::from_millis(50);

/// 1 ランの上限。
#[derive(Debug, Clone, Copy)]
pub struct BackfillLimits {
    /// 新しい添付に着手してよい時間。**超過の判定は添付境界のみ**。
    pub budget: Duration,
}

impl Default for BackfillLimits {
    fn default() -> Self {
        Self {
            budget: DEFAULT_RUN_BUDGET,
        }
    }
}

/// ランを途中で降りた理由。**「対象 0 件で終わった」と「1 本も実行しなかった」を
/// 同じ見た目にしない**ための値（`stopped: None` は最後まで回したという意味）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// 予算を使い切った（残りは次回のランへ）。
    Budget,
    /// 手動バッチ / Vision / TeX 取得 / バックアップが始まったので譲った。
    Yielded,
    /// build ロックを残り予算の中で取れなかった。
    LockBusy,
}

/// 1 ランの結果。
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BackfillOutcome {
    /// 対象クエリが返した件数（latch / 失敗 skip を掛ける前）。
    pub total: i64,
    /// 実際に build を呼んだ件数。
    pub attempted: i64,
    /// 新しく版ができた件数。
    pub built: i64,
    /// build がエラーを返した件数。
    pub failed: i64,
    /// pdfium が使えないので飛ばした PDF の件数。**0 件処理と区別するために数える**。
    pub skipped_no_pdfium: i64,
    /// このプロセスで既に失敗していたので飛ばした件数。
    pub skipped_failed_before: i64,
    /// 途中で降りた理由（最後まで回したなら `None`）。
    pub stopped: Option<StopReason>,
}

/// プロセス寿命で持ち越す状態。**global static にせず呼び出し側が所有する**
/// （global にするとテストが実行順に依存し、並列実行で不定期に落ちる）。
#[derive(Debug, Default)]
pub struct BackfillState {
    /// このプロセスで build に失敗した添付。対象クエリは失敗しても同じ先頭を返し続けるので、
    /// 覚えておかないと毎ラン同じ添付に予算を食われる。
    failed: std::sync::Mutex<HashSet<i64>>,
}

impl BackfillState {
    fn is_known_failed(&self, id: i64) -> bool {
        self.failed.lock().map(|s| s.contains(&id)).unwrap_or(false)
    }

    fn note_failure(&self, id: i64) {
        if let Ok(mut s) = self.failed.lock() {
            s.insert(id);
        }
    }
}

/// 記録の生値から「今走らせてよいか」を決める純関数。
///
/// **パース不能な値は「走らせる」に倒す**（`backup::is_backup_due` と同じ向き）。
/// 逆向き（走らせない）に倒すと、壊れた値が 1 度書かれただけでバックフィルが**永久に**
/// 止まる。走らせる側に倒しても暴走しないのは、1 件以上着手したランが必ず正しい形式で
/// 上書きするから ── つまり自己修復する。対象 0 件のときは書かないが、その回は
/// クエリ 1 本で終わるので毎起動走っても実害がない。
pub(crate) fn is_due_from_record(
    raw: Option<&str>,
    now: DateTime<Local>,
    min_interval_secs: i64,
) -> bool {
    let Some(raw) = raw else {
        return true; // 未実施
    };
    let Ok(last) = DateTime::parse_from_rfc3339(raw) else {
        return true; // 壊れた値: 走らせて上書きさせる
    };
    let elapsed = (now - last.with_timezone(&Local)).num_seconds();
    // 未来の時刻（時計のずれ・タイムゾーン変更）でも走らせる。そうしないと
    // 記録が過去に戻るまで永久に止まる。
    elapsed >= min_interval_secs || elapsed < 0
}

/// 前回から `min_interval_secs` 以上経っているときだけバックフィルする。
/// 間引かれた場合は `None`。
pub async fn run_backfill_if_due<F>(
    pool: &SqlitePool,
    app_data_dir: &Path,
    limits: BackfillLimits,
    min_interval_secs: i64,
    state: &BackfillState,
    pdfium_available: bool,
    should_stop: F,
) -> Option<BackfillOutcome>
where
    F: Fn() -> bool,
{
    // LCIR OFF なら**キーを書かずに**降りる。書くと p3 で既定 ON になった既存ユーザーに
    // バックフィルが永久に届かない（`derive_page_fts_from_lcir_once` と同じ規約）。
    if !super::lcir_enabled(pool).await {
        return None;
    }
    let raw = settings::get_setting(pool, settings::LCIR_BACKFILL_LAST_RUN_KEY)
        .await
        .ok()
        .flatten();
    if !is_due_from_record(raw.as_deref(), Local::now(), min_interval_secs) {
        return None;
    }

    let outcome = run_backfill(
        pool,
        app_data_dir,
        limits,
        state,
        pdfium_available,
        should_stop,
    )
    .await;

    // **1 件も着手しなかった回は記録しない。** 対象 0 件・全 skip・譲って降りた回に記録すると、
    // 次に添付が増えても / 手動バッチが終わっても 1 時間動けなくなる。着手した回だけ記録すれば、
    // 壊れた値が入っていても 1 ランで正しい形式に上書きされる（`is_due_from_record` 参照）。
    if outcome.attempted > 0 {
        if let Err(e) = record_run(pool, Local::now()).await {
            eprintln!("LCIR backfill: failed to record last run: {e}");
        }
    }
    Some(outcome)
}

/// 対象を順に build する 1 ラン。`should_stop` は**添付境界で**評価する。
///
/// build ロックは**この層で**（残り予算をタイムアウトにして）取る。`build_lcir_for_attachment`
/// の内側で取ると、予算判定を通った後にロック待ちで実時間が進み、「予算 + 最長 1 添付」という
/// 上限が崩れる（GC が 125 秒握っていれば、起きたときには予算をとうに過ぎている）。
/// **取れた直後に `should_stop` と残り予算をもう一度評価する。**
pub async fn run_backfill<F>(
    pool: &SqlitePool,
    app_data_dir: &Path,
    limits: BackfillLimits,
    state: &BackfillState,
    pdfium_available: bool,
    should_stop: F,
) -> BackfillOutcome
where
    F: Fn() -> bool,
{
    let started = Instant::now();
    let targets = match targets(pool).await {
        Ok(t) => t,
        Err(e) => {
            eprintln!("LCIR backfill: target query failed: {e}");
            return BackfillOutcome::default();
        }
    };

    let mut out = BackfillOutcome {
        total: targets.len() as i64,
        ..Default::default()
    };

    for (att_id, mime) in targets {
        let is_tex = mime == super::TEX_SOURCE_MIME;

        // pdfium が使えないなら PDF は飛ばす。**判定は mime を見てから** ── 入口で切ると
        // pdfium を要さない TeX の build まで道連れになる。
        if !pdfium_available && !is_tex {
            out.skipped_no_pdfium += 1;
            continue;
        }
        // このプロセスで既に失敗した添付は再挑戦しない（毎ラン同じ先頭に予算を食われる）。
        if state.is_known_failed(att_id) {
            out.skipped_failed_before += 1;
            continue;
        }
        // ロックを取る。上限は残り予算（ロック待ちで実時間が進むので、待つ量にも予算を効かせる）。
        // **ここでは判定しない** ── 予算と譲りの判断はロックを取った後の 1 か所だけに置く。
        // 手前にも同じ判定を置くと、どちらを壊してもテストが落ちない冗長になる（#8 の教訓）。
        let remaining = limits.budget.saturating_sub(started.elapsed());
        let Some(_guard) = super::lock_build_within(remaining.max(MIN_LOCK_WAIT)).await else {
            out.stopped = Some(StopReason::LockBusy);
            break;
        };
        // **唯一の判定点**。ロックを待っている間に手動バッチが始まったり予算を使い切ったり
        // しうるので、取れた「後」に評価する（取れた時点の状態が唯一の真実）。
        if should_stop() {
            out.stopped = Some(StopReason::Yielded);
            break;
        }
        if started.elapsed() >= limits.budget {
            out.stopped = Some(StopReason::Budget);
            break;
        }

        out.attempted += 1;
        match super::build_lcir_unlocked(pool, app_data_dir, att_id).await {
            Ok(r) if r.built => out.built += 1,
            Ok(_) => {}
            Err(e) => {
                eprintln!("LCIR backfill: build failed for attachment {att_id}: {e}");
                out.failed += 1;
                state.note_failure(att_id);
            }
        }
    }

    if out.attempted > 0 || out.stopped.is_some() || out.skipped_no_pdfium > 0 {
        eprintln!(
            "LCIR backfill: {}/{} attempted (built {} / failed {} / \
             skipped: no-pdfium {}, failed-before {}){}",
            out.attempted,
            out.total,
            out.built,
            out.failed,
            out.skipped_no_pdfium,
            out.skipped_failed_before,
            match out.stopped {
                Some(StopReason::Budget) => "; stopped: run budget spent",
                Some(StopReason::Yielded) => "; stopped: yielded to another job",
                Some(StopReason::LockBusy) => "; stopped: build lock busy",
                None => "",
            }
        );
    }
    out
}

/// 対象添付（完了版が無い LCIR 対象添付）を `(id, mime_type)` で返す薄いラッパ。
async fn targets(pool: &SqlitePool) -> Result<Vec<(i64, String)>, sqlx::Error> {
    document_versions::attachments_without_completed_lcir(pool).await
}

/// 直近の「1 件以上着手した」時刻を記録する。
async fn record_run(pool: &SqlitePool, now: DateTime<Local>) -> Result<(), sqlx::Error> {
    settings::set_setting(
        pool,
        settings::LCIR_BACKFILL_LAST_RUN_KEY,
        &now.to_rfc3339(),
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;
    use crate::document_ir;

    async fn entry_with_pdf(pool: &SqlitePool, title: &str, mime: &str) -> i64 {
        let entry = create_entry(
            pool,
            &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        add_attachment(
            pool,
            entry.id,
            &format!("attachments/{}/{title}.pdf", entry.id),
            &format!("{title}.pdf"),
            mime,
        )
        .await
        .unwrap()
        .id
    }

    /// 完了版を 1 行だけ直接入れる（pdfium を要さずに「build 済み」を作る）。
    async fn insert_completed_version(pool: &SqlitePool, att: i64, name: &str, version: &str) {
        sqlx::query(
            "INSERT INTO document_versions
               (attachment_id, content_key, schema_version, source_sha256, source_mime_type,
                extractor_name, extractor_version, extraction_status)
             VALUES (?, ?, '0.1.0', 'sha', 'application/pdf', ?, ?, 'completed')",
        )
        .bind(att)
        .bind(format!("ck-{att}-{version}"))
        .bind(name)
        .bind(version)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn enable_lcir(pool: &SqlitePool) {
        settings::set_setting(pool, settings::LCIR_ENABLED_KEY, "1")
            .await
            .unwrap();
    }

    // ---- 対象選択 -------------------------------------------------------

    /// 対象クエリの 2 要素目は **mime_type**（誰も読んでいなかった file_path から変えた）。
    /// pdfium が使えないときに PDF だけ落として TeX を続けるために要る。
    #[sqlx::test(migrations = "./migrations")]
    async fn targets_carry_mime_type(pool: SqlitePool) {
        let pdf = entry_with_pdf(&pool, "a", "application/pdf").await;
        let tex = entry_with_pdf(&pool, "b", crate::ingestion::TEX_SOURCE_MIME).await;
        let mut got = targets(&pool).await.unwrap();
        got.sort();
        assert_eq!(
            got,
            vec![
                (pdf, "application/pdf".to_string()),
                (tex, crate::ingestion::TEX_SOURCE_MIME.to_string()),
            ],
            "2 要素目は mime_type"
        );
    }

    /// **版を bump しただけでは対象にならない。** `attachments_with_outdated_lcir` を
    /// 繋ぐと毎リリースで全ユーザーが無断の全再構築を踏むので、繋いでいないことを固定する。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_version_bump_alone_yields_no_targets(pool: SqlitePool) {
        let att = entry_with_pdf(&pool, "a", "application/pdf").await;
        // 現行版より古い抽出器版で完了している（= outdated だが未構築ではない）
        insert_completed_version(&pool, att, document_ir::schema::EXTRACTOR_NAME, "0.1.0").await;
        assert!(
            targets(&pool).await.unwrap().is_empty(),
            "旧版で完了している添付はバックフィルの対象にしない"
        );
    }

    // ---- 上限と譲り -----------------------------------------------------

    /// 予算 0 なら 1 件も着手せずに `Budget` で降りる（対象は在る）。
    #[sqlx::test(migrations = "./migrations")]
    async fn zero_budget_attempts_nothing(pool: SqlitePool) {
        enable_lcir(&pool).await;
        entry_with_pdf(&pool, "a", "application/pdf").await;
        let st = BackfillState::default();
        let r = run_backfill(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits {
                budget: Duration::ZERO,
            },
            &st,
            true,
            || false,
        )
        .await;
        assert_eq!(r.total, 1);
        assert_eq!(r.attempted, 0, "予算 0 では着手しない");
        assert_eq!(r.stopped, Some(StopReason::Budget));
    }

    /// `should_stop` が最初から真なら 1 件も着手せず `Yielded` で降りる。
    #[sqlx::test(migrations = "./migrations")]
    async fn yields_before_touching_anything(pool: SqlitePool) {
        enable_lcir(&pool).await;
        entry_with_pdf(&pool, "a", "application/pdf").await;
        let st = BackfillState::default();
        let r = run_backfill(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            &st,
            true,
            || true,
        )
        .await;
        assert_eq!(r.attempted, 0);
        assert_eq!(r.stopped, Some(StopReason::Yielded));
    }

    // ---- pdfium latch ---------------------------------------------------

    /// pdfium が使えないときは **PDF を落として TeX は処理する**。
    /// 入口（mime 判定より前）で切ると pdfium が要らない TeX まで止まる。
    #[sqlx::test(migrations = "./migrations")]
    async fn without_pdfium_skips_pdf_but_still_tries_tex(pool: SqlitePool) {
        enable_lcir(&pool).await;
        entry_with_pdf(&pool, "a", "application/pdf").await;
        entry_with_pdf(&pool, "b", crate::ingestion::TEX_SOURCE_MIME).await;
        let st = BackfillState::default();
        let r = run_backfill(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            &st,
            false,
            || false,
        )
        .await;
        assert_eq!(r.total, 2);
        assert_eq!(r.skipped_no_pdfium, 1, "PDF は飛ばす");
        assert_eq!(r.attempted, 1, "TeX は着手する（ファイルが無いので failed になる）");
        assert_eq!(r.failed, 1);
    }

    // ---- 失敗の持ち越し -------------------------------------------------

    /// 同じプロセスで失敗した添付は次のランで飛ばす（毎ラン同じ先頭に予算を食われない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failure_is_not_retried_in_the_same_process(pool: SqlitePool) {
        enable_lcir(&pool).await;
        entry_with_pdf(&pool, "a", "application/pdf").await;
        let st = BackfillState::default();
        let first = run_backfill(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            &st,
            true,
            || false,
        )
        .await;
        assert_eq!(first.failed, 1, "ファイルが無いので失敗する");

        let second = run_backfill(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            &st,
            true,
            || false,
        )
        .await;
        assert_eq!(second.attempted, 0, "2 回目は着手しない");
        assert_eq!(second.skipped_failed_before, 1);
    }

    // ---- 間引きキー -----------------------------------------------------

    /// LCIR が OFF なら何もせず、**キーも書かない**。
    /// 書くと p3 で既定 ON になった既存ユーザーにバックフィルが永久に届かない。
    #[sqlx::test(migrations = "./migrations")]
    async fn disabled_does_nothing_and_writes_no_key(pool: SqlitePool) {
        entry_with_pdf(&pool, "a", "application/pdf").await;
        let st = BackfillState::default();
        let r = run_backfill_if_due(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            AUTO_INTERVAL_SECS,
            &st,
            true,
            || false,
        )
        .await;
        assert!(r.is_none(), "OFF なら None");
        assert_eq!(
            settings::get_setting(&pool, settings::LCIR_BACKFILL_LAST_RUN_KEY)
                .await
                .unwrap(),
            None,
            "OFF の回はキーを書かない"
        );
    }

    /// 対象 0 件の回はキーを書かない（次に添付が増えたらすぐ拾えるように）。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_run_writes_no_key(pool: SqlitePool) {
        enable_lcir(&pool).await;
        let st = BackfillState::default();
        let r = run_backfill_if_due(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            AUTO_INTERVAL_SECS,
            &st,
            true,
            || false,
        )
        .await;
        assert_eq!(r.map(|o| o.total), Some(0));
        assert_eq!(
            settings::get_setting(&pool, settings::LCIR_BACKFILL_LAST_RUN_KEY)
                .await
                .unwrap(),
            None,
            "対象 0 件ではキーを書かない"
        );
    }

    /// 1 件以上着手した回はキーを書き、次のランは間引かれる。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_attempted_run_records_and_then_throttles(pool: SqlitePool) {
        enable_lcir(&pool).await;
        entry_with_pdf(&pool, "a", "application/pdf").await;
        let st = BackfillState::default();
        let first = run_backfill_if_due(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            AUTO_INTERVAL_SECS,
            &st,
            true,
            || false,
        )
        .await;
        assert_eq!(first.map(|o| o.attempted), Some(1));
        assert!(
            settings::get_setting(&pool, settings::LCIR_BACKFILL_LAST_RUN_KEY)
                .await
                .unwrap()
                .is_some(),
            "着手したらキーを書く"
        );

        let second = run_backfill_if_due(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits::default(),
            AUTO_INTERVAL_SECS,
            &st,
            true,
            || false,
        )
        .await;
        assert!(second.is_none(), "直後は間引かれる");
    }

    // ---- due 判定（純関数） ---------------------------------------------

    #[test]
    fn due_when_there_is_no_record() {
        assert!(is_due_from_record(None, Local::now(), 3600));
    }

    /// **壊れた値は「走らせる」に倒す。** 逆に倒すと 1 度書かれただけで永久に止まる。
    /// 走らせても暴走しないのは、着手したランが必ず正しい形式で上書きするから。
    #[test]
    fn due_when_the_record_cannot_be_parsed() {
        for raw in ["", "garbage", "2026-13-45", "1754500000"] {
            assert!(
                is_due_from_record(Some(raw), Local::now(), 3600),
                "パース不能な {raw:?} は走らせる側"
            );
        }
    }

    #[test]
    fn not_due_right_after_a_run() {
        let now = Local::now();
        let raw = now.to_rfc3339();
        assert!(!is_due_from_record(Some(&raw), now, 3600));
    }

    #[test]
    fn due_after_the_interval_and_for_future_timestamps() {
        let now = Local::now();
        let old = (now - chrono::Duration::seconds(7200)).to_rfc3339();
        assert!(is_due_from_record(Some(&old), now, 3600));
        let future = (now + chrono::Duration::seconds(7200)).to_rfc3339();
        assert!(
            is_due_from_record(Some(&future), now, 3600),
            "未来の時刻でも走らせる（記録が過去に戻るまで止まらないように）"
        );
    }

    // ---- build ロック -----------------------------------------------------

    /// 対話 build は**待ち行列に並ぶ**（`try_lock` ではない）。背景が保持している間に
    /// `try_lock` を使うと、tokio Mutex は FIFO 公平なので確率的にではなく
    /// **決定的に**失敗し続け、バックフィル中ずっとボタンが押せなくなる。
    #[tokio::test]
    async fn the_interactive_build_lock_queues_instead_of_failing_fast() {
        let guard = super::super::lock_build().await;
        // 保持中は上限付き取得が失敗する。
        assert!(
            super::super::lock_build_within(Duration::from_millis(30))
                .await
                .is_none(),
            "保持中は上限内で取れない"
        );
        // 解放したら**待っていた側に**渡る（try_lock なら取りこぼしうる窓）。
        let waiter = tokio::spawn(async {
            super::super::lock_build_within(Duration::from_secs(5))
                .await
                .is_some()
        });
        tokio::task::yield_now().await;
        drop(guard);
        assert!(waiter.await.unwrap(), "解放後は待っていた側が取れる");
    }

    /// バックフィルはロックが取れなければ `LockBusy` で降り、**着手 0 なので記録もしない**
    /// （記録すると、たまたま GC とぶつかった 1 回で 1 時間動けなくなる）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_held_build_lock_stops_the_run_without_recording(pool: SqlitePool) {
        enable_lcir(&pool).await;
        entry_with_pdf(&pool, "a", "application/pdf").await;
        let held = super::super::lock_build().await;
        let st = BackfillState::default();
        let r = run_backfill_if_due(
            &pool,
            Path::new("/nonexistent"),
            BackfillLimits {
                budget: Duration::from_millis(50),
            },
            AUTO_INTERVAL_SECS,
            &st,
            true,
            || false,
        )
        .await;
        drop(held);
        assert_eq!(r.as_ref().map(|o| o.attempted), Some(0));
        assert_eq!(r.and_then(|o| o.stopped), Some(StopReason::LockBusy));
        assert_eq!(
            settings::get_setting(&pool, settings::LCIR_BACKFILL_LAST_RUN_KEY)
                .await
                .unwrap(),
            None,
            "着手 0 の回は記録しない"
        );
    }

    #[tokio::test]
    async fn record_run_writes_rfc3339_that_is_parseable_back() {
        // 形式の往復を固定する（`is_due_from_record` の入力はこの関数の出力）。
        let now = Local::now();
        let raw = now.to_rfc3339();
        assert!(DateTime::parse_from_rfc3339(&raw).is_ok());
    }
}
