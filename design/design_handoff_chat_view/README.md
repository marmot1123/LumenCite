# Handoff: LumenCite — LLM Chat View (v0.2.0)

## Overview

LumenCite に新規追加する **LLM Chat 画面** のデザインハンドオフです。研究者がライブラリ内の文献を対象に自然言語で質問でき、LLM が FTS5 全文検索ツール・DB 書き換えツール・外部 MCP ツールを（承認制で）反復呼び出ししながら回答する画面です。

ライブラリ画面・詳細画面と並ぶ **第3のスクリーン** として実装してください。トップレベルのルートに追加します。

## About the Design Files

このバンドルに含まれている `Chat.html` / `chat-*.jsx` は **HTML で作られたデザインリファレンス**です。最終的な見た目と挙動を示す **プロトタイプ**であり、そのままプロダクションコードとして使うものではありません。

ターゲットコードベース（**Tauri 2 + React 18 + TypeScript + Vite**）の既存の構造・パターン・コンポーネント群に沿って **再実装** してください。具体的には:

- LumenCite 本体は `LumenCite/src/` 配下に React + TypeScript で実装されています。新しい Chat 画面も同様に TypeScript で書き、`LumenCite/src/components/chat/` 配下に配置することを推奨します。
- バックエンドの Chat ストリーミングは Tauri コマンド経由で受け取る前提です（`ChatStreamEvent` を listen → state を駆動）。本ハンドオフでは UI 側の状態遷移のみ定義しており、Tauri 側との接続コードは別途実装が必要です。
- `MathMarkdown` という既存コンポーネントがブリーフで言及されています（KaTeX による Markdown + 数式レンダラ）。assistant メッセージの本文レンダリングは **そちらを再利用** し、このプロトタイプの簡易レンダラ（`MdBody`）は破棄してください。

## Fidelity

**High-fidelity (hifi)**。色（oklch）、書体（IBM Plex Sans / Mono / Serif、Noto Sans JP）、余白、状態遷移を確定値として扱ってください。ただし下記は柔軟に扱って問題ありません:

- React コンポーネント名・props の形は再構成して構いません。
- 既存のアイコンセットがあればそれに置き換えてください（本リファレンスのインライン SVG は捨てて可）。
- ライブラリ画面（`Library.html`）・詳細画面（`Detail.html`）と同じテーマ変数（`--bg` / `--surface` / `--accent-strong` 等）を **そのまま流用**してください。新しいトークンを足すのは ToolCallCard 用の `--tc-*` のみです。

## Screens / Views

ルート画面は 1 画面（`Chat`）ですが、内部で 3 つのペインと複数のオーバーレイで構成されます。

### Pane A — SessionList（左サイドバー）

幅: **244px**、`flex-shrink: 0`、背景 `var(--sidebar)`、右側に `1px solid var(--border)`。

レイアウト（縦）:

1. **ヘッダ**（padding `12px 14px 10px`）
   - 24×24 の「ライブラリに戻る」ボタン（左矢印アイコン、`var(--surface)` 背景、1px border）
   - ブランド `LumenCite Chat`（13px / 600）と `N sessions`（10px / monospace / faint）
2. **新規 Chat ボタン**（padding 横 12px）
   - フル幅、高さ 32px、角丸 6px、`1px solid var(--border-strong)`
   - 内部に 16×16 のアクセントカラー塗りつぶし正方形 + 白の `+` アイコン
   - ラベル「新しい Chat」、右端に `⌘ N` のキーキャップ
   - ホバーで枠線とテキストがアクセントカラーに
3. **検索入力**（高さ 26px、`var(--surface-2)` 背景、`1px solid var(--border)`）
4. **セッションリスト**（`flex: 1`、縦スクロール）
   - **グループヘッダ**（10px / 600 / uppercase / letter-spacing 0.08em / `var(--text-faint)`）: 「今日」「昨日」「それ以前」
   - **SessionRow**（margin 横 6px、角丸 6px、padding `9px 12px`）
     - タイトル（12.5px / 500、選択時 600、最大 2 行で省略）
     - サブ行: ScopeChip + 相対日時 + メッセージ数（10px / monospace / faint、右寄せ）
     - 選択時: 背景 `color-mix(var(--accent-strong) 8%, var(--sidebar))`、左端に 2px のアクセントバー、`outline: 1px solid` でアクセント色 30% 透過
     - ホバー時: 右上に 18×18 の「…」メニューボタンが出現
   - **ScopeChip**: `all` の場合は `var(--surface-2)` 背景、`N papers` の場合はアクセントカラー薄塗り
