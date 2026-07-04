# LumenCite

[![Sponsor](https://img.shields.io/github/sponsors/marmot1123?logo=github&label=Sponsor&color=ea4aaa)](https://github.com/sponsors/marmot1123)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A desktop reference management application for researchers, built with **Tauri 2 + React + TypeScript**.

![Library view](docs/screenshots/library_view.png)

![Detail view](docs/screenshots/detail_view.png)

## Features

- 📚 **Entry management** — 論文・書籍・会議録・Webページ等の CRUD、タグ・コレクション（ネスト対応）、お気に入り、ゴミ箱
- 🔍 **Auto metadata** — DOI / arXiv / ISBN から CrossRef / arXiv API / Open Library 経由でメタデータを取得
- 📄 **PDF viewer** — pdf.js ベースの 3 ペイン詳細ビュー、3 色ハイライト、テキスト選択、ページサムネイル、印刷 (⌘P)
- ✨ **LLM summarization** — OpenAI / Anthropic 対応、API キーは OS キーチェーン保管、ストリーミング表示、カスタムシステムプロンプト
- 📐 **KaTeX** — 抄録 / ノートで `$…$` / `$$…$$` 数式レンダリング
- 🔗 **BibTeX workflow** — インポート / エクスポート + 指定パスへの自動同期 (VSCode LaTeX Workshop 連携前提)
- ⌘K **Command palette** — エントリ横断検索とグローバルアクションを一発起動
- 🌗 **i18n + theme** — 日本語 / 英語 UI、ライト / ダーク / システム追従、4 アクセントカラー
- 💾 **Backup & export** — SQLite を `VACUUM INTO` で日次自動バックアップ (14 世代保持) + JSON / BibTeX / Markdown 手動エクスポート
- ⬆️ **Auto-updater** — Tauri Updater プラグインで署名検証付きアップデート

## Download & install

最新版は [GitHub Releases](https://github.com/marmot1123/LumenCite/releases/latest) から入手できます（macOS: `.dmg` / Windows: `.msi`・`.exe` / Linux: `.AppImage`・`.deb`・`.rpm`）。macOS は署名＋notarize 済みで、アプリ内 **設定 → アップデート**から自動更新できます。

### macOS: Homebrew

macOS では [Homebrew](https://brew.sh/) 経由でもインストールできます（自前 tap [marmot1123/homebrew-lumencite](https://github.com/marmot1123/homebrew-lumencite) から universal `.dmg` を配布）。

```bash
brew tap marmot1123/lumencite
brew trust marmot1123/lumencite   # Homebrew 6.0+ ではサードパーティ tap に必須
brew install --cask lumencite
```

アップデートは `brew upgrade --cask lumencite`、またはアプリ内の自動更新（Tauri Updater）のどちらでも行えます。

> ⚠️ **v0.1.0 をお使いの方へ:** v0.1.0 は updater 鍵の設定漏れにより**自動更新が動作しません**（「アップデートを確認」で `Invalid symbol 95, offset 7.` というエラーになります）。お手数ですが、上記 Releases から**最新版を一度だけ手動でダウンロードして入れ直して**ください。以降は自動更新が有効になります。v0.2.0 以降のバージョンはこの問題の影響を受けません。

## Requirements

- [Node.js](https://nodejs.org/) 18+ と [pnpm](https://pnpm.io/) 9+
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)
- Tauri prerequisites: https://tauri.app/start/prerequisites/

## Development

```bash
pnpm install
pnpm tauri dev
```

Vite (port 1420) と Rust backend が連動し、ホットリロードで開発できます。

## Build

```bash
pnpm tauri build
```

`src-tauri/target/release/bundle/` 配下に各 OS 用のインストーラ (.dmg / .msi / .deb / .AppImage) が出力されます。リリース署名手順は [docs/RELEASE.md](docs/RELEASE.md) を参照してください。

## Tests

```bash
# Rust
cd src-tauri && cargo test

# Frontend (型 + ビルド)
pnpm build

# ブラウザ拡張
pnpm --filter lumencite-clipper test
```

## Browser extension (Web Clipper)

LumenCite には Chrome 拡張（Manifest V3）の **Web クリッパー**が付属します。論文ページを開いてツールバーボタンをクリックすると、起動中の LumenCite にエントリを作成します（DOI / arXiv / ISBN を自動抽出し、arXiv は PDF も自動添付）。拡張と LumenCite は**同じ PC 内の localhost** でのみ通信し、外部サーバーは経由しません。

> ℹ️ Chrome ウェブストアでの公開は準備中です。現在は下記の手順で**手動インストール**（load unpacked）します。Chromium 系ブラウザ（Chrome / Edge / Brave など）で利用できます。

### インストール（ユーザー向け）

1. [GitHub Releases](https://github.com/marmot1123/LumenCite/releases/latest) から `lumencite-clipper-<version>.zip` をダウンロードし、**任意の場所に解凍**します（解凍後のフォルダは削除・移動しないでください。拡張はそのフォルダを直接読み込みます）。
2. Chrome で `chrome://extensions` を開き、右上の **デベロッパーモード**を ON にします。
3. **「パッケージ化されていない拡張機能を読み込む」**をクリックし、手順 1 で解凍したフォルダ（`manifest.json` を含むフォルダ）を選択します。
4. LumenCite を起動し、**設定 → Chat → Web クリッパー**を有効化して、表示される**接続コードをコピー**します。
5. 拡張のアイコンを右クリック →「オプション」（または `chrome://extensions` の拡張の「詳細」→「拡張機能のオプション」）でオプションページを開き、接続コードを貼り付けて**保存**します。

これで論文ページのツールバーボタンからクリップできます。

> 🔑 接続コードには秘密トークンが含まれます。LumenCite 側でトークンを再生成した場合や、MCP サーバーのポートを変更した場合はペアリングが切れるため、新しい接続コードで**手順 4〜5 をやり直して**ください。

### ソースからビルド（開発者向け）

```bash
pnpm --filter lumencite-clipper build   # extension/dist を生成
```

`chrome://extensions` →「パッケージ化されていない拡張機能を読み込む」で `extension/dist` を選択すれば、上記のインストール手順 4〜5 に進めます。拡張のバージョン（`extension/manifest.json`）はアプリと独立採番です。

## CLI（コマンドライン）

LumenCite は GUI を起動せず、ターミナルからライブラリを**読取専用**で照会できる CLI を内蔵します（本体バイナリの `argv` 分岐で動作。新しいバイナリは増やしていません）。主な用途は **AI エージェント × LaTeX 執筆**（`\cite` キー → `refs.bib` 生成）とシェルスクリプト連携です。

出力は既定で **JSON**（`jq` 連携向け）、`--human` で人間可読テキストに切り替わります。SQLite を `PRAGMA query_only = ON` の読取専用接続で開くため、GUI アプリ起動中でも安全に共存し、停止中でも動作します。

```bash
# メタデータ検索（フィルタ: --type / --year-min / --year-max / --starred / --has-attachment / --limit）
lumencite search "quantum walk" --year-min 2018 --limit 10

# 単一エントリ（数値 id でも citation key でも可）
lumencite get smith2020a
lumencite get smith2020a --human

# \cite キー群から refs.bib を生成（キーは化けずに \cite と一致。未解決キーは stderr に警告）
lumencite bib smith2020a jones2021 > refs.bib

# フィルタ条件で BibTeX 一括エクスポート
lumencite export --type article --year-min 2020 > articles.bib

# タグ / コレクション一覧・PDF 全文検索
lumencite tags
lumencite collections
lumencite fulltext "topological"
```

DB は Tauri の `app_data_dir`（macOS: `~/Library/Application Support/com.lumencite.app/lumencite.db`）を自動解決します。環境変数 `LUMENCITE_DB_PATH` で上書き可能です。ライブラリが未作成の場合はアプリを一度起動してください。

> ℹ️ v0.7.0 の CLI は読取専用です。書き込み系コマンドと、サーバ起動中の HTTP プロキシ経由（ハイブリッド C）は、書き込みガードを厳格化した上で次版で追加予定です。

## Documentation

- [docs/SPEC.md](docs/SPEC.md) — 機能仕様と v0.1.0 / Phase 2+ のロードマップ
- [docs/DATA_MODEL.md](docs/DATA_MODEL.md) — SQLite スキーマと設計判断
- [docs/API_SPEC.md](docs/API_SPEC.md) — Tauri コマンド一覧
- [docs/RELEASE.md](docs/RELEASE.md) — コード署名 / notarization / リリース手順

## Sponsor

LumenCite はオープンソースの個人プロジェクトです。継続的な開発を応援していただける方はぜひ [**GitHub Sponsors**](https://github.com/sponsors/marmot1123) で支援をお願いいたします。

## License

[MIT](LICENSE) © 2026 Motoki Seki and LumenCite contributors.
