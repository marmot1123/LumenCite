//! v0.7.0 CLI — GUI/Tauri を起動せず、LumenCite ライブラリを **読取専用**で照会する
//! サブコマンド群。第一の対象は「AI エージェント × LaTeX 執筆」ワークフロー
//! （`\cite` キー → `refs.bib` 生成）とシェルスクリプト連携。
//!
//! 起動: `main.rs` が `argv[1]` を [`is_cli_invocation`] で判定し、既知のサブコマンドなら
//! [`run`] を呼ぶ（`--mcp-stdio` shim と同型のディスパッチ）。引数なしは従来どおり GUI。
//!
//! バックエンド: v0.7.0 のコマンドはすべて読取専用のため、原則「読みは自由」に従い
//! SQLite を直接開く。全コネクションに `PRAGMA query_only = ON` を適用し、CLI が絶対に
//! 書き込まないことを構造的に保証する（書き込みガード）。GUI 起動中でも WAL の並行
//! リーダーとして安全に共存し、停止中でも動く。HTTP プロキシ経由のハイブリッド C 本実装と
//! 書き込みコマンドは、書き込みガードを厳格化した上で次版で追加する。

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::db;
use crate::models::{EntryFilter, TagMatch};

/// Tauri identifier（`app_data_dir` の末尾セグメント）。keychain の SERVICE と一致。
const APP_IDENTIFIER: &str = "com.lumencite.app";
const DB_FILENAME: &str = "lumencite.db";
/// DB パスの明示上書き（テスト・非標準配置向け）。
const DB_PATH_ENV: &str = "LUMENCITE_DB_PATH";

/// トップレベルで指定されたとき CLI ヘルプ/バージョンを表示すべきフラグ。
const HELP_VERSION_FLAGS: &[&str] = &["--help", "-h", "--version", "-V"];

/// `argv` が CLI 起動か（GUI 起動と区別するゲート）。
/// `args` は `std::env::args().collect()` を想定（`args[0]` は実行ファイル名）。
///
/// 判定: `argv[1]` が
/// - ヘルプ/バージョンフラグ → CLI（clap がヘルプを整形）
/// - `-` で始まらない語（= サブコマンド候補） → CLI。未知の語でも clap が
///   "unrecognized subcommand" を出せるよう回す（GUI へ落として無言でウィンドウを開くより親切）。
///
/// GUI は引数なし、または macOS の `-psn_xxxx` のような `-` 始まりで起動されるため、
/// それらは GUI（`false`）へ落とす。`--mcp-stdio` は呼び出し元がこの関数より前に処理する。
/// 本アプリはファイル関連付け / deep-link を持たず GUI は argv を消費しないため、
/// bare word を CLI とみなして安全。
pub fn is_cli_invocation(args: &[String]) -> bool {
    match args.get(1).map(String::as_str) {
        Some(a) if HELP_VERSION_FLAGS.contains(&a) => true,
        Some(a) => !a.starts_with('-'),
        None => false,
    }
}

#[derive(Parser, Debug)]
#[command(
    name = "lumencite",
    bin_name = "lumencite",
    about = "Query your LumenCite reference library from the terminal (read-only).",
    version
)]
struct Cli {
    /// Print human-readable text instead of JSON.
    #[arg(long, global = true)]
    human: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Search entries by metadata (title/author/…), with optional filters.
    Search(SearchArgs),
    /// Show one entry by numeric id or citation key.
    Get {
        /// Numeric entry id or citation key (e.g. `smith2020a`).
        id_or_key: String,
    },
    /// Generate BibTeX for the given citation keys (the LaTeX \cite workflow).
    Bib {
        /// Citation keys to resolve. Unresolved keys are reported on stderr.
        keys: Vec<String>,
    },
    /// Export BibTeX for entries selected by key or by filter.
    Export(ExportArgs),
    /// List all tags.
    Tags,
    /// List all collections (nested).
    Collections,
    /// Full-text search across attached PDFs.
    Fulltext(FulltextArgs),
}

