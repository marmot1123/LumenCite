# LumenCite Tauri コマンド API 仕様

フロントエンド（React）とバックエンド（Rust）のやりとりは `invoke()` を通じて行う。

```ts
import { invoke } from "@tauri-apps/api/core";
const entry = await invoke("get_entry", { id: 1 });
```

## データ型

```ts
// 既存 6 種は BibTeX 由来のキー、v0.4.0 追加分は Zotero のアイテムタイプ名（camelCase）。
type EntryType =
  | "article" | "inproceedings" | "preprint"
  | "book" | "bookSection"
  | "thesis" | "report"
  | "magazineArticle" | "newspaperArticle" | "encyclopediaArticle" | "dictionaryEntry"
  | "manuscript" | "presentation" | "patent" | "standard" | "dataset" | "computerProgram"
  | "webpage" | "misc";
type RelationType = "preprint_of" | "version_of" | "supplement_of";

type Author = {
  id: number;
  name: string;
  given_name?: string | null;
  middle_name?: string | null;             // v0.3.0
  family_name?: string | null;
  suffix?: string | null;                  // v0.3.0
  name_particle?: string | null;           // v0.3.0
  name_original?: string | null;           // v0.3.0 — 原語表記フルネーム
  given_name_original?: string | null;     // v0.3.0
  family_name_original?: string | null;    // v0.3.0
  original_script?: string | null;         // v0.3.0 — ISO 15924 (Hani/Hang/Cyrl/...)
  reading_family?: string | null;          // v0.3.0 — 読み仮名（五十音ソート用）
  reading_given?: string | null;           // v0.3.0
  is_organization: boolean;                // v0.3.0 — 団体著者
  email?: string | null;                   // v0.3.0
  homepage_url?: string | null;            // v0.3.0
  notes?: string | null;                   // v0.3.0
  orcid?: string | null;                   // 互換維持の専用カラム
  updated_at?: string | null;              // v0.3.0
  identifiers: AuthorIdentifier[];         // v0.3.0 — JOIN で詰めた識別子配列
};

type AuthorIdentifier = {
  author_id: number;
  scheme: string;   // 'orcid' / 'scopus' / 'dblp' / 'semantic_scholar' / 'wikidata' / 'isni' / 'viaf' / 'researcher_id' / 'google_scholar'
  value: string;
  url?: string | null;
};

type AuthorInput = {  // v0.3.0 — update_author / EntryInput.authors で使う
  name: string;
  given_name?: string | null;
  middle_name?: string | null;
  family_name?: string | null;
  suffix?: string | null;
  name_particle?: string | null;
  name_original?: string | null;
  given_name_original?: string | null;
  family_name_original?: string | null;
  original_script?: string | null;
  reading_family?: string | null;
  reading_given?: string | null;
  is_organization?: boolean;
  email?: string | null;
  homepage_url?: string | null;
  notes?: string | null;
  orcid?: string | null;
  identifiers?: AuthorIdentifierInput[];
};

type AuthorIdentifierInput = { scheme: string; value: string; url?: string | null };

type Tag = { id: number; name: string };

type Collection = {
  id: number;
  name: string;
  parent_id?: number;
  children: Collection[];
};

type Attachment = {
  id: number;
  entry_id: number;
  file_name: string;
  mime_type: string;
  created_at: string;
};

// 一覧表示用（軽量）
type EntrySummary = {
  id: number;
  title: string;
  year?: number;
  entry_type: EntryType;
  authors: Author[];
  tags: Tag[];
  has_attachment: boolean;
  journal?: string; // extra_fields の `journal` を投影（一覧テーブル用）
  starred: boolean;
};

// 詳細画面用（フル情報）
type EntryDetail = EntrySummary & {
  citation_key?: string; // BibTeX エントリキー。null/未設定なら export 時に自動生成
  doi?: string;
  isbn?: string;
  arxiv_id?: string;
  url?: string;
  abstract?: string;
  notes?: string;
  deleted_at?: string; // ゴミ箱内なら datetime 文字列
  extra_fields: Record<string, string>;
  attachments: Attachment[];
  relations: {
    entry: EntrySummary;
    relation_type: RelationType;
    direction: "from" | "to";
  }[];
};

// 登録・更新時の入力型
type EntryInput = {
  title: string;
  year?: number;
  entry_type: EntryType;
  citation_key?: string; // 省略/空文字なら自動生成（NULL 保存）。サニタイズして保存
  doi?: string;
  isbn?: string;
  arxiv_id?: string;
  url?: string;
  abstract?: string;
  notes?: string;
  extra_fields?: Record<string, string>;
  author_ids?: number[];   // 既存著者のID（順序＝著者順）
  author_names?: string[]; // 新規著者名（IDがない場合）
  authors?: AuthorInput[]; // v0.3.0 — 構造化された著者入力。Some の時は author_names を無視
  tag_ids?: number[];
};

type LlmSettings = {
  provider: "openai" | "anthropic";
  model: string;
  summary_source: "abstract" | "fulltext"; // 要約入力ソース（v0.1.0 から）
  ocr_provider?: "openai" | "anthropic";    // v0.2.0: OCR 用プロバイダ。未指定なら provider にフォールバック
  ocr_model?: string;                        // v0.2.0: OCR 用モデル。未指定なら model にフォールバック
};

type HighlightColor = "yellow" | "green" | "blue";

type Highlight = {
  id: number;
  entry_id: number;
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  color: HighlightColor;
  text: string;
  note?: string;
  created_at: string;
};

type HighlightInput = {
  entry_id: number;
  page: number;
  x: number;
  y: number;
  width: number;
  height: number;
  color: HighlightColor;
  text: string;
  note?: string;
};

// 要約ストリーミングイベント（tauri::ipc::Channel 経由で送出）
type SummaryStreamEvent =
  | { kind: "start"; model: string }
  | { kind: "delta"; text: string }
  | { kind: "done"; full_text: string }
  | { kind: "error"; message: string };

type UpdateInfo = {
  version: string;
  date?: string;
  notes?: string;
  available: boolean;
};

// === Chat / MCP / OCR（v0.2.0 追加） ===

type ChatRole = "user" | "assistant" | "tool";
type ScopeMode = "all" | "entries"; // DB 全体検索 / 特定文献に絞る

type ChatSession = {
  id: number;
  title: string;
  provider: string;
  model: string;
  system_prompt?: string;
  scope_mode: ScopeMode;
  entry_count: number; // scope_mode='entries' のとき紐づく文献数
  created_at: string;
  updated_at: string;
  archived_at?: string;
};

// LLM のツール呼び出し 1 件（assistant メッセージに付随）
type ToolCallSpec = {
  call_id: string;
  tool_name: string; // 例 "fulltext_search" / "add_tag" / "mcp_obsidian_append_note"
  arguments: Record<string, unknown>; // JSON 引数
};

type ChatMessage = {
  id: number;
  session_id: number;
  role: ChatRole;
  content: string;
  tool_calls?: ToolCallSpec[]; // role='assistant' のとき
  tool_call_id?: string;       // role='tool' のとき
  created_at: string;
  position: number;
};

type SessionWithMessages = {
  session: ChatSession;
  messages: ChatMessage[];
  entry_ids: number[]; // scope の対象 entry 集合
};

// LLM に渡すツール定義。OpenAI / Anthropic 形式へは Rust 側で変換
type ToolSpec = {
  name: string;
  description: string;
  parameters: Record<string, unknown>; // JSON Schema
  needs_approval: boolean;              // ホワイトリスト評価結果
};

// agentic ループのストリーミングイベント（tauri::ipc::Channel 経由）
// Rust enum を serde(tag = "kind", rename_all = "snake_case") で送出する
type ChatStreamEvent =
  | { kind: "session_started"; session_id: number }
  | { kind: "delta"; text: string } // assistant の自然言語ストリーム
  | { kind: "tool_call_proposed"; call_id: string; tool_name: string; args_preview: string; needs_approval: boolean }
  | { kind: "tool_call_executed"; call_id: string; result_summary: string }
  | { kind: "message_persisted"; message_id: number; role: ChatRole }
  | { kind: "done" }
  | { kind: "error"; message: string };

// MCP サーバー設定（Claude Desktop の mcpServers 互換）
type McpServerConfig = {
  id: string;       // サーバー識別子。ツールプレフィックス mcp_<id>_<tool> にも使う
  command: string;  // 起動コマンド
  args?: string[];
  env?: Record<string, string>;
};

// MCP サーバーの起動状態（list_mcp_servers が config に重ねて返す）
type McpServerStatus =
  | { state: "running"; tool_count: number } // 起動成功・取得ツール数
  | { state: "failed"; error: string };       // 起動/ハンドシェイク失敗

type McpServerInfo = McpServerConfig & {
  status: McpServerStatus | null; // null = 状態不明（未起動試行）
};
```

