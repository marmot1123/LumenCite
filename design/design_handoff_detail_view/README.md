# Handoff: LumenCite — 文献詳細画面（PDFビューワー含む）

## Overview

LumenCite の **文献詳細画面**（個別エントリのPDFビューワーとメタデータパネル）のデザイン仕様です。`Library.html` のテーブル行から「PDFを開く」やダブルクリックで遷移する、ライブラリビューに次ぐ第二のメイン画面です。

これは前回の **`design_handoff_library_view/`（ライブラリ画面）と同じデザイントークン**（OKLCH配色・IBM Plex タイポグラフィ・余白スケール）を共有します。両者を一つのアプリ内で実装する想定なので、CSS変数やテーマプロバイダは共通化してください。

## About the Design Files

`prototype/` 配下のHTML/JSXは **デザインリファレンス** です。実際の実装では：

- React 18 + TypeScript で実装
- スタイルは現状ベタ書き（`style={...}`）— 本実装ではプロジェクトの方針（CSS Modules / Tailwind / Vanilla Extract等）に合わせて書き換え
- PDFレンダリングは **`pdf.js`（pdfjs-dist）** を使う（後述）
- データ取得は `invoke()` 経由で Tauri コマンドを呼ぶ（`docs/API_SPEC.md`）

ブラウザで `Detail.html` を開けば動きます（ただしPDFは擬似再現で、実装ではpdf.jsで実PDFを描画する）。

## Fidelity

**High-fidelity** — レイアウト・配色・タイポグラフィすべて確定済み。ライブラリ画面と同じデザイントークンに従ってピクセル一致で再現してください。

---

## 画面構成

```
┌──────────────────────────────────────────────────────────────────┐
│ Header  ← ライブラリ │ ARTICLE │ Title…    │ ☆ 引用 要約 ⬇ ⋯     │ 50px
├────────────────────────────────────────────────────┬─────────────┤
│ PDFToolbar   📑 ◀ 1/15 ▶ │ - 100% + ⛶ │ ↖✏️📝🖊️ │ 🔍 検索 │ 📑   │ 38px
├────┬───────────────────────────────────────────────┼─────────────┤
│Thmb│                                               │ MetaTabs    │
│96px│            PDF Page (612×792)                 │             │
│    │            (US Letter, scrollable)            │ Info /      │
│    │                                               │ Highlights /│
│    │                                               │ Notes /     │
│    │                                               │ Related     │
│    │                                               │   340px     │
└────┴───────────────────────────────────────────────┴─────────────┘
```

全体: 1400×900 デスクトップウィンドウ前提。

---

### 1. Header（高さ50px）

`prototype/detail-app.jsx` の `Header`

- **戻る**: 「← ライブラリ」ボタン → `Library.html` に遷移（実装では React Router / 状態管理）
- **種別チップ**: `ARTICLE` 11pxアップロー、`var(--surface-2)` 背景、4px角丸
- **タイトル**: 14px / 600、`text-overflow: ellipsis`（長いタイトルは1行で切る）
- **アクション**: ☆お気に入り / 引用 / 要約 / DL / ⋯
  - フォント12px、padding `5px 9px`、ホバー時 `var(--surface-2)` 背景

### 2. PDF Toolbar（高さ38px）

`prototype/detail-app.jsx` の `PDFToolbar`

| ブロック | 内容 |
|---------|------|
| 左サイドバートグル | サムネイル一覧の表示/非表示 |
| ページナビ | `◀` `[ 1 ]` `/ 15` `▶`、ページ番号は直接入力可、IBM Plex Mono |
| ズーム | `−` `100%` `+` `⛶` (フィット) — 50% to 200%、10%刻み |
| 注釈ツール | セグメントトグル: 選択 / ハイライト / ノート / ペン |
| 本文検索 | 200px幅、`🔍 本文を検索...`、結果は `3 / 12` 形式表示（mono） |
| 右サイドバートグル | メタパネルの表示/非表示 |

### 3. PDF Thumbnails（左レール 96px）

