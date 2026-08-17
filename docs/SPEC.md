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
  - **全文索引の手動トリガ（v0.7.0 追加）**: 通常は添付時に自動索引するが、過去に添付済み・索引失敗のエントリ向けに任意タイミングで再索引できる。詳細パネルの各添付に**索引状態バッジ + 索引/再索引ボタン**（`index_attachment`）を、設定 → データに**「未索引の PDF を一括索引」**（`index_missing_attachments` = `attachments_without_fulltext` で未索引 PDF を洗い出し、順に索引する）を用意。**v1.0.0-p1 以降、どちらのボタンもテキストの出どころを選ばない** —— 索引ソースは単一の決定点（`ingestion::index_fulltext_for_attachment`）が **①OCR 由来の記録があれば触らない → ②LCIR → ③`pdf-extract`** の順で決める（**OCR が最優先**。下の「v1.0.0」節）。テキストレイヤーが無い（0 ページ）添付は「OCR 候補」として集計し、スキャン PDF は詳細ビューの OCR へ誘導する。**v1.0.0: 再索引ボタンは既存の索引を消さない** —— 抽出が 1 ページも本文を返さなかった場合は既存の索引をそのまま残して「テキストが取れなかったので既存の索引を残しました」と報告する（以前は無言で全削除しており、課金して起こした OCR 転写が 1 操作で失われた）。索引そのものを捨てたいときは、索引済みの添付に出る**「索引を削除」ボタン**（`unindex_attachment`・PDF 実体は消えない）。なお「OCR 候補」の集計は**索引がまだ無い添付**が対象なので、この変更で件数の意味は変わらない
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
- **自動バックアップ（CR-018: 添付本体込み）**: アプリ起動時 + 1日1回、`<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.zip` に**完全バックアップ**を作成。起動時の実行は前回成功（`settings.backup.last_run`）から 24h 未満なら**間引く**（フルバックアップは分単位でディスクと CPU を占有し、起動のたびに走ると MCP サーバーの起動や初期描画を巻き添えにするため）。手動実行（`run_backup_now`）は間引かない。dev ビルドでは再ビルドのたびに再起動するため自動バックアップは既定で無効（`LUMENCITE_STARTUP_BACKUP=1` で有効化）。作業ファイル（`.vacuum-*.db.tmp` / 書きかけの `*.zip.partial`）は途中終了時に残るため起動時に回収する。アーカイブは `db.sqlite`（`VACUUM INTO` によるクリーンコピー。highlights/chat/settings/fulltext 込み）＋ `attachments/<entry_id>/<file_name>`（添付本体）を deflate 圧縮で束ねる。14世代まで保持（旧 `.db` バックアップも世代管理・一覧の対象）。**走査中に消えたファイルは飛ばして続行し、一覧を `SKIPPED.txt` としてアーカイブに同梱する**（DB スナップショットは先頭で固まるのに走査は分単位かかるため、削除とのすれ違いは通常のレース。黙って飛ばすと「完全な成功」と「静かに欠けた成功」が同じ見た目になる）。ただし**記録できるのは親ディレクトリを列挙した後に消えたものだけ**で、`SKIPPED.txt` が無いことは完全性の証明にならない。容量不足・権限エラーは従来どおりバックアップ全体を失敗させる。また **LCIR build 後の旧 crop 回収は（同一プロセスの）バックアップ実行中は見送る**（アーカイブが「`assets` 行はあるがファイルが無い」状態で固まるのを避ける）
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
  - `add_tag` / `update_notes` / `add_to_collection`: デフォルト自動（設定で都度承認に変更可）
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
- **実行中に止められる（v1.0.0）**: OCR は**ページ 1 枚ごとに課金**され、開始前にページ数は分からない
  （実ライブラリでテキスト層が空＝ OCR 候補の PDF は 4 冊あり、最大は **608 頁**・次が 527 頁）。リーダーに `n/total` と停止ボタンを出し、
  **設定 → データにも走行中の OCR を出す**（リーダーから始めたものもチャットから始めたものも）。
  **同時に走る OCR は 1 本だけ**。停止・失敗しても**そこまでの転写ページは残る**が、
  **再開は無い**（もう一度回すと 1 ページ目から全ページ課金し直す ── アプリはそう明言する）。
  索引を置き換えて「OCR 由来」の印を付けるのは**完走したランだけ**
- **チャット（LLM）からは、索引済みの添付を OCR できない**（2026-08-18・issue #42）。
  対象添付に索引済みページが 1 ページでもあれば `get_fulltext` を案内して実行しない ──
  **ページを指定しても同じ**。判定は entry ではなく**実際に OCR される添付**で取るので、
  本体 PDF が索引済みでも**未索引のスキャン補遺は名指しすれば OCR できる**。
  **リーダーの手動ボタンはこの制限を受けない** ── テキスト層が壊れた PDF の焼き直しは正当な操作で、
  「OCR 由来」の印を添付単位で付ける根拠（ユーザーがテキスト層を信用しないと宣言した）も
  そちらにしか成立しないため
- 断るときは、同じエントリに**未索引の PDF 添付**があればその id を添える
  （添えないと、チャットからは添付ごとの id を知る手段が無く補遺に到達できない）
