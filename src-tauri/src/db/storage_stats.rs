//! DB ファイルの容量内訳と、LCIR の GC で回収できる量の見積り（v1.0.0-p4）。
//!
//! **「予告は行数・事後はバイト」で非対称にしてある。** GC の前に「何 MB 空きます」を
//! 出すには「表全体のバイト × 死行率」の按分推定しか作れず、実測すると 6.5% 上振れした
//! （free page になるのは丸ごと空いたページだけで、生き残り行が 1 行でも載っている
//! ページは解放されないため）。だから確認ダイアログには**行数と安全述語**を出し、
//! バイトは GC 後に `freelist_count` の実測差分として報告する。
//!
//! `dbstat` 仮想表はこのビルドで実際に使える（`libsqlite3-sys` の bundled 経路が
//! `-DSQLITE_ENABLE_DBSTAT_VTAB` 付き）が、**使わない**。使用中 / 再利用可は
//! `(page_count - freelist_count) * page_size` で `dbstat` と 1 バイトも違わない値が
//! マイクロ秒で出るからで、`dbstat` が要るのは表ごとの内訳を出すときだけ。

use serde::Serialize;
use sqlx::SqlitePool;

/// DB ファイルのページ収支。3 値とも 1 クエリで取れる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DbSize {
    pub page_size: i64,
    pub page_count: i64,
    pub free_pages: i64,
}

impl DbSize {
    /// ファイル全体のバイト数。`auto_vacuum = 0`（migration に `PRAGMA auto_vacuum` は
    /// 1 件も無い）なので ptrmap ページが無く、これは実ファイルサイズと一致する。
    pub fn file_bytes(&self) -> i64 {
        self.page_size * self.page_count
    }

    /// 実データが載っているバイト数。
    pub fn used_bytes(&self) -> i64 {
        self.page_size * (self.page_count - self.free_pages)
    }

    /// 解放済みで再利用待ちのバイト数。**GC してもファイルは縮まない**ぶんがここに出る。
    pub fn free_bytes(&self) -> i64 {
        self.page_size * self.free_pages
    }
}

/// `page_size` / `page_count` / `freelist_count` を 1 往復で読む。
pub async fn db_size(pool: &SqlitePool) -> Result<DbSize, sqlx::Error> {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT * FROM pragma_page_size),
                (SELECT * FROM pragma_page_count),
                (SELECT * FROM pragma_freelist_count)",
    )
    .fetch_one(pool)
    .await?;
    Ok(DbSize {
        page_size: row.0,
        page_count: row.1,
        free_pages: row.2,
    })
}

/// GC を押す前に見せる内訳。**`alt_texts_at_risk` と `carry_refs_at_risk` が
/// この構造体の存在理由**で、ここが 0 でないかどうかが「押していいボタンか」を決める。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct GcPreview {
    /// 回収対象の版数（述語を満たすもの）。
    pub versions: i64,
    /// そのうち、行ごと消せる版数（carry 元として参照されていない）。
    pub versions_removable: i64,
    /// そのうち、**子だけ消して行を残す**版数（生存 alt text の carry 元）。
    pub versions_tombstoned: i64,
    /// 消える `document_nodes` の行数。
    pub nodes: i64,
    /// 消える `assets` の行数と、それが指す crop ファイルの合計バイト数。
    pub asset_rows: i64,
    pub asset_bytes: i64,
    /// **安全述語**: 回収対象から外した版が抱えている alt text の行数。
    /// 0 でなければ「課金済みの説明を持つ superseded 版がある」ということなので、
    /// 対象から外れていること自体が正しい（表示は情報提供）。
    pub alt_texts_protected: i64,
    /// 生存 alt text の `carried_from_version_id` から参照されている版数
    /// （= 行を残す理由。`versions_tombstoned` と同値だが由来を明示する）。
    pub carry_refs_protected: i64,
    /// superseded だが**生存版が 1 本も無い**添付の版数（回収すると LCIR が丸ごと消える）。
    pub orphan_versions_skipped: i64,
}

