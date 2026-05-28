//! 著者マスタへのアクセス層。
//!
//! v0.3.0 で `entries.rs` から独立させた。主目的:
//! - 名寄せロジックを ORCID → 正規化 name → INSERT の 3 段照合に拡張
//! - 多言語名 / 国際識別子の編集 API（M7 で追加予定）の置き場
//!
//! `update_author` / `merge_authors` / 識別子編集系の Tauri コマンドは
//! 後続マイルストン（M7）で足す。M3 では「entry 作成/更新時の名寄せ」を担う
//! `get_or_create_author` 周辺のみ提供する。

use sqlx::{Sqlite, SqlitePool, Transaction};
use unicode_normalization::UnicodeNormalization;

use crate::models::{Author, AuthorIdentifier, AuthorInput};

/// 著者名の照合用に正規化する。
///
/// 同一著者の表記揺れ（半角/全角・大文字小文字・前後空白・合成済み/分解済み等価）
/// を吸収する目的で、`get_or_create_author` の第 2 段照合キーとして使う。
///
/// - 前後の空白を除去（`str::trim`）
/// - Unicode NFKC 正規化（"ＳＥＫＩ" → "SEKI"、合成済み é と分解 e+◌́ を統一）
/// - ASCII lowercase 寄りの `String::to_lowercase`
///
/// CJK 文字は NFKC で半角化される可能性があるため、保存用ではなく**照合キーとしてのみ**
/// 使い、表示用には触らないこと。
pub(crate) fn normalize_name(name: &str) -> String {
    name.trim().nfkc().collect::<String>().to_lowercase()
}

/// `authors` の全列を Author 構造体の field 名で SELECT する SQL。
/// FromRow は field 名でマッチングするため、列名と field 名は揃える。
/// identifiers は別テーブルから JOIN するため SELECT には含めない（[`load_identifiers`] が補う）。
const AUTHOR_COLUMNS: &str = "id, name,
    given_name, middle_name, family_name, suffix, name_particle,
    name_original, given_name_original, family_name_original, original_script,
    reading_family, reading_given,
    is_organization,
    email, homepage_url, notes,
    orcid, updated_at";