5. **フッタ**（高さ ~38px、`1px solid var(--border)` 上線）
   - 緑のドット + 「すべてローカル保存」 + 右端に `chat.db`（monospace）
   - 密度 `compact / default / comfortable` で SessionRow の上下パディングが `7 / 9 / 12` に変化

**空状態**: セッション 0 件のとき、サイドバー内側中央に 36×36 のスパークルアイコンチップ + 「最初の Chat を始めよう」+ 説明文（11px）。

### Pane B — Conversation（中央）

`flex: 1`、`min-width: 0`、`position: relative`、内側でさらに縦 3 段。

#### B-1. SessionHeader（高さ ~50px）

- 背景 `var(--surface)`、下端 `1px solid var(--border)`、padding `10px 18px 11px 22px`、`display: flex`、`align-items: center`、`gap: 14px`。
- **タイトル**（14.5px / 600 / letter-spacing -0.01em）— クリックでインライン編集（`<input>` に切り替わり、`var(--accent-strong)` のフォーカスリング）
- **ScopeChip**（クリックで ScopePicker ポップオーバー）
  - 形: 角丸 999px、padding `3px 8px 3px 7px`
  - 内部: `scope:`（monospace / uppercase / 9.5px）+ ラベル + 下矢印
- **モデルバッジ** — `var(--surface-2)` の小チップに provider ドット + `Claude · sonnet-4.5`（monospace / 10.5px）
- 右側に検索 / アーカイブ / パネルトグル / その他の 4 つの 26×26 アイコンボタン

#### B-2. MessageList

- 背景 `var(--surface)`、padding `20px 40px 24px`、`max-width: 820px` センター揃え、`overflow: auto`
- **UserMessage**: 右寄せ、最大幅 76%、padding `10px 14px`、角丸 12px（右上のみ 4px）、背景 `color-mix(var(--accent-strong) 9%, var(--surface))`、`1px solid` アクセント 22%、白テキスト無し（`var(--text)`）。`white-space: pre-wrap`
- **AssistantMessage**: 左寄せ、左に 26×26 のグラデーション丸（LumenCite ブランドのトーチ意匠）+ 本文。本文は `<MdBody>` で `md` ブロック配列を順に描画
  - `p` → `<p>`（11px 下マージン）
  - `h` → `<h3>`（13.5px / 600 / 上 18px 下 8px）
  - `ul` → `<ul>`（padding-left 22px、要素 3px 上下）
  - `math_display` → `font-family: "IBM Plex Serif"`、italic、16px、中央寄せ、上下 14px。右端に `(N)` の式番号タグ
  - 行内 `<span class="math-inline">` は serif italic 14px
  - `tools` → `<ToolCallCard>` の縦スタック（gap 6px、上下マージン 10/14px）
- **ストリーミング中**: 最後のメッセージ末尾に 6×14px の点滅キャレット（`@keyframes blink`、1s steps(2)、アクセントカラー）

#### B-3. ToolCallCard（最重要）

5 系統 × 複数状態の単一コンポーネント。

| Kind | アイコン | テキスト色 | 背景（聴覚的に強い時のみ） | 用途 |
|---|---|---|---|---|
| `read` | 🔍 search | `--tc-read-fg` (`oklch(0.45 .01 70)`) | `--tc-read-bg` | `fulltext_search` / `get_entry` / `list_*` |
| `write` | ✏️ pencil | `--tc-write-fg` (`oklch(0.42 .08 170)` / teal-green) | `--tc-write-bg` | `add_tag` / `update_notes` / `attach_ocr_text` / `add_to_collection` |
| `approve` | ⚠️ warn | `--tc-approve-fg` (`oklch(0.42 .12 65)` / amber) | `--tc-approve-bg` | `create_entry` / `update_entry`（要承認） |
| `delete` | 🗑️ trash | `--tc-delete-fg` (`oklch(0.46 .16 20)` / rose) | `--tc-delete-bg` | `delete_*`（常時承認） |
| `mcp` | 🔌 plug | `--tc-mcp-fg` (`oklch(0.42 .13 285)` / violet) | `--tc-mcp-bg` | `mcp_<server>_<tool>` |

