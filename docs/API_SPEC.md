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
| `delete_attachment` | `id: i64` | `Result<()>` |
| `open_attachment` | `id: i64` | `Result<()>` |
| `index_attachment` | `id: i64` | `Result<i64>` — 索引した非空ページ数 |
| `is_attachment_indexed` | `id: i64` | `Result<bool>` — fulltext 行が 1 件以上あるか |
| `unindex_attachment` | `id: i64` | `Result<()>` |
| `index_missing_attachments` | — | `Result<IndexMissingResult>` — 未索引 PDF を一括索引（v0.7.0） |

`index_attachment` はPDFからテキストを抽出してFTS5インデックスに登録する（冪等：既存行を削除して再登録）。`add_attachment` 後に自動で呼ばれるほか、詳細パネルの索引/再索引ボタンからも任意タイミングで呼べる。

`index_missing_attachments` は、まだ全文索引の無い PDF 添付（ゴミ箱を除く）を `db::fulltext::attachments_without_fulltext` で洗い出し、順に `pdf-extract` で抽出して索引する。過去に添付済み・自動索引を逃したエントリの後追い用（設定 → データの「未索引の PDF を一括索引」）。

**添付後の自動索引（CR-027）:** 手動添付（`add_attachment`）・arXiv 取得（`download_arxiv_pdf`）・Web クリッパー（MCP `spawn_pdf_job`）のいずれの経路も、添付成功後に共有ヘルパ `db::fulltext::extract_and_index` でバックグラウンド索引する（best-effort・スキャン PDF は OCR へ誘導）。以前はリーダーからの手動添付とクリッパー経路が索引されなかった。

`download_arxiv_pdf` は、arXiv からメタデータ取得してエントリを作成した直後に「PDF も一括で取得する」ためのコマンド（AddSheet の arXiv タブのチェックボックス。デフォルト ON）。`arxiv_id` を正規化して `https://arxiv.org/pdf/<id>` を `download::download_and_attach`（50MB 上限・`%PDF-` マジックバイト検証・タイムアウト付き）でダウンロードし添付、成功後はバックグラウンドで `pdf-extract` → 全文索引を試みる（索引失敗は無視）。ペイウォールやネットワーク障害で失敗しても呼び出し側はエントリ作成を成功扱いにする（フロントは警告ログのみで詳細パネルからの手動添付に誘導）。

```ts
type IndexMissingResult = {
  total: number;     // 処理対象（未索引 PDF）の総数
  indexed: number;   // テキストを抽出して索引できた数
  needs_ocr: number; // 0 ページ＝テキストレイヤー無し（OCR 候補）
  failed: number;    // 読み込み/抽出に失敗した数
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

### LCIR（機械可読中間形式）— 実験 / `lcir.enabled`

論文全文を型付きノード木 + PDF 座標 + provenance で保存する中間表現。設計は `docs/LCIR_design_overview.md`、スキーマは `DATA_MODEL.md`「LCIR 関連テーブル」。settings `lcir.enabled = "1"` のときだけ動く追加の side-build（既存 `fulltext` 検索は不変）。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `build_lcir_for_attachment` | `attachment_id: i64` | `LcirBuildResult` |
| `get_lcir_document` | `attachment_id: i64` | `LcirDocument \| null` |

```ts
type LcirBuildResult = {
  enabled: boolean;      // lcir.enabled が off なら false（何もしない）
  built: boolean;        // 新規に構築したか
  reused: boolean;       // 同一 content_key の既存を再利用したか（冪等）
  version_id: number | null;
  content_key: string | null;
  page_count: number;
  message: string;
};

