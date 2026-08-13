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
  abstract_?: string; // DB 列は `abstract` だが IPC/TS 境界では serde 既定の `abstract_`
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
  abstract_?: string; // DB 列は `abstract` だが IPC/TS 境界では serde 既定の `abstract_`
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

// `@tauri-apps/plugin-updater` の `check()` が返す `Update` の投影（独自コマンドではない）
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
| `get_entries` | `collection_id?: i64, tag_id?: i64, view?: "starred"\|"unfiled"\|"trash", filter?: EntryFilter` | `Vec<EntrySummary>` |
| `get_entry` | `id: i64` | `Result<EntryDetail>` |
| `create_entry` | `input: EntryInput` | `Result<EntryDetail>` — DOI/arXiv/ISBN が現役エントリと重複する場合は**新規作成せず既存を返す**（CR-019・全経路で冪等） |
| `update_entry` | `id: i64, input: EntryInput` | `Result<EntryDetail>` |
| `set_starred` | `id: i64, starred: bool` | `Result<()>` |
| `trash_entry` | `id: i64` | `Result<()>` — ソフト削除（`deleted_at` をセット） |
| `restore_entry` | `id: i64` | `Result<()>` — ゴミ箱から復元。復元後に現役エントリと識別子（DOI/arXiv/ISBN）が衝突する場合は `Err`（CR-019） |
| `find_duplicate_entry` | `doi?: String, arxiv_id?: String, isbn?: String` | `Result<Option<i64>>` — 現役エントリのうち canonical 一致する最小 id。UI が作成前に事前チェックする |
| `delete_entry` | `id: i64` | `Result<()>` — ハード削除（永久）。通常 UI からは `trash_entry` を経由。 |
| `fetch_metadata_by_doi` | `doi: String` | `Result<EntryInput>` |
| `fetch_metadata_by_arxiv` | `arxiv_id: String` | `Result<EntryInput>` |
| `fetch_metadata_by_isbn` | `isbn: String` | `Result<EntryInput>` |
| `is_citation_key_available` | `key: String, exclude_id?: i64` | `Result<bool>` — 固定 cite key が使用可能か（サニタイズ後に他エントリと重複しないか）。`exclude_id` は編集中エントリ自身を除外。空キーは常に `true`（自動扱い） |
| `resolve_citation_key` | `entry_id: i64` | `Result<String>` — `.bib` 同期（ゴミ箱を除く全件書き出し）で実際に割り当てられる cite key。`export_bibtex(None)` と同じ並び・衝突回避を再現。詳細ビューの表示/コピー用 |

`create_entry` / `update_entry` の `EntryInput.citation_key` はサニタイズ後 `entries.citation_key` に保存する（空なら NULL = 自動）。既存の固定キーと重複する非 NULL 値は UNIQUE 制約で拒否される（`Result` の `Err`）。UI は保存前に `is_citation_key_available` で検証する。生成・重複回避の規則は `DATA_MODEL.md` の `citation_key` 節を参照。

`create_entry` は識別子（DOI/arXiv/ISBN）の正準値で現役エントリの重複を判定し、一致すれば新規作成せず既存エントリを返す（clipper だけでなく UI/import/LLM の全経路で有効・CR-019）。正規化規則と DB レベルの部分 UNIQUE 制約（best-effort）は `DATA_MODEL.md` の「識別子の canonical 化と重複防止」節を参照。

`get_entries` の `view` は特殊ビュー専用フィルタ。`collection_id` / `tag_id` と組み合わせる場合は `view` は無視され、コレクション/タグの所属で絞られる（いずれも `deleted_at IS NULL` を満たすもののみ）。`search_entries` / `fulltext_search` も同じ `view` を受け取り、`view = "trash"` のときはゴミ箱内（`deleted_at IS NOT NULL`）を、それ以外（省略含む）は現役（`deleted_at IS NULL`）を対象に検索する（CR-001）。これによりゴミ箱ビューでの検索結果に現役エントリが紛れ込まない。

**`filter`（v0.6.0・複合フィルタ）:** `get_entries` / `search_entries` の任意引数。省略・全フィールド空なら従来どおり無制約。scope（`collection_id`/`tag_id`/`view`）や検索クエリと **AND で合成**する。

```ts
type EntryFilter = {
  entry_types?: string[];      // 種別。非空なら entry_type IN (...)（要素どうしは OR）
  year_min?: number;           // year >= year_min
  year_max?: number;           // year <= year_max
  starred?: boolean;           // true=star付きのみ / false=starなしのみ /（省略=指定なし）
  has_attachment?: boolean;    // true=添付あり / false=添付なし /（省略=指定なし）
  tag_ids?: number[];          // 複合タグ。非空なら tag_match で結合
  tag_match?: "and" | "or";    // tag_ids の結合（既定 "or"）。"and"=全タグを含む
};
```

- 各軸どうしは AND。空（未指定）の軸は制約を課さない。フィールドはすべて省略可（Tauri のトップレベル引数と異なり、ネストオブジェクトのキーは serde 定義どおり **snake_case**）
- `tag_ids` は scope の単一 `tag_id` とは独立に AND 合成される（サイドバーでタグ A を選びつつフィルタで B・C を AND 指定、等）
- 全文検索（`fulltext_search`）への `filter` 適用は v0.6.0 では未対応

### BibTeX 自動同期

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_bibtex_sync_path` | — | `Option<String>` — `settings.bibtex_sync_path` の値 |
| `set_bibtex_sync_path` | `path: String` | `Result<()>` — 設定後に即同期リクエストを送る |
| `clear_bibtex_sync_path` | — | `Result<()>` — 同期を無効化 |
| `get_bibtex_exclude_abstract_note` | — | `Result<bool>` — `settings.bibtex.exclude_abstract_note`（`"1"` で true） |
| `set_bibtex_exclude_abstract_note` | `exclude: bool` | `Result<()>` — 設定後に即同期リクエストを送る。BibTeX 出力（同期・エクスポート・MCP）から abstract / note を除外する |
| `pick_bibtex_sync_path` | `default_name?: String` | `Result<Option<String>>` — 保存ダイアログを開き選択パスを返す（キャンセル時 None） |
| `sync_bibtex_now` | — | `Result<()>` — debounce をバイパスして即時書き出し |

BibTeX 出力時はフィールド値の TeX 特殊文字（`_ & % # $ { } ~ ^ \`）を自動エスケープする（biber/biblatex のパースエラー防止）。ただし biblatex の verbatim フィールド（`url` / `doi` / `eprint`）と数値・ISBN は URL/DOI を壊さないようエスケープしない。また `$…$` / `$$…$$` の数式区間は意図的な LaTeX とみなし保護する（区間内はエスケープしない）。誤検出防止のため、開き `$` の直後・閉じ `$` の直前が空白の組（例: `between $5 and $10`）は数式とみなさない。

ミューテーション系コマンド（`create_entry` / `update_entry` / `delete_entry` / `trash_entry` / `restore_entry` / `bulk_*` / `import_bibtex`）が呼ばれると、内部の `sync_tx` 経由でコーディネーターに通知される。チャットの write 系ツール（`llm::tools::is_local_write_tool`）が成功した場合も同様に通知される（MCP サーバー経由の write は従来どおり）。コーディネーターは 800ms の trailing-edge デバウンスで `bibtex::sync_bibtex` を呼び出し、書き込み完了/失敗を `bibtex-synced` イベントで UI に通知する。

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
| `bulk_purge` | `ids: Vec<i64>` | `Result<()>` — **ゴミ箱内（`deleted_at IS NOT NULL`）の id だけ**を hard delete。現役エントリの id が混ざっても無視する（CR-001）。entries_fts と fulltext もまとめてクリーンアップ |
| `empty_trash` | なし | `Result<()>` — ゴミ箱を空にする。表示中 id ではなく DB 側で `deleted_at IS NOT NULL` を評価するため、検索・フィルタで現役が混ざっても安全（CR-001） |
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
| `download_arxiv_pdf` | `entry_id: i64, arxiv_id: String` | `Result<Attachment>` — arXiv PDF を DL して添付（v0.7.0） |
| `download_arxiv_source` | `entry_id: i64, arxiv_id: String, automatic: bool` | `Result<Attachment>` — arXiv TeX ソース（e-print）を DL して添付（LCIR Phase 4）。**`automatic: true`（自動経路）のときだけ同意面 AND `lcir.enabled` を要求**し、閉じていれば通信前に `Err("tex_autofetch_disabled")`（v1.0.0・下記） |
| `delete_attachment` | `id: i64` | `Result<()>` |
| `open_attachment` | `id: i64` | `Result<()>` |
| `index_attachment` | `id: i64` | `Result<{pages, outcome}>` — `pages` = 実行後の索引済みページ数 / `outcome` = `lcir` \| `pdf_extract` \| `skipped_ocr` \| `skipped_lcir` \| `skipped_empty_extract`（v1.0.0-p1。守って何もしなかった場合を UI が区別できるようにするため。**v1.0.0 で `skipped_empty_extract` を追加** = 抽出が 1 ページも本文を返さなかったので既存索引を残した） |
| `is_attachment_indexed` | `id: i64` | `Result<bool>` — fulltext 行が 1 件以上あるか |
| `unindex_attachment` | `id: i64` | `Result<()>` — 全文索引だけを消す（PDF 実体・LCIR には触らない）。**v1.0.0 で詳細パネルにボタンを出した** —— 空抽出で既存索引を守るようにした結果、再索引ボタンが「索引を捨てる」手段ではなくなったため |
| `index_missing_attachments` | — | `Result<IndexMissingResult>` — 未索引 PDF を一括索引（v0.7.0） |

`index_attachment` はその添付の全文索引を張り直す（冪等）。**呼び出し元は詳細パネルの索引/再索引ボタンだけ**（v1.0.0-p2 以降）── 添付直後の索引は `ingestion::ingest_new_pdf_attachment` が決定点を `replace_existing = false` で呼ぶ側に移った。**「`add_attachment` 後に自動で呼ばれる」という配線を復活させないこと**: このコマンドは名指しの再索引なので `replace_existing = true` で決定点を呼び、自動 build が張った LCIR 由来の索引を pdf-extract で上書きし返す（勝敗はタイミング次第・下記 :448 と同じ話）。**v1.0.0-p1 以降、テキストの出どころは 1 つの決定点が選ぶ**（下記）。

`index_missing_attachments` は、まだ全文索引の無い PDF 添付（ゴミ箱を除く）を `db::fulltext::attachments_without_fulltext` で洗い出し、順に索引する。過去に添付済み・自動索引を逃したエントリの後追い用（設定 → データの「未索引の PDF を一括索引」）。`IndexMissingResult = {total, indexed, needs_ocr, failed, skipped}`（`skipped` = 既存の索引を守って触らなかった添付数・v1.0.0-p1）。

**索引ソースの決定点（v1.0.0-p1）:** **pdf_extract を使う本番経路 5 つ**（添付 3 経路 + `index_attachment` + `index_missing_attachments`）はすべて `ingestion::index_fulltext_for_attachment` を通る。OCR の保存（`llm::tools::ocr::save_ocr_pages`）と LCIR からの再導出バッチは別の writer で、そちらの保護は `index_attachment_from_lcir` / `index_attachment_from_pdf_extract` の**トランザクション内の判定**が担う。判断は 3 段で、①この添付の索引が **OCR 由来**として記録されていれば触らない → ②`lcir.enabled` かつ LCIR の page ノードに本文があれば **LCIR から派生**（`regenerate_page_fts_from_lcir`）→ ③どちらでもなければ従来どおり `pdf-extract`。③は判定と書き込みを同一トランザクションで行い、**LCIR / OCR 由来の索引には譲る**（添付直後に spawn した `pdf-extract` は数十秒かかるので、後から書き戻すと LCIR 由来の索引を壊す = debt-17）。**さらに v1.0.0 で「抽出が 1 ページも本文を返さなかったとき、既存の索引行が 1 行以上あるなら書かない」を同じ tx に足した**（`skipped_empty_extract`）—— `replace_pages` は無条件に `DELETE` してから空ページを `INSERT` しないので、テキスト層の無い PDF で `Ok(全ページ空)` が返ると**削除だけが走って索引が 0 行になる**（確認ダイアログの無い再索引ボタン 1 回で、課金して起こした OCR 転写が消えていた）。守るのは**非空 0 件のときだけ**で、縮小（既存 500 行 → 新規 1 行）は守らない。既存が 0 行なら従来どおり書く（出どころ記録の後始末が飛ぶと、行が無いのに `source` だけ残って以後の自動経路が永久に譲る）。索引を捨てたいときは `unindex_attachment`。

