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
| 8 図表機械可読化 | `assets`/`node_assets`・図切出/SVG/OCR・表セル・plot | F | 1,2 | XL | 予定 |
| **9a エクスポート第一段** | LCIR JSON 書き出し・構造付き Markdown 出力（決定的レンダリング） | — | 1-6 | M | **着手（v0.10.0 予定）** |
| 9b 標準形式エクスポート | JATS/TEI/HTML+MathML 出力 | — | 7, 9a | M | 予定（post-1.0 可） |
| 10 LLM/エージェント | ノードチャンク・provenance 付き回答・embedding 再生成 | — | 2-8 | L | 予定 |

**推奨実装順序**（ロードマップ §11 を LumenCite に合わせて・2026-07-23 改訂）: 1 → 2 → 3(表層) → 4 → **取得整備（クリッパー欠落補完・TeX 一括取得バッチ — Phase 5 が TeX の恩恵を最も受けるため先に取得面を固める・SPEC.md 参照）** → 5 → 6 → **9a(前倒し)** → 8 → 7(意味) → 9b → 10。Content MathML・OpenMath・図の意味解析は重要だが**最初から完全実装を目指さない**。まず原資料・位置・構造・由来を失わない基盤（Phase 1）を作る。

**9a 前倒しと Phase 9 分割の理由（2026-07-23 決定）**: ①エクスポートの中身（`LcirDocument` 派生ビュー・`load_lcir_document`・validation）は Phase 6b 時点で実質完成しており、残作業は書き出し UX と Markdown レンダラのみ（migration 不要・依存追加なし・ヒューリスティックなし＝「誤検出より欠損」を構造的に満たす）。②フラグ OFF で積んだ Phase 4〜6b の成果（原文 LaTeX 数式・定理番号・cite key）を初めて目に見えるユーザー価値に変換できる。③Phase 9 のうち Phase 7（Presentation MathML）に本質依存するのは JATS/TEI/HTML+MathML だけなので、9b に分離すれば二度手間は生じない。`skip_serializing_if` の追加式スキーマにより、Phase 7/8 完了後の拡張は「レンダラの分岐追加」の増分で済む。なお **8 を 7 より先に置く根拠**（従来から・明文化）: 原典 §11 が図表構造化を数式意味より先に置く／Phase 8 の依存（1,2）が Phase 7 の依存（3,6）より浅い／「意味理解より保存基盤先行」原則の下で Phase 8 の中核は『保存』・Phase 7 の中核は『意味』である。

**リリースとの対応（2026-07-19 決定・2026-07-23 改訂・詳細は SPEC.md「v0.8.0 > リリース方針」）**: v0.8.0 = 取得整備と同時（Phase 5 前）。以後はフラグ付きで main に積み、リリースは 2〜3 フェーズごとに間引く。**Phase 9a/10 到達 + `lcir.enabled` 既定 ON 化 = v1.0.0 の看板**（9b は post-1.0 可）。

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
| `assets` | 図・画像・SVG・表データ（SHA-256 + 相対パス参照） | 8 | 0015+ |
| `node_assets` | ノード ↔ アセット（`role`: original/page_crop/vector/thumbnail/ocr_source/plot_data/…） | 8 | 0015+ |
| `node_relations` | ノード間の型付き関係（cites/refers_to_equation/refers_to_theorem/proves/…）。**Phase 6a で migration 0017 として実装済**（参照グラフ・`\ref`/`\eqref`/`\cite` と番号一致で解決・origin+confidence 付き） | 6a | **0017** |
| `symbols` | 記号定義（surface_form/normalized_form/description/symbol_type/scope/semantic_json）。**Phase 6b で migration 0018 として実装済**（TeX 定義文からインライン数式を抽出・origin=tex_source・confidence 付き・TeX のみ） | 6b | **0018** |
| `symbol_occurrences` | 数式中の記号出現 → 定義への関連付け。**Phase 6b で 0018 として実装済**（display 数式内の表層一致・保守的） | 6b | **0018** |

