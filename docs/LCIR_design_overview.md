# LCIR 設計概観 — LumenCite Document Intermediate Representation

## この文書の位置づけ

`docs/LumenCite_machine_readable_document_roadmap.md`（別 LLM 生成・汎用的な理想像）を、**LumenCite の実コード規約に接地し、設計判断を確定した「決定版」**に落とす文書。ロードマップが提示する全10フェーズを俯瞰し、着手前の全体合意を取るためのもの。

- ロードマップ = 何を目指すか（vision・汎用）。
- **本書 = LumenCite で実際にどう作るか（接地・確定）。**
- 実装が進むにつれ `docs/DATA_MODEL.md`（新テーブル）・`docs/API_SPEC.md`（新コマンド）へ反映していく。

**LCIR = LumenCite Document Intermediate Representation**。論文全文を「ページ単位のプレーン文字列」ではなく、**型付きノードの木 + PDF 座標 + 出典(provenance) + 信頼度**として保存する内部中間形式。FTS5 は LCIR からの派生索引として位置づけ直す。

---

## 1. なぜやるか

現状の全文パイプライン（`src-tauri/src/db/fulltext.rs`）は、`pdf_extract::extract_text_by_pages` が返す**ページ単位のプレーン文字列**を、単一の FTS5 仮想表 `fulltext(content, attachment_id, page)` に格納しているだけ。全文検索には十分だが、次を一切保持しない。

- 節・段落・定理・証明・数式・図・表などの論理構造
- 数式の構文/意味構造
- PDF 上の位置（bbox）と抽出結果の対応
- OCR・数式認識・構造認識の信頼度
- 抽出器・変換器のバージョン（再処理可能性）
- 図中の軸・系列・ノード・エッジなどの意味情報

LCIR はこれらを失わずに保存する基盤を作る。**最優先は高度な意味理解ではなく、「原資料・位置・構造・由来・信頼度を失わずに保存できる基盤」を先に作ること。** その基盤があれば、数式認識・図解析・記号解決・LLM・知識グラフの技術が今後改善したときにも、既存文献を再処理しながら継続的に進化させられる。

---

## 2. 三層モデルと LumenCite 現状の対応

| 層 | 役割 | LumenCite での実体 |
|----|------|--------------------|
| **原資料層** | PDF・TeX・JATS・HTML・補助ファイル。正本その1 | `attachments`（ファイル本体はアプリデータ dir・DB は相対パスのみ） |
| **正規化文書層（LCIR）** | 構造・数式・図・出典・信頼度を保持。正本その2 | **新規**: `document_versions` / `document_nodes` / `source_fragments` ほか |
| **派生索引層** | 再生成可能な検索用データ | 既存 `fulltext`(FTS5) / 将来 `document_nodes_fts` / ベクトル / 数式索引 |

**原資料層と LCIR を正本とし、FTS5 やベクトル埋め込みは再生成可能な派生データとして扱う。** 既存 `highlights`（PDF ハイライト・PDF ポイント左下原点座標系）と `fulltext` は LCIR と同じ座標系・ページ番号規約を共有する（後述）。

---

## 3. フェーズ → 増分マッピング

ロードマップの10フェーズを、LumenCite の実装増分（≒ PR 群 / Milestone）に対応づける。**依存**は先行して完了が必要なフェーズ、**規模**は S/M/L/XL の目安。

| Phase | 内容 | Milestone | 依存 | 規模 | 状態 |
|-------|------|-----------|------|------|------|
| 0 設計準備 | 境界確定・ADR・0.1 schema・座標系・ID 規則・実験フラグ | 本書 | — | S | 本書で確定 |
| **1 ページ/ブロック/出典** | `document_versions`/`document_nodes`/`source_fragments`・PDF 座標・provenance・派生 FTS 再生成 | **A** | 0 | M | **実装済**（PR #46/#47） |
| **2 論理構造** | 見出し/段落/参考文献/caption 認識・ノード単位 FTS（`document_nodes_fts`） | **B** | 1 | M | **実装済** |
| **3 数式表層** | display math 認識・`math_expressions`（表層）・数式検索文字列 | **C** | 1,2 | L | **実装済**（表層のみ・LaTeX/MathML は Phase 4/7） |
| **4 TeX/JATS/HTML 取込** | arXiv TeX・JATS・複数表現の優先順位・source 切替 | D | 1 | L | **実装済**（arXiv TeX のみ。JATS/HTML/LaTeXML は後続） |
| **5 定理/定義/証明** | theorem-like 環境・proof・型付きノード（定理間参照グラフは Phase 6 の node_relations へ） | **E** | 2 | M | **実装済**（TeX=環境名+`\newtheorem`／PDF=行頭キーワード・信頼度付き） |
| 6 記号/参照グラフ | `symbols`/`symbol_occurrences`・`node_relations`・スコープ | E | 2,3 | L | **実装済**（6a 参照グラフ = `node_relations`／6b 記号系 = `symbols`/`symbol_occurrences`・TeX の定義文認識） |
| 7 数式意味表現 | 数式 AST・Content MathML・OpenMath・α 正規化・部分式検索 | — | 3,6 | XL | 予定 |
| 8 図表機械可読化 | `assets`/`node_assets`・図切出/SVG/OCR・表セル・plot | F | 1,2 | XL | **8a 実装済**（assets 基盤 + 図 crop + figure ノード + caption_of）。**8b 実装済**（TeX tabular セル構造化・`table` ノード）。**8c 実装済**（図の Vision alt text・`node_alt_texts`・opt-in バッチ）。8d（SVG/plot 構造化）予定 |
| **9a エクスポート第一段** | LCIR JSON 書き出し・構造付き Markdown 出力（決定的レンダリング） | — | 1-6 | M | **実装済**（v0.10.0 で出荷済） |
| 9b 標準形式エクスポート | JATS/TEI/HTML+MathML 出力 | — | 7, 9a | M | 予定（post-1.0 可） |
| 10 LLM/エージェント | ノードチャンク・provenance 付き回答・embedding 再生成 | — | 2-8 | L | **10a/10b 実装済**（文脈バンドル `get_node_context`・チャットへの露出と根拠ジャンプ）。10c（embedding）は post-1.0 |

**推奨実装順序**（ロードマップ §11 を LumenCite に合わせて・2026-07-23 改訂）: 1 → 2 → 3(表層) → 4 → **取得整備（クリッパー欠落補完・TeX 一括取得バッチ — Phase 5 が TeX の恩恵を最も受けるため先に取得面を固める・SPEC.md 参照）** → 5 → 6 → **9a(前倒し)** → 8 → 7(意味) → 9b → 10。Content MathML・OpenMath・図の意味解析は重要だが**最初から完全実装を目指さない**。まず原資料・位置・構造・由来を失わない基盤（Phase 1）を作る。

**9a 前倒しと Phase 9 分割の理由（2026-07-23 決定）**: ①エクスポートの中身（`LcirDocument` 派生ビュー・`load_lcir_document`・validation）は Phase 6b 時点で実質完成しており、残作業は書き出し UX と Markdown レンダラのみ（migration 不要・依存追加なし・ヒューリスティックなし＝「誤検出より欠損」を構造的に満たす）。②フラグ OFF で積んだ Phase 4〜6b の成果（原文 LaTeX 数式・定理番号・cite key）を初めて目に見えるユーザー価値に変換できる。③Phase 9 のうち Phase 7（Presentation MathML）に本質依存するのは JATS/TEI/HTML+MathML だけなので、9b に分離すれば二度手間は生じない。`skip_serializing_if` の追加式スキーマにより、Phase 7/8 完了後の拡張は「レンダラの分岐追加」の増分で済む。なお **8 を 7 より先に置く根拠**（従来から・明文化）: 原典 §11 が図表構造化を数式意味より先に置く／Phase 8 の依存（1,2）が Phase 7 の依存（3,6）より浅い／「意味理解より保存基盤先行」原則の下で Phase 8 の中核は『保存』・Phase 7 の中核は『意味』である。

**リリースとの対応（2026-07-19 決定・2026-07-23 改訂・詳細は SPEC.md「v0.8.0 > リリース方針」）**: v0.8.0 = 取得整備と同時（Phase 5 前）。以後はフラグ付きで main に積み、リリースは 2〜3 フェーズごとに間引く。**Phase 9a/10 到達 + `lcir.enabled` 既定 ON 化 = v1.0.0 の看板**（9b は post-1.0 可）。

**残 Phase の詳細な棚卸しは `docs/LCIR_REMAINING_PHASES.md`**（2026-07-28・v0.10.0 出荷時点）。分割（7a/7b/7c・8d-1〜8d-8・9b-0〜9b-4・10a/10b/10c）ごとの難易度・概算行数・migration 要否・実測値・積み残し債務・推奨着手順序をそこに置く。**`lcir.enabled` 既定 ON には pdfium の Windows/Linux 同梱が hard blocker** だった（LCIR 抽出は OCR と同じ `bind_pdfium()` を通るため）— **2026-07-28 に対応済**（`docs/RELEASE.md` §4）。

### 3.1 実装史（マージ記録）

各増分がどの PR で入り、どの migration と抽出器版を伴ったか。**「この挙動はいつからか」「この表はどの PR で
増えたか」を 1 か所で引くための索引**で、設計の理由は各 Phase の「実装状況」節、残作業は
`LCIR_REMAINING_PHASES.md` にある。squash ハッシュは `git show <hash>` でそのまま辿れる。

抽出器版の意味と各版の変更点は `src-tauri/src/document_ir/schema.rs` の `EXTRACTOR_VERSION` /
`TEX_EXTRACTOR_VERSION` の doc コメントが正本（**版を上げても再構築は起きない** — `LCIR_REMAINING_PHASES.md` §2.1）。