索引の出どころは **`settings` の添付単位キー `fulltext.source.<attachment_id>`**（値 `lcir` / `ocr`・キーが無ければ `pdf-extract` 由来か未索引）に記録する。`fulltext` は FTS5 仮想表なので provenance 列を足せず（`virtual tables may not be altered`）、側表を足すと migration が要るため。索引や添付を消すときは同じトランザクションでこの記録も消す。

**添付後の自動取り込み（CR-027 / v1.0.0-p2）:** 手動添付（`add_attachment`）・arXiv 取得（`download_arxiv_pdf`）・Web クリッパー（MCP `spawn_pdf_job`）のいずれの経路も、添付成功後に共有ヘルパ `ingestion::ingest_new_pdf_attachment` を 1 回呼ぶ（best-effort・スキャン PDF は OCR へ誘導）。ヘルパは **①上記の決定点で全文索引 → ②`lcir.enabled` が ON なら LCIR を自動 build** の順に走る。

**順序が「索引 → build」である理由**: build を先にすると、テキスト層の無いスキャン本で全文索引が最長 8 分（実測 att37）遅れ、その間その論文は検索に出ない。索引を先にしても収束先は同じで、build の中の `regenerate_page_fts_from_lcir` が pdf-extract 由来を LCIR 由来へ置き換える（逆向きは起きない — 自動経路の `index_attachment_from_pdf_extract` は LCIR 由来に譲る）。build が固まってもプロセスが落ちても全文検索だけは生き残る。

**build を決定点の中に入れない理由**: 決定点は添付経路以外からも呼ばれる（`index_attachment` と `index_missing_attachments`）。そちらにも build が配られると、秒オーダーで終わるはずの「未索引の PDF を一括索引」ボタンが pdfium の全件バッチに化ける。

自動取り込みは開始と終了に `attachment-ingest` イベント（`{ attachment_id, busy }`）を発火する。フロントはこれで索引インジケータ（`StatusBar`）を出す。**以前はフロントが添付成功後に `index_attachment` を invoke して自前で数えていたが、あれは `replace_existing = true`（＝名指しの再索引）を自動経路から呼ぶ形で、自動 build が張った LCIR 由来の索引を pdf-extract で上書きし返す競合になるため v1.0.0-p2 で削除した。**

`download_arxiv_pdf` は、arXiv からメタデータ取得してエントリを作成した直後に「PDF も一括で取得する」ためのコマンド（AddSheet の arXiv タブのチェックボックス。デフォルト ON）。`arxiv_id` を正規化して `https://arxiv.org/pdf/<id>` を `download::download_and_attach`（50MB 上限・`%PDF-` マジックバイト検証・タイムアウト付き）でダウンロードし添付、成功後はバックグラウンドで `pdf-extract` → 全文索引を試みる（索引失敗は無視）。ペイウォールやネットワーク障害で失敗しても呼び出し側はエントリ作成を成功扱いにする（フロントは警告ログのみで詳細パネルからの手動添付に誘導）。

`download_arxiv_source`（LCIR Phase 4）は、同じ正規化 ID で `https://arxiv.org/e-print/<id>` から **TeX ソース**（gzip された tar または単一 .tex）をダウンロードし、`arxiv-<id>-source.gz`・mime `application/gzip` として添付する。同じ SSRF ガード/リダイレクト検証/50MB 上限を共有する。応答検証は `%PDF-`（PDF-only submission = TeX 未公開の明示エラー）と HTML（エラーページ）を弾き、それ以外は受理して形式判定（gzip/tar/単一 .tex）は LCIR ビルド側の内容スニッフィングに委ねる。**再取得は既存の TeX ソース添付を上書きする**（別添付を積まない — 中身が変われば sha256 → content_key が変わり、次のビルドが新版を作って旧版を supersede する）。**全文索引は行わない**（PDF ではないため）。

**引数 `automatic: bool` が同意面の要否を決める**（v1.0.0・ゲート ②b の W2-1）。`false` = ユーザーがそのボタンを押した（詳細パネル）＝ **明示操作そのものが同意**なので同意面を問わず取りに行く。`true` = アプリが自動で決めた（AddSheet の arXiv 追加）＝ `ingestion::tex_autofetch_enabled`（**同意 AND `lcir.enabled`**）を要求し、閉じていれば**通信する前に** `Err("tex_autofetch_disabled")` を返す。フロントはこのエラーを失敗として見せない（ユーザーは何も頼んでいない）。**判定をフロントに任せない** — p3 まではここが無ゲートで、`AddSheet.tsx` が同意だけを見ていたため `lcir.enabled` を明示 OFF にしたユーザーの arXiv 追加で毎回数 MB を落としていた。

LCIR ビルド（`build_lcir_for_attachment`）は呼び出し側が添付成功後に実行する — 経路は **4 つ**: 詳細パネルのボタン（明示・await）／AddSheet の arXiv 追加（`tex_autofetch_enabled` のとき自動・fire-and-forget）／Web クリッパー（`tex_autofetch_enabled` のとき自動・`spawn_tex_source_job`）／設定→データの「TeX ソースを一括取得」（`fetch_missing_arxiv_sources`）。

```ts
type IndexMissingResult = {
  total: number;     // 処理対象（未索引 PDF）の総数
  indexed: number;   // テキストを抽出して索引できた数
  needs_ocr: number; // 0 ページ＝テキストレイヤー無し（OCR 候補）
  failed: number;    // 読み込み/抽出に失敗した数
  skipped: number;   // 既存の索引（OCR / LCIR 由来）を守って触らなかった数（v1.0.0-p1）
};
```

### 検索（search）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `search_entries` | `query: String, collection_id?: i64, tag_id?: i64, view?: String, filter?: EntryFilter` | `Vec<EntrySummary>` |
| `fulltext_search` | `query: String, collection_id?: i64, tag_id?: i64, view?: String` | `Vec<FulltextResult>` |

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
- `filter`（v0.6.0）が指定された場合は、FTS ヒットをさらに `EntryFilter` の条件で AND 絞り込みする（上記「文献」節の型定義参照）
- 並び順: BM25 ランクスコア降順
- 空クエリは呼び出さない（フロント側で `get_entries` にフォールバック）

将来 `fulltext_search`（PDF ページ単位）を実装する際は、結果型を `Vec<SearchHit>` に拡張する形で `search_entries` 内に統合する想定。

### LCIR（機械可読中間形式）— `lcir.enabled`（**v1.0.0-p3 で既定 ON**）

