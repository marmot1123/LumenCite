//! 長時間バッチの**実行状態と直近の結果をバックエンドに置く**（debt-32・v1.0.0 の #10 の完了条件）。
//!
//! **読み手は `SettingsModal.tsx`**（設定→データ）。今まで実行状態も進捗も対象件数も
//! コンポーネントローカルの state にしか無く、モーダルを閉じるとアンマウントで消えていた。
//! `lcir-build-progress` 等のリスナーは `[]` 依存の `useEffect` で貼り直されるが、
//! **表示条件がローカル state なのでイベントは届いているのに捨てられる**。実害は 2 つ:
//!
//! 1. 閉じている間にバッチが終わると **`failed` 件数を読む手段が消える**（#7 の完了条件そのもの）。
//! 2. **課金操作の同意を古い件数で取る**。2026-08-06 の #7 で実際に起きた ── 再構築の途中で
//!    モーダルを開き直したため、マウント時点の 89 件を表示し続けたまま、実際の対象 346 件を
//!    課金しうる状態になっていた（約 4 倍の過少表示）。
//!
//! ## ここは「開始してよいか」を決めない
//!
//! 裁定は呼び出し元の `compare_exchange`（`LCIR_BATCH_RUNNING` / `VISION_ALT_TEXT_RUNNING` /
//! `TEX_FETCH_RUNNING`）と `LCIR_BUILD_LOCK` が持つ。**このモジュールはその結果を写すだけ**で、
//! 判定は 1 つも持たない。2 か所に判定を置くと、片方だけ直したときに無言でずれる
//! （#9 の変異 survivor は全部この形だった）。したがって [`RunningMark`] は「取れた後」に作る。
//!
//! ## 実行中は**集合**であって 1 枠ではない
//!
//! 種別どうしは全部が排他ではない。**LCIR 系の 6 種は v1.0.0 で互いに排他になった**
//! （②b の W1-6。3 つの入口 `begin_lcir_batch` / `begin_vision_alt_text_batch` /
//! `begin_tex_fetch_batch` が互いを見る）が、`ocr` はそれとは独立に走り、
//! **排他の外にある build も残っている**（p2 の自動 build と、supersede しない 1 件 build）。
//! 1 枠にすると**実際に走っている 2 本目が「走っていない」ことになる**ので集合で持つ。

use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// 長時間バッチの種別。文字列は `SettingsModal.tsx` の分岐と共有する契約なので、
/// **変えるときは両方**（`BatchKind` の `as_str` とフロントの型）を直すこと。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum BatchKind {
    /// 完了 LCIR が無い PDF の一括構築（`build_missing_lcir`）
    Build,
    /// 旧抽出器版の現行版への再構築（`rebuild_outdated_lcir`）
    Rebuild,
    /// 全文索引を LCIR の page から張り直す（`rederive_fulltext_from_lcir`・p1）
    Rederive,
    /// superseded 版の GC（`run_lcir_gc`・p4）。**非可逆**
    Gc,
    /// 図の代替テキスト一括生成（`generate_vision_alt_texts`・8c）。**課金される**
    VisionAltText,
    /// arXiv e-print の一括取得（`fetch_missing_arxiv_sources`）
    TexFetch,
    /// スキャン PDF の OCR（`run_ocr`）。**1 ページごとに課金される。**
    ///
    /// 他の 6 種と違い、起動口が 2 つある（リーダーのボタン / チャットの `ocr_pdf` ツール）。
    /// **どちらから始めても必ずここに載せる** ── 載せないと、リーダーを離れた瞬間に
    /// 「走っていることも、止める手段も」画面から消える（PR-1b のレビューで実際に出た）。
    Ocr,
}

impl BatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BatchKind::Build => "build",
            BatchKind::Rebuild => "rebuild",
            BatchKind::Rederive => "rederive",
            BatchKind::Gc => "gc",
            BatchKind::VisionAltText => "vision_alt_text",
            BatchKind::TexFetch => "tex_fetch",
            BatchKind::Ocr => "ocr",
        }
    }
}

impl Serialize for BatchKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// バッチ 1 本の進捗。イベント（`lcir-build-progress` 等）と同じ `{done, total}`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub done: i64,
    pub total: i64,
}