ルール: 「**呼び出し系統で色が変わるのは、approve と delete のときだけ濃く塗り、それ以外は白カードに色付きアイコン**」。読み取り系の自動実行が会話を埋めて鬱陶しくならないようにするためです。

**Card chrome**（共通）:
- 角丸 7px、`1px solid var(--border)`（louder 時は `--tc-*-bd` に切り替え）
- ヘッダ（クリックで開閉、padding `8px 10px`、`display: flex`、`gap: 9px`）
  - 22×22 のアイコンチップ
  - 中央 2 行: `tool_name(arg=val, ...)`（monospace / 11.5px）+ サマリ（11px / `--text-mute`）
  - 右: Kind バッジ（monospace / 9.5px / uppercase）+ シェブロン（開閉で 0°/90°）

**States**:

| State | 表示 |
|---|---|
| `done_collapsed` | ヘッダのみ。サマリは「`"quantum walk Hilbert space" — 7 hits across 3 papers`」のような 1 行 |
| `done_expanded` | ヘッダ + ボディ。ボディに引数 JSON プレビュー + 結果（検索ヒットスニペット / diff / MCP レスポンス） |
| `running` | ヘッダのみ。サマリ位置に 11×11 のスピナー（1.4px border、0.7s linear）+「実行中…」 |
| `needs_approval` | ヘッダ + ボディ + 承認バー。カード全体に `pulse-approve` アニメ（2.4s ease-in-out、外側に黄色グロー）。**ボディは常時 open**。クリックで閉じられない |
| `rejected` | `opacity: 0.7`、サマリが「拒否済み」になる |

**ヒットスニペット** (read kind expanded): `var(--surface)` の中に 7px padding カード、`entry-name`（monospace）+ `p.N` + スニペット本文（11.5px / line-height 1.5）。`<em>` 強調はそのまま太字に。

**Diff ブロック** (approve / write kind expanded): monospace 11.5px、`-` 行は `--tc-delete-fg` の薄塗り、`+` 行は `oklch(0.55 0.13 145)` の薄塗り。

**承認バー** (needs_approval のみ): カードボディ末尾、`color-mix(currentKindColor 12%, var(--surface))` 背景、`1px solid kindBorder`、warn アイコン + 説明テキスト + **[拒否]** ボタン（outline）+ **[許可]** ボタン（fill、Kind カラー、白テキスト、check アイコン付き）

#### B-4. Composer

- 背景 `var(--surface)`、`border-top: 1px solid var(--border-subtle)`、padding `10px 40px 16px`、内部 `max-width: 820px` センター
- 中央のフレーム: `1px solid var(--border-strong)`、角丸 10px、`box-shadow: 0 1px 0 rgba(0,0,0,0.03)`。フォーカス時は枠がアクセント色 + `0 0 0 3px var(--accent-ring)`
- `<textarea>` 行数 2、最小高さ 56px、padding `11px 14px 4px`、13.5px / line-height 1.55、フォーカス時の枠色変化のみ。`resize: none`
- 下端のツールバー（padding `4px 8px 8px 10px`、`gap: 8px`）: paperclip / library アイコンボタン + scope ラベル（monospace 10.5px）+ flex spacer + **送信ボタン**
- **送信ボタン**: `var(--accent-strong)` 塗り、白、padding `5px 12px 5px 11px`、角丸 6px、12px / 600、右側に Enter 矢印アイコン
- **ストリーミング中**: 送信ボタンを **中断** ボタンに置換（outline 風、stop アイコン + 「中断」テキスト）
- **承認待ち時 (blocked)**: textarea を disabled、placeholder を「承認待ち — まずツール呼び出しを許可または拒否してください」に変更、フレーム全体 `opacity: 0.65`。**上部に黄色の警告バナーを表示**（`--tc-approve-bg`、warn アイコン + 説明）
- 最下部 6px 上に超小さなヒント行: 「LumenCite はライブラリ内の検索結果を引用します。」+ `N chars`

### Pane C — ContextPanel（右パネル）

幅 **280px**、`flex-shrink: 0`、`var(--surface)` 背景、左に `1px solid var(--border)`。SessionHeader のパネルボタンでトグル。

縦構成:

1. **ヘッダ**（12px padding）: 「コンテキスト」+ 数バッジ + 追加 `+` ボタン
2. **選択中の文献**: アクセント色丸番号 + タイトル + `entry #ID`（monospace）の縦カード（gap 4px）
3. **このターンの引用**: 番号バッジ + 文献名 + 右側に `p.N §X`（monospace 10px）
4. **このセッションで使われたツール**: 5 系統の集計行。pending があるものは Kind カラーで `● N` 表示
5. **承認ポリシー説明**: `1px dashed var(--border)` の囲み、info アイコン + 短い解説（11px）

### Overlay 1 — ScopePicker

絶対配置、`top: 60px / left: 280px`、幅 **420px**、`var(--surface)`、`1px solid var(--border-strong)`、角丸 9px、強めの drop shadow。背景クリックで閉じる（透明オーバーレイ）。

- ヘッダ: タイトル「このセッションの検索対象」+ サブ
- モード切替（2 つの大きなボタン）: `all` / `entries`
- `entries` 選択時: 検索入力 + 文献リスト（max-height 240px、チェックボックス風の四角に check アイコン）
- `all` 選択時: 中央寄せの説明 + 件数
- フッタ: キャンセル / 適用

### Overlay 2 — NewSessionDialog

中央モーダル、幅 **520px**、暗い backdrop（rgba(20,18,14,0.32) + blur 2px）。

- ヘッダ: スパークルチップ + タイトル「新しい Chat を開始」+ サブ
- フィールド「プロバイダ / モデル」: Anthropic / OpenAI のピル + モデル `<select>`
- フィールド「初期スコープ」: 全体 / 特定（ScopePicker と同じ ScopeMode コンポーネント）
- 特定選択時: 選択済み文献のミニリスト（削除可、追加 CTA）
- フッタ: キャンセル / 開始

### Empty State — Conversation

サイドバーが空（または未選択）時の中央。56×56 のグラデーションスパークルアイコン + 「ライブラリを LLM に相談する」+ 説明 + 4 枚の SuggestionCard（2×2 グリッド、最大幅 560px）。

## Interactions & Behavior

### ストリーミング

サーバから来る `ChatStreamEvent` で駆動:

```
session_started   → 新しいセッションを state に追加 / activeId 更新
delta             → 最後の assistant メッセージの本文末尾に文字列を追加
tool_call_proposed → 最後の assistant メッセージに ToolCallCard を append。
                     needs_approval=true なら state="needs_approval" で blocking=true に
tool_call_executed → 該当 call_id のカードを state="done_collapsed" に遷移
message_persisted → 楽観的 UI を確定。id を反映
done              → streaming=false、composer を再有効化
error             → 最後の assistant メッセージ末尾にエラーバナーを差し込み
```

### スクロール追従

新規 delta 到着時、ユーザーがスクロール末尾近く（例: 末尾から 80px 以内）にいる場合のみ自動で末尾に追従。それより上にいるときは追従しない。`MessageList` の親要素に `scroll` イベントを張り、`scrollHeight - scrollTop - clientHeight < 80` の閾値で判定。

### 中断

`onStop` が呼ばれたら Tauri 側に cancel イベントを送る。受信側では「進行中の応答を止めるが、これまで届いた部分応答は保存」する仕様。UI 上は streaming=false に戻し、最後のメッセージの末尾に小さな「中断されました」ラベルを差し込む（本ハンドオフ未実装、追加してください）。

### 承認カード

```
[許可] → tool_call を実行に進める Tauri コマンドを発火 → state="running" → 結果が来たら "done_collapsed"
[拒否] → reject イベントを送る → state="rejected"、blocking=false に戻す
```

両方とも完了するまで Composer は無効化（`blocking` フラグ）。

### タイトル編集

`<h1>` クリックで `<input>` に置換。`Enter` または `Escape` または `blur` で保存。Tauri 側にタイトル更新コマンドを送る。LLM 自動生成タイトルは最初のターン完了後に postMessage 等で上書き可能にしてください。

### ScopePicker

適用時、現在のセッションの `scope_mode` / `entry_ids` を更新するコマンドを送る。以降の `fulltext_search` 呼び出しはこのスコープを尊重します。

### キーボードショートカット

