# LumenCite データモデル

## エンティティ一覧

### `entries` — 文献本体

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `citation_key` | TEXT | BibTeX エントリキー（cite key）。NULL = 自動生成、値あり = ユーザーがピン留めした固定キー。`migrations/0008` で追加 |
| `title` | TEXT NOT NULL | |
| `year` | INTEGER | |
| `entry_type` | TEXT NOT NULL | 種別キー（制約なしの自由 TEXT）。既存 6 種 `article` `book` `inproceedings` `thesis` `webpage` `misc` は BibTeX 由来。v0.4.0 で Zotero アイテムタイプを追加: `preprint` `bookSection` `report` `magazineArticle` `newspaperArticle` `encyclopediaArticle` `dictionaryEntry` `manuscript` `presentation` `patent` `standard` `dataset` `computerProgram`。一覧は `src/types.ts` の `EntryType` が正 |
| `doi` | TEXT | 入力そのまま（表示用）。重複判定は `doi_canonical` を使う |
| `isbn` | TEXT | 入力そのまま（表示用）。重複判定は `isbn_canonical` を使う |
| `arxiv_id` | TEXT | `2301.00001` 形式。入力そのまま（表示用）。重複判定は `arxiv_canonical` を使う |
| `doi_canonical` | TEXT | DOI の正準値（`doi.org`/`doi:` prefix 除去＋小文字化）。`migrations/0013`・CR-019 |
| `arxiv_canonical` | TEXT | arXiv の正準値（prefix/版番号除去・旧形式カテゴリ保持・小文字化）。`migrations/0013`・CR-019 |
| `isbn_canonical` | TEXT | ISBN の正準値（英数字のみ・大文字化）。`migrations/0013`・CR-019 |
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

#### 識別子の canonical 化と重複防止 — migration 0013（CR-019）

DOI / arXiv / ISBN は表記揺れ（大小・`doi.org` prefix・arXiv の版番号 `vN` や `arXiv:` prefix・ISBN のハイフン）が多く、同一文献が二重登録されやすい。これを防ぐため正準値を専用列 `*_canonical` に持つ。

- **正規化の単一ソース**: `db::entries::canonical_{doi,arxiv,isbn}()`（Rust）。書込（`create_entry`/`update_entry`）・重複判定（`find_duplicate_entry`）・起動時 backfill のすべてがこれを経由する。SQL 側で `LOWER`/`REPLACE` を書いて非対称に揃える旧方式は廃止（stored 側が arXiv の版番号を剥がさず dedup をすり抜けていた）。
- **全経路 dedup**: `create_entry` は UI 追加 / import / LLM / clipper のいずれの経路でも、現役エントリに同一 canonical があれば新規作成せず既存を返す（冪等）。ゴミ箱内（`deleted_at IS NOT NULL`）は対象外。
- **DB 制約（best-effort）**: `CREATE UNIQUE INDEX ... ON entries(<col>) WHERE <col> IS NOT NULL AND deleted_at IS NULL` の部分インデックスで現役エントリの一意性を DB でも保証する。ただし**既存 DB に重複があると `CREATE UNIQUE INDEX` が失敗して起動不能（brick）になる**ため、migration では張らず、起動時に `try_create_identifier_unique_indexes` が**重複が無い識別子だけ**張る（重複が残るものは非 UNIQUE 索引のままにして警告ログ）。
- **backfill**: 既存行の canonical 埋めは arXiv の版番号除去などを SQL で表現できないため、migration 0013 は列と非 UNIQUE 部分索引だけを作り、実際の埋めは起動時 `backfill_canonical_identifiers`（canonical が NULL の行だけ対象・冪等）で行う。
- **restore との一貫性**: 部分索引・dedup とも「現役（`deleted_at IS NULL`）のみ」を対象にするため、ゴミ箱の文献と現役が同一識別子を持つことは許容する。`restore_entry`/`bulk_restore` は untrash 前に現役の衝突相手を検出したら明示エラーで復活を拒否し、不変条件を守る。

---

### `authors` — 著者マスタ