---

## コマンド一覧

### 文献（entries）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_entries` | `collection_id?: i64, tag_id?: i64, view?: "starred"\|"unfiled"\|"trash"` | `Vec<EntrySummary>` |
| `get_entry` | `id: i64` | `Result<EntryDetail>` |
| `create_entry` | `input: EntryInput` | `Result<EntryDetail>` |
| `update_entry` | `id: i64, input: EntryInput` | `Result<EntryDetail>` |
| `set_starred` | `id: i64, starred: bool` | `Result<()>` |
| `trash_entry` | `id: i64` | `Result<()>` — ソフト削除（`deleted_at` をセット） |
| `restore_entry` | `id: i64` | `Result<()>` — ゴミ箱から復元 |
| `delete_entry` | `id: i64` | `Result<()>` — ハード削除（永久）。通常 UI からは `trash_entry` を経由。 |
| `fetch_metadata_by_doi` | `doi: String` | `Result<EntryInput>` |
| `fetch_metadata_by_arxiv` | `arxiv_id: String` | `Result<EntryInput>` |
| `fetch_metadata_by_isbn` | `isbn: String` | `Result<EntryInput>` |
| `is_citation_key_available` | `key: String, exclude_id?: i64` | `Result<bool>` — 固定 cite key が使用可能か（サニタイズ後に他エントリと重複しないか）。`exclude_id` は編集中エントリ自身を除外。空キーは常に `true`（自動扱い） |
| `resolve_citation_key` | `entry_id: i64` | `Result<String>` — `.bib` 同期（ゴミ箱を除く全件書き出し）で実際に割り当てられる cite key。`export_bibtex(None)` と同じ並び・衝突回避を再現。詳細ビューの表示/コピー用 |