- `⌘ N` — 新規 Chat
- `⌘ K` — グローバルコマンドパレット（既存）から「新規 Chat」「セッション一覧」へジャンプ
- `⌘ [` — サイドバー トグル（既存アプリと整合）
- `⌘ ↩` — Composer 送信
- `Esc` — オーバーレイ閉じる、タイトル編集確定

## State Management

最小限の React state 設計例:

```ts
type ChatState = {
  sessions: ChatSession[];
  activeSessionId: number | null;
  messagesBySession: Record<number, ChatMessage[]>;
  streaming: boolean;
  blocking: boolean;                    // 承認待ちで Composer ブロック中か
  pendingApprovals: Record<string, ToolCallSpec>; // call_id → spec
  scopePickerOpen: boolean;
  newSessionDialogOpen: boolean;
  rightPanelOpen: boolean;
  sidebarOpen: boolean;
  // ストリーミング進行中の最後のメッセージへの逐次 append 用
  lastAssistantId: number | null;
};
```

`ChatStreamEvent` を Tauri の `event::listen` で受け、reducer で上記を更新するのが素直です。

## Design Tokens

すべて既存の `:root` 変数を流用してください。Chat 専用に追加する変数のみここに列挙します。

### Light theme tokens (追加分)

```css
--tc-read-bg:    oklch(0.975 0.004 80);
--tc-read-bd:    oklch(0.91 0.005 80);
--tc-read-fg:    oklch(0.45 0.01 70);

--tc-write-bg:   oklch(0.97 0.025 170);
--tc-write-bd:   oklch(0.86 0.04 170);
--tc-write-fg:   oklch(0.42 0.08 170);

--tc-approve-bg: oklch(0.97 0.06 75);
--tc-approve-bd: oklch(0.78 0.13 75);
--tc-approve-fg: oklch(0.42 0.12 65);

--tc-delete-bg:  oklch(0.97 0.04 20);
--tc-delete-bd:  oklch(0.78 0.14 20);
--tc-delete-fg:  oklch(0.46 0.16 20);

--tc-mcp-bg:     oklch(0.96 0.04 285);
--tc-mcp-bd:     oklch(0.82 0.08 285);
--tc-mcp-fg:     oklch(0.42 0.13 285);
```

### Dark theme tokens (追加分)

```css
--tc-read-bg:    oklch(0.33 0.004 80);
--tc-read-bd:    oklch(0.40 0.004 80);
--tc-read-fg:    oklch(0.72 0.005 80);

--tc-write-bg:   oklch(0.34 0.025 170);
--tc-write-bd:   oklch(0.45 0.04 170);
--tc-write-fg:   oklch(0.78 0.08 170);

--tc-approve-bg: oklch(0.36 0.04 75);
--tc-approve-bd: oklch(0.55 0.10 75);
--tc-approve-fg: oklch(0.82 0.12 75);

--tc-delete-bg:  oklch(0.34 0.04 20);
--tc-delete-bd:  oklch(0.50 0.10 20);
--tc-delete-fg:  oklch(0.80 0.14 20);

--tc-mcp-bg:     oklch(0.33 0.03 285);
--tc-mcp-bd:     oklch(0.46 0.06 285);
--tc-mcp-fg:     oklch(0.80 0.10 285);
```

### Spacing / radius

| トークン | 値 | 用途 |
|---|---|---|
| Card radius | 7px | ToolCallCard |
| Bubble radius | 12px (4px on speaker corner) | user message |
| Composer radius | 10px | composer frame |
| Pill radius | 999px | ScopeChip / Provider pill |
| Small chip radius | 3-5px | type chip / kind badge |

### Typography

| 用途 | フォント | サイズ | weight |
|---|---|---|---|
| Body | IBM Plex Sans / Noto Sans JP | 13.5px | 400 |
| Session title | IBM Plex Sans | 12.5px | 500/600 |
| Message body | IBM Plex Sans | 13.5px (line-height 1.66) | 400 |
| Tool name | IBM Plex Mono | 11.5px | 500/600 |
| Tool summary | IBM Plex Sans | 11px | 400 |
| Kind badge | IBM Plex Mono | 9.5px / uppercase / letter-spacing 0.06em | 600 |
| Math inline | IBM Plex Serif italic | 14px | 400 |
| Math display | IBM Plex Serif italic | 16px | 400 |
| Section label | IBM Plex Sans | 9.5-10.5px / uppercase / letter-spacing 0.08em | 600 |
| Citation chip | IBM Plex Mono | 10px superscript | 500 |