同一著者を複数文献にまたがって管理する。ORCID で名寄せを補助する。v0.3.0 で多言語名（漢字名等）・読み仮名・団体著者・CSL 互換フィールドに対応。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `name` | TEXT NOT NULL | 表示用フルネーム |
| `given_name` | TEXT | 名（任意） |
| `middle_name` | TEXT | ミドルネーム / "John F. Kennedy" の `F.`（v0.3.0 / migration 0009） |
| `family_name` | TEXT | 姓（任意） |
| `suffix` | TEXT | `Jr.` / `Sr.` / `III` 等。CSL の suffix に対応（v0.3.0 / migration 0009） |
| `name_particle` | TEXT | `von` / `van der` / `de la` 等。CSL の non-dropping-particle に対応。`family_name` に混ぜない（v0.3.0 / migration 0009） |
| `name_original` | TEXT | オリジナル言語表記のフルネーム（例 `関 元樹` / `毛沢东`）。区切りが曖昧な言語向け（v0.3.0 / migration 0009） |
| `given_name_original` | TEXT | オリジナル言語の名（例 `元樹`）。分割できる場合のみ（v0.3.0 / migration 0009） |
| `family_name_original` | TEXT | オリジナル言語の姓（例 `関`）。分割できる場合のみ（v0.3.0 / migration 0009） |
| `original_script` | TEXT | ISO 15924 文字種コード（例 `Hani` 漢字 / `Hang` ハングル / `Cyrl` キリル）。正規化・ソート判定に利用（v0.3.0 / migration 0009） |
| `reading_family` | TEXT | 姓の読み仮名（例 `せき`）。五十音ソート・かな検索用（v0.3.0 / migration 0009） |
| `reading_given` | TEXT | 名の読み仮名（例 `もとき`）（v0.3.0 / migration 0009） |
| `is_organization` | INTEGER | 団体著者フラグ（0/1）。`1` のとき given/family を無視し `name` を literal として扱う（CSL の literal 相当）。BibTeX の `{IEEE}` 等から自動検出。DEFAULT 0（v0.3.0 / migration 0009） |
| `email` | TEXT | corresponding author 追跡用（任意・v0.3.0 / migration 0009） |
| `homepage_url` | TEXT | 著者プロフィールページ URL（任意・v0.3.0 / migration 0009） |
| `notes` | TEXT | 「同名別人」「2024 改姓」等の自由メモ（v0.3.0 / migration 0009） |
| `orcid` | TEXT UNIQUE | ORCID識別子（任意）。互換維持のため専用カラムを残しつつ、新規取得時は `author_identifiers` にも併記する |
| `created_at` | TEXT | |
| `updated_at` | TEXT | `datetime('now')`。編集機能で更新（v0.3.0 / migration 0009） |

#### 名寄せロジック（v0.3.0 で改善）

`get_or_create_author`（`db/authors.rs`）は v0.3.0 で 3 段照合に拡張:

1. ORCID があれば ORCID で照合（`authors.orcid` 列を優先し、無ければ `author_identifiers (scheme='orcid')` も見る）→ ヒットすれば既存を返す
2. ORCID なし or 未ヒットなら 正規化済み `name`（trim + Unicode NFKC + lowercase、`unicode-normalization` クレート使用）で照合。SQLite は NFKC 関数を持たないので、authors を全件 SELECT → Rust 側で比較する素朴実装（個人ライブラリ規模で十分。将来は `authors.normalized_name` 列で O(1) 化する余地）
3. それでもなければ INSERT。`orcid` が入力されていれば `authors.orcid` 列と `author_identifiers(scheme='orcid')` の両方に書く（互換維持運用）

#### 編集・統合（v0.3.0 新規）

- `update_author(id, AuthorInput)` — 全列 UPDATE + `author_identifiers` を **DELETE → INSERT で総差し替え**。`input.orcid` がセットされているのに `input.identifiers` に scheme='orcid' が含まれていなければ暗黙で `author_identifiers` にも書く。完了後、当該著者が紐づく全 entry の `entries_fts` を再構築する
- `merge_authors(from_id, into_id)` — `entry_authors` を `into` に集約（同 entry に両方ぶら下がっている衝突行は `from` を削除して `into` を残す）。`author_identifiers` は `into` を優先 — まず `from` 側の同一 scheme 行を DELETE してから `UPDATE` で `from→into` に付け替える（`INSERT…SELECT ON CONFLICT` 方式は `(scheme, value)` UNIQUE INDEX に短絡されるため不採用）。最後に関連 entry の FTS を再同期
- `add_author_identifier` / `delete_author_identifier` — scheme='orcid' のときは `authors.orcid` 列も同期 (set / clear)

