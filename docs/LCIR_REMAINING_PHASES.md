# LCIR 残 Phase の棚卸し（2026-07-28 時点）

`docs/LCIR_design_overview.md` が「何をどう作るか（設計の決定版）」であるのに対し、
本書は **v0.10.0 出荷直後の時点で何が残っているか**を実コードに対して確認した記録である。

- 対象コミット: `fa215d7`（v0.10.0 + バックアップ修正までマージ済み）
- 確認方法: 5 領域（Phase 7 / 8d / 9b / 10 / v1.0.0 前提）を並列調査エージェントがソース・migration・`Cargo.toml`・実ライブラリ DB に当たって確定。
  「積み残し債務」節のみ調査を中断したため、対話中の直接 grep で埋めた（他節より精度が粗い）。
- **点在する実測値は調査時点のもの**。再着手時は再計測すること。

実装済み Phase（0/1/2/3/4/5/6a/6b/8a/8b/8c/9a）の内容は設計概観 §3・§5.4 を参照。

---

## 1. 残 Phase 一覧

難易度は S（数百行以下・1 PR）/ M（~500 行・1 PR）/ L（~1000 行超 or 新表+UI）/ XL（自前パーサ級 or 複数 PR）。

| Phase | 内容 | 難易度 | 依存 | migration | 状態 |
|-------|------|--------|------|-----------|------|
| **7a** | 数式 AST + α 正規化 + エントリ内部分式検索 | **XL** | 3,4（済） | 不要 | 未着手（器のみ） |
| **7b** | 横断部分式索引・数式類似度検索 | **L** | 7a | 0021 | 前提未達 |
| **7c** | Content MathML / OpenMath / Presentation MathML + 型推定 + 確認 UI | **XL** | 7a, 6b | 0022 | 未着手 |
| **8d-1** | 元画像ストリーム保存（`role='original'`） | **M** | — | 不要 | 未着手 |
| **8d-2** | ベクター図（tikz/pgf）の領域検出 + crop | **L** | — | 不要 | 未着手 |
| **8d-3** | SVG 抽出 | **XL** | 8d-2 | 不要 | 未着手（非目標化を検討） |
| **8d-4** | 図の分類（plot / diagram / photo） | **M** | — | 0021 | 未着手 |
| **8d-5** | plot 軸・凡例・系列 / diagram の node-edge | **L** | 8d-4 | 8d-4 と共有 | 未着手 |
| **8d-6** | PDF 側の表認識 | **L**（Vision）/ **XL**（幾何復元） | 8d-2 | 不要 | 未着手 |
| **8d-7** | 本文 → 図表参照の解決（PDF `refers_to_figure`/`refers_to_table`） | **S** | — | 不要 | **実装済**（2026-07-28） |
| **8d-8** | 8a の既知の穴（XObjectForm・回転ページ） | **M** | — | 不要 | 意図的未対応 |
| **9b-0** | 共有基盤（XML エスケープ・平坦木→包含再構成・`ExportHeader`） | **M** | 9a | 不要 | 未着手（欠落警告は debt-8 で実装済） |
| **9b-M** | Presentation MathML の供給元決定 | **S〜XL** | (7a) | 不要 | **分岐点** |
| **9b-1** | HTML + MathML | **M〜L** | 9b-0 | 不要 | 未着手 |
| **9b-2** | JATS XML | **L** | 9b-0 | 不要 | 未着手 |
| **9b-3** | TEI XML | **M** | 9b-0, 9b-2 | 不要 | 未着手 |
| **9b-4** | Web Annotation / JSON-LD 領域注釈 | **S** | 9a のみ | 不要 | 未着手 |
| **10a** | 文脈バンドル（`get_node_context`） | **M** | 6a/6b/8a/8c（済） | 不要 | 未着手 |
| **10b** | チャットへの LCIR 露出 + provenance 付き回答 | **M〜L** | 10a | 不要 | 未着手 |
| **10c** | embedding / ベクトル検索 / 文献横断グラフ | **XL** | 10a, 7 | 2 表以上 | 前提未達 |
| **v1.0.0-p0** | pdfium を Windows / Linux に同梱 | **M** | — | 不要 | **実装済**（2026-07-28・実機検証は次回リリース時） |
| **v1.0.0-p1** | FTS 派生化（`pdf_extract` → LCIR page ノード） | **M** | p0 | 不要 | seam のみ |
| **v1.0.0-p2** | LCIR の自動 build（添付時・バックフィル） | **M**（〜L） | p0, p1 | 不要 | 未着手 |
| **v1.0.0-p3** | `lcir.enabled` 既定 ON + 外部通信の同意分離 | **M** | p0, p2 | 不要 | 未着手 |
| **v1.0.0-p4** | superseded 版の GC + 容量可視化 | **M** | — | 不要 | 未着手 |