| 増分 | PR | squash | 日付 | migration | 抽出器版 |
|---|---|---|---|---|---|
| Phase 0+1 基盤（pdfium 座標付き・実験フラグ） | #46 | `ec5fcdd` | 2026-07-16 | **0014**（3 表） | pdfium `0.1.0` |
| 未構築 PDF の一括バックフィル + 設定トグル | #47 | `308f6d0` | 2026-07-16 | — | — |
| Phase 2 論理構造 + ノード単位 FTS | #48 | `0e3b2af` | 2026-07-16 | **0015** | → `0.2.0` |
| Phase 3 数式表層 | #49 | `12862f9` | 2026-07-18 | **0016** | → `0.3.0` |
| Phase 3.5 LCIR を MCP に出す read ツール | #50 | `675df39` | 2026-07-18 | — | — |
| Phase 4 arXiv TeX 取込 + source 切替 | #51 | `a11ea8c` | 2026-07-19 | — | tex `0.1.0` 新設 |
| arXiv クリップ/追加時の TeX 自動取得 | #52 | `7c85048` | 2026-07-19 | — | — |
| Phase 5 定理・定義・証明 | #54 | `2731413` | 2026-07-22 | — | pdfium → `0.4.0` / tex → `0.2.0` |
| Phase 6a 参照グラフ | #55 | `b38d890` | 2026-07-22 | **0017** | pdfium → `0.5.0` / tex → `0.3.0` |
| Phase 6b 記号系 | #56 | `343d767` | 2026-07-22 | **0018**（2 表） | tex → `0.4.0` |
| Phase 9a エクスポート第一段 | #57 | `3881618` | 2026-07-23 | — | — |
| Phase 8a 図表アセット基盤 | #58 | `fd75d13` | 2026-07-23 | **0019**（2 表） | pdfium → `0.6.0` |
| Phase 8b TeX tabular のセル構造化 | #59 | `1b2dac4` | 2026-07-24 | — | tex → `0.5.0` |
| Phase 8c 図の Vision alt text | #60 | `5ae8a62` | 2026-07-26 | **0020** | 不変（生成が build の外） |
| 再構築 UI（一括再構築 + 添付ごとビルド） | #61 | `ca295ab` | 2026-07-27 | — | — |
| alt text 生成の絞り込み | #62 | `18b01a2` | 2026-07-27 | — | — |
| alt text 一括生成のヘッドレスハーネス | #63 | `7a2f245` | 2026-07-27 | — | — |
| pdfium を Windows / Linux にも同梱（v1.0.0-p0） | #66 | `1d52e78` | 2026-07-28 | — | — |
| 8d-7 図表参照 + debt-6/7/8 | #67 | `48afa6f` | 2026-07-28 | — | → `0.7.0` |
| Phase 10a 文脈バンドル（`get_node_context`） | #68 | `c6073e0` | 2026-07-28 | — | — |
| Phase 10b チャットへの LCIR 露出 + provenance | #69 | `37e24fa` | 2026-07-29 | — | — |
| v1.0.0 #0 GC 猶予 + GUI ロック | #72 | `242be07` | 2026-08-02 | — | — |
| v1.0.0 #1 debt-12 ローマ数字 caption | #73 | `4fedd39` | 2026-08-02 | — | → `0.8.0` |
| v1.0.0 #2 debt-14 図領域クランプ | #74 | `c2c06b7` | 2026-08-02 | — | → `0.9.0` |
| v1.0.0 #2.5 debt-18 走り柱の帯 | #76 | `305739f` | 2026-08-02 | — | → `0.10.0` |
| v1.0.0 #3 8d-8 XObjectForm 内の画像 | #78 | — | 2026-08-03 | — | → `0.11.0` |

**migration は 0014〜0020 の 7 本**（すべて新規テーブルの追加で、既存表の破壊的変更なし）。
v1.0.0 の残スコープに migration は無い。

**出荷**: Phase 0-4 = v0.8.0（2026-07-22）／Phase 5・6 = v0.9.0（2026-07-23）／
Phase 9a・8a/8b/8c・再構築 UI = v0.10.0（2026-07-28）。**`lcir.*` は全て既定 OFF**で出しており、
既定 ON 化（+ Phase 10 到達）が v1.0.0 の看板。

**実装の進め方の学び**（Phase 4 以降で定着）: 多段の並列調査 → **実装前**の敵対的批評 → 実装 →
diff レビュー + 敵対的検証。批評が実装前に blocker を捕まえた例（Phase 4 のバックスラッシュ偶奇パリティ
`\\[4pt]` ≠ `\[`・main 検出の standalone/subfiles 罠）と、レビューが実装後に捕まえた例
（frontend の stale closure・マルチバイト panic）の両方がある。**実データで測ってから決める**のも同じ流れで、
Phase 6b は実データ検証で 57 → 38 記号に精度が上がった。

---

## 4. 設計判断（ADR） — ロードマップ §17「重要な判断事項」への回答

着手前に確定した10論点。以降の実装はこの決定に従う。

| # | 論点 | 決定 | 理由 |
|---|------|------|------|
| 1 | LCIR 主ストレージ（正規化テーブル vs JSON blob） | **正規化テーブル + `payload_json`/`metadata_json` 逃がし列** | 既存 DB は 100% 正規化 sqlx。JSON blob を主状態にする表は存在しない。JSON 列は未モデル化の型固有属性用に残し、後続フェーズで再 migration 不要にする |
| 2 | ノード ID（UUID vs 内容由来安定 ID） | **INTEGER PK + 派生 `content_key TEXT`** | `uuid` は非依存。全表 `INTEGER PRIMARY KEY` / `last_insert_rowid()` / `i64` / FK 規約を維持。「同一 PDF → 同一 version」の再現性は row id ではなく `content_key` で満たす（`doi_canonical` の canonical 列前例と同型） |
| 3 | バイナリアセット（BLOB vs FS） | **ファイルシステム + 相対パス**（Phase 8 で使用・第一段は未使用） | `attachments` の既存前例（BLOB 不使用・DB は相対パス + SHA-256 参照） |
| 4 | PDF 座標系の統一規則 | **PDF user space・左下原点・y 上・単位 pt・rotation = ページ `/Rotate` 度** | pdfium ネイティブ空間。無損失。既存 `highlights` と一致し PDF ビューアがそのまま消費できる |
| 5 | version 差分管理 vs 完全スナップショット | **完全スナップショット** | 単純・再現可能。差分マージの複雑さを持ち込まない |
| 6 | 抽出ジョブのキュー実装 | **当面はキューを作らない**（`spawn` + `spawn_blocking`。第一段はフラグ ON 時に添付後 background build） | 既存に耐久ジョブキューは無く（近いのは debounce mpsc `run_sync_task`）、実験段階で新機構を持ち込まない。Phase 8+ で必要になれば導入 |
| 7 | ユーザー修正と再抽出のマージ | **上書きせず新しい provenance として保存**（第一段は未実装・seam のみ） | ロードマップ 4.3/Phase 7 の原則。`parent_version_id` + `origin='user_edited'` で表現する余地を残す |
| 8 | TeX/JATS/PDF の対応付け粒度 | **抽出器ごとに別 `document_version` として併存**（Phase 4） | 一本化せず由来の異なる表現を残す。`extractor_name` + `content_key` が識別子 |
| 9 | Rust 型 vs JSON Schema の一次仕様 | **Rust 型（`document_ir/`）を一次仕様**。JSON Schema/JSON は export・テスト・交換用の派生 | 既存は serde 構造体が単一ソース。`sqlx::FromRow` と共用できる |
| 10 | LCIR 公開仕様化の時期 | **当面は内部仕様**。Phase 9（外部エクスポート）到達後に公開を検討 | まず内部で安定させる。`schema_version` は最初から持たせ将来公開に備える |
| — | 実験フラグ | **settings `lcir.enabled`（"1" 規約）** | `mcp_server.enabled` / `clipper.enabled` の既存前例。Cargo feature は存在しない。OFF で既存挙動 byte-for-byte 不変 |
| — | 抽出器（座標問題） | **pdfium-render を LCIR 抽出器に採用し最初から bbox 取得**（ユーザー選択） | pdfium は既に依存（OCR で使用中）。座標が無い `pdf-extract` では Phase 1 完了条件「検索ヒット → PDF 領域ハイライト」に到達不能。pdfium の text bounds は `highlights` と同じ空間に直行する |

---

## 5. LCIR データモデル

### 5.1 保存戦略

- **主ストレージ = SQLite 正規化テーブル。** アプリが join / フィルタする属性（kind・ordinal・parent・page・座標・provenance・status）は実カラム。
- **`payload_json` / `metadata_json` = 逃がし列。** 型固有・未モデル化の属性（節番号・数式番号・スタイル・座標系記述子など）は JSON で持ち、後続フェーズのスキーマ変更を避ける。
- **LCIR JSON = 派生。** デバッグ・エクスポート・テスト・交換のために SQLite から JSON を生成できるようにするが、正本は SQLite。

### 5.2 第一段（Milestone A / migration `0014`）で作る3テーブル

残り6テーブルは後続フェーズの `0015+` で追加する。FK 先の `document_versions` / `document_nodes` は 0014 で先に用意されるため、後続追加は無改変で載る。

#### `document_versions` — 添付ごとの抽出/変換結果 1 回分

provenance と再現性の正本。1 添付に複数バージョン（再抽出・別抽出器）が併存しうる。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `attachment_id` | INTEGER FK → attachments | ON DELETE CASCADE |
| `content_key` | TEXT NOT NULL | `sha256(source_sha256 \| extractor_name \| extractor_version \| config_hash)`。**再現可能な内容由来 ID**（row id は SQLite 採番で再現不能なため）。起動時 best-effort UNIQUE |
| `schema_version` | TEXT NOT NULL | `document_ir::SCHEMA_VERSION`（例 `0.1.0`） |
| `source_sha256` | TEXT NOT NULL | 原ファイル本体の SHA-256。`attachments` に列が無く抽出時に計算 |
| `source_mime_type` | TEXT NOT NULL | `application/pdf` 等 |
| `extractor_name` | TEXT NOT NULL | `lumencite-pdfium`（PDF）/ `lumencite-tex`（arXiv TeX・Phase 4 で併存を実証）。将来 JATS/HTML 抽出器も別名で併存 |
| `extractor_version` | TEXT NOT NULL | **抽出ロジックの semver（手動 const）**。supersede 判定基準。pdfium クレート版とは別 |
| `config_hash` | TEXT NOT NULL DEFAULT '' | 抽出設定のハッシュ（既定設定は空） |
| `parent_version_id` | INTEGER FK → document_versions | supersede チェーン（同一添付内の再抽出。source 切替は別添付の版併存 + read 優先順位で実現し、このチェーンは使わない） |
| `extraction_status` | TEXT NOT NULL | `pending`/`processing`/`completed`/`completed_with_warnings`/`failed`/`superseded` |
| `warnings_json` | TEXT | 抽出失敗・警告ログ（Phase 1 完了条件） |
| `metadata_json` | TEXT | 座標系記述子・ページ数・pdfium/クレート版・計測値 |
| `created_at` | TEXT NOT NULL | `datetime('now')` |

#### `document_nodes` — 文書の型付きノード木

第一段のノード型: `document` / `page` / `text_block` / `line` / `unknown_block`。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `document_version_id` | INTEGER FK → document_versions | ON DELETE CASCADE |
| `parent_id` | INTEGER FK → document_nodes | ON DELETE CASCADE。ルートは NULL |
| `node_kind` | TEXT NOT NULL | `NodeKind` の snake_case。未知は `unknown_block` |
| `ordinal` | INTEGER NOT NULL | 同一親内の読み順 |
| `plain_text` | TEXT | `page` ノードはページ全文（= FTS 再生成元） |
| `language` | TEXT | 言語コード（任意） |
| `confidence` | REAL | 構造認識信頼度（0–1・任意） |
| `origin` | TEXT | `Origin`（`pdf_text_layer` 等） |
| `payload_json` | TEXT | 型固有（`page_width_pt`/`page_height_pt`/`rotation_deg` 等） |
| `created_at` | TEXT NOT NULL | `datetime('now')` |

#### `source_fragments` — ノード ↔ PDF 領域

座標は `highlights` と同一系（PDF user space・左下原点・pt）。1 段落/証明が複数ページ・複数領域にまたがる場合は複数行を持つ。

