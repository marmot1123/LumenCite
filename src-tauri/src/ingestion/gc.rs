//! superseded 版の GC（v1.0.0-p4）。
//!
//! LCIR は再構築のたびに旧版を `superseded` にして残す。provenance としては正しいが、
//! 実 DB では **`document_nodes` の 83% が superseded 版のもの**になっていた
//! （2,663,234 行中 2,213,085 行）。ここはその回収を行う。
//!
//! **なぜ再構築（#7）より前に実行するか**が設計の中心。今の削除対象 145 版は
//! `node_alt_texts` 0 行 / `assets` 0 行なので、**述語がバグっても失うものが実測でゼロ**。
//! 再構築の後だと、carry に失敗した課金済み alt text を抱えた版が対象になる。
//!
//! ## 3 つの実測が形を決めた
//!
//! 1. **`parent_version_id` は `NO ACTION`**（`0014_lcir_foundation.sql:21`）で、実 DB の
//!    superseded 145/145 が新版から参照されている ＝ 素朴な `DELETE` は FK エラーで
//!    1 件も消えない。同一 tx の pre-step で NULL 化する。
//! 2. **大きい版の削除は 1 tx で 17 秒かかる**（実測・最大は 272,583 ノードの版）。
//!    プールの `busy_timeout` は 5 秒なので、版単位に割るだけでは他の書き手が
//!    `SQLITE_BUSY` で落ちる。**ノード数でチャンクを切る**。
//! 3. **`symbols(scope_node_id)` だけが未索引**だった（`document_nodes` を参照する
//!    子キーで唯一）。索引が無いと削除ノード 1 件ごとに `symbols` を全走査する。
//!    張ると実測で 125 秒 → 48 秒・最大 tx 16.7 秒 → 5.7 秒。
//!
//! ## 行を消す版と、子だけ消す版
//!
//! 生存 alt text の `carried_from_version_id` から参照されている版は**行を残す**
//! （`ON DELETE SET NULL` なので消すと carry 行が「NULL = この版で生成」という
//! スキーマの契約を偽る）。今は該当 0 件だが、**#7 の再構築後は最初の生成版が
//! すべてこれになる** ── そこで諦めると 45 万ノードが永久に回収できなくなるので、
//! 子（ノード・アセット・辺・記号）だけ消して骨を残す。回収量はほぼ同じで、
//! 実 DB の容量の 83% は `document_nodes` + `source_fragments` にある。

use crate::attachment_trash;
use crate::db::storage_stats;
use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::path::Path;

/// 1 トランザクションで消すノード数の上限。
///
/// 索引を張った後の実測が約 49,000 ノード/秒なので、50,000 で 1 秒強。プールの
/// `busy_timeout`（sqlx 既定 5 秒）に対して 4 倍以上の余裕を取ってある。
/// **版単位に割るだけでは足りない**（最大の版は 272,583 ノードで 5.7 秒）。
const NODE_DELETE_CHUNK: i64 = 50_000;

/// GC の実行結果。**回収したバイト数は `freelist_count` の実測差分**で、
/// 按分推定ではない（`db::storage_stats` の doc コメントを参照）。
#[derive(Debug, Clone, Default, Serialize)]
pub struct GcOutcome {
    /// 行ごと消した版数。
    pub versions_removed: i64,
    /// 子だけ消して行を残した版数（生存 alt text の carry 元）。
    pub versions_tombstoned: i64,
    /// 削除直前の再評価で対象から外した版数（**0 でなければ実行中に別経路が
    /// 書き込んだということ**なので、ログと UI に出す）。
    pub versions_skipped: i64,
    pub nodes_removed: i64,
    pub asset_rows_removed: i64,
    /// **実際に** trash へ送った crop ファイルの数。
    pub files_trashed: i64,
    /// `assets` 行はあったが実体が既に無かった crop の数（8a の build 時 GC が先に
    /// 回収済みの正常ケース）。**`files_trashed` と分けて数える**のが要点で、
    /// 合算すると「1 枚も当たらなかった」異常が正常と同じ見た目になる。
    pub files_already_gone: i64,
    /// 掃除した node-FTS の孤児行数（生存確認のセンチネル。通常 0）。
    pub fts_orphans_removed: i64,
    /// 再利用可になったバイト数（`freelist` の増分 × `page_size`）。
    pub freed_bytes: i64,
    /// 実行後のファイル収支。
    pub db_size: Option<storage_stats::DbSize>,
}

/// 削除対象の版が指していた crop のうち、**生存版のどれも指していない**ものを返す。
///
/// content_key からディレクトリ名を組んで消してはいけない。アセットディレクトリの
/// キーは `(attachment_id, content_key[..16])` であって版 id ではなく、
/// `(attachment_id, content_key)` の UNIQUE 索引は起動時 best-effort（既存重複があると
/// 張られない）なので、**同一ディレクトリを superseded 版と生存版が共有しうる**。
/// パスの集合差なら content_key を見ずに安全になる。
///
/// 純関数にしてあるのは、ここが「何を消すか」を決める唯一の判断だから
/// （FS を叩く層には判定を置かない）。
pub fn reclaimable_files(dead: &[String], live: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = dead
        .iter()
        .filter(|p| !live.contains(*p))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// この版がまだ回収対象の述語を満たしているか（削除直前の再評価）。
///
/// 対象一覧を引いてから実際に消すまでの間に、別インスタンスや
/// `build_lcir_for_attachment`（**排他フラグを 1 つも持たない**）が新版を作って
/// 旧 completed を supersede しうる。その版の alt text がまだ刈られていない状態で
/// 削除対象に紛れると、課金済みの説明が無音で消える。Vision バッチが課金直前に
/// `still_latest` を再確認するのと同じ型。
/// 条件は `db::storage_stats::GC_TARGET_PREDICATE` の (i)(iii) と同じ。
/// (iv)（回収するものが残っているか）はここでは見ない ── 途中まで消した版を
/// 再開できなくなるため。
async fn still_collectable(pool: &SqlitePool, version_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM document_versions dv
              WHERE dv.id = ?
                AND dv.extraction_status = 'superseded'
                AND NOT EXISTS (
                    SELECT 1 FROM node_alt_texts n WHERE n.document_version_id = dv.id
                )
                AND NOT EXISTS (
                    SELECT 1 FROM node_alt_texts n
                      JOIN document_nodes dn ON dn.id = n.node_id
                     WHERE dn.document_version_id = dv.id
                )
                AND EXISTS (
                    SELECT 1 FROM document_versions live
                     WHERE live.attachment_id = dv.attachment_id
                       AND live.extraction_status IN ('completed', 'completed_with_warnings')
                )
         )",
    )
    .bind(version_id)
    .fetch_one(pool)
    .await
}

/// この版が生存 alt text の carry 元として参照されているか。
async fn is_carry_source(pool: &SqlitePool, version_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM node_alt_texts WHERE carried_from_version_id = ?)",
    )
    .bind(version_id)
    .fetch_one(pool)
    .await
}