---

## 2. Phase 7（数式意味表現）

**器は完成・中身はゼロ。** `math_expressions` の `ast_json` / `presentation_mathml` / `content_mathml` /
`openmath_json` は migration `0016` に列が存在し（`0016_math_expressions.sql:18-22`）、
INSERT も 12 列すべてを bind 済み（`db/math_expressions.rs:30-46`）。
しかし書込側は PDF 経路・TeX 経路の両方が `None` 固定（`ingestion/mod.rs:559`, `:927`）で、
**実 DB 32,508 行すべてが NULL**。

**入力素材が薄い。** `latex` が入っているのは TeX 由来の **645 行のみ**（`source_provided`）。
残り 31,863 行は PDF 由来の `surface_only` で LaTeX を持たない。
つまり Phase 7 の実効価値は **TeX 取得率にそのまま比例する**。

**自前パーサが確定。** `Cargo.toml` に LaTeX/MathML 系クレートは無い
（latex2mathml / pulldown-latex は Presentation MathML のみで AST 不可、texmath は Haskell）。
規模の比較対象は `ingestion/tex/tabular.rs` の 1,019 行（本体 ~600 + テスト ~420）で、
それは 2 次元 grid の分割だけを扱う。数式は演算子優先順位・上下付き・`\frac`/`\sqrt`・
大型演算子・区切り記号・行列環境を扱うため確実にこれを上回る。

| 分割 | 内容 | 難易度 | 概算行数 | migration |
|------|------|--------|----------|-----------|
| 7a | LaTeX → AST、α 正規化、正準ハッシュ、エントリ内部分式検索 | XL | 1,800–2,800 | 不要 |
| 7b | 横断部分式索引（`math_subexpressions`）・類似度検索・MCP `search_math` | L | 700–1,100 | `0021` |
| 7c | Presentation/Content MathML・OpenMath・型推定・曖昧性解消・確認 UI | XL | 2,000–3,200 | `0022`（satellite） |

**設計上の決めごと（着手時に確定させる）**

- `math_expressions` に **UPDATE 経路が存在しない**（`insert_math` と `math_for_version` のみ）。
  AST を build 内で埋めるか、後追いバッチ + UPDATE 経路の新設かを先に決める。
- 併置 `AB` は中立ノード（積/適用を確定しない・§16）。parse 失敗は `ast_json` NULL（欠損許容）。
- 7a では migration を切らず、正準形は `ast_json` のエンベロープ内
  （例 `{"ast":…,"alpha":…,"alpha_sha256":…}`）に入れる。本リポジトリは `json_extract`/`json_each` を
  一切使わず JSON は Rust 側で parse する規約なので、JSON 列を検索キーにするのは筋が悪い。
  横断索引は 7b の実表に切り出す。
- 7c のユーザー修正は **satellite 表**が要る。理由は `node_alt_texts`（8c）と完全に同型:
  ① `math_expressions` は build ごとに insert し直す ② 再構築で id が変わるので版跨ぎの持ち越しキーが要る
  ③ `math_expressions.confidence` は「表層検出の確からしさ（意味の確からしさではない）」と
  `0016_math_expressions.sql:24` が明記しており、意味の provenance を混ぜられない。
  持ち越しキーは 8c の crop sha256 に相当するものとして 7a の正準 AST ハッシュを使える。
- `MathSemanticStatus` の `NotAttempted` / `Inferred` / `Verified` と `Origin::MathRecognition` は
  enum に定義済みだが **どこからも構築されていない**（実在は `SurfaceOnly` と `SourceProvided` のみ）。

ロードマップ §10 Phase 7 の完了条件 4 件は **0 達成**（API 面のみ部分達成）。

---

## 3. Phase 8d（図表の残り）