### Animations

| 名前 | 用途 | spec |
|---|---|---|
| `blink` | ストリーミングキャレット | `1s steps(2) infinite`、`opacity: 0` at 50% |
| `spin` | ツール実行スピナー | `0.7s linear infinite` |
| `pulseApprove` | 承認待ちカード | `2.4s ease-in-out infinite`、`box-shadow` を 0→3px の黄色グローへ |

### Tone

セッション一覧は読みやすさ優先、本文は **学術論文に近い静かな密度**。色は accent と tool kind 以外ほぼニュートラル。emoji は使わず、すべてインライン SVG アイコンで統一。

## Assets

すべてインラインで完結しています。新しい外部画像アセットは不要です。

- アイコン: `chat-icons.jsx` の `ChatIcon` コンポーネント（16×16 viewBox、共通の `name` プロップ）。既存の Library 画面の `Icon` コンポーネントと同一の語彙です。マージしてください。
- フォント: `IBM Plex Sans` / `IBM Plex Mono` / `IBM Plex Serif` / `Noto Sans JP`（Google Fonts）。既存画面と同じ。
- ブランドマーク: assistant アバターの 26×26 トーチ意匠は SVG インライン。既存の `sidebar.jsx` 内のブランドマークと同じ意匠を流用しています。

## Files

このバンドルの内訳:

| ファイル | 役割 |
|---|---|
| `Chat.html` | ホスト HTML。Mac window 風シェル + 1400×900 のスケーラブルキャンバス。CSS 変数定義、フォント読み込み、Markdown 用ユーティリティクラス（`.md`、`.math-display`、`.citation`、`.caret`、`.pulse-approve`）を含む |
| `chat-data.jsx` | サンプルデータ。`CHAT_SESSIONS`（8 セッション）+ `CHAT_MESSAGES`（量子ウォーク 3 本に関する 6 メッセージのモック、read/write/approve/MCP の 4 種類のツール呼び出しを含む）+ `SCOPE_LIB_ENTRIES` |
| `chat-icons.jsx` | インライン SVG アイコン辞書 |
| `chat-sidebar.jsx` | `ChatSidebar`（SessionList、グループ化、検索、新規ボタン、フッタ、空状態） |
| `chat-messages.jsx` | `MessageList`、`UserMessage`、`AssistantMessage`、`MdBody`、`ToolCallCard`（5 kind × 5 state すべて）、`JsonPreview`、`HitSnippet`、`DiffBlock` |
| `chat-composer.jsx` | `SessionHeader`、`Composer`、`ScopePicker` ポップオーバー、`NewSessionDialog` モーダル |
| `chat-main.jsx` | App ルート。Tweaks（テーマ/アクセント/密度/レイアウト/状態確認）と承認状態を mock 駆動する `resolveMessages` を含む |
| `starters/tweaks-panel.jsx` | Tweaks UI（実装時は不要、デザインリファレンス用） |
| `CHAT_UI_BRIEF.md` | 元のブリーフ（v0.2.0） |

実装上は `Tweaks` 関連は **不要** です（プロトタイプの状態切替用）。本番では実状態（streaming / blocking / approval state）から自然に駆動してください。

## 実装の優先順位

1. **MVP**: SessionList + 中央 Conversation + Composer + ToolCallCard（read / write / approve の 3 種 × done_collapsed / done_expanded / needs_approval / running）
2. **次**: delete / mcp 種のカード、ScopePicker、ContextPanel（右パネル）
3. **後回しでも可**: NewSessionDialog、Tweaks（要らない）、SuggestionCard（空状態）、アニメーション微調整

## 既存システムとの整合チェックリスト

- [ ] `--bg` / `--surface` / `--sidebar` / `--border` / `--accent-strong` 等の既存 CSS 変数を共有
- [ ] 4 アクセント（amber / indigo / teal / rose）すべてで色破綻なし
- [ ] light / dark 両テーマで成立
- [ ] 3 密度（compact / default / comfortable）で SessionRow と MessageList が反応
- [ ] 日本語 / 英語ラベル幅の差で SessionHeader が破綻しない
- [ ] Markdown 本文は既存 `MathMarkdown` で置換
- [ ] サイドバーの `⌘ [` トグルが既存ライブラリ画面と同じ動作
