//! LCIR `node_alt_texts` テーブルのアクセサ（図の代替テキスト・migration 0020・Phase 8c）。
//! `figure` ノード（Phase 8a）の crop PNG を LLM Vision に説明させた alt text を持つ satellite 表。
//! 生成は build の外の opt-in バッチ（`generate_vision_alt_texts`）で、build のトランザクション内は
//! **版跨ぎ carry と旧版の刈り取りだけ**を行う。read 面（`load_lcir_document`）が版単位で引く。

use crate::models::NodeAltText;
use sqlx::SqlitePool;
use std::collections::HashMap;

/// alt text の挿入用パラメータ。
pub struct NewAltText<'a> {
    pub node_id: i64,
    pub document_version_id: i64,
    /// 説明した crop PNG の SHA-256（provenance ＋ 版跨ぎ carry のキー）。
    pub source_asset_sha256: &'a str,
    pub text: &'a str,
    /// `llm_inference`（生成）/ `user_edited`（将来の手編集）。
    pub origin: &'a str,
    pub confidence: Option<f64>,
    pub model: Option<&'a str>,
    /// 引き継ぎ元の版（NULL = この版で生成）。
    pub carried_from_version_id: Option<i64>,
}

/// バッチ（`generate_vision_alt_texts`）の対象 1 件。alt text がまだ無い `figure` ノードと、
/// 説明させる crop PNG の参照。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AltTextTarget {
    pub node_id: i64,
    pub document_version_id: i64,
    pub attachment_id: i64,
    /// crop PNG の SHA-256（`node_alt_texts.source_asset_sha256` に記録する）。
    pub asset_sha256: String,
    /// app data dir 相対・`/` 区切り。**存在保証なし**（欠損はスキップして次へ）。
    pub relative_path: String,
    pub mime_type: String,
}

/// alt text を挿入して id を返す。carry は build tx 内で呼ぶため executor を取る。
/// `UNIQUE (node_id, origin)` により同一ノード・同一 origin の二重挿入はエラーになる
/// （= 再生成による再課金を構造的に防ぐ）。
pub async fn insert_alt_text<'e, E>(executor: E, a: &NewAltText<'_>) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO node_alt_texts
            (node_id, document_version_id, source_asset_sha256, text, origin, confidence,
             model, carried_from_version_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(a.node_id)
    .bind(a.document_version_id)
    .bind(a.source_asset_sha256)
    .bind(a.text)
    .bind(a.origin)
    .bind(a.confidence)
    .bind(a.model)
    .bind(a.carried_from_version_id)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// 1 バージョンの全 alt text を返す（read 面の `LcirNode.alt_text` 組み立て用）。
pub async fn alt_texts_for_version(
    pool: &SqlitePool,
    version_id: i64,
) -> Result<Vec<NodeAltText>, sqlx::Error> {
    sqlx::query_as::<_, NodeAltText>(
        "SELECT * FROM node_alt_texts WHERE document_version_id = ? ORDER BY id",
    )
    .bind(version_id)
    .fetch_all(pool)
    .await
}

/// 版跨ぎ carry の材料: **同一添付の過去の全版**から、crop PNG の SHA-256 をキーに
/// `llm_inference` の alt text を引く（指紋一致 = バイト同一画像なので、どの版由来でも有効）。
/// 同一指紋が複数版にあるときは **最新（id 最大）** を採る。`user_edited` は対象外
/// （手編集は carry せず、当該版のノードに紐づいたまま残す）。
pub async fn alt_texts_by_asset_sha256(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<HashMap<String, NodeAltText>, sqlx::Error> {
    let rows = sqlx::query_as::<_, NodeAltText>(
        "SELECT nat.* FROM node_alt_texts nat
         JOIN document_versions dv ON dv.id = nat.document_version_id
         WHERE dv.attachment_id = ? AND nat.origin = 'llm_inference'
         ORDER BY nat.id",
    )
    .bind(attachment_id)
    .fetch_all(pool)
    .await?;
    // id 昇順で詰めるので、同一指紋は後勝ち = 最新が残る。
    let mut by_sha: HashMap<String, NodeAltText> = HashMap::new();
    for r in rows {
        by_sha.insert(r.source_asset_sha256.clone(), r);
    }
    Ok(by_sha)
}

/// GC（carry と同じ tx で呼ぶ）: 同一添付の **`keep_version_id` 以外の版**の `llm_inference`
/// 行のうち、**その画像が新版にも存在するもの**（= carry 済み）だけを削除する。crop PNG 自体は
/// 8a の GC で trash 済なので、引き継げた行を旧版に残しても履歴価値が薄く肥大化するだけ。
///
/// 逆に**新版に同一指紋の画像が無い行は残す**: crop の書き出しは領域単位で失敗しうる
/// （`ingestion/pdf` は失敗した領域を `file: None` + warning で継続する）ので、その図は
/// carry されず・新版では `page_crop` が無いため再生成対象にもならない。ここで消すと
/// 課金済みの説明が復旧不能に失われる。**`user_edited` は常に削除しない**（手編集の保護）。
/// 削除行数を返す。
pub async fn prune_carried_alt_texts<'e, E>(
    executor: E,
    attachment_id: i64,
    keep_version_id: i64,
) -> Result<u64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let res = sqlx::query(
        "DELETE FROM node_alt_texts
         WHERE origin = 'llm_inference'
           AND document_version_id != ?1
           AND document_version_id IN (
               SELECT id FROM document_versions WHERE attachment_id = ?2
           )
           AND source_asset_sha256 IN (
               SELECT sha256 FROM assets WHERE document_version_id = ?1
           )",
    )
    .bind(keep_version_id)
    .bind(attachment_id)
    .execute(executor)
    .await?;
    Ok(res.rows_affected())
}