### 5.4 ノード型の全体像（フェーズ別）

| Phase | 追加ノード型 |
|-------|-------------|
| 1 | `document` `page` `text_block` `line` `unknown_block` |
| 2 | `abstract` `front_matter` `section` `subsection` `heading` `paragraph` `list` `list_item` `figure_caption` `table_caption` `footnote` `citation` `bibliography` `bibliography_entry` `code_block` |
| 3 | `inline_math` `display_math` `equation_group` |
| 5 | `definition` `theorem` `lemma` `proposition` `corollary` `remark` `example` `proof` |
| 8 | `figure` `table` |

`node_kind` は制約なし TEXT + `NodeKind` enum（`UnknownBlock`/`from_db` フォールバック付き）。後続フェーズの型追加は enum の variant 追加のみで migration 不要。**認識に確信が持てないブロックは、誤った型を確定するより `unknown_block` + 信頼度で残す。**

**Phase 2 実装状況（`ingestion/structure.rs`・pdfium 非依存の純関数で CI テスト可能）**: セグメント→行→ブロックにまとめ、`section`/`subsection`/`heading`/`paragraph`/`abstract`/`figure_caption`/`table_caption`/`bibliography`/`bibliography_entry`/`unknown_block` を確信度付きで出す。番号付き節・caption はパターン、abstract/参考文献は「見出し→本文」の状態機械で認識する。ランニングヘッダ/ページ番号（`104 A. Suzuki`・`123`）や記号主体の display 数式は、誤って見出しにせず `unknown_block`/`paragraph` に留めるガードを入れた（`front_matter`/`list`/`list_item`/`footnote`/`citation`/`code_block` は enum 済・認識は後続で拡充）。tree は `document > page > block > line`。

**Phase 3 実装状況（数式表層）**: 独立した数式ブロックを検出して `display_math` にし（`ingestion/structure::detect_display_math`）、`math_expressions`（migration 0016）に表層表現を 1 行作る。検出は強い数式記号（`= − ∈ ∞ ≤ →` 等の**タイポグラフィ記号**で ASCII ハイフン/`x` と区別）+ 短いブロック + 散文優位でないこと、で保守的に判定（演算子が飛んだ純英字の式は拾わない＝欠損を許容）。数式番号 `(2.1)` を抽出し、pdfium の制御文字グリフ化けを除去する。**PDF からは LaTeX/MathML を確実に復元できないので `semantic_status='surface_only'` + `normalized_text`（Unicode 線形）のみ**。本物の LaTeX は Phase 4（TeX 取込）、Content MathML/OpenMath/AST は Phase 7（意味）。`inline_math`（本文中の数式スパン）・`equation_group` は enum 済・認識は後続。実 PDF（Suzuki 2016）で display_math 93 件・数式番号抽出・制御文字除去を確認。