論文全文を型付きノード木 + PDF 座標 + provenance で保存する中間表現。設計は `docs/LCIR_design_overview.md`、スキーマは `DATA_MODEL.md`「LCIR 関連テーブル」。settings `lcir.enabled = "1"` のときだけ動く追加の side-build（既存 `fulltext` 検索は不変）。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `build_lcir_for_attachment` | `attachment_id: i64` | `LcirBuildResult` — 添付 1 件の LCIR を構築（詳細パネルの「LCIR を構築 / 再構築」・TeX 取得後の追い build・`AddSheet` の添付後 build）。背景の build が走っていて上限（15 秒）内にロックが取れなければ **`build_busy`**（失敗ではない ── 順番が来れば構築されるし起動時バックフィルの対象にもなる）。**v1.0.0: 既存の completed 版を作り替える場合に限り、代替テキスト生成 / TeX 一括取得の実行中は `already_running`**（②b の W1-6 の PR-3 レビュー指摘 ── 作り替えると走行中の課金バッチの書き込み先が消える）。**新規添付の初回 build は弾かない**（supersede する版が無いので行き先を動かさない。無条件に弾くと課金中に追加した論文がバックフィルの周期まで LCIR を持たない）|
| `get_lcir_document` | `attachment_id: i64` | `LcirDocument \| null` |
| `build_missing_lcir` | — | `LcirBatchResult` — 多重起動ガード（`already_running`）+ `lcir-build-progress {done,total}` 進捗イベント（**総数が確定した直後に `(0, total)` を 1 回**・②b の F-2）。**v1.0.0: 代替テキスト生成 / TeX 一括取得の実行中も `already_running`**（②b の W1-6。裁定は `begin_lcir_batch` の 1 か所で build / rebuild / rederive / gc の 4 本に効く）|
| `rebuild_outdated_lcir` | — | `LcirBatchResult` — 同上。**既存ライブラリに新フェーズの成果（定理・参照グラフ・記号・図・表）を行き渡らせる唯一の経路**。設定 → データのボタンから実行 |
| `fetch_missing_arxiv_sources` | — | `{enabled, total, fetched, built, pdf_only, failed, aborted}` — ゴミ箱以外で arxiv_id を持ち gzip 添付が無いエントリの e-print を直列取得（3 秒スロットル）して LCIR 構築。**v1.0.0: 他の LCIR バッチ（build / rebuild / rederive / gc / 代替テキスト生成）が走っていたら `already_running`**（②b の W1-6 の PR-3 レビュー指摘 ── 取得ごとに `build_lcir_for_attachment` を呼ぶので実質 build バッチ。ここだけ相手を 1 つも見ておらず「後から始める」5 組が素通りしていた。弾けないと、GC が build ロックを握っている間は上限なしで待って**進捗表示のまま無言で止まる**）+ 多重起動ガード + `tex-fetch-progress {done,total}` 進捗イベント（**総数が確定した直後に `(0, total)` を 1 回**・②b の F-2）+ 完了時 `entries-changed`。**同意面は `ingestion::tex_autofetch_enabled`（同意 AND `lcir.enabled`）**で、実行中も毎回読み直して外されたら `aborted: true` で打ち切る。**判定は多重起動ガードを取った後に置く**（v1.0.0・ゲート ②b の F-1）── 前に置くと `record_success` を通らず `batch_status.last` に載らないので、`last` だけを見る設定モーダルでは**押しても完全に無反応**になる。閉じていたときは `enabled: false` で返し、**「対象 0 件」と別の文言**を出す（両方 `total: 0` だが意味が違う） |
| `rederive_fulltext_from_lcir` | — | `FulltextDeriveResult = {total, derived, skipped_ocr, skipped_empty, skipped_existing, failed}` — **v1.0.0-p1**。既存ライブラリの全文索引を LCIR の page ノードから張り直す。pdfium を使わない純 SQL なので**実測 12〜20 秒 / 138 添付**（進捗イベントは出さない）。OCR 由来の索引（`skipped_ocr`）と、LCIR に本文が 1 ページも無い添付（`skipped_empty`）は触らない。**v1.0.0 で分類の優先順を「出どころ優先」に揃えた** —— OCR 済みのスキャン本は LCIR の本文も 0 ページなので両方に当てはまるが、`skipped_ocr`（守った）に数える。`skipped_empty`（＝まだ OCR が要る候補）に数えると、実ライブラリでは `skipped_ocr` が構造的に 0 件になり UI の文言が誰にも当たらない。`build_missing_lcir` と同じ排他を使う（2 本目は `already_running`）。**起動時にも 1 回だけ走るが、そちらは「索引がまだ無い添付を埋めるだけ」**（`AddMissingOnly`）── `fulltext.source.*` はこの版で初めて書かれるキーで、**記録の無い既存索引には 3 つの母集団がある** —— ①この版より前に入った索引 ②OCR がページを書いてから封印するまでの窓 ③中断・部分 OCR（封印しないので記録が立たない・debt-43）。どれも課金して得た転写でありうるので、**自動の再導出だけ**は置き換えない。⚠ **「置き換えるのはこのボタンだけ」ではない** —— ①このボタン ②詳細パネルの再索引ボタン ③**LCIR の build**（`regenerate_page_fts_from_lcir` は `protect_unrecorded = false` で呼ばれる）の 3 経路が置き換える。③を守れないのは、添付時の「pdf_extract で索引 → build が LCIR へ置き換え」という p1 の設計そのものが止まるため（記録なしが『旧 OCR』か『たった今 pdf_extract で張った新規』か区別できない = debt-37）。**v1.0.0 でこの判定を書き込みと同じトランザクションの中へ畳んだ** —— 以前は tx の外で `indexed_page_count(..).unwrap_or(0)` を見ており、(a) 読みが Err のとき「索引なし」に倒れてそのまま置き換え、(b) 読んでから書くまでの窓に部分 OCR が着地すると素通りしていた。今は読めなければ tx ごと失敗する（＝ 1 行も書かない）。一度きりフラグは `settings.fts.fulltext_lcir_derived`（`lcir.enabled` が OFF / 対象 0 件 / 失敗ありのときは立てないので、後から ON にした・後から build した場合に走る） |
| `lcir_storage_stats` | — | `{file_bytes, used_bytes, free_bytes, gc: GcPreview}` — **v1.0.0-p4**。DB ファイルのページ収支（`(page_count − freelist_count) × page_size`・実測でファイルサイズと 1 バイト一致）と superseded 版の回収見積り。`GcPreview = {versions, versions_removable, versions_tombstoned, nodes, asset_rows, asset_bytes, alt_texts_protected, carry_refs_protected, orphan_versions_skipped}`。**`lcir.enabled` で gate しない**（切った人ほど旧版を消したい）。読み取りのみで実測 0.06 秒 |
| `run_lcir_gc` | — | `GcOutcome` — **v1.0.0-p4・非可逆**。superseded 版を回収する。`GcOutcome = {versions_removed, versions_tombstoned, versions_skipped, nodes_removed, asset_rows_removed, files_trashed, files_already_gone, fts_orphans_removed, freed_bytes, db_size}`（**`files_already_gone` は「消しに行ったら既に無かった crop」を `files_trashed` と別に数える** ── 合算すると「1 枚も当たらなかった」異常が正常と同じ見た目になる。⚠ **この 2 つの和は `asset_rows_removed` と一致しない**のが正常 ── ファイル側の 2 つは重複除去と「生存版が指しているパスの除外」を通した**後**のパスにしか付かず、`move_to_trash` が `Err` を返した回はどちらも増えない。版をまたいで crop ディレクトリを共有するので実ライブラリではさらに開く）。`build_missing_lcir` と同じガードを使い（2 本目は `already_running`）、Vision / TeX 取得が走っていても `already_running`（**v1.0.0 でこの判定は `begin_lcir_batch` に移り、build / rebuild / rederive にも効くようになった** ── ②b の W1-6）。`lcir-gc-progress {done,total}` を版単位で emit（`(0, total)` から）。**`freed_bytes` は `freelist` の実測差分**で按分推定ではない。**ファイルサイズは縮まない**（free page になるだけ・次のバックアップと次の再構築で回収される） |
| `lcir_batch_status` | — | `{running: BatchKind[], progress: {[kind]: {done,total}}, last: {kind, finished_at, result, error} | null}` — **debt-32**。長時間バッチの実行状態・進捗・**直近に終わった 1 本の結果**を返す**読み取り専用**コマンド。`BatchKind = "build" | "rebuild" | "rederive" | "gc" | "vision_alt_text" | "tex_fetch" | "ocr"`。`running` が**配列**なのは全種別が排他ではないため（`ocr` は LCIR 系とは独立に走る。v1.0.0 で LCIR 系の 6 種は**3 つの入口**（`begin_lcir_batch` / `begin_vision_alt_text_batch` / `begin_tex_fetch_batch`）が互いを見る形で両方向に排他になった ── ②b の W1-6。**排他の外に残る build が 2 種ある**: p2 の自動 build（ユーザー操作ではないので返す先が無い。起動時バックフィルだけは添付境界ごとに譲る）と、**supersede しない 1 件 build**）。**`progress` は総数が確定した時点で `{done: 0, total: N}` として現れる**（②b の F-2。以前は 1 件目が終わるまでキーごと無く、att37 では最大 8 分ぶんの盲窓になっていた）。⚠ **`rederive` にはそもそも進捗が現れない**（`derive_page_fts_from_lcir_once` は純 SQL で秒オーダーなので `set_progress` を呼ばない）。`running` には出る。`last.result` は**そのコマンドの戻り値そのもの**で、文言整形は i18n を持つフロントの仕事。**読んでも `last` は消えない**（2 つ開いた画面のうち先に読んだ方だけが見られる状態を作らないため）ので、再掲の抑制は `finished_at` を見るフロント側の担当。**排他に弾かれた `already_running` は `last` に載らない**（バッチが走っていないので） |
| `search_lcir_nodes` | `query, collection_id?, tag_id?, view?` | `NodeFtsHit[]` |
| `get_lcir_node_region` | `node_id: i64` | `{node_id, attachment_id, source: "pdf"\|"tex", page: i64\|null, bbox: [f64;4]\|null} \| null` — **Phase 10b で拡張**（旧: `SourceFragment`）。ノードの代表領域に**所属添付**を添えて返すので、これ 1 本で `open_pdf_viewer` を呼べる。TeX 由来の版は座標を持たないので `page`/`bbox` は null（`attachment_id` も TeX ソース添付を指すためPDF ビューアには渡せない）。**`page` と `bbox` が揃うときだけ**ジャンプ可能と判断すること |
| `get_lcir_enabled` / `set_lcir_enabled` | `—` / `enabled: bool` | `bool` / `()` — **v1.0.0-p3 で既定 ON**。判定は「`"0"` でなければ ON」で、**「未設定」と「明示 OFF」を区別する**（未設定 = 一度も触っていない = ON）。この不変条件は `set_lcir_enabled` が `"0"`/`"1"` しか書かず、他に書く本番経路が無く、migration に seed も無いことに依存する |
| `get_lcir_tex_autofetch_enabled` / `set_lcir_tex_autofetch_enabled` | `—` / `enabled: bool` | `bool` / `()` — **v1.0.0-p3**: settings `lcir.tex_autofetch.enabled`。arXiv から e-print を**自動で**取得してよいかの同意で、`lcir.enabled` とは独立（`lcir.vision_alt_text.enabled` と同型）。**未設定のときの既定は「この版より前に `lcir.enabled` を明示 ON にしていたか」** —— 既定 ON に反転した `lcir.enabled` で判定すると新規ユーザー全員に自動ダウンロードが付いてくるため、生の保存値が `"1"` かだけを見る。起動時に `backfill_tex_autofetch_consent` が 1 回だけ明示値（`"1"`/`"0"`）へ確定させる（**`"0"` でも書く** —— 未設定を残すと、後からユーザーが `lcir.enabled` を入れ直したときに同意していない取得が有効になる）。従うのは 4 経路: クリップ時の自動取得 / 重複クリップの欠落補完 / `fetch_missing_arxiv_sources` / AddSheet の arXiv 追加。**`get_` が返すのは同意の生値**（チェックボックス表示用）で、Rust 側では `ConsentForDisplay` という newtype に包んであり**そのまま判定に使えない** —— 取りに行ってよいかは `get_lcir_tex_autofetch_effective`（＝同意 AND `lcir.enabled`）を使う |
| `get_lcir_tex_autofetch_effective` | — | `bool` — **v1.0.0（ゲート ②b の W2-1）**。`ingestion::tex_autofetch_enabled` の実効値（同意 AND `lcir.enabled`）。フロントが「自動取得するかどうか」を先に判断する場所（AddSheet の arXiv タブ）が使う。**チェックボックスの表示には使わない**（そちらは同意の生値）。2 本に分けているのは、1 本しか無いと必ず取り違えるため —— W2-1 はまさにそれで、AddSheet が同意だけを見ていた |
| `get_lcir_vision_alt_text_enabled` / `set_lcir_vision_alt_text_enabled` | `—` / `enabled: bool` | `bool` / `()` — **Phase 8c**: settings `lcir.vision_alt_text.enabled`（既定 off・`lcir.enabled` とは独立の同意面）。**`get_` が返すのは同意の生値**（チェックボックス表示用）で、Rust 側では `ConsentForDisplay` newtype に包んであり**そのまま判定に使えない** —— 課金してよいかは `ingestion::vision_alt_text_allowed`（同意 AND `lcir.enabled`）|
| `count_figures_missing_alt_text` | `entry_id?: i64, attachment_id?: i64` | `i64` — **Phase 8c**: 代替テキストがまだ無い図の件数（読み取り専用・フラグ非依存）。**課金の前に規模を見せる**ため設定画面と詳細パネルが引く。絞り込み・しきい値は生成バッチと同一述語 |
| `generate_vision_alt_texts` | `entry_id?: i64, attachment_id?: i64` | `{enabled, total, generated, skipped, stale, failed, aborted, abort_reason}` — **Phase 8c**: alt text の無い `figure` ノードの crop PNG を LLM Vision に説明させて `node_alt_texts` に保存する後追いバッチ。1 図ずつ best-effort（1 図の失敗で全体を捨てない）・リクエスト間 1 秒スロットル・多重起動ガード + `vision-alt-text-progress {done,total}` 進捗イベント（**総数が確定した直後に `(0, total)` を 1 回**出す ── v1.0.0・②b の F-2）。`lcir.enabled` / `lcir.vision_alt_text.enabled` のどちらかが OFF なら `enabled: false` で全 0（**課金する操作なので暗黙には走らせない**）。**同じ添付の版を作り替えるバッチ（build / rebuild / rederive / gc / TeX 一括取得）が走っていたら `already_running`**（v1.0.0・②b の W1-6。逆向きは `begin_lcir_batch` / `begin_tex_fetch_batch` が持つので**両方向で排他**。弾けないのは (a) p2 の自動 build（ユーザー操作でないため）と (b) **supersede しない 1 件 build**の 2 種だけ）。`skipped` = crop ファイル欠損・空応答。**`stale` = 実行中にその添付が再構築されて書き込み先の版が最新でなくなった図**（v1.0.0・②b の W2-4。`skipped` と分けてある ── 混ぜると「説明できなかった」と「行き先が消えた」が同じ数字になる）。判定は 2 段で、Vision を呼ぶ**前**（大多数・課金なし）と、**書き込みと同じ 1 文の中**（`insert_alt_text_if_version_is_latest`・レースなので 1 添付につき高々 1 件・課金済みで stderr に 1 行）。**書き込み側は版だけでなく crop の指紋（`assets.sha256`）も見る** ── 版が動かないまま `heal_missing_assets` が欠けた crop を再レンダリングすると絵が変わるので、古い指紋のまま書くと `assets.sha256 == node_alt_texts.source_asset_sha256` の不変量を破った行が旧版に残る（v1.0.0・PR-3 のレビュー指摘）。次回の実行で新版の同じ図が対象に戻る。対象は**ゴミ箱のエントリを除外**し、**短辺 200px 未満の crop（ロゴ・装飾等の小片）も除外**する（`DEFAULT_MIN_CROP_PX`。実蔵書では 1198 crop のうち 310 件が該当）。`entry_id`/`attachment_id` で範囲を絞れる（**まず 1 本で品質と費用を確かめてから広げる**ため。詳細パネルのボタンはエントリ単位で呼ぶ）。同一ラン内で crop 指紋が一致する図は API を呼ばず複製する。**打ち切り**は `aborted: true` + `abort_reason`: `"failures"`（1 件も生成できないまま連続 3 件失敗 = キー不正/レート制限/画像非対応モデル）/ `"lcir_disabled"` / `"consent_withdrawn"`（実行中に**同意面が閉じた** = 実質のキャンセル・毎図の先頭で再評価する）。未処理の図は対象のまま残り次回に拾える。**どちらの面で閉じたかは区別する** ── LCIR を切って止めた人に「同意チェックが外された」と説明すると嘘になるため（`ingestion::VisionGate`）。

