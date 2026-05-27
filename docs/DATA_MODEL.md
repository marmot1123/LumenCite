# LumenCite データモデル

## エンティティ一覧

### `entries` — 文献本体

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `citation_key` | TEXT | BibTeX エントリキー（cite key）。NULL = 自動生成、値あり = ユーザーがピン留めした固定キー。`migrations/0008` で追加 |
| `title` | TEXT NOT NULL | |
| `year` | INTEGER | |
| `entry_type` | TEXT NOT NULL | `article` `book` `inproceedings` `thesis` `webpage` `misc` 等（BibTeX型に対応） |
| `doi` | TEXT | |
| `isbn` | TEXT | |
| `arxiv_id` | TEXT | `2301.00001` 形式 |
| `url` | TEXT | |
| `abstract` | TEXT | |
| `notes` | TEXT | |
| `summary` | TEXT | LLM 生成要約（v0.1.0 から）。NULL = 未生成 |
| `summary_model` | TEXT | 要約生成に使ったモデル識別子（例 `openai:gpt-4o-mini`） |
| `summary_generated_at` | TEXT | 要約生成日時（ISO8601 / `datetime('now')`） |
| `starred` | INTEGER | お気に入り（0/1）。DEFAULT 0 |
| `deleted_at` | TEXT | ゴミ箱（ソフト削除）。NULL = アクティブ。`datetime('now')` がセットされた行はゴミ箱内。 |
| `created_at` | TEXT | `datetime('now')` |
| `updated_at` | TEXT | `datetime('now')` |

型固有フィールド（`journal`, `volume`, `pages`, `publisher` 等）は `extra_fields` に格納する。

#### `citation_key`（BibTeX エントリキー） — migration 0008

LaTeX の `\cite{...}` で参照されるキー。LaTeX 連携が安定するよう永続化する。

- **意味づけ**: `NULL` = 自動生成（後述）、非 NULL = ユーザーがピン留めした固定キー。Zotero の Better BibTeX における「pinned citation key」に相当する。
- **一意性**: `CREATE UNIQUE INDEX ux_entries_citation_key ON entries(citation_key) WHERE citation_key IS NOT NULL` の部分インデックスで、非 NULL 値はグローバル一意。NULL（=自動）は複数行で許容。
- **サニタイズ**: 保存時に英数字と `_ : - . / +` のみを残し、それ以外を除去。トリム後に空になれば `NULL`（=自動）にフォールバック。
- **自動生成（NULL のとき）**: エクスポート時に `第一著者の姓 + 年`（著者なしはタイトル先頭語、年なしは `nd`）から生成し、**同一 `.bib` ファイル内**で重複したら接尾辞 `a` / `b` / `c` …（26 を超えたら `aa` `ab` …）を付与して一意化する。ピン留め済みキーは予約済みとして衝突を避ける。
- **インポート**: 元 `.bib` の cite key をサニタイズして `citation_key` に保持する。既存キー（および同一インポート内で先に確定したキー）と衝突する場合は接尾辞 `a` / `b` / `c` … で一意化する。
- **手動編集の衝突**: ユーザーが入力した固定キーが既存と重複する場合は UNIQUE 制約違反として保存を拒否する（自動の a/b/c は付けない）。UI は保存前に `is_citation_key_available` で事前チェックする。

---

### `authors` — 著者マスタ

同一著者を複数文献にまたがって管理する。ORCID で名寄せを補助する。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `name` | TEXT NOT NULL | 表示用フルネーム |
| `given_name` | TEXT | 名（任意） |
| `family_name` | TEXT | 姓（任意） |
| `orcid` | TEXT UNIQUE | ORCID識別子（任意） |
| `created_at` | TEXT | |

### `entry_authors` — 文献↔著者（多対多・順序付き）

| カラム | 型 | 備考 |
|--------|-----|------|
| `entry_id` | INTEGER FK → entries | ON DELETE CASCADE |
| `author_id` | INTEGER FK → authors | ON DELETE RESTRICT |
| `position` | INTEGER | 著者順（0始まり） |

---

### `entry_relations` — エントリ間の関連

arXivプレプリントと出版版など、別エントリとして管理しつつ関連を表現する。

| カラム | 型 | 備考 |
|--------|-----|------|
| `from_entry_id` | INTEGER FK → entries | ON DELETE CASCADE |
| `to_entry_id` | INTEGER FK → entries | ON DELETE CASCADE |
| `relation_type` | TEXT | 下記参照 |

**`relation_type` の値：**

| 値 | 意味 |
|----|------|
| `preprint_of` | from がプレプリント、to が出版版 |
| `version_of` | 一般的な別バージョン関係 |
| `supplement_of` | from が to の補足資料 |

---