**Phase 4 実装状況（TeX 取込・`ingestion/tex/`・純関数で CI テスト可能）**: arXiv e-print（gzip された tar か単一 .tex）を `download_arxiv_source` で `application/gzip` 添付として保存し、`build_lcir_for_attachment` が **mime だけ**で抽出器を選ぶ（`%pdf%` → pdfium / `application/gzip` → **`lumencite-tex`**・独自 semver。バッチ対象クエリと同一述語・手動 .tex 添付はスコープ外）。コンテナは**メモリ内でのみ展開**（`.tex`/`.bbl`/`.ltx` だけ読み、展開合計 64 MiB 等の上限で decompression bomb とパストラバーサルを構造的に排除。非 UTF-8 は latin-1 として救済）。**字句規則**: `\[` `\]` `$` `$$` `%` `{` `}` は直前の連続バックスラッシュが偶数個の位置でだけトークンと認識する（`\\[4pt]`（改行+間隔）を display 数式 `\[` と誤認しない・`\%` 保護・`\\%` はコメント開始）。main ファイルはコメント除去後に `\documentclass`/`\documentstyle` で検出し、`standalone`/`subfiles` クラスと他ファイルから `\input` されるものを除外して選ぶ（候補ゼロなら最大の TeX らしいファイルへ degrade + warning — 旧 hep-th の plain TeX 対応）。`\input`/`\include`/`\subfile`（braceless `\input file` 含む）を include-once + 総量上限で再帰スプライスし、`\bibliography{..}` は同梱 `.bbl` へ差し替える。認識は `\title`（preamble でも本文でも可 — revtex は `\begin{document}` 後に置く）→ `front_matter`、`abstract` 環境と `\abstract{..}` コマンド形（jheppub）、`\(sub)*section`（**共有引数リーダ**が `[short]` 光学引数を消費・節番号は LaTeX カウンタを再現・`*` 付きは番号なし・**`\appendix` 後は A/B..**）、display 数式環境（`equation`/`align`/`alignat`/`flalign`/`gather`/`multline`/`eqnarray`/`displaymath`/`\[..\]`/`$$..$$` + preamble の自明な `\newcommand`/`\def` エイリアス（`\be`/`\ee` 等）— **原文スニペットをそのまま `math_expressions.latex` に保存**し `semantic_status='source_provided'`・`origin='tex_source'`。`\tag{X}` → `equation_label`・`\label` 名は payload の `labels`）、`figure`/`table` 内 `\caption`、`itemize`/`enumerate` → `list`、`verbatim`/`lstlisting` → `code_block`（内部は認識しない）、`thebibliography`（`{widest}` 引数消費・`\bibitem[..]{key}` の光学引数対応）→ `bibliography_entry`（payload に `cite_key`）。**未知環境の三分法**: 透過（`center`/`widetext`/`subequations`/`acknowledgments`/`quote` 等 + 既定の未知環境はマーカー除去して中身を解析）/ 本体破棄（`tikzpicture`/`tabular`/figure 内の非 caption 等）/ opaque（verbatim 系）。段落分割はコメント専用行を完全削除してから空行で区切り、brace 深度 > 0 では区切らない。木は `document > block` フラットで **page/line ノードと source_fragments を作らない**（TeX に PDF 座標は無い。read 面の `page`/`bbox` は null・派生 JSON の `coordinate_space` も省略）。**TeX 版は `document_nodes_fts`/`fulltext` に索引しない**（同一エントリの PDF 版と重複ヒットし bbox も無いため。検索 = PDF 版 / 読み出し = TeX 優先の分担）。read 面はエントリ解決時に `extractor_priority`（tex > pdfium）で優先し、MCP ツールの `source` 引数で切替・`available_sources` で列挙できる。`page` フィルタ指定時は PDF 版へ自動フォールバック（page は PDF 空間の概念）。**LaTeX 数式番号の完全エミュレーションはしない**（`\tag` のみ。誤った番号を確定するより欠番を許容）。インライン数式 `$..$` は本文に生 LaTeX のまま残す（独立ノード化は後続）。JATS/HTML/LaTeXML は取得経路が無いため後続（抽出器 seam は 2 抽出器の併存で実証済み）。

