//! OCR ツール: スキャン PDF をページ画像化（pdfium）→ LLM Vision で文字起こし →
//! `fulltext` に保存して全文検索可能にする。ツール経由（LLM）と手動コマンドの両方から使う。

use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use serde_json::json;

use super::{ToolContext, ToolError};
use crate::keychain;
use crate::llm::{ocr, ToolCallSpec, ToolSpec};

/// 2 本目の OCR を弾いたことを表す印。フロントは専用の文言に変換する。
/// ⚠ 実際にフロントへ届く文字列は `ToolError::Execution` の Display が前置した
/// `"execution error: already_running"` なので、**読み手は必ず部分一致で拾う**こと
/// （等価比較で書くと一生マッチしない）。
pub const OCR_ALREADY_RUNNING: &str = "already_running";

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "ocr_pdf".to_string(),
        description: "OCR a scanned PDF attachment that has NO text layer: rasterize its pages, \
            transcribe them with the vision model, and index the text for full-text search. \
            This costs money and REPLACES the attachment's existing index, so it is a last resort: \
            first call get_fulltext, and only OCR when it answers `indexed: false`. An attachment \
            whose text is already indexed refuses OCR — whole or partial, naming `pages` does not \
            change that. Note that fulltext_search finding nothing only means those words are \
            absent — it does not mean the PDF is unindexed."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "entry_id": { "type": "integer", "description": "The entry whose PDF to OCR." },
                "attachment_id": {
                    "type": "integer",
                    "description": "Optional specific PDF attachment to OCR. Omit to use the entry's first PDF."
                },
                "pages": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "1-based page numbers to OCR. Omit to OCR all pages."
                }
            },
            "required": ["entry_id"]
        }),
        needs_approval: true,
    }]
}

pub async fn try_execute(
    ctx: &ToolContext<'_>,
    call: &ToolCallSpec,
) -> Option<Result<String, ToolError>> {
    if call.tool_name != "ocr_pdf" {
        return None;
    }
    let entry_id = match call.arguments.get("entry_id").and_then(|v| v.as_i64()) {
        Some(id) => id,
        None => {
            return Some(Err(ToolError::InvalidArguments(
                "entry_id is required".into(),
            )))
        }
    };
    let pages = requested_pages(&call.arguments);
    // チャットのスコープ外 entry は OCR させない（CR-024）。
    if let Err(e) = ctx.ensure_entry_in_scope(entry_id) {
        return Some(Err(e));
    }
    let attachment_id = requested_attachment_id(&call.arguments);

    // 索引済みの PDF を LLM に OCR し直させない（issue #42）。OCR は課金し、その添付の
    // テキスト層を Vision の出力で置き換え、完走すれば添付ごと「OCR 由来」に封印する。
    // 既に読めるなら `get_fulltext` へ誘導する。**ユーザーが UI から明示的に押す経路
    // （`ocr_pdf` コマンド → `run_ocr` 直呼び）はこの制限を受けない** —
    // テキスト層が壊れている PDF を OCR し直すのは正当な操作なので。
    if let Some(refusal) = guard_llm_ocr(ctx.pool, entry_id, attachment_id).await {
        return Some(Ok(refusal));
    }

    // チャットの停止ボタンを OCR の中まで届かせる（`ToolContext::should_stop`）。
    // 進捗はチャットには出さない（ツール結果 1 本で返す形なので出す先が無い）が、
    // `batch_status` へはループ自身が書くので、設定 → データには載る。
    // **フックの組み立ては `run_ocr` の中**（ここに置くと、潰す変異を落とすテストが書けない）。
    Some(run_ocr(ctx.pool, ctx.app_data_dir, entry_id, attachment_id, pages, ctx.should_stop, None).await)
}

/// `ocr_pdf` の `pages` 引数を読む。配列でなければ `None`（＝全ページ）、
/// 数値でない要素は落とす。
///
/// **切り出しているのは、`None` へ潰す変異が入口のテストだけでは落ちないため。**
/// 潰れると「5 ページだけ」と頼まれた 527 ページ本が全ページ課金される ──
/// 旧実装ではガードが `pages.is_none()` を見ていたので入口テストが間接的に守っていたが、
/// ガードが `pages` を見なくなった今は、ここを直接固定しないと誰も主張しない。
pub(crate) fn requested_pages(arguments: &serde_json::Value) -> Option<Vec<i64>> {
    arguments
        .get("pages")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
}

/// `ocr_pdf` の `attachment_id` 引数を読む。省略時は `None`（＝先頭の PDF）。
/// 潰れるとガードも `prepare_ocr` も先頭の PDF を見に行き、名指しした添付が無視される。
pub(crate) fn requested_attachment_id(arguments: &serde_json::Value) -> Option<i64> {
    arguments.get("attachment_id").and_then(|v| v.as_i64())
}

/// LLM 経路のガード（issue #42）。`Some(拒否文)` を返したら OCR に進まない。
///
/// **判定は「実際に OCR される添付」で取る。** `entry_id` だけで呼ばれたときは
/// [`resolve_target_pdf`] が [`prepare_ocr`] と同じ規則で対象を決めるので、
/// 「ガードが見た添付」と「OCR する添付」がずれない。entry 単位で数えていた頃は、
/// 本体 PDF が索引済みなだけで**未索引のスキャン補遺**まで拒否していた（過剰拒否）。
///
/// ⚠ **`pages` は判定材料にしない。** 「ページ指定なら通す」は迂回路になる ──
/// 完走した OCR は部分・全体を問わず添付ごと `fulltext.source = Ocr` を立てる（§2.6-1）ので、
/// 1 ページの部分 OCR でも**添付全体の索引が恒久的に再索引を拒む**ようになる。
/// §2.6-1 がその倒し込みの根拠に置いた「ユーザーが OCR を回した＝この PDF のテキスト層は
/// 信用できないという明示宣言」は UI 経路にしか成立しないので、LLM 経路は入口で断る。
/// **リーダーの OCR ボタン（`ocr_pdf` コマンド直呼び）はこのガードを受けない。**
/// ⚠ **読み取りが失敗したときは断る**（`unwrap_or(0)` にしない）── 誤って通すと課金と
/// 恒久的な封印が起き、誤って断ってもアプリの OCR ボタンが残る。安全側は断る方。
/// **「行が無い」ときだけ通す** ── `run_ocr` が同じ規則で引き直して「添付が無い」で落ちるので、
/// 同じ文言をここにも持たないため。読み取り自体の失敗（`Err`）と「行が無い」（`Ok(None)`）を
/// 同じ枝に潰すと、一過性の DB エラーのときだけガードが素通りする（`prepare_ocr` は
/// 同じクエリを投げ直すので、そこで成功すれば索引状態を一度も見ないまま OCR が走る）。
///
/// ⚠ **判定は入口 1 回だけ**で、実際の書き込みはラスタライズ（527 頁で数分）と全ページ転写
/// （時間単位）を挟んだ後に走る。その窓の間に他経路（p2 の自動 build / `index_missing_attachments`）が
/// 同じ添付に索引を張ると、「索引 0 だから安全」と判断した対象が書き込み時には索引済みになる
/// ── debt-64。`begin_ocr` の排他は OCR 同士しか直列化しない。
pub(crate) async fn guard_llm_ocr(
    pool: &sqlx::SqlitePool,
    entry_id: i64,
    attachment_id: Option<i64>,
) -> Option<String> {
    let target_id = match resolve_target_pdf(pool, entry_id, attachment_id).await {
        Ok(Some((id, _))) => id,
        // 対象の行が無い ── `run_ocr` の「添付が無い」エラーに委ねる。
        Ok(None) => return None,
        Err(e) => {
            return Some(format!(
                "could not look up the target attachment of entry {entry_id}: {e}. No OCR was \
                 started and nothing was billed. Refusing to OCR: running it blind could replace \
                 an existing text layer and bill every page. Ask the user to retry from the app."
            ))
        }
    };
    let indexed = match crate::db::fulltext::indexed_page_count(pool, target_id).await {
        Ok(n) => n,
        Err(e) => {
            return Some(format!(
                "could not read the index state of attachment {target_id} (entry {entry_id}): {e}. \
                 No OCR was started and nothing was billed. Refusing to OCR: running it blind \
                 could replace an existing text layer and bill every page. Ask the user to retry \
                 from the app."
            ))
        }
    };
    let mut refusal = refuse_ocr_of_indexed_attachment(entry_id, target_id, indexed)?;
    // 断るときは、同じ entry に**未索引の PDF 添付**があればその id を添える。
    // 添えないと「補遺だけ OCR する」に到達できない ── チャットの `get_entry` が返すのは
    // entry 単位の `has_fulltext` だけで、添付ごとの id を出す read ツールは
    // LCIR が構築済みの添付に限られるため（debt-65）。
    if let Ok(others) = unindexed_pdf_attachments(pool, entry_id, target_id).await {
        if !others.is_empty() {
            let ids = others.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(", ");
            refusal.push_str(&format!(
                " This entry has other PDF attachment(s) with no indexed text at all: {ids}. \
                 If the user asked about one of those (a scanned supplement, for example), call \
                 ocr_pdf again with that attachment_id — this guard is per attachment, not per entry."
            ));
        }
    }
    Some(refusal)
}

/// `entry_id` 配下で**全文索引が 1 行も無い** PDF 添付の id（`exclude` は除く）。
/// 拒否文に「代わりにこれなら OCR できる」を載せるために使う。
pub(crate) async fn unindexed_pdf_attachments(
    pool: &sqlx::SqlitePool,
    entry_id: i64,
    exclude: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT a.id FROM attachments a
         WHERE a.entry_id = ? AND a.id != ? AND a.mime_type = 'application/pdf'
           AND NOT EXISTS (SELECT 1 FROM fulltext f WHERE f.attachment_id = a.id)
         ORDER BY a.id",
    )
    .bind(entry_id)
    .bind(exclude)
    .fetch_all(pool)
    .await
}