**毎図の再評価は入口と同じ `vision_alt_text_allowed`（同意 AND `lcir.enabled`）を読む**（v1.0.0・ゲート ②b の W1-4）── ここが同意だけを読んでいた頃は、**LCIR を切っても課金が止まらず**、しかも設定 UI が同意チェックを `disabled` にするので**止める手段が同時に消えた**。同意面が閉じているときの結果も、多重起動ガードを取った後に判定して `record_success` するので `batch_status.last` に載る（`already_running` だけは載らない）|
| `export_lcir_json` | `entry_id: i64, source?: "tex" \| "pdf"` | `Result<{path: String\|null, warnings: ExportWarning[]}>` — **Phase 9a**: 保存ダイアログで LCIR JSON（`LcirDocument` 派生ビュー・validation 通過必須）を書き出す。キャンセルで `path: null`（**警告は返す**）。LCIR 未構築はエラー（`available_sources` 相当の案内文）。`warnings` は下記「エクスポート欠落警告」 |
| `export_lcir_markdown` | `entry_id: i64, source?: "tex" \| "pdf"` | `Result<{path: String\|null, warnings: ExportWarning[]}>` — **Phase 9a**: 保存ダイアログで構造付き Markdown を書き出す。キャンセルで `path: null`（**警告は返す**） |

**LCIR エクスポート（Phase 9a・v0.10.0 予定）**: エントリ→版解決は MCP と同じ共有ロジック（`ingestion::load_entry_lcir`・添付ごとの最新 completed 版を `extractor_priority`（tex > pdfium）で並べ、`source` 指定時はその抽出器に限定）。Markdown は `export::markdown::render_markdown`（**決定的純関数**・pdfium 非依存で CI テスト可能）が `LcirDocument` を描画する: YAML フロントマター（title/authors/year/doi/arxiv/citation_key + `lcir_source` = 抽出器名・版で由来を明示）→ 節見出し（`section_number` 付き・`##`〜）→ 段落（インライン数式 `$..$` は生 LaTeX のまま温存）→ display 数式（`latex` があれば `$$..$$`・**無ければ surface-only の Unicode 線形をそのまま段落に出し `$$` を付けない** — 生 LaTeX でないものを数式と偽らない）→ 定理/補題/証明（blockquote・`theorem_number`/`note` 付き）→ 図表 caption（イタリック）→ 参考文献（`cite_key` 付きリスト）。未知の `node_kind` は plain_text の段落に degrade（Phase 7/8 のノード型追加でレンダラが壊れない）。`document`/`page`/`line` ノードと **`page` の全文 `plain_text` は描画しない**（ブロックと重複するため）。

**エクスポート欠落警告（Phase 9 完了条件・debt-8）**: 「LCIR 固有情報が失われる場合に警告を出せる」の実装。
`export::warning` に置き、Phase 9b（HTML/JATS/TEI）も同じチャネルを共有する。`render_markdown` /
`lcir_json_pretty` は `ExportReport { text, warnings }` を返す。**警告はエラーではない**（書き出しは成功している）。
`document_ir::validation`（不正な LCIR を弾く・Err）とは別物。形式ごとの表現力を `FormatCapabilities` で宣言し、
警告はそこから機械的に導く。**その文書に実際に存在するデータのうち出力に現れないものだけ**を報告する
（relations が 0 本なら relations の警告は出ない）。どの形式でも常に落ちる縮約（ノード id・`content_key`・
schema URI 等）は警告にしない — 狼少年にしないため。

| code | severity | 発火条件 | count |
|------|----------|----------|-------|
| `relations_dropped` | warn | 形式が関係を運べず `relations` がある | 辺数（`detail` に type 別内訳） |
| `symbols_dropped` | warn | 形式が記号定義を運べず `symbols` がある | 記号数 |
| `inferred_provenance_dropped` | warn | 形式が provenance を運べず推定 origin のノードがある | ノード数（`detail` に origin 別内訳） |
| `source_fragments_dropped` | info | 形式が座標を運べず fragment がある | fragment 数 |
| `assets_not_embedded` | info | 形式がアセット実体を同梱できず asset がある | asset 数 |
| `table_cell_spans_flattened` | info | 形式が結合セルを運べず `colspan`/`rowspan > 1` のセルがある | セル数 |

推定 origin は `layout_model` / `llm_inference` / `math_recognition` / `ocr`（`pdf_text_layer` / `tex_source` は
原文由来なので数えない）。並びは `(severity, code)` で決定的。**Markdown** は全項目が落ちうる形式、
**LCIR JSON** は無損失なので `assets_not_embedded` しか出ない。UI は保存後に一覧表示（i18n キーは
`detailPanel.lcirExportWarn.<code>`）、CLI は stderr に 1 行ずつ。

```ts
type LcirBuildResult = {
  enabled: boolean;      // lcir.enabled が off なら false（何もしない）※ v1.0.0-p3 で既定 ON
  built: boolean;        // 新規に構築したか
  reused: boolean;       // 同一 content_key の既存を再利用したか（冪等）
  version_id: number | null;
  content_key: string | null;
  page_count: number;    // TeX 版（Phase 4）は 0（message にブロック数が入る）
  message: string;
};

// PDF 座標付きの木（正本は SQLite、これはその JSON 派生ビュー）
type LcirDocument = {
  schema: string;
  schema_version: string;
  version_id: number;
  content_key: string;
  source: { sha256: string; mime_type: string; extractor_name: string; extractor_version: string };
  coordinate_space?: { space: string; origin: string; unit: string; y_axis: string }; // PDF 由来のみ（TeX 由来は座標を持たないため省略）
  nodes: Array<{
    id: number;
    kind: string;           // document / page / section / paragraph / figure_caption / line / ...
    ordinal: number;
    parent_id?: number;
    plain_text?: string;
    origin?: string;        // pdf_text_layer（原文由来） / layout_model（構造推定）
    confidence?: number;
    payload?: unknown;      // 型固有属性。見出しは { heading_level, section_number }
    math?: {                // 数式（Phase 3/4・inline_math/display_math のみ）
      display_mode: string;           // inline / display
      equation_label?: string;        // "(2.1)" 等（TeX 由来は \tag{X} → "(X)"）
      normalized_text?: string;       // 検索用の正規化線形文字列
      latex?: string;                 // TeX 由来は原文スニペット（Phase 4）。PDF 由来は undefined
      presentation_mathml?: string; content_mathml?: string; openmath?: string; // 後続フェーズ
      semantic_status: string;        // PDF 由来は surface_only / TeX 由来は source_provided
      confidence?: number; origin?: string; // TeX 由来は origin=tex_source
    };
    source_fragments: Array<{ page: number; bbox: { x: number; y: number; width: number; height: number }; fragment_type?: string }>;
  }>;
};

// ノード単位 FTS（Phase 2）のヒット。段落・見出し・caption 等のブロック粒度で当たる。
type NodeFtsHit = {
  entry: EntrySummary;
  attachment_id: number;
  node_id: number;
  page: number;
  node_kind: string;        // paragraph / section / figure_caption / ...
  snippet: string;
  bbox: { x: number; y: number; width: number; height: number } | null; // ブロック領域（ハイライト用）
};
```

