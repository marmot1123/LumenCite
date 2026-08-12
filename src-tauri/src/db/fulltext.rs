use crate::models::{Author, EntrySummary, FulltextHit, Tag};
use sqlx::{Row, SqlitePool};

/// 抽出済みの本文を attachment_id に紐付けて fulltext テーブルへ書き込む。
/// `pages` は (page_number, text) のリスト。空文字列のページはスキップする。
pub async fn index_attachment(
    pool: &SqlitePool,
    attachment_id: i64,
    pages: &[(i64, String)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    replace_pages(&mut tx, attachment_id, pages).await?;
    tx.commit().await?;
    Ok(())
}

/// 添付の全ページを与えられた内容で置き換える（tx 内の共通部）。
/// 空文字列のページはスキップする。
async fn replace_pages(
    conn: &mut sqlx::SqliteConnection,
    attachment_id: i64,
    pages: &[(i64, String)],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM fulltext WHERE attachment_id = ?")
        .bind(attachment_id)
        .execute(&mut *conn)
        .await?;

    for (page, content) in pages {
        if content.trim().is_empty() {
            continue;
        }
        sqlx::query("INSERT INTO fulltext (content, attachment_id, page) VALUES (?, ?, ?)")
            .bind(content)
            .bind(attachment_id)
            .bind(page)
            .execute(&mut *conn)
            .await?;
    }

    Ok(())
}

/// 指定ページのみを差し替える（他ページの行は保持）。部分 OCR の保存用。
/// 空文字列のページは削除のみ行う（再処理の結果テキストが無かった場合）。
pub async fn update_attachment_pages(
    pool: &SqlitePool,
    attachment_id: i64,
    pages: &[(i64, String)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    for (page, content) in pages {
        sqlx::query("DELETE FROM fulltext WHERE attachment_id = ? AND page = ?")
            .bind(attachment_id)
            .bind(page)
            .execute(&mut *tx)
            .await?;
        if content.trim().is_empty() {
            continue;
        }
        sqlx::query("INSERT INTO fulltext (content, attachment_id, page) VALUES (?, ?, ?)")
            .bind(content)
            .bind(attachment_id)
            .bind(page)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 添付ごとの「全文索引の出どころ」（p1）。
///
/// `fulltext` は FTS5 仮想表で provenance 列を持てない（列の追加は
/// `virtual tables may not be altered` で拒否される）ため、添付単位の記録を settings KV
/// （`fulltext.source.<attachment_id>`）に置く ＝ **migration 0 件**（§2.6-1 の決定）。
///
/// 記録するのは「上書きされたら困る」2 つだけ。pdf_extract 由来は既定なので記録しない
/// （キー無し = pdf_extract 由来、または未索引）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FulltextSource {
    /// LCIR の page ノードから派生（p1 の既定ソース）。
    Lcir,
    /// ユーザーが明示的に回した OCR の出力。**LCIR 派生で上書きしない**
    /// （OCR を回した添付 = 「この PDF のテキスト層は信用できない」という宣言）。
    Ocr,
}

impl FulltextSource {
    pub fn as_str(self) -> &'static str {
        match self {
            FulltextSource::Lcir => "lcir",
            FulltextSource::Ocr => "ocr",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "lcir" => Some(FulltextSource::Lcir),
            "ocr" => Some(FulltextSource::Ocr),
            _ => None,
        }
    }
}

fn fulltext_source_key(attachment_id: i64) -> String {
    format!(
        "{}{attachment_id}",
        crate::db::settings::FULLTEXT_SOURCE_KEY_PREFIX
    )
}

/// この添付の索引が何由来かを返す（記録が無ければ `None` = pdf_extract 由来 or 未索引）。
pub async fn get_fulltext_source(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<Option<FulltextSource>, sqlx::Error> {
    let raw = crate::db::settings::get_setting(pool, &fulltext_source_key(attachment_id)).await?;
    Ok(raw.as_deref().and_then(FulltextSource::parse))
}

/// 索引の出どころを記録する。
pub async fn set_fulltext_source(
    pool: &SqlitePool,
    attachment_id: i64,
    source: FulltextSource,
) -> Result<(), sqlx::Error> {
    crate::db::settings::set_setting(pool, &fulltext_source_key(attachment_id), source.as_str())
        .await
}

/// 索引の出どころの記録を消す（守る中身が無くなったときの後始末）。
pub async fn clear_fulltext_source(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<(), sqlx::Error> {
    crate::db::settings::delete_setting(pool, &fulltext_source_key(attachment_id)).await
}

/// 索引の出どころの記録を tx 内で消す（添付削除・索引削除の後始末）。
///
/// 記録を残したまま索引だけ消すと、`index_attachment_from_pdf_extract` が
/// **中身がもう無い索引を守り続けて**その添付を永久に未索引にする。
pub(crate) async fn clear_fulltext_source_tx(
    conn: &mut sqlx::SqliteConnection,
    attachment_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(fulltext_source_key(attachment_id))
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// エントリ配下の全添付ぶんの記録を tx 内で消す（エントリの hard delete の後始末・
/// `fulltext` / `document_nodes_fts` と同じ扱い）。
pub(crate) async fn clear_fulltext_sources_for_entry_tx(
    conn: &mut sqlx::SqliteConnection,
    entry_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM settings WHERE key IN (
            SELECT ? || a.id FROM attachments a WHERE a.entry_id = ?
        )",
    )
    .bind(crate::db::settings::FULLTEXT_SOURCE_KEY_PREFIX)
    .bind(entry_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// [`index_attachment_from_pdf_extract`] が実際に何をしたか。
///
/// **bool で返さない。** 「譲った」には理由が 2 つあり（出どころ記録に守られた / 抽出が空だった）、
/// 呼び出し側の UI 文言が別物になる ── 1 つに潰すと「テキストが見つかりません」と
/// 「既存の索引を残しました」を区別できない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PdfExtractWrite {
    /// 索引を置き換えた（出どころの記録は既定 = pdf_extract 由来に戻した）。
    Replaced,
    /// 出どころの記録（OCR / LCIR）に守られたので触らなかった。
    SkippedProtected,
    /// **抽出が 1 ページも本文を返さなかったので、既存の索引を残した**（v1.0.0・②b の W1-1）。
    SkippedEmptyExtract,
}

/// pdf_extract 由来のページで索引を置き換える。ただし **LCIR / OCR 由来の索引は上書きしない**
/// （debt-17 の last-writer-wins 回避）。
///
/// 判定と書き込みは同一トランザクションで行う
/// （spawn した pdf_extract は数十秒かかるので、抽出前に 1 度読むだけでは競合を塞げない）。
///
/// `replace_existing` は「ユーザーがこの添付を名指しで再索引した」経路だけ `true`。
/// **OCR 由来は名指しでも守る**（守る対象はユーザーが課金して起こした転写）。
///
/// ## 空の抽出結果は既存の索引を消さない（v1.0.0・②b の W1-1）
///
/// [`replace_pages`] は先頭で無条件に `DELETE` し、空ページを `INSERT` しない ＝
/// **全ページ空の入力は「削除だけ」になる**。テキスト層の無いスキャン PDF で
/// `extract_text_by_pages` が `Ok(全ページ空)` を返すのは正常系（`lib.rs` が
/// `PdfExtract(0)` を「OCR で拾う候補」として数えている）なので、確認ダイアログの無い
/// 再索引ボタン 1 回で、課金して起こした OCR 転写が消えていた。
///
/// **守るのは「非空 0 件」のときだけで、縮小（既存 500 行 → 新規 1 行）は守らない。**
/// 縮小まで守ると、本当に内容が減った PDF を張り直せなくなる。
pub async fn index_attachment_from_pdf_extract(
    pool: &SqlitePool,
    attachment_id: i64,
    pages: &[(i64, String)],
    replace_existing: bool,
) -> Result<PdfExtractWrite, sqlx::Error> {
    let key = fulltext_source_key(attachment_id);
    // **`BEGIN IMMEDIATE`**（既定の DEFERRED ではない）。読んでから書く tx を deferred で始めると、
    // 読みスナップショットを取った後に他の接続が commit した場合、昇格時に `SQLITE_BUSY_SNAPSHOT`
    // が返る ── これは busy handler の対象外なので `busy_timeout` では待てず即失敗する。
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    let current: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await?;
    let blocked = match current.as_deref().and_then(FulltextSource::parse) {
        Some(FulltextSource::Ocr) => true,
        Some(FulltextSource::Lcir) => !replace_existing,
        None => false,
    };
    if blocked {
        // 譲る（tx は drop でロールバック）。
        return Ok(PdfExtractWrite::SkippedProtected);
    }

    // **空抽出のガードは `blocked` 判定の後**。前に置くと、OCR 由来の添付に空抽出が来たとき
    // `SkippedProtected` ではなくこちらが返り、詳細パネルの「OCR で取り込んだ本文を残しました」
    // （取り直す手順を書いた文言）が、まさにスキャン本で出なくなる。
    //
    // 数えるのは非空 0 件のときだけ ── `fulltext` は FTS5 で `attachment_id` が UNINDEXED
    // なので `COUNT(*)` は全行スキャンになり、しかもこの tx は `BEGIN IMMEDIATE` で
    // writer ロックを握っている。正常に張り直す大多数の呼び出しでロックを延ばさない。
    if pages.iter().all(|(_, t)| t.trim().is_empty()) {
        let existing: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
                .bind(attachment_id)
                .fetch_one(&mut *tx)
                .await?;
        if existing > 0 {
            return Ok(PdfExtractWrite::SkippedEmptyExtract);
        }
        // 既存 0 行なら通す。ここで譲ると下の `DELETE FROM settings` が飛び、
        // 「行は 0 なのに `source = lcir` の記録だけ残る」添付ができて、以後の自動経路が
        // 永久に譲り続ける（`clear_fulltext_source_tx` の doc が名指しで警告している状態）。
    }

    replace_pages(&mut tx, attachment_id, pages).await?;
    // 置き換えたので出どころは既定（pdf_extract 由来）に戻る。
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(&key)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(PdfExtractWrite::Replaced)
}

/// [`index_attachment_from_lcir`] が実際に何をしたか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LcirWrite {
    /// LCIR のページで差し替えて `source = lcir` を立てた。
    Replaced,
    /// OCR 由来として記録済みなので触らなかった。
    SkippedOcr,
    /// **出どころの記録が無い既存索引を守った**（`protect_unrecorded` が真のときだけ）。
    SkippedUnrecorded,
}

/// LCIR の page ノード由来のページで索引を差し替え、**出どころの記録を同一 tx で**立てる。
///
/// 2 文に分けると「索引は LCIR 由来なのに記録がまだ無い」窓ができ、その隙に走った
/// pdf_extract が上書きしてしまう（debt-17 と同じ形の競合）。
/// OCR 由来として記録済みの添付には書かない（[`LcirWrite::SkippedOcr`]）。
///
/// ## `protect_unrecorded`（v1.0.0・②b の W1-2）
///
/// 真なら「**出どころの記録が無く、かつ既存の索引行がある**」添付も守る（自動の再導出だけ）。
/// 呼び出し側にも同じ判定があるが、あちらは tx の外なので check-then-act の窓が開く ──
/// 評価した直後に部分 OCR（`seal = false` なので記録を立てない）が着地すると、
/// 記録の無い課金済み転写を LCIR で置き換える。**判定を tx の中にも置いて塞ぐ。**
///
/// 記録が無い既存索引には 3 つの母集団がある ── ①この版より前に入った索引
/// ②OCR が行を書いてから封印するまでの窓 ③中断・部分 OCR（封印しない・debt-43）。
/// 「この版より前」だけではない（実 DB に 0 件でも安全にならない）。
///
/// **明示操作（設定→データのボタン）と build 経路では偽**を渡すこと。あちらは
/// 「記録が無い既存索引を LCIR へ移行する」ことそのものが目的（p1 の唯一の移行経路）。
///
/// **LCIR が本文を持たないページの既存行は残す**（debt-34）。pdfium がそのページだけ空を返す
/// ことがあり、添付ごと置き換えると pdf_extract / OCR で入っていた本文が消える（実 DB で 2 ページ）。
/// 残す行にも `clean` を通す ── 残すのは旧抽出器由来の行なので、通さないと
/// 「派生後の索引に C0 制御文字を含む行が 0 件」という受け入れ条件が崩れる。
pub async fn index_attachment_from_lcir(
    pool: &SqlitePool,
    attachment_id: i64,
    pages: &[(i64, String)],
    clean: fn(&str) -> String,
    protect_unrecorded: bool,
) -> Result<LcirWrite, sqlx::Error> {
    let key = fulltext_source_key(attachment_id);
    // `BEGIN IMMEDIATE`（理由は `index_attachment_from_pdf_extract` と同じ）。
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;

    let current: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await?;
    let recorded = current.as_deref().and_then(FulltextSource::parse);
    if recorded == Some(FulltextSource::Ocr) {
        return Ok(LcirWrite::SkippedOcr);
    }

    let covered: std::collections::HashSet<i64> = pages.iter().map(|(p, _)| *p).collect();
    let existing: Vec<(i64, String)> =
        sqlx::query_as("SELECT page, content FROM fulltext WHERE attachment_id = ?")
            .bind(attachment_id)
            .fetch_all(&mut *tx)
            .await?;
    // 既存行はこの下の merge でどのみち読むので、守る判定の追加コストは 0。
    if protect_unrecorded && recorded.is_none() && !existing.is_empty() {
        return Ok(LcirWrite::SkippedUnrecorded);
    }
    let mut merged: Vec<(i64, String)> = pages.to_vec();
    merged.extend(
        existing
            .into_iter()
            .filter(|(p, _)| !covered.contains(p))
            .map(|(p, c)| (p, clean(&c))),
    );

    replace_pages(&mut tx, attachment_id, &merged).await?;
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&key)
    .bind(FulltextSource::Lcir.as_str())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(LcirWrite::Replaced)
}

pub async fn unindex_attachment(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM fulltext WHERE attachment_id = ?")
        .bind(attachment_id)
        .execute(&mut *tx)
        .await?;
    // 出どころの記録も同時に落とす（守る中身がもう無いため）。
    clear_fulltext_source_tx(&mut tx, attachment_id).await?;
    tx.commit().await?;
    Ok(())
}

/// `fulltext` FTS5（trigram）の逆索引を起動時に 1 回だけ再構築する自己修復。
///
/// 一部の既存ライブラリでは `fulltext` の逆索引が malformed になっており、新しい
/// SQLite では `PRAGMA integrity_check` が "malformed inverted index for FTS5 table
/// main.fulltext" を返す（アプリ内蔵の古い SQLite では検出できないため素通りしていた）。
/// これを放置すると全文検索が誤動作し得るので、`settings.fts.fulltext_rebuilt` が未セット
/// なら FTS5 の `'rebuild'` コマンドで %_content から索引を作り直し、完了後にフラグを立てる。
/// 2 回目以降は no-op。malformed でない健全な索引でも rebuild は安全（同じ索引を作り直すだけ）。
///
/// 戻り値: 実際に再構築が走ったら `true`、フラグ既設で skip したら `false`。
/// `rebuild_authors_fts_once` と同じく起動時に background で呼ぶ。失敗時はフラグを立てず
/// Err を返すので次回起動でリトライされる。
pub async fn rebuild_fulltext_fts_once(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    use crate::db::settings;

    if settings::get_setting(pool, settings::FTS_FULLTEXT_REBUILT_KEY)
        .await?
        .is_some()
    {
        return Ok(false);
    }

    sqlx::query("INSERT INTO fulltext(fulltext) VALUES('rebuild')")
        .execute(pool)
        .await?;

    settings::set_setting(pool, settings::FTS_FULLTEXT_REBUILT_KEY, "1").await?;
    Ok(true)
}

/// 今この添付に入っている索引済みページ数。
pub async fn indexed_page_count(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
        .bind(attachment_id)
        .fetch_one(pool)
        .await
}

pub async fn is_indexed(pool: &SqlitePool, attachment_id: i64) -> Result<bool, sqlx::Error> {
    let row =
        sqlx::query("SELECT COUNT(*) AS cnt FROM fulltext WHERE attachment_id = ?")
            .bind(attachment_id)
            .fetch_one(pool)
            .await?;
    Ok(row.get::<i64, _>("cnt") > 0)
}

/// まだ全文索引が無い PDF 添付を `(attachment_id, file_path)` で返す（ゴミ箱のエントリは除外）。
/// 「未索引の添付を一括索引」バッチが処理対象を集めるのに使う。順序は id 昇順で安定。
pub async fn attachments_without_fulltext(
    pool: &SqlitePool,
) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT a.id AS id, a.file_path AS file_path
         FROM attachments a
         JOIN entries e ON e.id = a.entry_id
         WHERE e.deleted_at IS NULL
           AND a.mime_type LIKE '%pdf%'
           AND NOT EXISTS (SELECT 1 FROM fulltext f WHERE f.attachment_id = a.id)
         ORDER BY a.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<i64, _>("id"), r.get::<String, _>("file_path")))
        .collect())
}