`create_entry` / `update_entry` の `EntryInput.citation_key` はサニタイズ後 `entries.citation_key` に保存する（空なら NULL = 自動）。既存の固定キーと重複する非 NULL 値は UNIQUE 制約で拒否される（`Result` の `Err`）。UI は保存前に `is_citation_key_available` で検証する。生成・重複回避の規則は `DATA_MODEL.md` の `citation_key` 節を参照。

`get_entries` の `view` は特殊ビュー専用フィルタ。`collection_id` / `tag_id` と組み合わせる場合は `view` は無視され、コレクション/タグの所属で絞られる（いずれも `deleted_at IS NULL` を満たすもののみ）。`search_entries` は常にゴミ箱を除外する。

### BibTeX 自動同期

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_bibtex_sync_path` | — | `Option<String>` — `settings.bibtex_sync_path` の値 |
| `set_bibtex_sync_path` | `path: String` | `Result<()>` — 設定後に即同期リクエストを送る |
| `clear_bibtex_sync_path` | — | `Result<()>` — 同期を無効化 |
| `pick_bibtex_sync_path` | `default_name?: String` | `Result<Option<String>>` — 保存ダイアログを開き選択パスを返す（キャンセル時 None） |
| `sync_bibtex_now` | — | `Result<()>` — debounce をバイパスして即時書き出し |

ミューテーション系コマンド（`create_entry` / `update_entry` / `delete_entry` / `trash_entry` / `restore_entry` / `bulk_*` / `import_bibtex`）が呼ばれると、内部の `sync_tx` 経由でコーディネーターに通知される。コーディネーターは 800ms の trailing-edge デバウンスで `bibtex::sync_bibtex` を呼び出し、書き込み完了/失敗を `bibtex-synced` イベントで UI に通知する。

```ts
// Tauri イベント: "bibtex-synced"
type BibtexSyncEvent = {
  path: string;
  synced_at: string | null; // epoch seconds 文字列。error が null のときのみセット
  error: string | null;
};
```

書き込みは `<path親>/.<filename>.tmp` を作って `rename` するアトミックな置換。書き出し対象は **ゴミ箱を除く全エントリ**。

### 一括操作（bulk）

複数選択された文献に対する一括処理。それぞれ ids が空のときは no-op。内部でトランザクションを張る。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `bulk_trash` | `ids: Vec<i64>` | `Result<()>` |
| `bulk_restore` | `ids: Vec<i64>` | `Result<()>` |
| `bulk_purge` | `ids: Vec<i64>` | `Result<()>` — entries_fts と fulltext もまとめてクリーンアップ |
| `bulk_add_to_collection` | `ids: Vec<i64>, collection_id: i64` | `Result<()>` — 重複は INSERT OR IGNORE |
| `bulk_add_tag` | `ids: Vec<i64>, tag_id: i64` | `Result<()>` — 重複は INSERT OR IGNORE |

### サイドバー件数（counts）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_sidebar_counts` | — | `SidebarCounts` |