### `entry_authors` — 文献↔著者（多対多・順序付き）

| カラム | 型 | 備考 |
|--------|-----|------|
| `entry_id` | INTEGER FK → entries | ON DELETE CASCADE |
| `author_id` | INTEGER FK → authors | ON DELETE RESTRICT |
| `position` | INTEGER | 著者順（0始まり） |

### `author_identifiers` — 著者の外部識別子（v0.3.0 / migration 0009）

ORCID 以外の識別子（Scopus / DBLP / Semantic Scholar / Wikidata / ISNI / VIAF / ResearcherID / Google Scholar 等）を正規化して保持する。追加のたびに `authors` テーブルへ migration するのを避けるため別テーブル化。

| カラム | 型 | 備考 |
|--------|-----|------|
| `author_id` | INTEGER FK → authors | ON DELETE CASCADE |
| `scheme` | TEXT NOT NULL | `orcid` / `scopus` / `dblp` / `semantic_scholar` / `wikidata` / `isni` / `viaf` / `researcher_id` / `google_scholar` 等 |
| `value` | TEXT NOT NULL | 識別子の値（例 ORCID `0000-0002-1825-0097`、Wikidata `Q937`） |
| `url` | TEXT | 任意。`scheme` から導出できる場合は省略可 |

- `PRIMARY KEY (author_id, scheme)` — 1 著者 1 scheme につき 1 行
- `UNIQUE INDEX idx_author_identifiers_scheme_value ON author_identifiers(scheme, value)` — 同じ識別子が複数著者に紐づかないようにする

ORCID は `authors.orcid` 専用カラムと併記する運用（v0.3.0 時点）。v0.4.0 以降で `authors.orcid` を廃止し、本テーブルに一本化する余地は残す。

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
| `attachment_id` | INTEGER FK → attachments | ON DELETE CASCADE。**どの添付 PDF に属すか**（CR-015 / migration 0011）。旧データは各エントリの primary（最小 id）添付へ移行。添付削除で当該ハイライトも CASCADE |
| `page` | INTEGER NOT NULL | 1 始まりのページ番号 |
| `x` | REAL NOT NULL | バウンディング左下 X（PDF pt） |
| `y` | REAL NOT NULL | バウンディング左下 Y（PDF pt） |
| `width` | REAL NOT NULL | |
| `height` | REAL NOT NULL | |
| `color` | TEXT NOT NULL | `yellow` / `green` / `blue`（v0.1.0 は 3 色固定） |
| `text` | TEXT NOT NULL | 抽出済みテキスト（ハイライトタブの引用表示用） |
| `note` | TEXT | ハイライトに紐付くノート（任意） |
| `created_at` | TEXT NOT NULL | DEFAULT `CURRENT_TIMESTAMP` |

インデックス: `idx_highlights_entry_page ON highlights(entry_id, page)`、`idx_highlights_attachment_page ON highlights(attachment_id, page)`（CR-015）

---

### `attachments` — 添付ファイル

ファイル本体はアプリデータディレクトリに保存し、DBにはパスのみ持つ。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `entry_id` | INTEGER FK → entries | ON DELETE CASCADE |
| `file_path` | TEXT NOT NULL | アプリデータディレクトリからの相対パス。**UNIQUE**（`idx_attachments_file_path` / migration 0012・CR-008）。保存側は O_EXCL で名前を原子的に予約するので 1 ファイルを 2 行が共有しない |
| `file_name` | TEXT NOT NULL | 表示用ファイル名 |
| `mime_type` | TEXT NOT NULL | デフォルト `application/pdf`。arXiv TeX ソース（`download_arxiv_source`・LCIR Phase 4）は `application/gzip` — PDF 前提の経路（ビューア・全文索引・PDF 向け一括バッチ・`has_attachment` の一覧バッジ/「添付 PDF」フィルタ/CLI `--has-attachment`）はこの mime で除外される |
| `created_at` | TEXT | |

