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

use crate::db::entries::sync_entries_fts;
use crate::models::{Author, AuthorIdentifier, AuthorIdentifierInput, AuthorInput, EntryInput};

/// `EntryInput` から create/update に渡す著者リストを取り出す。
///
/// 優先順位:
/// 1. `input.authors` (`Some`) — 構造化された AuthorInput をそのまま使う
///    （bibtex の `{...}` literal / metadata の ORCID 等を伝搬する経路）
/// 2. それ以外 — `input.author_names` を AuthorInput { name } にフォールバック
///    （フロント既存のペイロード / 単純なテキスト入力）
pub(crate) fn author_inputs_from(input: &EntryInput) -> Vec<AuthorInput> {
    if let Some(rich) = input.authors.as_ref() {
        rich.clone()
    } else {
        input
            .author_names
            .iter()
            .map(|name| AuthorInput {
                name: name.clone(),
                ..Default::default()
            })
            .collect()
    }
}

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
    let like = crate::db::entries::like_pattern(query);
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

// ── M7: 編集系 API ──────────────────────────────────────────────────────

/// 著者の全フィールドを `input` で差し替え、関連 entry の `entries_fts` を再同期する。
///
/// - `authors` の全列を UPDATE（`updated_at = datetime('now')`）
/// - `author_identifiers` は **DELETE → INSERT で総差し替え**（1 著者 10 件程度の前提で素朴）
/// - `input.orcid` が指定されているのに `input.identifiers` に scheme='orcid' が無ければ
///   暗黙で補う（authors.orcid 列との二重書き運用を維持）
/// - 当該著者を含む全 entry に対して `sync_entries_fts` を実行（authors_text 反映）
///
/// 同一 (scheme, value) が他著者で使われている identifier を渡すと UNIQUE 制約で失敗する。
pub async fn update_author(
    pool: &SqlitePool,
    id: i64,
    input: &AuthorInput,
) -> Result<Author, sqlx::Error> {
    let orcid = trimmed(&input.orcid);

    let mut tx = pool.begin().await?;

    let rows_affected = sqlx::query(
        "UPDATE authors SET
            name = ?,
            given_name = ?, middle_name = ?, family_name = ?, suffix = ?, name_particle = ?,
            name_original = ?, given_name_original = ?, family_name_original = ?, original_script = ?,
            reading_family = ?, reading_given = ?,
            is_organization = ?,
            email = ?, homepage_url = ?, notes = ?,
            orcid = ?,
            updated_at = datetime('now')
         WHERE id = ?",
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
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Err(sqlx::Error::RowNotFound);
    }

    // identifiers を DELETE → INSERT で差し替え
    sqlx::query("DELETE FROM author_identifiers WHERE author_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;

    // input.identifiers をそのまま投入
    let mut wrote_orcid_via_identifiers = false;
    for ident in &input.identifiers {
        let scheme = ident.scheme.trim();
        let value = ident.value.trim();
        if scheme.is_empty() || value.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO author_identifiers (author_id, scheme, value, url)
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(scheme)
        .bind(value)
        .bind(ident.url.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .execute(&mut *tx)
        .await?;
        if scheme == "orcid" {
            wrote_orcid_via_identifiers = true;
        }
    }

    // authors.orcid が立っているのに identifiers に 'orcid' が無いケースを暗黙補完
    if !wrote_orcid_via_identifiers {
        if let Some(o) = orcid.as_deref() {
            sqlx::query(
                "INSERT INTO author_identifiers (author_id, scheme, value)
                 VALUES (?, 'orcid', ?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(o)
            .execute(&mut *tx)
            .await?;
        }
    }

    // 関連 entry の FTS を再構築
    let entry_ids: Vec<i64> =
        sqlx::query_scalar("SELECT DISTINCT entry_id FROM entry_authors WHERE author_id = ?")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    for eid in entry_ids {
        sync_entries_fts(&mut tx, eid).await?;
    }

    // 再フェッチして返す
    let mut updated: Author = sqlx::query_as(&format!(
        "SELECT {AUTHOR_COLUMNS} FROM authors WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    updated.identifiers = load_identifiers_in_tx(&mut tx, id).await?;

    tx.commit().await?;
    Ok(updated)
}

/// 2 著者を統合する。`from_id` を `into_id` に集約し、`from_id` を削除する。
///
/// - `entry_authors`: `from_id` の行を `into_id` へ付け替え。
///   両者が同じ entry に既にぶら下がっている場合は付け替え不能（PRIMARY KEY 衝突）なので
///   `from_id` 側の行を素直に削除（into の position は維持）
/// - `author_identifiers`: `from_id` の identifier を `into_id` へ移す。
///   `(author_id, scheme)` PRIMARY KEY は ON CONFLICT DO NOTHING で **into 側を優先**。
///   `(scheme, value)` UNIQUE は from→into 移動先で衝突する可能性があるが、
///   その場合も DO NOTHING で skip（from のみが持つ identifier だけが残る）
/// - 関連 entry すべての `entries_fts` を再構築（into の author 表記が反映される）
/// - 最後に `from_id` を DELETE
pub async fn merge_authors(
    pool: &SqlitePool,
    from_id: i64,
    into_id: i64,
) -> Result<(), sqlx::Error> {
    if from_id == into_id {
        return Ok(()); // no-op
    }

    let mut tx = pool.begin().await?;

    // 影響を受ける entry を先に把握しておく（DELETE 後だと from 経由では辿れない）
    let entry_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT entry_id FROM entry_authors WHERE author_id IN (?, ?)",
    )
    .bind(from_id)
    .bind(into_id)
    .fetch_all(&mut *tx)
    .await?;

    // entry_authors 付け替え（into が既にいる行は from を捨てる）
    sqlx::query(
        "DELETE FROM entry_authors
         WHERE author_id = ?
           AND entry_id IN (SELECT entry_id FROM entry_authors WHERE author_id = ?)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE entry_authors SET author_id = ? WHERE author_id = ?")
        .bind(into_id)
        .bind(from_id)
        .execute(&mut *tx)
        .await?;

    // identifiers を移動。INSERT…SELECT 方式は (scheme, value) UNIQUE INDEX が
    // 「from にも into にも同じ値が無い限り保たれている」前提を活かせず、from の
    // 既存行と (scheme, value) で衝突して全行 skip されてしまう。代わりに:
    //   ① into 側で既に持っている scheme は from 側から DELETE（into を優先）
    //   ② 残りは UPDATE で from → into に付け替え（PK 衝突解消済み、
    //      (scheme, value) UNIQUE は同値が別著者にあり得ないので無事）
    sqlx::query(
        "DELETE FROM author_identifiers
          WHERE author_id = ?
            AND scheme IN (SELECT scheme FROM author_identifiers WHERE author_id = ?)",
    )
    .bind(from_id)
    .bind(into_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE author_identifiers SET author_id = ? WHERE author_id = ?")
        .bind(into_id)
        .bind(from_id)
        .execute(&mut *tx)
        .await?;

    // from を削除（この時点で entry_authors / author_identifiers が空なので RESTRICT に触れない）
    let deleted = sqlx::query("DELETE FROM authors WHERE id = ?")
        .bind(from_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(sqlx::Error::RowNotFound);
    }

    // 関連 entry の FTS を再構築（into の表記が authors_text へ反映される）
    for eid in entry_ids {
        sync_entries_fts(&mut tx, eid).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 著者に identifier を 1 件追加する（scheme='orcid' の場合は authors.orcid も同期する）。
/// `(author_id, scheme)` PRIMARY KEY 衝突時は **既存値を上書き** する upsert 動作。
pub async fn add_author_identifier(
    pool: &SqlitePool,
    author_id: i64,
    input: &AuthorIdentifierInput,
) -> Result<(), sqlx::Error> {
    let scheme = input.scheme.trim();
    let value = input.value.trim();
    if scheme.is_empty() || value.is_empty() {
        return Err(sqlx::Error::Protocol(
            "scheme と value は必須".to_string(),
        ));
    }
    let url = input
        .url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO author_identifiers (author_id, scheme, value, url)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(author_id, scheme) DO UPDATE SET
            value = excluded.value,
            url   = excluded.url",
    )
    .bind(author_id)
    .bind(scheme)
    .bind(value)
    .bind(url)
    .execute(&mut *tx)
    .await?;

    if scheme == "orcid" {
        // authors.orcid 列とも同期する（v0.3.0 互換運用）
        sqlx::query(
            "UPDATE authors SET orcid = ?, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(value)
        .bind(author_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// 著者から identifier を削除する（scheme='orcid' の削除は authors.orcid もクリアする）。
pub async fn delete_author_identifier(
    pool: &SqlitePool,
    author_id: i64,
    scheme: &str,
) -> Result<(), sqlx::Error> {
    let scheme = scheme.trim();
    if scheme.is_empty() {
        return Err(sqlx::Error::Protocol("scheme は必須".to_string()));
    }

    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM author_identifiers WHERE author_id = ? AND scheme = ?")
        .bind(author_id)
        .bind(scheme)
        .execute(&mut *tx)
        .await?;

    if scheme == "orcid" {
        sqlx::query(
            "UPDATE authors SET orcid = NULL, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(author_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
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
        assert_eq!(normalize_name("関 元樹"), "関 元樹");
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
        // 先に「関 元樹」を ORCID 付きで登録
        let mut tx = pool.begin().await.unwrap();
        let first = get_or_create_author(
            &mut tx,
            &input_with_orcid("関 元樹", "0000-0002-1825-0097"),
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
        // 名前は既存 (関 元樹) のまま、Seki M. では上書きしない
        assert_eq!(second.name, "関 元樹");
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

    // ── M7: update_author / merge_authors / *_identifier (§8.5 含む) ─────────

    use crate::db::entries::{create_entry, search_entries};
    use crate::models::{AuthorIdentifierInput, EntryInput};

    async fn make_author(pool: &SqlitePool, name: &str) -> i64 {
        let mut tx = pool.begin().await.unwrap();
        let a = get_or_create_author(&mut tx, &input(name)).await.unwrap();
        tx.commit().await.unwrap();
        a.id
    }

    fn full_input(name: &str) -> AuthorInput {
        AuthorInput {
            name: name.to_string(),
            given_name: Some("Given".to_string()),
            family_name: Some("Family".to_string()),
            name_original: Some("関 元樹".to_string()),
            original_script: Some("Hani".to_string()),
            reading_family: Some("せき".to_string()),
            reading_given: Some("もとき".to_string()),
            email: Some("x@example.com".to_string()),
            ..Default::default()
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_author_replaces_fields_and_sets_updated_at(pool: SqlitePool) {
        let id = make_author(&pool, "Original Name").await;

        let updated = update_author(&pool, id, &full_input("New Name")).await.unwrap();
        assert_eq!(updated.id, id);
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.given_name.as_deref(), Some("Given"));
        assert_eq!(updated.family_name.as_deref(), Some("Family"));
        assert_eq!(updated.name_original.as_deref(), Some("関 元樹"));
        assert_eq!(updated.reading_family.as_deref(), Some("せき"));
        assert_eq!(updated.email.as_deref(), Some("x@example.com"));
        assert!(updated.updated_at.is_some());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_author_returns_row_not_found_for_missing_id(pool: SqlitePool) {
        let err = update_author(&pool, 9999, &input("X")).await.unwrap_err();
        assert!(matches!(err, sqlx::Error::RowNotFound));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_author_replaces_identifiers_diff_style(pool: SqlitePool) {
        let id = make_author(&pool, "X").await;
        // 先に 3 件
        let mut start = AuthorInput {
            name: "X".to_string(),
            identifiers: vec![
                AuthorIdentifierInput {
                    scheme: "dblp".to_string(),
                    value: "12/1".to_string(),
                    url: None,
                },
                AuthorIdentifierInput {
                    scheme: "scopus".to_string(),
                    value: "55".to_string(),
                    url: None,
                },
                AuthorIdentifierInput {
                    scheme: "wikidata".to_string(),
                    value: "Q1".to_string(),
                    url: None,
                },
            ],
            ..Default::default()
        };
        let after_first = update_author(&pool, id, &start).await.unwrap();
        assert_eq!(after_first.identifiers.len(), 3);

        // 1 件残し、2 件入れ替え（dblp は更新、scopus は削除、新規 viaf を追加）
        start.identifiers = vec![
            AuthorIdentifierInput {
                scheme: "dblp".to_string(),
                value: "12/9".to_string(),
                url: Some("https://dblp.org/x".to_string()),
            },
            AuthorIdentifierInput {
                scheme: "viaf".to_string(),
                value: "12345".to_string(),
                url: None,
            },
        ];
        let after_second = update_author(&pool, id, &start).await.unwrap();
        let by_scheme: std::collections::HashMap<&str, &AuthorIdentifier> =
            after_second.identifiers.iter().map(|i| (i.scheme.as_str(), i)).collect();
        assert_eq!(by_scheme.len(), 2);
        assert_eq!(by_scheme.get("dblp").unwrap().value, "12/9");
        assert_eq!(
            by_scheme.get("dblp").unwrap().url.as_deref(),
            Some("https://dblp.org/x")
        );
        assert_eq!(by_scheme.get("viaf").unwrap().value, "12345");
        assert!(!by_scheme.contains_key("scopus"));
        assert!(!by_scheme.contains_key("wikidata"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_author_orcid_writes_both_column_and_identifiers(pool: SqlitePool) {
        let id = make_author(&pool, "Cited").await;
        let inp = AuthorInput {
            name: "Cited".to_string(),
            orcid: Some("0000-0001-2345-6789".to_string()),
            // identifiers にも明示せず、暗黙補完を検証
            ..Default::default()
        };
        let updated = update_author(&pool, id, &inp).await.unwrap();
        assert_eq!(updated.orcid.as_deref(), Some("0000-0001-2345-6789"));
        let orcid_row = updated
            .identifiers
            .iter()
            .find(|i| i.scheme == "orcid")
            .expect("scheme='orcid' が identifiers にも入っていること");
        assert_eq!(orcid_row.value, "0000-0001-2345-6789");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_author_resyncs_fts_for_linked_entries(pool: SqlitePool) {
        // §8.4 で延期した 2 つ目のテスト
        let entry = create_entry(
            &pool,
            &EntryInput {
                title: "Random Title".to_string(),
                entry_type: "article".to_string(),
                author_names: vec!["Seki".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let author_id: i64 = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'Seki'")
            .fetch_one(&pool)
            .await
            .unwrap();

        // 漢字 / かなを update_author で付与（FTS 再同期込み）
        let updated = AuthorInput {
            name: "Seki".to_string(),
            name_original: Some("関 元樹".to_string()),
            reading_family: Some("せき".to_string()),
            reading_given: Some("もとき".to_string()),
            ..Default::default()
        };
        update_author(&pool, author_id, &updated).await.unwrap();

        // rebuild なしで漢字 / かながヒットする（=update_author が FTS を再同期した）
        for q in ["関", "せき"] {
            let hits = search_entries(&pool, q, None, None).await.unwrap();
            assert_eq!(hits.len(), 1, "{q} should hit after update_author");
            assert_eq!(hits[0].id, entry.id);
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_authors_moves_entry_links(pool: SqlitePool) {
        // 2 entry を別々の author にぶら下げる
        let e1 = create_entry(
            &pool,
            &EntryInput {
                title: "E1".to_string(),
                entry_type: "article".to_string(),
                author_names: vec!["From Name".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let e2 = create_entry(
            &pool,
            &EntryInput {
                title: "E2".to_string(),
                entry_type: "article".to_string(),
                author_names: vec!["Into Name".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let from_id: i64 = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'From Name'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let into_id: i64 = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'Into Name'")
            .fetch_one(&pool)
            .await
            .unwrap();

        merge_authors(&pool, from_id, into_id).await.unwrap();

        // from は削除
        let from_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE id = ?")
                .bind(from_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(from_count, 0);

        // 2 entry とも into に紐付くようになっている
        for eid in [e1.id, e2.id] {
            let aid: i64 = sqlx::query_scalar(
                "SELECT author_id FROM entry_authors WHERE entry_id = ?",
            )
            .bind(eid)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(aid, into_id);
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_authors_dedups_when_both_on_same_entry(pool: SqlitePool) {
        // 同じ entry に from と into 両方が co-author としているケース
        let e = create_entry(
            &pool,
            &EntryInput {
                title: "Co-authored".to_string(),
                entry_type: "article".to_string(),
                author_names: vec!["A".to_string(), "B".to_string()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let from_id: i64 = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'A'")
            .fetch_one(&pool).await.unwrap();
        let into_id: i64 = sqlx::query_scalar("SELECT id FROM authors WHERE name = 'B'")
            .fetch_one(&pool).await.unwrap();

        merge_authors(&pool, from_id, into_id).await.unwrap();

        // entry には into 1 行のみ残る
        let rows: Vec<i64> = sqlx::query_scalar(
            "SELECT author_id FROM entry_authors WHERE entry_id = ?",
        )
        .bind(e.id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows, vec![into_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_authors_resolves_identifier_conflict(pool: SqlitePool) {
        // 両者に同 scheme の identifier があったら into 側を残す
        let from_id = make_author(&pool, "From").await;
        let into_id = make_author(&pool, "Into").await;
        sqlx::query(
            "INSERT INTO author_identifiers (author_id, scheme, value)
             VALUES (?, 'dblp', 'from/123'), (?, 'dblp', 'into/456')",
        )
        .bind(from_id)
        .bind(into_id)
        .execute(&pool)
        .await
        .unwrap();
        // from にだけ別 scheme も持たせる（こちらは into に移動する）
        sqlx::query(
            "INSERT INTO author_identifiers (author_id, scheme, value) VALUES (?, 'wikidata', 'Q1')",
        )
        .bind(from_id)
        .execute(&pool)
        .await
        .unwrap();

        merge_authors(&pool, from_id, into_id).await.unwrap();

        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT scheme, value FROM author_identifiers WHERE author_id = ? ORDER BY scheme",
        )
        .bind(into_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            rows,
            vec![
                ("dblp".to_string(), "into/456".to_string()),    // into 側を保持
                ("wikidata".to_string(), "Q1".to_string()),       // from 由来は移動
            ]
        );
        // from の identifiers は空
        let from_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM author_identifiers WHERE author_id = ?",
        )
        .bind(from_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(from_count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_authors_into_self_is_noop(pool: SqlitePool) {
        let id = make_author(&pool, "Solo").await;
        merge_authors(&pool, id, id).await.unwrap(); // panic しない
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM authors WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn merge_authors_not_found_for_unknown_from(pool: SqlitePool) {
        let into_id = make_author(&pool, "Into").await;
        let err = merge_authors(&pool, 9999, into_id).await.unwrap_err();
        assert!(matches!(err, sqlx::Error::RowNotFound));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_author_identifier_inserts_then_upserts(pool: SqlitePool) {
        let id = make_author(&pool, "X").await;
        add_author_identifier(
            &pool,
            id,
            &AuthorIdentifierInput {
                scheme: "dblp".to_string(),
                value: "v1".to_string(),
                url: None,
            },
        )
        .await
        .unwrap();
        // 上書き
        add_author_identifier(
            &pool,
            id,
            &AuthorIdentifierInput {
                scheme: "dblp".to_string(),
                value: "v2".to_string(),
                url: Some("https://dblp.org/x".to_string()),
            },
        )
        .await
        .unwrap();
        let row: (String, Option<String>) = sqlx::query_as(
            "SELECT value, url FROM author_identifiers WHERE author_id = ? AND scheme = 'dblp'",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.0, "v2");
        assert_eq!(row.1.as_deref(), Some("https://dblp.org/x"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_author_identifier_orcid_syncs_authors_column(pool: SqlitePool) {
        let id = make_author(&pool, "X").await;
        add_author_identifier(
            &pool,
            id,
            &AuthorIdentifierInput {
                scheme: "orcid".to_string(),
                value: "0000-1111-2222-3333".to_string(),
                url: None,
            },
        )
        .await
        .unwrap();
        let orcid: Option<String> =
            sqlx::query_scalar("SELECT orcid FROM authors WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(orcid.as_deref(), Some("0000-1111-2222-3333"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn add_author_identifier_rejects_empty(pool: SqlitePool) {
        let id = make_author(&pool, "X").await;
        let err = add_author_identifier(
            &pool,
            id,
            &AuthorIdentifierInput {
                scheme: "".to_string(),
                value: "x".to_string(),
                url: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, sqlx::Error::Protocol(_)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_author_identifier_removes_only_that_scheme(pool: SqlitePool) {
        let id = make_author(&pool, "X").await;
        sqlx::query(
            "INSERT INTO author_identifiers (author_id, scheme, value)
             VALUES (?, 'dblp', 'a'), (?, 'scopus', 'b')",
        )
        .bind(id)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();

        delete_author_identifier(&pool, id, "dblp").await.unwrap();

        let remaining: Vec<String> = sqlx::query_scalar(
            "SELECT scheme FROM author_identifiers WHERE author_id = ? ORDER BY scheme",
        )
        .bind(id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(remaining, vec!["scopus".to_string()]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_author_identifier_orcid_clears_authors_column(pool: SqlitePool) {
        let id = make_author(&pool, "X").await;
        sqlx::query("UPDATE authors SET orcid = '0000-X' WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO author_identifiers (author_id, scheme, value) VALUES (?, 'orcid', '0000-X')")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();

        delete_author_identifier(&pool, id, "orcid").await.unwrap();

        let orcid: Option<String> =
            sqlx::query_scalar("SELECT orcid FROM authors WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(orcid.is_none(), "authors.orcid もクリアされること");
    }
}