**実測で効くのは 8d-2。** `ingestion/pdf/mod.rs:172-175` が
`if object.as_image_object().is_none() { continue; }` で path/text オブジェクトを無条件に捨てているため、
ベクター図（tikz/pgf）は図領域にならない。設計上は「アセット 0 件が正当」だが、実ライブラリでは
**pdfium 版 138 件中 69 件（50%）が figure ノード 0 件**、
**PDF 側 figure_caption 904 件に対し `caption_of` 辺は 262 件のみ**（642 件のキャプションが図に結びついていない）。

**8d-7 は実装済（2026-07-28）。** `RefCategory::{Figure,Table}` + `FloatTargets` 索引 + `graph_nodes` への
figure ノード push で、PDF 本文の "Figure 3" / "Fig. 3" / "Table 2" を図表番号と照合するようになった
（`extractor_version` 0.6.0 → 0.7.0・migration 不要）。詳細は `LCIR_design_overview.md` の「Phase 8d-7 実装状況」。

**実測で分かった重心**: 解決先は **caption ノード宛が主経路**（`figure` ノードの図番号保有率は 261/1198 = 21.8%・
PDF 側に `table` ノードは 0 件）。全体の約 65% が caption 宛になる。**解決率の実測上限は約 93%** で、
残り 7% は被参照キャプションが `figure_caption` と認識されず paragraph に落ちているケース（下記 debt-12）。
参照スキャナ側では回収できない。

**TeX 側の `refers_to_figure` は caption ノード宛のままで正**（`graph.rs` の `relation_type_for_target`）。
TeX 経路は `figure` ノードを作らないので他に指す先が無い。TeX の `\ref{tab:..}` を `table` ノードへ
リダイレクトする案は却下した — `resolve_relations` は純関数で `caption_of` を知らず（`env_group` は `GraphNode` に無い）、
出荷済みの `to_node_id` が変わるうえ、LLM には本文を持つ caption の方が有用なため。

| 分割 | 内容 | 難易度 | 概算行数 | 備考 |
|------|------|--------|----------|------|
| 8d-1 | 元画像ストリーム保存（`role='original'`） | M | ~300 | 列追加不要（`0019_assets.sql:32` が role を予約済み）。全 PDF 再構築と容量増が運用コスト |
| 8d-2 | ベクター図の領域検出 + crop | L | 700–900 | path 列挙 + クラスタリング純関数。`origin='layout_model'` の confidence を画像由来と別値に |
| 8d-3 | SVG 抽出 | XL | 1,200+ | 依存クレート皆無（svg/usvg/resvg なし）。代替: TeX 同梱の原図を `role='vector'` で保存すれば 300–400 行 |
| 8d-4 | 図の分類（plot/diagram/photo） | M | ~450 | `llm/ocr.rs:16-24` の alt text プロンプトが既に図種別を第 1 文で要求済み。satellite 表が要る |
| 8d-5 | plot 軸/凡例/系列・diagram node-edge | L | ~700 | 8d-4 と同一 migration を共有 |
| 8d-6 | PDF 側の表認識 | L（Vision）/ XL（幾何） | 700 / 1,200+ | 8b の `table` payload 契約にそのまま合わせられる。実 DB の table ノード 15 件は全て TeX 版・PDF 側は 0 件（table_caption は 68 件ある） |
| 8d-7 | 本文 → 図表参照の解決 | S | ~250 | **実装済**（上記） |
| 8d-8 | XObjectForm 内画像・回転ページ | M | ~400 | form の `matrix()` 連鎖を子矩形に適用してページ空間へ変換（pdfium-render 0.8.37 に `matrix()` あり）。回転ページは現在 skip + warning |

8d-3（SVG）は §16 の非目標に最も近い。**部分的に非目標として明文化する**判断もありうる。

---

## 4. Phase 9b（標準形式エクスポート）

**拡張点は安い。** `export/mod.rs` は 73 行で、追加は `pub mod` 1 行。
`load_entry_lcir` / `validation` / `LcirDocument` 派生ビューは 9a で共有化済み。

**本丸は木の再構成。** PDF 側は `section` が `paragraph` を含まない **平坦木**
（block が page 直下・`ingestion/mod.rs:498`）。XML は包含構造を要求するので、
`payload_json` の heading_level から節木を組み直す再構成器が 9b-0 として全形式の共通前提になる。

**XML writer 未導入。** `Cargo.toml` に quick-xml / xml-rs は無い。依存追加が要る
（CI の `cargo audit` / clippy `-D warnings` への影響を確認すること）。