/// 1 版ぶんの子（ノード・アセット・辺・記号）を消す。ノードはチャンクに割って
/// 1 tx あたりの保持時間を抑える。返り値は消したノード数。
async fn delete_version_children(
    pool: &SqlitePool,
    version_id: i64,
    chunk: i64,
) -> Result<(i64, i64), sqlx::Error> {
    let nodes_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM document_nodes WHERE document_version_id = ?")
            .bind(version_id)
            .fetch_one(pool)
            .await?;

    loop {
        let mut tx = pool.begin().await?;
        // **`ORDER BY id DESC` が本質。** `document_nodes.parent_id` は自己参照 CASCADE で、
        // 木は根（`document`）→ `page` → `block` → `line` の順に挿入される ＝ **子の id は
        // 親より大きい**。昇順で切ると 1 チャンク目に根が入り、その 1 行の削除が版の全ノードを
        // カスケードで消してしまう ── **チャンクが丸ごと無効化される**。
        // 実測（実 DB の版 34・272,583 ノード）: 昇順は `changes()` が **1** を返して
        // 残り **0**（= 1 tx で全部消えた・7.95 秒）／降順は `changes()` 378 で
        // ちょうど 50,000 減る。降順なら葉から剥がすので 1 tx が chunk 件に収まる。
        //
        // `rows_affected` はカスケードを数えない（降順でも 50,000 ではなく 378）ので、
        // **終了判定は残数で行う**（件数で判定すると取りこぼす）。
        let n = sqlx::query(
            "DELETE FROM document_nodes
              WHERE id IN (
                  SELECT id FROM document_nodes
                   WHERE document_version_id = ?
                   ORDER BY id DESC
                   LIMIT ?
              )",
        )
        .bind(version_id)
        .bind(chunk)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        if n == 0 {
            break;
        }
    }

    // **「消す前の数」ではなく実際に減った数を返す。** 前者だとチャンクのループが
    // 途中で抜けても報告値は満額のままで、取りこぼしが集計に出ない
    // （行ごと消す版では最後の版削除がカスケードで後始末をしてしまうため、
    //   残数を数えるこの引き算だけが取りこぼしを可視化する）。
    let nodes_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM document_nodes WHERE document_version_id = ?")
            .bind(version_id)
            .fetch_one(pool)
            .await?;

    // ノードにぶら下がらない版直下の行（ノード削除では消えない）。
    let mut tx = pool.begin().await?;
    let asset_rows = sqlx::query("DELETE FROM assets WHERE document_version_id = ?")
        .bind(version_id)
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;
    for table in ["node_relations", "symbols"] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE document_version_id = ?"
        ))
        .bind(version_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok((nodes_before - nodes_after, asset_rows))
}