**Phase 5 実装状況（定理・定義・証明）**: 型付きノード `definition`/`theorem`/`lemma`/`proposition`/`corollary`/`remark`/`example`/`proof` を 2 経路で認識する（**新規テーブルなし** — 既存 `document_nodes` + `payload_json` に載る）。**TeX（`lumencite-tex` 0.2.0・原文由来・高信頼 0.95）**: preamble の `\newtheorem{env}{Display}` を回収して独自環境名・略記（`thm`/`lem`…）を表示名からノード種別に対応づけ（`\newtheorem*`・共有カウンタ `[shared]`・`{Display}[within]` 形も対応）、標準英名 + `proof`（amsthm 予約）は既定マップで拾う。`\begin{theorem}[note]` の付記名と `\label` を捕捉し、本文は 1 ブロックに collapse（`\label` は除去・内側 display 数式は生 LaTeX のまま残し別ノード化しない＝flat 統計を保つ）。**PDF（`lumencite-pdfium` 0.4.0・レイアウト由来・中信頼 0.6–0.7）**: 行頭キーワード + 番号 + 終端記号（`. : (` ダッシュ）で判定し、参照文中の "Theorem 2 shows …"（終端記号が続かない）は棄却する（誤検出より欠損）。`theorem_number`（"2.3"/"A.1"）と丸括弧の付記名を payload に載せ、参考文献モードでは検出しない。**定理間参照グラフ（proves 等）は Phase 6（`node_relations`）に委譲**し、Phase 5 は型付きノード + メタデータ（番号・付記名・label）までを担う。read 面は汎用（`is_content_block` は blacklist・node-FTS の索引対象・`get_document_blocks` の `kinds` フィルタ）なので追加改修なしで surface し、`get_document_blocks(kinds:["theorem","proof"])` で「定理と証明を一問い合わせ」が満たされる（番号・付記名も応答に付く）。

**Phase 6a 実装状況（参照グラフ・`ingestion/graph.rs`・DB 非依存の純関数）**: ノード間の参照を `node_relations`（migration 0017）に**有向辺**として張る（**新規ノード型なし** — 既存ノード間の辺）。build のトランザクション内で、Phase 5 までに永続化されたノードの軽量ビュー（`GraphNode`）から `resolve_relations` が解決する。**TeX**（`RefStrategy::Tex`・`origin='tex_source'`・confidence 0.9）: 段落等の `plain_text` に原文のまま残る `\ref`/`\eqref`/`\cite`（+ `\autoref`/`\cref` 系・biblatex cite 系）を、`\label`（`payload.labels`）/ `\bibitem` の cite key（`payload.cite_key`）と照合。参照先ノードの種別で `refers_to_equation`/`refers_to_theorem`/`refers_to_figure`/`refers_to_table`/`refers_to_section`/`refers_to` を張り分け、`\cite` は `cites`。**PDF**（`RefStrategy::Pdf`・`origin='layout_model'`・confidence 0.6）: `plain_text` 中の "Theorem 2.3" / "Eq. (2.1)" を定理番号（`payload.theorem_number`）/ 数式番号（`math_expressions.equation_label`）と照合（PDF は `\label` を復元できないため番号一致・大文字始まりのみ拾い plural/小文字は保守的に無視）。**proof → theorem の `proves`**: TeX は `\ref` 先が定理系ならそれ、無ければ読み順（ページ跨ぎの通し番号）の直前の定理系ノード。PDF は "Proof of Theorem 2.3" の番号一致（confidence 0.7）、無ければ直前。**解決できない参照（ターゲット不在）・自己参照（定理見出しが自分を指す）は張らない**（roadmap §16「誤検出より欠損」）。read 面は `LcirDocument` に文書レベルの `relations` を載せ（`get_lcir_document` / MCP）、MCP `get_node_relations`（`source` 切替・`relation_type`/`node_id` フィルタ・端点 enrich）で「この証明は何を証明するか」「式 (2.1) を参照/使用するのは何か」を一問い合わせで解ける。**記号系（記号候補抽出・"let/define/denote" 定義文認識・スコープ・記号出現）は Phase 6b（`symbols`/`symbol_occurrences`・別 migration）に分けた**（誤検出が多い領域を高精度な参照グラフと切り離す）。

