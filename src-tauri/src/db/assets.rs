//! LCIR `assets` / `node_assets` テーブルのアクセサ（図表アセット・migration 0019・Phase 8a）。
//! 図領域のページ crop PNG 等のバイナリをファイルシステムに置き、DB は相対パス + SHA-256 参照を
//! 持つ。build のトランザクション内で挿入し、read 面（`load_lcir_document`）が版単位で引く。
//! PDF 版のみ。ファイルの存在は保証しない（欠損許容）。

use crate::models::{Asset, NodeAsset};
use sqlx::SqlitePool;

/// アセットの挿入用パラメータ。
pub struct NewAsset<'a> {
    pub document_version_id: i64,
    pub sha256: &'a str,
    pub mime_type: &'a str,
    pub relative_path: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub size_bytes: Option<i64>,
    pub metadata_json: Option<&'a str>,
}

/// ノード ↔ アセット紐づけの挿入用パラメータ。
pub struct NewNodeAsset {
    pub node_id: i64,
    pub asset_id: i64,
}

/// アセットを挿入して id を返す。木構築のためトランザクション内でも使えるよう executor を取る。
pub async fn insert_asset<'e, E>(executor: E, a: &NewAsset<'_>) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO assets
            (document_version_id, sha256, mime_type, relative_path, width, height,
             size_bytes, metadata_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(a.document_version_id)
    .bind(a.sha256)
    .bind(a.mime_type)
    .bind(a.relative_path)
    .bind(a.width)
    .bind(a.height)
    .bind(a.size_bytes)
    .bind(a.metadata_json)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// ノード ↔ アセット紐づけを挿入する。
pub async fn insert_node_asset<'e, E>(
    executor: E,
    n: &NewNodeAsset,
    role: &str,
) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query("INSERT INTO node_assets (node_id, asset_id, role) VALUES (?, ?, ?)")
        .bind(n.node_id)
        .bind(n.asset_id)
        .bind(role)
        .execute(executor)
        .await?
        .last_insert_rowid();
    Ok(id)
}

/// `refresh_asset_file` が動かした行数。**2 つを別々に数える**のが要点で、
/// 「assets は更新したが alt text は 0 件」は正常（指紋が変わらなかった）だが、
/// 「assets が 0 件」は relative_path がずれた兆候。合算すると区別がつかない。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefreshedAsset {
    /// メタデータを更新した `assets` の行数。
    pub assets: u64,
    /// 指紋を付け替えた `node_alt_texts` の行数（指紋が同じなら 0）。
    pub alt_texts: u64,
}

/// self-heal（Phase 8a・reuse 経路）: 再レンダリングしたファイルのメタデータを
/// relative_path 一致で更新する。行が無ければ 0 を返す（新規行は作らない）。
///
/// **指紋が動いたら、その crop を説明している `node_alt_texts.source_asset_sha256` も
/// 同じトランザクションで付け替える**（debt-16）。付け替えないと、次の再構築で
/// `alt_texts_by_asset_sha256` が引けず carry が外れ、課金済みの説明を捨てて
/// Vision に再課金する。しかも症状が出るのは heal の何週間も後なので、
/// 抽出器のバグと区別がつかない。
///
/// **ファイル 1 枚ごとに tx を張る**（呼び出し側でまとめて張らない）。
/// 守りたい不変量は「`assets.sha256` と `node_alt_texts.source_asset_sha256` が
/// 乖離しない」ことで、それは 1 枚単位で閉じる。
///
/// 途中で失敗したときに**そこまでの更新が残る**のが分割した理由。まとめて張ると、
/// 再レンダリング（tx の外・`heal_missing_assets` がループの前に完了させる）は
/// 全ファイルを書き直したのに DB は 1 行も追随していない、という最悪の食い違いになる
/// ── heal は「ファイルが欠けている」ときしか起動しないので、全ファイルが揃った後は
/// 二度と走らず、その食い違いは恒久化する。分割すれば残るのは未処理ぶんだけで済む。
pub async fn refresh_asset_file(
    pool: &SqlitePool,
    version_id: i64,
    relative_path: &str,
    sha256: &str,
    dims: (i64, i64),
    size_bytes: i64,
) -> Result<RefreshedAsset, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // **assets の UPDATE より前に打つ** — 旧指紋を `assets` から読むため。
    //
    // 述語が 3 つ要る。実 DB の実測がそれぞれの理由:
    //   ①版スコープ … 同じ sha256 が添付を跨いで 3 件ある（他文献の行を書き換えない）
    //   ②旧指紋一致 … 既に別の絵を指している行を巻き込まない
    //   ③ノードリンク … 同一版で同じ sha を共有する crop が最大 11 枚ある版がある。
    //                    これが無いと、先に処理した 1 枚の新指紋が同版の全行に伝播する
    // ④は「実際に変わったときだけ数える」ためのもの（heal は変わっていない
    //    ファイルもすべて描き直すので、これが無いと毎回全行を「付け替えた」と数える）。
    let retargeted = sqlx::query(
        "UPDATE node_alt_texts
            SET source_asset_sha256 = ?1
          WHERE document_version_id = ?2
            AND source_asset_sha256 <> ?1
            AND source_asset_sha256 = (
                SELECT a.sha256 FROM assets a
                 WHERE a.document_version_id = ?2 AND a.relative_path = ?3
            )
            AND node_id IN (
                SELECT na.node_id FROM node_assets na
                  JOIN assets a2 ON a2.id = na.asset_id
                 WHERE na.role = 'page_crop'
                   AND a2.document_version_id = ?2 AND a2.relative_path = ?3
            )",
    )
    .bind(sha256)
    .bind(version_id)
    .bind(relative_path)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    let res = sqlx::query(
        "UPDATE assets SET sha256 = ?, width = ?, height = ?, size_bytes = ?
         WHERE document_version_id = ? AND relative_path = ?",
    )
    .bind(sha256)
    .bind(dims.0)
    .bind(dims.1)
    .bind(size_bytes)
    .bind(version_id)
    .bind(relative_path)
    .execute(&mut *tx)
    .await?;

    let assets = res.rows_affected();
    tx.commit().await?;
    Ok(RefreshedAsset {
        assets,
        alt_texts: retargeted,
    })
}

