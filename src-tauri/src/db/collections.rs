use crate::models::Collection;
use sqlx::{Row, SqlitePool};

pub async fn get_collections(pool: &SqlitePool) -> Result<Vec<Collection>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, name, parent_id FROM collections ORDER BY name",
    )
    .fetch_all(pool)
    .await?;

    let nodes: Vec<Collection> = rows
        .iter()
        .map(|r| Collection {
            id: r.get("id"),
            name: r.get("name"),
            parent_id: r.get("parent_id"),
            children: vec![],
        })
        .collect();

    Ok(build_tree(nodes))
}

/// 隣接リストから再帰的に階層ツリーを構築する（CR-007）。
///
/// 旧実装は `all` を破壊的に畳み込みながら親を線形探索していたため、孫（3 階層以上）が
/// 「親が先に入れ子化されると top-level から親を見つけられず」ツリーから消えていた。
/// 消えるかどうかは名前順（＝処理順）に依存した。ここでは id→子 id の隣接リストを作って
/// ルートから再帰し、深さと名前順に依存せず全ノードを保持する。親が存在しない孤児は
/// ルート扱いにし、循環は訪問済み集合で防ぐ。入力の名前順は保持する。
fn build_tree(nodes: Vec<Collection>) -> Vec<Collection> {
    use std::collections::{HashMap, HashSet};

    let all_ids: HashSet<i64> = nodes.iter().map(|n| n.id).collect();
    let mut children_of: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut roots: Vec<i64> = Vec::new();
    let mut by_id: HashMap<i64, Collection> = HashMap::new();

    for n in nodes {
        match n.parent_id {
            Some(pid) if all_ids.contains(&pid) => {
                children_of.entry(pid).or_default().push(n.id);
            }
            // parent_id が None、または存在しない親を指す孤児はルートとして残す。
            _ => roots.push(n.id),
        }
        by_id.insert(n.id, n);
    }

    fn build(
        id: i64,
        by_id: &mut HashMap<i64, Collection>,
        children_of: &HashMap<i64, Vec<i64>>,
        visiting: &mut HashSet<i64>,
    ) -> Option<Collection> {
        if !visiting.insert(id) {
            return None; // 循環ガード
        }
        let mut node = by_id.remove(&id)?;
        if let Some(kids) = children_of.get(&id) {
            for &kid in kids {
                if let Some(child) = build(kid, by_id, children_of, visiting) {
                    node.children.push(child);
                }
            }
        }
        Some(node)
    }

    let mut visiting = HashSet::new();
    roots
        .into_iter()
        .filter_map(|id| build(id, &mut by_id, &children_of, &mut visiting))
        .collect()
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

    fn node(id: i64, name: &str, parent: Option<i64>) -> Collection {
        Collection { id, name: name.to_string(), parent_id: parent, children: vec![] }
    }

    /// CR-007: 3 階層のツリーが、入力（名前）順の並びに関係なく全ノード保持される。
    #[test]
    fn build_tree_keeps_three_levels_regardless_of_order() {
        // A(root) > B > C
        let a = node(1, "A", None);
        let b = node(2, "B", Some(1));
        let c = node(3, "C", Some(2));

        // 入力順の全順列で同じツリーになること（旧実装は順序依存で孫が消えた）。
        let orders: Vec<Vec<Collection>> = vec![
            vec![a.clone(), b.clone(), c.clone()],
            vec![a.clone(), c.clone(), b.clone()],
            vec![c.clone(), b.clone(), a.clone()],
            vec![b.clone(), c.clone(), a.clone()],
        ];
        for order in orders {
            let tree = build_tree(order);
            assert_eq!(tree.len(), 1, "root は 1 つ");
            assert_eq!(tree[0].id, 1);
            assert_eq!(tree[0].children.len(), 1);
            assert_eq!(tree[0].children[0].id, 2, "B は A の子");
            assert_eq!(tree[0].children[0].children.len(), 1);
            assert_eq!(tree[0].children[0].children[0].id, 3, "C は B の子（消えない）");
        }
    }

    /// 親が存在しない孤児はルート扱いにする（消さない）。
    #[test]
    fn build_tree_treats_orphan_as_root() {
        let orphan = node(5, "Orphan", Some(999)); // 親 999 は存在しない
        let tree = build_tree(vec![orphan]);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].id, 5);
    }

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