- 各サムネイル: 72×92px の白いページ
- アクティブ: 2px `var(--accent-strong)` ボーダー、ページ番号もアクセント色
- ページ番号は IBM Plex Mono
- スクロール可

**実装時:** pdf.js の `PDFPageProxy.render({ canvasContext })` で各ページを小さい canvas にレンダリングするのが標準。最初は1ページ目だけ高速描画して、可視範囲外は遅延描画する。

### 4. PDF Viewer（中央）

#### プロトタイプの実装
`prototype/detail-app.jsx` の `PDFPage1` で、Transformer論文を **HTML+CSSで擬似再現**（IBM Plex Serif、2段組、数式・図キャプション・ハイライト3色）。これはあくまでビジュアル参照用。

#### 本実装
**`pdfjs-dist`** を使う。例：

```ts
import * as pdfjsLib from "pdfjs-dist";
import "pdfjs-dist/build/pdf.worker.mjs";

// PDFファイルパスは Tauri から取得
const filePath = await invoke<string>("get_attachment_path", { entry_id });
const pdfBytes = await readBinaryFile(filePath);
const pdf = await pdfjsLib.getDocument({ data: pdfBytes }).promise;

// レンダリング
const page = await pdf.getPage(pageNum);
const viewport = page.getViewport({ scale: zoom / 100 });
const canvas = document.createElement("canvas");
const ctx = canvas.getContext("2d");
canvas.width = viewport.width;
canvas.height = viewport.height;
await page.render({ canvasContext: ctx, viewport }).promise;
```

ハイライト・選択は **テキストレイヤー**（`page.getTextContent()` で取得した位置情報を絶対配置の `<span>` として canvas の上に重ねる）で実装。テキスト選択 → ハイライトボタンで色を保存、というUXがpdf.js標準。

ハイライト保存先: SQLiteに `highlights` テーブルを追加し、`(entry_id, page, x, y, width, height, color, note)` を持たせる。`docs/DATA_MODEL.md` を更新。

### 5. Right Metadata Panel（340px・4タブ）

`prototype/detail-app.jsx` の `MetaPanel`

#### タブ
- **情報** — タイトル / 著者 / venue / DOI / arXiv（mono） / 抄録 / タグ（TagPill） / コレクション / 関連
- **ハイライト** — ページ別にカード表示。色チップ（4px幅・縦長）+ ページ番号（mono）+ 引用文（IBM Plex Serif、`"…"` でくくる）+ オプションのノート（破線セパレータの下）
- **ノート** — Markdown風自由記述（実装ではTiptap / CodeMirror / Milkdownなど推奨）
- **関連** — 引用関係カード。`preprint of` / `cited by` ラベル（uppercase / アクセント色）+ タイトル + 年

#### Field 共通スタイル
- ラベル: 10.5px / 600 / uppercase / `letter-spacing: 0.06em` / `var(--text-faint)`
- 値: 12.5px / `line-height: 1.45`
- DOI/arXiv等の識別子は `font-family: var(--mono)`

---

## デザイントークン

**ライブラリ画面と完全共通**。前回ハンドオフ（`design_handoff_library_view/README.md`）の「デザイントークン」セクションをそのまま使ってください。本画面で追加されるのは以下のみ：

```css
--highlight-yellow: oklch(0.93 0.13 95 / 0.55);
--highlight-green:  oklch(0.92 0.13 145 / 0.5);
--highlight-blue:   oklch(0.92 0.10 240 / 0.5);
```

ハイライトのチップ色（メタパネル側）：
```js
yellow: oklch(0.85 0.15 95)
green:  oklch(0.78 0.13 145)
blue:   oklch(0.7  0.13 240)
```

---

## State Management

| state | 型 | 初期値 | 用途 |
|-------|----|----|----|
| `entryId` | `number` | URLパラメータから | 表示中エントリ |
| `page` | `number` | `1` | 現在のPDFページ |
| `pages` | `number` | PDF読込時 | 総ページ数 |
| `zoom` | `number` | `100` | ズーム倍率（%） |
| `mode` | `"select" \| "highlight" \| "note" \| "pen"` | `"select"` | 注釈モード |
| `search` | `string` | `""` | 本文検索クエリ |
| `leftOpen` | `boolean` | `true` | サムネイル表示 |
| `rightOpen` | `boolean` | `true` | メタパネル表示 |
| `metaTab` | `"info" \| "highlights" \| "notes" \| "related"` | `"info"` | メタパネルタブ |