- ⚠ **全ページを完走した OCR の索引は、再索引では戻らない**。`fulltext.source = Ocr` の印が付いた添付は
  pdf_extract 由来の再索引も LCIR 派生も譲らない（名指しの再索引も `skipped_ocr` になる）。
  テキスト層に戻したいときは、詳細パネルの**「索引を削除」を先に押してから**再索引する。
  **ページを名指しした回はこの印を立てない** ── 印は添付単位なので、1 ページの転写で立てると
  その PDF の残りのページが二度と索引されなくなる（名指しできるのはチャット経由だけ）
- 転写は**数式を LaTeX で書く**（インラインは `$…$`・独立式は `$$…$$`・式番号は見えたまま）。
  スキャン本には転写しか本文の経路が無く、完走すると封印されてその形が正本になるため。
  **既に OCR 済みの添付には遡及しない**（作り直しは全ページ課金し直し）

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
- **TeX ソース自動取得（LCIR Phase 4 の自動化）**: arXiv クリップで **`lcir.tex_autofetch.enabled`（e-print 自動取得の同意）と `lcir.enabled` の**両方** ON のときだけ**、e-print も取得して LCIR（構造 + 生 LaTeX 数式）を自動構築する。どちらかが OFF なら取得しない（v1.0.0-p3 で同意面を分離）。重複クリップでは再取得しない（詳細パネルのボタンで明示再取得可）
- **PDF の LCIR 自動 build（v1.0.0-p2）**: **`lcir.enabled` が ON のときだけ**（v1.0.0-p3 で**既定 ON**に反転）、PDF 添付が増える 3 経路（手動添付 / arXiv 取得 / クリッパー）で**全文索引に続けて** LCIR を構築する。加えて既存ライブラリには**起動時バックフィル**が少しずつ行き渡らせる（1 ランの時間予算あり・添付境界で判定 / 手動バッチ・Vision 生成・TeX 一括取得・バックアップのいずれかが動いていれば譲る / dev ビルドは既定オフ。かつては「別インスタンス起動中は走らない」ゲートもあったが、②c C-01 で同一ライブラリの第2インスタンスは起動自体を拒否するようになったため不要になった）。**抽出器版を上げただけでは自動再構築しない**（旧版更新は明示ボタンのまま）。pdfium を読み込めない環境では PDF を飛ばして件数に数え、TeX ソースの構築は続ける
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
- **欠落検出**（duplicate 判定直後・エントリ単位）: PDF 欠落 = mime `%pdf%` の添付なし かつ クリップから PDF URL が導出できる ／ TeX 欠落 = mime `application/gzip` の添付なし かつ arxiv_id あり かつ **e-print 自動取得が有効**（`lcir.tex_autofetch.enabled` と `lcir.enabled` の両方 ON・既存の TeX 自動取得と同一ゲート）。TeX は上書き契約（LCIR Phase 4）なので「欠落」= 添付行の有無のみ（在れば対象外）。
- **duplicate 応答を拡張**: 設定 `"1"` なら即補完し `completing: ["pdf","tex"]` を返す（バッジで「補完中」を表現可能）。未設定かつ欠落ありなら `confirm_missing: ["pdf","tex"]` を返す（この時点では何もしない）。
- **拡張ポップアップ（ボタン直下の確認）**: service worker は `confirm_missing` を受けたら pending payload（entry_id/title/missing）を `chrome.storage.session` に置き、`chrome.action.setPopup({popup:"confirm.html"})` → **`chrome.action.openPopup()`**（Chrome 127+。使えない環境ではバッジ `?` を出し、次のボタンクリックがポップアップを開くフォールバック）。ポップアップの選択肢は「補完する」／「今回はしない」／「**次回以降は確認せず補完する**」。選択後は payload を消して `setPopup({popup:""})` で通常動作に戻す。
- **新エンドポイント `POST /clipper/complete`**: `{entry_id, remember?: bool}`（同一 Bearer 認証・`clipper.enabled` ゲート）。アプリ側で欠落を**再検証**してから既存の `spawn_pdf_job` / `spawn_tex_source_job` を発行し、`remember` なら `clipper.complete_missing="1"` を保存。PDF URL・arxiv_id はクリップ時の値をアプリ側で保持せず、エントリの識別子から再導出する（arXiv 導出 PDF / e-print。`citation_pdf_url` 由来の補完はこの版では対象外 = arXiv 前提で十分。ゴミ箱のエントリは `deleted_at IS NULL` で弾き、confirm 後に trash された TOCTOU も空プランにする）。
- **AddSheet 側も同じ設定に従う**: `create_entry` の CR-019 dedup で既存エントリが返るケースはアプリ内のインライン確認で尋ねる（選択肢は同じ 3 つ・WKWebView 安全に `window.confirm` 不使用）。「既存か」の判定は submit 直前に `find_duplicate_entry` を再照会して権威的に決める（fetch 時の値は probe 失敗・競合で不正確なため）。あわせて既知の quirk を修正 — 現状 AddSheet はチェック ON だと**既存エントリへ PDF を重複添付し得る**（`download_arxiv_pdf` を無条件実行）ため、返ってきた `entry.attachments` を見て「PDF/TeX 欠落時のみ実行」に変える。
- **拡張ポップアップの実装（重要）**: 確認ページ（`confirm.html`）は表示と選択の受け渡しだけを行う純粋なビューで、実際の `/clipper/complete` 呼び出しと popup の arm/disarm は **service worker が担う**。ポップアップはフォーカス喪失で即破棄されるため、ネットワークをそこに置くと補完が黙って中断し、また popup の解除漏れはツールバーボタンを無反応にする（onClicked は popup 設定中は発火しない）。どの終了経路（選択 / 空 pending / pagehide）でも SW にメッセージを送り、SW が状態を通常へ戻す。`openPopup` は Chrome 127+（`minimum_chrome_version` 宣言）で、失敗時はバッジ `?` + 次クリックで開くフォールバック。