/// 直近に終わったバッチ 1 本。**モーダルを閉じている間に終わった結果を後から読むためのもの。**
#[derive(Clone, Debug, Serialize)]
pub struct FinishedBatch {
    pub kind: BatchKind,
    /// RFC3339。フロントは「前回表示した時刻」と比べて、同じ結果を開くたび再掲しない。
    pub finished_at: String,
    /// 成功時は**コマンドの戻り値そのもの**を JSON にしたもの。文言整形は i18n を持つ
    /// フロントの仕事なので、ここでは組み立てない（同じ整形を 2 か所に置かない）。
    pub result: Option<serde_json::Value>,
    /// 失敗時のエラー文字列。`result` とはどちらか一方。
    ///
    /// **`already_running` はここに載らない。** あれは「バッチが走らなかった」であって
    /// 「バッチが終わった」ではないので、排他に弾かれた呼び出しは記録側（`record_batch`）へ
    /// 到達する前に返る。載せると、実際に走っている本物のバッチの結果を上書きしてしまう。
    pub error: Option<String>,
}

/// [`snapshot`] の戻り値。フロントがマウント時に 1 回引く。
#[derive(Clone, Debug, Default, Serialize)]
pub struct BatchStatus {
    /// 今走っている種別（複数ありうる）。
    pub running: Vec<BatchKind>,
    /// 種別 → 直近の進捗。**実行中でないものは載らない**（[`RunningMark`] の Drop が消す）。
    pub progress: BTreeMap<String, Progress>,
    /// 直近に終わったバッチ 1 本。表示面が 1 つ（メッセージ欄）なので 1 枠で足りる。
    pub last: Option<FinishedBatch>,
}

struct State {
    running: Vec<BatchKind>,
    progress: BTreeMap<String, Progress>,
    last: Option<FinishedBatch>,
}

impl State {
    const fn new() -> Self {
        State { running: Vec::new(), progress: BTreeMap::new(), last: None }
    }
}

static STATE: Mutex<State> = Mutex::new(State::new());

/// ロックが毒されていても状態表示は続ける。ここが握るのは表示用の値だけで、
/// **排他の裁定は 1 つも持っていない**ので、毒されたロックを避けて進んでも
/// 二重実行にはつながらない（裁定側の `compare_exchange` は無傷）。
fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

/// 「この種別が走っている」印。**Drop で必ず消える**ので、途中エラーや panic でも残らない。
///
/// ⚠ **裁定はしない。** 呼び出し元が排他フラグを取れた**後**に作ること。
pub struct RunningMark(BatchKind);

impl RunningMark {
    /// 進捗はここでは触らない。**前回の残骸を消すのは [`Drop`] の仕事**で、
    /// 開始側でも消すと同じ判断が 2 か所になる（変異を当てると開始側は誰にも観測されない
    /// ＝ 消しても何も壊れない死んだ行だと分かった）。
    pub fn new(kind: BatchKind) -> Self {
        with(|s| {
            if !s.running.contains(&kind) {
                s.running.push(kind);
                s.running.sort();
            }
        });
        Self(kind)
    }
}

impl Drop for RunningMark {
    fn drop(&mut self) {
        with(|s| {
            s.running.retain(|k| *k != self.0);
            s.progress.remove(self.0.as_str());
        });
    }
}

/// 進捗を更新する（イベント送出と同じ場所から呼ぶ）。
///
/// **イベントの貼り直しだけでは足りない**のでここに置く ── 実ライブラリの att37（527 頁）は
/// 1 添付に約 8 分かかり、その間 `lcir-build-progress` は 1 通も飛ばない。閉じて開き直した
/// フロントは、次のイベントが来るまで「何も走っていない」ように見えてしまう。
pub fn set_progress(kind: BatchKind, done: i64, total: i64) {
    with(|s| {
        s.progress.insert(kind.as_str().to_string(), Progress { done, total });
    });
}

/// バッチが成功で終わったことと**戻り値そのもの**を残す。
pub fn record_success<T: Serialize>(kind: BatchKind, result: &T) {
    let value = serde_json::to_value(result).ok();
    with(|s| {
        s.last = Some(FinishedBatch {
            kind,
            finished_at: chrono::Local::now().to_rfc3339(),
            result: value,
            error: None,
        });
    });
}

/// バッチが失敗で終わったことを残す。
pub fn record_failure(kind: BatchKind, error: &str) {
    with(|s| {
        s.last = Some(FinishedBatch {
            kind,
            finished_at: chrono::Local::now().to_rfc3339(),
            result: None,
            error: Some(error.to_string()),
        });
    });
}