添付削除は `delete_attachment_with_fulltext` が attachments 行と全文索引（`fulltext`）を単一トランザクションで消す（orphan index を残さない・CR-008）。ファイル本体の削除は best-effort（失敗はログのみ）。

---

### `entries_fts` — メタデータ全文検索（FTS5仮想テーブル）

エントリのメタデータ（タイトル・著者・タグ・abstract・識別子）を統合検索するためのインデックス。`rowid = entries.id` で 1 エントリ 1 行。

| カラム | 型 | 備考 |
|--------|-----|------|
| `title` | TEXT | エントリのタイトル |
| `authors_text` | TEXT | 著者名をスペース区切りで結合。v0.3.0 で `name_original`（漢字名等）と `reading_family || ' ' || reading_given`（読み仮名）も同じセルへ追記し、「せき」「関」「Seki」のどれでもヒットさせる |
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
| `mcp.servers` | JSON | 外部 MCP サーバー設定（クライアント側）。Claude Desktop の `mcpServers` 互換形式。**`env` の値は平文で置かない**（CR-012）。値は `secretbox`（AES-256-GCM）で暗号化し `enc:v1:<base64>` 形式で保存する。復号鍵はキーチェーン（`secretbox.master_key`）。一覧 API はキー名のみ返し値は伏せる。旧・平文値は起動時に一度だけ暗号化して書き戻す |

#### キー追加（v0.3.0）

| キー | 値 | 用途 |
|------|------|------|
| `fts.authors_v030_rebuilt` | `"1"`（または未設定） | v0.3.0 で `entries_fts.authors_text` の合成式が変わったため、起動時に 1 回だけ全 entry の FTS を再構築する。完了したらこのキーが立つ。失敗時は立てずに次回起動でリトライ |
| `mcp_server.enabled` | `"1"` \| `"0"`（または未設定） | LumenCite 自身を MCP サーバーとして公開するかのフラグ。`"1"` で起動時に自動起動 |
| `mcp_server.port` | 文字列の数値（未設定なら既定 `3917`） | MCP サーバーのバインドポート。`port=0` で起動した場合は OS 割り当ての実ポートをここに保存する |
| `mcp_server.write_enabled` | `"1"` \| `"0"`（または未設定） | **Phase 2**: MCP サーバー公開で write 系ツールを許可するフラグ（既定 false）。承認 UI が無いためサーバー側でこのゲートを enforce する。サーバーはリクエスト毎に評価するので変更は再起動不要 |

OS キーチェーン側のサービス名: `com.lumencite.app`、アカウント名は `llm.api_key.openai` / `llm.api_key.anthropic` のように `<scope>.<key>` 形式。MCP **サーバー公開**の Bearer 認可トークンも同サービスのアカウント名 `mcp_server.token` に保管する（`settings` には置かない）。外部 MCP **クライアント**に渡す `env` 秘匿情報（API キー等）は、`secretbox.master_key`（キーチェーンの 32byte マスター鍵）で AES-256-GCM 暗号化して `settings` に保存する（平文は置かない・CR-012）。マスター鍵はプロセス内でキャッシュするため keychain へ触るのは起動〜MCP 起動時の実質 1 回。**資格情報のローテーション**は、外部 MCP を一旦削除して新しい `env` で再登録すれば、古い暗号値は上書きされる。

### `mcp_audit_log` — MCP 経由 write の監査ログ（Phase 2 / migration 0010）

外部 MCP クライアント（Claude Desktop/Code 等）からの書き込みは承認 UI を介さないため、何が・いつ・成否を後から追えるよう append-only で記録する。read 系は記録しない。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | |
| `tool_name` | TEXT NOT NULL | 実行された write ツール名（例 `create_entry`） |
| `arguments` | TEXT NOT NULL | ツール引数（JSON 文字列） |
| `result` | TEXT | 成功サマリ or エラーメッセージ |
| `is_error` | INTEGER NOT NULL | 0/1。実行が論理的に失敗したか |
| `created_at` | TEXT NOT NULL | `datetime('now')` 既定 |

`get_mcp_audit_log` コマンドで新しい順に取得する。閲覧 UI は未実装（Phase 4 候補）。

---

### LCIR 関連テーブル（機械可読中間形式 / migration 0014）