**arXiv TeX ソースの一括取得バッチ（v0.8.0 実装済み・2026-07-19）:**

既存コーパスのバックフィル用。クリッパーの欠落補完は「再遭遇した論文を拾う」増分向けで、手持ちの arXiv エントリ全部に TeX を揃えるにはこちらが本命。

- **設定 → データ**に「arXiv の TeX ソースを一括取得」ボタン（**LCIR が有効かつ** e-print 自動取得の同意が ON のときのみ活性（v1.0.0 で `lcir.enabled` を条件に追加 ── 無いと「押せるのに何も起きないボタン」になる）。**実行中に同意を外すと添付境界で打ち切る**（v1.0.0-p3。3 秒スロットルの間に外した場合もダウンロード直前の再評価で止まる ── ②c C-03）。既存の「未構築 PDF を一括 LCIR 化」ボタンの隣・同じ busy/結果表示パターン）。
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

- 添付成功後は**バックグラウンドで全文索引**まで自動で行い、直後から PDF 全文検索の対象になる（索引失敗＝スキャン PDF 等は無視し、後追いの手動索引に委ねる）。**v1.0.0-p1 以降、テキストは単一の決定点が選ぶ**（**①OCR 記録 → ②LCIR → ③`pdf-extract`**）。**v1.0.0-p2 以降、この経路は `ingestion::ingest_new_pdf_attachment` が担い、索引に続けて LCIR も build する**（フロントが添付後に `index_attachment` を呼ぶ配線は、LCIR 由来の索引を pdf-extract で上書きし返す競合になるため削除した）。
- ダウンロード失敗（ペイウォール・ネットワーク障害・ID 不正）でも**エントリ作成は成功扱い**。フロントは警告をログに残すのみで、詳細パネルからの手動添付に誘導する。
- **TeX ソース自動取得（LCIR Phase 4 の自動化）**: **`lcir.tex_autofetch.enabled`（e-print 自動取得の同意）と `lcir.enabled` の**両方** ON のときだけ**、追加直後に fire-and-forget で `download_arxiv_source` → `build_lcir_for_attachment` も実行する（PDF チェックボックスとは独立・Web クリッパーと同じゲートと best-effort 契約。失敗はログのみ・詳細パネルのボタンで再取得可）。
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
- **v1.0.0 の看板 = LCIR 完成**: Phase **9a**/10 到達（9b は post-1.0） + `lcir.enabled` 既定 ON 化（**v1.0.0-p3 で実施済み**。外部通信を伴う e-print 自動取得は `lcir.tex_autofetch.enabled` へ分離）のタイミングで「機械可読文献基盤の完成」として 1.0 を名乗る。
- **フェーズ順序の変更と Phase 9 の分割（2026-07-23 決定）**: Phase 6 完了後の実装順は **9a → 8 → 7 → 9b/10**。Phase 9 を **9a（エクスポート第一段 = LCIR JSON + Markdown 書き出し・v0.10.0 で出荷済）**と **9b（JATS/TEI/HTML+MathML — Presentation MathML を出す Phase 7 が本質的前提）**に分割する。9a を前倒しする理由: ①中身（`LcirDocument` 派生ビュー・validation・`get_lcir_document`）は Phase 6b 時点で実質完成しており、残作業はファイル書き出しと Markdown レンダラのみ（migration 不要・依存追加なし・ヒューリスティックなし＝「誤検出より欠損」を構造的に満たす）。②フラグ OFF で main に積んできた Phase 4〜6b の成果（原文 LaTeX 数式・定理番号・cite key）を初めて目に見えるユーザー価値（Obsidian 論文ノート直行の Markdown）に変換できる。③Phase 9 のうち Phase 7 に本質依存するのは 9b だけなので、分割すれば二度手間は生じない。**v1.0.0 の「Phase 9 到達」は 9a を指し、9b は post-1.0 でもよい。**

### LCIR エクスポート（Phase 9a・**v0.10.0 で出荷済**）

エントリ単位で LCIR を **LCIR JSON**（`LcirDocument` 派生ビューそのまま・validation 通過必須）と **構造付き Markdown**（節見出し・段落・原文 LaTeX 数式・定理/証明・参考文献）へ書き出す。読み出しは MCP と同じ**エントリ→版解決（tex > pdfium 優先・`source` で明示切替）**を共有する。

- **経路は 3 つ**: 詳細パネルのボタン（保存ダイアログ・`lcir.enabled` ON のときのみ表示）／CLI `export-lcir <id_or_key> [--format json|md] [--source tex|pdf] [-o <path>]`（stdout 既定・読取専用）／既存 MCP read ツール（変更なし）。
- **Markdown の品質は由来に依存**: TeX 版は原文 LaTeX（`$..$` インライン温存・display は `$$..$$`）・定理番号・cite key まで出る。PDF 版は surface-only（数式は Unicode 線形のまま・`$$` を付けない）。出力の YAML フロントマターに `lcir_source`（抽出器名・版）を記録し、由来を常に区別する（roadmap §16）。
- **やらないこと（9b へ）**: JATS/TEI/HTML+MathML。embedding・ノードチャンク API は Phase 10。

### LCIR 図表アセット基盤（Phase 8a・**v0.10.0 で出荷済**）