| カラム | 型 | 備考 |
|--------|-----|------|
| `id` | INTEGER PK | AUTOINCREMENT |
| `node_id` | INTEGER FK → document_nodes | ON DELETE CASCADE |
| `page_number` | INTEGER NOT NULL | 1 始まり（`fulltext.page` / `highlights.page` と同じ） |
| `x` / `y` / `width` / `height` | REAL NOT NULL | バウンディング（PDF pt・左下原点） |
| `rotation` | REAL NOT NULL DEFAULT 0 | ページ `/Rotate`（0/90/180/270） |
| `reading_order` | INTEGER | 読み順（任意） |
| `fragment_type` | TEXT | `page` / `text_block` / `line` |

**ロードマップ DDL からの適応**: TEXT-UUID PK → `INTEGER PK AUTOINCREMENT`／`content_key`・`config_hash`・`warnings_json` を追加／`datetime('now')` 既定／全子 FK に `ON DELETE CASCADE`（実表なので FK 可能。`fulltext` の手動クリーンアップより堅牢で、添付削除で LCIR 木ごとカスケード消去される）。

### 5.3 後続フェーズで追加するテーブル（forward sketch）

| テーブル | 内容 | Phase | migration |
|----------|------|-------|-----------|
| `math_expressions` | 数式の複数表現（LaTeX/Presentation MathML/Content MathML/OpenMath/AST/正規化文字列/`semantic_status`/信頼度）。**Phase 3 で migration 0016 として実装済**（PDF 由来は `normalized_text` + `semantic_status='surface_only'` のみ・LaTeX/MathML/AST は後続） | 3/7 | **0016** |
| `assets` | 図・画像・SVG・表データ（SHA-256 + 相対パス参照）。**Phase 8a で migration 0019 として実装済**（ページ crop PNG・`attachments/<entry_id>/.lcir/` 配下・`size_bytes` 付き） | 8 | **0019** |
| `node_assets` | ノード ↔ アセット（`role`: original/page_crop/vector/thumbnail/ocr_source/plot_data/…）。**Phase 8a で 0019 として実装済**（8a は `page_crop` のみ） | 8 | **0019** |
| `node_relations` | ノード間の型付き関係（cites/refers_to_equation/refers_to_theorem/proves/…）。**Phase 6a で migration 0017 として実装済**（参照グラフ・`\ref`/`\eqref`/`\cite` と番号一致で解決・origin+confidence 付き） | 6a | **0017** |
| `symbols` | 記号定義（surface_form/normalized_form/description/symbol_type/scope/semantic_json）。**Phase 6b で migration 0018 として実装済**（TeX 定義文からインライン数式を抽出・origin=tex_source・confidence 付き・TeX のみ） | 6b | **0018** |
| `symbol_occurrences` | 数式中の記号出現 → 定義への関連付け。**Phase 6b で 0018 として実装済**（display 数式内の表層一致・保守的） | 6b | **0018** |
| `node_alt_texts` | 図の代替テキスト（LLM Vision 生成・`origin='llm_inference'` + `confidence` + `model` + `source_asset_sha256` + `carried_from_version_id`）。**Phase 8c で migration 0020 として実装済**（`figure` ノードの satellite 表・PDF のみ・build 外の opt-in バッチで生成） | 8c | **0020** |

### 5.4 ノード型の全体像（フェーズ別）

| Phase | 追加ノード型 |
|-------|-------------|
| 1 | `document` `page` `text_block` `line` `unknown_block` |
| 2 | `abstract` `front_matter` `section` `subsection` `heading` `paragraph` `list` `list_item` `figure_caption` `table_caption` `footnote` `citation` `bibliography` `bibliography_entry` `code_block` |
| 3 | `inline_math` `display_math` `equation_group` |
| 5 | `definition` `theorem` `lemma` `proposition` `corollary` `remark` `example` `proof` |
| 8 | `figure`（8a で実装）・`table`（8b で実装 — TeX tabular のセル構造化） |

`node_kind` は制約なし TEXT + `NodeKind` enum（`UnknownBlock`/`from_db` フォールバック付き）。後続フェーズの型追加は enum の variant 追加のみで migration 不要。**認識に確信が持てないブロックは、誤った型を確定するより `unknown_block` + 信頼度で残す。**

**Phase 2 実装状況（`ingestion/structure.rs`・pdfium 非依存の純関数で CI テスト可能）**: セグメント→行→ブロックにまとめ、`section`/`subsection`/`heading`/`paragraph`/`abstract`/`figure_caption`/`table_caption`/`bibliography`/`bibliography_entry`/`unknown_block` を確信度付きで出す。番号付き節・caption はパターン、abstract/参考文献は「見出し→本文」の状態機械で認識する。ランニングヘッダ/ページ番号（`104 A. Suzuki`・`123`）や記号主体の display 数式は、誤って見出しにせず `unknown_block`/`paragraph` に留めるガードを入れた（`front_matter`/`list`/`list_item`/`footnote`/`citation`/`code_block` は enum 済・認識は後続で拡充）。tree は `document > page > block > line`。**caption の番号は算用数字に加えてローマ数字も取る**（2026-08-02・debt-12・`lumencite-pdfium` 0.7.0→**0.8.0**）: 規則は「**全大文字ラベル + 標準形ローマ数字 + 終端記号（`.` / `:`）**」で、"TABLE III. …" を `table_caption` にする（実測 48 ブロック / 12 版・`table_caption` 68→116）。番号は算用数字に正規化せず `"III"` のまま payload に載せる（参照側 `graph::take_ref_number` は ASCII 数字しか読まないので照合には使われない）。ガードが 3 つあるのは、**終端記号だけでは本文と分離できない**ため — "… as shown in Table III. Since the extended …" のように文末のピリオドが次文の直前に来る型が実在する（分離しているのはラベルの全大文字性）。標準形判定（値に読んで正準表記へ描き直す）はローマ数字の文字だけでできた英単語（"DIM"）を弾く。詳細は `LCIR_REMAINING_PHASES.md` §2.9 / §8.1。

**走り柱（ランニングヘッダ/フッタ）の帯は「ページ境界 box の原点 + 高さの 10%/90%」**（抽出器版 `0.10.0`・debt-18）。ページ寸法だけで測っていた頃は、原点が非ゼロの PDF で帯が下へずれ、本文の短い行が `unknown_block` に降格し（実測 107 件 / 6 版）、逆に box 下端すぐ上の走り柱が `paragraph` として残っていた（同 12 件 / 2 版）。**帯はブロックが丸ごと帯に入ることを要求する**（上端帯は下端 `y`、下端帯は上端 `y+h` を見る）ので、跨いでいるだけの行は降格しない。回転ページでは `page.height()` が box の幅を返すため帯は依然ずれるが、実ライブラリの回転頁は 5 頁とも box 原点が (0,0) で降格の実例は 0 件（debt-9 の領分・post-1.0）。

**Phase 3 実装状況（数式表層）**: 独立した数式ブロックを検出して `display_math` にし（`ingestion/structure::detect_display_math`）、`math_expressions`（migration 0016）に表層表現を 1 行作る。検出は強い数式記号（`= − ∈ ∞ ≤ →` 等の**タイポグラフィ記号**で ASCII ハイフン/`x` と区別）+ 短いブロック + 散文優位でないこと、で保守的に判定（演算子が飛んだ純英字の式は拾わない＝欠損を許容）。数式番号 `(2.1)` を抽出し、pdfium の制御文字グリフ化けを除去する。**PDF からは LaTeX/MathML を確実に復元できないので `semantic_status='surface_only'` + `normalized_text`（Unicode 線形）のみ**。本物の LaTeX は Phase 4（TeX 取込）、Content MathML/OpenMath/AST は Phase 7（意味）。`inline_math`（本文中の数式スパン）・`equation_group` は enum 済・認識は後続。実 PDF（Suzuki 2016）で display_math 93 件・数式番号抽出・制御文字除去を確認。

**Phase 4 実装状況（TeX 取込・`ingestion/tex/`・純関数で CI テスト可能）**: arXiv e-print（gzip された tar か単一 .tex）を `download_arxiv_source` で `application/gzip` 添付として保存し、`build_lcir_for_attachment` が **mime だけ**で抽出器を選ぶ（`%pdf%` → pdfium / `application/gzip` → **`lumencite-tex`**・独自 semver。バッチ対象クエリと同一述語・手動 .tex 添付はスコープ外）。コンテナは**メモリ内でのみ展開**（`.tex`/`.bbl`/`.ltx` だけ読み、展開合計 64 MiB 等の上限で decompression bomb とパストラバーサルを構造的に排除。非 UTF-8 は latin-1 として救済）。**字句規則**: `\[` `\]` `$` `$$` `%` `{` `}` は直前の連続バックスラッシュが偶数個の位置でだけトークンと認識する（`\\[4pt]`（改行+間隔）を display 数式 `\[` と誤認しない・`\%` 保護・`\\%` はコメント開始）。main ファイルはコメント除去後に `\documentclass`/`\documentstyle` で検出し、`standalone`/`subfiles` クラスと他ファイルから `\input` されるものを除外して選ぶ（候補ゼロなら最大の TeX らしいファイルへ degrade + warning — 旧 hep-th の plain TeX 対応）。`\input`/`\include`/`\subfile`（braceless `\input file` 含む）を include-once + 総量上限で再帰スプライスし、`\bibliography{..}` は同梱 `.bbl` へ差し替える。認識は `\title`（preamble でも本文でも可 — revtex は `\begin{document}` 後に置く）→ `front_matter`、`abstract` 環境と `\abstract{..}` コマンド形（jheppub）、`\(sub)*section`（**共有引数リーダ**が `[short]` 光学引数を消費・節番号は LaTeX カウンタを再現・`*` 付きは番号なし・**`\appendix` 後は A/B..**）、display 数式環境（`equation`/`align`/`alignat`/`flalign`/`gather`/`multline`/`eqnarray`/`displaymath`/`\[..\]`/`$$..$$` + preamble の自明な `\newcommand`/`\def` エイリアス（`\be`/`\ee` 等）— **原文スニペットをそのまま `math_expressions.latex` に保存**し `semantic_status='source_provided'`・`origin='tex_source'`。`\tag{X}` → `equation_label`・`\label` 名は payload の `labels`）、`figure`/`table` 内 `\caption`、`itemize`/`enumerate` → `list`、`verbatim`/`lstlisting` → `code_block`（内部は認識しない）、`thebibliography`（`{widest}` 引数消費・`\bibitem[..]{key}` の光学引数対応）→ `bibliography_entry`（payload に `cite_key`）。**未知環境の三分法**: 透過（`center`/`widetext`/`subequations`/`acknowledgments`/`quote` 等 + 既定の未知環境はマーカー除去して中身を解析）/ 本体破棄（`tikzpicture`/`tabular`/figure 内の非 caption 等）/ opaque（verbatim 系）。段落分割はコメント専用行を完全削除してから空行で区切り、brace 深度 > 0 では区切らない。木は `document > block` フラットで **page/line ノードと source_fragments を作らない**（TeX に PDF 座標は無い。read 面の `page`/`bbox` は null・派生 JSON の `coordinate_space` も省略）。**TeX 版は `document_nodes_fts`/`fulltext` に索引しない**（同一エントリの PDF 版と重複ヒットし bbox も無いため。検索 = PDF 版 / 読み出し = TeX 優先の分担）。read 面はエントリ解決時に `extractor_priority`（tex > pdfium）で優先し、MCP ツールの `source` 引数で切替・`available_sources` で列挙できる。`page` フィルタ指定時は PDF 版へ自動フォールバック（page は PDF 空間の概念）。**LaTeX 数式番号の完全エミュレーションはしない**（`\tag` のみ。誤った番号を確定するより欠番を許容）。インライン数式 `$..$` は本文に生 LaTeX のまま残す（独立ノード化は後続）。JATS/HTML/LaTeXML は取得経路が無いため後続（抽出器 seam は 2 抽出器の併存で実証済み）。