- `build_lcir_for_attachment` は pdfium で抽出し `document_versions`/`document_nodes`/`source_fragments` を作る。`content_key`（= `sha256(source_sha256|extractor_name|extractor_version|config_hash)`）で冪等：同一 PDF+同一抽出器版なら再抽出せず reuse。新版採用時は同一添付の旧 completed を `superseded` にする。
- **Phase 2**: 抽出後に論理構造を認識し `document > page > block(段落/見出し/caption 等) > line` の木にする（`extractor_version` 0.1.0→0.2.0）。build 時に派生の `document_nodes_fts` も張り、`search_lcir_nodes` がブロック粒度で検索できる。ヒットの `bbox` で該当ブロックを直接ハイライトできる（`get_lcir_node_region` でも個別取得可）。
- **Phase 3**: 独立した数式を `display_math` ノードとして認識し `math_expressions`（表層）を作る（`extractor_version` 0.2.0→0.3.0）。PDF 由来は `semantic_status='surface_only'`・`normalized_text` のみ埋め、LaTeX/MathML は Phase 4（TeX）以降。`get_lcir_document` の該当ノードに `math` が付く。制御文字（pdfium のグリフ化け）は除去する。
- **Phase 4（TeX 取込）**: `build_lcir_for_attachment` は添付の **mime だけ**で抽出器を選ぶ（バッチ対象クエリと同一述語） — `%pdf%` は pdfium、`application/gzip`（`download_arxiv_source` が登録する唯一の値）は **`lumencite-tex`**（独自 semver 0.1.0）、それ以外はエラー。手動での .tex 添付は Phase 4 のスコープ外（`add_attachment` は PDF 前提のため）。gzip 添付の中身は内容スニッフィングで tar / 単一 .tex / PDF-only を判定する。TeX 版の木は `document > block` フラット（page/line/fragment 無し・`LcirBuildResult.page_count` は 0 で `message` にブロック数）。display 数式は**生 LaTeX** を `math_expressions.latex` に `semantic_status='source_provided'`・`origin='tex_source'` で保存する。`\input`/`\include` は tar 内で再帰解決（循環ガード・欠落は warning）。冪等・supersede は従来どおり添付単位（PDF 版と TeX 版は別添付なので互いに supersede しない）。**TeX 版は `document_nodes_fts`/`fulltext` に索引しない**（同一エントリの PDF 版と重複ヒットし bbox も無いため。検索は PDF 版・読み出しは TeX 優先という分担）。
- **Phase 5（定理・定義・証明）**: `definition`/`theorem`/`lemma`/`proposition`/`corollary`/`remark`/`example`/`proof` ノードを認識する（**新規テーブルなし**・`extractor_version` は pdfium 0.3.0→**0.4.0** / TeX 0.1.0→**0.2.0**）。**TeX** は環境名から種別を決め（preamble の `\newtheorem{env}{Display}` を回収し表示名から独自名・略記を対応づけ、標準英名 + amsthm 予約の `proof` は既定マップ）、`[note]` と `\label` を `payload_json`（`note`/`labels`）へ。本文は 1 ブロックに collapse し内側 display 数式は生 LaTeX のまま残す（別ノード化しない）。**PDF** は行頭キーワード + 番号 + 終端記号で認識し `payload_json` に `{theorem_number, note}`（参照文中の "Theorem 2 shows …" は棄却）。定理間参照グラフ（`proves` 等）は Phase 6a（`node_relations`）で張る。読み出しは汎用経路（`is_content_block` blacklist・node-FTS・`get_document_blocks` の `kinds`）で追加改修なく surface する。
- **Phase 6a（参照グラフ）**: ノード間の参照を `node_relations`（migration 0017）に有向辺として張る（**新規ノード型なし**・`extractor_version` は pdfium 0.4.0→**0.5.0** / TeX 0.1.0系列の 0.2.0→**0.3.0**）。build のトランザクション内で純関数 `ingestion::graph::resolve_relations` が解決する。**TeX** は本文に原文のまま残る `\ref`/`\eqref`/`\cite` を `\label`（`payload.labels`）/ `\bibitem` cite key と照合（`origin='tex_source'`・confidence 0.9）。**PDF** は本文の "Theorem 2.3"/"Eq. (2.1)" を定理番号/数式番号と照合（`origin='layout_model'`・confidence 0.6–0.7・PDF は `\label` 復元不可のため番号一致）。proof→theorem の `proves` は TeX が読み順の直前（または `\ref` 先）、PDF が "Proof of Theorem 2.3" の番号一致（無ければ直前）。**解決できない参照・自己参照は張らない**（誤検出より欠損）。`relation_type` = `cites`/`refers_to_equation`/`refers_to_theorem`/`refers_to_figure`/`refers_to_table`/`refers_to_section`/`refers_to`/`proves`。`LcirDocument` に文書レベルの `relations:[{from_node_id, relation_type, to_node_id, confidence, origin, metadata}]` が載る（`get_lcir_document` / MCP から読める）。記号系（`symbols`/`symbol_occurrences`）は Phase 6b。
- **Phase 6b（記号系）**: 論文が定義する記号を `symbols`/`symbol_occurrences`（migration 0018）に持つ（**新規ノード型なし**・`TEX_EXTRACTOR_VERSION` 0.3.0→**0.4.0**・PDF 版は不変）。build のトランザクション内で純関数 `ingestion::symbols::extract_symbols` が、TeX 本文のインライン数式 `$...$` を定義文（"let $U$ be ...", "define $H$ as ...", "denote by $\mathcal{H}$ ...", "$U := ...$"）から記号として取り出し、説明・型（best-effort）・定義ノード・スコープ（直前の節）を付ける。**TeX 版のみ**（PDF はインライン数式を切り出せない・PDF-only エントリは空）。surface/description は原文 verbatim だが対応づけは推定なので `confidence` 中程度・`origin='tex_source'`。出現は保守的に **display 数式内の表層一致**のみ。`LcirDocument.symbols:[{surface_form, normalized_form, description, symbol_type, defined_at_node_id, scope_node_id, confidence, origin, occurrences:[{node_id, surface_form}]}]`。
- **Phase 8a（図表アセット基盤）**: PDF build がページ内の埋込画像（トップレベル Image オブジェクトのみ）から図領域を検出し、`figure` ノード + ページ crop PNG（`assets`/`node_assets`・migration 0019）+ `caption_of` 辺を作る（`extractor_version` pdfium 0.5.0→**0.6.0**・TeX 不変）。ファイルは `attachments/<entry_id>/.lcir/<attachment_id>/<content_key16>/` に原子的（tmp+rename）に書き、build 失敗時は best-effort 削除・成功 commit 後に旧 content_key ディレクトリを trash（GC）。reuse 経路はファイル欠損を検知すると再抽出で自己修復する。詳細は `DATA_MODEL.md`「assets / node_assets」。
- **Phase 8b（表セル構造化）**: TeX build が table float 内および裸の `tabular`/`tabular*`/`tabularx` をセル構造化して `table` ノードを作る（**新規テーブルなし**・`TEX_EXTRACTOR_VERSION` 0.4.0→**0.5.0**・pdfium 不変）。純関数 `ingestion::tex::tabular::parse_tabular` が行 × セルの grid（`colspan`/`rowspan`/`rule_above`/`alignments`）を `payload_json` に載せ、原文スニペットも `latex_source` として保存する（40k 以下）。同一 table 環境由来の caption とは `caption_of` 辺（caption → table・conf 0.95・origin=tex_source）。確信の持てない表（ネスト環境・列数超過・`longtable`/`tabu`/subfloat）は構造化せず warning に理由を残して従来どおり破棄する。version `metadata_json` に `table_count`。詳細は `DATA_MODEL.md`「Phase 8b」。
- **Phase 8c（図の代替テキスト）**: `figure` ノードの crop PNG を LLM Vision に説明させ `node_alt_texts`（migration 0020）に保存する（**新規ノード型なし・抽出器版は不変** — 生成は build の外）。生成は opt-in バッチ `generate_vision_alt_texts` のみで、`lcir.enabled` と `lcir.vision_alt_text.enabled`（既定 off・独立の同意面）の両方 ON が条件。**build には混ぜない**（Vision は非同期・課金・非決定的で content_key の冪等性を壊す）。既に alt text がある `figure` は対象外なので再実行しても再課金しない（空応答の図は行が無いので再試行される）。対象は**ゴミ箱のエントリを除外**する。抽出器版を上げた再構築では、crop PNG の SHA-256 が一致する `llm_inference` 行を過去の全版から新版へ carry（`carried_from_version_id` に由来版）し、同 tx で現版以外の `llm_inference` 行のうち**新版にも同一指紋の画像がある**ものだけを刈る（crop 書き出しが一部失敗した図の行は残す）。`user_edited` 行は carry も削除もしない。原文 caption は上書きせず、生成文は `fulltext`/`document_nodes_fts` に索引しない。詳細は `DATA_MODEL.md`「node_alt_texts」。
- `rebuild_outdated_lcir` は旧抽出器版で作った LCIR を現行版へ再構築する（`build_missing_lcir` は未構築のみ・こちらは版が古いものを対象）。どちらも mime で対象抽出器を選び、pdfium 版・TeX 版それぞれの現行 semver と比較する。**両者は同じ添付・同じ表を触る長時間処理（PDF 1 本ごとに pdfium 抽出 + ページレンダ + crop 書き出し）なのでプロセス全体で 1 本に絞り**（2 本目は `already_running`）、1 添付ごとに `lcir-build-progress {done,total}` を emit する。1 添付の失敗はバッチを止めず `failed` に数えて次へ進む。**添付 1 件だけを現行版で作り直したいときは詳細パネルの添付行のボタン**（`build_lcir_for_attachment`・content_key が変われば新版を作って旧版を supersede するので「未構築」と「旧版」で操作は同じ）。
- 座標は既存 `highlights` と同一系（PDF user space・左下原点・pt）。
- フラグ OFF なら書き込み系は DB に一切書かず（`build`/バッチは `enabled:false`、`get` は `null`）、既存挙動は不変。`search_lcir_nodes` はフラグに関係なく空表を引くだけ。
- **外部 LLM 向け（MCP サーバー・Phase 3.5/4）**: 上記 LCIR を MCP read ツール `get_document_structure`（節アウトライン＋カウント＋abstract）／`get_document_blocks`（構造タグ付きブロック・`kinds`/`page` フィルタ）／`search_document_nodes`（ブロック粒度検索＋`bbox`・PDF 版のみ）として公開する（**両者は各ブロックの `node_id` を返す**（Phase 10a で追加）。`get_document_blocks` の `index` はその応答内の通番でしかないので、`get_node_context` / `get_node_relations` に渡すのは `node_id` の方）（`docs/SPEC.md`「MCP サーバー公開」）。未構築エントリは `has_lcir:false`。**Phase 4**: エントリに複数表現があるときは **TeX 版を優先**して読む（`document_ir::extractor_priority`）。両ツールに `source` 引数（`"tex"`/`"pdf"`）で表現を明示切替でき、応答に `source`（採用した抽出器）と `available_sources`（併存する表現一覧）が付く。TeX 由来の数式ブロックは `latex`（原文）を返す。`get_document_structure` は `block_count`（本文ブロック総数）を常に返し、TeX 版では `page_count` を `null` にする（ページを持たないため）。`page` フィルタは PDF 版専用（`source` 未指定で `page` を渡すと PDF 版に自動フォールバック。`source:"tex"` と併用時や PDF 版不在時は黙って空にせず明示メッセージ）。**Phase 5**: 定理・証明ノードは `is_content_block` なので追加改修なく両ツールに流れる。`get_document_structure` の `counts` に `theorem`/`proof`/… が汎用的に現れ、`get_document_blocks(kinds:["theorem","proof"])` で「定理と証明を一問い合わせ」で取得でき、各ブロックに `theorem_number`・`note`（付記名）が付く。**Phase 6a**: 参照グラフを読む `get_node_relations`（entry_id/citation_key・`source` 切替・`relation_type`/`node_id` フィルタ）を追加。各辺の端点を `{node_id, kind, page, snippet, theorem_number?, equation_label?, section_number?, labels?}` に enrich し、`counts_by_type` と併存表現 `available_sources` を返す。「この証明は何を証明するか」「式 (2.1) を参照/使用するのは何か」「この節が参照する結果は何か」を一問い合わせで解ける。**Phase 6b**: 記号定義を読む `get_symbol_definitions`（`symbol` 完全一致 / `query` 部分一致フィルタ）を追加。各記号に `defined_at`（定義ノード・"jump to definition"）・`scope`（節）・`occurrences`（出現数式 + `equation_label`）を付ける。**TeX 版のみ**（PDF-only エントリは count 0）。「$U$ とは何か」「記号一覧」「$\mathcal{H}$ はどこで定義されるか」「$\gamma$ を使う式は」を一問い合わせで解ける。**Phase 8a**: 図を読む `get_figures`（entry_id/citation_key）を追加。**PDF 版のみ**（TeX 版に図領域は無い。PDF 版不在は `has_lcir:false` 系メッセージ）。各図に `{node_id, page, bbox, figure_number?, caption:{node_id, text}?, assets:[{role, relative_path, mime_type, width, height, size_bytes}]}` を返す。caption は `caption_of` 辺から解決。**`relative_path` は app data dir 相対のメタデータ参照であり、ファイルの存在は保証しない**（欠損許容・base64 は返さない）。`figure` ノードは `is_content_block` なので `get_document_structure` の `counts` にも現れ、`get_document_blocks` では `figure_number`・`asset_count` 付きの空テキストブロックとして流れる。`get_node_relations` の `relation_type` に `caption_of`（caption → figure）が加わる。`LcirNode` に `assets:[{role, mime_type, relative_path, width, height, size_bytes, sha256, metadata}]` が載り、`get_lcir_document` / LCIR JSON エクスポートに透過で現れる（Markdown レンダラは figure を存在マーカー `**[Figure 3]** (p. 5)` として描画する。**画像リンクは張らない** — `relative_path` は app data dir 相対の内部参照で `.md` の保存先から解決できず、存在保証も無いため）。**Phase 8b**: 表を読む `get_tables`（entry_id/citation_key・`max_tables` 既定 20・`max_chars` 予算既定 24k）を追加。**TeX 版のみ**（PDF-only エントリは `has_lcir:false` + TeX 取得を案内するメッセージ）。各表に `{node_id, caption:{node_id, text}?, column_spec, n_columns, n_rows, alignments?, rows:[{cells:[{text, colspan?, rowspan?}], rule_above?}]}` を返す（caption は `caption_of` 辺から解決・`latex_source` は返さない — rows が構造を持ち二重送出になるため）。`table` ノードは `is_content_block` なので `counts` にも現れ、`get_document_blocks` では " \| " 結合の可読テキスト + 寸法（`column_spec`/`n_columns`/`n_rows`）付きで流れる（セル構造は `get_tables` へ）。Markdown エクスポートは GFM パイプテーブルとして描画する（アライメントは build 時確定の `alignments` を使用・セル内 `|` は数式内 `\vert `/数式外 `\|` の二層エスケープ・payload の無い table ノードは plain_text 段落に degrade）。**Phase 8c**: `get_figures` の各図に `alt_text:{text, origin, confidence, model}?` が付く（**LLM Vision の生成物**であることを `origin='llm_inference'` と `model` で明示・原文 caption とは別フィールドで併存し上書きしない）。バッチ未実行・生成対象外の図では欠落する（欠損許容）。`LcirNode.alt_text` として `get_lcir_document` / LCIR JSON エクスポートにも透過で載る。Markdown エクスポートは figure マーカー直下に blockquote で出し、由来ラベル（`**AI-generated description** (model: X).` / `**Figure description (user-edited)**.`）を必ず添える（原文 caption と区別する・§16）。`confidence` は可視出力に出さない（「意味の正しさの尺度ではない」ので正答率と誤読される）。 **Phase 8d-7**: PDF 版の参照グラフに `refers_to_figure` / `refers_to_table` が加わる（本文の "Figure 3" / "Fig. 3" / "Table 2" を図表番号と照合・`origin='layout_model'`・confidence 0.6）。端点は図領域が検出できていれば `figure` ノード、できなければ `figure_caption` / `table_caption` ノードで、`metadata.resolved_via` が `"node"` / `"caption"` を区別する（bbox が引けるのは `"node"` のときだけ・caption から実体へは `caption_of` を 1 ホップ）。**実データでは caption 宛が主経路**（figure ノードの図番号保有率は 2 割強・PDF 側に table ノードは無い）。端点 enrich に `figure_number` / `caption_number` が加わる。**複数形・範囲参照（"Figures 3 and 4"）と同一版で番号が衝突する図表には辺を張らない**ので、辺が無いことは「本文に参照が無い」ことを意味しない。 **Phase 10a**: 1 ブロックの読解文脈を server-side で結合する `get_node_context`（**引数は `node_id` のみ**）を追加。他の LCIR ツールと違い entry_id / `source` を取らない — **版はノード id が決める**（エントリ起点の tex > pdf 優先で版を選び直すと、呼び出し側が握っている id が引けない版に化けるため。superseded 版のノードも読める）。返すのは `{focus, section_path, before, continuation, continuation_stopped_at, proofs, proves, premises, equations, figures, citations, references, notes}` + 封筒 `{node_id, found, entry_id, citation_key, attachment_id, version_id, source, extractor_version, available_sources}`（**空のリストはキーごと省略される**）。`continuation_stopped_at` は `{reason, node_id?, kind?}` で、`boundary`（次の論理単位が始まった — `kind` が `figure_caption`/`table_caption` ならフロートに割り込まれただけで主張は続きうる）/ `max_continuation` / `max_continuation_chars` / `end_of_document` を区別する。**`continuation` が空/短いことの意味はこれを読まないと決まらない**。**`continuation` が中核**: PDF 版の定理ノードは主張の先頭 1 レイアウトブロックしか持たず（実測 平均 168 字・TeX 版は 975 字）、続きは page 直下の兄弟に落ちて 33%（proof は 53%）がページをまたぐ。木の pre-order で読み順を作り、次の構造境界の手前まで連結することで完了条件「ページ境界で文脈が切れない」を満たす。辺は**焦点 + `continuation`** の両方から拾う（PDF では参照が続きのブロックに乗るため）。`premises`（前提定義）は辺だけでは 1.4% しか埋まらないので 3 経路を `via` で区別して返す: `reference`（`refers_to_theorem` の指し先が `definition`・正確だが希少）/ `occurrence`（`symbol_occurrences` の記録・TeX）/ `symbol`（記号の表層が本文に `$X$` で現れ、かつ定義が読み順で前にある・TeX・非永続の読み側照合）。図表参照は `caption_of` を解決して `{node（辺の指し先）, figure（領域・crop・alt text の持ち主）, caption}` の 3 点で返す（実測で caption の 3/4 は実体に到達できないので `figure` は欠落しうる）。**2 ホップはしない** — 定理のバンドルに証明の参照まで畳み込むと長さが予測不能になるので、`proofs` の `node_id` で呼び直す設計。届かなかったものは `notes`（`float_entity_unreachable` / `proves_target_is_not_a_theorem` / `continuation_truncated` / `related_truncated` / `premises_truncated` / `focus_is_not_a_content_block`）に機械可読コードで出す。**表現ごとに恒真な事実（TeX に座標が無い / PDF に記号が無い）は notes に載せない** — `source` から機械的に決まり、PDF 版バンドルの 100% に付いてしまうので注記としての情報量が無い（規約②）。何も欠けていなければ `notes` は省略される。読み順スパンの境界判定は **`heading_level` を宣言している見出しだけ**を境界にする（実ライブラリの PDF `heading` の 94% はレベル宣言を持たない数式断片で、境界に数えると定理の主張がその式の直前で切れる）。`export::ExportWarning`（「この**形式**では運べない」）とは別チャネル（こちらは「この**バンドル**が届かなかった」）だが、「狼少年にしない 3 規約」は共有する。 **Phase 10b**: これら文献本文の read ツール 9 種（`get_fulltext` + LCIR 8 種）の定義と実行は `llm::tools::document` へ移設し、**MCP とアプリ内チャットで単一ソース**にした（`mcp_server` は `tool_specs` / `exec_tool` から委譲する。`tools/list` の並び順と各ツールの入出力は移設前と同じ）。あわせて次を追加した — `search_document_nodes` に `max_results`（既定 50・上限 200）と `truncated`、ヒット 0 件時の `index_built`（「一致しない」と「まだ索引が無い」の区別）、チャットのスコープで検索範囲が選択エントリに限定されたときの `scope_filtered`（**絞り込みは SQL に押し込む** — 上限を SQL の LIMIT に落とす以上、スコープを取得後に Rust 側で落とすとライブラリ全体の上位 N を先に取ってしまい、選択したエントリのブロックがそこに入らなければ 0 件になる。MCP 経路では常に付かない）／`get_document_blocks` の各ブロックに `origin` と `confidence`（原文由来と推定の区別をデータ側で運ぶ。従来は `get_node_context` にしか無かった）／`get_node_context` の各サイズ引数に上限（`max_before` 8 / `max_continuation` 64 / `max_continuation_chars` 40000 / `max_related` 50 / `max_premises` 20）。いずれも追加のみで、既存クライアントの読み方は変わらない。

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
| `get_highlights` | `entry_id: i64` | `Result<Vec<Highlight>>` — ページ昇順、同ページ内は `y` 降順（エントリ全添付を含む） |
| `get_highlights_by_attachment` | `attachment_id: i64` | `Result<Vec<Highlight>>` — 指定添付 PDF のハイライトのみ（CR-015）。UI は選択中の添付でこれを使う |
| `create_highlight` | `input: HighlightInput` | `Result<Highlight>` — `input.attachment_id` で属す添付を指定 |
| `update_highlight` | `id: i64, color?: HighlightColor, note?: String` | `Result<Highlight>` — 部分更新 |
| `delete_highlight` | `id: i64` | `Result<()>` |

