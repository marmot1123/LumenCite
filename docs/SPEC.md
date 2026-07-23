# LumenCite 機能仕様

## MVP（v0.1.0）

### 文献管理
- エントリの登録・編集・削除（論文・書籍・Webページ・その他）
- DOI / ISBN / arXiv ID からメタデータ自動取得
- タグ・コレクション（フォルダ）管理
- BibTeX インポート・エクスポート（Zoteroからの移行対応）
- 基本的な検索・フィルタリング

### PDF / 詳細ビュー
- **詳細ビュー全体のデザイン刷新**（`design/design_handoff_detail_view/`）
  - 3ペイン構造: 左サムネイル（96px） / 中央 PDF / 右メタパネル（340px・4タブ）
  - メタパネルタブ: **情報 / ハイライト / ノート / 関連**（既存の info/abstract/notes/related から再編、abstract は info に統合）
  - PDFツールバー: ページナビ / ズーム（50–200%、10%刻み）/ 注釈モード（選択・ハイライト・ノート・ペン）/ 本文検索
  - 状態永続化: `zoom`, `leftOpen`, `rightOpen`, `metaTab` は localStorage、`page` はエントリごとに `settings` 表へ
- PDF テキスト選択 → 3色ハイライト（yellow / green / blue）の作成・保存・ノート付与
- PDF全文検索（既存 FTS5 を継続）
  - **全文索引の手動トリガ（v0.7.0 追加）**: 通常は添付時に自動索引するが、過去に添付済み・索引失敗のエントリ向けに任意タイミングで再索引できる。詳細パネルの各添付に**索引状態バッジ + 索引/再索引ボタン**（`index_attachment`）を、設定 → データに**「未索引の PDF を一括索引」**（`index_missing_attachments` = `attachments_without_fulltext` で未索引 PDF を洗い出し順次 `pdf-extract` → `fulltext`）を用意。テキストレイヤーが無い（0 ページ）添付は「OCR 候補」として集計し、スキャン PDF は詳細ビューの OCR へ誘導する
- キーボードショートカット: `←/→` ページ移動 / `⌘+/⌘-/⌘0` ズーム / `⌘F` 検索 / `⌘[/⌘]` サイドバートグル / `H` ハイライト / `N` ノート / `Esc` 戻る