| 分割 | 内容 | 難易度 | 概算行数 | 依存 |
|------|------|--------|----------|------|
| 9b-0 | XML エスケープ・包含再構成・`ExportHeader` | M | 200–280 + テスト 160–200 | 9a |
| 9b-M | Presentation MathML の供給元決定 | S〜XL | 0–120 | (7a) |
| 9b-1 | HTML + MathML（単一ファイル・図つき） | M〜L | 350–500 + テスト 250–350 | 9b-0 |
| 9b-2 | JATS XML | L | 450–650 + テスト 300–400 | 9b-0 |
| 9b-3 | TEI XML | M | 300–400 + テスト 200–250 | 9b-0, 9b-2 |
| 9b-4 | Web Annotation / JSON-LD 領域注釈 | S | 120–200 + テスト 100–150 | **9a のみ** |

**9b-M が分岐点。** `presentation_mathml` は全書込経路で NULL 固定なので、9b が数式をどう出すかは
(a) Phase 7 を待つ / (b) 変換クレートを導入 / (c) 生 LaTeX のまま出す の 3 択。
**(c) を選べば 9b は Phase 7 に依存しない**。7c と 9b は依存が逆向き（9b が Presentation MathML を欲しがる）なので、
9b を先に出すなら (c) と決めておくと二度手間にならない。

**9b-4 は独立して先行できる**（9b-0 も 9b-M も不要）。必要データ（page + bbox + `CoordinateSpace`）は
Phase 1 から揃っている。

JATS は「LCIR 固有の信頼度・PDF 座標・抽出履歴は JATS 外に保持してよい」と
ロードマップ §9.1 が明記しており、完全対応は不要。
TEI は原典 §9.2 が「GROBID の解析結果を受け取る**入力**形式」と位置づけており、出力の優先度は最も低い
（TEI インポートが無い現状ではテスト戦略 §13.4 の round-trip テストが書けない）。

---

## 5. Phase 10（LLM・エージェント向け）

**エージェント表面の現状（調査時点）**: MCP サーバー 17 ツール / アプリ内チャット 11 ツール / CLI 12 サブコマンド。
**チャットには LCIR 系ツールも `get_fulltext` も一切露出していない**（`llm/tools/mod.rs`）。
provenance 付き回答の入口が構造的に存在せず、issue #42（索引済み PDF を再 OCR する穴）も現存する。

| 分割 | 内容 | 難易度 | 概算行数 | migration |
|------|------|--------|----------|-----------|
| 10a | 文脈バンドル（定理 + 前提定義 + 証明 + 参照数式を server-side 結合）| M | 500–700 | **不要** |
| 10b | チャットへの LCIR ツール露出 + provenance 付き回答 | M（バックエンド）/ L（bbox ハイライト UI 込み） | 900–1,400（+UI 400–600） | 不要 |
| 10c | embedding / ベクトル検索 / 文献横断グラフ | XL | 2,500+ | 2 表以上 |

**10a は新表も新推定器も不要。** 既存 7 表（`document_nodes` / `source_fragments` / `math_expressions` /
`node_relations` / `symbols`+`symbol_occurrences` / `assets`+`node_assets` / `node_alt_texts`）からの導出のみで、
永続化すべき新事実が無い。`get_lcir_node_region` は実装済み。`#[sqlx::test]` で CI 完結する。

**10c は切り離す。** embedding・ベクトルの基盤はコード・DB・依存のすべてでゼロ。
加えて「文献横断の引用グラフ」は `bibliography_entry` → ライブラリ `entries` の解決器という
新規推定器が必要で、既存データの結合では作れない。単一 PR に収まった LCIR の最大は
Phase 8b（#59・2,424 insertions）で、10c は確実にそれを超える。**post-1.0 に置く。**

---

## 6. v1.0.0 の前提（`lcir.enabled` 既定 ON）

### v1.0.0-p0 — pdfium の Windows / Linux 同梱（**2026-07-28 実装済**）

**調査時点の問題**: pdfium 動的ライブラリは macOS にしか同梱されておらず
（`release.yml` の取得ステップが `if: startsWith(matrix.platform, 'macos')`）、
LCIR 抽出は OCR と同じ `bind_pdfium()` を通るため、**Win/Linux では `lcir.enabled` を ON にしても
LCIR build が動かない**状態だった。既定 ON（p3）も FTS 派生化（p1）も、これを解かないと macOS 限定機能になる。

**対応**（詳細は `docs/RELEASE.md` §4）:

- `tauri.release-linux.conf.json` を新設し `bundle.resources` で `libpdfium.so` を同梱。
  `release.yml` の Linux ジョブに取得 + SHA-256 検証ステップを追加し、matrix args に `--config` を通した。
- `tauri.release-windows.conf.json` に `bundle.resources` で `pdfium.dll` を追加。
  Windows は CI 非対象（Certum SimplySign の対話ログイン要件）なので、VM 上の手動配置手順を
  `RELEASE.md` §4 に SHA-256 付きで記載した。
- `bind_pdfium()` の探索候補を純関数 `library_search_dirs()` に切り出し、Linux のリソースディレクトリ
  （`<exe>/../lib/<name>`）を追加。`<name>` は productName / crate 名 / バイナリ名のどれになるか
  配布形態依存のため 3 つとも探索する。単体テスト 6 本で探索順を固定（pdfium 実体不要・CI 実行可）。
- Windows は Tauri のリソース配置が exe と同じディレクトリなので、既存の探索候補で足りる（Rust 変更なし）。

**残る検証**: 実際に `.deb` / `.AppImage` / `.msi` を作って
`/usr/lib/<name>/libpdfium.so` と exe 隣の `pdfium.dll` が入ることを確認するのは**次回リリースビルド時**。
macOS 経路は変更していない。

### v1.0.0-p1 — FTS 派生化

設計概観 §8 の「差し替えは 1 行」は実コードとずれている。`regenerate_page_fts_from_lcir` の seam
（`ingestion/mod.rs:1277-1306`）は存在するが、実際には OCR 由来行の保護・`pdf_extract` フォールバック維持・
呼び出し元 5 箇所の書き換え・新旧の検索品質比較が要り、**300–450 行**。

「この `fulltext` 行が OCR 由来か LCIR 由来か」を列で持つのは FTS5 仮想表の作り直し（全再索引を伴う破壊的
migration）になるので採らない。「LCIR テキストが空のページは既存行を残す」非破壊更新で回避する。

### v1.0.0-p2 — LCIR の自動 build

PDF は完全に手動（設定 → データのボタン）。TeX ソースだけ 3 経路で自動 build 済み。
`post_attach` への組み込み（p1 と共有）+ 直列キュー/セマフォ + 起動時バックフィル（間引き付き）で 250–450 行。
同時実行制御と起動時掃引まで入れると L。

### v1.0.0-p3 — 既定 ON + 同意分離

判定式（`ingestion/mod.rs:28-36`）の反転自体は 1 行だが、既定 OFF 前提のテスト更新が 10 箇所程度。
本丸は **TeX 自動取得（外部通信）が `lcir.enabled` に抱き合わせになっている**こと。
`lcir.vision_alt_text.enabled` と同じく別フラグ（例 `lcir.tex_autofetch.enabled`）へ分離する。
既存ユーザーの「明示 OFF」と「未設定」を区別する必要があるため、`set_lcir_enabled` が `"0"`/`"1"` を
書く現仕様は維持すること。

### v1.0.0-p4 — superseded 版の GC

supersede は status を更新するだけで行は永久に残る。`document_nodes`/`source_fragments`/`assets`/
`node_alt_texts` はすべて `ON DELETE CASCADE` なので `DELETE FROM document_versions` だけで木ごと消える
（接続時 `foreign_keys=ON` は設定済み）。ただし **8c の alt text carry を壊さない条件付け**が要る。

---

## 7. 積み残し債務（完了扱い Phase の中の未実装）

この節のみ並列調査を中断したため、対話中の直接 grep で埋めた。他節より精度が粗い。