メタパネル「ハイライト」タブの一覧表示で、クリックすると該当ページにジャンプする想定。

### LLM

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `generate_summary` | `entry_id: i64, source: "abstract" \| "fulltext", channel: Channel<SummaryStreamEvent>` | `Result<()>` — ストリーミング送出。完了時にDB側 `entries.summary` も更新 |
| `cancel_summary` | `entry_id: i64` | `Result<()>` — 進行中の要約生成を中断（sheet close / 再生成時にフロントが呼ぶ）。LLM future を drop して有料 HTTP リクエストを実際に停止。対応 run が無ければ no-op（CR-034） |
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
| `run_backup_now` | — | `Result<String>` — 作成された `.zip` のパス。DB（`VACUUM INTO`）＋添付本体を同梱（CR-018） |
| `list_backups` | — | `Result<Vec<BackupInfo>>` — `<app_data_dir>/backups/` 配下のメタ情報（`.zip`／旧 `.db` 両対応） |
| `open_backup_folder` | — | `Result<()>` — バックアップフォルダを OS のファイラで開く |
| `pick_backup_archive` | — | `Result<Option<String>>` — 復元用のバックアップ `.zip` を選ぶダイアログ。キャンセルで `None`（CR-018） |
| `restore_from_archive` | `path: String` | `Result<()>` — バックアップ `.zip` から復元を**ステージング**。検証＋復元前の自動バックアップ後に成功。実際の DB 差し替えは次回起動時（CR-018） |
| `export_database_json` | — | `Result<Option<String>>` — 保存ダイアログで `EntryDetail[]` を JSON 書き出し（メタデータのみ） |
| `export_database_markdown` | — | `Result<Option<String>>` — 保存ダイアログで notes＋summary を Markdown 書き出し（メタデータのみ） |

```ts
type BackupInfo = { path: string; file_name: string; created_at: string; size_bytes: number };
```

**完全バックアップ（CR-018）**: `run_backup_now` は `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.zip` を作る。アーカイブ内レイアウトは `db.sqlite`（DB 全体＝highlights/chat/settings/fulltext 込み）＋ `attachments/<entry_id>/<file_name>`（添付本体）。deflate 圧縮。14 世代保持。自動バックアップは Rust 側で起動時 + 24h 間隔のタイマーから呼ばれ、前回成功（`settings.backup.last_run`）から 24h 未満なら間引かれる（`run_backup_if_due`）。`run_backup_now` は間引かず常に実行する。アーカイブは `<stem>.zip.partial` に書いてから `<stem>.zip` へ rename するため、途中終了しても中身の欠けたアーカイブが一覧・世代管理に混ざらない。

**走査中に消えたエントリ（②b W2-5）**: DB スナップショット（`VACUUM INTO`）は先頭で固まるのに `attachments/` の走査は実測 7〜9 分かかるので、その間の削除・trash 送りでファイルが消えるのは**通常のレース**。消えたエントリ（`NotFound`）は**飛ばして続行**し、一覧を `SKIPPED.txt`（先頭 200 件）としてアーカイブに同梱する（stderr にも総数と先頭 5 件を出す）。`SKIPPED.txt` は復元の allowlist（`db.sqlite` / `attachments/` 配下のみ）に当たらないので展開時は無視される ── **自動で読むものは無い**（復元時に警告を出すのは debt-45）。**`NotFound` 以外（容量不足・権限）はこれまでどおりバックアップ全体を失敗させる** ── 中身の欠けたアーカイブを「成功」として並べないため。

⚠ **記録できるのは「その親ディレクトリを `read_dir` で列挙した後に消えた」ものだけ**（列挙は各ディレクトリに到達した時点で 1 回）。したがって窓の長さは階層で桁違いに違い、**エントリ丸ごと削除は走査のほぼ全域が窓**なのに対し、添付 1 件の削除や crop の回収は**そのディレクトリを詰める一瞬だけ**。`SKIPPED.txt` があれば確実に欠けているが、**無いことは完全性の証明にならない**（完成後に DB と突き合わせる検算は debt-45）。なお `write_atomic` の作業ファイル（`<name>.tmp`）が消えた場合は正常動作なので記録しない（狼少年にしないため）。

- **復元（CR-018）**: `restore_from_archive(path)` はライブ DB を握ったまま差し替える危険を避けるため **2 フェーズ**で動く。①稼働中に `.zip` を検証（`db.sqlite` 存在・`PRAGMA integrity_check`・スキーマ版がアプリ以下か）し、**復元前に現行状態を自動フルバックアップ**したうえで `<app_data_dir>/pending-restore/` へ展開＋マーカー設置。②次回起動時、pool を開く前に現行 DB（＋ `-wal`/`-shm`）と `attachments/` を `<app_data_dir>/pre-restore/` へ退避し、staged を所定位置へ移す（失敗時は退避物から自動ロールバックし、旧 DB のまま起動継続）。フロントは `restore_from_archive` 成功後に `@tauri-apps/plugin-process` の `relaunch()` で再起動する。
- `export_database_json` / `export_database_markdown` は再インポート不可の**メタデータ書き出し**（PDF・ハイライト・チャット・設定は含まない）。

### アップデーター（v0.1.0 追加）

アプリ内更新（DL + 検証 + 再起動）は `@tauri-apps/plugin-updater` の JS API（`check()` / `update.downloadAndInstall()`・`src/lib/updater.ts`）をフロントから直接呼ぶ。専用の Rust ラッパーコマンド（`check_for_updates` / `apply_update` / `get_updater_channel` / `set_updater_channel`）は存在しない。バックエンド側に独自コマンドとして存在するのは通知のみの経路 `check_latest_github_release` だけ。更新チャンネル切替（stable/beta）は未実装（UI も非表示）。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `check_latest_github_release` | — | `Result<GithubReleaseInfo>` — **v0.5.0**: GitHub Releases API で最新 tag を取得し `env!("CARGO_PKG_VERSION")` と semver 比較（下記） |

**`check_latest_github_release`（v0.5.0・通知のみの更新確認）:** `tauri-plugin-updater` とは独立した経路。`latest.json` は darwin エントリしか持たないため Windows/Linux では updater の `check()` が新版を見つけられない。この経路は GitHub API（`repos/marmot1123/LumenCite/releases/latest`）で全 OS 共通に新版有無を判定し、**DL/インストールはせず** `html_url`（Releases ページ）を返すだけなので updater 署名鍵も `latest.json` も不要で全 OS 安全。戻り値 `GithubReleaseInfo { current_version, latest_version, is_newer, html_url, body? }`。`is_newer` は tag（先頭 `v` 除去）と現行の semver 比較で、どちらか解釈不能なら `false`（誤って更新を促さない）。フロントの更新タブは updater `check()` と本コマンドを並行実行し、updater が `available` を返せば従来のアプリ内更新、そうでなく `is_newer` なら「Releases を開く」通知バナーを表示する。

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
| `chat_tool_refs` | `tool_name: String, result_json: String` | `ToolResultRef[]` — **Phase 10b**。ツール結果テキストから根拠参照 `{node_id, kind, page}` を取り出す純関数（DB も state も触らない）。ライブ配信では `ChatStreamEvent::ToolCallExecuted.refs` に載って来るので**セッションを開き直したときだけ**使う（ライブの `result_summary` は 500 文字で切られていてフロントでは JSON として読めない。抽出ロジックを TS 側に写さないためのコマンド） |
| `open_pdf_viewer` | `id: i64（attachment）, page?: i64, region?: [f64;4]` | `Result<()>` — 別ウィンドウの PDF ビューアを開く/フォーカスする。`region`（**Phase 10b**・PDF user space・左下原点・pt）を渡すとそのページの該当領域を一時強調する。既に開いているウィンドウへは `emit_to` で**宛先を指定して** `jump-to-region` / `jump-to-page` を送る（`emit` は全ウィンドウ broadcast なので、2 枚目のビューアが別の論文の同じ位置を強調してしまう） |

