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

**Phase 2（実装済み — write 公開＋ゲート）:**
- write 系を `mcp_server.write_enabled`（一括ゲート・**デフォルト false**）が有効なときだけ公開: `add_tag` / `update_notes` / `add_to_collection` / `create_entry` / `update_entry`。**破壊系 `delete_entry` は常に非公開**（許可リスト外で `tools/call` でも到達不可）。承認 UI が無いためサーバー側でこのゲートを enforce する（設定はリクエスト毎に評価し、トグル変更は再起動なしで即反映）。
- write 成功時は **監査ログ**（`mcp_audit_log` 表 / migration 0010）に記録し、`.bib` 自動同期キック（`sync_tx`）＋ 一覧へのライブ反映（`entries-changed` イベント → `loadEntries`）を発火する。
- 設定 UI に write 許可トグル（警告付き）、Tauri コマンド `set_mcp_server_write_enabled` / `get_mcp_audit_log`。`get_mcp_server_status` は `write_enabled` を返す。

**Phase 3（実装済み — Claude Desktop 向け stdio shim）:**
- stdio しか使えない Claude Desktop 向けに、本体バイナリ自身を `--mcp-stdio` 付きで起動すると「stdio↔localhost HTTP プロキシ」として振る舞う（`main.rs` が GUI 起動前に検出 → `mcp_shim::run_stdio_proxy`）。別 sidecar バイナリにしないことで追加の署名・notarize 対象を増やさない。
- 接続先は Claude Desktop 設定の `env`（`LUMENCITE_MCP_URL` / `LUMENCITE_MCP_TOKEN`）で受け取る。`get_mcp_server_config_snippet("claude_desktop")` が `command`=現在の実行ファイル絶対パス・`args`=`["--mcp-stdio"]`・`env` 込みの `mcpServers` JSON を生成し、設定 UI に Claude Code / Claude Desktop 双方のスニペットを表示する。
- **堅牢化（レビュー対応）**: shim は URL/トークン未設定を起動時にエラー化、id 付きリクエストへの空ボディはハング防止に JSON-RPC エラー化、非 UTF-8 の stdin 行はセッションを落とさず読み飛ばす。`command` は `current_exe()` の絶対パスを埋め込むため、**/Applications へ設置してからスニペットをコピー**する旨を UI に警告表示（移動・再ビルド・App Translocation でパスが無効化するため）。**検証は macOS の Claude Desktop で実施**。Windows の GUI-subsystem 子プロセスでの stdio 継承は未検証（将来 Windows 対応時に要スモークテスト）。

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
- **重複**: 既存エントリ（DOI/arXiv/ISBN 一致）があれば作成せず duplicate 応答 → 拡張はバッジで通知
- **対象ブラウザ**: Chrome（Manifest V3）。WebExtension 標準準拠で実装し Firefox は将来の小差分。配布は v1 では load-unpacked + GitHub Releases の zip（Chrome Web Store は後日）

**アーキテクチャ:**
- 通信路は既存の localhost HTTP サーバー（MCP サーバーと同一プロセス・同一ポート・同一 Bearer トークン）にパスベースルーティングを追加し `/clipper` を新設。JSON-RPC（`/mcp`）は無変更で後方互換
- **同意モデル**: 新設定 `clipper.enabled`（デフォルト off）。`mcp_server.write_enabled` とは独立のゲート（クリッパーは拡張のインストール＋接続コード貼り付けという別の同意面を持つため）。サーバープロセスは「MCP 有効 OR クリッパー有効」で起動
- **ペアリング**: 設定画面の「接続コード」（`lc1.` + base64url の `{v, port, token}`）をコピーして拡張のオプションページに貼り付け。トークン再生成でペアリングは無効化される（設定 UI に注記）
- 拡張は常駐 content script を持たない: アクションクリック時のみ `chrome.scripting.executeScript` で抽出関数を注入（権限は `activeTab` / `scripting` / `storage` / `notifications` と `http://127.0.0.1/*` のみ）
- リポジトリは monorepo: `extension/` パッケージ + pnpm workspace 化

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
- 対象は arXiv タブのみ（DOI / ISBN は出版社側の PDF 配布が不定のため対象外）。詳細は `API_SPEC.md` の `download_arxiv_pdf` を参照。

### 将来検討事項（lumencite-bib Skill の駆動方式）

CLI（読取＋書込）が揃ったので、LaTeX 執筆支援の `lumencite-bib` Agent Skill をどう仕上げるかが未決。現状は **MCP 駆動の個人 Skill**（`~/.claude/skills/lumencite-bib/`・リポジトリ非同梱）で dogfood 中。次のセッションで以下を詰める:

- **A. 駆動方式**: (1) **MCP 駆動のまま** — アプリ起動＋MCP サーバー有効が前提。リアルタイム UI 反映と書込ゲートの恩恵。(2) **CLI 駆動へ寄せる** — アプリ停止中でも動き、MCP サーバー設定が不要。読取は `query_only` プールで安全、書込は CLI 側の**ハイブリッド C** が「アプリ起動中は自動で HTTP 委譲（UI 反映）／停止中は直接 DB」を内包するため、CLI に寄せると「常に動く＋起動中は UI 反映」を両取りできる可能性が高い。(3) 併用。
- **B. 配布**: 現状は個人 Skill のみ。リポジトリ同梱で他ユーザーへ配布するか。同梱するなら CLI 駆動の方が前提が軽い（利用者に MCP 有効化を課さない）が、CLI の PATH 配置（Homebrew `binary` 等の配布導線）とセットになる。
- **C. 検証**: 決めた駆動方式で `refs.bib` 生成 / `\cite` 解決 / 欠落追加を E2E。

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