/// 1 バージョンの全アセットを返す（read 面の `LcirNode.assets` 組み立て用）。
pub async fn assets_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<Asset>, sqlx::Error> {
    sqlx::query_as::<_, Asset>("SELECT * FROM assets WHERE document_version_id = ? ORDER BY id")
        .bind(version_id)
        .fetch_all(pool)
        .await
}

/// 1 バージョンの全ノード ↔ アセット紐づけを返す（`document_nodes` を JOIN して版でスコープ）。
pub async fn node_assets_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<NodeAsset>, sqlx::Error> {
    sqlx::query_as::<_, NodeAsset>(
        "SELECT na.* FROM node_assets na
         JOIN document_nodes dn ON dn.id = na.node_id
         WHERE dn.document_version_id = ?
         ORDER BY na.node_id, na.id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::document_nodes::{insert_node, NewDocumentNode};
    use crate::db::document_versions::{insert_version, NewDocumentVersion};
    use crate::db::entries::create_entry;
    use crate::document_ir::{schema, ExtractionStatus, NodeKind};
    use crate::models::EntryInput;

    async fn setup_node(pool: &SqlitePool) -> (i64, i64) {
        let entry = create_entry(
            pool,
            &EntryInput {
                title: "P".to_string(),
                entry_type: "article".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let att = add_attachment(pool, entry.id, "attachments/1/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap()
            .id;
        let vid = insert_version(
            pool,
            &NewDocumentVersion {
                attachment_id: att,
                content_key: "ck",
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let node = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: vid,
                parent_id: None,
                node_kind: NodeKind::Figure.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: Some(r#"{"figure_index":1}"#),
            },
        )
        .await
        .unwrap();
        (vid, node)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_fetch_asset_and_link(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: vid,
                sha256: "abc123",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/deadbeef/fig-p001-00.png",
                width: Some(800),
                height: Some(600),
                size_bytes: Some(12345),
                metadata_json: Some(r#"{"page":1,"region_index":0}"#),
            },
        )
        .await
        .unwrap();
        insert_node_asset(&pool, &NewNodeAsset { node_id: node, asset_id: aid }, "page_crop")
            .await
            .unwrap();

        let assets = assets_for_version(&pool, vid).await.unwrap();
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].sha256, "abc123");
        assert_eq!(assets[0].relative_path, "attachments/1/.lcir/1/deadbeef/fig-p001-00.png");
        assert_eq!(assets[0].width, Some(800));
        assert_eq!(assets[0].size_bytes, Some(12345));

        let links = node_assets_for_version(&pool, vid).await.unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].node_id, node);
        assert_eq!(links[0].asset_id, aid);
        assert_eq!(links[0].role, "page_crop");
    }

    /// 同一 (node, asset, role) の重複紐づけは UNIQUE 制約で拒否される。
    #[sqlx::test(migrations = "./migrations")]
    async fn duplicate_link_is_rejected(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: vid,
                sha256: "abc",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/deadbeef/fig-p001-00.png",
                width: None,
                height: None,
                size_bytes: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        let link = NewNodeAsset { node_id: node, asset_id: aid };
        insert_node_asset(&pool, &link, "page_crop").await.unwrap();
        assert!(insert_node_asset(&pool, &link, "page_crop").await.is_err());
        // 別 role なら許される。
        insert_node_asset(&pool, &link, "thumbnail").await.unwrap();
    }

    /// version 削除でアセットが、ノード削除で紐づけが CASCADE 削除される。
    #[sqlx::test(migrations = "./migrations")]
    async fn cascades_on_delete(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: vid,
                sha256: "abc",
                mime_type: "image/png",
                relative_path: "attachments/1/.lcir/1/deadbeef/fig-p001-00.png",
                width: None,
                height: None,
                size_bytes: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        insert_node_asset(&pool, &NewNodeAsset { node_id: node, asset_id: aid }, "page_crop")
            .await
            .unwrap();

        // ノード削除 → 紐づけだけ消え、アセット行は残る。
        sqlx::query("DELETE FROM document_nodes WHERE id = ?")
            .bind(node)
            .execute(&pool)
            .await
            .unwrap();
        assert!(node_assets_for_version(&pool, vid).await.unwrap().is_empty());
        assert_eq!(assets_for_version(&pool, vid).await.unwrap().len(), 1);

        // version 削除 → アセット行も消える。
        sqlx::query("DELETE FROM document_versions WHERE id = ?")
            .bind(vid)
            .execute(&pool)
            .await
            .unwrap();
        assert!(assets_for_version(&pool, vid).await.unwrap().is_empty());
    }

    // ---- debt-16: self-heal が指紋を動かしたら alt text の指紋も追随する ----

    /// エントリ + 添付を 1 組。`title` を変えると別文献になる。
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

    async fn setup_version(pool: &SqlitePool, attachment_id: i64, ckey: &str) -> i64 {
        insert_version(
            pool,
            &NewDocumentVersion {
                attachment_id,
                content_key: ckey,
                schema_version: schema::SCHEMA_VERSION,
                source_sha256: "sha",
                source_mime_type: "application/pdf",
                extractor_name: schema::EXTRACTOR_NAME,
                extractor_version: schema::EXTRACTOR_VERSION,
                config_hash: "",
                parent_version_id: None,
                status: ExtractionStatus::Completed,
                warnings_json: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap()
    }

    /// figure ノード + `page_crop` アセット + 生成済み alt text を 1 組作る。
    /// 返り値は figure ノード id。
    async fn setup_figure_with_alt_text(
        pool: &SqlitePool,
        version_id: i64,
        relative_path: &str,
        sha: &str,
        origin: &str,
    ) -> i64 {
        let node = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: version_id,
                parent_id: None,
                node_kind: NodeKind::Figure.as_str(),
                ordinal: 0,
                plain_text: None,
                language: None,
                confidence: Some(0.6),
                origin: Some("layout_model"),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        let aid = insert_asset(
            pool,
            &NewAsset {
                document_version_id: version_id,
                sha256: sha,
                mime_type: "image/png",
                relative_path,
                width: Some(800),
                height: Some(600),
                size_bytes: Some(1000),
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        insert_node_asset(pool, &NewNodeAsset { node_id: node, asset_id: aid }, "page_crop")
            .await
            .unwrap();
        crate::db::node_alt_texts::insert_alt_text(
            pool,
            &crate::db::node_alt_texts::NewAltText {
                node_id: node,
                document_version_id: version_id,
                source_asset_sha256: sha,
                text: "A description.",
                origin,
                confidence: Some(0.5),
                model: Some("claude-sonnet-5"),
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();
        node
    }

    async fn alt_sha_of(pool: &SqlitePool, node_id: i64) -> String {
        sqlx::query_scalar("SELECT source_asset_sha256 FROM node_alt_texts WHERE node_id = ?")
            .bind(node_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// 再レンダリングで指紋が動いたら、その crop を説明している alt text の
    /// `source_asset_sha256` も同じ tx で追随する（debt-16）。
    ///
    /// 追随しないと、次の再構築で `alt_texts_by_asset_sha256` が引けず carry が外れ、
    /// 課金済みの説明を捨てて Vision に再課金する。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_retargets_the_alt_text_of_the_refreshed_asset(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let vid = setup_version(&pool, att, "ck").await;
        let rel = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let node = setup_figure_with_alt_text(&pool, vid, rel, "old-sha", "llm_inference").await;

        let n = refresh_asset_file(&pool, vid, rel, "new-sha", (900, 700), 2000)
            .await
            .unwrap();

        assert_eq!(n.assets, 1, "assets 行が 1 件更新される");
        assert_eq!(n.alt_texts, 1, "alt text 行も 1 件付け替わる");
        assert_eq!(assets_for_version(&pool, vid).await.unwrap()[0].sha256, "new-sha");
        assert_eq!(alt_sha_of(&pool, node).await, "new-sha");
    }

    /// 同一版で **同じ旧 sha を共有する 2 枚**（実 DB に最大 11 多重の版がある）のうち
    /// 片方だけを refresh したとき、もう片方の alt text は動かない。
    ///
    /// 版スコープだけの `WHERE source_asset_sha256 = ?old` だと両方が動く。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_retargets_only_the_alt_text_of_the_refreshed_asset(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let vid = setup_version(&pool, att, "ck").await;
        let rel_a = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let rel_b = "attachments/1/.lcir/1/deadbeef/fig-p002-00.png";
        let node_a = setup_figure_with_alt_text(&pool, vid, rel_a, "dup-sha", "llm_inference").await;
        let node_b = setup_figure_with_alt_text(&pool, vid, rel_b, "dup-sha", "llm_inference").await;

        let n = refresh_asset_file(&pool, vid, rel_a, "new-sha", (900, 700), 2000)
            .await
            .unwrap();

        assert_eq!(n.alt_texts, 1, "付け替わるのは refresh した 1 枚ぶんだけ");
        assert_eq!(alt_sha_of(&pool, node_a).await, "new-sha");
        assert_eq!(
            alt_sha_of(&pool, node_b).await,
            "dup-sha",
            "同じ旧 sha を共有する別の crop の説明は動かさない"
        );
    }

    /// 別の版・別の添付に同じ旧 sha の行があっても巻き込まない
    /// （実 DB に添付を跨ぐ sha が 3 件ある）。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_does_not_touch_other_versions_or_attachments(pool: SqlitePool) {
        let rel = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let att1 = setup_attachment(&pool, "P1").await;
        let v1 = setup_version(&pool, att1, "ck1").await;
        let target = setup_figure_with_alt_text(&pool, v1, rel, "old-sha", "llm_inference").await;
        // 同じ添付の別版。
        let v2 = setup_version(&pool, att1, "ck2").await;
        let other_ver = setup_figure_with_alt_text(&pool, v2, rel, "old-sha", "llm_inference").await;
        // 別添付（別文献）。
        let att2 = setup_attachment(&pool, "P2").await;
        let v3 = setup_version(&pool, att2, "ck3").await;
        let other_att = setup_figure_with_alt_text(&pool, v3, rel, "old-sha", "llm_inference").await;

        let n = refresh_asset_file(&pool, v1, rel, "new-sha", (900, 700), 2000)
            .await
            .unwrap();

        assert_eq!(n.alt_texts, 1);
        assert_eq!(alt_sha_of(&pool, target).await, "new-sha");
        assert_eq!(alt_sha_of(&pool, other_ver).await, "old-sha", "別版は無傷");
        assert_eq!(alt_sha_of(&pool, other_att).await, "old-sha", "別添付は無傷");
    }

    /// 指紋が変わっていなければ alt text は 1 行も触らない。
    ///
    /// heal は欠けた 1 枚だけでなく**その版の全ファイルを描き直す**ので、
    /// 変わっていない行に対してもこの経路を毎回通る。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_is_a_no_op_for_alt_texts_when_the_fingerprint_is_unchanged(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let vid = setup_version(&pool, att, "ck").await;
        let rel = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let node = setup_figure_with_alt_text(&pool, vid, rel, "same-sha", "llm_inference").await;

        let n = refresh_asset_file(&pool, vid, rel, "same-sha", (800, 600), 1000)
            .await
            .unwrap();

        assert_eq!(n.assets, 1, "assets 行の UPDATE 自体は走る");
        assert_eq!(n.alt_texts, 0, "指紋が同じなら alt text は動かさない");
        assert_eq!(alt_sha_of(&pool, node).await, "same-sha");
    }

    /// 版スコープが効いていること。
    ///
    /// **`node_alt_texts.document_version_id` が「そのノードの版」と一致することを
    /// スキーマは強制していない**（両方とも独立した列で、揃えているのは挿入経路の作法だけ）。
    /// 通常のデータではノードリンク条件だけで版は絞れてしまうので、ここでは
    /// その不変条件が破れた行を 1 つ置いて、版スコープが独立に効くことを固定する。
    /// 破れていた場合に版スコープが無いと、**他の版の行を書き換える**。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_scopes_the_retarget_to_the_version_even_if_the_row_is_inconsistent(
        pool: SqlitePool,
    ) {
        let att = setup_attachment(&pool, "P").await;
        let v1 = setup_version(&pool, att, "ck1").await;
        let v2 = setup_version(&pool, att, "ck2").await;
        let rel = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let node = setup_figure_with_alt_text(&pool, v1, rel, "old-sha", "llm_inference").await;
        // v1 のノードに紐づく alt text が、版としては v2 を指している行（不整合）。
        sqlx::query("UPDATE node_alt_texts SET document_version_id = ? WHERE node_id = ?")
            .bind(v2)
            .bind(node)
            .execute(&pool)
            .await
            .unwrap();
        // v1 側にも asset を持たせて、ノードリンク条件だけなら当たる状態にする。
        let n = refresh_asset_file(&pool, v1, rel, "new-sha", (900, 700), 2000)
            .await
            .unwrap();

        assert_eq!(n.assets, 1);
        assert_eq!(n.alt_texts, 0, "版が違う行は付け替えない");
        assert_eq!(alt_sha_of(&pool, node).await, "old-sha");
    }

    /// **別の絵を指している alt text は付け替えない。**
    ///
    /// 同じ版・同じノードでも、alt text の指紋がこの crop の旧指紋と一致しないなら
    /// それは別の画像についての記述なので触らない（「ついでの穴埋め」で
    /// 前から在った行の値を壊さないための条件）。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_leaves_an_alt_text_that_points_at_a_different_image(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let vid = setup_version(&pool, att, "ck").await;
        let rel = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let node = setup_figure_with_alt_text(&pool, vid, rel, "asset-sha", "llm_inference").await;
        // alt text だけが別の指紋を指している状態にする。
        sqlx::query("UPDATE node_alt_texts SET source_asset_sha256 = 'unrelated-sha'")
            .execute(&pool)
            .await
            .unwrap();

        let n = refresh_asset_file(&pool, vid, rel, "new-sha", (900, 700), 2000)
            .await
            .unwrap();

        assert_eq!(n.assets, 1);
        assert_eq!(n.alt_texts, 0, "旧指紋と一致しない行は付け替えない");
        assert_eq!(alt_sha_of(&pool, node).await, "unrelated-sha");
    }

    /// 手編集（`user_edited`）の指紋も付け替える。
    ///
    /// carry / prune は `user_edited` を必ず除外するが、付け替えは削除でも上書きでもなく
    /// 「同じ絵を指し直す」だけなので、除外すると手編集行だけが恒久的に
    /// 存在しない指紋を指し続ける。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_retargets_user_edited_alt_texts_too(pool: SqlitePool) {
        let att = setup_attachment(&pool, "P").await;
        let vid = setup_version(&pool, att, "ck").await;
        let rel = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let node = setup_figure_with_alt_text(&pool, vid, rel, "old-sha", "user_edited").await;

        let n = refresh_asset_file(&pool, vid, rel, "new-sha", (900, 700), 2000)
            .await
            .unwrap();

        assert_eq!(n.alt_texts, 1);
        assert_eq!(alt_sha_of(&pool, node).await, "new-sha");
    }

    /// alt text がまだ無い版でも壊れない（8c を回していない添付が大多数）。
    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_without_any_alt_text_reports_zero(pool: SqlitePool) {
        let (vid, node) = setup_node(&pool).await;
        let rel = "attachments/1/.lcir/1/deadbeef/fig-p001-00.png";
        let aid = insert_asset(
            &pool,
            &NewAsset {
                document_version_id: vid,
                sha256: "old-sha",
                mime_type: "image/png",
                relative_path: rel,
                width: None,
                height: None,
                size_bytes: None,
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        insert_node_asset(&pool, &NewNodeAsset { node_id: node, asset_id: aid }, "page_crop")
            .await
            .unwrap();

        let n = refresh_asset_file(&pool, vid, rel, "new-sha", (1, 1), 1)
            .await
            .unwrap();
        assert_eq!(n.assets, 1);
        assert_eq!(n.alt_texts, 0);
    }
}