Phase 8（図表機械可読化）の最小スライス。**PDF 版のみ**（`lumencite-pdfium` 0.5.0→0.6.0・TeX 抽出器は不変）。

- **図領域検出**: ページ内の埋込画像オブジェクト（トップレベル Image のみ）の bbox を近接マージして図領域とし、`figure` ノード（bbox 付き・`origin='layout_model'`・confidence 0.6）を作る。⚠ **以下は 8a 当時の限界で、v1.0.0 で解消済**（8d-2 = PR #80 `95a9f65` / 8d-8 = PR #78 `6386ef5`）── **tikz/pgf 等のベクター図は Image オブジェクトを持たないためアセット 0 件が正当**（誤検出より欠損。数学系コーパスでは体感が薄い既知の限界）。
- **ページ crop アセット**: 図領域をページレンダリング（幅 1600px・OCR と同値）から切り出した PNG として `attachments/<entry_id>/.lcir/` 配下に保存し、`assets`/`node_assets`（migration 0019）で参照する。バイナリは FS・DB は相対パス + SHA-256（ADR #3）。
- **caption 関連付け**: 同一ページの figure caption と幾何ペアリング（相互最近のみ・曖昧なら張らない）して `caption_of` 辺を張り、caption の番号（"Figure 2" → "2"）を figure ノードの `figure_number` に載せる。
- **読み出し**: MCP `get_figures`（図番号 → 画像パス・caption・本文位置を一問い合わせ）+ `LcirDocument` に `assets` が透過で載る（JSON エクスポート含む）。
- **やらないこと**（8a 当時）: ~~XObjectForm 内画像（誤配置 crop 回避を優先）~~ **8d-8 で追うようになった**（form 単位の自己校正・包含率が閾値未満なら棄却）／plot 軸・凡例・alt text（alt text は 8c・Vision opt-in で実装済）／TeX tarball 内画像の取込。表のセル構造化は 8b で実装済（次節）。

### LCIR 表セル構造化（Phase 8b・**v0.10.0 で出荷済**）

Phase 8（図表機械可読化）の表スライス。**TeX 版のみ**（`lumencite-tex` 0.4.0→0.5.0・pdfium 抽出器は不変・migration 不要 — セル構造は `document_nodes.payload_json`）。

- **セル構造化**: table float 内および裸の `tabular`/`tabular*`/`tabularx` を行 × セルの grid にし、`table` ノード（`origin='tex_source'`・confidence 0.9、列仕様が検証できない表は 0.8）を作る。payload に `column_spec`（原文 verbatim）・`n_rows`/`n_columns`・`alignments`（列型レター・検証成功時のみ）・`rows`（セル text は LaTeX 温存・`colspan`/`rowspan`・`rule_above`）・`latex_source`（原文スニペット・40k 以下）。
- **誤検出より欠損**: パースに確信が持てない表（ネスト環境・列数超過・`longtable`/`tabu`/subfloat 混在・verbatim（`lstlisting` 等）内の例示 tabular など）は構造化せず従来どおり破棄し warning に理由を残す。`\cline` 等の部分罫線は `rule_above` に数えない。ヘッダ行の推定はしない。
- **caption 関連付け**: 同一 table 環境由来という構造的事実で `caption_of` 辺（caption → table・conf 0.95・origin=tex_source）を張る。`\label` は従来どおり caption 側（caption の無い環境のみ table 側）で、`\ref{tab:..}` は `refers_to_table` として解決される。
- **読み出し**: MCP `get_tables`（caption・rows・alignments を一問い合わせ・TeX 版固定）＋ `get_document_blocks` に表の寸法（`n_rows`/`n_columns`/`column_spec`）。Markdown エクスポートは GFM パイプテーブルとして描画（セル内 `|` は数式内 `\vert `・数式外 `\|` の二層エスケープで LaTeX の意味を変えない）。
- **やらないこと**: PDF 側の表認識／`longtable`・`tabu` のセル構造化／multirow の grid 再解釈（下行の空セルは原文どおり）／単位（siunitx `S` 列）・表脚注（`\tnote`）の意味抽出／CSV アセット化。

### LCIR の再構築 UI（v0.10.0）

抽出器の版を上げても既存ライブラリは自動では作り直されないため、**旧版の LCIR に新フェーズの成果（定理・参照グラフ・記号・図・表・代替テキストの前提となる crop）が入らない**。この経路を UI から踏めるようにする。

- **設定 → データ**: 「旧版の LCIR を現行版へ再構築」ボタン（`rebuild_outdated_lcir`）。従来の「未構築の PDF を一括で LCIR 化」（`build_missing_lcir` = 未構築のみ）と並べる。対象は数百本になりうるので **1 添付ごとの進捗**（`lcir-build-progress`）を表示し、**多重起動を弾く**（2 本目は「既に実行中」）。実行中は同じ添付を触る TeX 一括取得も無効化する。
- **詳細パネルの添付行**: 添付 1 件だけを現行版で構築/再構築するボタン（`lcir.enabled` ON のときだけ表示）。`build_lcir_for_attachment` は content_key が変われば新版を作って旧版を supersede するので、「未構築」と「旧版」で操作は同じ。1 本で結果を確かめたいとき（新フェーズの動作確認）用。
- 完了後は代替テキストの生成対象件数を取り直す（図が増えるため）。

### LCIR 図の代替テキスト（Phase 8c・**v0.10.0 で出荷済**）