**Phase 5 実装状況（定理・定義・証明）**: 型付きノード `definition`/`theorem`/`lemma`/`proposition`/`corollary`/`remark`/`example`/`proof` を 2 経路で認識する（**新規テーブルなし** — 既存 `document_nodes` + `payload_json` に載る）。**TeX（`lumencite-tex` 0.2.0・原文由来・高信頼 0.95）**: preamble の `\newtheorem{env}{Display}` を回収して独自環境名・略記（`thm`/`lem`…）を表示名からノード種別に対応づけ（`\newtheorem*`・共有カウンタ `[shared]`・`{Display}[within]` 形も対応）、標準英名 + `proof`（amsthm 予約）は既定マップで拾う。`\begin{theorem}[note]` の付記名と `\label` を捕捉し、本文は 1 ブロックに collapse（`\label` は除去・内側 display 数式は生 LaTeX のまま残し別ノード化しない＝flat 統計を保つ）。**PDF（`lumencite-pdfium` 0.4.0・レイアウト由来・中信頼 0.6–0.7）**: 行頭キーワード + 番号 + 終端記号（`. : (` ダッシュ）で判定し、参照文中の "Theorem 2 shows …"（終端記号が続かない）は棄却する（誤検出より欠損）。`theorem_number`（"2.3"/"A.1"）と丸括弧の付記名を payload に載せ、参考文献モードでは検出しない。**定理間参照グラフ（proves 等）は Phase 6（`node_relations`）に委譲**し、Phase 5 は型付きノード + メタデータ（番号・付記名・label）までを担う。read 面は汎用（`is_content_block` は blacklist・node-FTS の索引対象・`get_document_blocks` の `kinds` フィルタ）なので追加改修なしで surface し、`get_document_blocks(kinds:["theorem","proof"])` で「定理と証明を一問い合わせ」が満たされる（番号・付記名も応答に付く）。

**Phase 6a 実装状況（参照グラフ・`ingestion/graph.rs`・DB 非依存の純関数）**: ノード間の参照を `node_relations`（migration 0017）に**有向辺**として張る（**新規ノード型なし** — 既存ノード間の辺）。build のトランザクション内で、Phase 5 までに永続化されたノードの軽量ビュー（`GraphNode`）から `resolve_relations` が解決する。**TeX**（`RefStrategy::Tex`・`origin='tex_source'`・confidence 0.9）: 段落等の `plain_text` に原文のまま残る `\ref`/`\eqref`/`\cite`（+ `\autoref`/`\cref` 系・biblatex cite 系）を、`\label`（`payload.labels`）/ `\bibitem` の cite key（`payload.cite_key`）と照合。参照先ノードの種別で `refers_to_equation`/`refers_to_theorem`/`refers_to_figure`/`refers_to_table`/`refers_to_section`/`refers_to` を張り分け、`\cite` は `cites`。**PDF**（`RefStrategy::Pdf`・`origin='layout_model'`・confidence 0.6）: `plain_text` 中の "Theorem 2.3" / "Eq. (2.1)" を定理番号（`payload.theorem_number`）/ 数式番号（`math_expressions.equation_label`）と照合（PDF は `\label` を復元できないため番号一致・大文字始まりのみ拾い plural/小文字は保守的に無視）。**proof → theorem の `proves`**: TeX は `\ref` 先が定理系ならそれ、無ければ読み順（ページ跨ぎの通し番号）の直前の定理系ノード。PDF は "Proof of Theorem 2.3" の番号一致（confidence 0.7）、無ければ直前。**解決できない参照（ターゲット不在）・自己参照（定理見出しが自分を指す）は張らない**（roadmap §16「誤検出より欠損」）。read 面は `LcirDocument` に文書レベルの `relations` を載せ（`get_lcir_document` / MCP）、MCP `get_node_relations`（`source` 切替・`relation_type`/`node_id` フィルタ・端点 enrich）で「この証明は何を証明するか」「式 (2.1) を参照/使用するのは何か」を一問い合わせで解ける。**記号系（記号候補抽出・"let/define/denote" 定義文認識・スコープ・記号出現）は Phase 6b（`symbols`/`symbol_occurrences`・別 migration）に分けた**（誤検出が多い領域を高精度な参照グラフと切り離す）。

**Phase 6b 実装状況（記号系・`ingestion/symbols.rs`・DB 非依存の純関数）**: 論文が定義する記号を `symbols`（migration 0018）に、その出現を `symbol_occurrences` に持つ（**新規ノード型なし**）。build のトランザクション内で `extract_symbols` が、**TeX 本文のインライン数式 `$...$` / `\(...\)`** を定義文から取り出す。定義文パターン（**強いトリガ + インライン数式が揃ったときだけ**）: `let $X$ be/denote ...`、`(we) define $X$ as/to be/by ...`、`denote by $X$ ...`、`we write $X$ for ...`、`$X$ denotes/is defined as/is called/stands for ...`、`$X := ...$`（無条件）、トリガ + `$X = ...$`。表層は先頭記号にそろえる（`$U_\beta = U_\beta(G,a)$` → `U_\beta`）。説明は文末/display 数式/長さで切り、**インライン数式は説明に含める**（`$\tau$-periodic Grover walk` 等・LaTeX 読者向け）。`symbol_type`（operator/matrix/graph/…）は説明語からの best-effort、`normalized_form` は `\mathcal`/`\hat` 等の装飾を剥いた形、`scope_node_id` は直前の節、**同一節内の同一表層の再定義は 1 個に畳む**。出現は保守的に **display 数式内の定義済み記号の表層一致**のみ（英字境界トークン一致・conf 0.5）。**PDF は対象外**（インライン数式が区切り無しで潰れる・PDF-only エントリは空）。surface/description は原文 verbatim だが対応づけはヒューリスティックなので `confidence` 中程度（0.5–0.6）・`origin='tex_source'`。read 面は `LcirDocument.symbols`（出現つき）+ MCP `get_symbol_definitions`（`symbol`/`query` フィルタ・`defined_at`/`scope`/`occurrences` を enrich）。**実 arXiv 論文（2607.14797）で 38 記号 / 31 出現**を確認（"$U_\beta$"・"the magnetic vector potential"→vector 等）。**スコープの厳密化・意味の別テーブル化・PDF 記号は後続**（Phase 7 の数式意味と接続）。

**Phase 8a 実装状況（図表アセット基盤・`ingestion/figures.rs`・migration 0019）**: PDF 版のみ（`lumencite-pdfium` 0.5.0→**0.6.0**・TeX 不変）。ページ内の**トップレベル Image オブジェクト**（抽出器版 `0.11.0` = 8d-8 以降は **XObjectForm 内の Image も**・後述）の bbox（`bounds().to_rect()`・PDF user space）を純関数 `figures::merge_image_regions` が近接マージ（短辺 16pt 未満/ページ面積 90% 超を除外・ギャップ 12pt 以内を union・面積上位 8 個/ページ・生矩形 256 超のページはスキップ + warning）して図領域とし、`figure` ノード（page 子・`origin='layout_model'`・conf 0.6・plain_text 無し・payload `{figure_index, figure_number?}`）+ 領域 bbox の fragment を作る。各領域はページ全体レンダリング（幅 1600px・`clip()` はビットマップを縮めないため不使用）から `figures::region_to_pixel_rect`（**ページ境界 box 原点補正** + y 反転 + スケール・純関数）で crop した PNG を `attachments/<entry_id>/.lcir/<attachment_id>/<content_key16>/` に **tmp+rename の原子的書き込み**で保存し、`assets`/`node_assets`（role=`page_crop`・`size_bytes` 付き）で参照する。同一ページの figure caption（`caption_label`/`caption_number` を payload に追加抽出・"Algorithm"/"Listing" は除外）と**相互最近**の幾何ペアリングで `caption_of` 辺（conf 0.6）を張る。ライフサイクル: build 失敗時はファイル best-effort 削除・成功 commit 後に旧 content_key ディレクトリを trash（GC）・reuse 経路はファイル欠損を検知したら再抽出で自己修復・添付削除時に `.lcir/<attachment_id>/` を trash。**（2026-08-03・8d-8 で更新）XObjectForm 内の Image も追う**（抽出器版 `0.11.0`）。子の `bounds()` は **form のコンテンツ空間**で返るので、そのままページ座標として扱うと誤配置 crop になる。座標空間は仮説で決めず form ごとに**自己校正**する — 子矩形に (a) 恒等 (b) 合成行列（入れ子 form は内側 → 外側の順に合成・深さ上限 8）を当てた 2 通りを作り、form 自身の `bounds()` への**面積包含率**が高い方を採り、**どちらも 0.9 未満ならその form の画像は捨てる**（誤配置 crop より欠損）。同率は form ローカル解釈に倒す。実測（生存 138 版 7,345 頁・form 内 Image 109 枚）では form 単位の結論が **`FormLocal` 43 個 / `PageSpace` 0 個 / 棄却 0 個**だった（子 1 枚単位では 96 枚が合成行列でだけ収まり、13 枚はどちらでも収まる ＝ 大きな form は何でも含むので**子 1 枚の包含率は証拠として弱く、form 単位で面積加重する必要がある**）。form を辿るのは**回転ページでも画像過多ページでもないページだけ**なので、それらのページの出力と warning は 8a 当時から変わらない。**tikz/pgf ベクター図はアセット 0 件が正当**は **8d-2（v1.0.0 スコープ）で撤回する** — 実測で pdfium 生存版 138 件中 69 件（50%）が figure ノード 0 件、未結合の図 caption が 632 件ある。回転ページは**引き続きスキップ + warning**（debt-9・post-1.0。実ライブラリの回転頁 5 頁には画像が 1 枚も無く、有効にしても増える図が 0 件のため）。母集団の内訳（form 内 Image あり 6 版 / 画像はあるが図にならない 21 版 / 純ベクター 15 版）と誤検出ガードは `docs/LCIR_REMAINING_PHASES.md` §4.1・§2.12。read 面は `LcirNode.assets` + MCP `get_figures`（`relative_path` は存在保証なしのメタデータ参照・base64 なし）。表（`table` ノード）は 8b（TeX tabular のセル構造化）で扱う。