```ts
type SidebarCounts = {
  total: number;     // ゴミ箱を除いた全件数
  starred: number;   // お気に入り（ゴミ箱を除く）
  unfiled: number;   // コレクション未割当（ゴミ箱を除く）
  trash: number;     // ゴミ箱内の件数
  collections: Record<string, number>; // collection_id -> 件数（ゴミ箱を除く）
  tags: Record<string, number>;        // tag_id -> 件数（ゴミ箱を除く）
};
```

エントリの作成・更新・削除・コレクション/タグの付け外し・スター切替などのミューテーション後に再取得する。フロントエンドでは `loadEntries` の都度フェッチして表示と整合させている。

### 著者（authors）

v0.3.0 で本格的な編集 API を追加。`Author` 型・`AuthorInput` / `AuthorIdentifierInput` は冒頭の型定義を参照。
名寄せロジック（ORCID → 正規化 name → INSERT）と FTS 再同期の詳細は `DATA_MODEL.md` の `authors` セクション。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `search_authors` | `query: String, limit?: i64` | `Result<Vec<Author>>` — name / name_original / orcid の部分一致。limit デフォルト 20 |
| `get_author` | `id: i64` | `Result<Option<Author>>` — identifiers 込み |
| `update_author` | `id: i64, input: AuthorInput` | `Result<Author>` — 全フィールド差し替え + identifiers 総差し替え + 関連 entry の FTS 再同期 |
| `merge_authors` | `from_id: i64, into_id: i64` | `Result<()>` — entry_authors を `into` に集約、`from` を削除。identifiers は `into` 優先。関連 entry の FTS を再同期 |
| `add_author_identifier` | `author_id: i64, input: AuthorIdentifierInput` | `Result<()>` — (author_id, scheme) で upsert。scheme='orcid' のときは `authors.orcid` 列も同期 |
| `delete_author_identifier` | `author_id: i64, scheme: String` | `Result<()>` — scheme='orcid' のときは `authors.orcid` 列もクリア |
| `fetch_author_from_orcid` | `orcid: String` | `Result<AuthorInput>` — ORCID Public API (`https://pub.orcid.org/v3.0/{id}/person`) から given/family/credit-name / public email / researcher-urls / external-identifiers を取得して AuthorInput に詰めて返す。DB には書かない pure fetcher（呼び出し側が `update_author` で保存する想定）。other-names に CJK / Hangul / Cyrillic が含まれていれば best-effort で `name_original` / `original_script` を推定する |

`update_author` と `merge_authors` は `.bib` 同期キックを送るため、エクスポート先ファイルにも自動反映される。

