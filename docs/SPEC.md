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

### データ保全 / 配布
- **自動バックアップ**: アプリ起動時 + 1日1回、SQLite DB を `<app_data_dir>/backups/lumencite-YYYYMMDD-HHmmss.db` にコピー。`VACUUM INTO` を使用、14世代まで保持
- **手動エクスポート**: 全データ JSON / BibTeX（既存） / Markdown（ノート＋要約）
- **Tauri auto-updater**: `tauri-plugin-updater` 経由。署名鍵で検証、GitHub Releases の `latest.json` を参照
- **コード署名**:
  - macOS: Developer ID Application + notarization（v0.1.0 配布前に必須）
  - Windows: コード署名証明書（配布対象に含めるなら必須、未対応なら v0.2.0 送り）

---

## Phase 2

- 複数文献の横断質問（LLM Chat タブ — 詳細ビューに第5タブ or 独立画面）
- LLM結果のMarkdownノート保存
- MCPサーバー実装（Obsidian等との双方向連携）
- 引用スタイル対応（CSL）
- ブラウザWebクリッパー
- ハイライトのカスタム色 / カラーピッカー UI

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