**Phase 8b 実装状況（表セル構造化・`ingestion/tex/tabular.rs`・migration 不要）**: **TeX 版のみ**（`lumencite-tex` 0.4.0→**0.5.0**・pdfium 不変）。table float（`table`/`table*`/`sidewaystable`）内および裸の `tabular`/`tabular*`/`tabularx` を純関数 `tabular::parse_tabular` がセル構造化し、`table` ノード（`origin='tex_source'`・conf 0.9 / spec 未検証 0.8・text = セルを " \| " 結合した可読形）の `payload_json` に `{column_spec(verbatim), n_columns, n_rows, alignments?(列型レター・spec 検証時のみ), rows:[{cells:[{text, colspan?, rowspan?}], rule_above?}], latex_source?(40k 以下)}` を載せる。字句は親モジュールのパリティ規則を共有し、`&`/`\\` は brace depth 0 かつ opaque 区間（`$..$`/`\(..\)`/`\verb`/`\url` 系）の外だけ構造と見なす。`\multicolumn`→colspan（非整数 n は表ごと skip）・whole-cell `\multirow`→rowspan（情報のみ・grid 再解釈なし）・全幅罫線（`\hline`/booktabs）→`rule_above`（`\cline`/`\cmidrule` は消費のみ — 部分的事実を全列に昇格しない。ヘッダ推定はしない）。**表ごと skip（誤検出より欠損）**: ネスト環境・brace 不均衡・未終端 opaque・spec 検証済みでの列数超過・行512/列64/スニペット100k超・subtable/subfloat 混在・verbatim（`lstlisting` 等）内の例示 tabular・`longtable`（独自プロトコル）・`tabu`（deprecated）。caption と同一環境由来であることを `env_group` で結び `caption_of` 辺（caption→table・conf 0.95・**原文由来として初の TeX 側 caption_of**）。labels は caption 側に維持（graph 6a 不変）・caption 無し環境のみ table 側 + `relation_type_for_target` に Table→refers_to_table。セル内 `\cite`/`\ref` は table ノードを出典とする辺になる（原文由来・意図した挙動）。read 面 = MCP `get_tables`（TeX 固定・caption/rows/alignments・`latex_source` は返さない・max_chars 予算）+ `get_document_blocks` に寸法（`column_spec`/`n_columns`/`n_rows`）+ Markdown エクスポートは GFM パイプテーブル（全行を n_columns にパディング・アライメントは build 時確定の `alignments` を使い再パースしない・セル内 `\|` は数式内 `\vert `/外 `\|` の二層エスケープ）。実機 smoke: Attention 論文（1706.03762）で **4/4 表構造化・skip 0**（22×13 の multicolumn 入り Table 3 含む）・2607.14797 で 1 表。**単位（siunitx S 列）・表脚注（`\tnote`）の意味抽出はしない**（S は spec 未検証扱い・脚注はセル text に verbatim 残留）。セル座標は rows/cells の添字が相当（TeX に物理座標なし）。ネスト起因 skip の欠損幅は warning 種別で計測可能 — 8c 前に実測して opaque 化を再検討。

**Phase 8c 実装状況（図の代替テキスト・`llm/ocr.rs::describe_image` + `db/node_alt_texts.rs`・migration 0020）**: 8a が作った `figure` ノードのページ crop PNG を LLM Vision に説明させ `node_alt_texts` に保存する。**PDF 版のみ**・**抽出器版は不変**（`lumencite-pdfium` 0.6.0 / `lumencite-tex` 0.5.0 のまま = 生成は build の外なので content_key に影響しない）。生成器は既存 OCR 配管（`llm::ocr::ocr_image` と同じ `ContentBlock::Image` + `stream_chat`・provider/model は `llm.ocr_provider`/`llm.ocr_model` にフォールバック解決・キーは keychain）を system プロンプトだけ差し替えて転用し、crop PNG は 8a がディスクに持っているものを `fs::read`→base64 する（**pdfium 再レンダは不要**）。**実行は build 外の opt-in 後追いバッチ** `generate_vision_alt_texts`（`fetch_missing_arxiv_sources` と同型: AtomicBool + compare_exchange の多重起動ガード・RAII Drop 解放・`vision-alt-text-progress` 進捗イベント・リクエスト間 1 秒スロットル）で、**1 図ずつ best-effort**（既存 OCR の all-or-nothing は流用しない — 1 図の失敗で全体を捨てない）。**build-inline は禁止**（Vision は非同期・課金・非決定的なので、混ぜると content_key の冪等性と §16 の決定的 build が壊れる）。フラグは `lcir.vision_alt_text.enabled`（既定 off・`lcir.enabled` とは**独立の同意面** — 画像 1 枚ごとに外部 API 送信 + 課金が発生するため。`clipper.enabled` の前例）。保存は `figure` ノードの satellite 表（`payload_json` 相乗りを却下 — ノードの `origin`/`confidence` は図領域検出で占有済みで provenance が opaque になる）で、`origin='llm_inference'` + `confidence` + `model` + `source_asset_sha256`（説明した画像の指紋）。**版跨ぎ provenance は「crop 画像同一性（sha256）で carry」**: 新版 build の tx 内で同一添付の**過去の全版**から指紋一致の `llm_inference` 行を探して最新を新版へコピーし（`carried_from_version_id` に由来版）、同 tx で現版以外の `llm_inference` 行のうち**新版にも同一指紋の画像がある**ものだけを刈る（crop PNG は 8a の GC で trash 済。crop 書き出しが一部失敗して carry できなかった行は残す = 課金済みの説明を失わない）。`user_edited` 行は carry も削除も上書きもしない（手編集 UI は初回スコープ外だが `origin` 列は最初から持つ）。同一 crop が複数 `figure` ノードに対応する場合は同じ alt text が両方に載るのを許容（同じ絵なら同じ説明）。原文 caption は**上書きしない**・生成文は `fulltext`/`document_nodes_fts` に**索引しない**（原文由来と生成物を混ぜない）。read 面は `LcirNode.alt_text`（`user_edited` > `llm_inference` 優先）+ MCP `get_figures` の `alt_text{text, origin, confidence, model}`。 UI は設定 → データの LCIR 節に「同意チェックボックス + 一括生成ボタン」で置き、**押す前に生成対象の件数**（`count_figures_missing_alt_text`）を見せる（課金の規模を知らせてから同意させる）。**やらないこと**: SVG/plot 軸凡例・diagram のノード/辺認識・PDF 表画像認識・ページ OCR 全文化（8d 以降 or 非目標）・1 バッチの図上限（進捗イベントで足りるため 8d 送り）。

**Phase 9a 実装状況（エクスポート第一段・`export/`・決定的純関数）**: エントリ単位で LCIR を **LCIR JSON**（`export::lcir_json_pretty` — 書き出し前に `validation::validate` を必ず通す）と **構造付き Markdown**（`export::markdown::render_markdown` — pdfium 非依存の純関数・CI テスト可能）へ書き出す。エントリ→版解決は MCP にあった tex > pdfium 優先ロジックを `ingestion::{entry_lcir_versions, load_entry_lcir, source_to_extractor, short_source_name}` へ共有化し、MCP / Tauri コマンド / CLI の単一ソースにした。Markdown は YAML フロントマター（書誌 + `lcir_source`（抽出器名・版）で由来を常に明示）→ 見出し（節番号は plain_text に既含なら二重付与しない）→ 段落（インライン数式 `$..$` は原文 verbatim・エスケープ一切なし）→ display 数式（原文 LaTeX を `$$..$$` に正規化: `\[..\]` は区切りを剥がして包み・`$$..$$` はそのまま・環境形 `\begin{..}` は包む＝二重区切りを作らない。**surface-only（PDF 由来）は `$$` を付けない** — 生 LaTeX でないものを数式と偽らない）→ 定理系 blockquote（`**Theorem 2.3** (付記名).`）→ 図表 caption（イタリック）→ 図（`**[Figure 3]** (p. 5)` の存在マーカー + alt text の blockquote。**画像リンク `![](..)` は張らない** — `relative_path` は app data dir 相対の内部参照で `.md` の保存先から解決できず、存在保証も無い（再構築 GC で消える）ので、書けば必ず壊れた参照になる。解決には app data dir が要るがレンダラは純関数で fs/DB に触らない。alt text は LLM Vision の生成物なので由来ラベル（`**AI-generated description** (model: X).`）を必ず添え、`confidence` は出さない。番号は caption 由来の `figure_number` があるときだけで、内部通番 `figure_index` は可視出力に出さない）→ 参考文献（`cite_key` 付きリスト）。`document`/`page`/`line` と page 全文 plain_text は描画しない（ブロックと重複）。**未知 node_kind は plain_text の段落に degrade**（Phase 7/8 の型追加でレンダラ不変）。経路は詳細パネルの 2 ボタン（`export_lcir_json`/`export_lcir_markdown`・保存ダイアログ・フラグ ON 時のみ表示）と CLI `export-lcir <id|key> [--format json|md] [--source tex|pdf] [-o]`（読取専用）。**migration なし・依存追加なし**。実機 smoke（entry 563・arXiv 2607.14797）でフロントマター＋節構造＋原文 LaTeX 数式＋cite_key 付き References を確認（当該 DB の版が旧 `lumencite-tex 0.1.0` のため定理/relations/symbols は空 — `rebuild_outdated_lcir` 後に反映される。由来はフロントマターで判別可能）。JATS/TEI/HTML+MathML は 9b（Phase 7 後）。

**Phase 8d-7 実装状況（本文 → 図表参照の解決・`ingestion/graph.rs`・migration 不要）**: **PDF 版のみ**（`lumencite-pdfium` 0.6.0→**0.7.0**・TeX 側は byte-for-byte 不変）。6a の PDF 参照スキャナは "Theorem 2.3" / "Eq. (2.1)" しか拾わなかったので、"Figure 3" / "Fig. 3" / "Fig.3" / "Table 2" を図表番号と照合して `refers_to_figure` / `refers_to_table` を張る（`origin='layout_model'`・confidence 0.6）。ターゲット索引は **実体（`figure`/`table` ノード）優先・無ければ caption （`figure_caption`/`table_caption`）** の 2 段だが、**実データ上の主経路は caption 側**である — `figure` ノードが番号を持つのは 8a の幾何ペアリングが成立したときだけ（実測で保有率 2 割強）で、PDF 側に `table` ノードは 1 件も無い（8d-6 未実装）。どちらに解決したかは `metadata.resolved_via`（`"node"`/`"caption"`）に残す — bbox が引けるのは `"node"` のときだけで、caption から実体へは `caption_of` を 1 ホップ辿る。`figure` ノードは `plain_text` を持たないので**ターゲット専用**として `graph_nodes` に積む（参照元にはならない）。**張らないもの**（すべて §16「誤検出より欠損」）: ①複数形・範囲参照（"Figures 3 and 4" / "Figs. 1-3"）— `take_ref_number` は先頭 1 個しか読まないので、拾うと「部分的な参照集合」を完全なものとして下流に見せる ②同一版で番号が衝突する図表（実測 45 件）— 先勝ちにせず索引ごと落とす（定理番号と違い種別で分けても衝突が残る）③全大文字 `FIG.` / 小文字 `fig.`（既存 3 キーワード表と同じく大文字始まりのみ）④caption 冒頭の自己ラベル "Figure 3: …" — 8a の `caption_of` と重複する冗長辺になるため、同カテゴリ caption かつ「同番号 **または** 先頭 2 バイト以内」で落とす（caption 内の**他図**への参照は残す）。参照キーワードは `structure::detect_caption` が caption として認識できるラベル語だけに絞る（`is_figure_caption_label` / `is_table_caption_label` に集約し 8a のペアリングと同じ集合を見る — Algorithm / Listing は `figure_caption` だが図番号ではないので両方から外れる）。解決率の実測は **91.18%（1302/1428・2026-07-30 再計測。「約 93%」は誤り）**。**残りの原因を debt-12 に帰したのも誤り** — 参照側の `take_ref_number` は ASCII 数字（と付録形 "A.1"）しか読まないので、本文の "Table I" はそもそも参照として走査されない。したがって debt-12 の解消（2026-08-02・#1）で増える解決辺は **+0** である（参照側もローマ対応させれば +81 辺だが参照母数が 1428→1529 に増えるので**率は 90.45% に下がる**）。**抽出器版を上げたが再構築は必須ではない** — 辺は次回 `rebuild_outdated_lcir` 実行時に付く。