`(scheme, value)` は `author_identifiers` で UNIQUE 制約。同一の識別子値を別著者にぶら下げようとすると保存失敗（`Err`）になる — その状況は通常「名寄せが正しく機能していない」シグナルなので、`merge_authors` で 1 著者に統合してから再度設定する想定。

### コレクション（collections）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_collections` | — | `Vec<Collection>` |
| `create_collection` | `name: String, parent_id?: i64` | `Result<Collection>` |
| `update_collection` | `id: i64, name: String` | `Result<Collection>` |
| `delete_collection` | `id: i64` | `Result<()>` |
| `add_entry_to_collection` | `entry_id: i64, collection_id: i64` | `Result<()>` |
| `remove_entry_from_collection` | `entry_id: i64, collection_id: i64` | `Result<()>` |

### タグ（tags）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_tags` | — | `Vec<Tag>` |
| `create_tag` | `name: String` | `Result<Tag>` |
| `delete_tag` | `id: i64` | `Result<()>` |
| `add_tag_to_entry` | `entry_id: i64, tag_id: i64` | `Result<()>` |
| `remove_tag_from_entry` | `entry_id: i64, tag_id: i64` | `Result<()>` |

### 添付ファイル（attachments）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `add_attachment` | `entry_id: i64, file_path: String` | `Result<Attachment>` |
| `delete_attachment` | `id: i64` | `Result<()>` |
| `open_attachment` | `id: i64` | `Result<()>` |
| `index_attachment` | `attachment_id: i64` | `Result<()>` |

`index_attachment` はPDFからテキストを抽出してFTS5インデックスに登録する。`add_attachment` 後に非同期で呼ぶ想定。

### 検索（search）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `search_entries` | `query: String, collection_id?: i64, tag_id?: i64` | `Vec<EntrySummary>` |
| `fulltext_search` | `query: String` | `Vec<FulltextResult>` |

```ts
type FulltextResult = {
  entry: EntrySummary;
  page: number;
  snippet: string;  // マッチ箇所の前後テキスト
};
```

`search_entries` はメタデータ FTS インデックス（`entries_fts`）を対象に検索する。
- 検索対象: title / authors / tags / abstract / 識別子（DOI・ISBN・arXiv ID）・year
- トークナイザ: `trigram`（日本語・英語ともに 3-gram 部分一致）
- `collection_id` / `tag_id` が指定された場合は、その絞り込みの中だけを検索する
- 並び順: BM25 ランクスコア降順
- 空クエリは呼び出さない（フロント側で `get_entries` にフォールバック）

将来 `fulltext_search`（PDF ページ単位）を実装する際は、結果型を `Vec<SearchHit>` に拡張する形で `search_entries` 内に統合する想定。

### エントリ間の関連（relations）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `add_relation` | `from_id: i64, to_id: i64, relation_type: String` | `Result<()>` |
| `remove_relation` | `from_id: i64, to_id: i64, relation_type: String` | `Result<()>` |

関連の一覧は `get_entry` の `EntryDetail.relations` に含まれる。

### BibTeX

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `import_bibtex` | `content: String` | `Result<ImportResult>` |
| `export_bibtex` | `entry_ids?: Vec<i64>` | `Result<String>` |
| `sync_bib_file` | `path: String` | `Result<()>` |

```ts
type ImportResult = { imported: number; skipped: number };
```

`export_bibtex` で `entry_ids` を省略した場合は全件エクスポート。  
`sync_bib_file` は指定パスの `.bib` ファイルを常に最新状態に保つ（LaTeX Workshop連携用）。

### ハイライト（highlights）— v0.1.0 追加