論文全文を「型付きノード木 + PDF 座標 + provenance + 信頼度」で保存する中間表現 **LCIR**（LumenCite Document Intermediate Representation）の基盤。設計全体は `docs/LCIR_design_overview.md`。**実験段階**で、settings `lcir.enabled` が `"1"` のときだけ構築する追加の side-build（既存 `fulltext` は不変）。抽出器は 2 系統: PDF 添付は pdfium（`lumencite-pdfium`）、arXiv TeX ソース添付（Phase 4・mime `application/gzip`）は TeX パーサ（`lumencite-tex`）。同一エントリに PDF 版と TeX 版の `document_version` が**別添付として併存**する（ADR #8）。第一段は下記 3 表のみ（math/assets/relations/symbols は後続フェーズの別 migration）。

#### `document_versions` — 添付ごとの抽出結果 1 回分（provenance の正本）

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `attachment_id` | INTEGER FK → attachments | ON DELETE CASCADE |
| `content_key` | TEXT NOT NULL | `sha256(source_sha256\|extractor_name\|extractor_version\|config_hash)`。**再現可能な内容由来 ID**（同一ソースファイル+同一抽出器 → 同一値）。row id は採番で再現不能なため。best-effort UNIQUE は `(attachment_id, content_key)`（起動時 `try_create_content_key_unique_index`・重複 DB で brick させないため migration では張らない） |
| `schema_version` | TEXT NOT NULL | `document_ir::SCHEMA_VERSION`（例 `0.1.0`） |
| `source_sha256` | TEXT NOT NULL | 原ファイル本体の SHA-256（`attachments` に列は無く抽出時に計算） |
| `source_mime_type` | TEXT NOT NULL | |
| `extractor_name` / `extractor_version` | TEXT NOT NULL | `lumencite-pdfium`（PDF）/ `lumencite-tex`（arXiv TeX ソース・Phase 4）。version は**抽出ロジックの semver**（抽出器ごとに独立採番・supersede 判定基準・クレート版とは別）。supersede は抽出器をまたがない（別添付に紐づくため） |
| `config_hash` | TEXT NOT NULL DEFAULT '' | 抽出設定のハッシュ |
| `parent_version_id` | INTEGER FK → document_versions | supersede チェーン |
| `extraction_status` | TEXT NOT NULL | `pending`/`processing`/`completed`/`completed_with_warnings`/`failed`/`superseded` |
| `warnings_json` / `metadata_json` | TEXT | 警告ログ / 座標系記述子・ページ数等 |
| `created_at` | TEXT | `datetime('now')` |

#### `document_nodes` — 型付きノード木

