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

### LLM

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `summarize_entry` | `entry_id: i64` | `Result<String>` |
| `get_llm_settings` | — | `LlmSettings` |
| `save_llm_settings` | `settings: LlmSettings` | `Result<()>` |

APIキーはOSキーチェーンに保存するため、`LlmSettings` には含まない。

### アプリ設定（settings）

| コマンド | 引数 | 戻り値 |
|---------|------|--------|
| `get_setting` | `key: String` | `Option<String>` |
| `set_setting` | `key: String, value: String` | `Result<()>` |