/// `search` / `export` 共通のメタデータフィルタ軸（`EntryFilter` にマップ）。
/// v0.7.0 では複合タグ（`tag_ids`/`tag_match`）は未対応（scope の `--tag` のみ）。
#[derive(Args, Debug, Default)]
struct FilterArgs {
    /// Restrict to entry type(s); repeatable. Types OR together.
    #[arg(long = "type", value_name = "TYPE")]
    entry_types: Vec<String>,
    /// Minimum year (inclusive).
    #[arg(long)]
    year_min: Option<i64>,
    /// Maximum year (inclusive).
    #[arg(long)]
    year_max: Option<i64>,
    /// Only starred entries.
    #[arg(long)]
    starred: bool,
    /// Only entries that have a PDF attachment.
    #[arg(long)]
    has_attachment: bool,
}

#[derive(Args, Debug)]
struct SearchArgs {
    /// Search terms (joined with spaces). Empty lists all entries.
    query: Vec<String>,
    /// Scope to a collection id.
    #[arg(long)]
    collection: Option<i64>,
    /// Scope to a single tag id.
    #[arg(long)]
    tag: Option<i64>,
    #[command(flatten)]
    filter: FilterArgs,
    /// Cap the number of results.
    #[arg(long)]
    limit: Option<usize>,
}

#[derive(Args, Debug)]
struct ExportArgs {
    /// Citation keys to export; repeatable. If given, filters are ignored.
    #[arg(long = "key", value_name = "KEY")]
    keys: Vec<String>,
    /// Scope to a collection id (when selecting by filter).
    #[arg(long)]
    collection: Option<i64>,
    /// Scope to a single tag id (when selecting by filter).
    #[arg(long)]
    tag: Option<i64>,
    #[command(flatten)]
    filter: FilterArgs,
}

#[derive(Args, Debug)]
struct FulltextArgs {
    /// Search terms (joined with spaces).
    query: Vec<String>,
    /// Scope to a collection id.
    #[arg(long)]
    collection: Option<i64>,
    /// Scope to a single tag id.
    #[arg(long)]
    tag: Option<i64>,
}

/// コマンドの出力。`stdout` は標準出力へ、`warnings` は 1 行ずつ標準エラーへ。
#[derive(Debug)]
struct CmdOutput {
    stdout: String,
    warnings: Vec<String>,
}

impl CmdOutput {
    fn new(stdout: String) -> Self {
        Self {
            stdout,
            warnings: Vec::new(),
        }
    }
}

/// CLI エントリポイント。`main.rs` から呼ばれ、プロセス終了コードを返す。
pub fn run() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // clap がヘルプ/バージョン/使い方エラーを整形して出力する。
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("lumencite: failed to start async runtime: {e}");
            return 1;
        }
    };

    match rt.block_on(execute(cli)) {
        Ok(out) => {
            for w in &out.warnings {
                eprintln!("lumencite: {w}");
            }
            print!("{}", out.stdout);
            if !out.stdout.ends_with('\n') {
                println!();
            }
            0
        }
        Err(msg) => {
            eprintln!("lumencite: {msg}");
            1
        }
    }
}

/// DB パスを解決してから、パース済みコマンドを実行する。
async fn execute(cli: Cli) -> Result<CmdOutput, String> {
    let db_path = resolve_db_path()?;
    let pool = open_readonly_pool(&db_path).await?;
    let human = cli.human;

    let result = match cli.command {
        Command::Search(a) => {
            let filter = build_filter(&a.filter);
            let q = a.query.join(" ");
            cmd_search(&pool, &q, a.collection, a.tag, &filter, a.limit, human).await
        }
        Command::Get { id_or_key } => cmd_get(&pool, &id_or_key, human).await,
        Command::Bib { keys } => cmd_bib(&pool, &keys).await,
        Command::Export(a) => {
            let filter = build_filter(&a.filter);
            cmd_export(&pool, &a.keys, a.collection, a.tag, &filter).await
        }
        Command::Tags => cmd_tags(&pool, human).await,
        Command::Collections => cmd_collections(&pool, human).await,
        Command::Fulltext(a) => {
            let q = a.query.join(" ");
            cmd_fulltext(&pool, &q, a.collection, a.tag, human).await
        }
    };

    pool.close().await;
    result
}

