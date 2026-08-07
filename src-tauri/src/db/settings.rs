use sqlx::SqlitePool;

/// settings テーブルの単純な key-value 取得。未設定なら None。
pub async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

/// upsert。空文字も「設定されている空文字」として保存する（呼び出し側で適宜 delete を使う）。
pub async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_setting(pool: &SqlitePool, key: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM settings WHERE key = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// BibTeX 同期先パスの設定キー。
pub const BIBTEX_SYNC_PATH_KEY: &str = "bibtex_sync_path";

/// BibTeX 出力（同期・エクスポート）で abstract / note フィールドを除外するフラグ
/// （"1" で除外）。普段使わない上に不正文字や別言語が混入しがちなため。
pub const BIBTEX_EXCLUDE_ABSTRACT_NOTE_KEY: &str = "bibtex.exclude_abstract_note";

pub const LLM_PROVIDER_KEY: &str = "llm.provider";
pub const LLM_MODEL_KEY: &str = "llm.model";
pub const LLM_SUMMARY_SOURCE_KEY: &str = "llm.summary_source";
pub const LLM_SUMMARY_PROMPT_KEY: &str = "llm.summary_prompt";

/// Chat のツール別自動承認ホワイトリスト（JSON: tool_name -> bool）。
pub const CHAT_TOOL_WHITELIST_KEY: &str = "chat.tool_whitelist";

/// 外部 MCP サーバー設定（Claude Desktop の mcpServers 互換 JSON）。
pub const MCP_SERVERS_KEY: &str = "mcp.servers";

/// LumenCite 自身を MCP サーバーとして公開する機能の有効フラグ（"1" で有効）。
pub const MCP_SERVER_ENABLED_KEY: &str = "mcp_server.enabled";
/// MCP サーバーのバインドポート（未設定なら `mcp_server::DEFAULT_PORT`）。
pub const MCP_SERVER_PORT_KEY: &str = "mcp_server.port";
/// MCP サーバー公開で write 系ツールを許可するフラグ（"1" で許可、既定 false）。
/// Phase 2。承認 UI が無いためサーバー側でこのゲートを enforce する。
pub const MCP_SERVER_WRITE_ENABLED_KEY: &str = "mcp_server.write_enabled";

/// Web クリッパー（ブラウザ拡張 → `POST /clipper`）の有効フラグ（"1" で有効、既定 off）。
/// `mcp_server.write_enabled` とは独立の同意面としてリクエスト毎に評価する。
pub const CLIPPER_ENABLED_KEY: &str = "clipper.enabled";

/// クリップ重複時に欠落（PDF / TeX ソース）を確認なしで自動補完するフラグ（"1" で自動、
/// 未設定なら初回確認）。判断は常にアプリ側で行い、拡張と AddSheet で共有する。
pub const CLIPPER_COMPLETE_MISSING_KEY: &str = "clipper.complete_missing";

/// OCR 用 LLM プロバイダ / モデル（未設定なら chat の provider / model にフォールバック）。
pub const LLM_OCR_PROVIDER_KEY: &str = "llm.ocr_provider";
pub const LLM_OCR_MODEL_KEY: &str = "llm.ocr_model";

/// v0.3.0 で entries_fts.authors_text の合成 SQL が name_original / reading_* も
/// 含む形に変わったため、既存ライブラリの FTS を 1 回だけ起動時に再構築するフラグ。
/// 値は "1"（再構築済み）のみで、未設定なら未実施扱い。
pub const FTS_AUTHORS_V030_REBUILT_KEY: &str = "fts.authors_v030_rebuilt";

/// PDF 全文の `fulltext` FTS5（trigram）逆索引を起動時に 1 回だけ再構築するフラグ。
/// 一部の既存ライブラリで逆索引が malformed になっており（アプリ内蔵の古い SQLite の
/// `integrity_check` では検出できないが、新しい SQLite では検出される）、全文検索が
/// 不正になり得るため、`INSERT INTO fulltext(fulltext) VALUES('rebuild')` で内容から
/// 索引を作り直す。値は "1"（再構築済み）のみで、未設定なら未実施扱い。
pub const FTS_FULLTEXT_REBUILT_KEY: &str = "fts.fulltext_rebuilt";

/// 直近のバックアップ成功時刻（RFC3339）。自動バックアップ（起動時 / 24h タイマー）が
/// 前回からの経過時間を見て間引くために使う。手動実行（`run_backup_now`）は間引かないが、
/// 成功時にはこの値を更新する。未設定なら「未実施」＝次の自動実行で走る。
pub const BACKUP_LAST_RUN_KEY: &str = "backup.last_run";

/// LCIR（機械可読中間形式）を使うか。**v1.0.0-p3 で既定 ON に反転した**（それまでは実験フラグで既定 off）。
///
/// 判定は **「`"0"` でなければ ON」**（`ingestion::lcir_enabled`）。`"0"` = 明示的に切った /
/// 未設定 = 一度も触っていない ＝ 既定 ON、を区別する。**書くのは `set_lcir_enabled` だけ**で
/// `"0"`/`"1"` しか書かず、migration に seed も無い ── この不変条件が崩れると
/// 「切ったのに戻る」か「新規ユーザーが OFF のまま」のどちらかになる。
///
/// このキーが許すのは①手動 build ②自動 build ③起動時バックフィル**まで**。
/// **arXiv からの e-print 自動取得は含まない**（[`LCIR_TEX_AUTOFETCH_ENABLED_KEY`] へ分離）。
pub const LCIR_ENABLED_KEY: &str = "lcir.enabled";

/// v1.0.0-p3: arXiv から e-print（TeX ソース）を**自動で**取得することへの同意フラグ
/// （"1" で有効）。`lcir.enabled` とは**独立の同意面**にする。
///
/// 分けた理由: `lcir.enabled` を既定 ON にすると、何も操作していないユーザーのクリップや
/// 論文追加のたびに**数 MB の外部ダウンロードが黙って始まる**。手元で LCIR を組むことと、
/// 外部サービスへ取りに行くこと（通信・相手への負荷）は同意の性質が違う
/// （`clipper.enabled` / `lcir.vision_alt_text.enabled` と同型）。
///
/// 未設定のときの既定は **この版より前に `lcir.enabled` を明示 ON にしていたか**
/// （`ingestion::tex_autofetch_default`）。既存ユーザーの挙動を無言で退行させないため。
/// 起動時に 1 回 `ingestion::backfill_tex_autofetch_consent` が明示値へ確定させる。
pub const LCIR_TEX_AUTOFETCH_ENABLED_KEY: &str = "lcir.tex_autofetch.enabled";

/// LCIR Phase 8c: 図の代替テキストを LLM Vision で生成することへの同意フラグ（"1" で有効・
/// 既定 off）。`lcir.enabled` とは**独立の同意面**にする — 画像 1 枚ごとに外部 API へ送信して
/// 課金が発生するため、LCIR の実験フラグ ON だけで暗黙に許可しない（`clipper.enabled` と同型）。
pub const LCIR_VISION_ALT_TEXT_ENABLED_KEY: &str = "lcir.vision_alt_text.enabled";

/// v1.0.0-p1: 添付ごとの「全文索引の出どころ」を記録するキーの接頭辞
/// （`fulltext.source.<attachment_id>` = `"lcir"` / `"ocr"`）。
///
/// `fulltext` は FTS5 仮想表なので provenance 列を持てず（`virtual tables may not be altered`）、
/// 側表を足すと dev 起動の瞬間に共有実 DB へ migration が適用されて配布版が起動不能になる
/// （`NewerSchema`）。そこで **DDL を伴わない settings KV** に置く。
/// 記録が無い = pdf_extract 由来 or 未索引（既定なので書かない）。
pub const FULLTEXT_SOURCE_KEY_PREFIX: &str = "fulltext.source.";

/// v1.0.0-p1: 既存ライブラリの `fulltext` を LCIR の page ノードから 1 回だけ再導出したか
/// （`FTS_FULLTEXT_REBUILT_KEY` と同型の一度きりフラグ）。
///
/// build 経路に派生を配線しても、既に完了版がある添付には二度と build が走らないので
/// （`attachments_without_completed_lcir` は完了版のある添付を除外し、
/// `attachments_with_outdated_lcir` は版 bump 無しでは 0 件）、この経路が無いと
/// 既存 138 添付に派生索引が永久に届かない。値は "1" のみ。
pub const FTS_FULLTEXT_LCIR_DERIVED_KEY: &str = "fts.fulltext_lcir_derived";

/// v1.0.0-p2: 起動時 LCIR バックフィルが**最後に 1 件以上着手した**時刻（RFC3339）。
///
/// `BACKUP_LAST_RUN_KEY` と同型の間引きキーで、`FTS_FULLTEXT_LCIR_DERIVED_KEY` のような
/// boolean の一度きりフラグにはしない ── バックフィルは 1 ランの予算で途中打ち切りするうえ、
/// 壊れた PDF が 1 本あるだけで「完了」条件を満たさなくなるので、boolean だとフラグが永久に
/// 立たないか、逆に残件があるのに「完了」で立つ。**「やり切ったか」は残件数 0 で毎回判定する**
/// （状態を二重に持たない）。**対象 0 件の回と LCIR OFF の回は書かない。**
pub const LCIR_BACKFILL_LAST_RUN_KEY: &str = "lcir.backfill.last_run";

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn get_setting_returns_none_for_unset_key(pool: SqlitePool) {
        let v = get_setting(&pool, "missing").await.unwrap();
        assert_eq!(v, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_then_get_roundtrips(pool: SqlitePool) {
        set_setting(&pool, "k1", "hello").await.unwrap();
        let v = get_setting(&pool, "k1").await.unwrap();
        assert_eq!(v.as_deref(), Some("hello"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_setting_upserts_existing_key(pool: SqlitePool) {
        set_setting(&pool, "k1", "first").await.unwrap();
        set_setting(&pool, "k1", "second").await.unwrap();
        let v = get_setting(&pool, "k1").await.unwrap();
        assert_eq!(v.as_deref(), Some("second"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_setting_removes_key(pool: SqlitePool) {
        set_setting(&pool, "k1", "v").await.unwrap();
        delete_setting(&pool, "k1").await.unwrap();
        let v = get_setting(&pool, "k1").await.unwrap();
        assert_eq!(v, None);
    }
}
