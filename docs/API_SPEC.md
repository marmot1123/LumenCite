# LumenCite Tauri コマンド API 仕様

フロントエンド（React）とバックエンド（Rust）のやりとりは `invoke()` を通じて行う。

```ts
import { invoke } from "@tauri-apps/api/core";
const entry = await invoke("get_entry", { id: 1 });
```

## データ型

```ts
type EntryType = "article" | "book" | "inproceedings" | "thesis" | "webpage" | "misc";
type RelationType = "preprint_of" | "version_of" | "supplement_of";

type Author = {
  id: number;
  name: string;
  given_name?: string;
  family_name?: string;
  orcid?: string;
};

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

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `search_authors` | `query: String` | `Vec<Author>` |
| `merge_authors` | `from_id: i64, into_id: i64` | `Result<()>` |

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

`create_entry` / `update_entry` は基本フィールド（`title` / `entry_type` / `year` / `abstract` / `doi` / `isbn` / `arxiv_id` / `url` / `notes` / `author_names`）に加え、型固有フィールドを `extra_fields`（`{string: string}`）で受け付ける（`journal` / `volume` / `issue` / `number` / `pages` / `publisher` / `booktitle` / `address` / `edition` / `series` / `school` / `institution` / `organization` / `howpublished` など、`DATA_MODEL.md` の `entries.extra_fields` 参照）。`update_entry` では指定したキーのみ上書き/追加し、未指定の既存 `extra_fields` は保持する。

ホワイトリストの上書きは `get_setting("chat.tool_whitelist")` / `set_setting` で読み書きする（専用コマンドは設けない）。

### MCP クライアント（v0.2.0 追加）

外部 MCP サーバー（Obsidian 等）を stdio で起動し、`tools/list` を取得して Chat ツールスキーマへ動的マージする（プレフィックス `mcp_<id>_<tool>`）。LLM がそのツールを呼ぶと内部で JSON-RPC により当該サーバーへ転送する。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `list_mcp_servers` | — | `Result<Vec<McpServerConfig>>` |
| `add_mcp_server` | `config: McpServerConfig` | `Result<()>` — 設定保存 + プロセス起動 |
| `remove_mcp_server` | `id: String` | `Result<()>` — プロセス停止 + 設定削除 |

設定は `settings` の `mcp.servers` キーに JSON（Claude Desktop の `mcpServers` 互換）で保存する。

### OCR（v0.2.0 追加）

テキストレイヤーのないスキャン PDF を LLM Vision で OCR し、結果を `fulltext` にページ単位で保存する。詳細ビューの手動ボタンと LLM ツール（`ocr_pdf` / `attach_ocr_text`）で内部実装を共有する。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `ocr_pdf` | `entry_id: i64, pages?: Vec<i64>` | `Result<()>` — `pages` 省略時は全ページ。OCR プロバイダは `LlmSettings.ocr_provider` → `provider` のフォールバック |