/// DB パス: `LUMENCITE_DB_PATH` 優先、無ければ `dirs::data_dir()/<identifier>/lumencite.db`
/// （Tauri `app_data_dir` と同一規則）。
fn resolve_db_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var(DB_PATH_ENV) {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let data = dirs::data_dir()
        .ok_or_else(|| "cannot determine the OS data directory".to_string())?;
    Ok(data.join(APP_IDENTIFIER).join(DB_FILENAME))
}

/// 読取専用プールを開く。`PRAGMA query_only = ON` で書き込みを構造的に禁じる。
async fn open_readonly_pool(db_path: &Path) -> Result<SqlitePool, String> {
    if !db_path.exists() {
        return Err(format!(
            "LumenCite library not found at {}.\n       \
             Launch the LumenCite app once to create it, or set {DB_PATH_ENV}.",
            db_path.display()
        ));
    }

    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("PRAGMA query_only = ON;").execute(conn).await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await
        .map_err(|e| format!("failed to open library read-only at {}: {e}", db_path.display()))
}

fn build_filter(f: &FilterArgs) -> EntryFilter {
    EntryFilter {
        entry_types: f.entry_types.clone(),
        year_min: f.year_min,
        year_max: f.year_max,
        starred: if f.starred { Some(true) } else { None },
        has_attachment: if f.has_attachment { Some(true) } else { None },
        tag_ids: Vec::new(),
        tag_match: TagMatch::default(),
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| e.to_string())
}

// ---- command handlers（読取専用・純粋。テストは `#[sqlx::test]` で直接呼ぶ） ----

async fn cmd_search(
    pool: &SqlitePool,
    query: &str,
    collection: Option<i64>,
    tag: Option<i64>,
    filter: &EntryFilter,
    limit: Option<usize>,
    human: bool,
) -> Result<CmdOutput, String> {
    let mut rows = db::entries::search_entries_filtered(pool, query, collection, tag, filter)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(n) = limit {
        rows.truncate(n);
    }
    let stdout = if human {
        rows.iter()
            .map(|e| {
                let year = e.year.map(|y| y.to_string()).unwrap_or_else(|| "----".into());
                format!("{:>6}  {}  {}", e.id, year, e.title)
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        to_json(&rows)?
    };
    Ok(CmdOutput::new(stdout))
}

async fn cmd_get(pool: &SqlitePool, id_or_key: &str, human: bool) -> Result<CmdOutput, String> {
    let id = resolve_entry_id(pool, id_or_key).await?;
    let detail = db::entries::get_entry(pool, id)
        .await
        .map_err(|e| e.to_string())?;
    let stdout = if human {
        let year = detail
            .year
            .map(|y| y.to_string())
            .unwrap_or_else(|| "----".into());
        let authors = detail
            .authors
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} ({})\n{}\n[{}] id={}",
            detail.title, year, authors, detail.entry_type, detail.id
        )
    } else {
        to_json(&detail)?
    };
    Ok(CmdOutput::new(stdout))
}

/// 数値なら id、そうでなければ citation key として解決する。
async fn resolve_entry_id(pool: &SqlitePool, id_or_key: &str) -> Result<i64, String> {
    if let Ok(n) = id_or_key.parse::<i64>() {
        return Ok(n);
    }
    crate::bibtex::find_entry_id_by_citation_key(pool, id_or_key)
        .await?
        .ok_or_else(|| format!("no entry with id or citation key '{id_or_key}'"))
}

async fn cmd_bib(pool: &SqlitePool, keys: &[String]) -> Result<CmdOutput, String> {
    if keys.is_empty() {
        return Err("no citation keys given".to_string());
    }
    let r = crate::bibtex::export_bibtex_by_keys(pool, keys).await?;
    let mut out = CmdOutput::new(r.bibtex);
    if !r.missing.is_empty() {
        out.warnings
            .push(format!("unresolved citation keys: {}", r.missing.join(", ")));
    }
    Ok(out)
}

