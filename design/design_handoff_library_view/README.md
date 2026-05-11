# Handoff: LumenCite — 文献ライブラリ画面

## Overview

LumenCite（Tauri 2 + React + TypeScript の文献管理デスクトップアプリ）の **メイン画面 = 文献ライブラリビュー** のデザイン仕様です。Zoteroを参考にしつつ、よりモダンで情報密度の高い3ペインレイアウトを設計しました。

将来的にPDF表紙ビュー・タイムライン・引用グラフなど複数のビューを切替えられるよう、ビュータブ機構も含んでいます。

## About the Design Files

`prototype/` 配下のHTML/JSXファイルは **デザインリファレンス（HTMLで作ったプロトタイプ）** であり、そのままプロダクションコードとしてコピーすることを意図していません。タスクは、これらのデザインを **既存の `LumenCite` プロジェクト（React 18 + TypeScript + Vite + Tauri 2）の流儀に沿って再実装する** ことです。

具体的には：

- React 18 + TypeScript で実装する（プロトタイプはJSXで書かれていますが、本体はTSXに書き換え）
- 状態は `useState` / `useReducer` で持ち、必要に応じて `Zustand` などの導入を検討
- データ取得は `invoke()` 経由で Tauri コマンドを呼ぶ（`docs/API_SPEC.md` 参照）
- スタイルは現状ベタ書き（`style={...}`）ですが、本実装では CSS Modules / Vanilla Extract / Tailwind など、プロジェクトで採用する手法に合わせて書き換えてOK
- フォント（IBM Plex Sans / Mono）はGoogle Fontsで読み込む or バンドルする

プロトタイプはブラウザで開けば動作するので、実装中の参照用に開いておくと便利です。

## Fidelity

**High-fidelity (hifi)** — 配色・タイポグラフィ・余白・インタラクションすべて確定済みです。色値はOKLCH、サイズ・余白はpxで明示。これに従ってピクセル一致で再現してください。

## 画面構成（3ペイン）

```
┌─────────────────────────────────────────────────────────────────┐
│ titlebar (macOSウィンドウ) — 本実装ではOSのネイティブ chrome     │
├──────────┬───────────────────────────────────────────┬──────────┤
│          │ Toolbar (タイトル + 件数 + アクション)     │          │
│ Sidebar  ├───────────────────────────────────────────┤ Detail   │
│          │ Sub-toolbar (検索 + フィルタ + 並替)       │ Panel    │
│ 232px    ├───────────────────────────────────────────┤          │
│          │ ViewTabs (表/カバー/タイムライン/グラフ)   │ 320px    │
│          ├───────────────────────────────────────────┤          │
│          │                                           │          │
│          │ Content (Table / Covers / ...)            │          │
│          │                                           │          │
│          ├───────────────────────────────────────────┤          │
│          │ StatusBar (件数/選択/DB情報)               │          │
└──────────┴───────────────────────────────────────────┴──────────┘
```

全体キャンバスサイズは **1400×900px** で設計。デスクトップウィンドウ前提のためビューポートに合わせて拡大縮小は不要（ネイティブウィンドウで実行）。

---

### 1. Sidebar（左 232px）

`prototype/sidebar.jsx`

#### ヘッダー
- LumenCiteロゴ（22×22 角丸6px、`linear-gradient(140deg, oklch(0.78 0.16 75), oklch(0.62 0.16 50))`）
- 「LumenCite / 研究ライブラリ」テキスト
- 同期アイコンボタン（右端）
- 領域は `WebkitAppRegion: 'drag'`（タイトルバー的に使う）

#### セクション

| セクション | 項目 |
|-----------|------|
| ライブラリ | すべての文献 / 最近追加 / お気に入り / 未整理 / ゴミ箱 |
| コレクション | 階層フォルダ（`expandable`、`chevronRight`が90°回転） |
| タグ | 色付き dot + タグ名 + 件数 |