ノード型（`node_kind` は自由 TEXT なので型追加に migration 不要）:
- **Phase 1**: `document` / `page` / `text_block` / `line` / `unknown_block`。
- **Phase 2（論理構造）**: `page > block(段落・見出し 等) > line` の木を作る。block 型 = `section` / `subsection` / `heading` / `paragraph` / `abstract` / `figure_caption` / `table_caption` / `bibliography` / `bibliography_entry` / `unknown_block`（enum には `front_matter` / `list` / `list_item` / `footnote` / `citation` / `code_block` も用意済で認識器は後続で拡充）。block は推定なので `origin='layout_model'` + `confidence`、`line` は原文由来なので `origin='pdf_text_layer'`。見出しは `payload_json` に `{heading_level, section_number}`。
- **Phase 3（数式表層）**: 独立した数式を `display_math` block として認識し、`math_expressions` に表層表現を持たせる（enum には `inline_math` / `equation_group` も用意済で認識は後続）。
- **Phase 4（TeX 取込）**: arXiv TeX ソースから `lumencite-tex` 抽出器が別 `document_version` を作る。木は `document > block` の**フラット構造で page/line ノード無し・`source_fragments` 無し**（TeX に PDF 座標は無い）。block 型は `front_matter`（\title）/ `abstract` / `section`・`subsection`・`heading`（節番号はカウンタ再現・`payload_json` に `{heading_level, section_number}`）/ `paragraph` / `display_math`（**生 LaTeX**）/ `figure_caption`・`table_caption` / `list` / `code_block` / `bibliography`・`bibliography_entry`（`payload_json` に `{cite_key}`）。原文由来なので `origin='tex_source'`。
- **Phase 5（定理・定義・証明）**: block 型に `definition` / `theorem` / `lemma` / `proposition` / `corollary` / `remark` / `example` / `proof` を追加（**新規テーブルなし**・`document_nodes` + `payload_json` に載る）。**TeX**（`lumencite-tex` 0.2.0・`origin='tex_source'`・confidence 0.95）は環境名から種別を決め（preamble の `\newtheorem{env}{Display}` を回収して独自名・略記を表示名から対応づけ、標準英名と `proof` は既定マップ）、`\begin{theorem}[note]` の付記名を `payload_json.note`・`\label` を `payload_json.labels` に載せる。**PDF**（`lumencite-pdfium` 0.4.0・`origin='layout_model'`・confidence 0.6–0.7）は行頭キーワード + 番号 + 終端記号で認識し、`payload_json` に `{theorem_number, note}`（参照文中の "Theorem 2 shows …" は棄却＝欠損を許容）。定理間参照グラフ（`proves`）は Phase 6a（`node_relations`・下記）で張る。
- **Phase 6a（参照グラフ）**: **新規ノード型なし**。ノード間の参照を `node_relations`（migration 0017・下記）に有向辺として張る。`extractor_version` を pdfium 0.4.0→**0.5.0** / TeX 0.2.0→**0.3.0** に上げる（派生の関係辺が出力に増えるため）。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `document_version_id` | INTEGER FK → document_versions | ON DELETE CASCADE |
| `parent_id` | INTEGER FK → document_nodes | ON DELETE CASCADE。ルートは NULL |
| `node_kind` | TEXT NOT NULL | `document_ir::NodeKind` の snake_case |
| `ordinal` | INTEGER NOT NULL | 同一親内の読み順（page は 0 始まり = page_number − 1） |
| `plain_text` | TEXT | page ノードはページ全文（= FTS 再生成元） |
| `language` / `confidence` / `origin` | TEXT / REAL / TEXT | 言語 / 構造認識信頼度 / 由来（`pdf_text_layer` 等） |
| `payload_json` | TEXT | 型固有（page は `page_width_pt`/`page_height_pt`/`rotation_deg`） |
| `created_at` | TEXT | `datetime('now')` |

#### `source_fragments` — ノード ↔ PDF 領域

座標は既存 `highlights` と同一系（PDF user space・左下原点・pt）。1 ノードが複数ページ/領域にまたがれば複数行。各 page ノードには常にページ全面（MediaBox）の fragment を 1 つ付与する。**TeX 由来（`lumencite-tex`）の version は座標を持たないため fragment 行を一切作らない**（read 面の `bbox` は `null` になる）。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `node_id` | INTEGER FK → document_nodes | ON DELETE CASCADE |
| `page_number` | INTEGER NOT NULL | 1 始まり（`fulltext.page` / `highlights.page` と同じ） |
| `x` / `y` / `width` / `height` | REAL NOT NULL | 左下角 + 幅高さ（PDF pt） |
| `rotation` | REAL NOT NULL DEFAULT 0 | ページ `/Rotate`（0/90/180/270） |
| `reading_order` | INTEGER | 読み順（任意） |
| `fragment_type` | TEXT | `page` / `text_block` / `line` |

設定キー `lcir.enabled`（`"1"` で有効・既定 off）は `db/settings.rs::LCIR_ENABLED_KEY`。OFF なら上記 3 表は空のまま、既存挙動は byte-for-byte 不変。

#### `document_nodes_fts` — ノード単位 FTS（Phase 2 / migration 0015）

段落・見出し・caption 等の**ブロック粒度**の全文検索用 FTS5 仮想表（trigram）。ページ粒度の既存 `fulltext` と併存する派生索引で、正本は `document_nodes`。LCIR build 時に `ingestion::regenerate_node_fts_from_lcir` が張り、`document`/`page`/`line` を除く本文つきブロックだけを載せる。**pdfium 版のみが対象**（TeX 版・Phase 4 は索引しない — 同一エントリの PDF 版と本文が重複ヒットし bbox も無いため。検索 = PDF 版 / 読み出し = TeX 優先の分担）。

