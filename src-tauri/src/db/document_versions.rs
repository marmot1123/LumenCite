//! LCIR `document_versions` テーブルのアクセサ（provenance と再現性の正本）。migration 0014。

use crate::document_ir::ExtractionStatus;
use crate::models::DocumentVersion;
use sqlx::SqlitePool;

/// 新規 document_version の挿入用パラメータ。
pub struct NewDocumentVersion<'a> {
    pub attachment_id: i64,
    pub content_key: &'a str,
    pub schema_version: &'a str,
    pub source_sha256: &'a str,
    pub source_mime_type: &'a str,
    pub extractor_name: &'a str,
    pub extractor_version: &'a str,
    pub config_hash: &'a str,
    pub parent_version_id: Option<i64>,
    pub status: ExtractionStatus,
    pub warnings_json: Option<&'a str>,
    pub metadata_json: Option<&'a str>,
}

/// document_version を挿入して id を返す。トランザクション内でも使えるよう executor を取る。
pub async fn insert_version<'e, E>(
    executor: E,
    v: &NewDocumentVersion<'_>,
) -> Result<i64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let id = sqlx::query(
        "INSERT INTO document_versions
            (attachment_id, content_key, schema_version, source_sha256, source_mime_type,
             extractor_name, extractor_version, config_hash, parent_version_id,
             extraction_status, warnings_json, metadata_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(v.attachment_id)
    .bind(v.content_key)
    .bind(v.schema_version)
    .bind(v.source_sha256)
    .bind(v.source_mime_type)
    .bind(v.extractor_name)
    .bind(v.extractor_version)
    .bind(v.config_hash)
    .bind(v.parent_version_id)
    .bind(v.status.as_str())
    .bind(v.warnings_json)
    .bind(v.metadata_json)
    .execute(executor)
    .await?
    .last_insert_rowid();
    Ok(id)
}

/// この添付に、同一 content_key の completed 系バージョンがあれば返す（冪等 build 用）。
pub async fn find_completed(
    pool: &SqlitePool,
    attachment_id: i64,
    content_key: &str,
) -> Result<Option<DocumentVersion>, sqlx::Error> {
    sqlx::query_as::<_, DocumentVersion>(
        "SELECT * FROM document_versions
         WHERE attachment_id = ? AND content_key = ?
           AND extraction_status IN ('completed', 'completed_with_warnings')
         ORDER BY id DESC LIMIT 1",
    )
    .bind(attachment_id)
    .bind(content_key)
    .fetch_optional(pool)
    .await
}

/// 添付の最新の completed 系バージョン（read 面 / FTS 再生成用）。
pub async fn latest_completed_for_attachment(
    pool: &SqlitePool,
    attachment_id: i64,
) -> Result<Option<DocumentVersion>, sqlx::Error> {
    sqlx::query_as::<_, DocumentVersion>(
        "SELECT * FROM document_versions
         WHERE attachment_id = ?
           AND extraction_status IN ('completed', 'completed_with_warnings')
         ORDER BY id DESC LIMIT 1",
    )
    .bind(attachment_id)
    .fetch_optional(pool)
    .await
}

/// 同一添付の（`except_id` 以外の）completed 系を superseded にする。新版採用時に呼ぶ。
pub async fn mark_superseded_for_attachment<'e, E>(
    executor: E,
    attachment_id: i64,
    except_id: i64,
) -> Result<u64, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let n = sqlx::query(
        "UPDATE document_versions SET extraction_status = 'superseded'
         WHERE attachment_id = ? AND id != ?
           AND extraction_status IN ('completed', 'completed_with_warnings')",
    )
    .bind(attachment_id)
    .bind(except_id)
    .execute(executor)
    .await?
    .rows_affected();
    Ok(n)
}