Phase 8（図表機械可読化）の alt text スライス。8a が作った `figure` ノードのページ crop PNG を LLM Vision に説明させ、`node_alt_texts`（migration 0020）へ保存する。**PDF 版のみ**（TeX 版に `figure` ノードは無い）。

- **AI 推定であることを型で示す**: alt text は原資料に無い**生成物**なので `origin='llm_inference'` + `confidence` + `model`（使ったモデル名）を必ず持たせる。原文 caption は**上書きしない**（別テーブル・別 provenance で併存）。全文検索（`fulltext` / `document_nodes_fts`）には**書かない** — 生成文が原文と混ざって検索結果の由来が曖昧になるため。
- **build の外・opt-in の後追いバッチ**: 生成は `generate_vision_alt_texts` だけで走る（設定 → データの明示ボタン = ライブラリ全体／**詳細パネルのボタン = そのエントリだけ**。まず 1 本で品質と費用を確かめてから広げられるようにする。ボタンのラベルに対象件数を出し、0 件なら出さない）。**短辺 200px 未満の crop（ロゴ・装飾等の小片）は対象外**で、説明する価値の薄い画像に課金しない（実蔵書では 1198 crop のうち 310 件が該当）。**build に混ぜない**（Vision は非同期・課金・非決定的なので、混ぜると content_key の冪等性と「同一 PDF → 同一 version」が壊れる）。1 図ずつ best-effort（1 図の失敗で全体を捨てない）。ただし**1 件も生成できないまま連続 3 件失敗したら打ち切る** — キー不正・レート制限・画像非対応モデルのような系統的失敗で残り全図を叩き続けないため（成功が 1 件でもあれば系統的失敗ではない。対象順序は決定的なので「必ず失敗する図」で永久に前進しなくなるのを避ける）。**実行中に同意チェックを外すと次の図の手前で停止する**（cancel 専用 UI は作らないが、止まって見える操作が黙って無効なのは不誠実なため。判定は各図のループ冒頭に加えて**課金する API 呼び出しの直前にも**再評価する ── スロットルの 1 秒と crop 読みの間に外した操作を、その 1 枚の課金前に拾う・②c C-03）。**v1.0.0: 止まるのは同意チェックだけではない** —— `lcir.enabled` を切っても同じ位置で止まり、判定は `lcir.enabled` が**先**（LCIR を切ると同意チェックが `disabled` になるので、**LCIR を切った人には同意を外す手段が無い**。②b の W1-4）。停止の説明は**どちらの面が閉じたかで書き分ける**（「LCIR が無効にされたため」/「同意チェックが外されたため」）—— LCIR を切って止めた人に「同意チェックが外された」と言うと嘘になる。**設定 → データと詳細パネルの両方**で同じ 3 通り（系統的失敗 / LCIR / 同意）を出す。
- **独立した同意面**: フラグ `lcir.vision_alt_text.enabled`（既定 **off**）は `lcir.enabled` とは別に持つ。**画像 1 枚ごとに外部 API へ送信して課金される**操作を、LCIR の実験フラグ ON だけで暗黙に許可しないため（`clipper.enabled` と同じ考え方）。両方 ON のときだけバッチが動く。UI（設定 → データ）は同意チェックボックスの下に**押す前の生成対象件数**を表示する（課金の規模を知らせてから同意させる）。プロバイダ・モデル・API キーは OCR の設定を共用する（Vision 用の設定面を増やさない）。
- **版を上げても再課金しない**: 抽出器版を上げて再構築（`rebuild_outdated_lcir`）すると `figure` ノードの row id は変わるが、crop PNG の SHA-256 が同一なら**同じ絵**なので、過去の全版から指紋一致の alt text を新版へ引き継ぐ（`carried_from_version_id` に由来を記録）。引き継ぎ後、旧版の `llm_inference` 行は刈る（crop PNG 自体も 8a の GC で trash 済）。手編集（`user_edited`）は引き継ぎ・再生成の対象外で絶対に触らない。
- **読み出し**: MCP `get_figures` に `alt_text {text, origin, confidence, model}` が付く（`LcirNode.alt_text` として `get_lcir_document` / LCIR JSON エクスポートにも透過）。
- **やらないこと**: SVG/ベクター図の構造化・plot の軸/凡例抽出・diagram のノード/辺認識・PDF 表画像の認識・ページ全体の OCR 全文化（8d 以降 or 非目標）。手編集 UI も初回は作らない（`origin` 列は最初から持つ）。Markdown エクスポートへの alt text 出力も初回は据置（MCP のみ）。

### LCIR 文脈バンドル（Phase 10a）

Phase 10（LLM・エージェント向け利用）の第一段。**1 ブロックを読んで引用するのに要るものを 1 回で返す** read 面（`get_node_context`）。migration なし・新表なし・**新しい永続推定なし**（既存 7 表からの導出のみ）。