/// **プロセス共有の static を触るテストの、モジュール横断の直列化ゲート。**
///
/// この表（running / progress / last）と OCR の排他フラグはプロセスに 1 つしか無いのに、
/// 読み書きするテストは 3 モジュール（ここ / `lib.rs` の `batch_wiring_tests` /
/// `llm::tools::ocr`）に散っている。**モジュールごとに別の gate を持つと相互の窓が残る**
/// ── 例えば ocr のテストが `RunningMark(Ocr)` を DB I/O をまたいで握っている間に、
/// ここの「誰も走っていない」前提 assert が並列スレッドで落ちる（#9 で CI だけ落とした形）。
/// tokio の Mutex なのは、`#[sqlx::test]` の async 本体が guard を await 越しに持つため。
/// 同期テストは `blocking_lock()` で取る。
#[cfg(test)]
pub(crate) static TEST_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 現在の状態を写して返す（**読み取り専用**）。
///
/// 「直近の結果」をここで消さないのは、読むだけの命令が状態を壊すと、
/// 2 つ開いた画面のうち先に読んだ方だけが結果を見られることになるため。
/// 再掲の抑制は `finished_at` を見るフロント側の仕事。
pub fn snapshot() -> BatchStatus {
    with(|s| BatchStatus {
        running: s.running.clone(),
        progress: s.progress.clone(),
        last: s.last.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **プロセス共有の static を触るテストは [`TEST_GATE`]（モジュール横断）で直列化する。**
    /// 並列実行だと他のテストが立てた印を自分のものと取り違える（#9 で CI だけ落とした形）。
    /// そのうえで、各テストは**自分が起こした遷移だけ**を assert する。
    fn gate() -> tokio::sync::MutexGuard<'static, ()> {
        TEST_GATE.blocking_lock()
    }

    #[test]
    fn mark_appears_while_alive_and_disappears_on_drop() {
        let _g = gate();
        assert!(
            !snapshot().running.contains(&BatchKind::Build),
            "前提: このテストの開始時に Build は走っていない"
        );
        {
            let _m = RunningMark::new(BatchKind::Build);
            assert!(snapshot().running.contains(&BatchKind::Build));
        }
        assert!(!snapshot().running.contains(&BatchKind::Build));
    }

    #[test]
    fn two_kinds_can_be_running_at_once() {
        let _g = gate();
        let _a = RunningMark::new(BatchKind::Rebuild);
        let _b = RunningMark::new(BatchKind::VisionAltText);
        let running = snapshot().running;
        // 1 枠だと 2 本目が「走っていない」ことになる。集合であることをここで固定する。
        assert!(running.contains(&BatchKind::Rebuild));
        assert!(running.contains(&BatchKind::VisionAltText));
    }

    #[test]
    fn progress_is_visible_while_running_and_cleared_after() {
        let _g = gate();
        {
            let _m = RunningMark::new(BatchKind::Gc);
            set_progress(BatchKind::Gc, 3, 10);
            assert_eq!(
                snapshot().progress.get("gc").copied(),
                Some(Progress { done: 3, total: 10 })
            );
        }
        assert!(
            !snapshot().progress.contains_key("gc"),
            "終わった種別の進捗は残さない（次に開いた画面が終わった仕事を進行中に見せる）"
        );
    }

    /// 2 回目が前回の進捗から始まって見えない、を端から端まで固定する。
    /// 保証しているのは 1 回目の `Drop` で、`RunningMark::new` は進捗に触らない。
    #[test]
    fn a_second_run_does_not_start_from_the_previous_progress() {
        let _g = gate();
        {
            let _m = RunningMark::new(BatchKind::TexFetch);
            set_progress(BatchKind::TexFetch, 7, 7);
        }
        let _m = RunningMark::new(BatchKind::TexFetch);
        assert!(
            !snapshot().progress.contains_key("tex_fetch"),
            "2 回目は前回の 7/7 を引き継がない"
        );
    }

    #[test]
    fn success_keeps_the_command_return_value_verbatim() {
        let _g = gate();
        #[derive(Serialize)]
        struct R {
            total: i64,
            failed: i64,
        }
        record_success(BatchKind::Build, &R { total: 138, failed: 2 });
        let last = snapshot().last.expect("直近の結果が残っている");
        assert_eq!(last.kind, BatchKind::Build);
        // #7 の完了条件そのもの: 閉じている間に終わっても failed を読めること。
        assert_eq!(last.result.as_ref().unwrap()["failed"], 2);
        assert_eq!(last.result.as_ref().unwrap()["total"], 138);
        assert!(last.error.is_none());
    }

    #[test]
    fn failure_keeps_the_error_and_no_result() {
        let _g = gate();
        record_failure(BatchKind::VisionAltText, "already_running");
        let last = snapshot().last.expect("直近の結果が残っている");
        assert_eq!(last.kind, BatchKind::VisionAltText);
        assert_eq!(last.error.as_deref(), Some("already_running"));
        assert!(last.result.is_none());
    }

    #[test]
    fn snapshot_does_not_consume_the_last_result() {
        let _g = gate();
        record_success(BatchKind::Rederive, &serde_json::json!({ "total": 1 }));
        let a = snapshot().last.expect("1 回目");
        let b = snapshot().last.expect("2 回目も読める（読み取りが状態を壊さない）");
        assert_eq!(a.finished_at, b.finished_at);
    }

    /// **終わったのは自分の種別だけ。** 1 本の Drop が他の実行中バッチの印まで消すと、
    /// 走っているバッチが UI から消えてボタンが押せる状態に戻る（並走しうる設計なので実在する）。
    #[test]
    fn dropping_one_mark_leaves_the_other_kinds_running() {
        let _g = gate();
        let outer = RunningMark::new(BatchKind::Build);
        {
            let _inner = RunningMark::new(BatchKind::VisionAltText);
            set_progress(BatchKind::Build, 1, 9);
        }
        let s = snapshot();
        assert!(s.running.contains(&BatchKind::Build), "他人の Drop で消えない");
        assert!(!s.running.contains(&BatchKind::VisionAltText));
        assert_eq!(
            s.progress.get("build").copied(),
            Some(Progress { done: 1, total: 9 }),
            "他人の Drop で進捗も消えない"
        );
        drop(outer);
    }

    /// 新しく作った記録には「まだ 1 件も無い」初期状態がある（`feedback_new_provenance_has_no_past`）。
    /// **`BatchStatus::default()` ではなく実際の `snapshot()` を通す** ── 既定値を直列化しても
    /// 「読み出し経路が初期状態で成立する」ことの証拠にはならない。
    #[test]
    fn a_snapshot_with_nothing_running_serializes_cleanly() {
        let _g = gate();
        let v = serde_json::to_value(snapshot()).unwrap();
        assert_eq!(v["running"], serde_json::json!([]), "このテストの開始時は誰も走っていない");
        assert!(v["progress"].is_object());
        // `last` は他のテストが残しうるので**存在すること**だけを見る（自分が起こしていない
        // 遷移は assert しない ── プロセス共有 static の作法）。
        assert!(v.get("last").is_some());
    }

    /// **フロントとの契約は「実際に届く JSON」**。`as_str` だけを見ても、serde の属性を
    /// 付け替えた瞬間に届く形が変わって気づけない（このテストが守るのは配線であって定数ではない）。
    #[test]
    fn the_json_the_frontend_receives_matches_the_agreed_shape() {
        let _g = gate();
        let mark = RunningMark::new(BatchKind::VisionAltText);
        set_progress(BatchKind::VisionAltText, 2, 5);
        record_success(BatchKind::VisionAltText, &serde_json::json!({ "total": 5 }));
        let v = serde_json::to_value(snapshot()).unwrap();

        // running は**文字列の配列**（フロントは includes() で引く）。
        let running = v["running"].as_array().expect("running は配列");
        assert!(running.contains(&serde_json::json!("vision_alt_text")));
        // progress は**種別文字列をキーにしたオブジェクト**（フロントは progress[kind] で引く）。
        assert_eq!(v["progress"]["vision_alt_text"], serde_json::json!({ "done": 2, "total": 5 }));
        // last は kind / finished_at / result / error の 4 つ。
        assert_eq!(v["last"]["kind"], serde_json::json!("vision_alt_text"));
        assert_eq!(v["last"]["result"]["total"], 5);
        assert!(v["last"]["error"].is_null());
        assert!(v["last"]["finished_at"].as_str().is_some_and(|s| s.contains('T')));
        drop(mark);
    }

    #[test]
    fn kind_strings_are_the_contract_with_the_frontend() {
        // フロントの型と 1:1。ここを変えたら SettingsModal.tsx の BatchKind も変える。
        assert_eq!(BatchKind::Build.as_str(), "build");
        assert_eq!(BatchKind::Rebuild.as_str(), "rebuild");
        assert_eq!(BatchKind::Rederive.as_str(), "rederive");
        assert_eq!(BatchKind::Gc.as_str(), "gc");
        assert_eq!(BatchKind::VisionAltText.as_str(), "vision_alt_text");
        assert_eq!(BatchKind::TexFetch.as_str(), "tex_fetch");
        assert_eq!(BatchKind::Ocr.as_str(), "ocr");
    }
}