async fn cmd_export(
    pool: &SqlitePool,
    keys: &[String],
    collection: Option<i64>,
    tag: Option<i64>,
    filter: &EntryFilter,
) -> Result<CmdOutput, String> {
    // キー指定があればそれを優先。無ければフィルタで一致したエントリの cite key を集める。
    let keys: Vec<String> = if !keys.is_empty() {
        keys.to_vec()
    } else {
        let index = crate::bibtex::citation_key_index(pool).await?;
        let id_to_key: std::collections::HashMap<i64, String> =
            index.into_iter().map(|(k, id)| (id, k)).collect();
        let rows = db::entries::search_entries_filtered(pool, "", collection, tag, filter)
            .await
            .map_err(|e| e.to_string())?;
        rows.iter()
            .filter_map(|r| id_to_key.get(&r.id).cloned())
            .collect()
    };

    if keys.is_empty() {
        // 一致 0 件。空 BibTeX を返す（エラーにはしない）。
        return Ok(CmdOutput::new(String::new()));
    }
    let r = crate::bibtex::export_bibtex_by_keys(pool, &keys).await?;
    Ok(CmdOutput::new(r.bibtex))
}

async fn cmd_tags(pool: &SqlitePool, human: bool) -> Result<CmdOutput, String> {
    let tags = db::tags::get_tags(pool).await.map_err(|e| e.to_string())?;
    let stdout = if human {
        tags.iter()
            .map(|t| format!("{:>6}  {}", t.id, t.name))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        to_json(&tags)?
    };
    Ok(CmdOutput::new(stdout))
}

async fn cmd_collections(pool: &SqlitePool, human: bool) -> Result<CmdOutput, String> {
    let collections = db::collections::get_collections(pool)
        .await
        .map_err(|e| e.to_string())?;
    let stdout = if human {
        collections
            .iter()
            .map(|c| format!("{:>6}  {}", c.id, c.name))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        to_json(&collections)?
    };
    Ok(CmdOutput::new(stdout))
}