### `collections` — コレクション（ネスト対応）

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `name` | TEXT NOT NULL | |
| `parent_id` | INTEGER FK → collections | NULL = ルート。ON DELETE CASCADE |
| `created_at` | TEXT | |

### `entry_collections` — 文献↔コレクション（多対多）

1つの文献を複数コレクションに所属させられる。

`PRIMARY KEY (entry_id, collection_id)`

---

### `tags` + `entry_tags`

- `tags(id, name UNIQUE)`
- `entry_tags(entry_id, tag_id)` — `PRIMARY KEY (entry_id, tag_id)`

---

### `highlights` — PDF ハイライト（v0.1.0 追加）

詳細ビューの PDF テキスト選択 → ハイライト保存に使う。`pdf.js` の座標系（PDF ポイント、左下原点）で保持する。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `entry_id` | INTEGER FK → entries | ON DELETE CASCADE |
| `page` | INTEGER NOT NULL | 1 始まりのページ番号 |
| `x` | REAL NOT NULL | バウンディング左下 X（PDF pt） |
| `y` | REAL NOT NULL | バウンディング左下 Y（PDF pt） |
| `width` | REAL NOT NULL | |
| `height` | REAL NOT NULL | |
| `color` | TEXT NOT NULL | `yellow` / `green` / `blue`（v0.1.0 は 3 色固定） |
| `text` | TEXT NOT NULL | 抽出済みテキスト（ハイライトタブの引用表示用） |
| `note` | TEXT | ハイライトに紐付くノート（任意） |
| `created_at` | TEXT NOT NULL | DEFAULT `CURRENT_TIMESTAMP` |

インデックス: `idx_highlights_entry_page ON highlights(entry_id, page)`

---

### `attachments` — 添付ファイル

ファイル本体はアプリデータディレクトリに保存し、DBにはパスのみ持つ。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `entry_id` | INTEGER FK → entries | ON DELETE CASCADE |
| `file_path` | TEXT NOT NULL | アプリデータディレクトリからの相対パス |
| `file_name` | TEXT NOT NULL | 表示用ファイル名 |
| `mime_type` | TEXT NOT NULL | デフォルト `application/pdf` |
| `created_at` | TEXT | |

---

### `entries_fts` — メタデータ全文検索（FTS5仮想テーブル）

エントリのメタデータ（タイトル・著者・タグ・abstract・識別子）を統合検索するためのインデックス。`rowid = entries.id` で 1 エントリ 1 行。

| カラム | 型 | 備考 |
|--------|-----|------|
| `title` | TEXT | エントリのタイトル |
| `authors_text` | TEXT | 著者名をスペース区切りで結合 |
| `tags_text` | TEXT | タグ名をスペース区切りで結合 |
| `abstract_text` | TEXT | abstract（NULL は空文字） |
| `identifiers` | TEXT | DOI・ISBN・arXiv ID・year をスペース区切り |

tokenizer: `trigram`（CJK・ラテン両対応の 3-gram、SQLite 3.34+ 標準搭載）

同期: `entries`・`entry_authors`・`entry_tags`・`extra_fields` の変更は Rust 側の create/update/delete 内で `entries_fts` を再構築（`DELETE FROM entries_fts WHERE rowid = ?` → `INSERT INTO entries_fts (rowid, ...) VALUES (?, ...)`）。マイグレーション時は既存データを SELECT して一括 INSERT する。

### `fulltext` — PDF全文検索（FTS5仮想テーブル）

SQLite FTS5 を使用。ページ単位でインデックスする。

| カラム | 型 | 備考 |
|--------|-----|------|
| `content` | TEXT | 検索対象テキスト |
| `attachment_id` | INTEGER UNINDEXED | |
| `page` | INTEGER UNINDEXED | ページ番号 |

tokenizer: `trigram`（PDF 添付フェーズで `unicode61` から変更予定）

v0.2.0 の **LLM Vision OCR**（`ocr_pdf` / `attach_ocr_text`）はスキャン PDF の認識結果を**この同じテーブル**にページ単位で書き込み、以後 `fulltext_search` でヒットするようにする（スキーマ変更なし）。

---

### Chat 関連テーブル（v0.2.0 追加 / migration 0007）

agentic LLM Chat のセッションとメッセージ履歴を永続化する。`migrations/0007_chat.sql` で追加。

#### `chat_sessions` — チャットセッション

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `title` | TEXT NOT NULL | LLM 自動生成（ユーザー編集可） |
| `provider` | TEXT NOT NULL | `openai` / `anthropic` 等 |
| `model` | TEXT NOT NULL | モデル識別子 |
| `system_prompt` | TEXT | セッション固有のシステムプロンプト（任意） |
| `scope_mode` | TEXT NOT NULL | `all`（DB 全体検索）/ `entries`（特定文献に絞る）。DEFAULT `'all'` |
| `created_at` | TEXT NOT NULL | |
| `updated_at` | TEXT NOT NULL | |
| `archived_at` | TEXT | ソフト削除（NULL = アクティブ） |

