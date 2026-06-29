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
- **自動バックアップ**: アプリ起動時 + 1日1回、SQLite DB を `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.db` にコピー。`VACUUM INTO` を使用、14世代まで保持
- **手動エクスポート**: 全データ JSON / BibTeX（既存） / Markdown（ノート＋要約）
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

**後続フェーズ（未実装）:**
- Phase 2: write 系の公開（`mcp_server.write_enabled` の一括ゲート・デフォルト false。承認 UI が無いためサーバー側でポリシー enforce。破壊系 hard delete は非公開）＋監査ログ
- Phase 3: stdio しか使えない Claude Desktop 向けの `lumencite-mcp` shim（stdio↔localhost HTTP プロキシ）同梱＋設定スニペット UI
- Phase 4（任意）: MCP *resources*（`lumencite://entry/{id}` で論文を @メンション）

### その他の v0.3.0 候補（要検討）

- 古典的 RAG（埋め込みベクトル検索）— v0.2.0 の FTS5 agentic 運用結果を見て採否判断
- ローカル LLM プロバイダ（Ollama / LM Studio）

---

## Phase 2（残り）

> ✅ v0.2.0 で消化: 複数文献の横断 Chat / LLM 結果の DB（ノート）書き込み / MCP **クライアント** → 上記「v0.2.0」セクション参照

- MCP **サーバー**実装（Obsidian 等から LumenCite を参照可能に — v0.3.0）
- 引用スタイル対応（CSL）
- ブラウザWebクリッパー
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