/// 索引済みの添付を LLM 経路が OCR し直そうとしたときの拒否文（issue #42）。
/// `None` を返したときだけ OCR に進む。
///
/// **逃げ道を LLM に案内しない。** 以前の文言は「`pages` を渡せば特定ページだけ焼き直せる」と
/// 書いていたが、それはガードの迂回路そのものだった。ここで案内する先は人間の操作
/// （リーダーの OCR ボタン）であって、同じツールの別の引数ではない。
pub(crate) fn refuse_ocr_of_indexed_attachment(
    entry_id: i64,
    attachment_id: i64,
    indexed_pages: i64,
) -> Option<String> {
    if indexed_pages <= 0 {
        return None;
    }
    Some(format!(
        "attachment {attachment_id} (entry {entry_id}) already has {indexed_pages} indexed \
         page(s) of full text — call get_fulltext to read it. No OCR was started and nothing was \
         billed. OCR is only for scanned PDFs with no text layer: it bills every page, replaces \
         this attachment's text with the vision model's transcription, and can permanently mark \
         the attachment as OCR-sourced, so it is refused here for every page range. Note: if an \
         earlier OCR was stopped or failed partway, those {indexed_pages} page(s) may be only a \
         partial transcription. Re-transcribing an attachment that is already indexed is the \
         user's call, not yours: tell them to open this paper's PDF in LumenCite and press the \
         OCR button in the header there, which this guard does not cover. Do not name the \
         attachment by its number when you tell them — it is an internal id that is not shown \
         anywhere in the app."
    ))
}

/// OCR 対象の PDF 添付を決める規則（`attachment_id` 指定があればその添付・無ければ先頭の PDF）。
/// **ガードと [`prepare_ocr`] は必ずこの 1 本を通す** ── 別々に書くと
/// 「ガードが見た添付」と「実際に OCR する添付」がずれ、ガードが素通りになる。
pub(crate) async fn resolve_target_pdf(
    pool: &sqlx::SqlitePool,
    entry_id: i64,
    attachment_id: Option<i64>,
) -> Result<Option<(i64, String)>, sqlx::Error> {
    match attachment_id {
        // 複数 PDF のとき「常に先頭」を OCR してしまわないよう、UI からは選択中の添付 id を渡す（CR-027）。
        Some(att_id) => {
            sqlx::query_as(
                "SELECT id, file_path FROM attachments
                 WHERE id = ? AND entry_id = ? AND mime_type = 'application/pdf'",
            )
            .bind(att_id)
            .bind(entry_id)
            .fetch_optional(pool)
            .await
        }
        None => {
            sqlx::query_as(
                "SELECT id, file_path FROM attachments
                 WHERE entry_id = ? AND mime_type = 'application/pdf' ORDER BY id LIMIT 1",
            )
            .bind(entry_id)
            .fetch_optional(pool)
            .await
        }
    }
}

/// 長い OCR の外部依存（停止要求と進捗）。**両方とも関数で受ける** ──
/// bool を渡すと開始時のスナップショットで凍り、実行中に押した停止が永久に届かない。
pub struct OcrHooks<'a> {
    /// 呼び出し元固有の中断要求（チャットの停止ボタンなど）。
    /// **これに加えて [`request_cancel`] のプロセス内フラグも必ず見る**ので、
    /// どの経路から始めた OCR も `cancel_ocr` で止まる。
    pub should_stop: &'a (dyn Fn() -> bool + Send + Sync),
    /// 1 ページ終わるごとに `(done, total)`。
    pub on_progress: &'a (dyn Fn(i64, i64) + Send + Sync),
}

/// 呼び出し元が持つ停止手段・進捗の受け口を OCR ループのフックへ詰め替える。
///
/// **起動口（`try_execute` / `ocr_pdf` コマンド）の中に埋めない。** 埋めると
/// `OcrHooks::none()` へ潰す変異が 1 本もテストを落とさない ── どちらの起動口も
/// `run_ocr` が添付の探索やキーチェーンで先に落ちるのでフックまで到達せず、
/// **チャットの停止ボタンだけが黙って効かなくなる**回帰が素通りする。
/// そこで [`run_ocr`] が引数から**ここで**組み立て、起動口には潰しどころを残さない。
pub(crate) fn hooks_for<'a>(
    should_stop: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
    on_progress: Option<&'a (dyn Fn(i64, i64) + Send + Sync)>,
) -> OcrHooks<'a> {
    OcrHooks {
        should_stop: should_stop.unwrap_or(&|| false),
        on_progress: on_progress.unwrap_or(&|_, _| {}),
    }
}

impl OcrHooks<'_> {
    /// 呼び出し元固有の停止手段も進捗も持たない場合。**`request_cancel` による停止は
    /// それでも効く。** 本番の起動口は [`hooks_for`] を通るので、これはテスト専用
    /// （`run_ocr` に直接フックを渡す口が無くなったため）。
    #[cfg(test)]
    pub fn none() -> OcrHooks<'static> {
        OcrHooks { should_stop: &|| false, on_progress: &|_, _| {} }
    }
}

/// 実行中の OCR があるか。**起動口は 2 つ（リーダーのボタン / チャットの `ocr_pdf`）**あり、
/// 排他は [`run_ocr`] の中で取るので**どちらから来ても 1 本に絞られる**。
static OCR_RUNNING: AtomicBool = AtomicBool::new(false);
/// 実行中の OCR への中断要求。**開始時に必ず倒す**（前回の押し忘れを引き継がない）。
static OCR_CANCEL: AtomicBool = AtomicBool::new(false);

/// 走っている OCR を次のページ境界で止める。どの経路から始まったものでも止まる。
pub fn request_cancel() {
    OCR_CANCEL.store(true, Ordering::SeqCst);
}

/// `OCR_RUNNING` を Drop で必ず倒す。途中 return・panic・future の drop でも残らない。
struct OcrRunGuard;
impl Drop for OcrRunGuard {
    fn drop(&mut self) {
        OCR_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// 1 本ぶんの実行権。排他・中断フラグの初期化・`batch_status` の印を**まとめて 1 か所**で取る。
///
/// `run_ocr` の入り口はこれだけ。ばらすと「排他は取ったが印を立て忘れた」のような
/// 片肺の変異が観測されずに残る（②b の配線 survivor は全部この形だった）。
///
/// ⚠ **フィールドの順序が正しさの一部。** drop は宣言順なので、`_mark` を先に置いて
/// **印を消してから排他フラグを離す**（取得の逆順）。逆にすると、フラグが離れてから
/// 印が消えるまでの数命令の窓で次のランが排他を取れてしまい、`RunningMark::new` の
/// 重複排除と旧ランの `retain` が相殺して**新しいランの印が誰にも立たない**
/// （＝設定 → データの停止ボタンが出ないまま 527 ページ課金し切る）。
struct OcrRun {
    _mark: crate::batch_status::RunningMark,
    _guard: OcrRunGuard,
}

/// 排他を取り、前回の中断要求を倒し、「OCR 実行中」の印を立てる。
/// 2 本目は [`OCR_ALREADY_RUNNING`] で弾く ── 起動口が 2 つあるので呼び出し側に置くと
/// 片方が素通りし、同じ添付に 2 本走って課金が倍になる（PR-1b のレビュー指摘）。
fn begin_ocr() -> Result<OcrRun, ToolError> {
    if OCR_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(ToolError::Execution(OCR_ALREADY_RUNNING.to_string()));
    }
    let guard = OcrRunGuard;
    // 前回の押し忘れを引き継がない（引き継ぐと 1 ページも処理せず終わる）。
    OCR_CANCEL.store(false, Ordering::SeqCst);
    Ok(OcrRun {
        _mark: crate::batch_status::RunningMark::new(crate::batch_status::BatchKind::Ocr),
        _guard: guard,
    })
}

/// ページ 1 枚を文字起こしする外部呼び出し。**課金はここでだけ発生する。**
///
/// trait にしているのは、[`transcribe_and_save`] を**テストできるようにする**ため。
/// 実装を注入できないと、停止・部分保存・失敗時の保全といった
/// この PR の主張が 1 本もテストできない（`#[tauri::command]` の本体と同じ袋小路 = debt-38）。
#[async_trait::async_trait]
pub(crate) trait PageTranscriber: Send + Sync {
    async fn transcribe(&self, png_base64: &str) -> Result<String, String>;
}

/// 本番の実装（Anthropic / OpenAI の Vision）。
struct VisionTranscriber {
    provider: String,
    model: String,
    api_key: String,
}

#[async_trait::async_trait]
impl PageTranscriber for VisionTranscriber {
    async fn transcribe(&self, png_base64: &str) -> Result<String, String> {
        ocr::ocr_image(&self.provider, &self.model, &self.api_key, "image/png", png_base64)
            .await
            .map_err(|e| e.to_string())
    }
}

/// [`transcribe_and_save`] の結果。**課金した枚数（processed）と索引に残した枚数（saved）を
/// 分けて持つ** ── Vision は白紙ページに空文字を返す（システムプロンプトがそう指示している）ので、
/// 2 つは正規の運用で食い違う。混ぜると「全ページ空白の課金ランが成功に見える」。
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize)]
pub(crate) struct OcrOutcome {
    /// 今回 API を呼んで課金したページ数。
    pub processed: i64,
    /// 本文が取れて索引に残したページ数（空白だけのページは含まない）。
    pub saved: i64,
    /// 今回処理する予定だった枚数。
    pub planned: i64,
    /// 停止要求で降りたか。
    pub stopped: bool,
    /// 失敗して降りたときのエラー。
    pub failure: Option<String>,
    /// 失敗した実ページ番号（1 始まり）。`pages` 指定の部分 OCR では
    /// 通し番号と実ページ番号がずれるので、通し番号から推定してはいけない。
    pub failed_page: Option<i64>,
    /// 保存を部分差し替えにしたか（`false` = 添付ごと置き換え）。
    pub partial: bool,
}