詳細ビューの PDF テキスト選択 → ハイライト保存に使う。座標は pdf.js の PDF ポイント（左下原点）。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_highlights` | `entry_id: i64` | `Result<Vec<Highlight>>` — ページ昇順、同ページ内は `y` 降順 |
| `create_highlight` | `input: HighlightInput` | `Result<Highlight>` |
| `update_highlight` | `id: i64, color?: HighlightColor, note?: String` | `Result<Highlight>` — 部分更新 |
| `delete_highlight` | `id: i64` | `Result<()>` |

メタパネル「ハイライト」タブの一覧表示で、クリックすると該当ページにジャンプする想定。

### LLM

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `generate_summary` | `entry_id: i64, source: "abstract" \| "fulltext", channel: Channel<SummaryStreamEvent>` | `Result<()>` — ストリーミング送出。完了時にDB側 `entries.summary` も更新 |
| `get_llm_settings` | — | `LlmSettings` |
| `save_llm_settings` | `settings: LlmSettings` | `Result<()>` |
| `get_api_key` | `provider: "openai" \| "anthropic"` | `Result<Option<String>>` — OSキーチェーンから取得（マスク表示用） |
| `set_api_key` | `provider: "openai" \| "anthropic", key: String` | `Result<()>` |
| `delete_api_key` | `provider: "openai" \| "anthropic"` | `Result<()>` |
| `test_llm_connection` | `provider: "openai" \| "anthropic", model: String` | `Result<()>` — 軽量プロンプトで疎通確認 |

APIキーはOSキーチェーン（`keyring` クレート経由）に保存するため、`LlmSettings` には含まない。`generate_summary` の `channel` 引数は `tauri::ipc::Channel<SummaryStreamEvent>` で、トークン到着ごとに `delta` イベントが届く。

### バックアップ / エクスポート（v0.1.0 追加）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `run_backup` | — | `Result<String>` — 作成された .db のパス。`VACUUM INTO` 使用 |
| `list_backups` | — | `Result<Vec<BackupInfo>>` — `<app_data_dir>/backups/` 配下のメタ情報 |
| `restore_backup` | `path: String` | `Result<()>` — 確認ダイアログ後、アプリ再起動して復元 |
| `export_database` | `path: String, format: "json" \| "bibtex" \| "markdown"` | `Result<()>` |

```ts
type BackupInfo = { path: string; created_at: string; size_bytes: number };
```

自動バックアップは Rust 側で起動時 + 24h 間隔のタイマーから呼ばれる。フロントからの手動呼び出しも可能。

### アップデーター（v0.1.0 追加）

`tauri-plugin-updater` のラッパー。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `check_for_updates` | — | `Result<UpdateInfo>` |
| `apply_update` | — | `Result<()>` — ダウンロード+検証+再起動 |
| `get_updater_channel` | — | `"stable" \| "beta"` |
| `set_updater_channel` | `channel: "stable" \| "beta"` | `Result<()>` |

### アプリ設定（settings）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_setting` | `key: String` | `Option<String>` |
| `set_setting` | `key: String, value: String` | `Result<()>` |

### Chat（v0.2.0 追加）

agentic LLM Chat のセッション管理と会話ループ。`chat_send_message` が中核で、tool_call があれば承認チェック → 実行 → 結果を会話に追加 → 再度 LLM 呼び出し、を完了まで反復する。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `list_chat_sessions` | `limit?: i64, offset?: i64` | `Result<Vec<ChatSession>>` — `updated_at` 降順。サイドバー用 |
| `create_chat_session` | `title: String, provider: String, model: String, scope_mode: ScopeMode, entry_ids: Vec<i64>` | `Result<ChatSession>` |
| `get_chat_session` | `id: i64` | `Result<SessionWithMessages>` — セッションを開く |
| `update_chat_session_title` | `id: i64, title: String` | `Result<()>` |
| `archive_chat_session` | `id: i64` | `Result<()>` — ソフト削除（`archived_at` をセット） |
| `chat_send_message` | `session_id: i64, user_text: String, channel: Channel<ChatStreamEvent>` | `Result<()>` — **agentic ループのエントリポイント** |
| `approve_tool_call` | `call_id: String, approved: bool` | `Result<()>` — UI の承認/拒否を進行中ループへ返す |
| `cancel_chat_stream` | `session_id: i64` | `Result<()>` — 進行中ストリームの中断。部分応答は保存される |
| `generate_chat_title` | `session_id: i64` | `Result<String>` — 自動タイトル生成（最初のターン後にバックグラウンドで呼ぶ） |