async fn cmd_fulltext(
    pool: &SqlitePool,
    query: &str,
    collection: Option<i64>,
    tag: Option<i64>,
    human: bool,
) -> Result<CmdOutput, String> {
    let hits = db::fulltext::search_fulltext(pool, query, collection, tag)
        .await
        .map_err(|e| e.to_string())?;
    let stdout = if human {
        hits.iter()
            .map(|h| format!("{:>6}  p.{}  {}", h.entry.id, h.page, h.snippet))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        to_json(&hits)?
    };
    Ok(CmdOutput::new(stdout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntryInput;

    fn sv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn sample_input(title: &str, key: &str, year: i64) -> EntryInput {
        EntryInput {
            title: title.to_string(),
            year: Some(year),
            entry_type: "article".to_string(),
            citation_key: Some(key.to_string()),
            doi: None,
            isbn: None,
            arxiv_id: None,
            url: None,
            abstract_: None,
            notes: None,
            extra_fields: Default::default(),
            author_ids: Vec::new(),
            author_names: vec!["Jane Doe".to_string()],
            authors: None,
            tag_ids: Vec::new(),
        }
    }

    #[test]
    fn is_cli_invocation_recognizes_subcommands() {
        assert!(is_cli_invocation(&sv(&["lumencite", "search", "neural"])));
        assert!(is_cli_invocation(&sv(&["lumencite", "bib", "a", "b"])));
        assert!(is_cli_invocation(&sv(&["lumencite", "help"])));
        assert!(is_cli_invocation(&sv(&["lumencite", "--help"])));
        assert!(is_cli_invocation(&sv(&["lumencite", "--version"])));
    }

    #[test]
    fn is_cli_invocation_ignores_gui_and_shim() {
        assert!(!is_cli_invocation(&sv(&["lumencite"])));
        assert!(!is_cli_invocation(&sv(&["lumencite", "--mcp-stdio"])));
    }

    #[test]
    fn resolve_db_path_honors_env_override() {
        // 直列化のため 1 プロセス内でのみ set/remove する（他テストと衝突しない一時値）。
        let key = DB_PATH_ENV;
        let prev = std::env::var(key).ok();
        std::env::set_var(key, "/tmp/custom-lumencite.db");
        assert_eq!(
            resolve_db_path().unwrap(),
            PathBuf::from("/tmp/custom-lumencite.db")
        );
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn resolve_db_path_defaults_to_app_data_dir() {
        let key = DB_PATH_ENV;
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
        let p = resolve_db_path().unwrap();
        assert!(p.ends_with(PathBuf::from(APP_IDENTIFIER).join(DB_FILENAME)));
        if let Some(v) = prev {
            std::env::set_var(key, v);
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_returns_json_array_of_matches(pool: SqlitePool) {
        db::entries::create_entry(&pool, &sample_input("Neural Networks", "doe2020a", 2020))
            .await
            .unwrap();
        db::entries::create_entry(&pool, &sample_input("Cooking Pasta", "doe2019a", 2019))
            .await
            .unwrap();

        let out = cmd_search(&pool, "neural", None, None, &EntryFilter::default(), None, false)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["title"], "Neural Networks");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_limit_truncates(pool: SqlitePool) {
        for i in 0..5 {
            db::entries::create_entry(
                &pool,
                &sample_input(&format!("Paper {i}"), &format!("k{i}"), 2020),
            )
            .await
            .unwrap();
        }
        let out = cmd_search(&pool, "", None, None, &EntryFilter::default(), Some(2), false)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_resolves_numeric_id_and_citation_key(pool: SqlitePool) {
        let created = db::entries::create_entry(&pool, &sample_input("Deep Learning", "doe2021a", 2021))
            .await
            .unwrap();

        let by_key = cmd_get(&pool, "doe2021a", false).await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&by_key.stdout).unwrap();
        assert_eq!(v["title"], "Deep Learning");

        let by_id = cmd_get(&pool, &created.id.to_string(), false).await.unwrap();
        let v2: serde_json::Value = serde_json::from_str(&by_id.stdout).unwrap();
        assert_eq!(v2["id"], created.id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn get_unknown_key_errors(pool: SqlitePool) {
        let err = cmd_get(&pool, "nosuchkey1999z", false).await.unwrap_err();
        assert!(err.contains("no entry"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bib_emits_bibtex_and_warns_on_missing(pool: SqlitePool) {
        db::entries::create_entry(&pool, &sample_input("Deep Learning", "doe2021a", 2021))
            .await
            .unwrap();

        let out = cmd_bib(&pool, &sv(&["doe2021a", "ghost1900a"]))
            .await
            .unwrap();
        assert!(out.stdout.contains("@article{doe2021a"));
        assert_eq!(out.warnings.len(), 1);
        assert!(out.warnings[0].contains("ghost1900a"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bib_with_no_keys_errors(pool: SqlitePool) {
        let err = cmd_bib(&pool, &[]).await.unwrap_err();
        assert!(err.contains("no citation keys"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_by_filter_selects_matching_keys(pool: SqlitePool) {
        db::entries::create_entry(&pool, &sample_input("Old Paper", "doe2015a", 2015))
            .await
            .unwrap();
        db::entries::create_entry(&pool, &sample_input("New Paper", "doe2022a", 2022))
            .await
            .unwrap();

        let filter = EntryFilter {
            year_min: Some(2020),
            ..EntryFilter::default()
        };
        let out = cmd_export(&pool, &[], None, None, &filter).await.unwrap();
        assert!(out.stdout.contains("doe2022a"));
        assert!(!out.stdout.contains("doe2015a"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tags_and_collections_emit_json_arrays(pool: SqlitePool) {
        let t = cmd_tags(&pool, false).await.unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&t.stdout)
            .unwrap()
            .is_array());
        let c = cmd_collections(&pool, false).await.unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&c.stdout)
            .unwrap()
            .is_array());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn readonly_pool_allows_reads_blocks_writes(pool: SqlitePool) {
        // sqlx::test の pool は書込可。ここでは after_connect 相当を手動適用して
        // query_only の効果だけを確認する（実ファイルのオープンは別テスト対象外）。
        sqlx::query("PRAGMA query_only = ON;")
            .execute(&pool)
            .await
            .unwrap();
        // 読取は成功
        assert!(db::tags::get_tags(&pool).await.is_ok());
        // 書込は失敗
        let w = sqlx::query("INSERT INTO tags (name) VALUES ('x')")
            .execute(&pool)
            .await;
        assert!(w.is_err());
    }
}