**永続化推奨:** `zoom`, `leftOpen`, `rightOpen`, `metaTab` を localStorage に。`page` はエントリごとに保存して復帰時に再開。

---

## Tauri バックエンド連携

詳細画面で必要となる新規/既存コマンド：

```ts
// 既存（library_view と共通）
const detail: EntryDetail = await invoke("get_entry", { id: entryId });

// 新規想定（DATA_MODEL.md / API_SPEC.md に追記が必要）
const path: string  = await invoke("get_attachment_path", { entry_id, kind: "pdf" });
const highlights: Highlight[] = await invoke("get_highlights", { entry_id });
await invoke("create_highlight", { entry_id, page, rect, color, note });
await invoke("update_note", { entry_id, body });
const related: Related[] = await invoke("get_related_entries", { entry_id });
```

PDFバイナリは Tauri ファイルAPI（`@tauri-apps/plugin-fs` の `readFile`）で読むのが速い。`get_attachment_path` でパスだけもらって、フロント側で `readFile` する構成を推奨（Rust側でバイト列をJSONシリアライズすると非効率）。

### Highlight データモデル例

```sql
CREATE TABLE highlights (
  id          INTEGER PRIMARY KEY,
  entry_id    INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
  page        INTEGER NOT NULL,
  -- pdf.js座標（PDFポイント、左下原点）
  x           REAL NOT NULL,
  y           REAL NOT NULL,
  width       REAL NOT NULL,
  height      REAL NOT NULL,
  color       TEXT NOT NULL,   -- 'yellow' | 'green' | 'blue'
  text        TEXT NOT NULL,   -- 抽出済みテキスト
  note        TEXT,
  created_at  TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_highlights_entry_page ON highlights(entry_id, page);
```

---

## キーボードショートカット（推奨）

| キー | 動作 |
|------|------|
| `←` / `→` | 前/次ページ |
| `↑` / `↓` | スクロール |
| `⌘+` / `⌘-` / `⌘0` | ズーム |
| `⌘F` | 本文検索 |
| `⌘[` / `⌘]` | 左右サイドバートグル |
| `Esc` | ライブラリへ戻る |
| `H` | ハイライトモード |
| `N` | ノートモード |

---

## 実装ステップ提案

1. ルーティング: `Library` ↔ `Detail` の遷移（`entryId` をURL or 状態に保持）
2. ヘッダー（戻る + メタアクション）
3. PDFビューワー骨格（pdfjs-dist導入、1ページ目を canvas 描画）
4. PDFToolbar（ページナビ・ズーム・モードトグル）
5. Thumbnails（全ページサムネ生成）
6. テキストレイヤー → 選択 → ハイライト保存（DB連携）
7. MetaPanel: 情報タブ（既存 `get_entry` 流用）
8. MetaPanel: ハイライトタブ（`get_highlights` 連携、クリックで該当ページにジャンプ）
9. MetaPanel: ノート（簡易Markdownエディタ）
10. MetaPanel: 関連（DOI/arXiv の参照解決→`get_related_entries`）
11. 本文検索（pdf.js の `findController`）
12. キーボードショートカット

---

## Files

`prototype/` 配下:
- `Detail.html` — エントリポイント・CSS変数・PDF擬似再現スタイル・Mac風ウィンドウシェル
- `detail-app.jsx` — 全コンポーネント（Header / PDFViewer / PDFToolbar / Thumbnails / PDFPage1 / MetaPanel / MetaTabs）

ブラウザで `Detail.html` を開けば動きます。`Library.html` から戻れるようにリンクが張ってあるので、両者をセットで配置すると行き来できます。

---

## 関連ハンドオフ

- `design_handoff_library_view/` — ライブラリ画面（先行ハンドオフ）

両ハンドオフのデザイントークンは共有なので、CSS変数や `theme.ts` 等を一箇所に集約してください。