#### NavRow仕様
- 高さ: 24-26px、padding `5px 14px 5px 10+indent*14 px`
- フォント: 13px / 450（active時 550）
- アクティブ背景: `var(--accent-soft)`、文字色: `var(--accent-strong)`
- ホバー背景: `var(--hover)`
- 角丸: 6px、左右マージン: 6px

#### ボトム
同期ステータス行（border-top、緑のdotで「references.bib と同期中」）

---

### 2. Toolbar（メインヘッダー2行）

`prototype/main.jsx` 内 `Toolbar`

#### Row 1（高さ50px）
- タイトル: 15px / 600 / `letter-spacing: -0.01em`
- 件数バッジ: 11px / 500、`var(--surface-2)` 背景、`border-radius: 999`
- サブタイトル: 11.5px / `var(--text-faint)`
- アクション: `インポート` ・ `+ 文献を追加`（プライマリ）

#### Row 2（高さ約38px）
- 検索ボックス: max-width 460px、placeholder「タイトル・著者・DOI・本文で検索…」、右端に⌘Kバッジ
- フィルタボタン / 列ボタン / 並び替えボタン
- 全 `var(--surface-2)` 内に1pxボーダー、6px角丸

---

### 3. ViewTabs（ビュー切替タブ・高さ34px）

| ID | ラベル | 状態 |
|----|--------|------|
| `table` | 表 | 実装済み |
| `covers` | カバー | 実装済み（PDF表紙風グリッド） |
| `timeline` | タイムライン | `soon` バッジ表示 |
| `graph` | 引用グラフ | `soon` バッジ表示 |

- タブはアイコン + ラベル
- アクティブ: 文字`var(--text)`、下線2px `var(--accent-strong)`
- 非アクティブ: `var(--text-mute)`
- 無効: opacity 0.55、カーソル `not-allowed`、`soon`バッジ（9.5px）
- 右端にビュー説明テキスト（`メタデータ重視` / `PDFサムネイル`）

将来ビューを足すときは `tabs` 配列に1要素足し、`viewMode === "..."` 分岐に対応コンポーネントを追加するだけ。

---

### 4. Content - Table View（`viewMode === 'table'`）

`prototype/table.jsx`

#### 列構成
| 列 | 幅 | 内容 |
|----|----|----|
| star | 28px | お気に入りトグル（hover時のみ表示） |
| type | 28px | 種別アイコン（article/book/inproceedings/thesis/webpage/misc） |
| title | flex | タイトル（13px / 500） + 📎添付 + 未読dot |
| authors | 200px | `formatAuthors`（>2人は `et al.`） |
| year | 56px | tabular-nums |
| venue | 150px | italic |
| tags | 200px | 最大3つ + `+N` |
| added | 100px | YYYY/MM/DD、右寄せ、tabular-nums |

#### Row仕様
- 高さ: density別 — compact 30 / default 36 / comfortable 42
- フォント: 12.5px、タイトルは13px / 500
- 選択時: 左2pxアクセントバー + `var(--row-selected)` 背景
- ホバー: `var(--row-hover)`
- 下線: `var(--border-subtle)` 1px

#### ヘッダー
- 背景: `var(--surface-2)`、sticky top:0
- 11px / 600 / `var(--text-mute)`
- ソート可能列はクリックで `asc` ↔ `desc` トグル、現在のソート列はアクセントカラーの矢印

---

### 5. Content - Covers View（`viewMode === 'covers'`）

`prototype/main.jsx` 内 `CoverCard` / `CoversGrid`

PDF表紙風の擬似サムネイル一覧。

- グリッド: `repeat(auto-fill, minmax(150px, 1fr))`、gap 10px、padding `16px 18px`
- カード:
  - カバー: aspectRatio 0.72、エントリIDから生成した hue で色変化（`oklch(0.96 0.02 ${hue})` → `oklch(0.88 0.04 ${hue})`）
  - 斜め45°ストライプパターン（opacity 0.18）
  - 左上: type chip（半透明白背景、9px / 600 / uppercase）
  - タイトル: IBM Plex Serif 8.5px、4行クランプ
  - 著者・年: 表紙下部
  - 添付ありは右上に丸い📎バッジ
