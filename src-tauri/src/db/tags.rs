use crate::models::Tag;
use sqlx::SqlitePool;

pub async fn get_tags(pool: &SqlitePool) -> Result<Vec<Tag>, sqlx::Error> {
    sqlx::query_as("SELECT id, name FROM tags ORDER BY name")
        .fetch_all(pool)
        .await
}

pub async fn create_tag(pool: &SqlitePool, name: &str) -> Result<Tag, sqlx::Error> {
    let id = sqlx::query("INSERT INTO tags (name) VALUES (?)")
        .bind(name)
        .execute(pool)
        .await?
        .last_insert_rowid();

    Ok(Tag { id, name: name.to_string() })
}

pub async fn delete_tag(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // タグ名は entries_fts.tags_text に含まれるため、削除の影響を受ける
    // エントリを控えておき、削除後に再同期する。
    let entry_ids: Vec<i64> =
        sqlx::query_scalar("SELECT entry_id FROM entry_tags WHERE tag_id = ?")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;

    let rows = sqlx::query("DELETE FROM tags WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }

    for entry_id in entry_ids {
        crate::db::entries::sync_entries_fts(&mut tx, entry_id).await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn add_tag_to_entry(
    pool: &SqlitePool,
    entry_id: i64,
    tag_id: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT OR IGNORE INTO entry_tags (entry_id, tag_id) VALUES (?, ?)",
    )
    .bind(entry_id)
    .bind(tag_id)
    .execute(&mut *tx)
    .await?;
    // タグの変更は entries_fts.tags_text に影響するので再同期する
    crate::db::entries::sync_entries_fts(&mut tx, entry_id).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn remove_tag_from_entry(
    pool: &SqlitePool,
    entry_id: i64,
    tag_id: i64,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM entry_tags WHERE entry_id = ? AND tag_id = ?")
        .bind(entry_id)
        .bind(tag_id)
        .execute(&mut *tx)
        .await?;
    crate::db::entries::sync_entries_fts(&mut tx, entry_id).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    #[sqlx::test(migrations = "./migrations")]
    async fn get_tags_returns_all_sorted(pool: SqlitePool) {
        create_tag(&pool, "Zebra").await.unwrap();
        create_tag(&pool, "Alpha").await.unwrap();

        let tags = get_tags(&pool).await.unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].name, "Alpha");
        assert_eq!(tags[1].name, "Zebra");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_tag_returns_tag_with_id(pool: SqlitePool) {
        let tag = create_tag(&pool, "ML").await.unwrap();
        assert!(tag.id > 0);
        assert_eq!(tag.name, "ML");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_tag_removes_tag(pool: SqlitePool) {
        let tag = create_tag(&pool, "Temp").await.unwrap();
        delete_tag(&pool, tag.id).await.unwrap();
        let tags = get_tags(&pool).await.unwrap();
        assert!(tags.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_tag_not_found_returns_error(pool: SqlitePool) {
        let result = delete_tag(&pool, 9999).await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_and_remove_tag_from_entry(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let tag = create_tag(&pool, "NLP").await.unwrap();

        add_tag_to_entry(&pool, entry.id, tag.id).await.unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags WHERE entry_id = ? AND tag_id = ?",
        )
        .bind(entry.id)
        .bind(tag.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);

        remove_tag_from_entry(&pool, entry.id, tag.id).await.unwrap();

        let count_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags WHERE entry_id = ? AND tag_id = ?",
        )
        .bind(entry.id)
        .bind(tag.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count_after, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_to_entry_syncs_fts_for_tag_name_search(pool: SqlitePool) {
        use crate::db::entries::search_entries;
        let entry = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let tag = create_tag(&pool, "transformer-architecture").await.unwrap();

        add_tag_to_entry(&pool, entry.id, tag.id).await.unwrap();

        let hits = search_entries(&pool, "transformer", None, None).await.unwrap();
        assert_eq!(hits.len(), 1, "タグ追加後にタグ名で検索できるべき");
        assert_eq!(hits[0].id, entry.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn remove_tag_from_entry_syncs_fts(pool: SqlitePool) {
        use crate::db::entries::search_entries;
        let entry = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let tag = create_tag(&pool, "transformer-architecture").await.unwrap();
        add_tag_to_entry(&pool, entry.id, tag.id).await.unwrap();

        remove_tag_from_entry(&pool, entry.id, tag.id).await.unwrap();

        let hits = search_entries(&pool, "transformer", None, None).await.unwrap();
        assert!(hits.is_empty(), "タグ削除後はタグ名検索でヒットしないべき");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_tag_syncs_fts_of_tagged_entries(pool: SqlitePool) {
        use crate::db::entries::search_entries;
        let entry = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let tag = create_tag(&pool, "obsolete-topic").await.unwrap();
        add_tag_to_entry(&pool, entry.id, tag.id).await.unwrap();

        delete_tag(&pool, tag.id).await.unwrap();

        let hits = search_entries(&pool, "obsolete", None, None).await.unwrap();
        assert!(hits.is_empty(), "削除済みタグ名で検索にヒットし続けないこと");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_tag_to_entry_is_idempotent(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let tag = create_tag(&pool, "CV").await.unwrap();

        add_tag_to_entry(&pool, entry.id, tag.id).await.unwrap();
        add_tag_to_entry(&pool, entry.id, tag.id).await.unwrap(); // idempotent

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_tags WHERE entry_id = ?",
        )
        .bind(entry.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }
}