/// エントリに紐づく（索引済み PDF の）全文を `(page, content)` のリストで返す。
/// 添付ごとの `attachment_id, page` 順で並べる。索引が無ければ空を返す。
/// `generate_summary`（fulltext ソース）と MCP の `get_fulltext` が共有する。
pub async fn get_entry_fulltext(
    pool: &SqlitePool,
    entry_id: i64,
) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT f.page AS page, f.content AS content
         FROM fulltext f
         JOIN attachments a ON a.id = f.attachment_id
         WHERE a.entry_id = ?
         ORDER BY f.attachment_id, f.page",
    )
    .bind(entry_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<i64, _>("page"), r.get::<String, _>("content")))
        .collect())
}

/// エントリの索引済み全文ページ数（0 なら全文なし）。`get_entry` の `has_fulltext`
/// フラグや `get_fulltext` の総ページ数表示に使う軽量カウント。
pub async fn entry_fulltext_page_count(
    pool: &SqlitePool,
    entry_id: i64,
) -> Result<i64, sqlx::Error> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS cnt
         FROM fulltext f
         JOIN attachments a ON a.id = f.attachment_id
         WHERE a.entry_id = ?",
    )
    .bind(entry_id)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("cnt"))
}

pub(crate) fn build_match_expr(tokens: &[&str]) -> String {
    tokens
        .iter()
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub async fn search_fulltext(
    pool: &SqlitePool,
    query: &str,
    collection_id: Option<i64>,
    tag_id: Option<i64>,
    view: Option<&str>,
) -> Result<Vec<FulltextHit>, sqlx::Error> {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // trigram は 3 文字未満を処理できない。短いトークンが含まれていれば LIKE フォールバック。
    let use_like = tokens.iter().any(|t| t.chars().count() < 3);

    let mut sql = String::new();
    sql.push_str(
        "SELECT f.attachment_id AS attachment_id, f.page AS page, ",
    );
    if use_like {
        sql.push_str("substr(f.content, 1, 200) AS snippet, ");
    } else {
        sql.push_str(
            "snippet(fulltext, 0, '⟨', '⟩', '…', 12) AS snippet, ",
        );
    }
    sql.push_str(
        "a.entry_id AS entry_id
         FROM fulltext f
         JOIN attachments a ON a.id = f.attachment_id
         WHERE ",
    );

    if use_like {
        // 各トークンが content に含まれること（AND）
        let likes: Vec<&str> = tokens
            .iter()
            .map(|_| "f.content LIKE ? ESCAPE '\\'")
            .collect();
        sql.push_str(&likes.join(" AND "));
    } else {
        sql.push_str("fulltext MATCH ?");
    }

    // view スコープ（CR-001）。trash ビュー時はゴミ箱内、それ以外は現役のみ。
    if matches!(view, Some("trash")) {
        sql.push_str(
            " AND a.entry_id IN (SELECT id FROM entries WHERE deleted_at IS NOT NULL)",
        );
    } else {
        sql.push_str(
            " AND a.entry_id IN (SELECT id FROM entries WHERE deleted_at IS NULL)",
        );
    }

    if collection_id.is_some() {
        sql.push_str(
            " AND a.entry_id IN (SELECT entry_id FROM entry_collections WHERE collection_id = ?)",
        );
    }
    if tag_id.is_some() {
        sql.push_str(" AND a.entry_id IN (SELECT entry_id FROM entry_tags WHERE tag_id = ?)");
    }

    if use_like {
        sql.push_str(" ORDER BY f.attachment_id, f.page");
    } else {
        sql.push_str(" ORDER BY bm25(fulltext)");
    }

    let mut q = sqlx::query(&sql);
    if use_like {
        for token in &tokens {
            q = q.bind(crate::db::entries::like_pattern(token));
        }
    } else {
        q = q.bind(build_match_expr(&tokens));
    }
    if let Some(cid) = collection_id {
        q = q.bind(cid);
    }
    if let Some(tid) = tag_id {
        q = q.bind(tid);
    }

    let rows = q.fetch_all(pool).await?;

    let mut hits = Vec::with_capacity(rows.len());
    for row in rows {
        let entry_id: i64 = row.get("entry_id");
        let summary = load_summary(pool, entry_id).await?;
        hits.push(FulltextHit {
            entry: summary,
            attachment_id: row.get("attachment_id"),
            page: row.get("page"),
            snippet: row.get("snippet"),
        });
    }

    Ok(hits)
}

/// エントリ要約を読む。全文検索系（`fulltext` / `document_nodes_fts`）のヒットで共有する。
pub(crate) async fn load_summary(pool: &SqlitePool, id: i64) -> Result<EntrySummary, sqlx::Error> {
    let row = sqlx::query(
        "SELECT id, title, year, entry_type, created_at, starred FROM entries WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    let authors: Vec<Author> = sqlx::query_as(
        "SELECT a.id, a.name,
                a.given_name, a.middle_name, a.family_name, a.suffix, a.name_particle,
                a.name_original, a.given_name_original, a.family_name_original, a.original_script,
                a.reading_family, a.reading_given,
                a.is_organization,
                a.email, a.homepage_url, a.notes,
                a.orcid, a.updated_at
         FROM authors a
         JOIN entry_authors ea ON ea.author_id = a.id
         WHERE ea.entry_id = ?
         ORDER BY ea.position",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let tags: Vec<Tag> = sqlx::query_as(
        "SELECT t.id, t.name FROM tags t
         JOIN entry_tags et ON et.tag_id = t.id
         WHERE et.entry_id = ?",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let journal: Option<String> = sqlx::query_scalar(
        "SELECT field_value FROM extra_fields WHERE entry_id = ? AND field_name = 'journal'",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(EntrySummary {
        id: row.get("id"),
        title: row.get("title"),
        year: row.get("year"),
        entry_type: row.get("entry_type"),
        created_at: row.get("created_at"),
        authors,
        tags,
        has_attachment: true,
        journal,
        starred: row.get::<i64, _>("starred") != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    async fn setup_attachment(pool: &SqlitePool, title: &str) -> (i64, i64) {
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

        // file_path は UNIQUE（CR-008）なので entry.id を含めて一意にする。
        let att = add_attachment(
            pool,
            entry.id,
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap();

        (entry.id, att.id)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn indexed_text_is_searchable(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(
            &pool,
            att_id,
            &[(1, "Transformer architecture is described here.".to_string())],
        )
        .await
        .unwrap();

        let hits = search_fulltext(&pool, "transformer", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attachment_id, att_id);
        assert_eq!(hits[0].page, 1);
        assert!(hits[0].snippet.to_lowercase().contains("transformer"));
    }

    /// 自己修復は 1 回だけ走り（`true`）、2 回目以降は flag で skip（`false`）。
    /// 再構築後も既存の索引内容は検索でき、FTS5 integrity-check を通る。
    #[sqlx::test(migrations = "./migrations")]
    async fn rebuild_fulltext_fts_once_is_idempotent_and_healthy(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(
            &pool,
            att_id,
            &[(1, "Transformer architecture is described here.".to_string())],
        )
        .await
        .unwrap();

        let first = rebuild_fulltext_fts_once(&pool).await.unwrap();
        assert!(first, "初回は再構築が走る");
        let second = rebuild_fulltext_fts_once(&pool).await.unwrap();
        assert!(!second, "2 回目は flag で skip");

        // 再構築後も検索でき、FTS5 の integrity-check を通る。
        let hits = search_fulltext(&pool, "transformer", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
        sqlx::query("INSERT INTO fulltext(fulltext) VALUES('integrity-check')")
            .execute(&pool)
            .await
            .expect("rebuild 後は FTS5 integrity-check を通る");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn index_returns_one_hit_per_page(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(
            &pool,
            att_id,
            &[
                (1, "Introduction to attention mechanisms.".to_string()),
                (2, "Attention layer details and equations.".to_string()),
                (3, "Conclusion section without the keyword.".to_string()),
            ],
        )
        .await
        .unwrap();

        let hits = search_fulltext(&pool, "attention", None, None, None).await.unwrap();
        let pages: Vec<i64> = hits.iter().map(|h| h.page).collect();
        assert!(pages.contains(&1));
        assert!(pages.contains(&2));
        assert!(!pages.contains(&3));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reindexing_replaces_old_rows(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(&pool, att_id, &[(1, "old keyword".to_string())])
            .await
            .unwrap();
        index_attachment(&pool, att_id, &[(1, "fresh content only".to_string())])
            .await
            .unwrap();

        let stale = search_fulltext(&pool, "old", None, None, None).await.unwrap();
        let fresh = search_fulltext(&pool, "fresh", None, None, None).await.unwrap();
        assert!(stale.is_empty());
        assert_eq!(fresh.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_attachment_pages_replaces_only_given_pages(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(
            &pool,
            att_id,
            &[
                (1, "page one original".to_string()),
                (2, "page two original".to_string()),
                (3, "page three original".to_string()),
            ],
        )
        .await
        .unwrap();

        // ページ 2 だけ差し替える。1 と 3 は保持される。
        update_attachment_pages(&pool, att_id, &[(2, "page two replaced".to_string())])
            .await
            .unwrap();

        assert_eq!(search_fulltext(&pool, "original", None, None, None).await.unwrap().len(), 2);
        assert_eq!(search_fulltext(&pool, "replaced", None, None, None).await.unwrap().len(), 1);
        assert!(search_fulltext(&pool, "two original", None, None, None).await.unwrap().is_empty());

        // 空文字列に差し替えた場合はそのページの行が消える（再OCRで空だったケース）
        update_attachment_pages(&pool, att_id, &[(3, "".to_string())]).await.unwrap();
        let row_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
                .bind(att_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row_count, 2); // page 1 original + page 2 replaced
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_removes_fulltext_rows(pool: SqlitePool) {
        let (entry_id, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att_id, &[(1, "needle".to_string())])
            .await
            .unwrap();

        crate::db::entries::delete_entry(&pool, entry_id).await.unwrap();

        let orphans: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
                .bind(att_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(orphans, 0, "hard delete must not orphan fulltext rows");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unindex_removes_rows(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att_id, &[(1, "needle".to_string())])
            .await
            .unwrap();

        unindex_attachment(&pool, att_id).await.unwrap();

        let hits = search_fulltext(&pool, "needle", None, None, None).await.unwrap();
        assert!(hits.is_empty());
        assert!(!is_indexed(&pool, att_id).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_pages_are_skipped(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;

        index_attachment(
            &pool,
            att_id,
            &[
                (1, "".to_string()),
                (2, "  \n  ".to_string()),
                (3, "real content".to_string()),
            ],
        )
        .await
        .unwrap();

        let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM fulltext WHERE attachment_id = ?")
            .bind(att_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row_count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_query_returns_empty(pool: SqlitePool) {
        let hits = search_fulltext(&pool, "  ", None, None, None).await.unwrap();
        assert!(hits.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_respects_collection_filter(pool: SqlitePool) {
        let col_id = sqlx::query("INSERT INTO collections (name) VALUES ('Inbox')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let (entry_in, att_in) = setup_attachment(&pool, "Inside").await;
        let (_, att_out) = setup_attachment(&pool, "Outside").await;

        sqlx::query("INSERT INTO entry_collections (entry_id, collection_id) VALUES (?, ?)")
            .bind(entry_in)
            .bind(col_id)
            .execute(&pool)
            .await
            .unwrap();

        index_attachment(&pool, att_in, &[(1, "transformer paper".to_string())])
            .await
            .unwrap();
        index_attachment(&pool, att_out, &[(1, "transformer review".to_string())])
            .await
            .unwrap();

        let hits = search_fulltext(&pool, "transformer", Some(col_id), None, None)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].attachment_id, att_in);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_matches_japanese_substring(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "和文").await;
        index_attachment(
            &pool,
            att_id,
            &[(1, "本論文では深層学習モデルの精度を評価する。".to_string())],
        )
        .await
        .unwrap();

        let hits = search_fulltext(&pool, "深層学習", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_entry_removes_fulltext_of_all_attachments(pool: SqlitePool) {
        let (entry_id, att1) = setup_attachment(&pool, "Paper").await;
        let att2 = add_attachment(
            &pool,
            entry_id,
            "attachments/x/p2.pdf",
            "p2.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;

        index_attachment(&pool, att1, &[(1, "alpha".to_string())])
            .await
            .unwrap();
        index_attachment(&pool, att2, &[(1, "beta".to_string())])
            .await
            .unwrap();

        crate::db::entries::delete_entry(&pool, entry_id).await.unwrap();

        let hits_a = search_fulltext(&pool, "alpha", None, None, None).await.unwrap();
        let hits_b = search_fulltext(&pool, "beta", None, None, None).await.unwrap();
        assert!(hits_a.is_empty());
        assert!(hits_b.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn like_fallback_treats_wildcards_literally(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(
            &pool,
            att_id,
            &[
                (1, "uses a_b indexing".to_string()),
                (2, "uses acb indexing".to_string()),
            ],
        )
        .await
        .unwrap();

        // 短いトークン → LIKE フォールバック。`_` はリテラル扱いであること。
        let hits = search_fulltext(&pool, "a_", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1, "`_` must not act as a wildcard");
        assert_eq!(hits[0].page, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn short_query_uses_like_fallback(pool: SqlitePool) {
        let (_, att_id) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att_id, &[(1, "AI models are evolving rapidly".to_string())])
            .await
            .unwrap();

        let hits = search_fulltext(&pool, "AI", None, None, None).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attachments_without_fulltext_lists_only_unindexed(pool: SqlitePool) {
        let (_, indexed) = setup_attachment(&pool, "Indexed").await;
        let (_, unindexed) = setup_attachment(&pool, "Unindexed").await;
        index_attachment(&pool, indexed, &[(1, "some text".to_string())])
            .await
            .unwrap();

        let missing = attachments_without_fulltext(&pool).await.unwrap();
        let ids: Vec<i64> = missing.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&unindexed));
        assert!(!ids.contains(&indexed));
        // file_path も返る。
        assert!(missing.iter().any(|(_, p)| p.ends_with("/p.pdf")));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attachments_without_fulltext_excludes_trashed_entries(pool: SqlitePool) {
        let (entry_id, att_id) = setup_attachment(&pool, "Trashed").await;
        crate::db::entries::trash_entry(&pool, entry_id).await.unwrap();

        let missing = attachments_without_fulltext(&pool).await.unwrap();
        assert!(!missing.iter().any(|(id, _)| *id == att_id));
    }

    // ---- v1.0.0-p1（索引の出どころ）------------------------------------------

    /// **索引を消したら出どころの記録も消す。**
    /// 残すと `index_attachment_from_pdf_extract` が永久に譲り続け、その添付は
    /// もう二度と索引されない（守るはずの中身がもう無いのに守り続ける）。
    #[sqlx::test(migrations = "./migrations")]
    async fn unindexing_clears_the_source_record(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Paper").await;
        index_attachment(&pool, att, &[(1, "derived body".to_string())])
            .await
            .unwrap();
        set_fulltext_source(&pool, att, FulltextSource::Lcir)
            .await
            .unwrap();

        unindex_attachment(&pool, att).await.unwrap();

        assert_eq!(get_fulltext_source(&pool, att).await.unwrap(), None);
        assert_eq!(
            index_attachment_from_pdf_extract(&pool, att, &[(1, "fresh text".to_string())], false)
                .await
                .unwrap(),
            PdfExtractWrite::Replaced,
            "記録が消えていれば pdf_extract で張り直せる"
        );
    }

    /// **名指しの再索引（`replace_existing`）は LCIR 由来を張り直せるが、OCR 由来は守る。**
    ///
    /// 一律に守ると、詳細パネルの再索引ボタンが黙って何もしない経路ができる（LCIR を後から
    /// OFF にした人が古い派生索引から抜け出せない）。逆に一律に許すと、ユーザーが課金して
    /// 起こした OCR の転写がボタン 1 つで消える。判定は書き込みと同じ tx の 1 箇所に置く。
    #[sqlx::test(migrations = "./migrations")]
    async fn explicit_reindex_replaces_lcir_but_not_ocr(pool: SqlitePool) {
        let (_, lcir_att) = setup_attachment(&pool, "Derived").await;
        index_attachment(&pool, lcir_att, &[(1, "derived body".to_string())])
            .await
            .unwrap();
        set_fulltext_source(&pool, lcir_att, FulltextSource::Lcir)
            .await
            .unwrap();

        // 自動経路（`replace_existing = false`）は譲る。
        assert_eq!(
            index_attachment_from_pdf_extract(
                &pool,
                lcir_att,
                &[(1, "stale output".to_string())],
                false
            )
            .await
            .unwrap(),
            PdfExtractWrite::SkippedProtected
        );
        // 名指し（`true`）は張り直し、出どころの記録も既定に戻す。
        assert_eq!(
            index_attachment_from_pdf_extract(
                &pool,
                lcir_att,
                &[(1, "reindexed body".to_string())],
                true
            )
            .await
            .unwrap(),
            PdfExtractWrite::Replaced
        );
        assert_eq!(get_fulltext_source(&pool, lcir_att).await.unwrap(), None);
        assert_eq!(
            search_fulltext(&pool, "reindexed", None, None, None)
                .await
                .unwrap()
                .len(),
            1
        );

        // OCR 由来は名指しでも守る。
        let (_, ocr_att) = setup_attachment(&pool, "Scanned").await;
        index_attachment(&pool, ocr_att, &[(1, "ocr transcript".to_string())])
            .await
            .unwrap();
        set_fulltext_source(&pool, ocr_att, FulltextSource::Ocr)
            .await
            .unwrap();

        assert_eq!(
            index_attachment_from_pdf_extract(
                &pool,
                ocr_att,
                &[(1, "garbage layer".to_string())],
                true
            )
            .await
            .unwrap(),
            PdfExtractWrite::SkippedProtected
        );
        assert_eq!(
            get_fulltext_source(&pool, ocr_att).await.unwrap(),
            Some(FulltextSource::Ocr)
        );
        assert_eq!(
            search_fulltext(&pool, "transcript", None, None, None)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    // ---- v1.0.0（②b の W1-1）空の抽出結果で既存索引を消さない ---------------------

    /// **本命**: 出どころの記録が無い添付（= p1 より前の OCR かもしれない）に空の抽出結果が
    /// 来ても、既存の索引を消さない。名指しの再索引でも守る。
    ///
    /// これが無いと、確認ダイアログの無い同期アイコンを 1 回押すだけで、課金して起こした
    /// 転写が全削除される（`replace_pages` は無条件 `DELETE` して空ページを `INSERT` しない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_extract_keeps_an_unrecorded_index(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Scanned").await;
        index_attachment(
            &pool,
            att,
            &[
                (1, "ocr transcript one".to_string()),
                (2, "ocr transcript two".to_string()),
                (3, "ocr transcript three".to_string()),
            ],
        )
        .await
        .unwrap();
        // p1 より前の OCR には出どころの記録が無い（= この状態が守る対象そのもの）。
        assert_eq!(get_fulltext_source(&pool, att).await.unwrap(), None);

        let empty = [(1, String::new()), (2, "   \n\t ".to_string()), (3, String::new())];

        // 名指しの再索引（`replace_existing = true`）でも守る。
        assert_eq!(
            index_attachment_from_pdf_extract(&pool, att, &empty, true)
                .await
                .unwrap(),
            PdfExtractWrite::SkippedEmptyExtract
        );
        assert_eq!(indexed_page_count(&pool, att).await.unwrap(), 3);
        // 自動経路でも同じ。
        assert_eq!(
            index_attachment_from_pdf_extract(&pool, att, &empty, false)
                .await
                .unwrap(),
            PdfExtractWrite::SkippedEmptyExtract
        );
        assert_eq!(indexed_page_count(&pool, att).await.unwrap(), 3);
        assert_eq!(
            search_fulltext(&pool, "transcript", None, None, None)
                .await
                .unwrap()
                .len(),
            3,
            "課金して起こした転写が残っている"
        );
    }

    /// LCIR 由来の索引にも同じガードが効き、**出どころの記録も残る**。
    ///
    /// ここで記録だけ消すと「行は LCIR 由来なのに記録は pdf_extract」という嘘になる。
    /// 代償は正直に書く: この添付（LCIR を OFF にした × テキスト層が無い）は
    /// 再索引ボタンでは LCIR 由来から抜けられなくなる ── 抜けたい人は索引を削除する。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_extract_keeps_a_lcir_index_and_its_record(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Derived").await;
        index_attachment(&pool, att, &[(1, "derived body".to_string())])
            .await
            .unwrap();
        set_fulltext_source(&pool, att, FulltextSource::Lcir)
            .await
            .unwrap();

        assert_eq!(
            index_attachment_from_pdf_extract(&pool, att, &[(1, String::new())], true)
                .await
                .unwrap(),
            PdfExtractWrite::SkippedEmptyExtract
        );
        assert_eq!(indexed_page_count(&pool, att).await.unwrap(), 1);
        assert_eq!(
            get_fulltext_source(&pool, att).await.unwrap(),
            Some(FulltextSource::Lcir),
            "行が LCIR 由来のままなら記録も LCIR のまま"
        );
    }

    /// **AND 条件の番人。** 既存が 0 行なら、空の抽出結果でも通す。
    ///
    /// ガードを「非空 0 件」だけに簡約すると、`source = lcir` かつ索引 0 行の添付で
    /// 記録の後始末（`DELETE FROM settings`）が飛び、**キーだけが残って以後の自動経路が
    /// 永久に譲り続ける**（守る中身がもう無いのに守り続ける）。
    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_extract_with_no_existing_rows_still_clears_the_record(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Emptied").await;
        set_fulltext_source(&pool, att, FulltextSource::Lcir)
            .await
            .unwrap();
        assert_eq!(indexed_page_count(&pool, att).await.unwrap(), 0);

        assert_eq!(
            index_attachment_from_pdf_extract(&pool, att, &[(1, String::new())], true)
                .await
                .unwrap(),
            PdfExtractWrite::Replaced
        );
        assert_eq!(
            get_fulltext_source(&pool, att).await.unwrap(),
            None,
            "守る中身が無いなら記録も残さない"
        );
    }

    /// **OCR 記録は空抽出ガードより先に効く。**
    ///
    /// 順序が逆だと、OCR 由来のスキャン本に空抽出が来たとき `SkippedProtected` ではなく
    /// `SkippedEmptyExtract` が返り、詳細パネルの「OCR で取り込んだ本文を残しました
    /// （取り直すには OCR を再実行）」という文言が、**まさにスキャン本で出なくなる**。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_ocr_record_is_reported_ahead_of_the_empty_extract_guard(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Scanned").await;
        index_attachment(&pool, att, &[(1, "ocr transcript".to_string())])
            .await
            .unwrap();
        set_fulltext_source(&pool, att, FulltextSource::Ocr)
            .await
            .unwrap();

        assert_eq!(
            index_attachment_from_pdf_extract(&pool, att, &[(1, String::new())], true)
                .await
                .unwrap(),
            PdfExtractWrite::SkippedProtected
        );
    }

    /// **守るのは「非空 0 件」だけで、縮小は守らない**（境界を固定する）。
    ///
    /// 既存 3 ページに対して非空 1 ページの抽出結果が来たら、そのまま 1 ページに置き換える。
    /// ここまで守ると「本当に内容が減った PDF を張り直せない」になる。
    #[sqlx::test(migrations = "./migrations")]
    async fn a_shrinking_extract_is_not_protected(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Shrunk").await;
        index_attachment(
            &pool,
            att,
            &[
                (1, "old one".to_string()),
                (2, "old two".to_string()),
                (3, "old three".to_string()),
            ],
        )
        .await
        .unwrap();

        assert_eq!(
            index_attachment_from_pdf_extract(
                &pool,
                att,
                &[(1, "only page".to_string()), (2, String::new()), (3, String::new())],
                true
            )
            .await
            .unwrap(),
            PdfExtractWrite::Replaced
        );
        assert_eq!(indexed_page_count(&pool, att).await.unwrap(), 1);
    }

    // ---- v1.0.0（②b の W1-2）tx 内で「記録の無い既存索引」を守る -------------------

    /// 自動の再導出は、**tx の中でも**記録の無い既存索引を守る。
    ///
    /// 呼び出し側にも同じ判定があるが、あちらは tx の外なので check-then-act の窓が開く ──
    /// 評価した直後に部分 OCR（封印しないので記録が立たない）が着地すると、
    /// 課金済みの転写を LCIR で置き換える。**同じテストの中で `false` 側も確かめる**
    /// （守るだけのテストは、引数を無視する実装でも通ってしまう）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_derive_path_can_protect_an_unrecorded_index_inside_the_tx(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Partially OCRd").await;
        index_attachment(&pool, att, &[(1, "interrupted ocr transcript".to_string())])
            .await
            .unwrap();
        assert_eq!(get_fulltext_source(&pool, att).await.unwrap(), None);

        let lcir = [(1, "lcir page text".to_string())];

        assert_eq!(
            index_attachment_from_lcir(&pool, att, &lcir, |s| s.to_string(), true)
                .await
                .unwrap(),
            LcirWrite::SkippedUnrecorded
        );
        assert_eq!(
            search_fulltext(&pool, "interrupted", None, None, None)
                .await
                .unwrap()
                .len(),
            1,
            "記録の無い転写が残っている"
        );
        assert_eq!(
            get_fulltext_source(&pool, att).await.unwrap(),
            None,
            "守ったのだから lcir を名乗らない"
        );

        // **生存確認**: 守らないと言われたら置き換える（＝上の assert が空ではない）。
        assert_eq!(
            index_attachment_from_lcir(&pool, att, &lcir, |s| s.to_string(), false)
                .await
                .unwrap(),
            LcirWrite::Replaced
        );
        assert_eq!(
            get_fulltext_source(&pool, att).await.unwrap(),
            Some(FulltextSource::Lcir)
        );
    }

    /// 守る対象は「記録が無い **かつ** 既存行がある」。索引がまだ無い添付は普通に埋める
    /// （`AddMissingOnly` の本来の仕事がここで止まると、p1 が 1 件も進まない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_derive_path_still_fills_an_attachment_with_no_index(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Unindexed").await;

        assert_eq!(
            index_attachment_from_lcir(
                &pool,
                att,
                &[(1, "lcir page text".to_string())],
                |s| s.to_string(),
                true
            )
            .await
            .unwrap(),
            LcirWrite::Replaced
        );
        assert_eq!(indexed_page_count(&pool, att).await.unwrap(), 1);
    }

    /// ゴミ箱の一括 purge でも記録を残さない（`purge_one` 側の後始末）。
    #[sqlx::test(migrations = "./migrations")]
    async fn purging_trash_clears_source_records(pool: SqlitePool) {
        let (entry_id, att) = setup_attachment(&pool, "Trashed").await;
        let (_, kept_att) = setup_attachment(&pool, "Kept").await;
        set_fulltext_source(&pool, att, FulltextSource::Ocr)
            .await
            .unwrap();
        set_fulltext_source(&pool, kept_att, FulltextSource::Lcir)
            .await
            .unwrap();
        crate::db::entries::trash_entry(&pool, entry_id).await.unwrap();

        crate::db::entries::purge_trash(&pool).await.unwrap();

        assert_eq!(get_fulltext_source(&pool, att).await.unwrap(), None);
        assert_eq!(
            get_fulltext_source(&pool, kept_att).await.unwrap(),
            Some(FulltextSource::Lcir),
            "ゴミ箱に無いエントリの記録まで消してはいけない"
        );
    }

    /// 添付を消したら出どころの記録も残さない（settings に孤児キーを溜めない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_an_attachment_clears_the_source_record(pool: SqlitePool) {
        let (_, att) = setup_attachment(&pool, "Paper").await;
        set_fulltext_source(&pool, att, FulltextSource::Ocr)
            .await
            .unwrap();

        crate::db::attachments::delete_attachment_with_fulltext(&pool, att)
            .await
            .unwrap();

        assert_eq!(get_fulltext_source(&pool, att).await.unwrap(), None);
    }

    /// エントリの hard delete でも同じ（`fulltext` / `document_nodes_fts` と同じ扱い）。
    /// **消すのはそのエントリ配下だけ**（他のエントリの記録を巻き添えにしない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_an_entry_clears_source_records_of_its_attachments(pool: SqlitePool) {
        let (entry_id, att) = setup_attachment(&pool, "Paper").await;
        let (_, other_att) = setup_attachment(&pool, "Other paper").await;
        set_fulltext_source(&pool, att, FulltextSource::Lcir)
            .await
            .unwrap();
        set_fulltext_source(&pool, other_att, FulltextSource::Ocr)
            .await
            .unwrap();

        crate::db::entries::delete_entry(&pool, entry_id).await.unwrap();

        assert_eq!(get_fulltext_source(&pool, att).await.unwrap(), None);
        assert_eq!(
            get_fulltext_source(&pool, other_att).await.unwrap(),
            Some(FulltextSource::Ocr),
            "別エントリの記録まで消してはいけない"
        );
    }
}