| カラム | 型 | 備考 |
|--------|-----|------|
| `content` | TEXT | ブロックの `plain_text`（索引対象） |
| `node_id` | UNINDEXED | `document_nodes.id`。ヒットから領域取得（`primary_fragment_for_node`）に使う |
| `attachment_id` | UNINDEXED | 削除・再索引のスコープ |
| `page` | UNINDEXED | 1 始まり |
| `node_kind` | UNINDEXED | `paragraph`/`section`/… |

FTS5 仮想表なので attachments への FK は張れず、手動クリーンアップする（`fulltext` と同型）: エントリ hard delete 時は `db/entries.rs` の削除経路が、**添付単体の削除時は `delete_attachment_with_fulltext` が同一トランザクションで** `attachment_id` 指定で消す（Phase 4 で後者の orphan 穴を修正）。

#### `math_expressions` — 数式の複数表現（Phase 3 / migration 0016）

`inline_math`/`display_math` ノードに 1:1 で付く。数式は単一形式に統一せず用途別の表現を列で併存させる。**PDF 由来は表層のみ**（`normalized_text` = 正規化した Unicode 線形文字列）を埋め `semantic_status='surface_only'`。**TeX 由来（Phase 4）は `latex` に原文スニペットをそのまま埋め `semantic_status='source_provided'`・`origin='tex_source'`**（`\tag{X}` があれば `equation_label="(X)"`・`\label` 名は親ノードの `payload_json.labels`）。Content MathML/OpenMath/AST は Phase 7（意味）で埋める。ノード削除でカスケード消去。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `node_id` | INTEGER FK → document_nodes | ON DELETE CASCADE |
| `display_mode` | TEXT NOT NULL | `inline` / `display` |
| `equation_label` | TEXT | 数式番号 `(2.1)` 等（あれば） |
| `latex` / `presentation_mathml` / `content_mathml` / `openmath_json` / `ast_json` | TEXT | `latex` は TeX 由来で原文スニペット（Phase 4）。PDF 由来は NULL。MathML/OpenMath/AST は後続フェーズ |
| `normalized_text` | TEXT | 検索用の正規化線形文字列（PDF 表層） |
| `semantic_status` | TEXT NOT NULL | `not_attempted`/`surface_only`/`inferred`/`verified`/`source_provided` |
| `confidence` | REAL | **表層検出**の確からしさ（意味の確からしさではない） |
| `origin` | TEXT | `pdf_text_layer`/`math_recognition`/`tex_source`/… |
| `created_at` | TEXT | `datetime('now')` |

#### `node_relations` — ノード間の型付き関係（Phase 6a / migration 0017）

paragraph/theorem/proof 等のノードから、それが参照する equation/theorem/figure/section/bibliography_entry ノードへの**有向辺** `(from_node, relation_type, to_node)`。build のトランザクション内で純関数 `ingestion::graph::resolve_relations` が解決して張る（`ingestion::graph`）。**原文由来と推定を必ず区別**する: **TeX**（`origin='tex_source'`・confidence 0.9）は本文に原文のまま残る `\ref`/`\eqref`/`\cite` を `\label`（親ノードの `payload_json.labels`）/ `\bibitem` の cite key（`payload_json.cite_key`）と照合。**PDF**（`origin='layout_model'`・confidence 0.6–0.7）は本文の "Theorem 2.3" / "Eq. (2.1)" を定理番号（`payload_json.theorem_number`）/ 数式番号（`math_expressions.equation_label`）と照合（PDF は `\label` を復元できないため番号一致）。**解決できない参照（ターゲット不在・自己参照）は張らない**（誤検出より欠損）。`relation_type` = `cites` / `refers_to_equation` / `refers_to_theorem` / `refers_to_figure` / `refers_to_table` / `refers_to_section` / `refers_to`（一般）/ `proves`（proof → 証明する定理）。記号系（`symbols`/`symbol_occurrences`・Phase 6b）は別 migration で追加する。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `document_version_id` | INTEGER FK → document_versions | ON DELETE CASCADE。版単位で引く/掃除する |
| `from_node_id` | INTEGER FK → document_nodes | ON DELETE CASCADE。参照元 |
| `relation_type` | TEXT NOT NULL | `cites`/`refers_to_*`/`proves`/…（自由 TEXT・`document_ir::RelationType`） |
| `to_node_id` | INTEGER FK → document_nodes | ON DELETE CASCADE。参照先 |
| `confidence` | REAL | 参照解決の確からしさ（意味の確からしさではない） |
| `origin` | TEXT | `tex_source`（原文由来）/ `layout_model`（PDF 推定） |
| `metadata_json` | TEXT | 生の参照文字列・突き合わせたキー/番号など |
| `created_at` | TEXT | `datetime('now')` |