`chat_send_message` の `channel` は `tauri::ipc::Channel<ChatStreamEvent>`。`tool_call_proposed` の `needs_approval=true` を受けたら UI は承認ダイアログを出し、`approve_tool_call` で応答する。承認制御はツール別ホワイトリスト（DATA_MODEL の `chat.tool_whitelist` 参照）に従う:

- read 系（`fulltext_search` / `get_entry` / `list_*`）: 常に自動
- `add_tag` / `update_notes` / `attach_ocr_text` / `add_to_collection`: デフォルト自動（設定で都度承認に変更可）
- `create_entry` / `update_entry`: 都度承認
- `delete_*` / MCP の write 系: 常時確認（ホワイトリストで上書き不可）

`create_entry` / `update_entry` は基本フィールド（`title` / `entry_type` / `year` / `abstract` / `doi` / `isbn` / `arxiv_id` / `url` / `notes` / `author_names` / `citation_key`）に加え、型固有フィールドを `extra_fields`（`{string: string}`）で受け付ける（`journal` / `volume` / `issue` / `number` / `pages` / `publisher` / `booktitle` / `address` / `edition` / `series` / `school` / `institution` / `organization` / `howpublished` など、`DATA_MODEL.md` の `entries.extra_fields` 参照）。`update_entry` では指定したキーのみ上書き/追加し、未指定の既存 `extra_fields` は保持する。

`citation_key`（固定 cite key）の扱い:
- `create_entry`: 省略/空文字なら自動生成（NULL 保存）。サニタイズ後に他エントリと重複する場合は実行前に検証で弾き、ツールはエラーを返す（LLM が別キーを選び直せるようメッセージを返す）。
- `update_entry`: **引数を省略すると現在のキーを保持**する（指定しない限り変更しない）。値を渡すとピン留めキーを差し替え、空文字を渡すと unpin（自動生成へ戻す）。重複は同上で弾く。
- `get_entry` ツールは戻り値に `citation_key`（ピン留めキー。未設定なら null）と `resolved_citation_key`（`.bib` / `\cite{}` で実際に使われるキー。未ピン留め時は自動生成値）を含む。

ホワイトリストの上書きは `get_setting("chat.tool_whitelist")` / `set_setting` で読み書きする（専用コマンドは設けない）。

### MCP クライアント（v0.2.0 追加）

外部 MCP サーバー（Obsidian 等）を stdio で起動し、`tools/list` を取得して Chat ツールスキーマへ動的マージする（プレフィックス `mcp_<id>_<tool>`）。LLM がそのツールを呼ぶと内部で JSON-RPC により当該サーバーへ転送する。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `list_mcp_servers` | — | `Result<Vec<McpServerInfo>>` — 設定 + 起動状態（起動失敗を UI に表示するため） |
| `add_mcp_server` | `config: McpServerConfig` | `Result<()>` — 設定保存 + プロセス起動 |
| `remove_mcp_server` | `id: String` | `Result<()>` — プロセス停止 + 設定削除 |

設定は `settings` の `mcp.servers` キーに JSON（Claude Desktop の `mcpServers` 互換）で保存する。

### MCP サーバー公開（v0.3.0 追加 — Phase 1: read-only / Phase 2: write ゲート）

LumenCite 自身を MCP サーバーとして公開し、Claude Desktop / Claude Code からライブラリを参照・操作できるようにする。起動中アプリ内に localhost HTTP（JSON-RPC 2.0）でサーバーを立て、`Authorization: Bearer <token>` で認可する。token は OS キーチェーン（アカウント名 `mcp_server.token`）に保管。サーバー側で LLM は呼ばない（推論は接続元のサブスク認証側）。詳細は `SPEC.md` の「MCP サーバー公開」節を参照。