`chat_send_message` の `channel` は `tauri::ipc::Channel<ChatStreamEvent>`。`tool_call_executed` は **Phase 10b** で `refs: ToolResultRef[]` を持つ（結果が指す PDF 上の根拠。`page` を持つものだけ・最大 5 件。TeX 由来の LCIR には座標が無いので TeX 版を読んだときは常に空）。`tool_call_proposed` の `needs_approval=true` を受けたら UI は承認ダイアログを出し、`approve_tool_call` で応答する。承認制御はツール別ホワイトリスト（DATA_MODEL の `chat.tool_whitelist` 参照）に従う:

- read 系（`fulltext_search` / `get_entry` / `list_*` ＋ **Phase 10b** の文献本文 9 種 = `get_fulltext` / `get_document_structure` / `get_document_blocks` / `search_document_nodes` / `get_node_relations` / `get_symbol_definitions` / `get_figures` / `get_tables` / `get_node_context`）: 常に自動。集合は `llm::tools::approval::READ_ONLY_TOOLS` が正本で、frontend の `src/chat/tools.ts` と一致させる（あちらはカードの色分けと「一覧を再読込するか」の判定に使うので、片方だけ足すと read ツールが write 扱いで表示され毎回一覧が再読込される）
- `add_tag` / `update_notes` / `attach_ocr_text` / `add_to_collection`: デフォルト自動（設定で都度承認に変更可）
- `create_entry` / `update_entry`: 都度承認
- `delete_*` / MCP の write 系: 常時確認（ホワイトリストで上書き不可）

`create_entry` / `update_entry` は基本フィールド（`title` / `entry_type` / `year` / `abstract_` / `doi` / `isbn` / `arxiv_id` / `url` / `notes` / `author_names` / `citation_key`）に加え、型固有フィールドを `extra_fields`（`{string: string}`）で受け付ける（`journal` / `volume` / `issue` / `number` / `pages` / `publisher` / `booktitle` / `address` / `edition` / `series` / `school` / `institution` / `organization` / `howpublished` など、`DATA_MODEL.md` の `entries.extra_fields` 参照）。`update_entry` では指定したキーのみ上書き/追加し、未指定の既存 `extra_fields` は保持する。

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
| `get_mcp_server_config_snippet` | `client: String` | `Result<String>` — クライアント別の貼り付け設定。`"claude_code"` は `claude mcp add --transport http ...` コマンド、`"claude_desktop"` は本体を `--mcp-stdio` shim として起動する `mcpServers` JSON（**Phase 3**）、`"codex"` は `~/.codex/config.toml` の `[mcp_servers.lumencite]` TOML（同じ `--mcp-stdio` shim を stdio 起動。**v0.5.0**）、それ以外は URL + ヘッダ |

**Phase 3（stdio shim）:** Claude Desktop は stdio トランスポートのみ対応しリモート HTTP MCP に直結できない。本体バイナリを `--mcp-stdio` 付きで起動すると（`main.rs` が GUI 起動前に検出）、Tauri を立ち上げず `mcp_shim::run_stdio_proxy` が「stdio ↔ localhost HTTP」プロキシとして動作し、`LUMENCITE_MCP_URL` / `LUMENCITE_MCP_TOKEN`（Claude Desktop 設定の `env`）を使って内蔵 MCP サーバーへ橋渡しする。別 sidecar バイナリにしないことで追加の署名・notarize 対象を増やさない。`claude_desktop` スニペットの `command` は `std::env::current_exe()` の絶対パス。

**Codex（OpenAI CLI）対応（v0.5.0）:** Codex も stdio MCP のみ対応のため、`claude_desktop` と同じ `--mcp-stdio` shim を流用する。`"codex"` スニペットは `~/.codex/config.toml` に追記する `[mcp_servers.lumencite]` テーブル（`command`=実行ファイル絶対パス・`args`=`["--mcp-stdio"]`・`env` に URL/トークン）。TOML 基本文字列を使い Windows パスの `\` をエスケープする。

**公開ツール（MCP `tools/list`）:**
- **read 系（常時）**: `fulltext_search` / `get_entry` / `list_collections` / `list_tags`（チャットの read ツール定義を流用）＋ `search_entries`（メタデータ FTS）/ `resolve_citation_key`（実 cite key）/ `export_bibtex`（.bib テキスト）/ `find_entries_by_citation_keys`（**v0.6.0**: cite key → entry 逆引き）/ `get_fulltext`（**v0.6.0**: 指定エントリの PDF 全文）。
  - **cite key 逆引き（v0.6.0）**: ユーザー（と LaTeX ソース）が持っているのは entry_id ではなく `\cite{}` キーなので、キーから直接引ける経路を追加した。3 点セット:
    - `find_entries_by_citation_keys` — `citation_keys`（文字列配列）→ 各キーの `{citation_key, found, entry_id?, title?, year?, authors?}` を返す。`\cite` キー群 → entry の解決を 1 コールでバッチ処理。未知キーは `found:false`。入力順・重複除去。
    - `export_bibtex` に **`citation_keys`（文字列配列）**を追加。指定時は該当エントリのみを **全ライブラリ同期時と同一の cite key（`smith2020a` のような接尾辞も維持）**で書き出し、JSON `{bibtex, found, missing}` を返す（`\cite` キー → refs.bib 生成の中核）。`export_bibtex(Some(entry_ids))` はサブセット内で再 dedup するためこの用途には使えない点に注意。`entry_ids` も `citation_keys` も省略すれば従来どおり全件 `.bib` テキスト。
    - `get_entry` は `entry_id` に加えて **`citation_key`** を受け付ける（いずれか一方を渡す）。cite key から直接メタデータ取得・要約できる。未解決キーは（`isError` ではなく）「見つからない」旨のテキストを返す。戻り値に **`has_fulltext`**（索引済み PDF 全文の有無）を追加。
    - 逆引きは `bibtex::citation_key_index` / `find_entry_id_by_citation_key` / `export_bibtex_by_keys` が基盤で、`resolve_citation_key` と**同一のキー割当ロジック**（`assign_keys_from`）を共有するため `\cite{}` と必ず一致する。DB 層は Tauri 非依存なので将来の CLI もこの関数群を再利用する。
  - **全文アクセス（v0.6.0）**: `fulltext_search` はキーワード検索（ヒットページのスニペット）だけで、**特定エントリの全文取得**はできなかった。abstract/notes が空だと MCP 経由の要約が一般知識にフォールバックする穴があったため `get_fulltext` を追加。
    - `get_fulltext(entry_id? | citation_key?, max_chars?=24000, page_start?=1)` — 索引済み PDF の抽出テキストを返す。戻り値 `{entry_id, indexed, total_pages, truncated, next_page?, text}`。**索引済み PDF が無ければ `indexed:false`**（テキスト無し）を明示し、クライアントが「全文が無い」と言える（捏造防止）。長い論文はページ単位で切り、`page_start`（前回の `next_page`）で続き読みできる。`max_chars` は 1,000〜200,000 にクランプ。
    - 基盤は `db::fulltext::get_entry_fulltext`（`(page, content)` を `attachment_id, page` 順で返す）と `entry_fulltext_page_count`。アプリ内蔵の `generate_summary`（fulltext ソース）も前者を共有し、全文ロードの単一ソース化。
- **write 系（`mcp_server.write_enabled` 有効時のみ）**: `add_tag` / `update_notes` / `add_to_collection` / `create_entry` / `update_entry`（`mutate` の定義を流用）。**破壊系 `delete_entry` は常に非公開**で、`tools/call` でも許可リスト外として `isError` で拒否する。write 無効時に write ツールを呼ぶと `isError` で拒否。
  - **バルク対応**: `add_tag` / `add_to_collection` は単一 `entry_id` に加えて **`entry_ids`（整数配列）**を受け付け、1 回の呼び出しで複数エントリへ適用する（両者は併用可・重複は順序保持で除去）。ベストエフォートで、存在しないエントリはスキップして成功分を適用し、結果サマリ（適用件数＋スキップ件数）を返す。1 件も成功しなければ `isError`。タグは get-or-create をバッチで 1 回だけ行う。
- write 成功時はサーバーが監査ログ記録＋ `.bib` 同期キック＋ `entries-changed` イベント（一覧ライブ反映）を発火する。

### Web クリッパー（v0.5.0 追加）

Chrome 拡張から起動中アプリへエントリを作成するローカル HTTP API。MCP サーバーと**同一プロセス・同一ポート・同一 Bearer トークン**を共有し、`handle_http_request` にパスベースルーティングを追加して `/clipper` を新設する（既存 JSON-RPC は `/mcp` ほか従来どおりで後方互換）。ゲートは新設定 `clipper.enabled`（"1"/""、デフォルト off）で、`mcp_server.write_enabled` とは独立。サーバープロセスは「`mcp_server.enabled` OR `clipper.enabled`」で起動する。

**HTTP ルート:**

| ルート | 認証 | 説明 |
|--------|------|------|
| `OPTIONS /clipper`・`OPTIONS /clipper/complete` | 不要 | CORS preflight。`Origin` が `chrome-extension://` で始まる場合のみ `Access-Control-Allow-*`（`Access-Control-Allow-Private-Network: true` 含む）を返す（204）。**認証チェックより前に処理**（preflight は Authorization ヘッダを持たないため） |
| `GET /clipper` | Bearer | ペアリング疎通確認。`{"ok":true,"app":"LumenCite","version":"..."}` |
| `POST /clipper` | Bearer | クリップ本体。`clipper.enabled` をリクエスト毎に評価（無効なら 403 `{"status":"error","code":"clipper_disabled"}`） |
| `POST /clipper/complete` | Bearer | 重複エントリの欠落補完。同ゲート。`{entry_id, remember?}`（下記）。catch-all の JSON-RPC より前にルーティングする |

```ts
// POST /clipper リクエストボディ
type ClipRequest = {
  url: string;
  title?: string;
  doi?: string;
  arxiv_id?: string;
  isbn?: string;
  pdf_url?: string;        // citation_pdf_url。無くても arxiv_id から導出する
  published_date?: string; // フォールバック用（先頭4桁を year に）
  site_name?: string;      // フォールバック用（og:site_name → extra_fields.organization）
  authors?: string[];      // citation_author 群（"Given Family"）。フォールバック用
  tags?: string[];         // get-or-create で付与
  collection_id?: number;
};

// 200 応答
type ClipResponse = {
  status: "created" | "duplicate";
  entry_id: number;
  title: string;
  pdf?: "downloading";       // created かつ PDF URL があるとき。添付は応答後に非同期実行
  // duplicate かつ欠落があるとき（arXiv 前提・["pdf"]/["tex"]/両方）:
  confirm_missing?: string[]; // clipper.complete_missing 未設定 → 拡張が確認ポップアップを出す
  completing?: string[];      // clipper.complete_missing=="1" → 応答後に補完ジョブを spawn 済み
};

// POST /clipper/complete リクエストボディ
type CompleteRequest = {
  entry_id: number;
  remember?: boolean;        // true なら clipper.complete_missing="1" を保存
};

// POST /clipper/complete 200 応答
type CompleteResponse = {
  status: "completing";
  entry_id: number;
  completing: string[];      // 実際に補完を開始した欠落（["pdf"]/["tex"]/両方・空なら対象なし）
  remembered: boolean;
};
```