// PDF 座標付きの木（正本は SQLite、これはその JSON 派生ビュー）
type LcirDocument = {
  schema: string;
  schema_version: string;
  version_id: number;
  content_key: string;
  source: { sha256: string; mime_type: string; extractor_name: string; extractor_version: string };
  coordinate_space: { space: string; origin: string; unit: string; y_axis: string };
  nodes: Array<{
    id: number;
    kind: string;           // document / page / text_block / ...
    ordinal: number;
    parent_id?: number;
    plain_text?: string;
    origin?: string;
    confidence?: number;
    source_fragments: Array<{ page: number; bbox: { x: number; y: number; width: number; height: number }; fragment_type?: string }>;
  }>;
};
```

- `build_lcir_for_attachment` は pdfium で抽出し `document_versions`/`document_nodes`/`source_fragments` を作る。`content_key`（= `sha256(source_sha256|extractor_name|extractor_version|config_hash)`）で冪等：同一 PDF+同一抽出器版なら再抽出せず reuse。新版採用時は同一添付の旧 completed を `superseded` にする。
- 座標は既存 `highlights` と同一系（PDF user space・左下原点・pt）なので、将来「検索ヒット → PDF 該当領域ハイライト」に直結できる。
- フラグ OFF なら両コマンドとも DB に一切書かず（`build` は `enabled:false`、`get` は `null`）、既存挙動は不変。

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

**完全バックアップ（CR-018）**: `run_backup_now` は `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.zip` を作る。アーカイブ内レイアウトは `db.sqlite`（DB 全体＝highlights/chat/settings/fulltext 込み）＋ `attachments/<entry_id>/<file_name>`（添付本体）。deflate 圧縮。14 世代保持。自動バックアップは Rust 側で起動時 + 24h 間隔のタイマーから呼ばれる。

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

`chat_send_message` の `channel` は `tauri::ipc::Channel<ChatStreamEvent>`。`tool_call_proposed` の `needs_approval=true` を受けたら UI は承認ダイアログを出し、`approve_tool_call` で応答する。承認制御はツール別ホワイトリスト（DATA_MODEL の `chat.tool_whitelist` 参照）に従う:

- read 系（`fulltext_search` / `get_entry` / `list_*`）: 常に自動
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
| `OPTIONS /clipper` | 不要 | CORS preflight。`Origin` が `chrome-extension://` で始まる場合のみ `Access-Control-Allow-*` を返す（204）。**認証チェックより前に処理**（preflight は Authorization ヘッダを持たないため） |
| `GET /clipper` | Bearer | ペアリング疎通確認。`{"ok":true,"app":"LumenCite","version":"..."}` |
| `POST /clipper` | Bearer | クリップ本体。`clipper.enabled` をリクエスト毎に評価（無効なら 403 `{"status":"error","code":"clipper_disabled"}`） |

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
  pdf?: "downloading";     // created かつ PDF URL があるとき。添付は応答後に非同期実行
};
```

**サーバー側フロー:** `find_duplicate_entry`（DOI/arXiv/ISBN）→ 重複なら `duplicate` 応答（作成も PDF 添付もしない）→ 識別子があれば `metadata::fetch_by_doi/arxiv/isbn` でメタデータ解決 → `create_entry` → PDF URL（明示 or arXiv 導出）があれば**応答後に** `download_and_attach` を spawn（50MB 上限・30 秒タイムアウト・先頭チャンクの `%PDF-` マジック検証。失敗してもエントリは残る）。作成・添付の成功時は `.bib` 同期キック＋ `entries-changed` を発火。

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

### OCR（v0.2.0 追加）

テキストレイヤーのないスキャン PDF を LLM Vision で OCR し、結果を `fulltext` にページ単位で保存する。詳細ビューの手動ボタンと LLM ツール（`ocr_pdf` / `attach_ocr_text`）で内部実装を共有する。

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `ocr_pdf` | `entry_id: i64, attachment_id?: i64, pages?: Vec<i64>` | `Result<()>` — `attachment_id` 省略時は先頭 PDF、指定時はその添付を OCR（複数 PDF 対応・CR-027）。`pages` 省略時は全ページ。OCR プロバイダは `LlmSettings.ocr_provider` → `provider` のフォールバック |

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

### 書込コマンド（ハイブリッド C）

書込は次のルーティングで実行する（`--force` は全書込コマンド共通のグローバルフラグ）:

1. `--force` → 直接 DB 書込（アプリ起動中なら一覧陳腐化の旨を stderr 警告）。
2. MCP サーバー到達可（keychain トークン有 + `ping` 成功）→ **HTTP 委譲**。サーバーが `mcp_server.write_enabled` ゲート適用＋`.bib` 同期＋GUI 更新。ゲート off なら「有効化 or `--force`」を明示。
3. 到達不可 → **直接 DB 書込** + `.bib` 同期（best-effort）。

どちらも `tools/call`（JSON-RPC）を組み、HTTP は POST、直接は `mcp_server::handle_rpc_with_write(pool, dir, write_on=true, req)` を呼ぶ（ツール実装・監査ログ・`mutated` を共有）。ポートは `settings.mcp_server.port`（既定 `DEFAULT_PORT=3917`）、トークンは keychain `mcp_server.token`。

| コマンド | 引数 / フラグ | MCP ツール |
|---------|--------------|-----------|
| `add` | `--title <T>`（必須）`--type` `--year` `--doi` `--isbn` `--arxiv` `--url` `--citation-key` `--notes` `--abstract` `--author <name>…` `--field <k=v>…` | `create_entry` |
| `update <id\|citation_key>` | 上記フィールドフラグ（指定分のみ変更。`--citation-key ""` で unpin） | `update_entry` |
| `notes <id\|citation_key> <text…>` | — | `update_notes` |
| `tag <id\|citation_key> <tag_name>` | — | `add_tag` |
| `collect <id\|citation_key> <collection_id>` | — | `add_to_collection` |

破壊系（`delete_entry`）、DOI/arXiv メタデータ自動取得付き `add`、CLI の PATH 配置は次版以降。
