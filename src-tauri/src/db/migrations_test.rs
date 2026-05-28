//! マイグレーション単体テスト。
//!
//! `#[sqlx::test(migrations = "./migrations")]` は常に全マイグレーション適用済みの
//! pool を渡してくるので「マイグレーション適用前後の差分」が見たいテストは
//! [`apply_migrations_up_to`] で段階適用する。

#![cfg(test)]

use sqlx::migrate::Migrator;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Row, SqlitePool};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// メモリ DB を立てて、指定バージョン以下のマイグレーションを順に適用する。
async fn apply_migrations_up_to(max_version: i64) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open in-memory sqlite");

    for migration in MIGRATOR.iter() {
        if migration.version <= max_version {
            sqlx::raw_sql(&migration.sql)
                .execute(&pool)
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "migration {} ({}) failed: {e}",
                        migration.version, migration.description
                    )
                });
        }
    }
    pool
}

/// authors テーブルの全カラム名（小文字）。
async fn author_columns(pool: &SqlitePool) -> Vec<String> {
    sqlx::query("PRAGMA table_info(authors)")
        .fetch_all(pool)
        .await
        .expect("PRAGMA table_info")
        .iter()
        .map(|row| row.get::<String, _>("name").to_lowercase())
        .collect()
}

#[tokio::test]
async fn migration_0009_columns_exist() {
    let pool = apply_migrations_up_to(9).await;
    let cols = author_columns(&pool).await;

    for expected in [
        "middle_name",
        "suffix",
        "name_particle",
        "name_original",
        "given_name_original",
        "family_name_original",
        "original_script",
        "reading_family",
        "reading_given",
        "is_organization",
        "email",
        "homepage_url",
        "notes",
        "updated_at",
    ] {
        assert!(
            cols.iter().any(|c| c == expected),
            "expected authors.{expected} after migration 0009, got columns: {cols:?}"
        );
    }
}

#[tokio::test]
async fn migration_0009_creates_author_identifiers_table() {
    let pool = apply_migrations_up_to(9).await;

    // テーブル存在チェック
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='author_identifiers'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, 1, "author_identifiers テーブルが存在すること");

    // (scheme, value) の UNIQUE 制約: 同じ identifier を別著者に紐付けようとすると失敗
    sqlx::query("INSERT INTO authors (name) VALUES ('Alice')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO authors (name) VALUES ('Bob')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO author_identifiers (author_id, scheme, value) VALUES (1, 'dblp', '12/345')")
        .execute(&pool)
        .await
        .unwrap();

    let conflict = sqlx::query(
        "INSERT INTO author_identifiers (author_id, scheme, value) VALUES (2, 'dblp', '12/345')",
    )
    .execute(&pool)
    .await;
    assert!(
        conflict.is_err(),
        "同じ (scheme, value) は別著者に紐付けられないこと"
    );
}

#[tokio::test]
async fn migration_0009_backfills_orcid_from_authors_column() {
    // 0008 までの状態（authors.orcid のみ存在 / author_identifiers は無い）で著者を入れる
    let pool = apply_migrations_up_to(8).await;
    sqlx::query("INSERT INTO authors (name, orcid) VALUES ('Motoki Seki', '0000-0002-1825-0097')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO authors (name) VALUES ('No ORCID author')")
        .execute(&pool)
        .await
        .unwrap();

    // 0009 を適用 → author_identifiers にバックフィルされる
    let m9 = MIGRATOR
        .iter()
        .find(|m| m.version == 9)
        .expect("migration 0009 が見つかること");
    sqlx::raw_sql(&m9.sql).execute(&pool).await.unwrap();

    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT author_id, scheme, value FROM author_identifiers ORDER BY author_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert_eq!(
        rows,
        vec![(1, "orcid".to_string(), "0000-0002-1825-0097".to_string())],
        "ORCID を持つ著者のみが author_identifiers に複製されること"
    );
}

#[tokio::test]
async fn migration_0009_backfills_trims_whitespace_and_skips_empty() {
    // 余分な空白 / 空文字の orcid が混在する場合、空は無視・空白は TRIM される
    let pool = apply_migrations_up_to(8).await;
    sqlx::query("INSERT INTO authors (name, orcid) VALUES ('Padded', '  0000-0000-0000-0001  ')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO authors (name, orcid) VALUES ('Empty', '   ')")
        .execute(&pool)
        .await
        .unwrap();

    let m9 = MIGRATOR.iter().find(|m| m.version == 9).unwrap();
    sqlx::raw_sql(&m9.sql).execute(&pool).await.unwrap();

    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT author_id, scheme, value FROM author_identifiers ORDER BY author_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "空白のみの orcid は無視されること");
    assert_eq!(rows[0].2, "0000-0000-0000-0001", "TRIM 適用後の値が入ること");
}

#[tokio::test]
async fn migration_0009_existing_authors_get_updated_at_filled() {
    // 0008 までで作った著者は updated_at 列が無い。0009 適用後は created_at と同値が入ること
    let pool = apply_migrations_up_to(8).await;
    sqlx::query("INSERT INTO authors (name) VALUES ('Pre-existing')")
        .execute(&pool)
        .await
        .unwrap();

    let m9 = MIGRATOR.iter().find(|m| m.version == 9).unwrap();
    sqlx::raw_sql(&m9.sql).execute(&pool).await.unwrap();

    let row: (String, String) =
        sqlx::query_as("SELECT created_at, updated_at FROM authors WHERE name = 'Pre-existing'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, row.1, "既存行の updated_at は created_at で埋められること");
}

#[tokio::test]
async fn migration_0009_is_organization_defaults_to_zero() {
    let pool = apply_migrations_up_to(9).await;
    sqlx::query("INSERT INTO authors (name) VALUES ('Default flag')")
        .execute(&pool)
        .await
        .unwrap();

    let (flag,): (i64,) =
        sqlx::query_as("SELECT is_organization FROM authors WHERE name = 'Default flag'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(flag, 0, "is_organization の DEFAULT は 0");
}