- **なぜ要るか**: PDF 版の定理ノードは主張の**先頭 1 レイアウトブロック**しか持たない（実測 平均 168 字。TeX 版は環境本文が丸ごと 1 ノードで 975 字）。続きの式や "where …" は theorem の子ではなく **page 直下の兄弟**に落ち、theorem の 33% / proof の 53% で**ページをまたぐ**。`get_document_blocks` でノードを 1 つ読んでも定理を読んだことにならない。
- **入口はノード id だけ**（`entry_id` も `source` も取らない）。ノードがどの版の話かを既に決めているので、エントリ起点の read 優先度（tex > pdfium）で選び直すと呼び出し側が握っている id が引けない版に化ける。superseded 版のノードも読める。
- **返すもの**: `focus` / `section_path` / `before` / `continuation`（読み順で次の構造境界の手前まで＝ページ境界で切れない）/ `continuation_stopped_at` / `proofs` / `proves` / `premises` / `equations` / `figures` / `citations` / `references` / `notes`。全要素に `origin` + `confidence`、PDF 版は `page` + `bbox`（既存ツールと同じ 4 要素配列）。**どこで・なぜ続きを止めたかを必ず返す** — 「主張が終わった」「フロートのキャプションに割り込まれた」「上限で切った」は意味が違うので、黙って空にしない。
- **前提定義は導出経路を明示する**: 辺（`refers_to_theorem` → `definition`）だけでは実測 1.4% しか埋まらないので、`via` で `reference` / `occurrence`（`symbol_occurrences` の記録）/ `symbol`（記号の表層が本文に `$X$` で現れ、定義が読み順で前にある・読み側の照合で保存はしない）を**区別したまま**返す。後 2 者は TeX 版のみ。
- **図表参照は caption_of を解決して返す**: `{node（辺の指し先）, figure（領域・crop・alt text の持ち主）, caption（原文）}`。実測で caption の 3/4 は実体に到達できないので `figure` の欠落は常態で、`notes` に出す。
- **やらないこと**: 2 ホップの畳み込み（証明の参照まで含めるとバンドル長が予測不能になる。`proofs` の node_id で呼び直す）／embedding・ベクトル検索・文献横断グラフ（10c・post-1.0）。チャットへの露出は Phase 10b で実装済み（下記）。

### LCIR のチャット露出と provenance 付き回答（Phase 10b）

Phase 10 の完成形。**アプリ内チャットが論文を構造単位で読み、根拠を示せる**ようにする。
migration なし・新表なし・新しい永続推定なし。

- **なぜ要るか**: LCIR の read ツールは MCP サーバーにしか出ておらず、アプリ内チャットからは
  `get_fulltext` すら呼べなかった。索引済みの PDF を読む手段が無いので、チャットは
  `fulltext_search` が空振りすると `ocr_pdf` を提案し、**既にあるテキスト層を Vision 出力で
  上書きしながら課金する**（issue #42）。
- **単一ソース化**: 文献本文の read ツール 9 種（`get_fulltext` + LCIR 8 種）の定義と実行を
  `llm::tools::document` に置き、MCP サーバーはそこへ委譲する。`search` / `mutate` が既に
  持っていた関係を広げただけで、`tools/list` の内容・並び順・各ツールの入出力は変えない。
- **スコープ**: チャットには対象エントリを絞る機能（CR-024）があるが MCP には無い。
  `entry_id` が確定したすべての経路で検査し、横断検索は絞ったことを `scope_filtered` で
  応答に出す（黙って絞ると LLM が「ライブラリに無い」と答える）。
- **一覧に出す条件は「読める版が実在するか」**。`lcir.enabled` を ON にしただけでは何も
  構築されないので、フラグだけで判定すると `has_lcir:false` しか返さないツールの定義で
  コンテキストを食う。
- **provenance**: ツール契約をシステムプロンプトに必ず足し、原文由来（`tex_source` /
  `pdf_text_layer`）と LumenCite の推定（`layout_model` / `llm_inference`）を回答中で
  区別させる。データ側でも `get_document_blocks` の各ブロックに `origin` / `confidence` を
  載せた（従来は `get_node_context` にしか無く、契約だけあってデータが無かった）。
- **根拠 → PDF 領域**: ツール結果カードに根拠チップを出し、押すと PDF ビューアがその
  ページを開いて領域を一時強調する。座標は既存ハイライトと同一系なので変換は不要。
- **やらないこと**: TeX 版ノードからの領域ジャンプ（TeX に座標が無い。tex → pdf の位置解決は
  post-1.0）／本体詳細画面へのインライン埋め込み（別ウィンドウで開く。チャットを閉じない）／
  embedding・ベクトル検索（10c・post-1.0）。

### 1エントリ複数 PDF 添付（本文＋補助資料）— Phase 1

同じ DOI の論文に **本文 PDF** と **supplemental material（SI）等の補助 PDF** が別ファイルで存在するとき、両方を同じエントリに添付して閲覧・全文検索できるようにする。「同一 DOI ＝同一の著作」という前提に立ち、補助 PDF は**別エントリ（別文献）ではなく、本文論文に紐づく添付ファイルの一つ**として扱う（Zotero が添付を item の子として複数ぶら下げるのと同型のモデル）。

**設計方針（モデル A）:**
- 1 エントリに複数の添付をぶら下げる。補助資料に独自の cite key や BibTeX エントリは与えない（＝引用は本文論文に一本化される）。
- 単独で `\cite` したい独立 DOI を持つデータセット/コード等は本スコープ外。将来 `entry_relations` の `supplement_of`（現状スキーマ・表示のみで書込パス未実装）を別エントリとして扱う別機能に切り出す。

**スキーマ / API への影響 — なし（migration 不要）:**
- `attachments` テーブルは既に `entry_id` に**ユニーク制約を持たず**、1 エントリに複数添付を許す。`get_entry_detail` も添付を全件（`Vec<Attachment>`）返す。
- 全文索引（`fulltext`）は**既に `attachment_id` 単位**で動作し、添付ごとに独立して索引される。補助 PDF を足せば自動でその添付も全文検索対象になる（添付経路の `ingest_new_pdf_attachment`・後追いは `attachments_without_fulltext` → `index_missing_attachments`）。
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