/// 指定 id の著者を identifiers 込みで取得する。
#[allow(dead_code)] // M7 の Tauri コマンドで配線
pub(crate) async fn get_author(pool: &SqlitePool, id: i64) -> Result<Option<Author>, sqlx::Error> {
    let mut author: Option<Author> = sqlx::query_as(&format!(
        "SELECT {AUTHOR_COLUMNS} FROM authors WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?;

    if let Some(a) = author.as_mut() {
        a.identifiers = load_identifiers(pool, a.id).await?;
    }
    Ok(author)
}

/// 単純な name 部分一致検索。M3 では LIKE 前方一致のみ。
/// M9 以降で reading_family / original 名も含めた検索に拡張する余地。
#[allow(dead_code)] // M7 の Tauri コマンドで配線
pub(crate) async fn search_authors(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
) -> Result<Vec<Author>, sqlx::Error> {
    let like = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
    let mut rows: Vec<Author> = sqlx::query_as(&format!(
        "SELECT {AUTHOR_COLUMNS}
           FROM authors
          WHERE name LIKE ? ESCAPE '\\'
             OR name_original LIKE ? ESCAPE '\\'
             OR orcid = ?
          ORDER BY name COLLATE NOCASE
          LIMIT ?"
    ))
    .bind(&like)
    .bind(&like)
    .bind(query.trim())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    for a in rows.iter_mut() {
        a.identifiers = load_identifiers(pool, a.id).await?;
    }
    Ok(rows)
}

/// 指定著者の identifiers を一括取得（scheme 昇順）。
async fn load_identifiers(
    pool: &SqlitePool,
    author_id: i64,
) -> Result<Vec<AuthorIdentifier>, sqlx::Error> {
    sqlx::query_as(
        "SELECT author_id, scheme, value, url
           FROM author_identifiers
          WHERE author_id = ?
          ORDER BY scheme",
    )
    .bind(author_id)
    .fetch_all(pool)
    .await
}

/// 入力された著者を ORCID → 正規化 name → INSERT の順で照合し、Author を返す。
///
/// トランザクション内で呼ぶ前提（entry 作成/更新の一部）。identifiers のフィールドは
/// **入力のものをそのまま返さない**（既存著者をヒットさせた場合は DB 上の状態を返す）。
/// 既存著者の identifiers を加筆したい場合は M7 の `update_author` を経由する。
pub(crate) async fn get_or_create_author(
    tx: &mut Transaction<'_, Sqlite>,
    input: &AuthorInput,
) -> Result<Author, sqlx::Error> {
    // ① ORCID 照合（authors.orcid 列 と author_identifiers の両方を見る）
    if let Some(orcid) = trimmed(&input.orcid) {
        if let Some(a) = find_by_orcid_in_tx(tx, &orcid).await? {
            return Ok(a);
        }
    }

    // ② 正規化 name 照合（NFKC + lowercase）
    //    SQLite では NFKC 関数が無いので全件比較する。個人ライブラリ規模では十分。
    //    将来 authors.normalized_name 列を持たせて O(1) lookup に置き換える余地あり。
    let norm = normalize_name(&input.name);
    if !norm.is_empty() {
        if let Some(a) = find_by_normalized_name_in_tx(tx, &norm).await? {
            return Ok(a);
        }
    }

    // ③ INSERT
    insert_author(tx, input).await
}

fn trimmed(opt: &Option<String>) -> Option<String> {
    opt.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

async fn find_by_orcid_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    orcid: &str,
) -> Result<Option<Author>, sqlx::Error> {
    // authors.orcid 列を優先（互換維持運用）。無ければ author_identifiers 経由でも探す。
    let mut author: Option<Author> = sqlx::query_as(&format!(
        "SELECT {AUTHOR_COLUMNS} FROM authors WHERE orcid = ?"
    ))
    .bind(orcid)
    .fetch_optional(&mut **tx)
    .await?;

    if author.is_none() {
        author = sqlx::query_as(&format!(
            "SELECT {AUTHOR_COLUMNS}
               FROM authors a
               JOIN author_identifiers ai ON ai.author_id = a.id
              WHERE ai.scheme = 'orcid' AND ai.value = ?"
        ))
        .bind(orcid)
        .fetch_optional(&mut **tx)
        .await?;
    }

    if let Some(a) = author.as_mut() {
        a.identifiers = load_identifiers_in_tx(tx, a.id).await?;
    }
    Ok(author)
}

async fn find_by_normalized_name_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    norm: &str,
) -> Result<Option<Author>, sqlx::Error> {
    // 全件取得 → Rust 側で normalize_name 一致比較。
    // 著者数 N に対して O(N) だが、INSERT 直前の 1 回だけなので個人規模では許容。
    let rows: Vec<Author> = sqlx::query_as(&format!("SELECT {AUTHOR_COLUMNS} FROM authors"))
        .fetch_all(&mut **tx)
        .await?;

    for mut a in rows {
        if normalize_name(&a.name) == norm {
            a.identifiers = load_identifiers_in_tx(tx, a.id).await?;
            return Ok(Some(a));
        }
    }
    Ok(None)
}

async fn load_identifiers_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    author_id: i64,
) -> Result<Vec<AuthorIdentifier>, sqlx::Error> {
    sqlx::query_as(
        "SELECT author_id, scheme, value, url
           FROM author_identifiers
          WHERE author_id = ?
          ORDER BY scheme",
    )
    .bind(author_id)
    .fetch_all(&mut **tx)
    .await
}