| id | 内容 | 難易度 | 後続の前提か |
|----|------|--------|--------------|
| debt-1 | `InlineMath` / `EquationGroup` を全経路で生成しない（TeX はインライン数式を本文に生 LaTeX のまま残す） | M | **Phase 7 の入力範囲を決める** |
| debt-2 | `ListItem` / `Footnote` / `Citation` を生成しない（`List` は TeX のみ・PDF 側は 0） | S〜M | 9b の構造品質 |
| debt-3 | `TextBlock` ノード型が未使用（実際の木は `document > page > block > line`） | S | — |
| debt-4 | JATS / HTML / LaTeXML 取込なし（`ingestion/{jats,tei,html}` 不在）・手動 `.tex` 添付もスコープ外 | L | 9b と対称 |
| debt-5 | PDF 側の記号抽出なし（6b は TeX のみ）・記号スコープの厳密化 | L / M | 7c の曖昧性解消 |
| ~~debt-6~~ | ~~9a Markdown で figure が完全に落ちている~~ **解消**（2026-07-28・存在マーカー + alt text。画像リンクは張らない） | S | 9b-1 |
| debt-7 | ~~8c の alt text が Markdown エクスポートに出ない~~（2026-07-28 解消）・**手編集 UI なし**（`origin` 列だけ用意済み。UI が入ると `user_edited` の出し分けが実データで初めて発火する） | S | — |
| ~~debt-8~~ | ~~9a の「LCIR 固有情報が失われる場合の警告」未実装~~ **解消**（2026-07-28・`export/warning.rs`・6 コード・9b と共有） | S | 9b-0 で一緒に |
| debt-9 | 回転ページは図領域を skip。設計概観 §6 の「要検証」（非ゼロ `/Rotate` での pdfium bounds）が未消化 | M | 8d-8 |
| debt-10 | 8b の longtable / tabu / siunitx S 列 / 表脚注 | M | — |
| debt-11 | 数式検索は trigram 部分一致のみ。**TeX 版は node-FTS 対象外**なので原文 LaTeX は検索対象ですらない | — | 7b の動機 |
| debt-13 | 実蔵書では `figure` 1,198 件のうち 523 件（43.7%）、alt text 888 件のうち 523 件（58.9%）が**スキャン本 1 冊**由来で、その alt text は図ではなく「スキャン頁」の説明。8c の課金の 6 割弱がここに費やされた計算になり、debt-6 の Markdown 出力もこの 1 冊では 523 個のマーカーになる。判定候補は「ページに block が 0 件」「crop が MediaBox のほぼ全面」。レンダラ側では解かない（ヒューリスティックを持ち込まない）ので 8a（領域検出）か 8c（生成対象の絞り込み）の担当 | M | 8c の費用対効果 |
| debt-12 | ローマ数字番号・全大文字の表 caption を `detect_caption` が拾わない（`lower.starts_with("table")` は通るが直後 6 文字以内に ASCII 数字が要るため `TABLE I. …` が paragraph 落ち・実測 60 件）。同種の穴で `Fig. 8.3 …` 形も落ちており、これが 8d-7 の解決率上限 93% の実体 | S〜M | 8d-7 の回収率 |

`NodeKind` は 29 種定義されているが、生成経路を持つのは PDF 18 種 / TeX 22 種。
`ListItem` / `Footnote` / `Citation` / `InlineMath` / `EquationGroup` / `TextBlock` はどちらも生成しない。

---

## 8. 推奨着手順序

設計概観 §3 の推奨（8 → 7 → 9b → 10）は維持できるが、
**v1.0.0 の看板（Phase 9a/10 到達 + `lcir.enabled` 既定 ON）を取りに行くなら Phase 7 は経路上にない。**
7 は最大の XL であり、かつ入力となる原文 LaTeX が 645 行しかない。

1. ~~**v1.0.0-p0（pdfium 同梱）**~~ — **2026-07-28 実装済**。他すべての前提だった
2. ~~**8d-7（S）+ debt-6（S）+ debt-8（S）**~~ — **2026-07-28 すべて実装済**。9b-1 と 10a の質が上がる
3. **10a（M）→ 10b（M）** — v1.0.0 の「Phase 10 到達」はここ
4. **v1.0.0-p1 → p2 → p3（各 M）** — 既定 ON 化。p4（GC）は並行可
5. **8d-2（L）** — 実ライブラリの半数が figure 0 件という実害の解消。再構築 1 回で 8d-1 / 8d-8 と束ねる
6. post-1.0: **9b-4（S）→ 9b-0 → 9b-1 → 9b-2**、そして **Phase 7a → 7b → 7c**

**v1.0.0 までの最短経路** = ~~p0 → 安い債務 3 件~~（済） → **10a/10b** → p1/p2/p3。いずれも M 以下。
**post-1.0 に回すもの** = Phase 7 一式・8d-3・8d-6・10c・9b（9b-4 を除く）。

---

## 関連ドキュメント

- `docs/LCIR_design_overview.md` — 設計の決定版（実装済み Phase の詳細はこちら）
- `docs/LumenCite_machine_readable_document_roadmap.md` — 元ロードマップ（Phase ごとの実装項目・完了条件）
- `docs/RELEASE.md` — §4 に pdfium 同梱の手順