## v1.0.0

看板は **LCIR の完成**（Phase 9a / 10 到達 + `lcir.enabled` の既定 ON）。実装順序・債務・実測値の正本は
`docs/LCIR_REMAINING_PHASES.md`。ここには**ユーザーから見える仕様**のうち、上の版の節に
書き足しただけでは所在が分からなくなるもの（p1 と p4）を置く。
p2（自動 build）は「Web クリッパー」節と「arXiv 追加時の PDF 一括ダウンロード」節に、
p3（既定 ON と同意分離）は同じ 2 節と「リリース方針」節に書いてある。

### 長時間バッチの状態はアプリが持つ（v1.0.0・debt-32 の解消）

長時間バッチ（LCIR の build / rebuild・全文索引の再導出・容量回収・図の説明生成・TeX 取得・**OCR**）の
**実行中・進捗・直近の結果はアプリ側が正本**で、設定ウィンドウは表示するだけ。したがって:

- 設定を閉じて開き直しても、**走行中のジョブが進捗つきで見える**。
- 閉じている間に終わったジョブの結果（**失敗件数を含む**）が、開いたときに見える。
- 設定の外から始めた**ユーザー操作の**ジョブ（詳細パネルの図の説明生成、リーダーやチャットから始めた OCR）も
  同じ場所に映る。
- ⚠ **「直近の結果」はジョブ種別ごとに 1 枠**で、次のランが上書きする。**アプリを落とすと消える**
  （プロセス内に持つため・debt-47）。
- ⚠ **この仕組みに載っていないものが 4 つある**: 未索引 PDF の一括索引（debt-48）/ バックアップ・復元
  （debt-45）/ **p2 の自動 build**（添付時の build と起動時バックフィル・`RunningMark` を取らない・debt-35）/
  **詳細パネルの 1 件 build**。**自動 build は v1.0.0 で全ユーザーの毎起動に走る経路**なので、
  「何も表示されていない ＝ 何も走っていない」ではない。

### v1.0.0 の LCIR が **しない**こと（非目標）

「LCIR の完成」という看板が誇大にならないよう、**v1.0.0 に入らないもの**をここに置く。
理由つきの記録は `docs/LCIR_REMAINING_PHASES.md` §0（post-1.0 の分類）と各 Phase 節、
利用者視点の説明は `docs/LCIR_v1_personas.md`。

- **数式の意味表現**: AST・Content MathML・OpenMath は持たない。TeX 由来は生 LaTeX 文字列、
  PDF 由来は表層文字列が上限で、**意味的に同じ式の検索も同値判定もできない**（Phase 7）。
- **PDF だけの文献の表と記号**: 表のセル構造化と記号定義の抽出は **TeX ソースがある文献に限られる**（8d-6 / debt-5）。
- **スキャン PDF の構造化**: OCR したテキストは全文検索には入るが **LCIR には統合されない**
  （定理ノードも参照グラフも付かない）。
- **embedding とベクトル検索**: 横断検索は**語彙の一致まで**。言い換えや概念での検索はできない（10c）。
- **図の完全な回収**: **caption を持たないベクター図は取り逃す設計**（採用の条件が caption とのペアで、
  罫線・白抜き・クリップ由来の誤検出は別に専用ガードで落としている）。寸法フィルタの都合で図が 0 件になる版も既知のまま残り（debt-26）、
  回転ページの図領域も skip する（debt-9）。
- **図の中身の構造化**: 種別分類（plot / diagram / photo）も、plot の軸と系列も、diagram のノードと辺も
  抽出しない（8d-4 / 8d-5）。SVG の取り込みもしない（8d-3）。図の説明は **caption と opt-in の
  Vision alt text（自然文）が上限**。
- **標準形式エクスポート**: JATS・TEI・HTML+MathML・Web Annotation は出せない（9b）。
  出せるのは **LCIR JSON と構造付き Markdown** の 2 つで、形式ごとに落ちる情報は欠落警告として明示する。
- **抽出器改良の自動反映**: 既存文献の再構築は**明示ボタンに限られる**（版 bump は再構築を誘発しない設計）。

### 全文検索の索引を LCIR 由来にする（v1.0.0-p1）

**`fulltext` は LCIR の派生索引になった。** pdfium が LCIR のために読んだページ本文が、
旧抽出器（`pdf-extract`）の出力に代わって索引の元になる。

- **決定点は 1 つ**（`ingestion::index_fulltext_for_attachment`）。pdf-extract を使う本番経路
  5 つ（添付 3 経路 + `index_attachment` + `index_missing_attachments`）はすべてここを通り、
  ①索引が **OCR 由来**と記録されていれば触らない → ②`lcir.enabled` かつ LCIR の page に本文が
  あれば **LCIR から派生** → ③どちらでもなければ従来どおり `pdf-extract`、の順に落ちる。
- **出どころの記録**は settings の添付単位キー `fulltext.source.<attachment_id>`（`lcir` / `ocr`）。
  FTS5 仮想表に列を足せないため（**migration は v1.0.0 全体で 0 件**）。記録が無い＝
  pdf-extract 由来か未索引。