/// 版の行そのものを消す。`parent_version_id` は `NO ACTION` なので、
/// **この版を指している行（生存版・削除対象どうしの両方）を先に NULL 化する**。
///
/// 失うのは将来の法医学的追跡だけ（`parent_version_id` にはリポジトリ内で読み手が
/// 1 つも無く、Phase 9a の JSON export にも出ない）。
async fn delete_version_row(pool: &SqlitePool, version_id: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE document_versions SET parent_version_id = NULL WHERE parent_version_id = ?")
        .bind(version_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM document_versions WHERE id = ?")
        .bind(version_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

/// node-FTS の孤児掃除。
///
/// `document_nodes_fts` は FTS5 仮想表で `node_id` に FK が無く、削除経路は
/// **添付単位しか無い**（版 id では一度も消されない）。通常は索引に載るのが
/// 生存版のノードだけなので孤児は出ないが、`mark_superseded_for_attachment` が
/// tx 内・`regenerate_node_fts_from_lcir` が tx 外で失敗しても `eprintln` だけ、
/// という窓が実在する。**件数を返して生存確認のセンチネルにする**（実 DB は現在 0 件）。
async fn sweep_fts_orphans(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let n = sqlx::query(
        "DELETE FROM document_nodes_fts
          WHERE node_id NOT IN (SELECT id FROM document_nodes)",
    )
    .execute(pool)
    .await?
    .rows_affected();
    Ok(n as i64)
}

/// 1 版をどう処理したか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// 子も版の行も消した。
    Removed,
    /// 子だけ消して版の行は残した（生存 alt text の carry 元）。
    Tombstoned,
    /// 削除直前の再評価で対象外になったので何もしなかった。
    Skipped,
}

/// 1 版ぶんの処理結果。
struct VersionStep {
    disposition: Disposition,
    nodes: i64,
    asset_rows: i64,
    /// この版が指していた crop の相対パス（**削除前に控えたもの**）。
    crop_paths: Vec<String>,
}

/// 1 版を回収する。**対象一覧を引いた後にもう一度述語を評価するのがこの関数の要点**で、
/// 一覧の取得から削除までの間に別経路（`build_lcir_for_attachment` は排他フラグを
/// 1 つも持たない）が新版を作って旧 completed を supersede すると、
/// alt text をまだ刈られていない版が対象に紛れる。
async fn collect_one_version(
    pool: &SqlitePool,
    version_id: i64,
    chunk: i64,
) -> Result<VersionStep, sqlx::Error> {
    if !still_collectable(pool, version_id).await? {
        eprintln!("LCIR GC: version {version_id} は削除直前の再評価で対象外になった（skip）");
        return Ok(VersionStep {
            disposition: Disposition::Skipped,
            nodes: 0,
            asset_rows: 0,
            crop_paths: Vec::new(),
        });
    }

    // **消す前にパスを控える**。`assets.document_version_id` は CASCADE なので
    // 削除した瞬間に relative_path が失われる。
    let crop_paths: Vec<String> =
        sqlx::query_scalar("SELECT relative_path FROM assets WHERE document_version_id = ?")
            .bind(version_id)
            .fetch_all(pool)
            .await?;

    let (nodes, asset_rows) = delete_version_children(pool, version_id, chunk).await?;

    let disposition = if is_carry_source(pool, version_id).await? {
        Disposition::Tombstoned
    } else {
        delete_version_row(pool, version_id).await?;
        Disposition::Removed
    };

    Ok(VersionStep {
        disposition,
        nodes,
        asset_rows,
        crop_paths,
    })
}

/// superseded 版の GC を実行する。`progress(done, total)` は版単位で呼ばれる。
///
/// **非可逆**。crop は `.attachment-trash` へ送るが、その trash に保持期間は無く
/// （`sweep_trash` は mtime を見ずに全消し）、次の削除操作かアプリ再起動で消える。
pub async fn run_gc<F>(
    pool: &SqlitePool,
    app_data_dir: &Path,
    progress: F,
) -> Result<GcOutcome, String>
where
    F: Fn(i64, i64),
{
    run_gc_with_chunk(pool, app_data_dir, NODE_DELETE_CHUNK, None, progress).await
}

/// [`run_gc`] の本体。
///
/// `chunk` はテストが小さい値を入れてチャンク境界を踏むための引数。
/// `targets` を `Some` にすると対象一覧の取得を差し替えられる ── **削除直前の再評価が
/// 実際に効くのは「一覧が古くなったとき」だけ**で、それは単一スレッドのテストでは
/// 作れない（対象述語は `still_collectable` の条件を包含するので、同一時点では
/// 両者が食い違わない）。古い一覧を注入して初めて `versions_skipped` の経路を通せる。
async fn run_gc_with_chunk<F>(
    pool: &SqlitePool,
    app_data_dir: &Path,
    chunk: i64,
    targets: Option<Vec<i64>>,
    progress: F,
) -> Result<GcOutcome, String>
where
    F: Fn(i64, i64),
{
    // **所要時間はこの索引に全面的に依存している**（無いと 125 秒・版あたり最大 16.7 秒。
    // 有ると 48 秒・最大 5.7 秒）。起動時にも張るが、そちらは spawn した best-effort で
    // 失敗しても eprintln だけなので、**GC 自身が入口で保証する**。
    // `CREATE INDEX IF NOT EXISTS` なので既にあれば数ミリ秒。
    crate::db::symbols::try_create_scope_node_index(pool)
        .await
        .map_err(|e| e.to_string())?;

    let before = storage_stats::db_size(pool)
        .await
        .map_err(|e| e.to_string())?;

    let targets = match targets {
        Some(t) => t,
        None => storage_stats::gc_target_versions(pool)
            .await
            .map_err(|e| e.to_string())?,
    };
    let total = targets.len() as i64;
    progress(0, total);

    let mut out = GcOutcome::default();
    // 削除した版が指していた crop のパス。FS の回収は全版の削除が終わってから
    // 「生存版が指していないもの」だけに絞る（同一ディレクトリを共有しうるため）。
    let mut dead_paths: Vec<String> = Vec::new();

    for (i, &vid) in targets.iter().enumerate() {
        let step = collect_one_version(pool, vid, chunk)
            .await
            .map_err(|e| e.to_string())?;
        dead_paths.extend(step.crop_paths);
        out.nodes_removed += step.nodes;
        out.asset_rows_removed += step.asset_rows;
        match step.disposition {
            Disposition::Removed => out.versions_removed += 1,
            Disposition::Tombstoned => out.versions_tombstoned += 1,
            Disposition::Skipped => out.versions_skipped += 1,
        }
        progress(i as i64 + 1, total);
    }

    out.fts_orphans_removed = sweep_fts_orphans(pool).await.map_err(|e| e.to_string())?;

    // FS: 生存版のどれも指していない crop だけを trash へ。
    if !dead_paths.is_empty() {
        let live: HashSet<String> = sqlx::query_scalar("SELECT relative_path FROM assets")
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        for rel in reclaimable_files(&dead_paths, &live) {
            let abs = app_data_dir.join(&rel);
            // **実在したものだけ数える。** `move_to_trash` は対象が無いとき何もせず `Ok(())` を
            // 返すので、素直に数えると「試行回数」になる。8a の build 時 GC が旧 content_key
            // ディレクトリを既に trash 済みで `assets` 行だけが provenance として残る、という
            // のは**正常な状態**（DATA_MODEL）なので、#7 の後はこれが多数派になる。
            // 「1 枚も当たらなかった」と「全部当たった」が同じ見た目だと異常に気づけない。
            if !abs.is_file() {
                out.files_already_gone += 1;
                continue;
            }
            match attachment_trash::move_to_trash(app_data_dir, &abs) {
                Ok(()) => out.files_trashed += 1,
                Err(e) => eprintln!("LCIR GC: crop の回収に失敗 {rel}: {e}"),
            }
        }
    }

    let after = storage_stats::db_size(pool)
        .await
        .map_err(|e| e.to_string())?;
    out.freed_bytes = (after.free_pages - before.free_pages) * after.page_size;
    out.db_size = Some(after);

    eprintln!(
        "LCIR GC: {} 版削除 / {} 版は行を残した / {} 版 skip / ノード {} / アセット行 {} / \
         ファイル {}（実体なし {}） / FTS 孤児 {} / 再利用可 +{} B",
        out.versions_removed,
        out.versions_tombstoned,
        out.versions_skipped,
        out.nodes_removed,
        out.asset_rows_removed,
        out.files_trashed,
        out.files_already_gone,
        out.fts_orphans_removed,
        out.freed_bytes,
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::assets::{insert_asset, insert_node_asset, NewAsset, NewNodeAsset};
    use crate::db::attachments::add_attachment;
    use crate::db::document_nodes::{insert_node, NewDocumentNode};
    use crate::db::document_versions::{insert_version, NewDocumentVersion};
    use crate::db::entries::create_entry;
    use crate::document_ir::{schema, ExtractionStatus, NodeKind, Origin};
    use crate::models::EntryInput;

    /// テスト DB で外部キーが実際に効いているか。
    ///
    /// **OFF だと以下の CASCADE の主張が全部空振りする**（子が孤児として残っても
    /// 版でスコープした SELECT が引かないので緑になる）。sqlx 0.8 は既定 ON だが、
    /// 既定が変わったら気づけるようここで固定する。
    #[sqlx::test(migrations = "./migrations")]
    async fn foreign_keys_are_on_in_the_test_database(pool: SqlitePool) {
        let on: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(on, 1, "FK が OFF だと GC のカスケード検証が空になる");
    }

    async fn setup_attachment(pool: &SqlitePool, title: &str) -> i64 {
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
            &format!("attachments/{}/p.pdf", entry.id),
            "p.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id
    }

    async fn insert_ver(
        pool: &SqlitePool,
        att: i64,
        ckey: &str,
        status: ExtractionStatus,
        parent: Option<i64>,
    ) -> i64 {
        insert_version(
            pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: ckey,
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: parent,
                status,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap()
    }

    /// `document > page > block` の 3 段（+ 追加の block）を作る。
    /// 親子があるので `parent_id` の自己参照 CASCADE がチャンク境界で効く。
    async fn insert_tree(pool: &SqlitePool, vid: i64, extra_blocks: usize) -> i64 {
        let root = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Document.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: None,
                origin: Some(Origin::PdfTextLayer.as_str()),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let page = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: Some(root),
                node_kind: NodeKind::Page.as_str(),
                ordinal: 0,
                plain_text: Some("page text"),
                language: None,
                confidence: None,
                origin: Some(Origin::PdfTextLayer.as_str()),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        for i in 0..extra_blocks {
            insert_node(
                pool,
                &NewDocumentNode {
                    document_version_id: vid,
                    parent_id: Some(page),
                    node_kind: NodeKind::Paragraph.as_str(),
                    ordinal: i as i64,
                    plain_text: Some("body"),
                    language: None,
                    confidence: None,
                    origin: Some(Origin::PdfTextLayer.as_str()),
                    payload_json: None,
                },
            )
            .await
            .unwrap();
        }
        page
    }

    async fn count(pool: &SqlitePool, sql: &str, vid: i64) -> i64 {
        sqlx::query_scalar(sql).bind(vid).fetch_one(pool).await.unwrap()
    }

    /// **素の COUNT で数える。** アクセサ経由（`document_nodes` を JOIN するもの）だと
    /// 孤児が残っていても JOIN が当たらず空を返し、CASCADE の主張が空振りする。
    async fn nodes_of(pool: &SqlitePool, vid: i64) -> i64 {
        count(
            pool,
            "SELECT COUNT(*) FROM document_nodes WHERE document_version_id = ?",
            vid,
        )
        .await
    }

    async fn version_exists(pool: &SqlitePool, vid: i64) -> bool {
        count(pool, "SELECT COUNT(*) FROM document_versions WHERE id = ?", vid).await == 1
    }

    fn noop(_: i64, _: i64) {}

    // ---- 対象の選び方 ----

    /// superseded の木は消え、生存版は 1 行も減らない。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_removes_superseded_subtrees_and_keeps_the_live_version(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, old, 3).await;
        let live = insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        insert_tree(&pool, live, 3).await;

        assert_eq!(nodes_of(&pool, old).await, 5, "前提: 旧版に木がある");
        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();

        assert_eq!(out.versions_removed, 1);
        assert_eq!(out.versions_tombstoned, 0);
        assert_eq!(out.versions_skipped, 0);
        assert_eq!(out.nodes_removed, 5);
        assert_eq!(nodes_of(&pool, old).await, 0);
        assert!(!version_exists(&pool, old).await, "版の行ごと消える");
        assert_eq!(nodes_of(&pool, live).await, 5, "生存版は無傷");
        assert!(version_exists(&pool, live).await);
    }

    /// 孫（`source_fragments` / `math_expressions` / `node_assets`）も一緒に消え、
    /// 版直下の `node_relations` / `symbols` / `assets` も消える。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_removes_every_cascading_table(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let page = insert_tree(&pool, old, 1).await;
        insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;

        crate::db::source_fragments::insert_fragment(
            &pool,
            &crate::db::source_fragments::NewSourceFragment {
                node_id: page,
                page_number: 1,
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rotation: 0.0,
                reading_order: Some(0),
                fragment_type: Some("page"),
            },
        )
        .await
        .unwrap();
        crate::db::math_expressions::insert_math(
            &pool,
            &crate::db::math_expressions::NewMathExpression {
                node_id: page,
                display_mode: "display",
                equation_label: None,
                latex: None,
                presentation_mathml: None,
                content_mathml: None,
                openmath_json: None,
                normalized_text: Some("x=1"),
                ast_json: None,
                semantic_status: "surface_only",
                confidence: None,
                origin: None,
            },
        )
        .await
        .unwrap();
        crate::db::node_relations::insert_relation(
            &pool,
            &crate::db::node_relations::NewNodeRelation {
                document_version_id: old,
                from_node_id: page,
                relation_type: "refers_to",
                to_node_id: page,
                confidence: None,
                origin: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let sym = crate::db::symbols::insert_symbol(
            &pool,
            &crate::db::symbols::NewSymbol {
                document_version_id: old,
                surface_form: "U",
                normalized_form: None,
                description: None,
                symbol_type: None,
                defined_at_node_id: Some(page),
                scope_node_id: Some(page),
                semantic_json: None,
                confidence: None,
                origin: None,
            },
        )
        .await
        .unwrap();
        crate::db::symbols::insert_occurrence(
            &pool,
            &crate::db::symbols::NewSymbolOccurrence {
                symbol_id: sym,
                node_id: page,
                local_offset_json: None,
                surface_form: "U",
                confidence: None,
                origin: None,
            },
        )
        .await
        .unwrap();
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: old,
                sha256: "s",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/aaaa/fig-p001-00.png",
                width: Some(1),
                height: Some(1),
                size_bytes: Some(10),
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        insert_node_asset(&pool, &NewNodeAsset { node_id: page, asset_id: aid }, "page_crop")
            .await
            .unwrap();

        let dir = std::env::temp_dir();
        run_gc(&pool, &dir, noop).await.unwrap();

        for (table, sql) in [
            ("document_nodes", "SELECT COUNT(*) FROM document_nodes"),
            ("source_fragments", "SELECT COUNT(*) FROM source_fragments"),
            ("math_expressions", "SELECT COUNT(*) FROM math_expressions"),
            ("node_relations", "SELECT COUNT(*) FROM node_relations"),
            ("symbols", "SELECT COUNT(*) FROM symbols"),
            ("symbol_occurrences", "SELECT COUNT(*) FROM symbol_occurrences"),
            ("assets", "SELECT COUNT(*) FROM assets"),
            ("node_assets", "SELECT COUNT(*) FROM node_assets"),
        ] {
            let n: i64 = sqlx::query_scalar(sql).fetch_one(&pool).await.unwrap();
            assert_eq!(n, 0, "{table} に行が残っている");
        }
    }

    /// `parent_version_id` は `NO ACTION` なので、pre-step で NULL 化しないと
    /// FK エラーで 1 件も消えない（実 DB の superseded 145/145 がこの形）。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_nulls_the_parent_reference_before_deleting(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, old, 1).await;
        let live = insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;

        // 前提: pre-step 抜きの素朴な DELETE は落ちる。
        assert!(
            sqlx::query("DELETE FROM document_versions WHERE id = ?")
                .bind(old)
                .execute(&pool)
                .await
                .is_err(),
            "pre-step 無しでは FK エラーになるはず（このテストの前提）"
        );

        let dir = std::env::temp_dir();
        run_gc(&pool, &dir, noop).await.unwrap();

        assert!(!version_exists(&pool, old).await);
        let parent: Option<i64> =
            sqlx::query_scalar("SELECT parent_version_id FROM document_versions WHERE id = ?")
                .bind(live)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(parent, None, "生存版の親参照は NULL 化される");
    }

    /// **削除対象の版どうしが親子でも消せる**（先に消した行を後の行が指している形）。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_removes_a_chain_of_superseded_versions(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let v1 = insert_ver(&pool, att, "ck1", ExtractionStatus::Superseded, None).await;
        let v2 = insert_ver(&pool, att, "ck2", ExtractionStatus::Superseded, Some(v1)).await;
        let v3 = insert_ver(&pool, att, "ck3", ExtractionStatus::Superseded, Some(v2)).await;
        for v in [v1, v2, v3] {
            insert_tree(&pool, v, 1).await;
        }
        insert_ver(&pool, att, "ck4", ExtractionStatus::Completed, Some(v3)).await;

        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.versions_removed, 3);
        for v in [v1, v2, v3] {
            assert!(!version_exists(&pool, v).await);
        }
    }

    // ---- 安全述語 ----

    /// (i) alt text を持つ superseded 版は**対象に入らない**。
    ///
    /// 2 段構え: まず「alt text が無ければ対象になる」ことを確かめてから条件を足す。
    /// でないと「前段の早期 return で本体に届いていないだけ」を検出できない。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_never_deletes_a_version_that_still_holds_alt_texts(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let page = insert_tree(&pool, old, 1).await;
        insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;

        // 前段: この時点では対象に入る。
        assert_eq!(
            storage_stats::gc_target_versions(&pool).await.unwrap(),
            vec![old],
            "alt text が無ければ対象になる（後段の 0 件が空振りでない証拠）"
        );

        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: page,
                document_version_id: old,
                source_asset_sha256: "sha",
                text: "課金済みの説明",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: Some("claude-sonnet-5"),
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        assert!(
            storage_stats::gc_target_versions(&pool).await.unwrap().is_empty(),
            "alt text を抱えた版は対象から外れる"
        );
        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.versions_removed, 0);
        assert_eq!(out.nodes_removed, 0);
        assert!(version_exists(&pool, old).await);
        assert_eq!(nodes_of(&pool, old).await, 3, "木も無傷");
        let alt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_alt_texts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(alt, 1, "課金済みの説明が残る");
    }

    /// (i) は **ノード経由でも**守る。
    ///
    /// `node_alt_texts` は `node_id` と `document_version_id` を独立に持ち、
    /// **両者が一致することをスキーマは強制していない**。version 列だけを見ると
    /// 「この版に alt text は無い」と判定するのに、ノードを消したカスケードで
    /// 実際には消える、という取り違えが起きる。今のデータでは no-op だが
    /// 失うと復旧不能なので両方見る。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_protects_an_alt_text_reachable_only_through_its_node(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let page = insert_tree(&pool, old, 1).await;
        let live = insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;

        assert_eq!(
            storage_stats::gc_target_versions(&pool).await.unwrap(),
            vec![old],
            "前提: alt text が無ければ対象"
        );

        // ノードは旧版のものだが、version 列は生存版を指している不整合な行。
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: page,
                document_version_id: live,
                source_asset_sha256: "sha",
                text: "課金済み",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        assert!(
            storage_stats::gc_target_versions(&pool).await.unwrap().is_empty(),
            "version 列は別を指していても、ノードがこの版にあるなら守る"
        );
        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.versions_removed, 0);
        assert_eq!(out.versions_skipped, 0, "対象一覧の時点で外れる");
        let alt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_alt_texts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(alt, 1, "課金済みの説明が残る");
    }

    /// (i) は `user_edited`（手編集）でも同じく守る。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_never_deletes_a_version_holding_a_hand_written_alt_text(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let page = insert_tree(&pool, old, 1).await;
        insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: page,
                document_version_id: old,
                source_asset_sha256: "sha",
                text: "人が書いた説明",
                origin: Origin::UserEdited.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        assert!(
            storage_stats::gc_target_versions(&pool).await.unwrap().is_empty(),
            "そもそも対象一覧に入らない（再評価で拾うのではなく）"
        );
        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.versions_removed, 0);
        assert_eq!(out.versions_skipped, 0);
        assert!(version_exists(&pool, old).await);
    }

    /// (ii) 生存 alt text の carry 元は**子だけ消して行を残す**。
    ///
    /// 消すと `carried_from_version_id` が `SET NULL` になり、「NULL = この版で生成」という
    /// スキーマの契約を carry 行が偽る。#7 の再構築後は最初の生成版がすべてこれになる。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_tombstones_a_carry_source_but_still_reclaims_its_rows(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let origin_ver = insert_ver(&pool, att, "ck1", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, origin_ver, 3).await;
        let live = insert_ver(&pool, att, "ck2", ExtractionStatus::Completed, Some(origin_ver)).await;
        let live_page = insert_tree(&pool, live, 1).await;
        // 生存版の alt text が origin_ver 由来だと記録している。
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: live_page,
                document_version_id: live,
                source_asset_sha256: "sha",
                text: "carry された説明",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: Some("claude-sonnet-5"),
                carried_from_version_id: Some(origin_ver),
            },
        )
        .await
        .unwrap();

        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();

        assert_eq!(out.versions_tombstoned, 1);
        assert_eq!(out.versions_removed, 0);
        assert_eq!(out.nodes_removed, 5, "子は回収する（容量の 83% はここ）");
        assert_eq!(nodes_of(&pool, origin_ver).await, 0);
        assert!(version_exists(&pool, origin_ver).await, "行は残す");
        let carried: Option<i64> = sqlx::query_scalar(
            "SELECT carried_from_version_id FROM node_alt_texts WHERE document_version_id = ?",
        )
        .bind(live)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(carried, Some(origin_ver), "由来が NULL に化けていない");
    }

    /// (iii) 生存版が 1 本も無い添付の superseded は触らない。
    /// 消すと LCIR が丸ごと消え、一括構築バッチが再抽出対象に戻す（att37 なら 75 分）。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_leaves_an_attachment_that_has_no_live_version(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let only = insert_ver(&pool, att, "ck", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, only, 1).await;

        // **対象一覧に入らないこと自体を主張する。** 削除直前の再評価も同じ条件を見るので、
        // 「結果として消えていない」だけだと述語 (iii) が効いているのか
        // 再評価が拾っているのか区別できない（変異 S2 がここで生き残った）。
        assert!(
            storage_stats::gc_target_versions(&pool).await.unwrap().is_empty(),
            "そもそも対象一覧に入らない"
        );

        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.versions_removed, 0);
        assert_eq!(out.versions_skipped, 0, "再評価で拾うのではなく最初から対象外");
        assert!(version_exists(&pool, only).await);
        assert_eq!(nodes_of(&pool, only).await, 3);
        assert_eq!(
            storage_stats::gc_preview(&pool).await.unwrap().orphan_versions_skipped,
            1
        );
    }

    /// 別の添付の superseded は消えるが、この添付の生存版は 1 行も減らない。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_is_scoped_per_attachment(pool: SqlitePool) {
        let a1 = setup_attachment(&pool, "P1").await;
        let old1 = insert_ver(&pool, a1, "ck1", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, old1, 1).await;
        insert_ver(&pool, a1, "ck2", ExtractionStatus::Completed, Some(old1)).await;

        let a2 = setup_attachment(&pool, "P2").await;
        let live2 = insert_ver(&pool, a2, "ck3", ExtractionStatus::CompletedWithWarnings, None).await;
        insert_tree(&pool, live2, 4).await;

        let dir = std::env::temp_dir();
        run_gc(&pool, &dir, noop).await.unwrap();
        assert!(!version_exists(&pool, old1).await);
        assert_eq!(nodes_of(&pool, live2).await, 6, "別添付の生存版は無傷");
    }

    // ---- 収束・チャンク ----

    /// 2 回目は何もしない（対象述語の (iv) が無いと、行を残した版を毎回数え続ける）。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_converges_when_run_twice(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let origin_ver = insert_ver(&pool, att, "ck1", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, origin_ver, 2).await;
        let live = insert_ver(&pool, att, "ck2", ExtractionStatus::Completed, Some(origin_ver)).await;
        let live_page = insert_tree(&pool, live, 1).await;
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: live_page,
                document_version_id: live,
                source_asset_sha256: "sha",
                text: "t",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: Some(origin_ver),
            },
        )
        .await
        .unwrap();

        let dir = std::env::temp_dir();
        let first = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(first.versions_tombstoned, 1);

        let second = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(second.versions_tombstoned, 0, "行を残した版を数え続けない");
        assert_eq!(second.versions_removed, 0);
        assert_eq!(second.nodes_removed, 0);
        assert!(
            storage_stats::gc_target_versions(&pool).await.unwrap().is_empty(),
            "2 回目には対象が空になる"
        );
    }

    /// チャンク境界をまたいでも全部消える。
    ///
    /// `parent_id` の自己参照 CASCADE があるので、`LIMIT` で切った集合の外の子も
    /// 一緒に消える。**残数で終了を判定している**ことをここで固定する
    /// （`rows_affected` はカスケードを数えないので、それを終了条件にすると
    /// 数が合わず無限ループか早期終了になる）。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_deletes_everything_even_when_the_chunk_is_smaller_than_the_tree(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, old, 20).await; // document + page + 20 = 22 ノード
        insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        assert_eq!(nodes_of(&pool, old).await, 22);

        let dir = std::env::temp_dir();
        let out = run_gc_with_chunk(&pool, &dir, 3, None, noop).await.unwrap();
        assert_eq!(out.nodes_removed, 22, "報告値は削除前の実数");
        assert_eq!(nodes_of(&pool, old).await, 0);
        assert!(!version_exists(&pool, old).await);
    }

    /// **チャンクの中に親と子が同居しても取りこぼさない。**
    ///
    /// `DELETE ... WHERE id IN (SELECT ... LIMIT n)` は副問い合わせを先に評価するので、
    /// 選ばれた集合の中に親子が居ると、親の削除が子を CASCADE で消し、
    /// **`rows_affected` は `n` より小さくなる**（カスケードは数えられない）。
    /// ここで「`rows_affected < chunk` なら終わり」と判定すると、まだ残っている
    /// ノードを置き去りにしたままループを抜ける。だから**残数で判定している**。
    ///
    /// 罠が 2 つある。
    /// ①**木の根から始まる素直な形だと 1 回のカスケードで全部消えてしまい**、
    ///   この違いが現れない。だから**平坦なノードの後ろに親子の組を置いて**
    ///   チャンク境界に親子を跨がせる。
    /// ②**行ごと消す版だと、最後の版削除がカスケードで取りこぼしを片付けてしまう**。
    ///   だから carry 元にして**行を残す版（tombstone）**で試す。
    ///   ①だけ・②だけでは変異が生き残る（実際に 2 回生き残らせた）。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_does_not_stop_early_when_a_chunk_cascades_into_itself(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let live = insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        let live_page = insert_tree(&pool, live, 0).await;
        // old を carry 元にして行を残させる（= 取りこぼしが後始末で隠れない）。
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: live_page,
                document_version_id: live,
                source_asset_sha256: "sha",
                text: "carry",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: Some(old),
            },
        )
        .await
        .unwrap();

        let flat = |i: i64| NewDocumentNode {
            document_version_id: old,
            parent_id: None,
            node_kind: NodeKind::Paragraph.as_str(),
            ordinal: i,
            plain_text: Some("x"),
            language: None,
            confidence: None,
            origin: Some(Origin::PdfTextLayer.as_str()),
            payload_json: None,
        };
        // id 昇順: 平坦 3 個 → 平坦 1 個 + 親 + 子 2 個 → 平坦 2 個。
        for i in 0..4 {
            insert_node(&pool, &flat(i)).await.unwrap();
        }
        let parent = insert_node(&pool, &flat(4)).await.unwrap();
        for i in 0..2 {
            insert_node(
                &pool,
                &NewDocumentNode {
                    parent_id: Some(parent),
                    ordinal: i,
                    ..flat(i)
                },
            )
            .await
            .unwrap();
        }
        for i in 5..7 {
            insert_node(&pool, &flat(i)).await.unwrap();
        }
        assert_eq!(nodes_of(&pool, old).await, 9);

        let dir = std::env::temp_dir();
        let out = run_gc_with_chunk(&pool, &dir, 3, None, noop).await.unwrap();
        assert_eq!(out.versions_tombstoned, 1, "前提: 行を残す版として処理される");
        assert_eq!(out.nodes_removed, 9, "報告値は実際に減った数");
        assert_eq!(nodes_of(&pool, old).await, 0, "1 ノードも置き去りにしない");
        assert!(version_exists(&pool, old).await, "行は残る");
    }

    /// **チャンクが本当に tx を分割している**こと。
    ///
    /// これが `NODE_DELETE_CHUNK` の存在理由（1 tx を `busy_timeout` 5 秒より十分短く保つ）で、
    /// 「全部消えたか」を見るテストでは**absolutely 検出できない** ── 昇順で切ると
    /// 1 チャンク目に木の根が入り、その 1 行の削除が版の全ノードをカスケードで消すので、
    /// 結果だけ見れば正しく全消えするからである（実 DB の 272,583 ノードの版が
    /// `changes()` = 1 で残り 0 になることを実測した）。
    /// だから**「1 回の DELETE で消える件数が chunk を超えないこと」を直接測る**。
    #[sqlx::test(migrations = "./migrations")]
    async fn one_delete_chunk_never_removes_more_than_the_chunk_size(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let vid = insert_ver(&pool, att, "ck", ExtractionStatus::Superseded, None).await;
        // 本番と同じ形（根 → page → block）で 1 + 1 + 20 = 22 ノード。
        insert_tree(&pool, vid, 20).await;

        let before: i64 = nodes_of(&pool, vid).await;
        assert_eq!(before, 22);

        // 本番と同一の DELETE を 1 回だけ打つ。
        sqlx::query(
            "DELETE FROM document_nodes
              WHERE id IN (
                  SELECT id FROM document_nodes
                   WHERE document_version_id = ?
                   ORDER BY id DESC
                   LIMIT ?
              )",
        )
        .bind(vid)
        .bind(5i64)
        .execute(&pool)
        .await
        .unwrap();

        let removed = before - nodes_of(&pool, vid).await;
        assert_eq!(
            removed, 5,
            "1 チャンクで消えるのはちょうど chunk 件（昇順だと根のカスケードで 22 件全部消える）"
        );
    }

    /// 進捗は版単位で 0 から total まで流れる。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_reports_progress_per_version(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let v1 = insert_ver(&pool, att, "ck1", ExtractionStatus::Superseded, None).await;
        let v2 = insert_ver(&pool, att, "ck2", ExtractionStatus::Superseded, Some(v1)).await;
        insert_ver(&pool, att, "ck3", ExtractionStatus::Completed, Some(v2)).await;

        let seen = std::sync::Mutex::new(Vec::new());
        let dir = std::env::temp_dir();
        run_gc(&pool, &dir, |d, t| seen.lock().unwrap().push((d, t)))
            .await
            .unwrap();
        assert_eq!(seen.into_inner().unwrap(), vec![(0, 2), (1, 2), (2, 2)]);
    }

    /// **tombstone 経路でも版直下の行（`assets` / `symbols`）を回収する。**
    ///
    /// 行ごと消す版では最後の版削除がカスケードで片付けてしまうので、
    /// `delete_version_children` の明示 DELETE を丸ごと消しても全テストが緑になる
    /// （実際に変異させて確認した）。**その明示 DELETE が load-bearing なのは
    /// tombstone 経路だけ**なので、ここで固定する。
    ///
    /// `node_relations` はここに書かない ── `from_node_id` / `to_node_id` が NOT NULL で
    /// ノードを参照するので、ノード削除のカスケードで必ず先に消える。
    /// 残りうるのは **`assets`（ノードを参照しない）**と
    /// **`defined_at_node_id` が NULL の `symbols`** の 2 つ。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_tombstone_also_reclaims_the_version_scoped_rows(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck1", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, old, 1).await;
        let live = insert_ver(&pool, att, "ck2", ExtractionStatus::Completed, Some(old)).await;
        let live_page = insert_tree(&pool, live, 0).await;
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: live_page,
                document_version_id: live,
                source_asset_sha256: "sha",
                text: "carry",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: Some(old),
            },
        )
        .await
        .unwrap();

        // ノードを参照しない版直下の行を 2 種類置く。
        insert_asset(
            &pool,
            &NewAsset {
                document_version_id: old,
                sha256: "s",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/aaaa/fig-p001-00.png",
                width: Some(1),
                height: Some(1),
                size_bytes: Some(10),
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        crate::db::symbols::insert_symbol(
            &pool,
            &crate::db::symbols::NewSymbol {
                document_version_id: old,
                surface_form: "U",
                normalized_form: None,
                description: None,
                symbol_type: None,
                defined_at_node_id: None, // ノードに紐づかない = カスケードで消えない
                scope_node_id: None,
                semantic_json: None,
                confidence: None,
                origin: None,
            },
        )
        .await
        .unwrap();

        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.versions_tombstoned, 1, "前提: 行を残す経路を通る");
        assert!(version_exists(&pool, old).await);
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM assets WHERE document_version_id = ?", old).await,
            0,
            "版直下の assets が残っている"
        );
        assert_eq!(
            count(&pool, "SELECT COUNT(*) FROM symbols WHERE document_version_id = ?", old).await,
            0,
            "ノードに紐づかない symbols が残っている"
        );
        assert_eq!(out.asset_rows_removed, 1);
    }

    /// 対象一覧が古くなっていたら（別経路が alt text を挿した後）、
    /// **`run_gc` の集計で `versions_skipped` に出る**。
    #[sqlx::test(migrations = "./migrations")]
    async fn run_gc_reports_a_stale_target_as_skipped(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let page = insert_tree(&pool, old, 2).await;
        insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        // 一覧を引いた「後」に alt text が挿さった状況を、古い一覧の注入で再現する。
        let stale = storage_stats::gc_target_versions(&pool).await.unwrap();
        assert_eq!(stale, vec![old], "前提: この時点では対象");
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: page,
                document_version_id: old,
                source_asset_sha256: "sha",
                text: "課金済み",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        let dir = std::env::temp_dir();
        let out = run_gc_with_chunk(&pool, &dir, NODE_DELETE_CHUNK, Some(stale), noop)
            .await
            .unwrap();
        assert_eq!(out.versions_skipped, 1);
        assert_eq!(out.versions_removed, 0);
        assert_eq!(out.nodes_removed, 0);
        assert_eq!(nodes_of(&pool, old).await, 4, "1 行も消えていない");
        assert!(version_exists(&pool, old).await);
        assert!(out.db_size.is_some(), "実行後のファイル収支を返している");
    }

    /// チャンクの大きさを**リテラルで**固定する。
    /// 定数シンボルで書いたテストは定数を変える変異と一緒に動くので守っていない。
    #[test]
    fn the_node_delete_chunk_is_fifty_thousand() {
        assert_eq!(
            NODE_DELETE_CHUNK, 50_000,
            "1 tx を busy_timeout(5 秒)より十分短く保つ値。実測で 1 チャンク 0.28〜0.63 秒"
        );
    }

    // ---- FS 側 ----

    /// crop は trash へ送られる。**生存版も指しているファイルは残す**
    /// （同一 content_key ディレクトリを共有しうるため、パスの集合差で判断する）。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_trashes_only_the_crops_no_live_version_points_at(pool: SqlitePool) {
        let dir = std::env::temp_dir().join(format!("lc-gc-crop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = dir.as_path();
        let shared_rel = "attachments/1/.lcir/1/aaaa/fig-p001-00.png";
        let dead_rel = "attachments/1/.lcir/1/aaaa/fig-p002-00.png";
        for rel in [shared_rel, dead_rel] {
            let abs = app.join(rel);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(&abs, b"png").unwrap();
        }

        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let old_page = insert_tree(&pool, old, 0).await;
        let live = insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        insert_tree(&pool, live, 0).await;

        for (vid, node, rel) in [(old, Some(old_page), shared_rel), (old, None, dead_rel)] {
            let aid = insert_asset(
                &pool,
                &NewAsset {
                    document_version_id: vid,
                    sha256: "s",
                    mime_type: "image/png",
                    relative_path: rel,
                    width: Some(1),
                    height: Some(1),
                    size_bytes: Some(3),
                    metadata_json: None,
                },
            )
            .await
            .unwrap();
            if let Some(n) = node {
                insert_node_asset(&pool, &NewNodeAsset { node_id: n, asset_id: aid }, "page_crop")
                    .await
                    .unwrap();
            }
        }
        // 生存版も同じファイルを指している（ディレクトリ共有の再現）。
        insert_asset(
            &pool,
            &NewAsset {
                document_version_id: live,
                sha256: "s",
                mime_type: "image/png",
                relative_path: shared_rel,
                width: Some(1),
                height: Some(1),
                size_bytes: Some(3),
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        // **実体が既に無い crop** も 1 枚混ぜる（8a の build 時 GC が先に回収した正常ケース）。
        // これを `files_trashed` に数えてしまうと「1 枚も当たらなかった」異常が
        // 正常と同じ見た目になる。
        let gone_rel = "attachments/1/.lcir/1/aaaa/fig-p003-00.png";
        insert_asset(
            &pool,
            &NewAsset {
                document_version_id: old,
                sha256: "s",
                mime_type: "image/png",
                relative_path: gone_rel,
                width: Some(1),
                height: Some(1),
                size_bytes: Some(3),
                metadata_json: None,
            },
        )
        .await
        .unwrap();

        let out = run_gc(&pool, app, noop).await.unwrap();
        assert_eq!(out.files_trashed, 1, "実際に動かしたのは 1 枚だけ");
        assert_eq!(out.files_already_gone, 1, "実体の無かった 1 枚は別に数える");
        assert!(app.join(shared_rel).is_file(), "生存版が指す crop は消さない");
        assert!(!app.join(dead_rel).exists(), "誰も指さない crop は trash へ");
    }

    // ---- node-FTS ----

    /// GC が孤児 FTS 行を掃除し、生存版の索引は 1 行も減らさない。
    #[sqlx::test(migrations = "./migrations")]
    async fn gc_sweeps_orphan_fts_rows_and_keeps_live_ones(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let old_page = insert_tree(&pool, old, 0).await;
        let live = insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        let live_page = insert_tree(&pool, live, 0).await;

        crate::db::document_nodes_fts::index_nodes(
            &pool,
            att,
            &[
                crate::db::document_nodes_fts::NodeFtsInput {
                    node_id: old_page,
                    page: 1,
                    node_kind: "page".to_string(),
                    content: "superseded page".to_string(),
                },
                crate::db::document_nodes_fts::NodeFtsInput {
                    node_id: live_page,
                    page: 1,
                    node_kind: "page".to_string(),
                    content: "live page".to_string(),
                },
            ],
        )
        .await
        .unwrap();

        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.fts_orphans_removed, 1);
        let left: Vec<i64> = sqlx::query_scalar("SELECT node_id FROM document_nodes_fts")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(left, vec![live_page], "生存版の索引だけが残る");
    }

    /// **確認ダイアログに出す見積りが実行結果と一致する。**
    ///
    /// 押す前に見せるのは行数と安全述語だけ（バイトは GC 後に `freelist` の実測差分で出す）。
    /// その行数が実際と食い違うと、ユーザーは非可逆な操作を誤った前提で承認することになる。
    #[sqlx::test(migrations = "./migrations")]
    async fn the_preview_matches_what_the_gc_actually_does(pool: SqlitePool) {
        // 消える版 2 本・行を残す版 1 本・守られる版 1 本を混ぜる。
        let a1 = setup_attachment(&pool, "P1").await;
        let d1 = insert_ver(&pool, a1, "ck1", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, d1, 2).await;
        let d2 = insert_ver(&pool, a1, "ck2", ExtractionStatus::Superseded, Some(d1)).await;
        insert_tree(&pool, d2, 1).await;
        let live1 = insert_ver(&pool, a1, "ck3", ExtractionStatus::Completed, Some(d2)).await;
        let live1_page = insert_tree(&pool, live1, 0).await;

        let a2 = setup_attachment(&pool, "P2").await;
        let carry_src = insert_ver(&pool, a2, "ck4", ExtractionStatus::Superseded, None).await;
        insert_tree(&pool, carry_src, 4).await;
        let live2 = insert_ver(&pool, a2, "ck5", ExtractionStatus::Completed, Some(carry_src)).await;
        let live2_page = insert_tree(&pool, live2, 0).await;
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: live2_page,
                document_version_id: live2,
                source_asset_sha256: "sha-carried",
                text: "carry された説明",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: Some(carry_src),
            },
        )
        .await
        .unwrap();

        // 自前の alt text を抱えた版（対象外）。
        let a3 = setup_attachment(&pool, "P3").await;
        let protected = insert_ver(&pool, a3, "ck6", ExtractionStatus::Superseded, None).await;
        let protected_page = insert_tree(&pool, protected, 9).await;
        insert_ver(&pool, a3, "ck7", ExtractionStatus::Completed, Some(protected)).await;
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: protected_page,
                document_version_id: protected,
                source_asset_sha256: "sha-own",
                text: "課金済み",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();
        let _ = live1_page;

        let preview = storage_stats::gc_preview(&pool).await.unwrap();
        assert_eq!(preview.versions, 3, "d1 / d2 / carry_src");
        assert_eq!(preview.versions_removable, 2);
        assert_eq!(preview.versions_tombstoned, 1);
        assert_eq!(preview.carry_refs_protected, 1);
        assert_eq!(preview.nodes, 4 + 3 + 6);
        assert_eq!(preview.alt_texts_protected, 1, "対象外の版が抱える alt text");
        assert_eq!(preview.orphan_versions_skipped, 0);

        let dir = std::env::temp_dir();
        let out = run_gc(&pool, &dir, noop).await.unwrap();
        assert_eq!(out.versions_removed, preview.versions_removable);
        assert_eq!(out.versions_tombstoned, preview.versions_tombstoned);
        assert_eq!(out.nodes_removed, preview.nodes, "見積りと実削除が一致する");
        assert_eq!(out.versions_skipped, 0);
        assert_eq!(nodes_of(&pool, protected).await, 11, "守られた版は無傷");
    }

    // ---- 実 DB のコピーに本番のコードをそのまま流すプローブ ----

    /// 実ライブラリのコピーで GC を完走させ、件数・所要時間・回収量を測る。
    ///
    /// **本番の `run_gc` をそのまま呼ぶ**（第 2 の実装を作らない）。手書きの SQL で
    /// 測ると、述語もチャンクの切り方も本番と違うものを測ることになる。
    ///
    /// ```sh
    /// cp ~/Library/Application\ Support/com.lumencite.app/lumencite.db "$TMPDIR/gc-probe.db"
    /// cd src-tauri && LCIR_GC_DB="$TMPDIR/gc-probe.db" \
    ///   cargo test --lib lcir_gc_on_a_copy_of_the_real_library -- --ignored --nocapture
    /// rm -f "$TMPDIR"/gc-probe.db*
    /// ```
    ///
    /// **コピーは `$TMPDIR` に置くこと。** スクラッチパッド配下は `sqlite3` からは開けるのに
    /// `cargo test` のプロセスからは `code 14 unable to open database file` になり、
    /// Dropbox 配下は 761MB が同期対象になる。
    #[tokio::test]
    #[ignore = "manual probe against a copy of the real library; needs LCIR_GC_DB"]
    async fn lcir_gc_on_a_copy_of_the_real_library() {
        let Ok(db) = std::env::var("LCIR_GC_DB") else {
            eprintln!("skip: set LCIR_GC_DB=<実 DB のコピー>");
            return;
        };
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&db)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(30));
        let pool = SqlitePool::connect_with(opts).await.unwrap();

        // 1 行目は libtest に食われるので捨て行を置く。
        eprintln!("GC_PROBE_BEGIN");

        let before = storage_stats::db_size(&pool).await.unwrap();
        let preview = storage_stats::gc_preview(&pool).await.unwrap();
        eprintln!(
            "GC_BEFORE file={} used={} free={} versions={} removable={} tombstoned={} \
             nodes={} asset_rows={} asset_bytes={} alt_protected={} orphan_skipped={}",
            before.file_bytes(),
            before.used_bytes(),
            before.free_bytes(),
            preview.versions,
            preview.versions_removable,
            preview.versions_tombstoned,
            preview.nodes,
            preview.asset_rows,
            preview.asset_bytes,
            preview.alt_texts_protected,
            preview.orphan_versions_skipped,
        );

        // 索引の有無で所要時間が 2.6 倍変わるので、張ったかどうかを明示して測る。
        if std::env::var("LCIR_GC_NO_INDEX").is_err() {
            crate::db::symbols::try_create_scope_node_index(&pool)
                .await
                .unwrap();
            eprintln!("GC_INDEX symbols_scope_node=created");
        } else {
            eprintln!("GC_INDEX symbols_scope_node=absent");
        }

        let started = std::time::Instant::now();
        let slowest = std::sync::Mutex::new((0i64, 0f64));
        let last = std::sync::Mutex::new(std::time::Instant::now());
        let app_dir = std::env::temp_dir().join("lcir-gc-probe-appdir");
        let out = run_gc(&pool, &app_dir, |done, _total| {
            let mut l = last.lock().unwrap();
            let dt = l.elapsed().as_secs_f64();
            *l = std::time::Instant::now();
            let mut s = slowest.lock().unwrap();
            if dt > s.1 {
                *s = (done, dt);
            }
        })
        .await
        .unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        let (slow_at, slow_secs) = *slowest.lock().unwrap();

        let after = storage_stats::db_size(&pool).await.unwrap();
        let live_nodes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM document_nodes")
            .fetch_one(&pool)
            .await
            .unwrap();
        let live_frags: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM source_fragments")
            .fetch_one(&pool)
            .await
            .unwrap();
        let alt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_alt_texts")
            .fetch_one(&pool)
            .await
            .unwrap();
        let sup: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM document_versions WHERE extraction_status = 'superseded'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        eprintln!(
            "GC_AFTER elapsed_s={elapsed:.2} slowest_version_s={slow_secs:.2} at={slow_at} \
             removed={} tombstoned={} skipped={} nodes={} asset_rows={} files={} fts_orphans={} \
             freed={} file={} used={} free={} live_nodes={live_nodes} live_frags={live_frags} \
             alt_texts={alt} superseded_left={sup}",
            out.versions_removed,
            out.versions_tombstoned,
            out.versions_skipped,
            out.nodes_removed,
            out.asset_rows_removed,
            out.files_trashed,
            out.fts_orphans_removed,
            out.freed_bytes,
            after.file_bytes(),
            after.used_bytes(),
            after.free_bytes(),
        );

        // **外部で分かっている不変量を集計に入れる**（生存確認のセンチネル）。
        assert_eq!(
            out.nodes_removed, preview.nodes,
            "見積りと実削除が一致する（実データでも）"
        );
        assert_eq!(alt, 888, "alt text は 1 行も減らない（実 DB の既知の値）");
        // **GC ではファイルは縮まない**（free page になるだけ）。増える方向には動きうる ──
        // このプローブは `symbols(scope_node_id)` の索引を張るので、その 1 ページぶん。
        assert!(
            out.freed_bytes > 400_000_000,
            "実ライブラリでは 471 MiB 前後が再利用可になる（実測 493,899,776 B）。got {}",
            out.freed_bytes
        );
        assert_eq!(
            out.versions_removed + out.versions_tombstoned + out.versions_skipped,
            preview.versions,
            "対象件数の内訳が見積りと一致する"
        );
        assert!(
            after.file_bytes() >= before.file_bytes(),
            "GC でファイルが縮んではいけない（before={} after={}）",
            before.file_bytes(),
            after.file_bytes()
        );
    }

    // ---- 純関数 ----

    #[test]
    fn reclaimable_files_keeps_paths_a_live_version_still_points_at() {
        let dead = vec![
            "a/.lcir/1/k/fig-p001-00.png".to_string(),
            "a/.lcir/1/k/fig-p002-00.png".to_string(),
            "a/.lcir/1/k/fig-p001-00.png".to_string(), // 同じパスを 2 版が指していた
        ];
        let live: HashSet<String> = ["a/.lcir/1/k/fig-p001-00.png".to_string()].into();
        assert_eq!(
            reclaimable_files(&dead, &live),
            vec!["a/.lcir/1/k/fig-p002-00.png".to_string()]
        );
    }

    #[test]
    fn reclaimable_files_returns_nothing_when_every_path_is_still_live() {
        let dead = vec!["x.png".to_string()];
        let live: HashSet<String> = ["x.png".to_string()].into();
        assert!(reclaimable_files(&dead, &live).is_empty());
    }

    #[test]
    fn reclaimable_files_deduplicates() {
        let dead = vec!["x.png".to_string(), "x.png".to_string()];
        assert_eq!(reclaimable_files(&dead, &HashSet::new()), vec!["x.png".to_string()]);
    }

    // ---- 削除直前の再評価 ----

    /// **削除の実行経路そのものが再評価を通っている**ことを固定する。
    ///
    /// `still_collectable` を単体で試すだけだと、`collect_one_version` が
    /// それを**呼んでいなくても**緑のままになる（#4 で実際に残した型の穴 ──
    /// 純関数だけ試して呼び出し側の受理規則をテスト 0 本で残す）。
    #[sqlx::test(migrations = "./migrations")]
    async fn collect_one_version_skips_a_version_that_became_ineligible(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let page = insert_tree(&pool, old, 2).await;
        insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;

        // 一覧を引いた時点では対象。
        assert_eq!(
            storage_stats::gc_target_versions(&pool).await.unwrap(),
            vec![old]
        );
        // 一覧の取得後に別経路が alt text を挿した（= 課金済みの説明ができた）。
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: page,
                document_version_id: old,
                source_asset_sha256: "sha",
                text: "課金済み",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        let step = collect_one_version(&pool, old, NODE_DELETE_CHUNK).await.unwrap();
        assert_eq!(step.disposition, Disposition::Skipped);
        assert_eq!(step.nodes, 0);
        assert!(step.crop_paths.is_empty());
        assert_eq!(nodes_of(&pool, old).await, 4, "1 行も消していない");
        assert!(version_exists(&pool, old).await);
    }

    /// `still_collectable` は 3 条件を個別に見る（対象一覧を引いてから消すまでの間に
    /// 別経路が書き込んだ場合の最後の砦）。
    #[sqlx::test(migrations = "./migrations")]
    async fn still_collectable_rejects_each_broken_condition(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let old = insert_ver(&pool, att, "ck-old", ExtractionStatus::Superseded, None).await;
        let page = insert_tree(&pool, old, 0).await;
        insert_ver(&pool, att, "ck-new", ExtractionStatus::Completed, Some(old)).await;
        assert!(still_collectable(&pool, old).await.unwrap(), "前提: 対象である");

        // (i) alt text が挿さった。
        crate::db::node_alt_texts::insert_alt_text(
            &pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: page,
                document_version_id: old,
                source_asset_sha256: "sha",
                text: "t",
                origin: Origin::LlmInference.as_str(),
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();
        assert!(!still_collectable(&pool, old).await.unwrap());
        sqlx::query("DELETE FROM node_alt_texts").execute(&pool).await.unwrap();

        // (iii) 生存版が消えた。
        sqlx::query("DELETE FROM document_versions WHERE content_key = 'ck-new'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(!still_collectable(&pool, old).await.unwrap());

        // superseded でなくなった（別経路が status を戻した）。
        sqlx::query("UPDATE document_versions SET extraction_status = 'completed' WHERE id = ?")
            .bind(old)
            .execute(&pool)
            .await
            .unwrap();
        let att2 = setup_attachment(&pool, "Q").await;
        let sup = insert_ver(&pool, att2, "ck-s", ExtractionStatus::Superseded, None).await;
        insert_ver(&pool, att2, "ck-l", ExtractionStatus::Completed, None).await;
        assert!(still_collectable(&pool, sup).await.unwrap(), "前提: superseded なら対象");
        sqlx::query("UPDATE document_versions SET extraction_status = 'completed' WHERE id = ?")
            .bind(sup)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            !still_collectable(&pool, sup).await.unwrap(),
            "superseded でなくなったら対象外"
        );
    }

    /// `symbols(scope_node_id)` の索引を張る（migration 0018 の抜け）。冪等であること。
    #[sqlx::test(migrations = "./migrations")]
    async fn scope_node_index_is_created_and_idempotent(pool: SqlitePool) {
        let exists = || async {
            let n: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_master
                  WHERE type = 'index' AND name = 'idx_symbols_scope_node'",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            n
        };
        assert_eq!(exists().await, 0, "前提: migration では張られていない");
        crate::db::symbols::try_create_scope_node_index(&pool).await.unwrap();
        assert_eq!(exists().await, 1);
        // 起動のたびに呼ばれるので冪等でなければならない。
        crate::db::symbols::try_create_scope_node_index(&pool).await.unwrap();
        assert_eq!(exists().await, 1);
    }
}