### 数式表示
- **KaTeX** によるレンダリング（抄録・ノート内の `$…$` / `$$…$$`）
- `react-markdown` + `remark-math` + `rehype-katex` 構成でノートはMarkdownとして描画
- モバイル対応フェーズで [RaTeX](https://ratex.lites.dev/) への移行を評価する

### UI / 多言語・テーマ
- **i18n**: 日本語 / 英語の 2 言語切替（`react-i18next`）。設定モーダルから切替、localStorage 永続化
- **テーマ**: light / dark / **auto**（`prefers-color-scheme` 追従）の 3 モード。設定モーダルから切替
  - PDF ビューワーの別ウィンドウもテーマを継承
- **コマンドパレット**（⌘K）: グローバルアクション（新規エントリ、設定、テーマ切替、.bib同期、エクスポート、アップデートチェック）+ エントリ横断検索

### LLM連携（基本）
- プロバイダ設定: OpenAI / Anthropic（v0.1.0 はこの 2 系統）
- API キーは **OS キーチェーン**（macOS Keychain / Windows Credential Manager / Linux secret-service）に保管。`settings` 表には**平文で書かない**
- **選択エントリの要約**: 抄録 or PDF 全文から生成。トークン上限を超える場合は `pdf-extract` 抽出後にチャンク化
- **ストリーミング表示**: `tauri::ipc::Channel` でトークン単位で UI に送出
- 生成結果は `entries.summary` に永続化（生成モデルと日時も保存）

### LaTeX引用ワークフロー
- `.bib` ファイルの自動エクスポート・同期（VSCode LaTeX Workshop連携前提）
  - 同期先パスは設定モーダル（サイドバー右下の同期アイコン）で指定
  - ミューテーション後 800ms デバウンスで自動書き出し（ゴミ箱を除く全エントリ）
  - 「今すぐ同期」ボタンで即時同期も可能
- **編集可能な cite key**（v0.2.1 で追加）: 各エントリの BibTeX エントリキーをユーザーが固定（ピン留め）できる。
  - 未設定なら従来どおり `第一著者姓+年` から自動生成。同一 `.bib` 内の重複は接尾辞 `a`/`b`/`c` で回避
  - インポート時は元 `.bib` のキーを保持（衝突時は接尾辞付与）
  - 固定キーはグローバル一意。編集フォームで重複を事前チェック
  - 詳細は `DATA_MODEL.md` の `citation_key` 節 / `API_SPEC.md` 参照

### データ保全 / 配布
- **自動バックアップ（CR-018: 添付本体込み）**: アプリ起動時 + 1日1回、`<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.zip` に**完全バックアップ**を作成。アーカイブは `db.sqlite`（`VACUUM INTO` によるクリーンコピー。highlights/chat/settings/fulltext 込み）＋ `attachments/<entry_id>/<file_name>`（添付本体）を deflate 圧縮で束ねる。14世代まで保持（旧 `.db` バックアップも世代管理・一覧の対象）
- **復元（CR-018）**: 設定 → データの「復元」から backup `.zip` を選ぶと、①稼働中に検証（`db.sqlite`・`PRAGMA integrity_check`・スキーマ版）＋復元前の自動フルバックアップ → `pending-restore/` へ展開、②アプリ再起動、③起動時に pool を開く前へ現行 DB＋添付を `pre-restore/` へ退避してから staged を差し替え（失敗時は自動ロールバックして旧 DB のまま起動継続）。ライブ DB を握ったままの上書きを避ける「次回起動時適用」方式
- **手動エクスポート**（CR-018 で範囲を明確化）: いずれも**エントリのメタデータ書き出し**であり、PDF 添付・ハイライト・チャット履歴・設定は含まず、再インポートによる復元もできない。
  - JSON: エントリのメタデータ（`EntryDetail[]`）
  - BibTeX: 引用情報（既存）
  - Markdown: ノート＋要約
- **Tauri auto-updater**: `tauri-plugin-updater` 経由。署名鍵で検証、GitHub Releases の `latest.json` を参照
- **コード署名**:
  - macOS: Developer ID Application + notarization（v0.1.0 配布前に必須）
  - Windows: コード署名証明書（配布対象に含めるなら必須、未対応なら v0.2.0 送り）

---

## v0.2.0

v0.1.0 で文献管理 / PDF ビュー / 単一エントリの LLM 要約まで揃った。v0.2.0 は LumenCite を **「研究の壁打ち相手」** として実用化するフェーズ。実装プランは `~/.claude/plans/v0-2-0-goofy-tome.md` を参照。

### Agentic LLM Chat（複数文献横断）
- **独立スクリーン**として Chat 画面を追加（App の `screen` 状態に `"chat"`）。サイドバー / コマンドパレット（⌘K）/ ライブラリ複数選択 / 詳細ビューから起動
- **Agentic keyword retrieval**: LLM が `fulltext_search`（FTS5）を tool 経由で反復呼び出ししながら回答を組み立てる
- **コンテキストスコープ（ハイブリッド）**: セッションごとに「DB 全体検索（`scope_mode='all'`）」/「特定文献に絞る（`'entries'`）」を切替
- **ツール呼び出し UI**: 検索・DB 書き換え・MCP 呼び出しを折りたたみ可能ブロックで全展開可視化。**ストリーミング中断ボタン**あり
- ストリーミング配信は `tauri::ipc::Channel<ChatStreamEvent>`（既存 `SummarySheet` の Channel 受信パターンを踏襲）

### チャット履歴の永続化
- `chat_sessions` / `chat_messages` / `chat_session_entries` の 3 テーブル（migration 0007）に保存
- アプリ再起動後もサイドバーから過去セッションを再開できる
- タイトルは最初のターン後に LLM が自動生成（ユーザー編集可）

### LLM への DB 書き換え権限（tool use）
- タグ付け・ノート追記・OCR 結果保存などを対話で実行
- **ツール別ホワイトリストで承認制御**:
  - read 系（`fulltext_search` / `get_entry` / `list_*`）: 常に自動承認
  - `add_tag` / `update_notes` / `attach_ocr_text` / `add_to_collection`: デフォルト自動（設定で都度承認に変更可）
  - `create_entry` / `update_entry`: 都度承認
  - `delete_*` / MCP の write 系: 常時確認（ホワイトリストで上書き不可）
- ホワイトリストの上書きは `settings` の `chat.tool_whitelist` キーに JSON 保存
- ロールバック専用 UI は設けず、既存の trash + 日次バックアップ（14 世代）で対応

### MCP クライアント
- Chat 内 LLM が外部 MCP サーバー（Obsidian 等）のツールを利用可能
- stdio で外部 MCP サーバープロセスを起動・管理し、起動時に `tools/list` を取得して Chat ツールスキーマへ動的マージ（プレフィックス `mcp_<server>_<tool>`）
- サーバー設定は Claude Desktop の `mcpServers` JSON 互換形式
- **クライアントのみ**。LumenCite を MCP サーバーとして公開するのは v0.3.0

### スキャン PDF の LLM Vision OCR
- テキストレイヤーのないスキャン PDF を LLM Vision で OCR し、`fulltext` に保存して全文検索可能にする
- トリガーは **LLM ツール（`ocr_pdf`）経由** と **詳細ビューの手動ボタン** の両対応
- **OCR プロバイダ設定を Chat とは独立**に保持（将来のローカル LLM 対応に備える）。未設定時は Chat プロバイダへフォールバック

### macOS auto-updater 有効化
- v0.1.0 で見送った `tauri-plugin-updater` を **macOS のみ有効化**。GitHub Releases の `latest.json` を ed25519 鍵で検証
- Windows のコード署名 + updater は v0.2.1（Certum 取得後）に送り

### v0.2.0 スコープ外（将来）
- MCP **サーバー**実装（v0.3.0）
- Windows コード署名 + Windows updater（v0.2.1）
- Homebrew Cask 登録（DL 実績が育ってから別作業 → Phase 2 参照）
- CSL / Web クリッパー / カスタムハイライト色（Phase 2 残）
- 古典的 RAG（埋め込みベクトル検索）— v0.3.0 で FTS5 agentic 運用結果を見て判断
- ローカル LLM プロバイダ（Ollama / LM Studio）— v0.3.0+。OCR プロバイダ独立化は本バージョンで先行整備済み

---

## v0.3.0

### 著者モデルの多言語・国際識別子対応

文献メタデータ取得の精度向上と、漢字圏・キリル圏の著者を一級市民として扱うため `authors` テーブルを大幅拡張する。スキーマ定義の詳細は `docs/DATA_MODEL.md` の `authors` / `author_identifiers` セクション、確定経緯は `memory/project_authors_v030.md`、実装順序・マイルストン分割は `~/.claude/plans/v0-3-0-authors-radiant-kana.md` を参照。

**追加フィールド（migration 0009）:**
- 名前構造: `middle_name` / `suffix`（Jr., III）/ `name_particle`（von, van der）
- オリジナル言語表記: `name_original` + `given_name_original` / `family_name_original` + `original_script`（ISO 15924）
- 読み仮名: `reading_family` / `reading_given` — **日本語著者の五十音ソート・かな検索のため必須**
- 団体著者: `is_organization` フラグ — BibTeX `{IEEE}` 等を自動検出。CSL の literal 相当
- 追加属性: `email` / `homepage_url` / `notes` / `updated_at`

**新規テーブル `author_identifiers`（migration 0009）:**
- ORCID 以外の識別子（Scopus / DBLP / Semantic Scholar / Wikidata / ISNI / VIAF / ResearcherID / Google Scholar）を `(author_id, scheme, value, url)` で正規化保持
- 追加スキームのたびに migration 不要
- 既存 `authors.orcid` 専用カラムは v0.3.0 時点では互換維持のため残し、新規取得時は両方に書く

**名寄せロジックの改善（`get_or_create_author`）:**
1. ORCID があれば ORCID で照合
2. なければ正規化済み name（trim + Unicode NFKC + lowercase）で照合
3. それでもなければ INSERT

これにより「ORCID 同一・name 表記揺れ」での著者重複を防ぐ。

**FTS への反映:**
- `entries_fts.authors_text` に `name_original` と読み仮名（`reading_family || ' ' || reading_given`）を追記し、「せき」「関」「Seki」のどれでもヒットさせる

**UI:**
- `AddSheet` / `EditSheet` / `DetailPanel` の著者編集 UI を拡張して新フィールドを編集可能にする
- DOI / arXiv / OpenLibrary メタデータ取得時に ORCID 以外の identifier も拾えるなら自動投入
- BibTeX インポート時、`{...}` で囲まれた著者を `is_organization=1` として登録

### MCP サーバー公開（LumenCite を MCP サーバーに）

Claude Desktop / Claude Code などの MCP クライアントから LumenCite のライブラリを参照・操作できるよう、LumenCite 自身を MCP **サーバー**として公開する。動機は「Claude のサブスクリプション枠を活用する」こと — サーバー側では LLM を呼ばず、推論は接続元（サブスク認証）が担うため API キーは不要。クライアント実装（v0.2.0 の外部 MCP 接続）とは逆向きの機能。

**アーキテクチャ（確定）:**
- **アプリ内蔵**: 起動中の LumenCite アプリ内に localhost HTTP（JSON-RPC 2.0 / Streamable HTTP）でサーバーを立てる。DB を単一プロセスが所有するため WAL 競合が無く、既存 `db::*` と .bib 同期コーディネータ（`sync_tx`）を再利用でき、変更を UI に即時反映できる。独立 stdio バイナリ案は二重 writer 問題と stale UI のため不採用。
- **認可**: `Authorization: Bearer <token>`。token はインストールごとに生成（SQLite `randomblob`）し OS キーチェーンに保管。localhost バインドと併せ同一マシンの他プロセスからの無断アクセスを防ぐ。
- **ツール定義の単一ソース化**: アプリ内チャットの read 系ツール定義（`llm::tools::search`）を流用し、定義の二重管理を避ける。

**Phase 1（実装済み — read-only MVP）:**
- localhost HTTP サーバー（`tiny_http`）+ Bearer 認可 + JSON-RPC ディスパッチ（`initialize` / `tools/list` / `tools/call` / `ping`）
- 公開ツールは **read 系のみ**: `fulltext_search` / `get_entry` / `list_collections` / `list_tags`（チャットから流用）＋ LaTeX 連携向けの `search_entries` / `resolve_citation_key` / `export_bibtex`。write/mutate/ocr は非公開（許可リスト外として拒否）
- 設定 `mcp_server.enabled` / `mcp_server.port`、Tauri コマンド `get_mcp_server_status` / `set_mcp_server_enabled` / `regenerate_mcp_server_token` / `get_mcp_server_config_snippet`（Claude Code 用の貼り付け設定生成）
- Claude Code はリモート MCP として直結（`claude mcp add --transport http ...`）

**Phase 2（実装済み — write 公開＋ゲート）:**
- write 系を `mcp_server.write_enabled`（一括ゲート・**デフォルト false**）が有効なときだけ公開: `add_tag` / `update_notes` / `add_to_collection` / `create_entry` / `update_entry`。**破壊系 `delete_entry` は常に非公開**（許可リスト外で `tools/call` でも到達不可）。承認 UI が無いためサーバー側でこのゲートを enforce する（設定はリクエスト毎に評価し、トグル変更は再起動なしで即反映）。
- write 成功時は **監査ログ**（`mcp_audit_log` 表 / migration 0010）に記録し、`.bib` 自動同期キック（`sync_tx`）＋ 一覧へのライブ反映（`entries-changed` イベント → `loadEntries`）を発火する。
- 設定 UI に write 許可トグル（警告付き）、Tauri コマンド `set_mcp_server_write_enabled` / `get_mcp_audit_log`。`get_mcp_server_status` は `write_enabled` を返す。

**Phase 3（実装済み — Claude Desktop 向け stdio shim）:**
- stdio しか使えない Claude Desktop 向けに、本体バイナリ自身を `--mcp-stdio` 付きで起動すると「stdio↔localhost HTTP プロキシ」として振る舞う（`main.rs` が GUI 起動前に検出 → `mcp_shim::run_stdio_proxy`）。別 sidecar バイナリにしないことで追加の署名・notarize 対象を増やさない。
- 接続先は Claude Desktop 設定の `env`（`LUMENCITE_MCP_URL` / `LUMENCITE_MCP_TOKEN`）で受け取る。`get_mcp_server_config_snippet("claude_desktop")` が `command`=現在の実行ファイル絶対パス・`args`=`["--mcp-stdio"]`・`env` 込みの `mcpServers` JSON を生成し、設定 UI に Claude Code / Claude Desktop 双方のスニペットを表示する。
- **堅牢化（レビュー対応）**: shim は URL/トークン未設定を起動時にエラー化、id 付きリクエストへの空ボディはハング防止に JSON-RPC エラー化、非 UTF-8 の stdin 行はセッションを落とさず読み飛ばす。`command` は `current_exe()` の絶対パスを埋め込むため、**/Applications へ設置してからスニペットをコピー**する旨を UI に警告表示（移動・再ビルド・App Translocation でパスが無効化するため）。**検証は macOS の Claude Desktop で実施**。Windows の GUI-subsystem 子プロセスでの stdio 継承は未検証（将来 Windows 対応時に要スモークテスト）。

**Phase 3.5（実装済み — LCIR read ツール）:**
- LCIR（機械可読中間形式）を外部 LLM に露出する read 系 3 ツール。`get_fulltext`（平坦なページ全文）と違い、論理構造・数式・PDF 座標を渡せる。
  - `get_document_structure`（entry_id/citation_key）: 節アウトライン（`section_number`/`level`/`page`）＋ブロック種別カウント＋abstract。論文の地図。
  - `get_document_blocks`（entry_id/citation_key・`kinds`/`page` フィルタ・`max_chars` ページング）: 構造タグ付きブロックを読み順で返す。`kinds=["display_math"]` で数式だけ（`equation_label`＋表層文字列）、`["section","paragraph"]` で本文。**PDF 由来の数式は表層のみ（LaTeX ではない）**。
  - `search_document_nodes`（query）: ブロック粒度検索（`fulltext_search` はページ粒度）。ヒットに `node_kind`＋`page`＋`bbox`（[x,y,w,h]・PDF pt・左下原点）を返し、該当ブロックを直接ハイライトできる（PDF 由来の LCIR のみ索引）。
- LCIR 未構築のエントリは `has_lcir:false` を返す（`get_fulltext` へ退避）。write ではないのでゲート不要。実 PDF で end-to-end 疎通確認済み（構造・数式・検索が MCP 経由で読める）。
- **LCIR Phase 4（TeX 取込）**: 詳細パネルの「TeX ソース取得」で arXiv e-print をダウンロードし、`lumencite-tex` 抽出器が PDF 版と**併存する別表現**を作る。エントリに両方あるときは read ツールが **TeX 版を優先**し（数式は**生 LaTeX** の `latex` フィールド付き・`semantic_status='source_provided'`）、`source` 引数（`"tex"`/`"pdf"`）で明示切替・`available_sources` で一覧できる。`page` フィルタは PDF 版専用（未指定 source なら PDF 版へ自動フォールバック）。
- **LCIR Phase 5（定理・定義・証明）**: `theorem`/`lemma`/`proposition`/`corollary`/`definition`/`remark`/`example`/`proof` を型付きノードとして認識する（新規テーブルなし）。**TeX** は環境名 + preamble の `\newtheorem` 宣言から種別を決め（原文由来・高信頼）、`[note]`・`\label` を捕捉。**PDF** は行頭キーワード + 番号で信頼度付きに認識し、番号（`theorem_number`）と付記名（`note`）を持つ。`get_document_blocks(kinds:["theorem","proof"])` で「定理と証明を一問い合わせ」で取得でき、応答に番号・付記名が付く。定理間参照グラフは Phase 6（`node_relations`）。

**後続フェーズ（未実装）:**
- Phase 4（任意）: MCP *resources*（`lumencite://entry/{id}` で論文を @メンション）／監査ログの閲覧 UI
- **OpenAI ChatGPT / Codex 対応**: MCP サーバーはプロトコル汎用（JSON-RPC 2.0 / localhost HTTP + Bearer / stdio shim / トランスポート非依存のツールレジストリ）に実装済みのため、Claude 以外のクライアントへも拡張しやすい。**Codex（OpenAI CLI）対応は v0.5.0 で実装済み**: `get_mcp_server_config_snippet` に `"codex"` arm を追加し、`~/.codex/config.toml` の `[mcp_servers.lumencite]` TOML（既存 `--mcp-stdio` shim を stdio 起動）を設定 UI に表示する。Codex 実機で end-to-end 疎通確認済み。**ChatGPT connector** は公開到達可能なリモート + OAuth を要求しがちで localhost + Bearer では繋がらない可能性が高く、別スコープ（要件調査を先行）。

### その他の v0.3.0 候補（要検討）

- 古典的 RAG（埋め込みベクトル検索）— v0.2.0 の FTS5 agentic 運用結果を見て採否判断
- ローカル LLM プロバイダ（Ollama / LM Studio）

---

## v0.5.0

### Web クリッパー（Chrome 拡張 + ローカル HTTP API）

論文ページでツールバーボタンをクリックすると、起動中の LumenCite にエントリを作成する。Phase 2 残の「ブラウザWebクリッパー」を消化する v0.5.0 の目玉機能。

**スコープ（v1）:**
- **識別子ベース抽出**: ページの meta タグ（`citation_doi` / `citation_arxiv_id` / `citation_isbn` / `citation_pdf_url` / `DC.Identifier`）と URL パターン（`arxiv.org/abs|pdf/...`、`doi.org/10.*` canonical）から DOI / arXiv ID / ISBN を抽出。メタデータの解決・重複判定・エントリ作成は**すべてアプリ側**（既存 `metadata.rs` / `find_duplicate_entry` / `create_entry` を再利用）で行い、拡張は「識別子を抜いて POST するだけ」の薄い実装
- **フォールバック**: 識別子が無いページは `webpage` エントリ（title + URL + OG タグの日付/サイト名）として保存
- **PDF 自動添付**: `citation_pdf_url` または arXiv ID から導出した PDF URL をアプリ側でダウンロードして添付（50MB 上限・`%PDF-` マジックバイト検証・タイムアウト付き。ペイウォール等で失敗してもエントリ作成は成功扱い）
- **TeX ソース自動取得（LCIR Phase 4 の自動化）**: arXiv クリップで **`lcir.enabled` が ON のときだけ**、e-print も取得して LCIR（構造 + 生 LaTeX 数式）を自動構築する。OFF なら取得しない。重複クリップでは再取得しない（詳細パネルのボタンで明示再取得可）
- **重複**: 既存エントリ（DOI/arXiv/ISBN 一致）があれば作成せず duplicate 応答 → 拡張はバッジで通知
- **対象ブラウザ**: Chrome（Manifest V3）。WebExtension 標準準拠で実装し Firefox は将来の小差分。配布は v1 では load-unpacked + GitHub Releases の zip（Chrome Web Store は後日）

**アーキテクチャ:**
- 通信路は既存の localhost HTTP サーバー（MCP サーバーと同一プロセス・同一ポート・同一 Bearer トークン）にパスベースルーティングを追加し `/clipper` を新設。JSON-RPC（`/mcp`）は無変更で後方互換
- **同意モデル**: 新設定 `clipper.enabled`（デフォルト off）。`mcp_server.write_enabled` とは独立のゲート（クリッパーは拡張のインストール＋接続コード貼り付けという別の同意面を持つため）。サーバープロセスは「MCP 有効 OR クリッパー有効」で起動
- **ペアリング**: 設定画面の「接続コード」（`lc1.` + base64url の `{v, port, token}`）をコピーして拡張のオプションページに貼り付け。トークン再生成でペアリングは無効化される（設定 UI に注記）
- 拡張は常駐 content script を持たない: アクションクリック時のみ `chrome.scripting.executeScript` で抽出関数を注入（権限は `activeTab` / `scripting` / `storage` / `notifications` と `http://127.0.0.1/*` のみ）
- リポジトリは monorepo: `extension/` パッケージ + pnpm workspace 化

**重複クリップ時の欠落補完（v0.8.0 実装済み・2026-07-19）:**

重複クリップ（エントリが既に在る）でエントリに PDF/TeX が欠けていれば補完する。「欠落分だけ補完する。ただし**初回は確認**を取り、以後は確認なしを選べる」設計。**確認 UI はツールバーボタン直下の拡張ポップアップ**（ユーザー要望・2026-07-19）— クリックした場所で確認が完結し、ブラウザ→アプリをまたぐ非同期 UX を持ち込まない。

- **新設定 `clipper.complete_missing`（アプリ側・全取込経路で共有）**: 未設定 = 初回確認 ／ `"1"` = 確認なしで自動補完。**判断は常にアプリ側**で行い、拡張は応答に従うだけの stateless 設計（AddSheet とも設定を共有するため）。
- **欠落検出**（duplicate 判定直後・エントリ単位）: PDF 欠落 = mime `%pdf%` の添付なし かつ クリップから PDF URL が導出できる ／ TeX 欠落 = mime `application/gzip` の添付なし かつ arxiv_id あり かつ **`lcir.enabled` ON**（既存の TeX 自動取得と同一ゲート）。TeX は上書き契約（LCIR Phase 4）なので「欠落」= 添付行の有無のみ（在れば対象外）。
- **duplicate 応答を拡張**: 設定 `"1"` なら即補完し `completing: ["pdf","tex"]` を返す（バッジで「補完中」を表現可能）。未設定かつ欠落ありなら `confirm_missing: ["pdf","tex"]` を返す（この時点では何もしない）。
- **拡張ポップアップ（ボタン直下の確認）**: service worker は `confirm_missing` を受けたら pending payload（entry_id/title/missing）を `chrome.storage.session` に置き、`chrome.action.setPopup({popup:"confirm.html"})` → **`chrome.action.openPopup()`**（Chrome 127+。使えない環境ではバッジ `?` を出し、次のボタンクリックがポップアップを開くフォールバック）。ポップアップの選択肢は「補完する」／「今回はしない」／「**次回以降は確認せず補完する**」。選択後は payload を消して `setPopup({popup:""})` で通常動作に戻す。
- **新エンドポイント `POST /clipper/complete`**: `{entry_id, remember?: bool}`（同一 Bearer 認証・`clipper.enabled` ゲート）。アプリ側で欠落を**再検証**してから既存の `spawn_pdf_job` / `spawn_tex_source_job` を発行し、`remember` なら `clipper.complete_missing="1"` を保存。PDF URL・arxiv_id はクリップ時の値をアプリ側で保持せず、エントリの識別子から再導出する（arXiv 導出 PDF / e-print。`citation_pdf_url` 由来の補完はこの版では対象外 = arXiv 前提で十分。ゴミ箱のエントリは `deleted_at IS NULL` で弾き、confirm 後に trash された TOCTOU も空プランにする）。
- **AddSheet 側も同じ設定に従う**: `create_entry` の CR-019 dedup で既存エントリが返るケースはアプリ内のインライン確認で尋ねる（選択肢は同じ 3 つ・WKWebView 安全に `window.confirm` 不使用）。「既存か」の判定は submit 直前に `find_duplicate_entry` を再照会して権威的に決める（fetch 時の値は probe 失敗・競合で不正確なため）。あわせて既知の quirk を修正 — 現状 AddSheet はチェック ON だと**既存エントリへ PDF を重複添付し得る**（`download_arxiv_pdf` を無条件実行）ため、返ってきた `entry.attachments` を見て「PDF/TeX 欠落時のみ実行」に変える。
- **拡張ポップアップの実装（重要）**: 確認ページ（`confirm.html`）は表示と選択の受け渡しだけを行う純粋なビューで、実際の `/clipper/complete` 呼び出しと popup の arm/disarm は **service worker が担う**。ポップアップはフォーカス喪失で即破棄されるため、ネットワークをそこに置くと補完が黙って中断し、また popup の解除漏れはツールバーボタンを無反応にする（onClicked は popup 設定中は発火しない）。どの終了経路（選択 / 空 pending / pagehide）でも SW にメッセージを送り、SW が状態を通常へ戻す。`openPopup` は Chrome 127+（`minimum_chrome_version` 宣言）で、失敗時はバッジ `?` + 次クリックで開くフォールバック。

**arXiv TeX ソースの一括取得バッチ（v0.8.0 実装済み・2026-07-19）:**

既存コーパスのバックフィル用。クリッパーの欠落補完は「再遭遇した論文を拾う」増分向けで、手持ちの arXiv エントリ全部に TeX を揃えるにはこちらが本命。

- **設定 → データ**に「arXiv の TeX ソースを一括取得」ボタン（`lcir.enabled` ON のときのみ活性。既存の「未構築 PDF を一括 LCIR 化」ボタンの隣・同じ busy/結果表示パターン）。
- **対象**: ゴミ箱以外で `arxiv_id` があり、mime `application/gzip` の添付が**無い**エントリ。
- 各対象に `download_and_attach_arxiv_source` → `build_lcir_for_attachment` を**直列**実行。**arXiv への礼儀としてリクエスト間 3 秒スロットル**（export.arxiv.org の慣行に合わせバーストしない）。
- PDF-only 投稿（TeX 未公開）は `failed` と分けて `pdf_only` としてカウント（`fetch_arxiv_source` は先頭 5 バイトの `%PDF-` で即打ち切るので再実行のコストは軽微。永続マーカーは持たず、手動バッチの再実行で再判定される割り切り）。
- 結果サマリ `{total, fetched, built, pdf_only, failed}`。Tauri コマンド名は `fetch_missing_arxiv_sources`。
- 数分かかる直列処理なので、**多重起動ガード**（プロセス全体で 1 本・設定を閉じ→開き直しての二重起動を弾く）・`tex-fetch-progress {done,total}` の**進捗イベント**（ボタンに `(done/total)` 表示）・完了時の `entries-changed` 発火を伴う。

**実装済み（取得整備セッション・2026-07-19）**: 上記 2 件（重複クリップの欠落補完 + 一括取得バッチ）と AddSheet の PDF 重複添付 quirk 修正を「取得整備」としてまとめて実装した（LCIR Phase 5 の前 — Phase 5（定理/証明）は TeX ソースの恩恵を最も受けるため、取得面を先に固めた）。拡張 zip の再配布を伴うため、リリース（v0.8.0）は拡張更新と同期させる。実ブラウザ E2E は配布前に必須。

### その他の v0.5.0 候補

- 更新通知（GitHub API で全 OS「新版あり」通知のみ — Windows/Linux は手動 DL 誘導）
- Codex（OpenAI CLI）向け MCP 設定スニペット（上記 Phase 3 shim の流用）

---

## v0.6.0

### 一覧の複合フィルタ（Filter）

ツールバーの「フィルタ」ボタン（v0.5.0 まではプレースホルダで無反応）に、複数条件を **AND で重ね掛け**して一覧を絞り込むパネルを実装する。全ユーザー・特に非技術層に効く必須 UX。CLI 等パワーユーザー向け機能より優先する（ロードマップ判断）。

**フィルタ軸（v0.6.0 スコープ）:**
- **種別（entry type）**: 19 種から複数選択（選択どうしは OR）。例: `article` OR `book`
- **年（year）**: 範囲指定（`year_min` 以上 / `year_max` 以下 / 区間 / 片側のみ）
- **スター（starred）**: 「star 付きのみ」/「star なしのみ」/「指定なし」の 3 値
- **添付 PDF（has_attachment）**: 「添付あり」/「添付なし」/「指定なし」の 3 値
- **タグ（複合）**: 複数タグを選び、**AND（すべて含む）/ OR（いずれか含む）** を切替。現状サイドバーの単一タグ選択（scope）とは独立で、フィルタ側で複数指定できる

**セマンティクス:**
- 各軸どうしは AND。空（未指定）の軸は制約を課さない
- サイドバーのビュー選択（コレクション / タグ / starred / unfiled / trash = **scope**）と **AND で合成**する。例: 「コレクション A を選択」＋「フィルタで種別=article・2020 年以降」→ A に属し `article` かつ `year>=2020` のもの
- **検索（`search_entries`）にも同じフィルタを適用**する（メタ検索の結果をさらに絞る）。全文検索（`fulltext_search`）は v0.6.0 では未対応（対象外）
- ゴミ箱（trash）ビューでもフィルタは有効

**UI:**
- ツールバーのフィルタボタン → ポップオーバー。適用中は件数バッジ、ワンクリックで全クリア
- フィルタ状態はビュー切替をまたいで保持（明示クリアするまで持続）。フロント state で保持し、backend へは `EntryFilter` オブジェクトとして渡す

**非対象（将来検討）:** 下記「将来検討事項」参照。

### 将来検討事項（v0.6.x 以降）

- **未読 / 既読フィルタ**: 「まだ読んでいない文献」を絞る需要は文献管理の定番ニーズ。ただし現行スキーマに既読状態を表す列が無く、実装には (1) `entries.read_at`（または `is_read`）を追加する migration、(2) 詳細/一覧での既読トグル UI、(3) 既読フィルタ軸の 3 点が必要で、v0.6.0 の他フィルタ軸（既存スキーマのみで完結）より工数が大きい。v0.6.0 では**見送り**、別バージョンで単独検討する。フィルタ基盤（`EntryFilter`）は列追加のみで拡張できるよう設計しておく
- 著者・ジャーナルでの絞り込み、保存済みフィルタ（スマートコレクション）も候補

---

## v0.7.0

### CLI（AI エージェント / スクリプト向けコマンドライン）

LumenCite ライブラリを **ターミナルから直接読める** CLI を実装する。第一の対象は「Zed / Claude Code / Codex 等の AI エージェント × LaTeX 執筆」ワークフロー（`lumencite-bib` Skill の駆動基盤）と、シェルスクリプト連携。GUI を起動せずヘッドレスで動く。

**起動形態（本体バイナリ再利用）:**
- 新規バイナリを増やさず（署名・notarize 対象を増やさない）、`main.rs` で `argv[1]` が既知の CLI サブコマンドなら Tauri/GUI を起動せず CLI として実行する。既存の `--mcp-stdio` shim と同型。
- 引数なし = 従来どおり GUI 起動。`--mcp-stdio` = 従来どおり stdio shim。

**バックエンド接続:**
- **読取コマンド**は原則「読みは自由」に従い **SQLite を直接読む**。接続は `PRAGMA query_only = ON` を全コネクションに適用した読取専用プールで開き、読取経路が絶対に書き込まないことを構造的に保証する。GUI アプリ起動中でも WAL の並行リーダーとして安全に共存でき、アプリ停止中でも動作する（CLI の主用途）。
- **書込コマンド**は**ハイブリッド C** でルーティングする（地雷＝「アプリ起動中 × 直接 DB 書込」による UI 陳腐化 / WAL 競合を回避）:
  1. `--force` 指定 → 直接 DB 書込（アプリが開いていれば一覧が陳腐化しうる旨を stderr に警告）。
  2. MCP サーバーに到達可（keychain にトークン有 + localhost へ `ping` 成功）→ **HTTP 経由**でサーバーに委譲。サーバーが公開用の書込ゲート（`mcp_server.write_enabled`）を適用し、成功時は `.bib` 同期と GUI 一覧のリアルタイム更新まで行う（＝UI が陳腐化しない安全経路）。到達可だが書込ゲート off の場合は「アプリ設定で有効化するか `--force`」を明示する。
  3. 到達不可（アプリ停止と判断）→ **直接 DB 書込**。成功後に `.bib` 同期を best-effort で行う。
- 実装は単一ソース: どちらの経路も MCP の `tools/call`（JSON-RPC）と同じリクエスト形状を作り、HTTP なら POST、直接なら `mcp_server::handle_rpc_with_write` を `write_on = true` で呼ぶ（ツール実装・監査ログ・`mutated` フラグを共有。書込は監査ログにも残る）。書込対象は MCP の write ツールに揃える（`create_entry` / `update_entry` / `update_notes` / `add_tag` / `add_to_collection`。破壊系 `delete_entry` は非公開）。
- DB パスは Tauri の `app_data_dir` と同一規則で解決する: `dirs::data_dir()` + identifier `com.lumencite.app`（macOS: `~/Library/Application Support/com.lumencite.app/lumencite.db`）。環境変数 `LUMENCITE_DB_PATH` で上書き可（テスト・非標準配置向け）。ライブラリが未作成なら「アプリを一度起動してください」と明示エラーにする（勝手に空 DB を作らない）。

**出力形式:**
- 既定は **JSON**（AI エージェント / `jq` 連携が主用途）。`--human` フラグで人間可読なテキスト出力に切替。書込コマンドはツールの結果メッセージ（例: `Entry created with id=42.`）を stdout に出す。
- 正常系は stdout、エラー / 警告は stderr。終了コードは 成功=0 / 使い方エラー=2 / 実行時エラー=1。

**サブコマンド（読取・v0.7.0 スコープ）:**
- `search <query…> [--collection <id>] [--tag <id>] [--type <t>]… [--year-min N] [--year-max N] [--starred] [--has-attachment] [--limit N]` — メタデータ検索（`search_entries_filtered` を再利用）。`EntryFilter` の各軸をフラグで指定できる。
- `get <id|citation_key>` — 単一エントリ詳細（`get_entry` / cite key は `find_entry_id_by_citation_key` で解決）。
- `bib <citation_key…>` — 指定した `\cite` キー群から `refs.bib` を生成（`export_bibtex_by_keys` を再利用。全体キーを維持するため `smith2020a` が化けない）。stdout に BibTeX、解決できなかったキーは stderr に警告。**LaTeX 執筆の中核コマンド**。
- `export [--key <k>…] [--collection <id>] [--tag <id>] [フィルタ軸…]` — 条件に一致するエントリ群を BibTeX 出力（キー指定は `bib` と同義、無指定は検索条件で選択）。
- `tags` — タグ一覧（`get_tags`）。
- `collections` — コレクション一覧（ネスト含む、`get_collections`）。
- `fulltext <query…>` — PDF 全文検索（`search_fulltext`）。ヒットのエントリ・ページ・スニペットを返す。

**サブコマンド（書込・v0.7.0 スコープ。全経路 `--force` 対応）:**
- `add --title <T> [--type <t>] [--year N] [--doi/--isbn/--arxiv/--url/--citation-key/--notes/--abstract <v>] [--author <name>]… [--field <key=value>]…` — エントリ作成（`create_entry`）。
- `update <id|citation_key> [同上フィールドフラグ]…` — 既存エントリの部分更新（`update_entry`。指定フィールドのみ変更。`--citation-key ""` で unpin）。
- `notes <id|citation_key> <text…>` — ノート設定（`update_notes`）。
- `tag <id|citation_key> <tag_name>` — タグ付与（`add_tag`。無ければ作成）。
- `collect <id|citation_key> <collection_id>` — コレクションへ追加（`add_to_collection`）。

**非対象（次版以降）:** 破壊系（`delete` / trash）。DOI/arXiv からのメタデータ自動取得付き `add`（ネットワーク取得は別スコープ）。CSL 引用スタイル。CLI 用の PATH 配置（Homebrew `binary` シンボリックリンク等の配布導線）は別途の単発 Win として扱う。

### arXiv 追加時の PDF 一括ダウンロード

文献追加（AddSheet）の **arXiv タブ**で ID からメタデータを取得すると、プレビュー下に「arXiv から PDF も一緒にダウンロード」チェックボックス（**デフォルト ON**）を表示する。「ライブラリに追加」で `create_entry` の直後に `download_arxiv_pdf` を呼び、`https://arxiv.org/pdf/<id>` を Web クリッパーと同じ `download::download_and_attach`（50MB 上限・`%PDF-` マジックバイト検証・タイムアウト付き）でダウンロードして添付する。

- 添付成功後は**バックグラウンドで全文索引**（`pdf-extract` → `fulltext`）まで自動で行い、直後から PDF 全文検索の対象になる（索引失敗＝スキャン PDF 等は無視し、後追いの手動索引に委ねる）。
- ダウンロード失敗（ペイウォール・ネットワーク障害・ID 不正）でも**エントリ作成は成功扱い**。フロントは警告をログに残すのみで、詳細パネルからの手動添付に誘導する。
- **TeX ソース自動取得（LCIR Phase 4 の自動化）**: **`lcir.enabled` が ON のときだけ**、追加直後に fire-and-forget で `download_arxiv_source` → `build_lcir_for_attachment` も実行する（PDF チェックボックスとは独立・Web クリッパーと同じゲートと best-effort 契約。失敗はログのみ・詳細パネルのボタンで再取得可）。
- 対象は arXiv タブのみ（DOI / ISBN は出版社側の PDF 配布が不定のため対象外）。詳細は `API_SPEC.md` の `download_arxiv_pdf` を参照。

### 将来検討事項（lumencite-bib Skill の駆動方式）

CLI（読取＋書込）が揃ったので、LaTeX 執筆支援の `lumencite-bib` Agent Skill をどう仕上げるかが未決。現状は **MCP 駆動の個人 Skill**（`~/.claude/skills/lumencite-bib/`・リポジトリ非同梱）で dogfood 中。次のセッションで以下を詰める:

- **A. 駆動方式**: (1) **MCP 駆動のまま** — アプリ起動＋MCP サーバー有効が前提。リアルタイム UI 反映と書込ゲートの恩恵。(2) **CLI 駆動へ寄せる** — アプリ停止中でも動き、MCP サーバー設定が不要。読取は `query_only` プールで安全、書込は CLI 側の**ハイブリッド C** が「アプリ起動中は自動で HTTP 委譲（UI 反映）／停止中は直接 DB」を内包するため、CLI に寄せると「常に動く＋起動中は UI 反映」を両取りできる可能性が高い。(3) 併用。
- **B. 配布**: 現状は個人 Skill のみ。リポジトリ同梱で他ユーザーへ配布するか。同梱するなら CLI 駆動の方が前提が軽い（利用者に MCP 有効化を課さない）が、CLI の PATH 配置（Homebrew `binary` 等の配布導線）とセットになる。
- **C. 検証**: 決めた駆動方式で `refs.bib` 生成 / `\cite` 解決 / 欠落追加を E2E。

---

## v0.8.0

### リリース方針（2026-07-19 決定）

**v0.8.0 のスコープ = 現 main の蓄積 + 取得整備**。LCIR の全フェーズ完了を待たない。

- **入るもの**: 1エントリ複数 PDF 添付（下記）／v0.7.0 以降の信頼性・レビュー修正（バックアップ自動リストア + FTS self-heal・BibTeX エスケープ 等）／LCIR Phase 0-4（`lcir.enabled` 既定 OFF の実験機能）／**取得整備**（クリッパー欠落補完 + TeX 一括取得バッチ + AddSheet quirk 修正 — 拡張 zip の更新を伴うため、配布の都合でリリースと同期させる）。**LCIR Phase 5 に入る前に出す。**
- **理由**: LCIR はフラグ既定 OFF でリリースを止める理由にならない／main に既にユーザー価値（特に信頼性修正）が溜まっている／拡張 zip は GitHub Release 添付でしか配布できない。
- **以後のリリース間引き**: LCIR フェーズはフラグ付きで main に積み、リリースは **2〜3 フェーズごと**（例: v0.9.0 = Phase 5+6）。署名・notarize 等のリリース作業コストと配信頻度のバランスを取る。
- **v1.0.0 の看板 = LCIR 完成**: Phase 9/10 到達 + `lcir.enabled` 既定 ON 化のタイミングで「機械可読文献基盤の完成」として 1.0 を名乗る。
- **フェーズ順序の変更と Phase 9 の分割（2026-07-23 決定）**: Phase 6 完了後の実装順は **9a → 8 → 7 → 9b/10**。Phase 9 を **9a（エクスポート第一段 = LCIR JSON + Markdown 書き出し・v0.10.0 予定）**と **9b（JATS/TEI/HTML+MathML — Presentation MathML を出す Phase 7 が本質的前提）**に分割する。9a を前倒しする理由: ①中身（`LcirDocument` 派生ビュー・validation・`get_lcir_document`）は Phase 6b 時点で実質完成しており、残作業はファイル書き出しと Markdown レンダラのみ（migration 不要・依存追加なし・ヒューリスティックなし＝「誤検出より欠損」を構造的に満たす）。②フラグ OFF で main に積んできた Phase 4〜6b の成果（原文 LaTeX 数式・定理番号・cite key）を初めて目に見えるユーザー価値（Obsidian 論文ノート直行の Markdown）に変換できる。③Phase 9 のうち Phase 7 に本質依存するのは 9b だけなので、分割すれば二度手間は生じない。**v1.0.0 の「Phase 9 到達」は 9a を指し、9b は post-1.0 でもよい。**

### LCIR エクスポート（Phase 9a・v0.10.0 予定）

エントリ単位で LCIR を **LCIR JSON**（`LcirDocument` 派生ビューそのまま・validation 通過必須）と **構造付き Markdown**（節見出し・段落・原文 LaTeX 数式・定理/証明・参考文献）へ書き出す。読み出しは MCP と同じ**エントリ→版解決（tex > pdfium 優先・`source` で明示切替）**を共有する。

- **経路は 3 つ**: 詳細パネルのボタン（保存ダイアログ・`lcir.enabled` ON のときのみ表示）／CLI `export-lcir <id_or_key> [--format json|md] [--source tex|pdf] [-o <path>]`（stdout 既定・読取専用）／既存 MCP read ツール（変更なし）。
- **Markdown の品質は由来に依存**: TeX 版は原文 LaTeX（`$..$` インライン温存・display は `$$..$$`）・定理番号・cite key まで出る。PDF 版は surface-only（数式は Unicode 線形のまま・`$$` を付けない）。出力の YAML フロントマターに `lcir_source`（抽出器名・版）を記録し、由来を常に区別する（roadmap §16）。
- **やらないこと（9b へ）**: JATS/TEI/HTML+MathML。embedding・ノードチャンク API は Phase 10。

### LCIR 図表アセット基盤（Phase 8a・v0.10.0 同梱候補）

Phase 8（図表機械可読化）の最小スライス。**PDF 版のみ**（`lumencite-pdfium` 0.5.0→0.6.0・TeX 抽出器は不変）。

- **図領域検出**: ページ内の埋込画像オブジェクト（トップレベル Image のみ）の bbox を近接マージして図領域とし、`figure` ノード（bbox 付き・`origin='layout_model'`・confidence 0.6）を作る。**tikz/pgf 等のベクター図は Image オブジェクトを持たないためアセット 0 件が正当**（誤検出より欠損。数学系コーパスでは体感が薄い既知の限界）。
- **ページ crop アセット**: 図領域をページレンダリング（幅 1600px・OCR と同値）から切り出した PNG として `attachments/<entry_id>/.lcir/` 配下に保存し、`assets`/`node_assets`（migration 0019）で参照する。バイナリは FS・DB は相対パス + SHA-256（ADR #3）。
- **caption 関連付け**: 同一ページの figure caption と幾何ペアリング（相互最近のみ・曖昧なら張らない）して `caption_of` 辺を張り、caption の番号（"Figure 2" → "2"）を figure ノードの `figure_number` に載せる。
- **読み出し**: MCP `get_figures`（図番号 → 画像パス・caption・本文位置を一問い合わせ）+ `LcirDocument` に `assets` が透過で載る（JSON エクスポート含む）。
- **やらないこと**: 表のセル構造化（8b・TeX tabular 救出）／XObjectForm 内画像（誤配置 crop 回避を優先）／plot 軸・凡例・alt text（8c・Vision opt-in）／TeX tarball 内画像の取込。

### 1エントリ複数 PDF 添付（本文＋補助資料）— Phase 1

同じ DOI の論文に **本文 PDF** と **supplemental material（SI）等の補助 PDF** が別ファイルで存在するとき、両方を同じエントリに添付して閲覧・全文検索できるようにする。「同一 DOI ＝同一の著作」という前提に立ち、補助 PDF は**別エントリ（別文献）ではなく、本文論文に紐づく添付ファイルの一つ**として扱う（Zotero が添付を item の子として複数ぶら下げるのと同型のモデル）。

**設計方針（モデル A）:**
- 1 エントリに複数の添付をぶら下げる。補助資料に独自の cite key や BibTeX エントリは与えない（＝引用は本文論文に一本化される）。
- 単独で `\cite` したい独立 DOI を持つデータセット/コード等は本スコープ外。将来 `entry_relations` の `supplement_of`（現状スキーマ・表示のみで書込パス未実装）を別エントリとして扱う別機能に切り出す。

**スキーマ / API への影響 — なし（migration 不要）:**
- `attachments` テーブルは既に `entry_id` に**ユニーク制約を持たず**、1 エントリに複数添付を許す。`get_entry_detail` も添付を全件（`Vec<Attachment>`）返す。
- 全文索引（`fulltext`）は**既に `attachment_id` 単位**で動作し、添付ごとに独立して索引される。補助 PDF を足せば自動でその添付も全文検索対象になる（`attachments_without_fulltext` → `index_attachment` の既存経路）。
- BibTeX / cite key には無影響（添付はエクスポート対象外）。
- 既存の Tauri コマンド（`add_attachment` / `open_pdf_viewer` / `read_attachment_bytes` / `delete_attachment` / `pick_pdf_file`）をそのまま流用。**新規コマンドは追加しない**。

**Phase 1 の実装スコープ（フロントエンドのみ）:**
- **フルスクリーンリーダー（`DetailView`）の添付切替**: 現状 `attachments[0]` 固定で先頭 1 件しか開けない箇所を、**添付セレクタ**に置き換える。添付が 2 件以上あるときにツールバー／サムネイル列上部へドロップダウン（またはタブ）を表示し、選択中の `attachmentId` を state として保持して PDF ビューワー・OCR・印刷・別ウィンドウ表示すべてへ渡す。添付が 1 件のときは従来どおり選択 UI を出さない。
- **リーダー内の手動 PDF 追加導線**: `DetailView` からも PDF を追加できるようにする。ロジックはサイドパネル（`DetailPanel`）に既にある `handleAttachPdf`（`pick_pdf_file` → `add_attachment`）を共通化して流用する。サイドパネルの複数添付リスト表示・削除・個別「開く」は既存のまま維持。
- 追加した補助 PDF は既存のバックグラウンド全文索引経路に乗せ、直後から `fulltext_search` の対象にする。

**Phase 1 で「本文＋SI を両方登録して両方読める」が成立する。** DB・全文索引・サイドパネルの複数添付表示／手動追加は既に揃っているため、Phase 1 は実質フロントエンド（リーダーの添付切替とリーダー内追加導線）のみで完結し、migration も新規 API も伴わない。

**後続フェーズ（v0.8.0 スコープ外・将来）:**
- **Phase 2（添付のラベル／種別）**: primary の判定が現状「配列の 0 番目」という暗黙の順序依存になっているため、`attachments` に `kind`（`document` / `supplement` / `other`、NULL=`document` 扱い）と任意 `label`（例 "Supplementary Information"）を追加する migration を入れ、リーダー既定表示を `kind='document'` 優先にし、補助資料を「補助資料」バッジで区別する。
- **Phase 3（取込導線の「既存に添付」分岐）**: Web クリッパーが既存エントリ（DOI/arXiv 一致）にヒットしたとき、現状の「何もせず `duplicate` 返却」ではなく、**補助 PDF を既存エントリの添付として追加**する分岐（`apply_clip` に `attach_to_existing` オプション、`kind='supplement'` 既定）を足す。CLI / MCP からの `add_attachment` 到達も併せて検討し、AI エージェント経由での SI 添付を自動化できるようにする。なお**欠落した primary（PDF/TeX ソース）の補完**は別設計 — 「Web クリッパー」節の「重複クリップ時の欠落補完（設計済み・未実装）」を参照（こちらは SI の**追加**添付）。

---

## Phase 2（残り）

> ✅ v0.2.0 で消化: 複数文献の横断 Chat / LLM 結果の DB（ノート）書き込み / MCP **クライアント** → 上記「v0.2.0」セクション参照
> ✅ v0.5.0 で消化予定: ブラウザWebクリッパー → 上記「v0.5.0」セクション参照

- MCP **サーバー**実装（Obsidian 等から LumenCite を参照可能に — v0.3.0）
- 引用スタイル対応（CSL）
- ハイライトのカスタム色 / カラーピッカー UI
- **Homebrew Cask 登録**（macOS 配布チャネル拡充 — v0.1.0 リリースから 1–2 ヶ月後、DL 実績ができてから `homebrew/homebrew-cask` に PR 申請）

---

## Phase 3

- マルチデバイス同期（自前サーバー実装、方針転換の可能性あり）
- 研究室共有DB（ホストDB → 個人DBへの選択的取り込み）
- モバイルアプリ（iOS / Android）— このフェーズで KaTeX → RaTeX 移行を評価

---

## 将来ビジョン

- LLMによるデイリー論文ダイジェスト（興味に合わせた自動サマリー）
- VSCode拡張（LaTeX執筆中の引用サジェスト）
- セルフホストサーバーのOSSとしての独立リリース