- 下部メタ: タイトル2行クランプ + 著者・年

**実装時の注意:** これはあくまでプレースホルダ。本実装では **PDF最初のページをサムネイル化** してそれを表紙として使う。`pdfium` / `pdf-rs` などRust側で1ページ目をPNGに落として `attachments/<id>/cover.png` に保存し、フロントから読む構成を推奨。

---

### 6. Detail Panel（右 320px）

`prototype/detail.jsx`

#### Hero（上部・border-bottom）
- type chip + 年 + ☆トグル
- タイトル: 15.5px / 600 / `line-height: 1.32` / `letter-spacing: -0.012em`
- 著者全員（カンマ区切り）: 12px / `var(--text-mute)`
- venue: 11.5px / italic / `var(--text-faint)`
- アクションボタン: `PDFを開く`（primary） / `要約` / `BibTeX`

#### Tabs
情報 / 抄録 / ノート / 関連（border-bottom、アクティブは `var(--accent-strong)` 1.5px下線）

#### Info タブ
- DOI / arXiv / ISBN / URL（mono フォント）
- 掲載年 / 出版 / 追加日
- タグ（`TagPill` 複数 + 「追加」破線ボタン）
- コレクション（フォルダアイコン + 名前）

#### Abstract タブ
12.5px / line-height 1.65。未登録時は灰色メッセージ。

#### Notes タブ
未登録時は CTA ボタン「ノートを作成」。

---

### 7. StatusBar（フッター・高さ24px）

- 「N / 全件 件」
- 「選択中: 1件 / なし」
- DB情報: 「SQLite · N entries · M authors · K tags」
- 11px / `var(--text-faint)`、`var(--surface-2)` 背景

---

### 8. Add Entry Sheet

`AddSheet`（`prototype/main.jsx`）

- モーダル（背景 `rgba(20,18,14,0.28)` + `backdrop-filter: blur(2px)`）
- 460px幅、10px角丸、上から90pxの位置
- タブ: DOI / arXiv / ISBN / BibTeX 貼付 / 手動入力
- 入力フィールド + 「CrossRef / arXiv / Google Books から取得します」インフォ
- フッター: キャンセル / 取得（プライマリ）

---

## デザイントークン

### Colors（OKLCH）

#### Light
```css
--bg:             oklch(0.985 0.003 80);
--surface:        #ffffff;
--surface-2:      oklch(0.975 0.004 80);
--sidebar:        oklch(0.972 0.004 80);
--border:         oklch(0.92  0.005 80);
--border-subtle:  oklch(0.95  0.004 80);
--border-strong:  oklch(0.86  0.006 80);
--text:           oklch(0.22  0.01  70);
--text-mute:      oklch(0.5   0.008 70);
--text-faint:     oklch(0.65  0.005 70);
--row-hover:      oklch(0.965 0.005 80);
--row-selected:   oklch(0.955 0.02  70);
--hover:          oklch(0.95  0.005 80);
```

#### Dark（グレー基調・低コントラスト）
```css
--bg:             oklch(0.27  0.004 80);
--surface:        oklch(0.31  0.004 80);
--surface-2:      oklch(0.29  0.004 80);
--sidebar:        oklch(0.285 0.004 80);
--border:         oklch(0.38  0.004 80);
--border-subtle:  oklch(0.34  0.004 80);
--border-strong:  oklch(0.44  0.004 80);
--text:           oklch(0.86  0.004 80);
--text-mute:      oklch(0.66  0.004 80);
--text-faint:     oklch(0.52  0.004 80);
--row-hover:      oklch(0.34  0.004 80);
--row-selected:   oklch(0.38  0.018 70);
--hover:          oklch(0.34  0.004 80);
```