/// 削除対象版の SQL 述語（`dv` に `document_versions` を束縛して使う）。
///
/// 条件は 4 つ。**(i)(ii) は `docs/LCIR_REMAINING_PHASES.md` §7 の定式化、(iii)(iv) は
/// 実スキーマで検算して足したもの**:
///
/// - (i) この版に紐づく `node_alt_texts` が 0 件。破ると課金済みの `llm_inference` と
///   人が書いた `user_edited` が無音で消える。crop PNG は trash 済み・新版に `page_crop` が
///   無いので `figures_missing_alt_text` の対象にも戻らず**復旧不能**。
/// - (ii) （行を消す条件・述語ではない）生存行の `carried_from_version_id` から
///   参照されていない。`ON DELETE SET NULL` なので、消すと carry 行が
///   「NULL = この版で生成」というスキーマの契約を偽る。**満たさない版は子だけ消して
///   行を残す**（`versions_tombstoned`）── 満たすまで待つ設計にすると、#7 の再構築後は
///   最初の生成版が永久に carry 元として指され続けるので二度と回収できない。
/// - (iii) その添付に生存版（`completed` 系）が実在する。破ると LCIR が丸ごと消え、
///   `attachments_without_completed_lcir` が再構築対象に戻す（att37 なら 75 分）。
/// - (iv) まだ回収するものが残っている（ノード or アセット or 消せる行）。
///   これが無いと、行を残した版を毎回対象に数え続けて収束しない。
pub const GC_TARGET_PREDICATE: &str = "
    dv.extraction_status = 'superseded'
    AND NOT EXISTS (
        SELECT 1 FROM node_alt_texts n WHERE n.document_version_id = dv.id
    )
    AND EXISTS (
        SELECT 1 FROM document_versions live
         WHERE live.attachment_id = dv.attachment_id
           AND live.extraction_status IN ('completed', 'completed_with_warnings')
    )
    AND (
        EXISTS (SELECT 1 FROM document_nodes dn WHERE dn.document_version_id = dv.id)
        OR EXISTS (SELECT 1 FROM assets a WHERE a.document_version_id = dv.id)
        OR NOT EXISTS (
            SELECT 1 FROM node_alt_texts c WHERE c.carried_from_version_id = dv.id
        )
    )
";

/// 回収対象の版 id を昇順で返す。
pub async fn gc_target_versions(pool: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT dv.id FROM document_versions dv WHERE {GC_TARGET_PREDICATE} ORDER BY dv.id"
    ))
    .fetch_all(pool)
    .await
}

/// 押す前の見積り。`source_fragments` の COUNT は実 DB で 0.79 秒かかり、しかも
/// ノード数とほぼ 1:1（実測 99.9%）なので**引かない**。安全述語は 3 本とも 1ms 未満。
pub async fn gc_preview(pool: &SqlitePool) -> Result<GcPreview, sqlx::Error> {
    let targets = format!(
        "SELECT dv.id FROM document_versions dv WHERE {GC_TARGET_PREDICATE}"
    );
    let carry_ref = "EXISTS (SELECT 1 FROM node_alt_texts c \
                     WHERE c.carried_from_version_id = document_versions.id)";

    let row: (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(&format!(
        "SELECT
            (SELECT COUNT(*) FROM ({targets})),
            (SELECT COUNT(*) FROM document_versions
              WHERE id IN ({targets}) AND NOT {carry_ref}),
            (SELECT COUNT(*) FROM document_versions
              WHERE id IN ({targets}) AND {carry_ref}),
            (SELECT COUNT(*) FROM document_nodes WHERE document_version_id IN ({targets})),
            (SELECT COUNT(*) FROM assets WHERE document_version_id IN ({targets})),
            (SELECT COALESCE(SUM(size_bytes), 0) FROM assets
              WHERE document_version_id IN ({targets})),
            (SELECT COUNT(*) FROM node_alt_texts
              WHERE document_version_id IN (
                  SELECT id FROM document_versions WHERE extraction_status = 'superseded'
              )),
            (SELECT COUNT(*) FROM document_versions dv
              WHERE dv.extraction_status = 'superseded'
                AND NOT EXISTS (
                    SELECT 1 FROM document_versions live
                     WHERE live.attachment_id = dv.attachment_id
                       AND live.extraction_status IN ('completed', 'completed_with_warnings')
                ))"
    ))
    .fetch_one(pool)
    .await?;

    Ok(GcPreview {
        versions: row.0,
        versions_removable: row.1,
        versions_tombstoned: row.2,
        nodes: row.3,
        asset_rows: row.4,
        asset_bytes: row.5,
        alt_texts_protected: row.6,
        carry_refs_protected: row.2,
        orphan_versions_skipped: row.7,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_size_splits_the_file_into_used_and_reusable() {
        // 実 DB の実測値（2026-08-05・GC 前）。
        let before = DbSize {
            page_size: 4096,
            page_count: 185_849,
            free_pages: 0,
        };
        assert_eq!(before.file_bytes(), 761_237_504, "ls のサイズと一致する");
        assert_eq!(before.used_bytes(), 761_237_504);
        assert_eq!(before.free_bytes(), 0);

        // GC 後（コピー DB で完走させた実測）。**ファイルは 1 バイトも縮まない。**
        let after = DbSize {
            page_size: 4096,
            page_count: 185_849,
            free_pages: 120_577,
        };
        assert_eq!(after.file_bytes(), 761_237_504, "GC ではファイルは縮まない");
        assert_eq!(after.free_bytes(), 493_883_392);
        assert_eq!(after.used_bytes() + after.free_bytes(), after.file_bytes());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn db_size_reads_the_pragmas(pool: SqlitePool) {
        let s = db_size(&pool).await.unwrap();
        assert!(s.page_size >= 512, "page_size = {}", s.page_size);
        assert!(s.page_count > 0);
        assert_eq!(s.used_bytes() + s.free_bytes(), s.file_bytes());
    }
}