**サーバー側フロー:** `find_duplicate_entry`（DOI/arXiv/ISBN）→ 重複なら `duplicate` 応答（作成も PDF 添付もしない）→ 識別子があれば `metadata::fetch_by_doi/arxiv/isbn` でメタデータ解決 → `create_entry` → PDF URL（明示 or arXiv 導出）があれば**応答後に** `download_and_attach` を spawn（50MB 上限・30 秒タイムアウト・先頭チャンクの `%PDF-` マジック検証。失敗してもエントリは残る）。作成・添付の成功時は `.bib` 同期キック＋ `entries-changed` を発火。

**TeX ソース自動取得（LCIR Phase 4 の自動化）:** arXiv クリップ（arxiv_id あり）で **`lcir.tex_autofetch.enabled` が ON のときだけ**（v1.0.0-p3 で `lcir.enabled` から分離 —— あちらは既定 ON なので、それで gate すると何も操作していないユーザー全員のクリップで外部ダウンロードが始まる）、応答後に `spawn_tex_source_job` が `download_and_attach_arxiv_source` → `build_lcir_for_attachment` を best-effort 実行する（`clipper::derive_tex_source_job` がジョブ発行時に同意を判定）。同意していないユーザーのクリップごとに e-print を落とすことはしない。失敗はログのみでクリップ成功は維持（PDF ジョブと同じ契約）。

**重複クリップ時の欠落補完:** 重複エントリに PDF / TeX ソースが欠けていれば補完する。欠落は `plan_completion(pool, entry_id)` がエントリの `arxiv_id` を唯一の導出源に判定する（PDF 欠落 = mime `%pdf%` 添付なし ／ TeX 欠落 = mime `application/gzip` 添付なし **かつ** `lcir.enabled` ON ／ ゴミ箱は `deleted_at IS NULL` で対象外）。`clipper.complete_missing=="1"` なら重複応答で即 `completing` を返しジョブを spawn、未設定なら `confirm_missing` を返す（この時点では何もしない）。`POST /clipper/complete` はアプリ側で欠落を**再検証**してから `spawn_pdf_job` / `spawn_tex_source_job` を発行し、`remember` なら設定を保存する。PDF URL・arxiv_id はエントリの識別子から再導出する（`citation_pdf_url` 由来の補完は対象外 = arXiv 前提）。拡張の確認ポップアップは表示だけを担い、`/clipper/complete` 呼び出しと popup 状態管理は拡張の service worker が持つ（ポップアップは閉じても補完が中断しない設計）。

**メタデータ解決の規則:**
- 試行順は DOI → arXiv → ISBN（各 10 秒タイムアウト）。ただし **arXiv の DataCite DOI（`10.48550/…`）は CrossRef に無い**ため、arxiv_id があるときは arXiv を先に試す。1 つ失敗しても次の識別子へカスケードする
- 全滅・識別子なしは**フォールバック入力**へ（クリップ自体は失敗させない）: 拡張が送った `title` / `authors` / `published_date` / `site_name` を使い、**arxiv_id があれば `preprint`、無ければ `webpage`** 種別で作成する。識別子は素通しで保存し、後からのクリップでも重複検出が効く
- フォールバックに落ちた理由（タイムアウト / API エラー）は stderr にログする
- クリップの解決処理は serve スレッド上の `block_on` ではなく**ランタイムのワーカーへ spawn** して結果を待つ（本番で動作実績のある PDF ダウンロードと同じ実行モデルに揃える）。serve スレッド自体は応答を返すまで待つため、解決中は他のリクエストが後続待ちになる（上限はタイムアウトの 10 秒）

**Tauri コマンド:**

```ts
type ClipperStatusInfo = {
  enabled: boolean;        // clipper.enabled == "1"
  server_running: boolean; // HTTP サーバースレッドが起動中か
  port: number;
};
```

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_clipper_status` | — | `Result<ClipperStatusInfo>` |
| `set_clipper_enabled` | `enabled: bool` | `Result<ClipperStatusInfo>` — 有効化時はサーバー未起動なら起動。無効化時、`mcp_server.enabled` も off ならサーバー停止 |
| `get_clipper_connect_code` | — | `Result<String>` — 拡張に貼る接続コード。形式は `lc1.` + base64url(`{"v":1,"port":<u16>,"token":"<48hex>"}`)。トークン再生成（`regenerate_mcp_server_token`）でペアリングは無効化される |
| `get_clipper_complete_missing` | — | `Result<bool>` — `clipper.complete_missing == "1"`。AddSheet が確認 vs 自動を判定する |
| `set_clipper_complete_missing` | `enabled: bool` | `Result<()>` — AddSheet の「次回以降は確認しない」で `"1"` を保存 |

### OCR（v0.2.0 追加）

テキストレイヤーのないスキャン PDF を LLM Vision で OCR し、結果を `fulltext` にページ単位で保存する。詳細ビューの手動ボタンと LLM ツール（`ocr_pdf` / `attach_ocr_text`）で内部実装を共有する。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `cancel_ocr` | — | `()` — **v1.0.0**。実行中の OCR を**次のページ境界で**止める。1 ページ = 1 回の課金 API 呼び出しで、ページ数は押す前に分からない（ラスタライズして初めて確定する）ため、**実行中に規模を見て降りられること**が唯一の歯止め。処理済みのページは保存される。**再開は無い** ── もう一度実行すると最初からやり直しで全ページ課金し直しになる（結果文言もそう明言する） |
| `ocr_pdf` | `entry_id: i64, attachment_id?: i64, pages?: Vec<i64>` | `Result<String>` — `attachment_id` 省略時は先頭 PDF、指定時はその添付を OCR（複数 PDF 対応・CR-027）。`pages` 省略時は全ページ。OCR プロバイダは `LlmSettings.ocr_provider` → `provider` のフォールバック。**v1.0.0 で以下が入った**: (a) **1 ページ = 1 課金**なのでループが毎ページ中断要求（`cancel_ocr` のプロセス内フラグ + チャットの停止 `ToolContext::should_stop`）を見る。2 本目は `already_running`（排他は `run_ocr` の中 ── 起動口が 2 つあるため）。⚠ フロントに届く実文字列は `ToolError` の Display が前置した **`"execution error: already_running"`** なので読み手は部分一致で拾うこと (b) `batch_status` の `BatchKind::Ocr` に実行中・進捗・直近結果が載る（**リーダーを離れても設定 → データに停止手段が残る**）。**ループ手前の失敗（添付なし・キー未設定・ラスタライズ失敗）も `record_failure` に載る**。`ocr-progress {done,total}` イベントは UI 起動のランのみ (c) **中断・失敗でも課金済みページは保存**し、届かなかったページがあるなら**部分差し替え**（添付ごと置き換えは完走時だけ）。**空白だけの転写は保存に回さない**（部分差し替えの空ページは既存行の削除になるため）── 結果は `processed`（課金枚数）と `saved`（本文が残った枚数）を分けて返す (d) **`fulltext.source = Ocr` の封印は完走時だけ**（中断・失敗で封印すると、pdf_extract 由来の壊れた既存索引ごと恒久保護してしまう） |

---

## CLI（v0.7.0 追加）

GUI を起動せず、`argv[1]` が `-` 始まりでない語（サブコマンド）または `--help`/`--version` なら本体バイナリをヘッドレス実行する（`--mcp-stdio` shim と同型のディスパッチ。引数なし・`-psn_…` 等は GUI）。DB パスは `dirs::data_dir()` + `com.lumencite.app`（環境変数 `LUMENCITE_DB_PATH` で上書き可）。

- 既定出力は **JSON**（stdout）。`--human` で人間可読テキスト。エラー / 警告は stderr。
- 終了コード: 成功 `0` / 使い方エラー `2` / 実行時エラー `1`。

### 読取コマンド

DB を `PRAGMA query_only = ON` の読取専用プールで直接開く（読取経路の書込を構造的に禁止）。

| コマンド | 引数 / フラグ | 出力 | 再利用する DB 関数 |
|---------|--------------|------|-------------------|
| `search <query…>` | `--collection <id>` `--tag <id>` `--type <t>…` `--year-min <N>` `--year-max <N>` `--starred` `--has-attachment` `--limit <N>` | `EntrySummary[]` | `db::entries::search_entries_filtered` |
| `get <id\|citation_key>` | — | `EntryDetail` | `db::entries::get_entry` / `bibtex::find_entry_id_by_citation_key` |
| `bib <citation_key…>` | — | BibTeX 文字列（stdout）＋未解決キーは stderr 警告 | `bibtex::export_bibtex_by_keys` |
| `export` | `--key <k>…` `--collection <id>` `--tag <id>` ＋ `search` と同じフィルタ軸 | BibTeX 文字列 | `bibtex::export_bibtex_by_keys` / `search_entries_filtered` |
| `tags` | — | `Tag[]` | `db::tags::get_tags` |
| `collections` | — | `Collection[]` | `db::collections::get_collections` |
| `fulltext <query…>` | `--collection <id>` `--tag <id>` | `FulltextHit[]` | `db::fulltext::search_fulltext` |
| `export-lcir <id\|citation_key>` | `--format json\|md`（既定 `json`） `--source tex\|pdf` `-o <path>` | LCIR JSON / Markdown（stdout・`-o` でファイル書き出し。**v0.10.0 / Phase 9a**）。**LCIR 固有情報の欠落警告は stderr**（stdout は本文のままなのでパイプ利用を壊さない）・**終了コードは 0**（警告はエラーではない） | `ingestion::load_entry_lcir` / `export::markdown::render_markdown` |
| `node-context <node_id>` | `--before <N>`（既定 2） `--continuation <N>`（既定 16） `--max-related <N>`（既定 12） | 文脈バンドル JSON（**Phase 10a**）。MCP `get_node_context` と同じ構造 + `entry_id`/`attachment_id`/`source`。**`--source` は無い** — 版はノード id が決める | `ingestion::load_node_lcir` / `context::build_node_context` |

### 書込コマンド（ハイブリッド C）

書込は次のルーティングで実行する（`--force` は全書込コマンド共通のグローバルフラグ）:

1. `--force` → 直接 DB 書込（アプリ起動中なら一覧陳腐化の旨を stderr 警告）。
2. MCP サーバー到達可（keychain トークン有 + `ping` 成功）→ **HTTP 委譲**。サーバーが `mcp_server.write_enabled` ゲート適用＋`.bib` 同期＋GUI 更新。ゲート off なら「有効化 or `--force`」を明示。
3. 到達不可 → **GUI 生存を独立に判定**（`GUI_LOCK_FILE` の advisory ロックを try_lock・CR-011）。
   - GUI 停止を確認できた → **直接 DB 書込** + `.bib` 同期（best-effort）。
   - GUI 起動中（MCP は無効だがアプリは開いている）→ **fail closed** で `--force` を要求。MCP 到達不可を一律「アプリ停止」と解釈して live DB に書き、UI 陳腐化 / WAL 競合を招くのを防ぐ。判定不能の異常時も安全側（起動中扱い）。

どちらも `tools/call`（JSON-RPC）を組み、HTTP は POST、直接は `mcp_server::handle_rpc_with_write(pool, dir, write_on=true, req)` を呼ぶ（ツール実装・監査ログ・`mutated` を共有）。ポートは `settings.mcp_server.port`（既定 `DEFAULT_PORT=3917`）、トークンは keychain `mcp_server.token`。

| コマンド | 引数 / フラグ | MCP ツール |
|---------|--------------|-----------|
| `add` | `--title <T>`（必須）`--type` `--year` `--doi` `--isbn` `--arxiv` `--url` `--citation-key` `--notes` `--abstract` `--author <name>…` `--field <k=v>…` | `create_entry` |
| `update <id\|citation_key>` | 上記フィールドフラグ（指定分のみ変更。`--citation-key ""` で unpin） | `update_entry` |
| `notes <id\|citation_key> <text…>` | — | `update_notes` |
| `tag <id\|citation_key> <tag_name>` | — | `add_tag` |
| `collect <id\|citation_key> <collection_id>` | — | `add_to_collection` |

破壊系（`delete_entry`）、DOI/arXiv メタデータ自動取得付き `add`、CLI の PATH 配置は次版以降。