**debt-8 実装状況（エクスポート欠落警告・`export/warning.rs`）**: ロードマップ Phase 9 完了条件「LCIR 固有情報が失われる場合に警告を出せる」。`render_markdown` / `lcir_json_pretty` は `ExportReport { text, warnings }` を返し、**警告はエラーではない**（書き出しは成功している）。`document_ir::validation`（不正な LCIR を弾く・Err）とは役割が違う。形式ごとの表現力を `FormatCapabilities`（`relations` / `symbols` / `node_provenance` / `coordinates` / `embedded_assets` / `cell_spans`）で宣言し、警告は**そこから機械的に導く 1 パス走査**にした — レンダラ側の instrumentation（出力バイト数を覗く等）は「ノードが出力に触れたか」しか見えず `render_node` の分岐と二重管理になるので採らない。**狼少年にしないための 3 規約**: ①その文書に実際に存在するデータだけ報告する（relations 0 本なら出さない）②どの形式でも常に落ちる縮約（ノード id・`content_key`・schema URI）は警告にしない ③1 つの損失を 1 コードで報告する。コードは 6 種（`relations_dropped` / `symbols_dropped` / `inferred_provenance_dropped` は warn、`source_fragments_dropped` / `assets_not_embedded` / `table_rowspan_flattened` は info）で、並びは `(severity, code)` で決定的。Markdown は全項目が落ちうる形式、LCIR JSON は無損失なので `assets_not_embedded` しか出ない。出口は UI（保存後に一覧・i18n キー `detailPanel.lcirExportWarn.<code>`）と CLI（**stderr**・stdout は本文のままなのでパイプ利用を壊さない・終了コード 0）。**9b（HTML/JATS/TEI）は `FormatCapabilities` の const を 1 つ増やすだけで同じチャネルに乗る**（棚卸し §4 の 9b-0 から「欠落警告チャネル」が外れる）。

**Phase 10a 実装状況（文脈バンドル・`context/`・migration 不要・新表も新推定器も無し）**: 「この定理の主張・前提定義・証明・参照数式/図表を 1 回で寄越せ」に答える `get_node_context`（MCP）/ `node-context`（CLI）。中身は `LcirDocument`（= 既存 7 表の派生ビュー）だけを入力に取る**決定的純関数** `context::build_node_context` で、`export/` と同じく fs/DB に触らない（pdfium 不要・`#[test]` 30 本で CI 完結）。DB 側の入口は `ingestion::load_node_lcir`（`document_nodes::version_id_for_node` → `document_versions::find_by_id` → `load_lcir_document_for_version`）。**引数は `node_id` のみで `entry_id` も `source` も取らない** — ノード id がどの版の話かを既に決めているので、エントリ起点の read 優先度（tex > pdfium）で版を選び直すと呼び出し側が握っている id が引けない版に化ける。`find_by_id` は status で絞らないので superseded 版のノードも読める。**中核は `continuation`（読み順スパン）**: PDF 版の定理ノードは主張の**先頭 1 レイアウトブロック**しか持たず（実測 `theorem` 平均 168 字 / `proof` 256 字。TeX 版は環境本文が丸ごと 1 ノードで 975 字）、続きの式や "where …" は theorem の子ではなく **page 直下の兄弟**に落ちている。しかもそのスパンは theorem の 33% / proof の 53% で**ページをまたぐ**。そこでノード木を pre-order（子は `(ordinal, id)` 昇順）で 1 本の列にし（PDF の `document > page > block` も TeX の `document > block` も同じ規則で並び、**列の上でページ境界が消える**）、次の構造境界の手前まで連結する ＝ ロードマップ完了条件「ページ境界で文脈が切れない」の実体。`before` は逆に最初の境界を**含めて**打ち切る（直前の見出し/定理まで見せて向き付けする）。**辺は焦点 + `continuation` の両方から拾う**（PDF では "by Lemma 2.1" が続きのブロックに乗るため）。**前提定義は辺だけでは足りない**（実測: 定理系 2,242 件のうち `definition` を指す出辺を持つのは 31 件 = 1.4%。`defines_symbol`/`depends_on` は語彙だけで生成経路が無い）ので 3 経路を `Premise.via` で**区別したまま**返す: `reference`（`refers_to_theorem` の指し先が `definition`）/ `occurrence`（`symbol_occurrences` の記録・display 数式のみ・TeX）/ `symbol`（記号の `surface_form` が本文に `$X$` の形で現れ、かつ `defined_at` が読み順で焦点より**前**・TeX）。`symbol` だけがこの関数内の表層照合だが、**新しい事実を保存するのではなく既存 `symbols` 行の絞り込み**であり、記号ごとに 1 件へ畳む（優先順は occurrence > description 有 > 直近）。図表参照は `caption_of`（from=caption → to=figure/table）を解決して `{node（辺の指し先・無加工）, figure（bbox・crop・alt text の持ち主）, caption（原文）}` の 3 点で返す — caption→実体は**順方向**、実体→caption は逆引き（`exec_get_figures` と同じ）。実測で `figure_caption` 1,021 件のうち `caption_of` を持つのは 262 件（25.7%）なので `figure` は欠落が常態。**2 ホップはしない**（定理のバンドルに証明の参照まで畳み込むと長さが予測不能になる。`proofs` の `node_id` で呼び直す＝合成は呼び出し側の仕事）。**読み順スパンの境界は「`heading_level` を宣言している見出し」だけ**にする — 実ライブラリの PDF `heading` 2,810 件のうち 2,640 件（94%）はレベル宣言の無い confidence 0.55 の推定で、中身は数式断片（`"AiAj ="` / `"fλ = n!"`）や柱。これを境界に数えると**定理の主張そのものの直前で continuation が止まる**（実測: 定理系スパン 2,526 件のうち 240 件が該当・うち 43 件は continuation 0）。§16「誤検出より欠損」は**構造の主張**に掛かる原則であって、「推定した境界で本文を落とす」ことの正当化にはならない（載れば読み手が判断でき、捨てれば見えない）。**止めた理由は必ず返す**（`continuation_stopped_at = {reason, node_id?, kind?}`・`boundary` / `max_continuation` / `max_continuation_chars` / `end_of_document`）— `continuation` が空/短いことの意味は「主張が終わった」「フロートの caption に割り込まれた（実測 63 件）」「上限で切った」で全く違う。**届かなかったものは `notes` に機械可読コードで出す**（`float_entity_unreachable` / `proves_target_is_not_a_theorem`（PDF の `proves` は 96% が読み順の隣接フォールバックで実測 9% が remark/example/definition を指す）/ `continuation_truncated` / `related_truncated` / `premises_truncated` / `focus_is_not_a_content_block`）。**表現ごとに恒真な事実（TeX に座標が無い / PDF に記号が無い）は載せない** — `source` から決まり全バンドルに必ず付くので注記の情報量が無い（規約②の適用）。文字数予算は本文だけでなく `math.latex` / alt text も数える（TeX の display 数式は plain_text と別に原文 LaTeX を持つため、本文長だけで測ると応答が入力依存で膨らむ）。`export::ExportWarningCode`（「この**形式**では運べない」）とは型を共有しない — こちらは形式ではなく**このバンドルの中身**の話だから。ただし「狼少年にしない 3 規約」（実際に起きたことだけ / 常に真の一般論は載せない / 1 事実 1 コード）は共有し、「複数形の参照には辺が無い」のような普遍の但し書きはツール説明の側に置く。完了条件 (1) は PDF 版で全ノードに `page`+`bbox`（既存 3 ツールと同じ 4 要素配列）を載せて満たす（**TeX 版は座標を持たないので原理的に満たせない** — `notes` で明示）。(3) は `latex`（TeX）/ `normalized_text`+`semantic_status`（PDF）を無加工で透過（`ast_json`/`presentation_mathml` は Phase 7 まで NULL）。(4) は全ノード・全辺・全記号の `origin` + `confidence` を透過して満たす。