#### Accents（hueを変えるだけで4色展開）
```js
amber:  { strong: oklch(0.62 0.14 65),  soft: oklch(0.95 0.04 70)  }  // default
indigo: { strong: oklch(0.52 0.16 270), soft: oklch(0.95 0.04 270) }
teal:   { strong: oklch(0.55 0.10 195), soft: oklch(0.95 0.04 200) }
rose:   { strong: oklch(0.58 0.16 15),  soft: oklch(0.95 0.04 15)  }
```
ダーク時 `accent-strong` は `oklch(0.74 0.12 65)` に。

#### Tag colors
```js
amber:  bg oklch(0.95 0.05 75)  / fg oklch(0.42 0.12 65)  / dot oklch(0.7  0.13 70)
blue:   bg oklch(0.95 0.04 240) / fg oklch(0.42 0.12 245) / dot oklch(0.6  0.13 245)
green:  bg oklch(0.95 0.04 150) / fg oklch(0.4  0.10 150) / dot oklch(0.62 0.12 150)
violet: bg oklch(0.95 0.04 295) / fg oklch(0.42 0.12 295) / dot oklch(0.6  0.13 295)
rose:   bg oklch(0.95 0.04 15)  / fg oklch(0.45 0.13 15)  / dot oklch(0.65 0.15 15)
cyan:   bg oklch(0.95 0.04 200) / fg oklch(0.42 0.10 210) / dot oklch(0.6  0.12 210)
```

### Typography
- UI: **IBM Plex Sans** (400/450/500/550/600/700)
- 日本語: **Noto Sans JP** (400/500/600) — フォールバック
- 識別子（DOI / arXiv / ISBN / URL / 件数）: **IBM Plex Mono** (400/500)
- カバー擬似タイトル: **IBM Plex Serif**（カバービューの中だけ）
- 読み込み: Google Fonts（`prototype/Library.html` 参照）

スケール:
| 用途 | サイズ | weight | letter-spacing |
|------|-------|--------|----------------|
| 詳細パネル タイトル | 15.5 | 600 | -0.012em |
| ツールバー タイトル | 15 | 600 | -0.01em |
| 本文（セクション） | 13 | 500 | -0.005em |
| テーブル row | 12.5 | — | — |
| メタ・サブ | 11.5–12 | — | — |
| ラベル/UPPERCASE | 10.5–11 | 600 | 0.06em |

### Spacing
- 角丸: 4 / 5 / 6 / 8 / 10 / 12px（コンポーネント階層に応じて）
- 余白: 4 / 6 / 8 / 10 / 12 / 14 / 16 / 18px グリッド
- ボーダー: 0.5 / 1 / 1.5 / 2px

### Shadows
```css
/* Window */
0 0 0 1px oklch(0 0 0 / 0.08),
0 1px 0 oklch(1 0 0 / 0.6) inset,
0 30px 60px oklch(0 0 0 / 0.18),
0 8px 20px oklch(0 0 0 / 0.10);

/* Modal */
0 20px 50px rgba(0,0,0,0.18), 0 1px 0 rgba(0,0,0,0.05);

/* Cover card */
0 1px 0 rgba(0,0,0,0.06),
0 4px 14px rgba(20,15,8,0.10),
0 0 0 0.5px oklch(0 0 0 / 0.10);
```

---

## アイコン

すべて自作16×16 SVG（`prototype/app.jsx` の `Icon` / `TypeIcon`）。
- UI: search, plus, chevronDown, chevronRight, folder, star/starFill, paperclip, library, clock, inbox, trash, tag, sortAsc, filter, columns, grid, list, download, upload, sync, info, sparkle, ext
- 種別: article, book, inproceedings, thesis, webpage, misc

本実装では **Lucide React** か **Tabler Icons** など整備されたアイコンライブラリへの移行を推奨します（自作SVGの保守コスト削減）。種別アイコンだけは独自で作る価値があります。

---

## State Management