```ts
type McpServerStatusInfo = {
  enabled: boolean;       // mcp_server.enabled == "1"
  running: boolean;       // サーバースレッドが起動中か
  port: number;           // 起動中なら実バインドポート、未起動なら設定値（既定 3917）
  has_token: boolean;     // キーチェーンに token があるか
  write_enabled: boolean; // Phase 2: write 系ツールを公開しているか（mcp_server.write_enabled）
};

// get_mcp_audit_log の戻り値（Phase 2）。MCP 経由の write を新しい順で返す。
type McpAuditEntry = {
  id: number;
  tool_name: string;
  arguments: string;   // JSON 文字列
  result: string | null;
  is_error: boolean;
  created_at: string;
};
```

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_mcp_server_status` | — | `Result<McpServerStatusInfo>` |
| `set_mcp_server_enabled` | `enabled: bool` | `Result<McpServerStatusInfo>` — 有効化時は token を用意してサーバー起動＋実バインドポートを `mcp_server.port` に保存。無効化時は停止 |
| `set_mcp_server_write_enabled` | `enabled: bool` | `Result<McpServerStatusInfo>` — **Phase 2**: write 系の公開可否を切替。サーバーはリクエスト毎に設定を読むため再起動不要 |
| `get_mcp_audit_log` | `limit?: i64` | `Result<Vec<McpAuditEntry>>` — **Phase 2**: MCP 経由 write の監査ログ（新しい順。limit 既定 100） |
| `regenerate_mcp_server_token` | — | `Result<String>` — token を再生成しキーチェーンへ保存。起動中なら新 token で再起動し、生成した token を返す（表示用） |
| `get_mcp_server_config_snippet` | `client: String` | `Result<String>` — クライアント別の貼り付け設定。`"claude_code"` は `claude mcp add --transport http ...` コマンド、`"claude_desktop"` は本体を `--mcp-stdio` shim として起動する `mcpServers` JSON（**Phase 3**）、それ以外は URL + ヘッダ |

**Phase 3（stdio shim）:** Claude Desktop は stdio トランスポートのみ対応しリモート HTTP MCP に直結できない。本体バイナリを `--mcp-stdio` 付きで起動すると（`main.rs` が GUI 起動前に検出）、Tauri を立ち上げず `mcp_shim::run_stdio_proxy` が「stdio ↔ localhost HTTP」プロキシとして動作し、`LUMENCITE_MCP_URL` / `LUMENCITE_MCP_TOKEN`（Claude Desktop 設定の `env`）を使って内蔵 MCP サーバーへ橋渡しする。別 sidecar バイナリにしないことで追加の署名・notarize 対象を増やさない。`claude_desktop` スニペットの `command` は `std::env::current_exe()` の絶対パス。

**公開ツール（MCP `tools/list`）:**
- **read 系（常時）**: `fulltext_search` / `get_entry` / `list_collections` / `list_tags`（チャットの read ツール定義を流用）＋ `search_entries`（メタデータ FTS）/ `resolve_citation_key`（実 cite key）/ `export_bibtex`（.bib テキスト）。
- **write 系（`mcp_server.write_enabled` 有効時のみ）**: `add_tag` / `update_notes` / `add_to_collection` / `create_entry` / `update_entry`（`mutate` の定義を流用）。**破壊系 `delete_entry` は常に非公開**で、`tools/call` でも許可リスト外として `isError` で拒否する。write 無効時に write ツールを呼ぶと `isError` で拒否。
  - **バルク対応**: `add_tag` / `add_to_collection` は単一 `entry_id` に加えて **`entry_ids`（整数配列）**を受け付け、1 回の呼び出しで複数エントリへ適用する（両者は併用可・重複は順序保持で除去）。ベストエフォートで、存在しないエントリはスキップして成功分を適用し、結果サマリ（適用件数＋スキップ件数）を返す。1 件も成功しなければ `isError`。タグは get-or-create をバッチで 1 回だけ行う。
- write 成功時はサーバーが監査ログ記録＋ `.bib` 同期キック＋ `entries-changed` イベント（一覧ライブ反映）を発火する。

### OCR（v0.2.0 追加）

テキストレイヤーのないスキャン PDF を LLM Vision で OCR し、結果を `fulltext` にページ単位で保存する。詳細ビューの手動ボタンと LLM ツール（`ocr_pdf` / `attach_ocr_text`）で内部実装を共有する。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `ocr_pdf` | `entry_id: i64, pages?: Vec<i64>` | `Result<()>` — `pages` 省略時は全ページ。OCR プロバイダは `LlmSettings.ocr_provider` → `provider` のフォールバック |
