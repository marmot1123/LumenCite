# LumenCite データモデル

## エンティティ一覧

### `entries` — 文献本体

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
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

OS キーチェーン側のサービス名: `com.lumencite.LumenCite`、アカウント名は `llm.api_key.openai` / `llm.api_key.anthropic` のように `<scope>.<key>` 形式。

---

## 設計上の注意

- `PRAGMA foreign_keys = ON` はRustの接続初期化時に毎回設定する（SQLiteはデフォルト無効）
- WAL モード（`journal_mode = WAL`）はマイグレーションで一度設定すれば永続化される
- `updated_at` の自動更新はRust側のupdateコマンドで `datetime('now')` をセットする（SQLiteにはUPDATEトリガーを使う方法もあるが、シンプルさを優先）