| state | 型 | 初期値 | 用途 |
|-------|----|----|----|
| `selectedView` | `"all" \| "starred" \| "recent" \| "unfiled" \| "trash" \| `col:${id}` \| `tag:${name}`` | `"all"` | サイドバー選択 |
| `selectedId` | `number \| null` | `1` | テーブルで選択中のentry |
| `sort` | `{ key, dir }` | `{ key: "added", dir: "desc" }` | ソート |
| `search` | `string` | `""` | 検索クエリ |
| `viewMode` | `"table" \| "covers" \| "timeline" \| "graph"` | `"table"` | ビュー切替 |
| `density` | `"compact" \| "default" \| "comfortable"` | `"default"` | テーブル行高 |
| `theme` | `"light" \| "dark"` | `"light"` | テーマ |
| `accent` | `"amber" \| "indigo" \| "teal" \| "rose"` | `"amber"` | アクセント色 |
| `showAdd` | `boolean` | `false` | Addシート表示 |

`theme`, `accent`, `density`, `viewMode`, `sort` は **`localStorage` で永続化** することを推奨。

---

## Tauri ↔ Frontend データ取得

`docs/API_SPEC.md` の `EntrySummary` をテーブル行で使用。

```ts
// 一覧読み込み
const entries: EntrySummary[] = await invoke("get_entries", {
  collection_id: selectedView.startsWith("col:") ? Number(selectedView.slice(4)) : undefined,
  tag_id: selectedView.startsWith("tag:") ? tagIdByName(selectedView.slice(4)) : undefined,
});

// 詳細読み込み（右パネル用）
const detail: EntryDetail = await invoke("get_entry", { id: selectedId });

// 検索
const results: EntrySummary[] = await invoke("search_entries", { query: search });
```

クライアント側ソート/フィルタはOK。ただし件数が増えたらRust側にソート渡す検討を。

---

## 実装ステップ提案

1. デザイントークン確立: CSS変数 or theme オブジェクト + テーマ切替フック
2. レイアウトshell（Sidebar / Main / DetailPanel）の grid配置
3. Sidebar — モックデータで描画 → `get_collections` / `get_tags` 連携
4. Toolbar + ViewTabs
5. Table view — `get_entries` 連携、ソート、選択
6. DetailPanel — `get_entry` 連携、タブ切替
7. Add Sheet — フォーム + `fetch_metadata_by_doi` / `fetch_metadata_by_arxiv` 連携
8. ⌘K検索 — 全文検索（`fulltext_search`）統合
9. Covers view — Rust側でPDF1ページ目→PNGサムネ生成 → 読み込み
10. キーボードショートカット（↑↓選択、⌘N新規、⌘F検索、Spaceで詳細パネルtoggle 等）

---

## Files

`prototype/` 配下:
- `Library.html` — エントリポイント・CSS変数・スケーリング・フォント読込
- `main.jsx` — App、Toolbar、ViewTabs、CoversGrid、AddSheet、テーマ適用
- `sidebar.jsx` — Sidebar、NavRow、SidebarSection
- `table.jsx` — EntriesTable、Row、ColumnHeader、TagPill
- `detail.jsx` — DetailPanel、Field、Tab
- `app.jsx` — Icon、TypeIcon、TAG_COLORS（共通プリミティブ）
- `data.jsx` — モックデータ（25件のサンプルエントリ）
- `starters/tweaks-panel.jsx` — Tweaksパネル（ダーク/アクセント/密度切替UI、本実装には不要）

ブラウザで `Library.html` を開けば動作します。

---

## Notes

- **モックデータの著者名・論文タイトル等は公開メタデータ（実在する論文）** ですがテスト目的で使用しているだけなので、本実装では空のDBから始めてください
- プロトタイプはJSXですが本実装は **TypeScript必須**（`docs/API_SPEC.md` の型定義を活用）
- 既存の `src/App.tsx` / `App.css` は丸ごと差し替えてOK