/// **ループ本体。ここが OCR の実質すべてで、唯一テストできる形になっている。**
///
/// - 毎ページ停止要求を見る（1 ページ = 1 課金なので、境界はページ）
/// - 途中で降りても**課金済みのページは保存する**
/// - ⚠ **届かなかったページがあるなら必ず部分差し替え。**
///   添付ごと置き換えを選ぶと、届かなかったページの既存本文が消える
/// - **封印（`fulltext.source = Ocr`）は完走したときだけ**（[`save_ocr_pages`] の `seal`）
/// - 進捗（`batch_status`）と直近の結果もここで書く ── アダプタ（`run_ocr`）や
///   コマンド側に置くと、片方の起動口だけが素通りする配線を作れてしまう
pub(crate) async fn transcribe_and_save<T: PageTranscriber + ?Sized>(
    pool: &sqlx::SqlitePool,
    attachment_id: i64,
    images: Vec<(i64, String)>,
    explicit_pages: bool,
    transcriber: &T,
    hooks: &OcrHooks<'_>,
) -> Result<OcrOutcome, ToolError> {
    let planned = images.len() as i64;
    let mut results: Vec<(i64, String)> = Vec::with_capacity(images.len());
    let mut out = OcrOutcome { planned, ..Default::default() };

    // **総数が分かった時点で 1 回報告する**（ゲート ②b の F-2）。1 ページ目の転写は
    // 数秒〜数十秒かかり、その間だけ分母が出ない。OCR は画面をまたいだ停止手段が
    // この表示にぶら下がっているので（`batch_status::BatchKind::Ocr`）、盲窓は短いほどよい。
    crate::batch_status::set_progress(crate::batch_status::BatchKind::Ocr, 0, planned);
    (hooks.on_progress)(0, planned);

    for (page_no, b64) in images {
        if OCR_CANCEL.load(Ordering::SeqCst) || (hooks.should_stop)() {
            out.stopped = true;
            break;
        }
        match transcriber.transcribe(&b64).await {
            Ok(text) => {
                out.processed += 1;
                // **空白だけの転写は保存に回さない。** 部分差し替えで空ページを渡すと
                // 「そのページの既存行を削除して何も入れない」になり、中断ランが
                // pdf_extract や前回の課金済み本文を黙って消す（さらに全行が消えると
                // 封印まで剥がれる）。課金は発生しているので processed には数える。
                if !text.trim().is_empty() {
                    results.push((page_no, text));
                }
            }
            Err(e) => {
                out.failure = Some(e);
                out.failed_page = Some(page_no);
                break;
            }
        }
        crate::batch_status::set_progress(crate::batch_status::BatchKind::Ocr, out.processed, planned);
        (hooks.on_progress)(out.processed, planned);
    }

    out.saved = results.len() as i64;
    if results.is_empty() {
        // 保存するものが無い ── 既存の索引にも封印にも一切触らない。
        crate::batch_status::record_success(crate::batch_status::BatchKind::Ocr, &out);
        return Ok(out);
    }

    out.partial = ocr_save_is_partial(explicit_pages, out.processed, planned);
    let seal = ocr_save_should_seal(explicit_pages, out.stopped, out.failure.is_some());
    if let Err(e) = save_ocr_pages(pool, attachment_id, &results, out.partial, seal).await {
        crate::batch_status::record_failure(crate::batch_status::BatchKind::Ocr, &e.to_string());
        return Err(e.into());
    }
    // 他の 6 種と同じく「直近の結果」に載せる ── 画面を離れている間に終わっても
    // 何ページ処理したかを後から読めるようにする（debt-32）。
    crate::batch_status::record_success(crate::batch_status::BatchKind::Ocr, &out);
    Ok(out)
}

/// entry の PDF 添付を OCR して `fulltext` に保存する。
///
/// **この関数はアダプタ**（添付の特定・プロバイダ解決・ラスタライズ）で、
/// 判定とループは [`transcribe_and_save`] にある。分けているのは、ここが
/// pdfium と HTTP とキーチェーンに触るためテストが届かないから。
pub async fn run_ocr(
    pool: &sqlx::SqlitePool,
    app_data_dir: &Path,
    entry_id: i64,
    attachment_id: Option<i64>,
    pages: Option<Vec<i64>>,
    should_stop: Option<&(dyn Fn() -> bool + Send + Sync)>,
    on_progress: Option<&(dyn Fn(i64, i64) + Send + Sync)>,
) -> Result<String, ToolError> {
    // フックはここで組む（起動口に判定を置かない・[`hooks_for`] の doc を見よ）。
    let hooks = hooks_for(should_stop, on_progress);
    // 排他・中断フラグ・実行中の印。**どの起動口から来てもここを通る。**
    let _run = begin_ocr()?;

    // **ループ手前の失敗（添付なし・キー未設定・ラスタライズ失敗…）も「直近の結果」に残す。**
    // 残さないと、リーダーを離れた後に失敗したラン（527 頁本のラスタライズは数分かかる）が
    // どの画面からも読めない ── 「OCR 実行中」が黙って消えるだけになる。
    // `already_running` はここに来ない（`begin_ocr` が印を作る前に返す）ので、
    // 「弾かれた呼び出しは本物の結果を上書きしない」契約（`FinishedBatch`）はそのまま。
    let (attachment_id, images, transcriber) =
        match prepare_ocr(pool, app_data_dir, entry_id, attachment_id, &pages).await {
            Ok(p) => p,
            Err(e) => {
                crate::batch_status::record_failure(
                    crate::batch_status::BatchKind::Ocr,
                    &e.to_string(),
                );
                return Err(e);
            }
        };

    // ループ本体（テスト可能な側）。進捗・成否の記録はループ自身が書く。
    let out =
        transcribe_and_save(pool, attachment_id, images, pages.is_some(), &transcriber, &hooks)
            .await?;

    Ok(describe_outcome(entry_id, &out))
}

/// ループ手前の段取り: 対象添付の特定 → プロバイダ/キー解決 → ラスタライズ。
/// ここの失敗は [`run_ocr`] が `record_failure` に通す（この関数は記録しない）。
async fn prepare_ocr(
    pool: &sqlx::SqlitePool,
    app_data_dir: &Path,
    entry_id: i64,
    attachment_id: Option<i64>,
    pages: &Option<Vec<i64>>,
) -> Result<(i64, Vec<(i64, String)>, VisionTranscriber), ToolError> {
    // 1. 対象 PDF 添付（CR-027）。**規則は [`resolve_target_pdf`] に 1 本化してある** ──
    //    チャット側のガードが同じ規則で対象を決めるので、ここで別に書くとガードが
    //    「別の添付を見て通す」形になりうる。
    let row = resolve_target_pdf(pool, entry_id, attachment_id).await?;
    let (attachment_id, file_path) = row
        .ok_or_else(|| ToolError::Execution(format!("entry {entry_id} has no matching PDF attachment")))?;
    let abs_path = app_data_dir.join(&file_path);

    // 2. OCR プロバイダ/モデル（未設定なら chat 用にフォールバック）+ API キー
    let (provider, model) = resolve_ocr_provider(pool).await?;
    let account = keychain::account_for_api_key(&provider);
    let api_key = keychain::get(&account)
        .map_err(|e| ToolError::Execution(e.to_string()))?
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| ToolError::Execution(format!("API key for {provider} is not configured")))?;

    // 3. ラスタライズ（pdfium・同期）。**必ず `spawn_blocking` の下で回す**（v1.0.0-p2）──
    //    pdfium-render の `thread_safe` は `FPDF_InitLibrary` から `FPDF_DestroyLibrary` まで
    //    `std::sync::Mutex` を握り続けるので、async fn の中で直接呼ぶと 1 冊ぶんのレンダリングが
    //    終わるまで tokio のワーカースレッドを 1 本占有する。p2 で LCIR の自動 build が
    //    常時走るようになると、その 1 本がランタイム全体を巻き添えにする。
    //    この間 `batch_status` は「OCR 実行中・進捗なし」＝設定 → データは「準備中」を出す。
    //    停止を押してもここでは降りず、ループの 1 ページ目の手前で降りる（課金 0 で済む）。
    let rasterize_path = abs_path.clone();
    let rasterize_pages = pages.clone();
    let images = tokio::task::spawn_blocking(move || {
        rasterize(&rasterize_path, rasterize_pages.as_deref())
    })
    .await
    .map_err(|e| {
        // OCR のラスタライズも pdfium を触るので、panic すると marshall が毒され、以後
        // このプロセスの `Pdfium::new` は Err ではなく panic する。自動経路（LCIR の
        // 自動 build / バックフィル）が残りを焼き切らないよう、ここでも印を立てる。
        crate::ingestion::pdf::pdfium::note_extraction_panic();
        ToolError::Execution(format!("rasterize task panicked: {e}"))
    })??;
    if images.is_empty() {
        return Err(ToolError::Execution("no pages to OCR".into()));
    }

    Ok((attachment_id, images, VisionTranscriber { provider, model, api_key }))
}

/// 結果を人（と LLM）に説明する文字列。**再開は実装していないので、「続きから」とは言わない**
/// ── 中断・失敗からもう一度実行すると最初からやり直しで、全ページが課金し直しになる。
/// ここで嘘をつくと、LLM が「続きを取ろう」と再実行して二重課金を起こす。
pub(crate) fn describe_outcome(entry_id: i64, out: &OcrOutcome) -> String {
    const RERUN_STARTS_OVER: &str = " Running OCR again does NOT resume: it starts over from the \
         first page and every page is billed again.";
    // 失敗位置は**実ページ番号**で言う。`pages` 指定の部分 OCR では通し番号と
    // 実ページ番号がずれ、通し番号で案内すると取り直しで別のページを再課金させる。
    if out.saved == 0 {
        return match &out.failure {
            Some(e) => {
                let at = out
                    .failed_page
                    .map(|p| format!(" on page {p}"))
                    .unwrap_or_default();
                format!(
                    "OCR failed{at} before any page could be saved for entry {entry_id}: {e}. \
                     The existing index was not touched."
                )
            }
            None if out.stopped && out.processed == 0 => format!(
                "OCR was stopped before any page was processed for entry {entry_id}; nothing \
                 changed and nothing was billed."
            ),
            None if out.stopped => format!(
                "OCR was stopped after {} page(s) for entry {entry_id}; none of them contained \
                 text, so the existing index was not touched.",
                out.processed
            ),
            // 全ページ課金したのに 1 行も残らなかった ── 成功に見せない（唯一の異常シグナル）。
            None if out.processed > 0 => format!(
                "OCR processed {} page(s) for entry {entry_id} but found no text on any of \
                 them; nothing was indexed and the existing index was not touched.",
                out.processed
            ),
            None => format!("OCR had nothing to do for entry {entry_id}."),
        };
    }
    if out.stopped {
        format!(
            "OCR stopped at {}/{} page(s) for entry {entry_id}; the {} page(s) with text were \
             saved.{RERUN_STARTS_OVER}",
            out.processed, out.planned, out.saved
        )
    } else if let Some(e) = &out.failure {
        let at = out
            .failed_page
            .map(|p| format!(" on page {p}"))
            .unwrap_or_default();
        format!(
            "OCR failed{at} after {}/{} page(s) for entry {entry_id}: {e}. The {} page(s) with \
             text transcribed before the failure were saved.{RERUN_STARTS_OVER}",
            out.processed, out.planned, out.saved
        )
    } else {
        format!(
            "OCR'd {} page(s) for entry {entry_id}; {} page(s) contained text and were indexed.",
            out.processed, out.saved
        )
    }
}