#### `symbols` / `symbol_occurrences` — 記号定義とその出現（Phase 6b / migration 0018）

論文が定義する記号（"let $U$ be ...", "define $H$ as ...", "denote by $\mathcal{H}$ ...", "$U := ...$"）を、TeX 本文のインライン数式 `$...$` から抽出する（`ingestion::symbols`）。**TeX 版のみ**（PDF はインライン数式が区切り無しで潰れ記号を切り出せない）。surface_form/description は原文の verbatim だが「この文がこの記号を定義している」対応づけは**ヒューリスティック推定**なので `confidence` で区別する（強いトリガ + インライン数式が揃ったときだけ拾う＝誤検出より欠損）。`symbol_occurrences` は保守的に、**定義済み記号が display 数式に表層一致した箇所だけ**を記録する。同一節内の同一表層の再定義は 1 個に畳む。version 削除でカスケード消去。

`symbols`:

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `document_version_id` | INTEGER FK → document_versions | ON DELETE CASCADE |
| `surface_form` | TEXT NOT NULL | 記号の表層（"U" / "\mathcal{H}" / "U_0"） |
| `normalized_form` | TEXT | 装飾（\mathbf/\mathcal 等）を剥いた正規化・任意 |
| `description` | TEXT | 定義文から取り出した説明（インライン数式を含めてよい） |
| `symbol_type` | TEXT | operator/matrix/set/graph/… の推定・任意 |
| `defined_at_node_id` | INTEGER FK → document_nodes | ON DELETE CASCADE。定義位置（"jump to definition"） |
| `scope_node_id` | INTEGER FK → document_nodes | ON DELETE SET NULL。定義を含む節（軽いスコープ） |
| `semantic_json` | TEXT | 未モデル化の意味属性（後続） |
| `confidence` | REAL | **定義検出**の確からしさ（意味ではない） |
| `origin` | TEXT | `tex_source` |
| `created_at` | TEXT | `datetime('now')` |

`symbol_occurrences`: `id` / `symbol_id`(FK → symbols CASCADE) / `node_id`(FK → document_nodes CASCADE) / `local_offset_json` / `surface_form` NOT NULL / `confidence`（表層一致は近似）/ `origin` / `created_at`。

---

## 設計上の注意

- `PRAGMA foreign_keys = ON` はRustの接続初期化時に毎回設定する（SQLiteはデフォルト無効）
- WAL モード（`journal_mode = WAL`）はマイグレーションで一度設定すれば永続化される
- `updated_at` の自動更新はRust側のupdateコマンドで `datetime('now')` をセットする（SQLiteにはUPDATEトリガーを使う方法もあるが、シンプルさを優先）

## 複合フィルタ（v0.6.0）とスキーマ

- v0.6.0 の一覧フィルタ（種別 / 年範囲 / スター / 添付 PDF / 複合タグ）は**既存の列とテーブルのみで実装**する（`entries.entry_type` / `entries.year` / `entries.starred` / `attachments` / `entry_tags`）。**migration 追加なし**。フィルタは `EntryFilter`（`models.rs`）を受け取り `sqlx::QueryBuilder` で動的に WHERE 句を組む（`db::entries` 参照）
- **未読 / 既読フィルタ（将来検討・未実装）**: 需要はあるが現行スキーマに既読状態の列が無い。実装するなら `entries` に `read_at TEXT`（NULL=未読）または `is_read INTEGER DEFAULT 0` を追加する migration が必要。`read_at` 方式なら「いつ読んだか」も残せて拡張性が高い。フィルタ基盤（`EntryFilter`）は列追加＋軸 1 本追加で対応できる設計。詳細は `SPEC.md`「v0.6.0 > 将来検討事項」を参照