**Phase 6b 実装状況（記号系・`ingestion/symbols.rs`・DB 非依存の純関数）**: 論文が定義する記号を `symbols`（migration 0018）に、その出現を `symbol_occurrences` に持つ（**新規ノード型なし**）。build のトランザクション内で `extract_symbols` が、**TeX 本文のインライン数式 `$...$` / `\(...\)`** を定義文から取り出す。定義文パターン（**強いトリガ + インライン数式が揃ったときだけ**）: `let $X$ be/denote ...`、`(we) define $X$ as/to be/by ...`、`denote by $X$ ...`、`we write $X$ for ...`、`$X$ denotes/is defined as/is called/stands for ...`、`$X := ...$`（無条件）、トリガ + `$X = ...$`。表層は先頭記号にそろえる（`$U_\beta = U_\beta(G,a)$` → `U_\beta`）。説明は文末/display 数式/長さで切り、**インライン数式は説明に含める**（`$\tau$-periodic Grover walk` 等・LaTeX 読者向け）。`symbol_type`（operator/matrix/graph/…）は説明語からの best-effort、`normalized_form` は `\mathcal`/`\hat` 等の装飾を剥いた形、`scope_node_id` は直前の節、**同一節内の同一表層の再定義は 1 個に畳む**。出現は保守的に **display 数式内の定義済み記号の表層一致**のみ（英字境界トークン一致・conf 0.5）。**PDF は対象外**（インライン数式が区切り無しで潰れる・PDF-only エントリは空）。surface/description は原文 verbatim だが対応づけはヒューリスティックなので `confidence` 中程度（0.5–0.6）・`origin='tex_source'`。read 面は `LcirDocument.symbols`（出現つき）+ MCP `get_symbol_definitions`（`symbol`/`query` フィルタ・`defined_at`/`scope`/`occurrences` を enrich）。**実 arXiv 論文（2607.14797）で 38 記号 / 31 出現**を確認（"$U_\beta$"・"the magnetic vector potential"→vector 等）。**スコープの厳密化・意味の別テーブル化・PDF 記号は後続**（Phase 7 の数式意味と接続）。

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
- **要検証**: 非ゼロ `/Rotate` ページで pdfium の text bounds が回転前/後どちらで返るか。raw bounds + `rotation_deg` の両方を保存し、消費側で合成する。テストコーパスに回転ページ PDF を必ず含める（Coordinate Test）。

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
- **派生化(B) への seam**: `ingestion::regenerate_page_fts_from_lcir(pool, version_id)` を第一段で実装・単体テストする（Phase 1 完了条件「FTS5 を削除しても LCIR から再構築できる」を証明）。ただし既定ソースにはしない。将来 (B) 化は post-attach で `index_attachment(pages)` を `regenerate_page_fts_from_lcir(version_id)` に差し替える1行で、ロードマップ §12 の「新旧品質を比較してから既定化」の後に行う。
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
    relation.rs / math.rs / figure.rs / symbol.rs   # 予約（Phase 3/6/8）
  ingestion/
    mod.rs      ★  # post_attach・build_lcir_for_attachment・regenerate_page_fts_from_lcir・lcir_enabled
    pdf/
      mod.rs    ★  # extract_document(path) -> ExtractedDocument（pdfium・spawn_blocking 下）
      pdfium.rs ★  # bind_pdfium() 集約（ocr.rs から移設して共用）
    tex/
      mod.rs  ★  # Phase 4: TeX 構造認識（parse_tex・純関数）
      source.rs ★ # Phase 4: gzip/tar のメモリ内展開・main 検出・\input 解決
    jats/ tei/ html/                                 # 予約（後続）
  db/                     # 既存 storage 層（一表一ファイル）
    document_versions.rs ★
    document_nodes.rs    ★
    source_fragments.rs  ★
  export/
    mod.rs      # Phase 9a: LCIR JSON 書き出し（validation 通過必須）
    markdown.rs # Phase 9a: LcirDocument → Markdown の決定的純関数レンダラ
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
- `docs/DATA_MODEL.md` — 既存 DB スキーマ（第一段実装時に新3表を追記）
- `docs/API_SPEC.md` — Tauri コマンド仕様（第一段実装時に新コマンドを追記）
- `docs/SPEC.md` — 機能要件・フェーズ