/// 保存を**部分差し替え**にするか（`true`）**添付ごと置き換え**にするか（`false`）。
///
/// ⚠ **ここを間違えると既存の索引が消える。** `false` は
/// 「この添付の索引を丸ごと入れ替える」意味なので、527 ページ中 3 ページで中断したのに
/// `false` を選ぶと、**残り 524 ページぶんの既存索引を削除して 3 ページだけ残す**。
/// 全ページを最後まで処理し切ったときだけ `false` にしてよい。
///
/// 判定を関数に出しているのは、`run_ocr` の中に埋めると `#[tauri::command]` 経由でしか
/// 到達できずテストが届かないため（ゲート ②b の debt-38 と同じ理由）。
///
/// 判定は **processed（課金して処理した枚数）**で取る。saved（本文が残った枚数）で取ると、
/// 白紙ページを含む本が完走しても「部分」扱いになり、完走時の全置換の意味論が壊れる。
pub(crate) fn ocr_save_is_partial(explicit_pages: bool, processed: i64, planned: i64) -> bool {
    explicit_pages || processed < planned
}

/// この保存で添付ごと「OCR 由来」を封印してよいか（p1・§2.6-1）。
///
/// 条件は 2 つ。**①頼まれたぶんを最後まで処理し切った**（中断・失敗では立てない ──
/// 立てると 3 ページで止めた 527 ページの本が添付ごと「OCR 由来」になり、以後の再索引も
/// LCIR 派生も譲ってしまう）。**②ページを名指ししていない。**
///
/// ②が要るのは、**封印が添付単位**だから ── ページ指定の部分 OCR で立てると
/// **1 ページの転写が添付全体の索引を恒久ロックする**（500 頁の PDF に `pages:[3]` を通すと、
/// 残り 499 頁は pdf_extract でも LCIR 派生でも二度と索引されない ──
/// `index_fulltext_for_attachment` が段 1 で `SkippedOcr` を返す）。
/// §2.6-1 は「部分 OCR でも添付ごと skip」を保守側への倒し込みとして選んだが、その根拠
/// （ユーザーが「この PDF のテキスト層は信用できない」と宣言した）は**添付全体を焼く操作**に
/// しか成立しない。ページ指定に到達できるのは今のところチャットの `ocr_pdf` だけで
/// （リーダーのボタンは常に全ページ）、そこでは対象とページを選ぶのが LLM なので、なおさら。
/// 封印しなかった課金済みページは debt-43 の母集団に入る（後の再索引に置き換えられうる）が、
/// 499 頁を恒久ロックするより害が小さい ── しかも純粋なスキャン本では LCIR も pdf_extract も
/// 本文が空なので、既存行を残す規則により置き換えは起きない。
pub(crate) fn ocr_save_should_seal(explicit_pages: bool, stopped: bool, failed: bool) -> bool {
    !explicit_pages && !stopped && !failed
}

/// OCR 結果を保存する。全ページ OCR なら添付ごと置き換え、部分 OCR なら該当ページのみ差し替え。
///
/// `seal` が真のときだけ、**この添付の索引は OCR 由来**だと記録する（p1）。記録しないと、
/// LCIR からの派生や添付経路の pdf_extract が後から上書きしてしまう。ページ単位の部分 OCR でも
/// 添付ごと保護されるのは承知の上での保守側への倒し込み（§2.6-1）── ユーザーが OCR を回した
/// 添付は「この PDF のテキスト層は信用できない」と明示的に宣言した添付だから。
///
/// ⚠ **`seal` は「頼まれたぶんを最後まで処理し切った」ときだけ真にすること。**
/// 中断・失敗で封印すると、pdf_extract が全ページに索引済みのスキャン本を 3 ページで止めた
/// だけで添付全体が「OCR 由来」になり、以後の再索引も LCIR 派生も譲ってしまう ──
/// 文字化けした 524 ページが全文検索の正本として恒久的に固定される（ページ単位の
/// provenance は存在しないので、封印を立ててよいのは完走した回だけ）。
pub(crate) async fn save_ocr_pages(
    pool: &sqlx::SqlitePool,
    attachment_id: i64,
    results: &[(i64, String)],
    partial: bool,
    seal: bool,
) -> Result<(), sqlx::Error> {
    if partial {
        crate::db::fulltext::update_attachment_pages(pool, attachment_id, results).await?;
    } else {
        crate::db::fulltext::index_attachment(pool, attachment_id, results).await?;
    }
    // **中身を 1 行も残せなかった OCR では記録しない（残さない）。** 空文字のページは索引に
    // 入らないので（`replace_pages` / `update_attachment_pages` は空をスキップする）、記録だけ
    // 立てると「中身 0 行の索引を守り続ける」状態になり、その添付は再索引もできなくなる。
    // 封印しない保存でも、空になったのに古い記録が残る形は同じ理由で刈る。
    if crate::db::fulltext::indexed_page_count(pool, attachment_id).await? == 0 {
        return crate::db::fulltext::clear_fulltext_source(pool, attachment_id).await;
    }
    if !seal {
        return Ok(());
    }
    crate::db::fulltext::set_fulltext_source(
        pool,
        attachment_id,
        crate::db::fulltext::FulltextSource::Ocr,
    )
    .await
}

/// OCR / Vision 用のプロバイダとモデル（未設定なら chat 用にフォールバック）。
/// Phase 8c の alt text 生成バッチも同じ設定を共有する（Vision 用の設定面を増やさない）。
pub(crate) async fn resolve_ocr_provider(
    pool: &sqlx::SqlitePool,
) -> Result<(String, String), ToolError> {
    use crate::db::settings::{
        get_setting, LLM_MODEL_KEY, LLM_OCR_MODEL_KEY, LLM_OCR_PROVIDER_KEY, LLM_PROVIDER_KEY,
    };
    let provider = match get_setting(pool, LLM_OCR_PROVIDER_KEY).await? {
        Some(p) if !p.trim().is_empty() => p,
        _ => get_setting(pool, LLM_PROVIDER_KEY)
            .await?
            .unwrap_or_else(|| "openai".to_string()),
    };
    let model = match get_setting(pool, LLM_OCR_MODEL_KEY).await? {
        Some(m) if !m.trim().is_empty() => m,
        _ => get_setting(pool, LLM_MODEL_KEY).await?.unwrap_or_else(|| {
            match provider.as_str() {
                "anthropic" => "claude-haiku-4-5-20251001".to_string(),
                _ => "gpt-4o-mini".to_string(),
            }
        }),
    };
    Ok((provider, model))
}