#### `chat_messages` — メッセージ履歴

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `session_id` | INTEGER FK → chat_sessions | ON DELETE CASCADE |
| `role` | TEXT NOT NULL | `user` / `assistant` / `tool` |
| `content` | TEXT NOT NULL | 本文（tool メッセージは結果テキスト） |
| `tool_calls` | TEXT | JSON: assistant のツール呼び出し列（任意） |
| `tool_call_id` | TEXT | `role='tool'` の結果が紐づく呼び出し ID（任意） |
| `created_at` | TEXT NOT NULL | |
| `position` | INTEGER NOT NULL | セッション内の並び順 |

#### `chat_session_entries` — セッション↔文献（scope の対象集合）

`scope_mode='all'` のとき空（DB 全体検索）。`'entries'` のとき、ここに含まれる `entry_id` 集合だけが FTS5 検索の対象。

| カラム | 型 | 備考 |
|--------|-----|------|
| `session_id` | INTEGER FK → chat_sessions | ON DELETE CASCADE |
| `entry_id` | INTEGER FK → entries | ON DELETE CASCADE |

`PRIMARY KEY (session_id, entry_id)`

インデックス:
- `idx_chat_messages_session ON chat_messages(session_id, position)`
- `idx_chat_sessions_updated ON chat_sessions(updated_at DESC)`

---

### `extra_fields` — BibTeX型固有フィールド

`PRIMARY KEY (entry_id, field_name)`

`journal`, `volume`, `issue`, `pages`, `publisher`, `booktitle`, `series`, `edition` 等を格納する。

---

### `settings` — アプリ設定（非機密）

`PRIMARY KEY (key)`

LLM APIキー等の機密情報は **OS キーチェーン**（macOS Keychain / Windows Credential Manager / Linux secret-service。`keyring` クレート経由）に保存し、このテーブルには含めない。

#### キー命名規約（v0.1.0）

| キー | 値 | 用途 |
|------|------|------|
| `ui.language` | `ja` \| `en` | i18n 設定（localStorage の `lc-language` と同期） |
| `ui.theme` | `light` \| `dark` \| `auto` | テーマ（localStorage の `lc-theme` と同期） |
| `ui.accent` | `amber` \| `indigo` \| `teal` \| `rose` | アクセントカラー |
| `ui.density` | `compact` \| `default` \| `comfortable` | 行密度 |
| `bibtex.sync_path` | 絶対パス | `.bib` 同期先（既存） |
| `llm.provider` | `openai` \| `anthropic` | 既定の LLM プロバイダ |
| `llm.model` | モデル識別子（例 `gpt-4o-mini`） | 既定モデル |
| `llm.summary_source` | `abstract` \| `fulltext` | 要約生成時の入力 |
| `backup.last_run` | ISO8601 | 直近の自動バックアップ完了時刻 |
| `backup.retention` | 整数文字列 | バックアップ保持世代数（既定 14） |
| `updater.channel` | `stable` \| `beta` | アップデートチャネル |
| `pdf.last_page.<entry_id>` | 整数文字列 | エントリごとの最終閲覧ページ |

#### キー追加（v0.2.0）

| キー | 値 | 用途 |
|------|------|------|
| `llm.ocr_provider` | `openai` \| `anthropic`（未設定可） | OCR 用 LLM プロバイダ。未設定なら `llm.provider` にフォールバック |
| `llm.ocr_model` | モデル識別子（未設定可） | OCR 用モデル。未設定なら `llm.model` にフォールバック |
| `chat.tool_whitelist` | JSON | ツール別自動承認のデフォルト上書き。`delete_*` / MCP write 系は上書き不可 |
| `mcp.servers` | JSON | 外部 MCP サーバー設定。Claude Desktop の `mcpServers` 互換形式 |

OS キーチェーン側のサービス名: `com.lumencite.LumenCite`、アカウント名は `llm.api_key.openai` / `llm.api_key.anthropic` のように `<scope>.<key>` 形式。MCP サーバーに渡す秘匿情報（API キー等）が必要な場合も、平文を `settings` に置かず環境変数 or キーチェーン経由とする。

---

## 設計上の注意

- `PRAGMA foreign_keys = ON` はRustの接続初期化時に毎回設定する（SQLiteはデフォルト無効）
- WAL モード（`journal_mode = WAL`）はマイグレーションで一度設定すれば永続化される
- `updated_at` の自動更新はRust側のupdateコマンドで `datetime('now')` をセットする（SQLiteにはUPDATEトリガーを使う方法もあるが、シンプルさを優先）