- **行き渡らせ方は 3 つ**（⚠ 「バッチは 2 つ」ではない）:
  ①起動時 1 回の自動処理（**索引が無い添付を埋めるだけ**・記録の無い既存索引には触らない）
  ②設定 → データの「全文索引を LCIR から張り直す」ボタン（`rederive_fulltext_from_lcir`・
  こちらは置き換えまでやる）
  ③**その添付の LCIR を build すること**（`regenerate_page_fts_from_lcir` を無条件に呼ぶ）。
  ③は明示バッチではないが**記録の無い既存索引を置き換える**ので、実質いちばん広く行き渡る
  ── 自動 build と起動時バックフィルがここを踏むため、**押していないのに置き換わる**のはこの経路。
- **OCR 転写は不可侵**。ただし**この版より前に OCR したテキストには記録が無い**ので、
  上記の手動ボタン・詳細パネルの再索引・**その論文の LCIR を build すること**の 3 経路では
  置き換わりうる（自動 build も起動時バックフィルもこの 3 つ目に当たる）。既知の債務 debt-37。
- **実測**（138 本の実ライブラリ）: 索引を持つ添付 133 → 135・**制御文字を含む行 729 → 0**・
  ページの消失 0。頻出語 12 語はすべて純増（新たに引けるページ 512 対 引けなくなった 216 ──
  新旧は上位集合の関係ではない）。

### LCIR の容量表示と superseded 版の回収（v1.0.0-p4）

再構築を重ねると旧版（`superseded`）が DB に積み上がる。実ライブラリでは
**全ノードの 83%** が旧版のものだった。

- **設定 → データ**に「使用中 / 再利用可」のバイト数を出し（`lcir_storage_stats`）、
  「旧版を回収」ボタン（`run_lcir_gc`）を置く。
- **非可逆**。押す前に対象の版数と crop 枚数（＋その crop の実バイト数）を出して確認を取る。
  **「何 MB 空くか」だけは予告しない** ── DB 側の解放量は按分推定しか作れず実測で 6.5% 上振れした
  ので、`freelist` の実測差分として**実行後に**報告する。確認ボックスは**開く直前に見積りを
  取り直す**（マウント時の古い数字で非可逆な同意を取らないため）。
- **確認ボックスの「削除する」は、他の LCIR バッチが走っている間は押せない**（v1.0.0・②b の PR-3。
  代替テキスト側の「実行する」と同型）。実行時に削除対象そのものも取り直すので
  **保護すべき版を消すことはない**（外した数は `versions_skipped` で報告する）。
  ⚠ **それでも「表示した版数より多く消える」は残る**（debt-58）── 代替テキスト側は
  **押した瞬間に**件数を取り直して食い違ったら聞き直すが、GC にはその再取得が無い。
  確認ボックスを開いたまま同じ画面の一括構築 / 一括再構築を回せる（build ボタンは確認中かを見ない）
  ので、それが終わった後に**古い数字のまま**「削除する」を押せる。
- **守るもの**: 生成済み / 手書きの図の説明（`node_alt_texts`）を持つ版は消さない。
  説明を新版へ carry した版は、**行だけ残して中身を捨てる**（carry の記録を切らないため）。
- **LCIR が無効でも実行できる**（切っている人にとって旧版は純粋な無駄なので）。
- **ファイルサイズは縮まない**。free page になるだけで、次のバックアップと次の再構築が使う。
- **実測**: 実 DB のコピーで **125 秒 → 索引を張って 48 秒**（`symbols(scope_node_id)`。
  ⚠ **出荷コードは GC の入口で毎回この索引を保証する**ので、ユーザーの手元で起こるのは
  48 秒側だけ。125 秒は索引を入れる前のプローブ値）。実 DB での本番実行は
  **145 版 / 493.9MB を回収**（所要は記録していない ── 上の 2 つはコピーでの別測定）。
  1 トランザクションで消すノード数は 50,000 で切る（最大の版は 272,583 ノードで、
  索引ありでも 1 tx なら 5.7〜7.95 秒かかる）。

### 同一ライブラリは 1 プロセス（ゲート②c C-01 / C-02）

**同じ app data dir を使う LumenCite は同時に 1 つしか起動できない。** 2 つ目を起動すると
「既に起動しています」のダイアログ（日英併記・理由つき）を出して終了する（exit 0）。

- **理由**: 課金バッチ（OCR / 図の説明生成）・LCIR build / GC・バックアップの排他と実行中表示は
  すべてプロセス内にしか無い。第2 GUI を許すと、同じ対象への課金を二重に走らせ、
  build / GC が同じ DB と crop ディレクトリを取り合える（②c C-01・high）。
- **判定は既存の GUI 生存ロック**（`lumencite.gui.lock` の flock・CR-011 と共用）。復元後の
  relaunch や dev の再ビルドでは旧プロセスがまだロックを握っていることがあるため、
  **約 3 秒の再試行**（200ms × 15 回）で吸収してから「別インスタンスあり」と確定する。
- **ロック機構が使えない環境（flock 非対応 FS 等）は従来どおり起動を続ける**。
  「判定不能」を「別インスタンスあり」に丸めると、そういう環境では永久に起動できなくなる。
  この環境では第2インスタンスも起動しうる（残る限界・許容）。
- **復元の適用はロック判定の後**（②c C-02）。以前は staged 復元の適用がロックより先だったので、
  稼働中の第1 GUI が pool を握ったまま live DB と `attachments/` を第2 GUI 由来の relaunch が
  `pre-restore/` へ差し替えられた。
- **開発時の逃げ道**: 配布版と `pnpm tauri dev` の併用は同一ライブラリでは塞がれる（それが目的）。
  別ライブラリでよければ `--config` で identifier を差し替えるとロックごと分離される。

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