/// バッチ対象: **最新 completed 版**の `figure` ノードのうち、`page_crop` アセットを持ち
/// alt text 行がまだ無いもの（**ゴミ箱のエントリは除外** — 捨てた文献の図を外部 API に送って
/// 課金しないため。他の一括バッチ対象クエリ `attachments_without_completed_lcir` /
/// `attachments_without_fulltext` と同じ規約）。既に行があるノードは返さない（`user_edited` も
/// 含めて尊重 = 手編集を Vision で上書きしない・再実行で再課金しない）。1 ノードに複数
/// `page_crop` がある場合は最初の 1 枚だけを対象にする。
pub async fn figures_missing_alt_text(
    pool: &SqlitePool,
) -> Result<Vec<AltTextTarget>, sqlx::Error> {
    sqlx::query_as::<_, AltTextTarget>(
        "SELECT dn.id AS node_id, dv.id AS document_version_id, dv.attachment_id,
                a.sha256 AS asset_sha256, a.relative_path, a.mime_type
         FROM document_nodes dn
         JOIN document_versions dv ON dv.id = dn.document_version_id
         JOIN attachments att ON att.id = dv.attachment_id
         JOIN entries e ON e.id = att.entry_id
         JOIN node_assets na ON na.node_id = dn.id AND na.role = 'page_crop'
         JOIN assets a ON a.id = na.asset_id
         WHERE e.deleted_at IS NULL
           AND dn.node_kind = 'figure'
           AND dv.extraction_status IN ('completed', 'completed_with_warnings')
           AND dv.id = (
               SELECT MAX(dv2.id) FROM document_versions dv2
               WHERE dv2.attachment_id = dv.attachment_id
                 AND dv2.extraction_status IN ('completed', 'completed_with_warnings')
           )
           AND NOT EXISTS (
               SELECT 1 FROM node_alt_texts nat WHERE nat.node_id = dn.id
           )
           AND na.id = (
               SELECT MIN(na2.id) FROM node_assets na2
               WHERE na2.node_id = dn.id AND na2.role = 'page_crop'
           )
         ORDER BY dv.attachment_id, dn.id",
    )
    .fetch_all(pool)
    .await
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

    async fn setup_attachment(pool: &SqlitePool) -> i64 {
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
        add_attachment(pool, entry.id, "attachments/1/p.pdf", "p.pdf", "application/pdf")
            .await
            .unwrap()
            .id
    }

    async fn insert_pdf_version(pool: &SqlitePool, attachment_id: i64, ckey: &str) -> i64 {
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

    /// figure ノード + page_crop アセットを 1 組作る。返り値は (node_id, asset_sha256)。
    async fn insert_figure_with_crop(
        pool: &SqlitePool,
        version_id: i64,
        index: i64,
        sha256: &str,
    ) -> i64 {
        let node = insert_node(
            pool,
            &NewDocumentNode {
                document_version_id: version_id,
                parent_id: None,
                node_kind: NodeKind::Figure.as_str(),
                ordinal: index,
                plain_text: None,
                language: None,
                confidence: Some(0.6),
                origin: Some(Origin::LayoutModel.as_str()),
                payload_json: Some(r#"{"figure_index":1}"#),
            },
        )
        .await
        .unwrap();
        let asset = insert_asset(
            pool,
            &NewAsset {
                document_version_id: version_id,
                sha256,
                mime_type: "image/png",
                relative_path: &format!("attachments/1/.lcir/1/abc/fig-p001-{index:02}.png"),
                width: Some(800),
                height: Some(600),
                size_bytes: Some(1234),
                metadata_json: None,
            },
        )
        .await
        .unwrap();
        insert_node_asset(pool, &NewNodeAsset { node_id: node, asset_id: asset }, "page_crop")
            .await
            .unwrap();
        node
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_fetch_alt_text(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = insert_pdf_version(&pool, att, "ck1").await;
        let node = insert_figure_with_crop(&pool, vid, 0, "cropsha1").await;

        insert_alt_text(
            &pool,
            &NewAltText {
                node_id: node,
                document_version_id: vid,
                source_asset_sha256: "cropsha1",
                text: "A pentagon graph with five labelled vertices.",
                origin: Origin::LlmInference.as_str(),
                confidence: Some(0.5),
                model: Some("gpt-4o-mini"),
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        let rows = alt_texts_for_version(&pool, vid).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].node_id, node);
        assert_eq!(rows[0].origin, "llm_inference");
        assert_eq!(rows[0].model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(rows[0].source_asset_sha256, "cropsha1");
        assert!(rows[0].carried_from_version_id.is_none());
    }

    /// UNIQUE (node_id, origin): 同一ノードへの生成 alt text は 1 件だけ（再課金の構造的防止）。
    /// 手編集は別 origin なので併存できる。
    #[sqlx::test(migrations = "./migrations")]
    async fn duplicate_generated_alt_text_is_rejected(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = insert_pdf_version(&pool, att, "ck1").await;
        let node = insert_figure_with_crop(&pool, vid, 0, "cropsha1").await;
        let row = |origin: &'static str, text: &'static str| NewAltText {
            node_id: node,
            document_version_id: vid,
            source_asset_sha256: "cropsha1",
            text,
            origin,
            confidence: Some(0.5),
            model: None,
            carried_from_version_id: None,
        };
        insert_alt_text(&pool, &row("llm_inference", "first")).await.unwrap();
        assert!(insert_alt_text(&pool, &row("llm_inference", "second")).await.is_err());
        insert_alt_text(&pool, &row("user_edited", "hand written")).await.unwrap();
        assert_eq!(alt_texts_for_version(&pool, vid).await.unwrap().len(), 2);
    }

    /// ノード削除 / 版削除の CASCADE。
    #[sqlx::test(migrations = "./migrations")]
    async fn cascades_on_delete(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = insert_pdf_version(&pool, att, "ck1").await;
        let node = insert_figure_with_crop(&pool, vid, 0, "cropsha1").await;
        insert_alt_text(
            &pool,
            &NewAltText {
                node_id: node,
                document_version_id: vid,
                source_asset_sha256: "cropsha1",
                text: "alt",
                origin: "llm_inference",
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();
        sqlx::query("DELETE FROM document_nodes WHERE id = ?")
            .bind(node)
            .execute(&pool)
            .await
            .unwrap();
        assert!(alt_texts_for_version(&pool, vid).await.unwrap().is_empty());
    }

    /// `alt_texts_by_asset_sha256` は同一添付の全版から指紋一致を集め、同一指紋は最新を採る。
    /// `user_edited` は含めない。
    #[sqlx::test(migrations = "./migrations")]
    async fn carry_lookup_prefers_latest_and_skips_user_edited(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let v1 = insert_pdf_version(&pool, att, "ck1").await;
        let v2 = insert_pdf_version(&pool, att, "ck2").await;
        let n1 = insert_figure_with_crop(&pool, v1, 0, "same").await;
        let n2 = insert_figure_with_crop(&pool, v2, 0, "same").await;
        let n3 = insert_figure_with_crop(&pool, v2, 1, "handmade").await;
        for (node, vid, sha, text, origin) in [
            (n1, v1, "same", "old description", "llm_inference"),
            (n2, v2, "same", "new description", "llm_inference"),
            (n3, v2, "handmade", "human wrote this", "user_edited"),
        ] {
            insert_alt_text(
                &pool,
                &NewAltText {
                    node_id: node,
                    document_version_id: vid,
                    source_asset_sha256: sha,
                    text,
                    origin,
                    confidence: Some(0.5),
                    model: None,
                    carried_from_version_id: None,
                },
            )
            .await
            .unwrap();
        }

        let map = alt_texts_by_asset_sha256(&pool, att).await.unwrap();
        assert_eq!(map.len(), 1, "user_edited は carry 対象に含めない");
        assert_eq!(map["same"].text, "new description");
    }

    /// 別添付の alt text は carry 対象にしない（指紋が同じでも添付を跨がない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn carry_lookup_is_scoped_to_attachment(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let other = add_attachment(
            &pool,
            1,
            "attachments/1/other.pdf",
            "other.pdf",
            "application/pdf",
        )
        .await
        .unwrap()
        .id;
        let v_other = insert_pdf_version(&pool, other, "ck-other").await;
        let n = insert_figure_with_crop(&pool, v_other, 0, "same").await;
        insert_alt_text(
            &pool,
            &NewAltText {
                node_id: n,
                document_version_id: v_other,
                source_asset_sha256: "same",
                text: "from the other attachment",
                origin: "llm_inference",
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        assert!(alt_texts_by_asset_sha256(&pool, att).await.unwrap().is_empty());
        assert_eq!(alt_texts_by_asset_sha256(&pool, other).await.unwrap().len(), 1);
    }

    /// prune は「新版にも同一指紋の画像がある」行だけを刈る。crop の書き出しが一部だけ失敗して
    /// carry できなかった行は残す（消すと課金済みの説明が復旧不能に失われる）。
    #[sqlx::test(migrations = "./migrations")]
    async fn prune_keeps_rows_whose_image_is_absent_from_the_new_version(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let v1 = insert_pdf_version(&pool, att, "ck1").await;
        let v2 = insert_pdf_version(&pool, att, "ck2").await;
        // 旧版には 2 図。新版では 1 図の crop しか書けなかった（残りは file: None 相当）。
        let carried = insert_figure_with_crop(&pool, v1, 0, "sha-carried").await;
        let lost = insert_figure_with_crop(&pool, v1, 1, "sha-lost").await;
        insert_figure_with_crop(&pool, v2, 0, "sha-carried").await;
        for (node, sha) in [(carried, "sha-carried"), (lost, "sha-lost")] {
            insert_alt_text(
                &pool,
                &NewAltText {
                    document_version_id: v1,
                    node_id: node,
                    source_asset_sha256: sha,
                    text: "t",
                    origin: "llm_inference",
                    confidence: None,
                    model: None,
                    carried_from_version_id: None,
                },
            )
            .await
            .unwrap();
        }

        let pruned = prune_carried_alt_texts(&pool, att, v2).await.unwrap();
        assert_eq!(pruned, 1, "新版に画像がある行だけ刈られる");
        let left = alt_texts_for_version(&pool, v1).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(
            left[0].source_asset_sha256, "sha-lost",
            "新版に画像が無い（carry できなかった）行は残る"
        );
    }

    /// prune は現版以外の生成行だけを刈り、`user_edited` と他添付には触らない。
    #[sqlx::test(migrations = "./migrations")]
    async fn prune_keeps_current_version_and_user_edits(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let v1 = insert_pdf_version(&pool, att, "ck1").await;
        let v2 = insert_pdf_version(&pool, att, "ck2").await;
        let n1 = insert_figure_with_crop(&pool, v1, 0, "sha-a").await;
        let n1b = insert_figure_with_crop(&pool, v1, 1, "sha-b").await;
        let n2 = insert_figure_with_crop(&pool, v2, 0, "sha-a").await;
        for (node, vid, sha, origin) in [
            (n1, v1, "sha-a", "llm_inference"),
            (n1b, v1, "sha-b", "user_edited"),
            (n2, v2, "sha-a", "llm_inference"),
        ] {
            insert_alt_text(
                &pool,
                &NewAltText {
                    node_id: node,
                    document_version_id: vid,
                    source_asset_sha256: sha,
                    text: "t",
                    origin,
                    confidence: None,
                    model: None,
                    carried_from_version_id: None,
                },
            )
            .await
            .unwrap();
        }

        let pruned = prune_carried_alt_texts(&pool, att, v2).await.unwrap();
        assert_eq!(pruned, 1, "旧版の生成行 1 件だけが刈られること");
        assert_eq!(alt_texts_for_version(&pool, v2).await.unwrap().len(), 1);
        let old = alt_texts_for_version(&pool, v1).await.unwrap();
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].origin, "user_edited", "手編集は残ること");
    }

    /// バッチ対象は「最新 completed 版・crop あり・alt text 無し」の figure だけ。
    #[sqlx::test(migrations = "./migrations")]
    async fn targets_exclude_done_old_version_and_cropless(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let v1 = insert_pdf_version(&pool, att, "ck1").await;
        let v2 = insert_pdf_version(&pool, att, "ck2").await;
        // 旧版の figure は対象外。
        insert_figure_with_crop(&pool, v1, 0, "old-sha").await;
        // 最新版: crop あり alt text 無し（対象）/ crop あり alt text 済（除外）/ crop 無し（除外）。
        let want = insert_figure_with_crop(&pool, v2, 0, "sha-want").await;
        let done = insert_figure_with_crop(&pool, v2, 1, "sha-done").await;
        insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: v2,
                parent_id: None,
                node_kind: NodeKind::Figure.as_str(),
                ordinal: 2,
                plain_text: None,
                language: None,
                confidence: Some(0.6),
                origin: Some(Origin::LayoutModel.as_str()),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        // 図でないノード（段落）は対象外。
        insert_node(
            &pool,
            &NewDocumentNode {
                document_version_id: v2,
                parent_id: None,
                node_kind: NodeKind::Paragraph.as_str(),
                ordinal: 3,
                plain_text: Some("text"),
                language: None,
                confidence: Some(0.9),
                origin: Some(Origin::LayoutModel.as_str()),
                payload_json: None,
            },
        )
        .await
        .unwrap();
        insert_alt_text(
            &pool,
            &NewAltText {
                node_id: done,
                document_version_id: v2,
                source_asset_sha256: "sha-done",
                text: "already described",
                origin: "llm_inference",
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();

        let targets = figures_missing_alt_text(&pool).await.unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].node_id, want);
        assert_eq!(targets[0].asset_sha256, "sha-want");
        assert_eq!(targets[0].document_version_id, v2);
        assert_eq!(targets[0].mime_type, "image/png");
    }

    /// **ゴミ箱のエントリの図は対象にしない** — 捨てた文献の図を外部 API に送って課金しないため
    /// （他の一括バッチ対象クエリと同じ規約。ゴミ箱は soft delete で LCIR 行は残る）。
    #[sqlx::test(migrations = "./migrations")]
    async fn targets_exclude_trashed_entries(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = insert_pdf_version(&pool, att, "ck1").await;
        insert_figure_with_crop(&pool, vid, 0, "sha").await;
        assert_eq!(figures_missing_alt_text(&pool).await.unwrap().len(), 1);

        // エントリをゴミ箱へ（soft delete: attachments / LCIR 行はそのまま残る）。
        sqlx::query("UPDATE entries SET deleted_at = datetime('now') WHERE id = 1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            figures_missing_alt_text(&pool).await.unwrap().is_empty(),
            "ゴミ箱のエントリの図は課金対象にならない"
        );
    }

    /// 手編集済みの図は Vision バッチの対象にしない（上書きしない）。
    #[sqlx::test(migrations = "./migrations")]
    async fn targets_exclude_user_edited(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let vid = insert_pdf_version(&pool, att, "ck1").await;
        let node = insert_figure_with_crop(&pool, vid, 0, "sha").await;
        insert_alt_text(
            &pool,
            &NewAltText {
                node_id: node,
                document_version_id: vid,
                source_asset_sha256: "sha",
                text: "hand written",
                origin: "user_edited",
                confidence: None,
                model: None,
                carried_from_version_id: None,
            },
        )
        .await
        .unwrap();
        assert!(figures_missing_alt_text(&pool).await.unwrap().is_empty());
    }
}
