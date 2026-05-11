use crate::models::Collection;
use sqlx::{Row, SqlitePool};

pub async fn get_collections(pool: &SqlitePool) -> Result<Vec<Collection>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, name, parent_id FROM collections ORDER BY name",
    )
    .fetch_all(pool)
    .await?;

    let mut all: Vec<Collection> = rows
        .iter()
        .map(|r| Collection {
            id: r.get("id"),
            name: r.get("name"),
            parent_id: r.get("parent_id"),
            children: vec![],
        })
        .collect();

    // build tree: attach children to their parents
    let ids: Vec<i64> = all.iter().map(|c| c.id).collect();
    for &child_id in &ids {
        let child_idx = all.iter().position(|c| c.id == child_id).unwrap();
        let parent_id = all[child_idx].parent_id;
        if let Some(pid) = parent_id {
            // temporarily remove child, then push into parent
            let child = all.remove(child_idx);
            if let Some(parent_idx) = all.iter().position(|c| c.id == pid) {
                all[parent_idx].children.push(child);
            } else {
                // parent not found (shouldn't happen), put back as root
                all.push(child);
            }
        }
    }

    // keep only root collections (those without parent_id)
    Ok(all.into_iter().filter(|c| c.parent_id.is_none()).collect())
}

pub async fn create_collection(
    pool: &SqlitePool,
    name: &str,
    parent_id: Option<i64>,
) -> Result<Collection, sqlx::Error> {
    let id = sqlx::query(
        "INSERT INTO collections (name, parent_id) VALUES (?, ?)",
    )
    .bind(name)
    .bind(parent_id)
    .execute(pool)
    .await?
    .last_insert_rowid();

    Ok(Collection { id, name: name.to_string(), parent_id, children: vec![] })
}

pub async fn update_collection(
    pool: &SqlitePool,
    id: i64,
    name: &str,
) -> Result<Collection, sqlx::Error> {
    let rows = sqlx::query("UPDATE collections SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }

    let row = sqlx::query("SELECT id, name, parent_id FROM collections WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;

    Ok(Collection {
        id: row.get("id"),
        name: row.get("name"),
        parent_id: row.get("parent_id"),
        children: vec![],
    })
}

pub async fn delete_collection(pool: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    let rows = sqlx::query("DELETE FROM collections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn add_entry_to_collection(
    pool: &SqlitePool,
    entry_id: i64,
    collection_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO entry_collections (entry_id, collection_id) VALUES (?, ?)",
    )
    .bind(entry_id)
    .bind(collection_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_entry_from_collection(
    pool: &SqlitePool,
    entry_id: i64,
    collection_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM entry_collections WHERE entry_id = ? AND collection_id = ?",
    )
    .bind(entry_id)
    .bind(collection_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entries::create_entry;
    use crate::models::EntryInput;

    #[sqlx::test(migrations = "./migrations")]
    async fn create_collection_returns_collection(pool: SqlitePool) {
        let col = create_collection(&pool, "My Papers", None).await.unwrap();
        assert!(col.id > 0);
        assert_eq!(col.name, "My Papers");
        assert!(col.parent_id.is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_collections_returns_all_roots(pool: SqlitePool) {
        create_collection(&pool, "A", None).await.unwrap();
        create_collection(&pool, "B", None).await.unwrap();

        let cols = get_collections(&pool).await.unwrap();
        assert_eq!(cols.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_collections_nests_children(pool: SqlitePool) {
        let parent = create_collection(&pool, "Parent", None).await.unwrap();
        create_collection(&pool, "Child", Some(parent.id)).await.unwrap();

        let cols = get_collections(&pool).await.unwrap();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].children.len(), 1);
        assert_eq!(cols[0].children[0].name, "Child");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_collection_changes_name(pool: SqlitePool) {
        let col = create_collection(&pool, "Old", None).await.unwrap();
        let updated = update_collection(&pool, col.id, "New").await.unwrap();
        assert_eq!(updated.name, "New");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_collection_not_found_returns_error(pool: SqlitePool) {
        let result = update_collection(&pool, 9999, "X").await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_collection_removes_it(pool: SqlitePool) {
        let col = create_collection(&pool, "Temp", None).await.unwrap();
        delete_collection(&pool, col.id).await.unwrap();
        let cols = get_collections(&pool).await.unwrap();
        assert!(cols.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_collection_not_found_returns_error(pool: SqlitePool) {
        let result = delete_collection(&pool, 9999).await;
        assert!(matches!(result, Err(sqlx::Error::RowNotFound)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_and_remove_entry_from_collection(pool: SqlitePool) {
        let entry = create_entry(&pool, &EntryInput {
            title: "Paper".to_string(),
            entry_type: "article".to_string(),
            ..Default::default()
        }).await.unwrap();
        let col = create_collection(&pool, "Col", None).await.unwrap();

        add_entry_to_collection(&pool, entry.id, col.id).await.unwrap();

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_collections WHERE entry_id = ? AND collection_id = ?",
        )
        .bind(entry.id)
        .bind(col.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 1);

        remove_entry_from_collection(&pool, entry.id, col.id).await.unwrap();

        let count_after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM entry_collections WHERE entry_id = ? AND collection_id = ?",
        )
        .bind(entry.id)
        .bind(col.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count_after, 0);
    }
}