**Phase 10b 実装状況（チャットへの LCIR 露出 + provenance 付き回答 + 根拠ジャンプ・migration 不要・新表も新推定器も無し）**: 文献本文の read ツール 9 種（`get_fulltext` + LCIR 8 種）の定義と実行を `mcp_server` から **`llm::tools::document` へ移設**し、**MCP サーバーとアプリ内チャットの単一ソース**にした（`mcp_server` は `tool_specs` / `exec_tool` から委譲する。`search` / `mutate` が既に持っていた関係を文献本文系へ広げただけで、`tools/list` の内容・並び順・各ツールの入出力は不変）。**`SHARED_READ_TOOLS` は流用せず専用の `DOCUMENT_TOOLS` を持つ** — あの定数は「`tools/list` のフィルタ」と「dispatch の早期 return」の二役なので、名前を足すと `search::try_execute` に流れて全 LCIR ツールが `unknown tool` になる。**スコープ（CR-024）は `entry_id` が確定したすべての経路**に置く: `resolve_entry_id` の直後 6 か所に加え、**`get_fulltext` は独自の解決を持つので個別に**（未知キーの返し方が `Ok{indexed:false}` と `Err` で違うため共有していない）、**`get_node_context` は引数が `node_id` だけなので `load_node_lcir` で entry が判った直後・バンドルを組む前に**。横断検索 `search_document_nodes` は取得後に落とし、**落としたことを `scope_filtered` で応答に出す**（黙って絞ると「ライブラリに無い」と読まれる）。MCP 経路は `mcp_ctx` が `scope_mode="all"` 固定なのでこれら全てが no-op で、挙動は変わらない。**チャットの一覧に出す条件は「フラグが ON か」ではなく「読める版が実在するか」**（`ingestion::lcir_readable`）— `lcir.enabled` を ON にしただけでは何も構築されない（PDF の build は手動ボタン）ので、フラグだけで判定すると `has_lcir:false` しか返さないツールの定義でコンテキストを食う。一覧から隠しても実行はできる（過去ターンの履歴に残った呼び出しが再送されても壊さない・読み取り専用なので安全側に倒す必要がない）。**承認**は read の集合を `approval::READ_ONLY_TOOLS` に定数化して 9 種を追加（足さないと全部「未知のツール = 要承認」に落ち、1 問ごとに承認ダイアログが並ぶ）。フロントに同じ分類の写しが 2 つあった（`toolKind` の色分けと `isLibraryMutatingTool` の一覧再読込判定・後者は既定 true なので read ツールを呼ぶたびに全再読込していた）ので `src/chat/tools.ts` に集約した。**provenance** は `effective_system_prompt` がツール契約を必ず末尾に足す（ペルソナではなくツール結果の読み方の規約なので、セッション固有プロンプトがあっても付ける）。契約は原文由来（`tex_source`/`pdf_text_layer`）と推定（`layout_model`/`llm_inference`）の出し分け・`scope_filtered` の扱い・定理は `get_node_context` で読むこと・領域を引用するなら pdf 表現を読むこと、を指示する。**契約だけでは足りない**ので `get_document_blocks` の各ブロックに `origin`/`confidence` を追加した（従来これらを持つのは `get_node_context` だけで、モデルが最も使うツールにはデータが無かった＝完了条件 (4) がプロンプト頼みだった）。**根拠ジャンプ**は `document::provenance_refs`（結果 JSON → `{node_id, kind, page}`・上限 5 件・**`page` を持つものだけ**・ツール名で先に振り分けるので生 .bib や失敗メッセージは parse に到達しない）を `ChatStreamEvent::ToolCallExecuted.refs` に載せ、ツールカードのチップから `get_lcir_node_region`（**Phase 10b で `{node_id, attachment_id, source, page, bbox}` へ拡張** — フロントから 1 度も呼ばれていなかったので新コマンドを足さずに広げた）→ `open_pdf_viewer(id, page, region)` で PDF ビューアの該当領域を一時強調する。**抽出を Rust 側に置く理由**はライブ配信の `result_summary` が 500 文字で切られていてフロントでは JSON として読めないから。履歴復元時はイベントが無いので同じ純関数を `chat_tool_refs` コマンドで引き直す（TS 側に写しを作らない）。既存ウィンドウへの通知は **`emit_to` で宛先指定**に変えた — `win.emit` は全ウィンドウ broadcast で、2 枚目のビューアが別の論文の同じ位置に嘘の根拠を描く（page だけの頃は余計なスクロールで済んでいた）。座標変換は `PdfPane` のハイライトと同一（LCIR の bbox は既存ハイライトと同じ PDF user space・左下原点・pt）。**チャットは MCP と応答の経済が違う** — ツール結果は会話履歴に永続化され以後のターンで毎回再送されるので、1 回の巨大な応答がセッション全体を壊す。そこで `search_document_nodes` に SQL の `LIMIT`（`max_results` 既定 50・上限 200 + `truncated`。ブロック粒度 FTS は 1 語が数千行に当たり、行ごとに 2 クエリ引く N+1 でもあった）と、`get_node_context` の各サイズ引数に上限を入れた（従来は負値を弾くだけ）。**issue #42** は `get_fulltext` の露出に加えて `ocr_pdf` 側も直した（説明文を「まず `get_fulltext`・`indexed:false` のときだけ OCR」に変え、索引済みエントリの全ページ OCR を LLM 経路で拒否する。全ページ OCR は添付の索引を置き換えるのでテキスト層が Vision 出力で消える。ユーザーが UI から押す経路は従来どおり）。**既知の限界**: TeX 由来の LCIR には `source_fragments` が無いので、TeX 版を読んだときは根拠チップが出ない（`load_entry_lcir` は tex を優先するので arXiv 論文では既定でこちら）。完了条件 (1)「LLM 回答から根拠ノードと PDF 領域へ移動できる」は **PDF 由来 LCIR について**満たす。tex → pdf の位置解決は post-1.0。

### 5.5 LCIR JSON 概念例（派生ビュー）

SQLite が正本だが、export/デバッグ/テスト用に次の JSON を生成できるようにする（ロードマップ §6 を LumenCite 形に）。

```json
{
  "schema": "https://lumencite.dev/schema/document-ir/0.1",
  "schema_version": "0.1.0",
  "version_id": 42,
  "content_key": "…sha256…",
  "source": {
    "sha256": "…",
    "mime_type": "application/pdf",
    "extractor": { "name": "lumencite-pdfium", "version": "0.1.0" }
  },
  "coordinate_space": { "space": "pdf_user_space", "origin": "bottom_left", "unit": "pt", "y_axis": "up" },
  "nodes": [
    { "id": 1, "kind": "document", "ordinal": 0, "children": [2] },
    { "id": 2, "kind": "page", "ordinal": 0,
      "payload": { "page_width_pt": 595.3, "page_height_pt": 841.9, "rotation_deg": 0 },
      "plain_text": "…page 1 full text…",
      "source_fragments": [ { "page": 1, "bbox": [0, 0, 595.3, 841.9], "fragment_type": "page" } ] }
  ]
}
```

---

## 6. 座標系仕様

- **保存空間 = PDF user space（左下原点・y 上・単位 pt・rotation = ページ `/Rotate` 度）。** 既存 `highlights`（DATA_MODEL.md「`pdf.js` の座標系（PDF ポイント、左下原点）」）と一致。抽出時に無損失、PDF ビューアがそのまま消費できる。
- `document_version.metadata_json` に `CoordinateSpace {"space":"pdf_user_space","origin":"bottom_left","unit":"pt","y_axis":"up"}` を記録（将来の top-left/pixel 系 layout model と混同しないため）。
- `page` ノード `payload_json` に `{page_width_pt, page_height_pt, rotation_deg}`（pdfium の `page.width()/height()/rotation()`）。
- **各 `page` ノードには常にページ全面（MediaBox）の `source_fragment` を1つ付与。** text_block 分割が失敗しても page 粒度に degrade し情報を失わない（ロードマップ 4.5「欠損を許容」）。
- **回転ページの座標系（旧「要検証」・2026-07-30 に実測で決着）**: pdfium の `page.width()/height()` は**回転適用後**の寸法を返す（`/Rotate 90` で幅と高さが入れ替わる）が、text/object の `bounds()` は**回転前の user space** のまま返る。したがって「raw bounds + `rotation_deg` の両方を保存し、消費側で合成する」という当初方針が正しく、保存値の変更は不要。フロント（`PdfViewer`）は pdf.js の `PageViewport` を通しており、viewport は viewBox（= CropBox）原点とページ回転の両方を吸収するので保存値をそのまま渡せる。**ページ全体をラスタライズする経路（8a の crop）だけは別**で、レンダリング結果が回転適用後のビットマップなので bbox（回転前空間）→ ピクセル（回転後空間）の写像に回転合成が要る。現在の「回転ページは skip + warning」は妥当（**2026-08-03 に debt-9 として post-1.0 へ確定** — 実ライブラリの回転頁 5 頁には top-level Image も XObjectForm 内 Image も 1 枚も無く、有効にしても増える図が 0 件のため）。テストコーパスに回転ページ PDF を含めること（Coordinate Test）は維持する。根拠と実測（実ライブラリの回転ページは 1 添付 5 ページのみ・figure 0 件）は `docs/LCIR_REMAINING_PHASES.md` §4.4。
- **既知の不整合（debt-19）**: `page` ノードの `payload_json` が持つ寸法は CropBox の幅・高さである一方、block/line の bbox は MediaBox 絶対座標で入るため、CropBox 原点が非ゼロの PDF では **fragment がページ矩形をはみ出す**（実測 約 11,970 件 / 12 版・5 件以上に絞ると 9 版・y 方向最大 +116pt・x 方向最大 +464pt）。保存契約（絶対 user space）としては正しく pdf.js が吸収するので表示は壊れないが、**「page 寸法で正規化する」下流コードを書いてはいけない**（9b-4 の座標変換が該当）。はみ出しの発生源は `ingestion/mod.rs:547-559`（page ノードの fragment を「原点 `(0,0)` + box の寸法」で入れている）。図領域のクランプにあった同じ取り違えは **debt-14 として解消済**（2026-08-02・抽出器版 0.9.0。クランプ範囲をページ境界 box へ、その原点を `CropBox ∩ MediaBox` へ）、構造認識の `in_margin` 判定にあった同じ取り違えも **debt-18 として解消済**（2026-08-03・抽出器版 0.10.0。走り柱の帯をページ境界 box の原点から測る。実測で `unknown_block`→`paragraph` 107 件 / `paragraph`→`unknown_block` 12 件が反転する）。**残るのは debt-19 本体**（この page fragment そのもの）。

---

## 7. provenance と再現性（content_key）

- **`source_sha256`**: `attachments` に SHA-256 列は無いので抽出時にファイルから計算（`document_ir::sha256_file`・ストリーム・小文字 hex）。
- **`extractor_version`**: `document_ir/schema.rs` の const。**抽出ロジックの semver で supersede トリガ**。pdfium クレート版・アプリ版は `metadata_json` に詳細として。
- **`content_key`** = `sha256("lcir-content-key-v1\n" | source_sha256 | "\n" | extractor_name | "\n" | extractor_version | "\n" | config_hash)`。
- **冪等**: build 時に `content_key` を先に計算し `find_by_content_key`。`completed` 行があれば**再抽出せず reuse**。version/バイトが変われば別 `content_key` → 新行を作り旧行を `superseded`・新行 `parent_version_id` で連結。
- **best-effort UNIQUE**: `try_create_content_key_unique_index` は `db/entries.rs` の `try_create_identifier_unique_indexes`（CR-019）を踏襲。重複が無い時だけ `UNIQUE INDEX ON document_versions(content_key)` を張り、あれば skip + 警告ログ（既存 DB に重複があっても起動不能=brick にしない）。起動フックの既存 best-effort index 作成群の隣で呼ぶ。

これにより Phase 0 完了条件「**同一 PDF から同一の文書バージョン ID（= content_key）を再現できる**」を満たす。row id は再現不能であることを明示的にドキュメント化する。

---

## 8. FTS5 との共存戦略

FTS5 は正本ではなく LCIR から生成される検索インデックス、というのが最終形。ただし移行はロードマップ §12 に従い**段階的**に行う。