/// best-effort で `(attachment_id, content_key)` の UNIQUE 索引を張る。
///
/// content_key は添付に依存しない（同一ファイルを別添付にすれば同じ値）ため UNIQUE は
/// 添付ごと。既存 DB に重複があると `CREATE UNIQUE INDEX` は失敗して起動不能（brick）に
/// なるため、migration では張らず、起動時にここで**重複が無い時だけ**張る
/// （`db::entries::try_create_identifier_unique_indexes` と同じ作法）。作成したら true。
pub async fn try_create_content_key_unique_index(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let has_dup: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM document_versions
             GROUP BY attachment_id, content_key HAVING COUNT(*) > 1
         )",
    )
    .fetch_one(pool)
    .await?;

    if has_dup {
        eprintln!(
            "LCIR: document_versions に (attachment_id, content_key) 重複があるため UNIQUE 索引をスキップ"
        );
        return Ok(false);
    }

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_document_versions_attachment_content_key
             ON document_versions(attachment_id, content_key)",
    )
    .execute(pool)
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::attachments::add_attachment;
    use crate::db::entries::create_entry;
    use crate::document_ir::schema;
    use crate::models::{DocumentVersion, EntryInput};

    /// テスト用: id で 1 バージョンを取る（本体は用途が出るまで持たない）。
    async fn fetch(pool: &SqlitePool, id: i64) -> DocumentVersion {
        sqlx::query_as::<_, DocumentVersion>("SELECT * FROM document_versions WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

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

    fn nv(attachment_id: i64, ckey: &str, status: ExtractionStatus) -> NewDocumentVersion<'_> {
        NewDocumentVersion {
            attachment_id,
            content_key: ckey,
            schema_version: schema::SCHEMA_VERSION,
            source_sha256: "sha",
            source_mime_type: "application/pdf",
            extractor_name: schema::EXTRACTOR_NAME,
            extractor_version: schema::EXTRACTOR_VERSION,
            config_hash: "",
            parent_version_id: None,
            status,
            warnings_json: None,
            metadata_json: None,
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_and_get(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let id = insert_version(&pool, &nv(att, "ck1", ExtractionStatus::Completed))
            .await
            .unwrap();
        let v = fetch(&pool, id).await;
        assert_eq!(v.attachment_id, att);
        assert_eq!(v.content_key, "ck1");
        assert_eq!(v.extraction_status, "completed");
        assert_eq!(v.extractor_name, schema::EXTRACTOR_NAME);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_completed_is_scoped_to_attachment_and_key(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        insert_version(&pool, &nv(att, "ck1", ExtractionStatus::Completed))
            .await
            .unwrap();
        assert!(find_completed(&pool, att, "ck1").await.unwrap().is_some());
        assert!(find_completed(&pool, att, "nope").await.unwrap().is_none());
        // pending は completed とみなさない。
        let att2 = setup_attachment(&pool).await;
        insert_version(&pool, &nv(att2, "p", ExtractionStatus::Pending))
            .await
            .unwrap();
        assert!(find_completed(&pool, att2, "p").await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn mark_superseded_flags_others_only(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let old = insert_version(&pool, &nv(att, "old", ExtractionStatus::Completed))
            .await
            .unwrap();
        let new = insert_version(&pool, &nv(att, "new", ExtractionStatus::Completed))
            .await
            .unwrap();
        let n = mark_superseded_for_attachment(&pool, att, new).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(fetch(&pool, old).await.extraction_status, "superseded");
        assert_eq!(fetch(&pool, new).await.extraction_status, "completed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn version_cascades_on_attachment_delete(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        let id = insert_version(&pool, &nv(att, "ck", ExtractionStatus::Completed))
            .await
            .unwrap();
        sqlx::query("DELETE FROM attachments WHERE id = ?")
            .bind(att)
            .execute(&pool)
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM document_versions WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unique_index_skips_on_dup_and_created_when_clean(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        insert_version(&pool, &nv(att, "dup", ExtractionStatus::Completed))
            .await
            .unwrap();
        insert_version(&pool, &nv(att, "dup", ExtractionStatus::Completed))
            .await
            .unwrap();
        // (att, "dup") が重複 → スキップ。
        assert!(!try_create_content_key_unique_index(&pool).await.unwrap());

        // 重複を解消すれば張れて、以後の重複挿入は UNIQUE で弾かれる。
        let att2 = setup_attachment(&pool).await;
        insert_version(&pool, &nv(att2, "x", ExtractionStatus::Completed))
            .await
            .unwrap();
        // まだ (att,"dup") の重複が残っているので依然スキップ。
        assert!(!try_create_content_key_unique_index(&pool).await.unwrap());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unique_index_enforced_after_creation(pool: SqlitePool) {
        let att = setup_attachment(&pool).await;
        insert_version(&pool, &nv(att, "a", ExtractionStatus::Completed))
            .await
            .unwrap();
        insert_version(&pool, &nv(att, "b", ExtractionStatus::Completed))
            .await
            .unwrap();
        assert!(try_create_content_key_unique_index(&pool).await.unwrap());
        // 同一 (att, "a") の再挿入は UNIQUE 違反。
        let dup = insert_version(&pool, &nv(att, "a", ExtractionStatus::Completed)).await;
        assert!(dup.is_err());
    }
}