/// PDF をページ画像（PNG base64）に。`pages` は 1 始まり、None で全ページ。
fn rasterize(path: &Path, pages: Option<&[i64]>) -> Result<Vec<(i64, String)>, ToolError> {
    use pdfium_render::prelude::*;
    let bindings = crate::ingestion::pdf::pdfium::bind_pdfium().map_err(ToolError::Execution)?;
    let pdfium = Pdfium::new(bindings);
    let doc = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| ToolError::Execution(format!("failed to open PDF: {e}")))?;
    let config = PdfRenderConfig::new().set_target_width(1600);
    let mut out = Vec::new();
    for (idx, page) in doc.pages().iter().enumerate() {
        let page_no = idx as i64 + 1;
        if let Some(ps) = pages {
            if !ps.contains(&page_no) {
                continue;
            }
        }
        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| ToolError::Execution(format!("render failed on page {page_no}: {e}")))?;
        let dynimg = bitmap.as_image();
        let mut buf: Vec<u8> = Vec::new();
        dynimg
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .map_err(|e| ToolError::Execution(e.to_string()))?;
        out.push((page_no, base64::engine::general_purpose::STANDARD.encode(&buf)));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::db::fulltext::index_attachment;
    use crate::models::EntryInput;
    use sqlx::SqlitePool;

    /// **プロセス共有の static（`OCR_RUNNING` / `OCR_CANCEL` / batch_status の表）を触る
    /// テストは、モジュール横断の `batch_status::TEST_GATE` で直列化する** ── batch_status /
    /// `batch_wiring_tests` のテストと同じ表を読み書きするので、モジュールごとの別 gate では
    /// 「こちらが `RunningMark(Ocr)` を握っている間に、あちらの『誰も走っていない』前提
    /// assert が落ちる」窓が残る。そのうえで各テストは**自分が起こした遷移だけ**を assert する。
    /// ロック取得時に中断要求を必ず倒すのは、前のテストが assert 失敗で途中脱落しても
    /// 残骸を引き継がないため。
    async fn gate() -> tokio::sync::MutexGuard<'static, ()> {
        let g = crate::batch_status::TEST_GATE.lock().await;
        OCR_CANCEL.store(false, Ordering::SeqCst);
        g
    }

    /// ガードのテスト用の最小 entry。
    async fn new_entry(pool: &SqlitePool, title: &str) -> crate::models::EntryDetail {
        create_entry(
            pool,
            &EntryInput {
                title: title.to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    }

    /// 同期テスト用（async でない `#[test]` はランタイムを持たないので blocking で取る）。
    fn gate_blocking() -> tokio::sync::MutexGuard<'static, ()> {
        let g = crate::batch_status::TEST_GATE.blocking_lock();
        OCR_CANCEL.store(false, Ordering::SeqCst);
        g
    }

    // ── 純関数（static に触らない）────────────────────────────────────────

    /// **中断したら必ず部分保存。** 全体置き換えを選ぶと、処理できなかったページの
    /// 既存索引（前回の OCR 結果など）を消して数ページだけ残す。
    #[test]
    fn an_interrupted_run_never_replaces_the_whole_index() {
        // 527 ページの本を 3 ページで止めた ── ここで false を返してはいけない。
        assert!(
            ocr_save_is_partial(false, 3, 527),
            "中断したのに添付ごと置き換えると 524 ページぶんの既存索引が消える"
        );
        assert!(ocr_save_is_partial(false, 526, 527), "1 ページ足りなくても部分保存");
        assert!(ocr_save_is_partial(false, 0, 527), "1 ページも取れていない場合も");
    }

    /// 最後まで処理し切ったときだけ添付ごと置き換える（従来の挙動）。
    #[test]
    fn a_complete_first_run_replaces_the_attachment_index() {
        assert!(!ocr_save_is_partial(false, 527, 527));
        assert!(!ocr_save_is_partial(false, 1, 1));
    }

    /// ページ指定つきの部分 OCR は、完走しても部分差し替えのまま。
    #[test]
    fn an_explicit_page_selection_stays_partial_even_when_complete() {
        assert!(
            ocr_save_is_partial(true, 3, 3),
            "「3 ページだけ OCR して」で添付ごと置き換えたら残りが消える"
        );
    }

    /// 停止手段を持たない呼び出し元の既定は「止めない・進捗を出さない」。
    #[test]
    fn the_default_hooks_never_stop() {
        let h = OcrHooks::none();
        assert!(!(h.should_stop)(), "既定で止まってしまうと 1 ページも処理されない");
        (h.on_progress)(1, 2); // panic しないこと
    }

    /// **結果文言は「続きから再開できる」と言わない**（実装していないので）。
    /// 嘘をつくと LLM が「続きを取ろう」と再実行して、最初から全ページ課金し直す。
    #[test]
    fn an_interrupted_outcome_says_rerun_starts_over_not_resumes() {
        let stopped = OcrOutcome {
            processed: 3,
            saved: 3,
            planned: 527,
            stopped: true,
            ..Default::default()
        };
        let msg = describe_outcome(1, &stopped);
        assert!(!msg.contains("continues"), "「続きから」と読める文言を出してはいけない: {msg}");
        assert!(msg.contains("starts over"), "やり直しになることを明言する: {msg}");
        assert!(msg.contains("billed again"), "全ページ課金し直しになることを明言する: {msg}");

        let failed = OcrOutcome {
            processed: 2,
            saved: 2,
            planned: 10,
            failure: Some("provider exploded".into()),
            failed_page: Some(7),
            ..Default::default()
        };
        let msg = describe_outcome(1, &failed);
        assert!(msg.contains("starts over"), "失敗も同じ: {msg}");
        assert!(msg.contains("page 7"), "失敗位置は実ページ番号で言う: {msg}");
    }

    /// 完走した結果に「もう一度走らせろ」と読める文言を書かない（二重課金の誘発）。
    #[test]
    fn a_complete_outcome_does_not_invite_a_rerun() {
        let complete =
            OcrOutcome { processed: 527, saved: 527, planned: 527, ..Default::default() };
        let msg = describe_outcome(1, &complete);
        assert!(!msg.contains("again"), "完走したのに再実行を勧めない: {msg}");
    }

    /// **全ページ課金したのに 1 行も残らなかったランを成功に見せない。**
    /// 索引行数との乖離が読める唯一のシグナル（v2 レビュー confirmed[4] の解消）。
    #[test]
    fn an_all_blank_run_is_not_reported_as_plain_success() {
        let out = OcrOutcome { processed: 40, saved: 0, planned: 40, ..Default::default() };
        let msg = describe_outcome(1, &out);
        assert!(msg.contains("no text"), "空振りだったことを明言する: {msg}");
        assert!(msg.contains("existing index was not touched"), "{msg}");
    }

    // ── 保存と封印（save_ocr_pages）──────────────────────────────────────

    async fn an_attachment(pool: &SqlitePool) -> i64 {
        // 同一テスト内で 2 回呼んでも `attachments.file_path` の UNIQUE に当たらないよう
        // 連番で一意にする（DB はテストごとに新品なので値自体に意味は無い）。
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "Scanned book".to_string(),
                entry_type: "book".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        add_attachment(pool, entry.id, &format!("a/b{n}.pdf"), "b.pdf", "application/pdf")
            .await
            .unwrap()
            .id
    }

    /// p1: **完走した** OCR の保存は、全ページ / 部分ページのどちらでも「この添付は OCR 由来」を
    /// 記録する。記録しないと LCIR 派生や添付経路の pdf_extract が後から上書きし、
    /// ユーザーが課金して起こした転写が消える（§2.6-1）。
    #[sqlx::test(migrations = "./migrations")]
    async fn saving_ocr_marks_the_attachment_as_ocr_sourced(pool: SqlitePool) {
        let full = an_attachment(&pool).await;
        let partial = an_attachment(&pool).await;

        save_ocr_pages(&pool, full, &[(1, "full transcript".to_string())], false, true)
            .await
            .unwrap();
        save_ocr_pages(&pool, partial, &[(3, "page 3 only".to_string())], true, true)
            .await
            .unwrap();

        for att in [full, partial] {
            assert_eq!(
                crate::db::fulltext::get_fulltext_source(&pool, att)
                    .await
                    .unwrap(),
                Some(crate::db::fulltext::FulltextSource::Ocr),
                "attachment {att} が OCR 由来として記録されていない"
            );
        }
    }

    /// **完走しなかった保存はページを残すが、封印はしない。** 添付単位の provenance を
    /// ページ単位の判断に流用できないので、中断・失敗の回で `source = Ocr` を立てると、
    /// pdf_extract が書いた残り 524 ページまで「OCR 由来」として保護してしまう。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unfinished_save_keeps_the_pages_but_does_not_seal(pool: SqlitePool) {
        let att = an_attachment(&pool).await;

        save_ocr_pages(&pool, att, &[(1, "page 1".to_string()), (2, "page 2".to_string())], true, false)
            .await
            .unwrap();

        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att).await.unwrap(),
            2,
            "課金済みのページは保存される"
        );
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att).await.unwrap(),
            None,
            "完走していないのに封印してはいけない"
        );
    }

    /// **中身を 1 行も残せなかった OCR は「OCR 由来」と記録しない。**
    /// 記録だけ立てると、中身 0 行の索引を守り続けて、その添付は再索引もできなくなる。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_ocr_that_transcribed_nothing_does_not_mark_the_attachment(pool: SqlitePool) {
        let att = an_attachment(&pool).await;

        save_ocr_pages(&pool, att, &[(1, "   ".to_string())], false, true)
            .await
            .unwrap();

        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att).await.unwrap(),
            0
        );
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att).await.unwrap(),
            None,
            "守る中身が無いのに記録を立ててはいけない"
        );
    }

    // ── ループ本体（transcribe_and_save・PageTranscriber を注入）──────────
    //
    // **ここが PR-1b の実体。** 以前は `#[tauri::command]` の下に埋まっていて
    // 「停止チェックを丸ごと消しても全テスト green」だった（レビューの変異 m2）。
    // ページ 1 枚の文字起こしを注入できるようにしたので、ループ全体を検証できる。

    /// 呼ばれた回数を数える偽の文字起こし。**課金の回数 = ここの呼び出し回数**。
    struct FakeTranscriber {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fail_on_call: Option<usize>,
        /// true なら全ページ空白を返す（Vision は白紙ページにそうするよう指示されている）。
        blank: bool,
    }

    #[async_trait::async_trait]
    impl PageTranscriber for FakeTranscriber {
        async fn transcribe(&self, _png: &str) -> Result<String, String> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if self.fail_on_call == Some(n) {
                return Err("provider exploded".into());
            }
            if self.blank {
                return Ok("   \n".to_string());
            }
            Ok(format!("page text {n}"))
        }
    }

    fn fake(
        fail_on_call: Option<usize>,
    ) -> (FakeTranscriber, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (FakeTranscriber { calls: calls.clone(), fail_on_call, blank: false }, calls)
    }

    fn fake_blank() -> FakeTranscriber {
        FakeTranscriber {
            calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            fail_on_call: None,
            blank: true,
        }
    }

    fn images(n: i64) -> Vec<(i64, String)> {
        (1..=n).map(|p| (p, format!("b64-{p}"))).collect()
    }

    /// **1 ページ目を転写する前に分母を出す**（ゲート ②b の F-2）。
    ///
    /// OCR は画面をまたいだ停止手段が `batch_status` の進捗表示にぶら下がっているので、
    /// 「何件中の何件目か」が出ない盲窓は短いほどよい。1 ページ目の往復は数秒〜数十秒。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_page_count_is_reported_before_the_first_transcription(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _calls) = fake(None);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(i64, i64)>::new()));
        let s2 = seen.clone();
        let prog = move |done: i64, total: i64| s2.lock().unwrap().push((done, total));
        let stop = || false;
        let hooks = OcrHooks { should_stop: &stop, on_progress: &prog };

        // 本番と同じく実行中の印を握った状態で呼ぶ（Drop が進捗表を掃除するのもここ）。
        let mark = crate::batch_status::RunningMark::new(crate::batch_status::BatchKind::Ocr);
        transcribe_and_save(&pool, att, images(3), false, &t, &hooks)
            .await
            .unwrap();
        drop(mark);

        assert_eq!(
            seen.lock().unwrap().first().copied(),
            Some((0, 3)),
            "最初の 1 通は「0 ページ処理済み・全 3 ページ」"
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec![(0, 3), (1, 3), (2, 3), (3, 3)],
            "開始の報告が 1 ページ目の報告を置き換えていない"
        );
    }

    /// 同じ開始報告が**バックエンドの正本にも**載る（ゲート ②b の F-2 / debt-32）。
    ///
    /// **ループが 1 度も回らない入力で撮る。** 1 ページでも回ると最後の 1 通が
    /// 同じキーを上書きするので、開始報告を消す変異と区別がつかない
    /// （[[feedback_gate_tests_go_vacuous]] の逆で、ここは「回らない入力」が必要）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_opening_report_reaches_the_backend_progress_table(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _calls) = fake(None);
        // **1 ページ目に入る前に降りる。** こうするとループの `set_progress` が
        // 開始報告を上書きしないので、**分母まで**そのまま観測できる ── `images(0)` で
        // 見ていた頃は total が 0 で、`(0, planned)` を `(0, 0)` に変える変異が
        // 素通りした（PR-3 のレビュー指摘。盲窓を消すのは分母であって 0 ではない）。
        let stop = || true;
        let hooks = OcrHooks { should_stop: &stop, on_progress: &|_, _| {} };

        let mark = crate::batch_status::RunningMark::new(crate::batch_status::BatchKind::Ocr);
        transcribe_and_save(&pool, att, images(3), false, &t, &hooks)
            .await
            .unwrap();
        let progress = crate::batch_status::snapshot().progress.get("ocr").copied();
        drop(mark);

        assert_eq!(
            progress,
            Some(crate::batch_status::Progress { done: 0, total: 3 }),
            "モーダルを開き直したフロントは、1 ページ目を待っている間もここを読む \
             ── そこに載るのは 0 ではなく**ページ数**"
        );
    }

    /// **停止したらそこで課金が止まる。** 527 ページ中 2 ページで降りたら
    /// API は 2 回しか呼ばれない（以前は最後まで呼び続けていた）。
    #[sqlx::test(migrations = "./migrations")]
    async fn stopping_halts_the_billing_at_the_next_page(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, calls) = fake(None);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let d2 = done.clone();
        // 2 ページ処理したら停止を要求する。
        let stop = move || d2.load(std::sync::atomic::Ordering::SeqCst) >= 2;
        let d3 = done.clone();
        let prog = move |n: i64, _t: i64| d3.store(n as usize, std::sync::atomic::Ordering::SeqCst);
        let hooks = OcrHooks { should_stop: &stop, on_progress: &prog };

        let out = transcribe_and_save(&pool, att, images(527), false, &t, &hooks)
            .await
            .unwrap();

        assert!(out.stopped, "停止要求で降りる");
        assert_eq!(out.saved, 2);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2, "課金は 2 回だけ");
        assert!(out.partial, "中断したら添付ごと置き換えない");
        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att).await.unwrap(),
            2,
            "課金済みの 2 ページは保存される"
        );
    }

    /// **途中で失敗しても、それまでに課金したページは残る。**
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failure_keeps_the_pages_already_paid_for(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, calls) = fake(Some(3)); // 3 回目で失敗
        let hooks = OcrHooks::none();
        let out = transcribe_and_save(&pool, att, images(10), false, &t, &hooks)
            .await
            .unwrap();

        assert_eq!(out.failure.as_deref(), Some("provider exploded"));
        assert_eq!(out.saved, 2, "失敗した 3 ページ目より前の 2 ページ");
        assert_eq!(out.failed_page, Some(3), "失敗した実ページ番号が残る");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att).await.unwrap(),
            2,
            "課金済みを捨てない"
        );
    }

    /// **完走したときだけ添付ごと置き換えて封印する。**
    #[sqlx::test(migrations = "./migrations")]
    async fn a_complete_run_replaces_and_seals_the_attachment(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _) = fake(None);
        let hooks = OcrHooks::none();
        let out = transcribe_and_save(&pool, att, images(4), false, &t, &hooks)
            .await
            .unwrap();

        assert!(!out.partial, "完走したら置き換え");
        assert_eq!(out.saved, 4);
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att).await.unwrap(),
            Some(crate::db::fulltext::FulltextSource::Ocr),
            "課金して起こした転写は OCR 由来として守る"
        );
    }

    /// **ページを名指しした回は、完走しても封印しない**（実経路で固定する）。
    /// 封印は添付単位なので、頼まれた 2 ページを処理し切っただけで立てると、
    /// その PDF の残りのページが pdf_extract でも LCIR 派生でも二度と索引されなくなる。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_completed_page_range_does_not_seal_the_attachment(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _) = fake(None);
        let hooks = OcrHooks::none();
        let pages = vec![(3, "b".to_string()), (7, "b".to_string())];

        let out = transcribe_and_save(&pool, att, pages, true, &t, &hooks).await.unwrap();

        assert!(out.partial, "ページ指定は常に部分差し替え");
        assert_eq!(out.saved, 2, "頼まれた 2 ページは保存する（課金したので捨てない）");
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att).await.unwrap(),
            None,
            "2 ページの転写で添付全体を「OCR 由来」にすると、残りが恒久的に索引されなくなる"
        );
    }

    /// **中断した回は封印しない**（§2.2 の事故の再発防止）。
    /// pdf_extract が全 527 ページに文字化けを索引済みのスキャン本を 3 ページで止めても、
    /// 添付全体が「OCR 由来」になって文字化けが恒久固定される、を二度と作らない。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_interrupted_run_does_not_seal_the_attachment(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _) = fake(None);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let d2 = done.clone();
        let stop = move || d2.load(std::sync::atomic::Ordering::SeqCst) >= 2;
        let d3 = done.clone();
        let prog = move |n: i64, _t: i64| d3.store(n as usize, std::sync::atomic::Ordering::SeqCst);
        let hooks = OcrHooks { should_stop: &stop, on_progress: &prog };

        let out = transcribe_and_save(&pool, att, images(5), false, &t, &hooks)
            .await
            .unwrap();

        assert!(out.stopped);
        assert_eq!(out.saved, 2, "課金済みの 2 ページは保存される");
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att).await.unwrap(),
            None,
            "中断した回で封印すると、以後の再索引と LCIR 派生が永久に譲ってしまう"
        );
    }

    /// **失敗した回も封印しない**（理由は中断と同じ）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_run_does_not_seal_the_attachment(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _) = fake(Some(2));
        let hooks = OcrHooks::none();
        let out = transcribe_and_save(&pool, att, images(3), false, &t, &hooks)
            .await
            .unwrap();

        assert_eq!(out.saved, 1);
        assert!(out.failure.is_some());
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att).await.unwrap(),
            None,
            "失敗した回で封印してはいけない"
        );
    }

    /// **中断した再実行は、届かなかったページの既存本文を消さない。**
    /// これは `ocr_save_is_partial` の単体テストでは守れない ── 前回のレビューで
    /// 「`save_ocr_pages` の**呼び出し口**を `partial=false` に戻しても全 green」という
    /// 変異が生き残った（m1）。呼び出し側の証拠は DB の残存ページ数で取る。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_interrupted_rerun_keeps_the_pages_it_did_not_reach(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        // 前回のラン（または pdf_extract）が 5 ページぶんの本文を残している状態。
        index_attachment(
            &pool,
            att,
            &[
                (1, "old page 1".into()),
                (2, "old page 2".into()),
                (3, "old page 3".into()),
                (4, "old page 4".into()),
                (5, "old page 5".into()),
            ],
        )
        .await
        .unwrap();

        let (t, _) = fake(None);
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let d2 = done.clone();
        let stop = move || d2.load(std::sync::atomic::Ordering::SeqCst) >= 2;
        let d3 = done.clone();
        let prog = move |n: i64, _t: i64| d3.store(n as usize, std::sync::atomic::Ordering::SeqCst);
        let hooks = OcrHooks { should_stop: &stop, on_progress: &prog };

        let out = transcribe_and_save(&pool, att, images(5), false, &t, &hooks)
            .await
            .unwrap();

        assert!(out.stopped);
        assert_eq!(out.saved, 2);
        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att).await.unwrap(),
            5,
            "添付ごと置き換えると、届かなかった 3〜5 ページの既存本文が消える"
        );
    }

    /// 1 ページも処理する前に止めたら**索引を触らない**（空の置き換えで消さない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn stopping_before_the_first_page_changes_nothing(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        index_attachment(&pool, att, &[(1, "existing".into())]).await.unwrap();
        let (t, calls) = fake(None);
        let stop = || true;
        let hooks = OcrHooks { should_stop: &stop, on_progress: &|_, _| {} };

        let out = transcribe_and_save(&pool, att, images(9), false, &t, &hooks)
            .await
            .unwrap();

        assert!(out.stopped);
        assert_eq!(out.saved, 0);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "1 回も課金しない");
        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att).await.unwrap(),
            1,
            "既存の索引はそのまま"
        );
    }

    /// **進捗は `batch_status` に載る**（起動口がどちらでも ── ループ自身が書くので）。
    /// コマンド側のクロージャだけに置くと、チャット起動の OCR が進捗を 1 通も出さない。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_loop_reports_progress_to_batch_status(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _) = fake(None);
        let hooks = OcrHooks::none();
        transcribe_and_save(&pool, att, images(3), false, &t, &hooks)
            .await
            .unwrap();

        assert_eq!(
            crate::batch_status::snapshot().progress.get("ocr").copied(),
            Some(crate::batch_status::Progress { done: 3, total: 3 }),
            "設定 → データが読む進捗はループ自身が書く"
        );
    }

    /// **結果は `batch_status` の「直近の結果」に載る**（画面を離れていても後から読める）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_loop_records_the_outcome_for_the_settings_screen(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _) = fake(None);
        let hooks = OcrHooks::none();
        let out = transcribe_and_save(&pool, att, images(2), false, &t, &hooks)
            .await
            .unwrap();
        assert_eq!(out.saved, 2);

        let last = crate::batch_status::snapshot().last.expect("記録されている");
        assert_eq!(last.kind, crate::batch_status::BatchKind::Ocr);
        let result = last.result.expect("成功の戻り値がそのまま入る");
        assert_eq!(result["saved"], 2);
        assert_eq!(result["processed"], 2);
        assert_eq!(result["planned"], 2);
        assert_eq!(result["stopped"], false);
    }

    /// **ループ手前の失敗（添付なし・キー未設定・ラスタライズ失敗）も「直近の結果」に残る。**
    /// 残さないと、リーダーを離れた後に失敗したランがどの画面からも読めない
    /// （v2 レビュー confirmed[7] の解消）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_pre_loop_failure_is_recorded_for_the_settings_screen(pool: SqlitePool) {
        let _g = gate().await;
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "No attachment".to_string(),
                entry_type: "book".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = run_ocr(&pool, Path::new(""), entry.id, None, None, None, None).await;

        assert!(err.is_err(), "添付が無いので失敗する");
        let last = crate::batch_status::snapshot().last.expect("ループ手前の失敗も記録される");
        assert_eq!(last.kind, crate::batch_status::BatchKind::Ocr);
        assert!(
            last.error.as_deref().unwrap_or("").contains("no matching PDF attachment"),
            "エラー内容がそのまま残る: {:?}",
            last.error
        );
    }

    /// **白紙ページの転写（空白だけ）は既存の索引にも封印にも触らない。**
    /// 以前は部分差し替えが「そのページの既存行を削除して何も入れない」になり、
    /// 中断ラン + 全ページ空白で、既存本文の削除と封印剥がしまで連鎖した（レビューの medium）。
    #[sqlx::test(migrations = "./migrations")]
    async fn blank_transcriptions_never_touch_the_existing_index(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        // 以前の完走ランが 3 ページを転写して封印済み、という状態。
        index_attachment(
            &pool,
            att,
            &[(1, "old 1".into()), (2, "old 2".into()), (3, "old 3".into())],
        )
        .await
        .unwrap();
        crate::db::fulltext::set_fulltext_source(
            &pool,
            att,
            crate::db::fulltext::FulltextSource::Ocr,
        )
        .await
        .unwrap();

        // 再実行が全ページ空白を返し、2 ページで停止した。
        let t = fake_blank();
        let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let d2 = done.clone();
        let stop = move || d2.load(std::sync::atomic::Ordering::SeqCst) >= 2;
        let d3 = done.clone();
        let prog = move |n: i64, _t: i64| d3.store(n as usize, std::sync::atomic::Ordering::SeqCst);
        let hooks = OcrHooks { should_stop: &stop, on_progress: &prog };

        let out = transcribe_and_save(&pool, att, images(3), false, &t, &hooks)
            .await
            .unwrap();

        assert_eq!(out.processed, 2, "課金は 2 回発生している");
        assert_eq!(out.saved, 0, "本文が取れたページは 0");
        assert_eq!(
            crate::db::fulltext::indexed_page_count(&pool, att).await.unwrap(),
            3,
            "空白転写が既存行を消してはいけない"
        );
        assert_eq!(
            crate::db::fulltext::get_fulltext_source(&pool, att).await.unwrap(),
            Some(crate::db::fulltext::FulltextSource::Ocr),
            "以前の完走ランの封印が剥がれてはいけない"
        );
    }

    /// **失敗位置は実ページ番号で報告する。** `pages` 指定では通し番号と実ページ番号がずれ、
    /// 通し番号で案内すると取り直しが別のページを再課金する（v2 レビュー confirmed[3] の解消）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_failure_reports_the_real_page_number_not_the_ordinal(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        let (t, _) = fake(Some(3)); // 3 枚目（実ページ 11）で失敗
        let hooks = OcrHooks::none();
        let pages = vec![(3, "b".into()), (7, "b".into()), (11, "b".into())];

        let out = transcribe_and_save(&pool, att, pages, true, &t, &hooks)
            .await
            .unwrap();

        assert_eq!(out.failed_page, Some(11), "通し番号の 3 ではなく実ページ番号の 11");
        let msg = describe_outcome(1, &out);
        assert!(msg.contains("page 11"), "案内も実ページ番号で: {msg}");
    }

    /// 保存に失敗したら**失敗として記録される**（無言で消えない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_save_failure_is_recorded_for_the_settings_screen(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        // 保存先を壊して save_ocr_pages を確実に失敗させる。
        sqlx::query("DROP TABLE fulltext").execute(&pool).await.unwrap();
        let (t, _) = fake(None);
        let hooks = OcrHooks::none();

        let err = transcribe_and_save(&pool, att, images(1), false, &t, &hooks).await;

        assert!(err.is_err(), "保存失敗はエラーとして返る");
        let last = crate::batch_status::snapshot().last.expect("失敗も記録される");
        assert_eq!(last.kind, crate::batch_status::BatchKind::Ocr);
        assert!(last.error.is_some(), "エラー文字列が残る");
        assert!(last.result.is_none());
    }

    /// **`request_cancel`（プロセス内フラグ）でもループは止まる。** 呼び出し元固有の
    /// 述語を持たない経路（チャット以外）からでも、`cancel_ocr` で必ず降りられる。
    #[sqlx::test(migrations = "./migrations")]
    async fn request_cancel_stops_the_loop_before_the_next_page(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        request_cancel();
        let (t, calls) = fake(None);
        let hooks = OcrHooks::none();

        let out = transcribe_and_save(&pool, att, images(5), false, &t, &hooks)
            .await
            .unwrap();

        assert!(out.stopped, "プロセス内フラグでも止まる");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0, "1 回も課金しない");
    }

    /// **`cancel_ocr` コマンドの中身が実際に停止述語へ届く**（配線・変異 10 の防波堤）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_cancel_ocr_command_reaches_the_running_loop(pool: SqlitePool) {
        let _g = gate().await;
        let att = an_attachment(&pool).await;
        crate::cancel_ocr();
        let (t, calls) = fake(None);
        let hooks = OcrHooks::none();

        let out = transcribe_and_save(&pool, att, images(4), false, &t, &hooks)
            .await
            .unwrap();

        assert!(out.stopped, "コマンド経由の停止要求がループへ届く");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    // ── 排他と印の配線（begin_ocr / run_ocr）─────────────────────────────
    //
    // PR の看板「起動口 2 つを 1 本に絞る」「画面をまたいで止められる」の配線そのもの。
    // 前回はここに 1 本も無く、変異 24 個中 12 個が survivor だった。

    /// **2 本目の `run_ocr` は入り口で弾かれ、1 本目の印を消さない。**
    /// `run_ocr` 本体を呼ぶので、「排他を取る行を `run_ocr` から消す」変異はここで落ちる
    /// （消すと添付探索まで進み、エラーが `already_running` ではなくなる）。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_second_ocr_is_rejected_and_leaves_the_first_running(pool: SqlitePool) {
        let _g = gate().await;
        let first = begin_ocr().expect("1 本目は取れる");

        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "No attachment".to_string(),
                entry_type: "book".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let second =
            run_ocr(&pool, Path::new(""), entry.id, None, None, None, None).await;

        let msg = second.expect_err("2 本目は弾かれる").to_string();
        assert!(
            msg.contains(OCR_ALREADY_RUNNING),
            "弾いた理由は already_running（他のエラーで弾けたのでは排他の証拠にならない）: {msg}"
        );
        assert!(
            crate::batch_status::snapshot()
                .running
                .contains(&crate::batch_status::BatchKind::Ocr),
            "弾かれた 2 本目が 1 本目の印を壊さない"
        );
        drop(first);
    }

    /// **実行権を落とすと次が取れるようになり、印も消える。** `OcrRunGuard` の Drop が
    /// 空になると、以後すべての OCR が `already_running` で永久に弾かれる。
    #[test]
    fn dropping_the_run_lowers_the_flag_and_clears_the_mark() {
        let _g = gate_blocking();
        let run = begin_ocr().expect("1 本目は取れる");
        assert!(
            crate::batch_status::snapshot()
                .running
                .contains(&crate::batch_status::BatchKind::Ocr),
            "実行中は設定 → データに「OCR 実行中」が出る（＝停止ボタンの表示条件）"
        );
        drop(run);
        assert!(
            !crate::batch_status::snapshot()
                .running
                .contains(&crate::batch_status::BatchKind::Ocr),
            "終わったら印は消える"
        );
        let again = begin_ocr();
        assert!(again.is_ok(), "実行権を落としたら次の OCR が始められる");
    }

    /// **開始時に前回の中断要求を必ず倒す。** 倒し忘れると、一度停止した後の OCR が
    /// 永久に 0 ページで終わる（フラグはプロセス内 static なので自然には消えない）。
    #[test]
    fn beginning_a_run_resets_a_stale_cancel_request() {
        let _g = gate_blocking();
        request_cancel();
        let _run = begin_ocr().expect("取れる");
        assert!(
            !OCR_CANCEL.load(Ordering::SeqCst),
            "前回の押し忘れを引き継ぐと、次の OCR が 1 ページも処理せず終わる"
        );
    }

    // ── チャット側のガード（guard_llm_ocr）───────────────────────────────
    //
    // ガードを `try_execute` から出しているのは、通す側（`None` を返す枝）を
    // assert するとその先の `run_ocr` が keychain と HTTP に触ってしまうため。
    // ここは pool だけで完結するので、拒否と通過の**両方**を書ける。

    /// 索引済みの添付は OCR し直させない（issue #42）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_guard_refuses_an_already_indexed_attachment(pool: SqlitePool) {
        let entry = new_entry(&pool, "Indexed paper").await;
        let att = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, att.id, &[(1, "existing text layer".to_string())])
            .await
            .unwrap();

        let msg = guard_llm_ocr(&pool, entry.id, Some(att.id))
            .await
            .expect("索引済みなら断る");
        assert!(msg.contains("get_fulltext"), "安い経路を指さないと LLM は OCR に戻る: {msg}");
    }

    /// **判定は「実際に OCR される添付」で取る。** `attachment_id` を省いた呼び出しは
    /// `prepare_ocr` と同じ規則（先頭の PDF）で解決されるので、その添付の索引を見る。
    /// ここがずれると、ガードは別の添付を見て通し、OCR は索引済みの添付を焼く。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_guard_reads_the_attachment_that_would_actually_run(pool: SqlitePool) {
        let entry = new_entry(&pool, "Paper with two PDFs").await;
        let first = add_attachment(&pool, entry.id, "a/main.pdf", "main.pdf", "application/pdf")
            .await
            .unwrap();
        let _second = add_attachment(&pool, entry.id, "a/si.pdf", "si.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, first.id, &[(1, "main text layer".to_string())])
            .await
            .unwrap();

        let msg = guard_llm_ocr(&pool, entry.id, None)
            .await
            .expect("省略時は先頭の PDF に解決される ── その添付が索引済みなら断る");
        // **どの添付で判定したか**まで見る。`is_some()` だけだと entry 合算（旧実装）へ
        // 戻す変異が緑のまま通る（entry 配下には索引が 1 ページあるので）。
        assert!(
            msg.contains(&format!("attachment {}", first.id)),
            "先頭の PDF の件数で判定していない: {msg}"
        );
    }

    /// **過剰拒否をしない。** 本体 PDF が索引済みでも、未索引のスキャン補遺は OCR してよい。
    /// entry 単位で数えていた頃は、名指ししても「entry は既に N ページ索引済み」で断っていた。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_guard_allows_an_unindexed_attachment_of_a_partly_indexed_entry(pool: SqlitePool) {
        let entry = new_entry(&pool, "Paper with a scanned supplement").await;
        let main = add_attachment(&pool, entry.id, "a/main.pdf", "main.pdf", "application/pdf")
            .await
            .unwrap();
        let si = add_attachment(&pool, entry.id, "a/si.pdf", "si.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, main.id, &[(1, "main text layer".to_string())])
            .await
            .unwrap();

        // ⚠ **「通した」が受け皿になっていないことを先に確かめる。** 対象が引けなくても
        // ガードは `None` を返すので、解決できたことを別 assert で固定しないと
        // このテストは本体（件数の判定）に到達しなくても緑になる。
        assert_eq!(
            resolve_target_pdf(&pool, entry.id, Some(si.id)).await.unwrap().map(|(id, _)| id),
            Some(si.id),
            "名指しした添付に解決できていない ── 以下の None は判定の結果ではない"
        );
        assert_eq!(
            guard_llm_ocr(&pool, entry.id, Some(si.id)).await,
            None,
            "テキスト層の無い添付を名指ししたのに断ると、OCR が本来の対象に届かない"
        );
        // 逆向き ── 索引済みの側を名指ししたら、その添付の名前で断る。
        assert!(
            guard_llm_ocr(&pool, entry.id, Some(main.id))
                .await
                .expect("索引済みを名指ししたら断る")
                .contains(&format!("attachment {}", main.id)),
            "名指しした添付ではなく別の添付で判定している"
        );
    }

    /// PDF が 1 つも無い entry はここでは断らない ── 「添付が無い」は `run_ocr` の
    /// エラーで返す（同じ文言を 2 か所に持たない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_guard_defers_when_the_entry_has_no_pdf(pool: SqlitePool) {
        let entry = new_entry(&pool, "No attachments").await;
        assert_eq!(guard_llm_ocr(&pool, entry.id, None).await, None);
    }

    // ── チャット側のガード（try_execute）─────────────────────────────────

    /// issue #42: 索引済みの PDF を LLM に丸ごと OCR し直させない。
    /// 全ページ OCR は添付の索引を置き換えるので、テキスト層が Vision 出力で消える。
    #[sqlx::test(migrations = "./migrations")]
    async fn refuses_full_ocr_of_an_already_indexed_entry(pool: SqlitePool) {
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Indexed paper".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, att.id, &[(1, "existing text layer".to_string())])
            .await
            .unwrap();

        let ctx = ToolContext { should_stop: None,
            pool: &pool,
            session_id: 1,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        };
        let call = ToolCallSpec {
            call_id: "c1".to_string(),
            tool_name: "ocr_pdf".to_string(),
            arguments: json!({ "entry_id": entry.id }),
        };
        let out = try_execute(&ctx, &call).await.unwrap().unwrap();
        assert!(out.contains("already has"), "{out}");
        assert!(out.contains("get_fulltext"), "must point at the cheap path: {out}");

        // 索引は無傷。
        let pages = crate::db::fulltext::get_entry_fulltext(&pool, entry.id).await.unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].1, "existing text layer");
    }

    /// **`pages` はガードの迂回路にならない。** 以前は部分 OCR を通していたが、完走した
    /// 部分 OCR も添付ごと `fulltext.source = Ocr` を立てる（§2.6-1）ので、1 ページ指定でも
    /// 添付全体の索引が恒久的に再索引を拒むようになる。§2.6-1 がその倒し込みの根拠に置いた
    /// 「ユーザーの明示宣言」は UI 経路にしかないので、LLM 経路は入口で断る。
    ///
    /// **`run_ocr` に入らずに返るので keychain に触らない**（旧テストは触っていた）。
    #[sqlx::test(migrations = "./migrations")]
    async fn naming_pages_does_not_bypass_the_indexed_guard(pool: SqlitePool) {
        let entry = new_entry(&pool, "Indexed paper").await;
        let att = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, att.id, &[(1, "existing".to_string())])
            .await
            .unwrap();

        let ctx = ToolContext { should_stop: None,
            pool: &pool,
            session_id: 1,
            scope_mode: "all",
            scope_entry_ids: &[],
            mcp: None,
            app_data_dir: std::path::Path::new(""),
        };
        let call = ToolCallSpec {
            call_id: "c1".to_string(),
            tool_name: "ocr_pdf".to_string(),
            arguments: json!({ "entry_id": entry.id, "pages": [2] }),
        };
        let out = try_execute(&ctx, &call).await.unwrap().unwrap();
        assert!(out.contains("already has"), "ページ指定でも断る: {out}");
    }

    /// **拒否文が迂回路を教えない。** 旧文言は「To re-transcribe specific pages anyway,
    /// pass `pages`」と、ガードを抜ける引数そのものを LLM に案内していた。
    ///
    /// 数の検査は**位置ごと**に取る ── 素の `contains("42")` だと `{attachment_id}` と
    /// `{indexed_pages}` を入れ替える変異が生き残り、LLM が「添付 23 が 42 ページ索引済み」と
    /// 誤って伝える（この文が LLM に渡る唯一の説明なので、取り違えは全部ユーザーに出る）。
    #[test]
    fn the_refusal_does_not_advertise_a_bypass() {
        let msg = refuse_ocr_of_indexed_attachment(7, 42, 23).expect("索引済みなら断る");
        assert!(
            !msg.contains("pass `pages`"),
            "ガードを抜ける引数を案内すると、ガードは助言に格下げされる: {msg}"
        );
        assert!(msg.contains("attachment 42"), "どの添付か: {msg}");
        assert!(msg.contains("23 indexed"), "何ページ索引済みか: {msg}");
        assert!(msg.contains("entry 7"), "どの entry か: {msg}");
        // 人間の操作へ渡す先は、UI に実在するラベルで書く（「リーダー」という画面名は無い）。
        assert!(msg.contains("OCR button"), "人間の操作へ渡す先を書く: {msg}");
        assert!(
            msg.contains("nothing was billed"),
            "空振りだと分かる 1 文が要る（承認した直後に返るので）: {msg}"
        );
    }

    /// 断るときは、同じ entry の**未索引の PDF 添付**を id 付きで添える。
    /// これが無いと「本体は索引済み・補遺はスキャン」の entry で補遺に到達できない ──
    /// チャットの `get_entry` は entry 単位の `has_fulltext` しか返さない。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_refusal_names_the_unindexed_sibling_attachment(pool: SqlitePool) {
        let entry = new_entry(&pool, "Paper with a scanned supplement").await;
        let main = add_attachment(&pool, entry.id, "a/main.pdf", "main.pdf", "application/pdf")
            .await
            .unwrap();
        let si = add_attachment(&pool, entry.id, "a/si.pdf", "si.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, main.id, &[(1, "main text layer".to_string())])
            .await
            .unwrap();

        let msg = guard_llm_ocr(&pool, entry.id, None).await.expect("先頭は索引済みなので断る");
        assert!(
            msg.contains(&format!("{}", si.id)) && msg.contains("no indexed text"),
            "未索引の補遺の id を添えないと、そこへ到達する手段が無い: {msg}"
        );
    }

    /// 未索引の添付が無ければ余計な案内を付けない（毎回付くと LLM が総当たりを始める）。
    ///
    /// ⚠ **添付は 2 つ要る。** 1 つだけだと `a.id != target` の除外で結果が空になり、
    /// 「未索引だけを挙げる」述語が無検証のまま緑になる（変異 N5 が生き残った）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_refusal_stays_short_when_every_attachment_is_indexed(pool: SqlitePool) {
        let entry = new_entry(&pool, "Fully indexed paper").await;
        let main = add_attachment(&pool, entry.id, "a/main.pdf", "main.pdf", "application/pdf")
            .await
            .unwrap();
        let other = add_attachment(&pool, entry.id, "a/si.pdf", "si.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, main.id, &[(1, "text".to_string())]).await.unwrap();
        index_attachment(&pool, other.id, &[(1, "si text".to_string())]).await.unwrap();

        let msg = guard_llm_ocr(&pool, entry.id, None).await.expect("断る");
        assert!(!msg.contains("no indexed text at all"), "他に候補が無いのに案内しない: {msg}");
        assert!(
            !msg.contains(&format!("{}", other.id)),
            "索引済みの添付を「OCR できる候補」として挙げてはいけない: {msg}"
        );
    }

    /// ⚠ **対象添付の引き当てが失敗したときも断る。** `Ok(None)`（行が無い）と
    /// `Err`（クエリが失敗）を同じ「通す」枝に潰すと、一過性の DB エラーのときだけ
    /// ガードが素通りする ── `prepare_ocr` は同じクエリを投げ直すので、そこで成功すれば
    /// 索引状態を一度も見ないまま索引済み添付を OCR して課金する。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_guard_refuses_when_the_target_cannot_be_looked_up(pool: SqlitePool) {
        let entry = new_entry(&pool, "Indexed paper").await;
        let att = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, att.id, &[(1, "existing".to_string())]).await.unwrap();
        sqlx::query("DROP TABLE attachments").execute(&pool).await.unwrap();

        let msg = guard_llm_ocr(&pool, entry.id, Some(att.id))
            .await
            .expect("引き当てられないなら断る");
        assert!(msg.contains("could not look up the target attachment"), "{msg}");
    }

    /// **`pages` 引数が実際にパースされる。** 旧ガードは `pages.is_none()` を見ていたので
    /// 入口テストが間接的にここを守っていたが、ガードが `pages` を見なくなったので
    /// 直接固定する ── `None` に潰れると「5 ページだけ」が 527 ページ課金になる。
    #[test]
    fn the_requested_pages_and_attachment_are_read_from_the_arguments() {
        assert_eq!(requested_pages(&json!({ "pages": [2, 5] })), Some(vec![2, 5]));
        assert_eq!(requested_pages(&json!({ "entry_id": 1 })), None, "省略は全ページ");
        assert_eq!(requested_pages(&json!({ "pages": [] })), Some(vec![]), "空配列は全ページではない");
        assert_eq!(
            requested_pages(&json!({ "pages": [1, "x", 3] })),
            Some(vec![1, 3]),
            "数値でない要素は落とす"
        );
        assert_eq!(requested_attachment_id(&json!({ "attachment_id": 9 })), Some(9));
        assert_eq!(requested_attachment_id(&json!({ "entry_id": 1 })), None, "省略は先頭の PDF");
    }

    /// **ページを名指しした OCR は封印しない。** 封印は添付単位なので、1 ページの転写で
    /// 立てると残りのページが二度と索引されなくなる（500 頁の PDF に `pages:[3]` で
    /// 499 頁が恒久ロック）。§2.6-1 の根拠は添付全体を焼く操作にしか成立しない。
    #[test]
    fn a_page_range_never_seals_the_whole_attachment() {
        assert!(!ocr_save_should_seal(true, false, false), "完走してもページ指定なら封印しない");
        assert!(ocr_save_should_seal(false, false, false), "全ページを完走したら封印する");
        assert!(!ocr_save_should_seal(false, true, false), "中断では封印しない");
        assert!(!ocr_save_should_seal(false, false, true), "失敗では封印しない");
    }

    /// **呼び出し元の停止述語が生きたままループへ渡る。** `hooks_for` を
    /// `OcrHooks::none()` に潰す変異はここだけが落とす（`try_execute` 経由では
    /// 添付探索で先に落ちてフックまで届かないので、配線は素通りになる）。
    #[test]
    fn the_callers_stop_predicate_reaches_the_loop_hooks() {
        let pressed = AtomicBool::new(false);
        let pred = || pressed.load(Ordering::SeqCst);
        let hooks = hooks_for(Some(&pred), None);

        assert!(!(hooks.should_stop)(), "まだ押していない");
        pressed.store(true, Ordering::SeqCst);
        assert!(
            (hooks.should_stop)(),
            "実行中に押した停止が届かない ── bool で渡すと開始時のスナップショットで凍る"
        );
    }

    /// 索引が 0 ページなら通す（スキャン PDF ＝ OCR の本来の対象）。
    #[test]
    fn an_unindexed_attachment_is_not_refused() {
        assert_eq!(refuse_ocr_of_indexed_attachment(7, 42, 0), None);
    }

    /// ⚠ **索引件数が読めなかったら断る。** ここを `unwrap_or(0)` にすると、DB が読めない
    /// ときだけ「未索引」に見えて課金と封印が通る ── fail-safe の向きが反転する。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_guard_refuses_when_the_index_cannot_be_read(pool: SqlitePool) {
        let entry = new_entry(&pool, "Indexed paper").await;
        let att = add_attachment(&pool, entry.id, "a/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap();
        index_attachment(&pool, att.id, &[(1, "existing".to_string())])
            .await
            .unwrap();
        // 添付は引けるが `fulltext` だけ読めない状態を作る。
        sqlx::query("DROP TABLE fulltext").execute(&pool).await.unwrap();

        let msg = guard_llm_ocr(&pool, entry.id, Some(att.id))
            .await
            .expect("読めないなら断る");
        assert!(msg.contains("could not read the index state"), "{msg}");
    }
}