- **第一段は並走(A)**: 既存 `fulltext` は今まで通り `pdf_extract` → `db::fulltext::index_attachment` で生成し続ける。LCIR は pdfium で**追加の side-build**。フラグ ON でも**検索挙動は変わらない**（実験トグルとして必須）。→ フラグ ON 時は同一 PDF を 2 回抽出（pdf-extract=検索用 / pdfium=LCIR 用）。実験期間の対価として許容。
- **派生化(B) への seam**: `ingestion::regenerate_page_fts_from_lcir(pool, attachment_id)` を第一段で実装・単体テストした（Phase 1 完了条件「FTS5 を削除しても LCIR から再構築できる」を証明）。ただし既定ソースにはしておらず、**本番呼び出し元は現在も 0 件**（テスト 2 箇所のみ）。
- **「(B) 化は 1 行の差し替え」は撤回する**（2026-07-30 の実コード調査）。実際には ①`fulltext` に**内容を書く**本番経路が 6 call site / 4 コードパスある（`extract_and_index` ×3・`index_attachment` コマンド・`index_missing_attachments`・`run_ocr` ×2）②`db::fulltext::index_attachment` は先頭で `DELETE FROM fulltext WHERE attachment_id = ?` を無条件に打つので、そのまま差し替えると LCIR が空を返す添付（スキャン本）の索引が丸ごと消える＝非破壊更新への切替が要る ③pdfium の `page` ノードは `structure::normalize_ws` を通らないため C0 制御文字を含み（実測: 非空 5,803 ページの 75.3%・`fulltext` 側は 11.6%）、素朴に派生化すると trigram 索引で語が割れて**現状より検索が悪くなる**（対策は「page の子 block を ordinal 順に連結する」— block は正規化済みで page 文字数の 98.4% をカバーする）④既存コーパスに派生索引を行き渡らせるバッチが存在しない（`attachments_without_completed_lcir` は完了版のある添付を除外し、`attachments_with_outdated_lcir` は版 bump 無しでは 0 件）⑤OCR 由来行を守る手段が無い（`fulltext` に provenance が無く、既存行が OCR 由来かは実 DB からも判定できない）。**規模は 900–1,250 行**（最小スライスでも 400–480 行）。段取り・落とし穴・OCR 保護の 3 択は `docs/LCIR_REMAINING_PHASES.md` の v1.0.0-p1 節を参照。ロードマップ §12 の「新旧品質を比較してから既定化」は維持する（比較自体は実 DB の読み取り専用 SQL だけでできる — LCIR page テキストと `pdf_extract` 由来 `fulltext` は既に同じ DB に共存している）。
- **ページ FTS(§7.1) と意味 FTS(§7.2)**: 第一段はページ単位（既存 `fulltext` 互換）。**Phase 2 で `document_nodes_fts`（段落/見出し/caption 単位）と `regenerate_node_fts_from_lcir` を実装済**。既存 `fulltext`（ページ粒度・pdf-extract 由来）と併存する追加の派生索引で、LCIR build 時に張る。`document`/`page`/`line` を除く本文つきブロックを索引し、`search_lcir_nodes` がヒットに `node_kind` と PDF 上の `bbox` を返す（検索→ブロックハイライトに直結）。既存 `fulltext` の検索挙動は不変。
- **TeX 版（Phase 4）は派生索引に載せない**: 同一エントリの PDF 版と本文が重複ヒットし、bbox も持たないため、`lumencite-tex` の version は `document_nodes_fts`/`fulltext` の対象外（`regenerate_node_fts_from_lcir` は非 pdfium 版しか無い添付では索引をクリアして 0 を返す）。検索は PDF 版・構造/数式の読み出しは TeX 優先、という分担。TeX 本文の検索が要る場合は将来 entry 単位の優先版だけを索引する方式で再検討する。

---

## 9. 既存データの移行方針（ロードマップ §12）

既存 FTS5 データを破壊的に変更しない。

1. 既存添付ごとに `document_versions` を作成（lazy: `build_missing_lcir` コマンドで明示実行。起動時 pdfium 一括掃引はしない）。
2. pdfium 再抽出で `page` / `text_block` ノードを生成（既存 `fulltext` は座標を持たないため、そのまま LCIR 化はできず**再抽出可能な PDF のみ**座標付き LCIR を得る）。
3. 既存 `fulltext` は legacy index として維持。
4. LCIR 由来の新インデックスを並行運用し検索品質を比較。
5. 十分な互換性が確認できたら新インデックスを既定化。
6. legacy index は再生成可能になった時点で削除候補。

`build_missing_lcir` は既存 `index_missing_attachments`（ユーザー起動バッチ）と同型で、`completed` バージョンが無い添付を走査する。

---

## 10. モジュール構成（目標ツリー）

既存 `db/` 一表一ファイル規約 + ロードマップ §18 ツリーを LumenCite に合わせる。**★ = 第一段で作成**、他は予約（後続フェーズで作成）。

```text
src-tauri/src/
  document_ir/            # DB 非依存の純型（一次仕様）
    mod.rs      ★  # 再エクスポート・content_key()・sha256_file()
    schema.rs   ★  # SCHEMA_URI/VERSION・EXTRACTOR_NAME/VERSION const
    node.rs     ★  # NodeKind/Origin/ExtractionStatus enum・ノード DTO
    source.rs   ★  # BBox・CoordinateSpace
    validation.rs ★ # LCIR JSON 最小 validation
    relation.rs / math.rs / symbol.rs   # Phase 3/6 で作成済
    asset.rs    # Phase 8a: LcirAsset ビュー（旧予約名 figure.rs 相当）
    alt_text.rs # Phase 8c: LcirAltText ビュー（図の代替テキスト・AI 生成）
  ingestion/
    mod.rs      ★  # build_lcir_for_attachment・regenerate_page_fts_from_lcir・lcir_enabled
                   # （※ `post_attach` という関数は存在しない。事実上の共有 post-attach フックは
                   #    db/fulltext.rs の extract_and_index で、LCIR build はまだそこに乗っていない = v1.0.0-p2）
    figures.rs  # Phase 8a: 図領域マージ・pt→px 変換・caption ペアリング（純関数）
    pdf/
      mod.rs    ★  # extract_document(path, asset_dir) -> ExtractedDocument（pdfium・spawn_blocking 下）
      pdfium.rs ★  # bind_pdfium() 集約（ocr.rs から移設して共用）
    tex/
      mod.rs  ★  # Phase 4: TeX 構造認識（parse_tex・純関数）
      source.rs ★ # Phase 4: gzip/tar のメモリ内展開・main 検出・\input 解決
      tabular.rs  # Phase 8b: tabular セル構造化（grid・colspan/rowspan・罫線・純関数）
    jats/ tei/ html/                                 # 予約（後続）
  db/                     # 既存 storage 層（一表一ファイル）
    document_versions.rs ★
    document_nodes.rs    ★
    source_fragments.rs  ★
    assets.rs   # Phase 8a: assets + node_assets（2 表 1 ファイル・symbols.rs と同型）
    node_alt_texts.rs # Phase 8c: node_alt_texts（carry / prune / バッチ対象クエリ）
  export/
    mod.rs      # Phase 9a: LCIR JSON 書き出し（validation 通過必須）
    markdown.rs # Phase 9a: LcirDocument → Markdown の決定的純関数レンダラ
    warning.rs  # debt-8: 形式ごとの欠落警告（FormatCapabilities から機械的に導く）
  context/
    mod.rs      # Phase 10a: 文脈バンドル（LcirDocument → NodeContext の決定的純関数）
  indexing/ jobs/                                    # 予約（Phase 2/8）
```

- DTO は `src-tauri/src/models.rs`（snake_case・`rename_all` 無し・`sqlx::FromRow`）。
- `lib.rs` 冒頭に `mod document_ir;` `mod ingestion;` を追加。
- `bind_pdfium()` は現在 `src-tauri/src/llm/tools/ocr.rs` にある。`ingestion/pdf/pdfium.rs` に集約し `ocr.rs` から呼ぶ（binding を一箇所に）。

---

## 11. 非目標（初期段階でやらないこと）

ロードマップ §16 を明示的に採用する。

- あらゆる PDF の完全な論理構造復元
- すべての数式の意味の自動確定（`AB` が数の積か行列積か作用素積か関数適用かは文脈なしに確定不能。意味表現には必ず `semantic_status` + `confidence` を付ける）
- すべての図から元データを完全復元
- 任意の TeX マクロの完全展開
- JATS/TEI/OpenMath への完全な可逆変換
- **AI 推定結果を人手確認なしで真実として扱うこと**
- 一つの万能フォーマットへの統一

### AI 推定と原文由来の区別

各データに `origin`（`publisher_source`/`tex_source`/`pdf_text_layer`/`ocr`/`layout_model`/`math_recognition`/`llm_inference`/`user_edited`）と `confidence` を付け、**原文由来と推定を常に区別する**。ユーザー修正は既存を上書きせず新しい provenance として保存する（Phase 7・seam のみ第一段で用意）。

---

## 12. テスト戦略（要点）

- **Golden File Test**: 同一入力から生成される LCIR JSON を固定し差分検査（抽出器更新時に改善か回帰かを判定）。実装は `src-tauri/src/document_ir/testdata/*.json` を `include_str!` で読み、手組み `LcirDocument` と**構造比較**（serde 経由・pdfium 不要で CI 実行可能）。
- **Schema Validation**: すべての LCIR JSON を検証（欠損フィールドで fail）。
- **再現性**: `content_key` 決定性（同一 → 同一 / version 変更 → 別 / sha 変更 → 別）、DB 冪等（2 回 build で `completed` 1 行）。
- **FTS 再構築**: page ノード → `regenerate_page_fts_from_lcir` → `search_fulltext` がヒット。
- **フラグ OFF 不変**: フラグ未設定で `build_lcir_for_attachment` が `document_versions` を 0 行、既存 `fulltext` テスト全 green。
- **Coordinate Test**: 検索結果/ノードから PDF 上の正しい領域をハイライトできる（回転ページ含むテストコーパス）。
- **pdfium 依存テストは `#[ignore]` gate**（headless CI に native lib 保証なし・手動/`just` 対象）。
- テストコーパス: 1 段組・2 段組・数式多・図多・表多・スキャン PDF・日本語論文・複数ページ定理/証明・Appendix・Supplementary。

CI の clippy `-D warnings` は hard gate。push 前に `rustup update stable` → ローカルで clippy を回す。

---

## 13. 最終到達像

LumenCite の内部構造を、単なる PDF 全文データベースではなく次の性質を持つ研究文書基盤にする。

- 元資料へ常に戻れる / 抽出結果を再現できる / PDF 上の位置を失わない
- 数式を複数表現で保持 / 定理・証明・定義・図・表を独立オブジェクトとして扱える
- 記号の定義と使用関係を追跡 / AI 推定と原文由来を区別
- FTS5・ベクトル検索・数式検索を再生成できる
- JATS/TEI/MathML/OpenMath 等の標準と接続できる
- 将来の LLM や研究エージェントが利用しやすく、人間にとっても検証可能

**最優先は高度な意味理解を一度に実現することではなく、原資料・位置・構造・由来・信頼度を失わずに保存できる基盤を先に作ること。**

---

## 関連ドキュメント

- `docs/LumenCite_machine_readable_document_roadmap.md` — 元ロードマップ（vision）
- `docs/LCIR_REMAINING_PHASES.md` — 残 Phase の棚卸し（難易度・依存・積み残し債務・着手順序）
- `docs/DATA_MODEL.md` — 既存 DB スキーマ（第一段実装時に新3表を追記）
- `docs/API_SPEC.md` — Tauri コマンド仕様（第一段実装時に新コマンドを追記）
- `docs/SPEC.md` — 機能要件・フェーズ