async fn insert_author(
    tx: &mut Transaction<'_, Sqlite>,
    input: &AuthorInput,
) -> Result<Author, sqlx::Error> {
    let orcid = trimmed(&input.orcid);

    let result = sqlx::query(
        "INSERT INTO authors (
            name,
            given_name, middle_name, family_name, suffix, name_particle,
            name_original, given_name_original, family_name_original, original_script,
            reading_family, reading_given,
            is_organization,
            email, homepage_url, notes,
            orcid, updated_at
         ) VALUES (?, ?,?,?,?,?, ?,?,?,?, ?,?, ?, ?,?,?, ?, datetime('now'))",
    )
    .bind(&input.name)
    .bind(&input.given_name)
    .bind(&input.middle_name)
    .bind(&input.family_name)
    .bind(&input.suffix)
    .bind(&input.name_particle)
    .bind(&input.name_original)
    .bind(&input.given_name_original)
    .bind(&input.family_name_original)
    .bind(&input.original_script)
    .bind(&input.reading_family)
    .bind(&input.reading_given)
    .bind(input.is_organization)
    .bind(&input.email)
    .bind(&input.homepage_url)
    .bind(&input.notes)
    .bind(&orcid)
    .execute(&mut **tx)
    .await?;
    let id = result.last_insert_rowid();

    // orcid を authors 列に書いたら、author_identifiers にも同じ値を併記する
    // （v0.3.0 の互換運用）。UNIQUE 違反は他著者と衝突した場合のみで、その時は
    // ORCID 照合で先にヒットしているはず → 通常は起きない。
    if let Some(o) = orcid.as_deref() {
        sqlx::query(
            "INSERT INTO author_identifiers (author_id, scheme, value)
             VALUES (?, 'orcid', ?)
             ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(o)
        .execute(&mut **tx)
        .await?;
    }

    // 挿入後に再フェッチして DEFAULT / generated 値を含む正しい Author を返す
    let mut inserted: Author = sqlx::query_as(&format!(
        "SELECT {AUTHOR_COLUMNS} FROM authors WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(&mut **tx)
    .await?;
    inserted.identifiers = load_identifiers_in_tx(tx, id).await?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    // ── normalize_name (pure) ────────────────────────────────────────────

    #[test]
    fn trims_whitespace() {
        assert_eq!(normalize_name("  Seki  "), "seki");
    }

    #[test]
    fn lowercases_ascii() {
        assert_eq!(normalize_name("Motoki Seki"), "motoki seki");
    }

    #[test]
    fn nfkc_folds_fullwidth_to_halfwidth() {
        assert_eq!(normalize_name("ＳＥＫＩ"), "seki");
    }

    #[test]
    fn nfkc_composes_decomposed_chars() {
        let decomposed = "Cafe\u{0301}";
        let composed = "Café";
        assert_eq!(normalize_name(decomposed), normalize_name(composed));
    }

    #[test]
    fn preserves_cjk_ideographs() {
        assert_eq!(normalize_name("関 茂樹"), "関 茂樹");
    }

    #[test]
    fn empty_input_yields_empty() {
        assert_eq!(normalize_name(""), "");
        assert_eq!(normalize_name("   "), "");
    }

    // ── get_or_create_author (§8.2) ─────────────────────────────────────

    fn input(name: &str) -> AuthorInput {
        AuthorInput {
            name: name.to_string(),
            ..Default::default()
        }
    }

    fn input_with_orcid(name: &str, orcid: &str) -> AuthorInput {
        AuthorInput {
            name: name.to_string(),
            orcid: Some(orcid.to_string()),
            ..Default::default()
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_or_create_author_matches_by_orcid(pool: SqlitePool) {
        // 先に「関 茂樹」を ORCID 付きで登録
        let mut tx = pool.begin().await.unwrap();
        let first = get_or_create_author(
            &mut tx,
            &input_with_orcid("関 茂樹", "0000-0002-1825-0097"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        // 同じ ORCID で別表記 "Seki M." を投げても既存 id が返るべき
        let mut tx = pool.begin().await.unwrap();
        let second = get_or_create_author(
            &mut tx,
            &input_with_orcid("Seki M.", "0000-0002-1825-0097"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(first.id, second.id, "ORCID 一致で既存著者にヒットすべき");
        // 名前は既存 (関 茂樹) のまま、Seki M. では上書きしない
        assert_eq!(second.name, "関 茂樹");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_or_create_author_matches_via_author_identifiers_table(pool: SqlitePool) {
        // 既存著者を ORCID なしで作り、後から author_identifiers 経由で
        // ORCID をぶら下げたケース（M7 で起こるシナリオの先取り検証）。
        let mut tx = pool.begin().await.unwrap();
        let first = get_or_create_author(&mut tx, &input("Motoki Seki"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO author_identifiers (author_id, scheme, value) VALUES (?, 'orcid', ?)",
        )
        .bind(first.id)
        .bind("0000-0002-1825-0097")
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();

        // 別表記 + 同 ORCID で照合 → author_identifiers 経由でヒット
        let mut tx = pool.begin().await.unwrap();
        let second = get_or_create_author(
            &mut tx,
            &input_with_orcid("M. Seki", "0000-0002-1825-0097"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(first.id, second.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_or_create_author_matches_by_nfkc_lowercase(pool: SqlitePool) {
        let mut tx = pool.begin().await.unwrap();
        let first = get_or_create_author(&mut tx, &input("Alice Smith"))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // 全角 + 大文字（NFKC + lowercase で同一になる表記）
        let mut tx = pool.begin().await.unwrap();
        let second = get_or_create_author(&mut tx, &input("ＡＬＩＣＥ ＳＭＩＴＨ"))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(first.id, second.id, "全角/大小同一は同じ著者にヒット");

        // 前後空白だけが違う表記
        let mut tx = pool.begin().await.unwrap();
        let third = get_or_create_author(&mut tx, &input("  alice smith  "))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(first.id, third.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_or_create_author_inserts_when_no_match(pool: SqlitePool) {
        let mut tx = pool.begin().await.unwrap();
        let a = get_or_create_author(&mut tx, &input("Alice Smith"))
            .await
            .unwrap();
        let b = get_or_create_author(&mut tx, &input("Bob Jones")).await.unwrap();
        tx.commit().await.unwrap();

        assert_ne!(a.id, b.id);
        // is_organization の DEFAULT 0 が反映されていること（型は bool）
        assert!(!a.is_organization);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_author_writes_orcid_to_both_places(pool: SqlitePool) {
        // 互換維持運用: 新規時は authors.orcid と author_identifiers 両方に書く
        let mut tx = pool.begin().await.unwrap();
        let a = get_or_create_author(
            &mut tx,
            &input_with_orcid("Cited One", "0000-0001-2345-6789"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(a.orcid.as_deref(), Some("0000-0001-2345-6789"));
        assert_eq!(a.identifiers.len(), 1);
        assert_eq!(a.identifiers[0].scheme, "orcid");
        assert_eq!(a.identifiers[0].value, "0000-0001-2345-6789");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn insert_author_sets_updated_at(pool: SqlitePool) {
        // M2 の単純な INSERT (name のみ) では updated_at が NULL になっていたが、
        // M3 では datetime('now') で必ず埋まる
        let mut tx = pool.begin().await.unwrap();
        let a = get_or_create_author(&mut tx, &input("Some One")).await.unwrap();
        tx.commit().await.unwrap();
        assert!(a.updated_at.is_some(), "updated_at must be filled on insert");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_author_returns_identifiers(pool: SqlitePool) {
        let mut tx = pool.begin().await.unwrap();
        let a = get_or_create_author(
            &mut tx,
            &input_with_orcid("Looked Up", "0000-0003-0000-0001"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let fetched = get_author(&pool, a.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, a.id);
        assert_eq!(fetched.identifiers.len(), 1);
        assert_eq!(fetched.identifiers[0].value, "0000-0003-0000-0001");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_authors_matches_name_substring(pool: SqlitePool) {
        let mut tx = pool.begin().await.unwrap();
        get_or_create_author(&mut tx, &input("Alice Smith")).await.unwrap();
        get_or_create_author(&mut tx, &input("Bob Jones")).await.unwrap();
        get_or_create_author(&mut tx, &input("Alice Brown")).await.unwrap();
        tx.commit().await.unwrap();

        let hits = search_authors(&pool, "Alice", 10).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|a| a.name.contains("Alice")));
    }
}
