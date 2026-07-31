# LCIR 残 Phase の棚卸しと v1.0.0 の実装順序（2026-07-31 改訂）

`docs/LCIR_design_overview.md` が「何をどう作るか（設計の決定版）」であるのに対し、
本書は **v1.0.0 に何を載せ、どの順で入れるか**を実コードと実 DB に対して確定させた記録である。

- 対象コミット: `37e24fa`（Phase 10b までマージ済み）
- 初版: 2026-07-28（v0.10.0 出荷直後の残 Phase 調査）
- 改訂: 2026-07-31（v1.0.0 のスコープ確定 + 実装順序の確定 + 実測値の全面再取得）
- 確認方法: 15 体の並列調査エージェントが 9 領域をソース・migration・`Cargo.lock`・vendored crate・
  実 PDF・実ライブラリ DB（読み取り専用）に当たり、3 レンズで順序を起案、2 種の批評で検証した。
  **初版の実測値・行番号のうち 12 箇所が実コードと食い違っていた**（§10 に一覧）。

**点在する実測値は 2026-07-30 時点のもの。再着手時は再計測すること。**
実装済み Phase（0/1/2/3/4/5/6a/6b/8a/8b/8c/9a/10a/10b）の内容は設計概観 §3・§5.4 を参照。

---

## 0. v1.0.0 のスコープ（2026-07-30 決定）

v1.0.0 の看板は「LCIR 完成（Phase 9a/10 到達 + `lcir.enabled` 既定 ON）」であり、
リリース条件は 3 段ゲート（①スコープ消化 → ②別モデルでリポジトリ全体コードレビュー → ③指摘修正）。

**スコープ = 「中間」**。ロードマップ全消化（+約 14,000〜18,000 行）と、既定 ON 化だけ（p1〜p4）の中間を採る。

| 区分 | 項目 |
|------|------|
| **v1.0.0 に載せる** | `p1` FTS 派生化 / `p2` 自動 build / `p3` 既定 ON + 同意分離 / `p4` superseded GC + 容量可視化 |
| | 図の実害の一括解消: `8d-2` ベクター図 / `8d-1` 元画像保存（条件付き）/ `8d-8` XObjectForm・回転頁 / `debt-14` CropBox クランプ |
| | `debt-12` caption 取りこぼし / `9b-4` Web Annotation（条件付き） |
| **post-1.0** | Phase 7 一式 / `8d-3` SVG / `8d-4` `8d-5` 図の分類と構造化 / `8d-6` PDF 表認識 / `9b-0`〜`9b-3` / `10c` embedding |

**post-1.0 に回す理由は 4 種類ある**（単に量が多いから、ではない）。

1. **入力データが無い** — Phase 7。`math_expressions` 32,508 行のうち LaTeX を持つのは TeX 由来の 645 行のみ。
   自前パーサ 4,500〜7,100 行を書いても効くのは蔵書の 2%。ボトルネックは実装ではなく TeX 取得率。
2. **検証手段が無い** — `9b-3`（TEI）。元ロードマップ §9.2 が TEI を「GROBID の結果を受け取る**入力**形式」と
   位置づけており、TEI インポートが無い現状ではテスト戦略 §13.4 の round-trip テストが書けない。
3. **基盤ゼロ + 新規推定器** — `10c`。embedding はコード・DB・依存のすべてでゼロ。加えて
   `bibliography_entry` → ライブラリ `entries` の解決器という新しい推定器が要る。
4. **非目標に近い** — `8d-3`（SVG）。依存クレート皆無（svg/usvg/resvg なし）で 1,200 行超。設計 §16 の非目標に最も近い。

**`8d-2` だけは上のどれにも当たらない**（構造的障害も検証不能性も無く、単に未着手）。
実測で pdfium 版 138 件中 69 件（50%）が figure ノード 0 件という実害があるため中間スコープに入れた。

**「完成度を上げる」と「既定 ON」は引っ張り合う。** 推定器を 1 つ増やすたびに黙って間違う面が増える。
したがって XL の意味層を足すより、**既に黙って間違っている所を潰す**方が v1.0.0 の質に直結する
（`debt-14` は出荷済みコードの無言のバグ）。

**post-1.0 に回すものは SPEC.md か設計概観に「v1.0.0 の LCIR が何をしないか」として明文化すること。**
本書の中だけの記述では、「LCIR 完成」の看板が誇大になる。

---

## 1. 残 Phase 一覧

難易度は S（数百行以下・1 PR）/ M（~500 行）/ L（~1000 行超 or 新表+UI）/ XL（自前パーサ級 or 複数 PR）。
**行数は 2026-07-30 の再見積り**（初版から変わったものは太字）。

| Phase | 内容 | 難易度 | 行数 | migration | 版 bump | 状態 |
|-------|------|--------|------|-----------|---------|------|
| **7a** | 数式 AST + α 正規化 + エントリ内部分式検索 | XL | 1,800–2,800 | 不要 | pdfium/tex | post-1.0 |
| **7b** | 横断部分式索引・数式類似度検索 | L | 700–1,100 | 0021 | — | post-1.0 |
| **7c** | Content MathML / OpenMath + 型推定 + 確認 UI | XL | 2,000–3,200 | 0022 | — | post-1.0 |
| **8d-1** | 元画像ストリーム保存（`role='original'`） | M | **400–520** | 不要 | pdfium | **v1.0.0（条件付き）** |
| **8d-2** | ベクター図（tikz/pgf）の領域検出 + crop | L | **800–1,050** | 不要 | pdfium | **v1.0.0** |
| **8d-3** | SVG 抽出 | XL | 1,200+ | 不要 | — | post-1.0（非目標化を検討） |
| **8d-4** | 図の分類（plot / diagram / photo） | M | ~450 | 0021 | — | post-1.0 |
| **8d-5** | plot 軸・凡例・系列 / diagram の node-edge | L | ~700 | 8d-4 と共有 | — | post-1.0 |
| **8d-6** | PDF 側の表認識 | L（Vision）/ XL（幾何） | 700 / 1,200+ | 不要 | — | post-1.0 |
| **8d-7** | 本文 → 図表参照の解決 | S | ~250 | 不要 | 0.6.0→0.7.0 | **実装済**（未再構築・§4） |
| **8d-8** | XObjectForm 内画像・回転ページ | M | **400–520** | 不要 | pdfium | **v1.0.0** |
| **9b-0** | 共有基盤（XML エスケープ・平坦木→包含再構成） | M | 360–480 | 不要 | — | post-1.0 |
| **9b-M** | Presentation MathML の供給元決定 | S〜XL | 0–120 | 不要 | — | post-1.0（分岐点） |
| **9b-1** | HTML + MathML | M〜L | 600–850 | 不要 | — | post-1.0 |
| **9b-2** | JATS XML | L | 750–1,050 | 不要 | — | post-1.0 |
| **9b-3** | TEI XML | M | 500–650 | 不要 | — | post-1.0 |
| **9b-4** | Web Annotation / JSON-LD 領域注釈 | **M**（初版は S） | **650–870** | 不要 | なし | **v1.0.0（条件付き）** |
| **10a** | 文脈バンドル（`get_node_context`） | M | — | 不要 | — | **実装済**（PR #68） |
| **10b** | チャットへの LCIR 露出 + provenance | M〜L | — | 不要 | — | **実装済**（PR #69） |
| **10c** | embedding / ベクトル検索 / 文献横断グラフ | XL | 2,500+ | 2 表以上 | — | post-1.0 |
| **v1.0.0-p0** | pdfium を Windows / Linux に同梱 | M | — | 不要 | — | **実装済**（実バンドル検証は未了） |
| **v1.0.0-p1** | FTS 派生化（`pdf_extract` → LCIR page ノード） | M | **900–1,250**（最小 400–480） | **要判断**（0021） | なし | **v1.0.0** |
| **v1.0.0-p2** | LCIR の自動 build（添付時・バックフィル） | **L**（初版は M〜L） | **420–620** | 不要 | なし | **v1.0.0** |
| **v1.0.0-p3** | `lcir.enabled` 既定 ON + 外部通信の同意分離 | M | **300–360** | 不要 | なし | **v1.0.0** |
| **v1.0.0-p4** | superseded 版の GC + 容量可視化 | M | **600–800** | 不要 | なし | **v1.0.0** |
| **debt-12** | ローマ数字・全大文字の caption 取りこぼし | S | **130–200** | 不要 | pdfium | **v1.0.0** |
| **debt-14** | 図領域のクランプが CropBox 原点を無視 | S | **160–220** | 不要 | pdfium | **v1.0.0** |

**中間スコープに migration は 0 件**（`p1` の OCR 保護に側表 `0021` を採る場合を除く）。
migration 20 本すべてに当たって確認した。0 件なら v1.0.0 のロールバックは旧バイナリへの差し戻しだけで成立する。

---

## 2. v1.0.0 の実装順序（確定）

### 2.1 前提: 版 bump と再構築は別物

順序設計の土台は次の 3 つ。すべて実コードで確認した。

- **`content_key` の `config_hash` は全経路 `""` 固定**（`ingestion/mod.rs:146`）。
  したがって `RENDER_TARGET_WIDTH` / `MIN_DIM_PT` / `MERGE_GAP_PT` などを変えても content_key は動かず、
  手で `EXTRACTOR_VERSION` を上げない限り既存版は再構築されない。
- **版 bump は再構築を誘発しない。** `build_missing_lcir` / `rebuild_outdated_lcir` の呼び出し元は
  `SettingsModal.tsx:853` の 1 箇所（ボタン）だけで、自動経路は存在しない。
- **逆に版を据え置くと実機 smoke が静かに no-op になる。** content_key が一致すると
  `build_lcir_for_attachment` は `reused: true` で早期 return し、**新しい抽出コードを 1 行も実行しない**
  （`ingestion/mod.rs:151-179`）。

したがって採るのは「**PR ごとに版を上げる・再構築は最後に 1 回だけ人が押す**」。
版 bump のコストは定数 1 行 + golden fixture 2 ファイルの各 1 行（`testdata/minimal_lcir.json:10` /
`structured_lcir.json:10`）で、据え置きで得るものは何も無い。

### 2.2 順序

| # | PR タイトル案 | 項目 | 難易度 | 版 | 再構築 | この位置の理由 |
|---|---|---|---|---|---|---|
| 0 | `fix(LCIR): 旧 content_key の GC に mtime 猶予 + GUI ロックの取得可否を返す` | debt-15 / debt-24 の一部 | S / 60–90 | なし | 否 | **既存の穴**。dev と配布版 v0.10.0 が同一 app data dir を共有しており、`gc_stale_asset_dirs` は猶予なしで「今回の content_key 以外」を trash へ送る。両方が build すると crop を消し合う。後回しにするとこの窓が計画全体に開いたままになる |
| 1 | `fix(LCIR): ローマ数字・全大文字の図表 caption（debt-12）` | debt-12 | S / 130–200 | →0.8.0 | 否 | **8d-2 より前が必須**。8d-2 は探索面を 6,321→477 ページに落とす caption アンカーを使い、その入力が `detect_caption` の分類。逆順だと FP gate を通した後にゲート条件の入力が変わる。10 項目で唯一 CI で完全に検証でき、版 bump 手順の予行にもなる |
| 2 | `fix(LCIR): 図領域のクランプをページ box 原点基準にする（debt-14）` | debt-14 | S / 160–220 | →0.9.0 | 否 | 図系の先頭。8d-2 / 8d-8 が追加する矩形も同じ `pdf/mod.rs:189-198` を通るので、先に直さないと新経路にバグを複製する。carry 破壊は実測 10 図・うち alt text 保持 6 件＝**carry 機構の本番初発火を最も安い賭けで踏める** |
| 3 | `feat(LCIR): XObjectForm と回転ページの図領域（8d-8 / debt-9）` | 8d-8 | M / 400–520 | →0.10.0 | 否 | 8d-2 の前。figure 0 件 + caption ありの 42 版のうち **21 版（caption 211 件）はラスタ画像を持つのに top-level 列挙に出ない＝8d-8 の担当**、純ベクターは 21 版 / caption 94 件。混ぜると 8d-2 のガード閾値を誤った母集団でチューニングする |
| 4 | `feat(LCIR): ベクター図（tikz/pgf）の領域検出（8d-2）` | 8d-2 | L / 800–1,050 | →0.11.0 | 否 | 最大の新推定器かつ**唯一 888 件の carry を全滅させうる**。他が全部正しくなってから最後に測る |
| 5 | `feat(LCIR): 埋込画像の原ストリーム保存（8d-1）`※条件付き | 8d-1 | M / 400–520 | →0.12.0 | 否 | **消費者を名指しできる場合のみ**（§4.3）。crop に触れないので carry 100% 成立し、この窓に相乗りする増分コストは実質ゼロ。切ると post-1.0 で追加するとき 80 分の再構築が 1 回増える |
| 6 | （PR なし）**ゲート ②a — 抽出器差分だけの限定レビュー** | 1〜5 の diff | — | なし | 否 | ここで出た指摘は版 bump のみで吸収でき再構築を増やさない。#8 の後に回すと、抽出器への指摘 1 件で 80 分再構築 + carry 再賭け + Vision 再課金がもう 1 回確定する |
| 7 | `feat(LCIR): superseded 版の GC と容量可視化（p4）` + **実 DB で GC を 1 回実行** | p4 | M / 600–800 | なし | 否 | **再構築の前に実行する**（§2.3）。削除対象 145 版は今 alt text 0 行 / assets 0 行＝述語がバグっても損失が実測ゼロ。加えて約 526MB を free page 化してから約 450k ノードを積むので DB が育たない |
| 8 | （PR なし）**唯一の再構築 80 分 + Vision 1 回** | — | — | なし | **実行** | 版を 5 回上げても実データは動いていない。ここで人が 1 回押す |
| 9 | `feat(LCIR): 全文検索の索引源を LCIR 優先にする（p1）` | p1 | M / 900–1,250 | なし | 否 | 再構築の**後**。側表 `0021` を採る場合、適用した瞬間に配布版 v0.10.0 が `NewerSchema`（`lib.rs:3325`）で起動不能になる。p0 の実機検証に配布版を使い終わるまで入れられない。p1 の再導出は pdfium 不要の純 SQL（秒オーダー）なので再構築に相乗りさせる利得はほぼ無い |
| 10 | `feat(LCIR): LCIR の自動 build（p2）` | p2 | L / 420–620 | なし | 否 | p1 の後。p1 が build 経路（`:160` reuse / `:224` 新版）に page-FTS を配線済みなら p2 は索引を意識しなくてよい。逆順だと p2 が pdf_extract 前提の配線を書き p1 で剥がす |
| 11 | `feat(LCIR): lcir.enabled 既定 ON + TeX 自動取得の同意分離（p3）` | p3 | M / 300–360 | なし | 否 | p2 の後でないと機能的 no-op（`lcir_readable` が false のままでチャットの LCIR ツールが出ない）。**分離と反転は同一 PR 必須**（分けると中間コミットで全 arXiv エントリに無言の e-print DL が走る） |
| 12 | `feat(LCIR): Web Annotation（JSON-LD）領域注釈（9b-4）`※条件付き | 9b-4 | M / 650–870 | なし | 否 | 純増・どこでも可。**日程が詰まったら最初に post-1.0 へ落とす候補**（Phase 9 の完了条件のどれにも必須でない） |
| 13 | （PR なし）**ゲート ②b — リポジトリ全体レビュー** + 指摘修正 PR | — | — | なし | 否 | 抽出器は ②a で通過済みなので p1/p2/p3/p4/9b-4 と横断関心事に絞れる。**抽出出力を変える指摘が出たら既定で v1.0.1 送り**（再構築 1 回を守る） |
| 14 | `release: v1.0.0` | — | — | アプリ 0.10.0→1.0.0 | 否 | 実データ・実 UI を見てからタグ（§9） |

**入れ替え可能なもの**: #0 はいつでも（早いほど得）。#1 と #2 は別ファイル・相互作用なしなので入れ替え可。
9b-4 はどこへでも。8d-1 は #2〜#4 のどこでも（page_crop に触れないため）。

### 2.3 再構築を 1 回に収める条件

再構築は**選択肢ではなく既に確定した負債**である。`EXTRACTOR_VERSION` は 8d-7（PR #67）で 0.7.0 に上がったが
実 DB の pdfium 完了版は 138 件すべて 0.6.0 で、**138/138 が既に outdated**。
`node_relations` に pdfium 由来の `refers_to_figure` は 1 本も無い（8d-7 は実データでまだ 1 件も効いていない）。

1. **PR ごとに版を上げる**（§2.1）。bump は再構築を強制しない。
2. **ゲート ②a を再構築の前に置く**。抽出器への指摘が再構築後に来ると 2 回目が確定する。
   レビュー資料として、未結合 caption を持つ 42 版だけを**コピー DB + 一時 appdir** で部分再構築し、
   未結合 caption 642 → ? と本文ページの figure 化件数を添える。
3. **8d-2 でベクタークラスタを既存ラスタ矩形と union しない。** union すると論文図に必ず隣接する
   軸・枠線・引出線とマージして既存 figure の bbox が動き、carry 破壊が debt-14 の 6 件（≒$0.04）から
   最大 888 件（≒$6・≒105 分）へ 2 桁跳ねる。union が避けられないと判明した時点で、**同じ PR に**
   幾何フォールバック carry（同一ページ・IoU ≥ 0.98 の旧 figure から引き継ぐ・migration 不要・~100 行）を足す。
4. **#8 の直前に att40（figure 287 / alt text 73）だけをパイロット再構築**して carry 件数を数える。
   888 件をいきなり賭けない（att37 でも同じ確認はできるが単独 75 分）。
5. **`heal_missing_assets` の穴を図系 PR 群のどこかで塞ぐ**（debt-16・~40 行 + テスト）。
   塞がないと、開発期間中に crop が 1 枚欠けた添付を build した時点で無言の carry 破壊が仕込まれ、
   原因から数週間離れた再構築時に「8d-2 のバグ」と誤診される。
6. **再構築の完了条件は `failed == 0`。** `run_build_batch` は 1 添付の失敗を eprintln と `failed` カウントに
   落として続行するだけなので、20 件失敗しても UI は完了を返す。不完全なコーパスで
   「未結合 caption 642 → ?」を測ると 8d-2 の効果を誤判定する。
7. **再構築の 80 分間は配布版を起動しない**（debt-15 の窓が開くため）。

### 2.4 #8 の実行手順

restore 単位を GC / 再構築 / Vision の 3 つに割る。

```
(a) 手動バックアップ取得 → zip サイズと db.sqlite 圧縮後サイズを控える
(b) 配布版 LumenCite を終了（以降 (f) まで起動しない）
(c) p4 の GC を実行 → 検算: superseded 145→0 / document_nodes 2,663,234→約 450,149 /
    node_alt_texts が 888 のまま減っていないこと
(d) 手動バックアップ取得
(e) att40 でパイロット再構築 → carry 件数を確認（期待 73・debt-14 の痕跡に当たれば 72 以下）
(f) rebuild_outdated_lcir（GUI・80 分）→ failed == 0 になるまで再実行
(g) 計測 → 文書ごとの figure 件数の増分（中央値・最大）が異常でないか
(h) 手動バックアップ取得
(i) Vision バッチ（押す前に対象件数を「carry 失敗ぶん」と「新規検出ぶん」に分けて確認・
    モデルが claude-sonnet-5 であることも確認）
```

再構築は **GUI ボタン専用**（CLI にも MCP にも無い）。中断した場合に再実行で残りを拾えるかは
事前に確認しておくこと（`run_build_batch` は対象クエリを毎回引き直すので冪等のはず）。

### 2.5 却下した並べ方

- **図系 5 項目をマージしてから版 bump を 1 回だけ打つ** — 窓の中の実機 smoke が reuse で no-op になり、
  8d-2 の最重要回帰（既存ラスタ図の bbox が動いていないか）が必ず「異常なし」を返す。
  「`schema.rs:34` が 0.7.0 のまま = 再構築が走っていない証跡」も偽の証跡
  （窓の中の個別ビルドが 0.6.0 版を supersede し `prune_carried_alt_texts` が旧 alt text を消していても
  0.7.0 のまま）。証跡は `SELECT extractor_version, extraction_status, COUNT(*) FROM document_versions GROUP BY 1,2` に置く。
- **p4 の GC を再構築の後に置く** — 「回収量が最大」という利点は、述語が正しく働けば alt text を持つ版が
  残るので実現しない一方、述語のバグの代償だけが 2 桁上がる（§2.3）。
- **中間リリース v0.11.0 を正式タグで打つ** — 目的（p0 の実インストーラ検証）に対して代償が大きい。
  リリース作業一式（§9）が丸ごともう 1 回、しかも `lcir.enabled` は既定 OFF なので一般ユーザーに可視の変化がない。
  さらに p4 の非可逆 GC ボタンがゲート ②b を通らないまま出荷される。
  → p0 検証はタグを打たず、VM 上のビルド成果物と Linux 検証環境で行う。
- **p2 の自動経路に `attachments_with_outdated_lcir` を結線する** — 抽出器版は LCIR の PR ごとにほぼ毎回上がる
  （pdfium 0.1→0.7 が 7 PR）ので、「毎リリースで全ユーザーが無断で 80 分の全再構築を踏む」と等価。
  旧版更新は明示ボタン + 「旧版 N 件」の受動表示に据え置く。

### 2.6 着手前に決めること

順序・コストに効くものだけ。

1. **`p1` の OCR 保護に側表 `0021_fulltext_page_origin` を採るか。** 採るなら (a) p1 は p0 検証・配布版利用が
   終わってから、(b)「migration 0 件なので rollback は旧バイナリ差し戻しで成立」を撤回し
   「v1.0.0 からのダウングレードは非対応」をリリースノートに明記。採らないなら settings KV か
   content 側マーカーで代替。**p1 着手前に決め切る。**
2. **`8d-2` が既存ラスタ領域と union するか。** しない設計にできれば carry 破壊は debt-14 の 6 件のみ。
   せざるを得ないなら幾何フォールバック carry を同じ PR に入れる。この 1 点が Vision 再課金の見積り
   （$0.04 か $6 か）を決める。
3. **`8d-1` の消費者を名指しできるか**（§4.3）。名指しできなければ切る。ただし後から入れると
   80 分の再構築が 1 回増えることを承知の上で切る。
4. **top-level XObjectForm の `bounds()` がページ空間で返るか。** vendored source からは判定不能。
   8d-8 の (a) スライス（form 自身の bbox のみ）を実機 1 本（att134: Form 35 個 + 短辺 200px 以上の画像 7 枚で
   figure 0）で確認してから (b)（子へ降りて matrix 合成）に着手するかを決める。
   (a) だけで vector-only 21 版の caption 57 件相当に届く見込み。
5. **Linux での p0 実バンドル検証環境。** 探索先が配布形態で変わる（deb/rpm は `/usr/lib/<name>/`、
   AppImage は `$APPDIR/usr/lib/<name>/`）ので **.deb と .AppImage の両方**が要る。
   開発機は macOS、Windows は署名用 VM がある。Linux の手段が無いままだと「計画に入っている検証」が実行されない。
   **p3 が既定 ON にする前**に手段を確保する。
6. **`9b-4` を v1.0.0 に載せるか**（実測 M・650–870 行）。載せるなら仕様
   （`Some("pdf")` 固定 / 座標を変換しない / IRI に `document_nodes.id` を使わない / line 除外とその警告コード）を
   設計概観に先に書く。

---

## 3. Phase 7（数式意味表現）— post-1.0

**器は完成・中身はゼロ。** `math_expressions` の `ast_json` / `presentation_mathml` / `content_mathml` /
`openmath_json` は migration `0016` に列が存在し（`0016_math_expressions.sql:18-22`）、
INSERT も 12 列すべてを bind 済み（`db/math_expressions.rs:30-46`）。
しかし書込側は PDF 経路・TeX 経路の両方が `None` 固定（`ingestion/mod.rs:559`, `:927`）で、
**実 DB 32,508 行すべてが NULL**（2026-07-28 実測）。

**入力素材が薄い。** `latex` が入っているのは TeX 由来の **645 行のみ**（`source_provided`）。
残り 31,863 行は PDF 由来の `surface_only` で LaTeX を持たない。
つまり Phase 7 の実効価値は **TeX 取得率にそのまま比例する**。

**自前パーサが確定。** `Cargo.toml` に LaTeX/MathML 系クレートは無い
（latex2mathml / pulldown-latex は Presentation MathML のみで AST 不可、texmath は Haskell）。
規模の比較対象は `ingestion/tex/tabular.rs` の 1,019 行（本体 ~600 + テスト ~420）で、
それは 2 次元 grid の分割だけを扱う。

**設計上の決めごと（着手時に確定させる）**

- `math_expressions` に **UPDATE 経路が存在しない**（`insert_math` と `math_for_version` のみ）。
  AST を build 内で埋めるか、後追いバッチ + UPDATE 経路の新設かを先に決める。
- 併置 `AB` は中立ノード（積/適用を確定しない・§16）。parse 失敗は `ast_json` NULL（欠損許容）。
- 7a では migration を切らず、正準形は `ast_json` のエンベロープ内に入れる。本リポジトリは
  `json_extract`/`json_each` を一切使わず JSON は Rust 側で parse する規約なので、
  JSON 列を検索キーにするのは筋が悪い。横断索引は 7b の実表に切り出す。
- 7c のユーザー修正は **satellite 表**が要る。理由は `node_alt_texts`（8c）と完全に同型:
  ① `math_expressions` は build ごとに insert し直す ② 再構築で id が変わるので版跨ぎの持ち越しキーが要る
  ③ `math_expressions.confidence` は「表層検出の確からしさ」と `0016_math_expressions.sql:24` が明記しており、
  意味の provenance を混ぜられない。持ち越しキーは 7a の正準 AST ハッシュを使える。
- `MathSemanticStatus` の `NotAttempted` / `Inferred` / `Verified` と `Origin::MathRecognition` は
  enum に定義済みだが **どこからも構築されていない**。

ロードマップ §10 Phase 7 の完了条件 4 件は **0 達成**（API 面のみ部分達成）。

---

## 4. Phase 8d（図表の残り）

### 4.1 「figure 0 件 69 版」の内訳（2026-07-30 実測・初版から更新）

初版は「実測で効くのは 8d-2」と一括で扱っていたが、**内訳は 8d-2 一色ではない**。

- pdfium 生存版 138 件・うち figure ノード 0 件が **69 件（50.0%）** — 初版どおり再現。
- そのうち **caption を持つのは 42 版**（残り 27 版は caption も無い＝スキャン本・ロゴのみ等）。
- 42 版のさらに内訳: **21 版（caption 211 件）はラスタ画像を持つのに top-level 列挙に出ない = 8d-8 の担当**、
  **21 版（caption 94 件）が純ベクター = 8d-2 の担当**。
- 純ベクター 21 版はさらに (a) top-level path 型 12 版 / caption 37 件、
  (b) `\includegraphics{*.pdf}` 等で XObjectForm に包まれる型 9 版 / caption 57 件に割れる。
  **(b) は form 自身の `bounds()` を使えば path クラスタリング無しで取れる**（追加実装ほぼゼロ）。

未結合 caption の内訳も更新する。総数 642 のうち **10 件は Algorithm caption で設計上ペアリング対象外**
（`is_figure_caption_label` が Figure/Fig のみ許可）なので**真の回収母数は 632 件**。
さらに 556 件は「そのページに figure ノードが 1 個も無い」＝検出側の穴、
86 件は「figure はあるがペアリングが成立しなかった」＝ `pair_captions` 側の問題で 8d-2 では回収できない。

### 4.2 8d-2 の誤検出リスク（初版に無かった実測）

**普通の本文ページにも path オブジェクトは大量にある。** APS 系 PDF では 1 ページに
高さ 0.0pt の水平ヘアライン（分数線・段落罫・脚注罫）が 20〜45 本、y=69〜734 の全面に散らばる。
`MERGE_GAP_PT=12` のまま素で束ねると本文段全体が 1 個の「図」になる。

**誤検出の露出面は 6,321 ページ**（生存 pdfium 版の総ページ 7,345 − 現在 figure を持つ 1,024）。
うち「未結合の図 caption を持つ」＝回収対象ページは 477 のみで、無条件に回すと FP:TP が最悪 13:1。
**caption アンカー**（同一ページに未結合の図 caption があるページだけ探す）で露出面を 477 に落とせる。
これが debt-12 を 8d-2 より前に置く理由（§2.2 #1）。

そのほか実測で判明した設計上の要点:

- **`merge_image_regions` はそのままは流用できない。** 短辺 16pt / 面積 90% のフィルタが
  **マージ前の生矩形**に適用されるため（`figures.rs:33-81`）、ベクター図を構成する細い線分は
  1 本ずつ落ちて図が丸ごと消える。画像 bbox では「1 矩形 = 図の一部」だが path では「1 オブジェクト = 線 1 本」で前提が逆。
- **fixpoint マージは最悪 O(n^3)**（1 回マージするたびに二重ループを i=0 から再開）。
  上限 `MAX_RAW_RECTS_PER_PAGE=256` は画像用の値で、密なプロットの path は数千に達しうる。
  path 用の別上限（512 程度）を置いて超過ページを skip + warning にするのが安い。
- **pdfium の `bounds()` はクリップパスを考慮しない。** tikz/pgf は巨大なパスをクリップして小さく見せることが
  あるので、`get_clip_path()`（`object.rs:523`・0.8.37 に存在）で交差させる。
- **`segments()` の座標は未変換の生値**でオブジェクト行列が掛かっていない。使うなら `matrix()` を取って
  `transform()` を通す必要があり、bounds() だけを使う設計より確実にコストが高い。
- **リンク矩形は誤検出源にならない**（`page.objects()` はコンテンツストリームのみ・注釈は別コレクション）。
- confidence は既存の前例（`structure.rs:641` が「番号が取れたら 0.7・取れなければ 0.6」）に沿って、
  ベクター領域 0.4 / caption と相互最近でペアした領域 0.5 が妥当。
- **判定は `pdf/mod.rs` 内・レンダリング前で完結させる。** DB 層で領域を捨てると孤児 crop PNG が
  現 content_key ディレクトリに残り、`gc_stale_asset_dirs` は「他の content_key」しか回収しないので永久に残る。

### 4.3 8d-1 の受益者問題（初版に無い・スコープ判断に直結）

**role='original' を読むコードは現時点で 1 つも無い。** role で絞る唯一の読み手は 8c の alt text バッチで、
そこは `na.role = 'page_crop'` 固定（`db/node_alt_texts.rs:185,204`）。
それ以外（`assets_for_version` / `LcirNode.assets` / MCP `get_figures` / 10a `get_node_context` /
export warning の件数集計）は **role で絞らず全件返す**。フロントは図の画像を一切描画していない
（`src/` に `relative_path` の参照が 0 件）。

したがって 8d-1 を入れると、**role を見ない読み手が黙って倍の件数を返す**副作用だけが先に出る。
実装するなら読み手を全部洗う必要がある（`get_figures` は role でグルーピング、`get_node_context` は
`page_crop` に絞る、export warning は role 別に数える）。

**生ストリーム保存は 92% のケースで「開けないバイト列」になる。** 実ライブラリの埋込画像 19,008 個の内訳は
CCITTFaxDecode 91.9% / FlateDecode 3.0% / DCTDecode 2.3% / JBIG2Decode 2.8% / LZW 0.1% で、
単体ファイルとして成立するのは DCTDecode（JPEG）と JPXDecode だけ。設計は二択:

- **(A) DCT/JPX のときだけ raw stream をそのまま保存**（mime を `filters()` から決める）、それ以外は作らない。
  増分は最大 +15MB 程度。§16「誤検出より欠損」に沿う。
- **(B) 全部 `get_raw_image()` → PNG 再エンコード。** デコード後の総画素が少なくとも 1,344 Mpx なので
  100MB〜1GB のオーダーになりうる（上限が読めない）。

埋込画像ストリームの合計は figure アセットを持つ 69 添付で **77.17MB**（crop PNG 555MB の 1/7）。
`get_raw_image_data()` は pdfium-render 0.8.37 に実在し（`image.rs:609`）、
同梱 `libpdfium.dylib` も `_FPDFImageObj_GetImageDataRaw` を export している。

backup / restore / purge / `delete_attachment` には**無改修で乗る**（8a と同じ経路）。
ただし `merge_image_regions` が「どの生矩形がどの領域に畳まれたか」を捨てているので、
`assign_rects_to_regions(rects, regions) -> Vec<Vec<usize>>` の純関数を新設する必要がある。

### 4.4 回転ページ（8d-8 の後半・debt-9）

**設計概観 §6 の「要検証」は実測で決着した。** pdfium の `page.width()/height()` は**回転適用後**の寸法だが、
text/object の `bounds()` は**回転前の user space** のまま返る。
根拠は実 DB の vid 267（rot 0 のページと rot 90 のページで block の x/y 範囲が一致）と
`bindings.rs:1198` の記述。フロント（pdf.js の `PageViewport`）は viewBox 原点と rotation の両方を吸収するので、
**保存すべき契約は「絶対 user space の bbox」で正しい**。

ただし**回転側の実害はゼロ**。実ライブラリの回転ページは **1 添付・5 ページのみ**（att127・すべて `/Rotate 90`）で、
その 5 ページの figure は 0 件、回転 skip の warning も 0 件。**8d-8 の回転半分は切る候補**。

### 4.5 8d-7 の実測値の訂正

初版の「解決率の実測上限は約 93%」は再現しない。`graph.rs` の解決規則を実 DB の全ブロックノードに適用して
再計算すると **91.18%（1302/1428）**。内訳は via_caption 877 / via_node 425、参照先は figure 1374 / table 54。

未解決 126 件の内訳も初版の説明と違う:

| 分類 | 件数 |
|------|------|
| 同一版に該当番号の caption 文字列が存在しない（**figure 0 件の文書に集中 = 8d-2 領域**） | 73 |
| 番号衝突で墓標（曖昧なので意図的に落とした） | 22 |
| figure_caption ノードは在るが索引の番号と一致しない | 13 |
| bibliography_entry 内の一致（参考文献中の Table/Figure＝真の非対象） | 12 |
| table_caption 別種別 | 6 |
| **数字番号の caption が paragraph に落ちている** | **0** |

つまり初版が書いた「残り 7% は caption が paragraph 落ち（= debt-12）」は誤り。
**8d-7 の残りを取りに行く投資先は 8d-2。**

---

## 5. Phase 9b（標準形式エクスポート）

**拡張点は初版が言うほど安くない。** `export/mod.rs` は初版の「73 行」から **118 行**に増えている
（debt-8 が `export/warning.rs` 556 行を追加した）。新形式 1 つを足すのに実際に触るのは
`export/mod.rs` 2 行 + `warning.rs` の `FormatCapabilities` const 約 10 行 + `lib.rs` のコマンド 25 行 +
`cli/mod.rs` 10 行 + `DetailPanel.tsx` 20–30 行 + i18n 8–12 行 + docs で、**グルーだけで Rust ~70 / TS ~30 行**。

**本丸は木の再構成。** PDF 側は `section` が `paragraph` を含まない **平坦木**
（block が page 直下・`ingestion/mod.rs:498`）。XML は包含構造を要求するので、
`payload_json` の heading_level から節木を組み直す再構成器が 9b-0 として全形式の共通前提になる。

**XML writer 未導入。** `Cargo.toml` に quick-xml / xml-rs は無い（9b-4 は JSON なので依存追加不要）。

**9b-M が分岐点。** `presentation_mathml` は全書込経路で NULL 固定なので、9b が数式をどう出すかは
(a) Phase 7 を待つ / (b) 変換クレートを導入 / (c) 生 LaTeX のまま出す の 3 択。
**(c) を選べば 9b は Phase 7 に依存しない。**

JATS は「LCIR 固有の信頼度・PDF 座標・抽出履歴は JATS 外に保持してよい」とロードマップ §9.1 が明記しており、
完全対応は不要。TEI は §0 の理由 2 により post-1.0。

### 5.1 9b-4 の再評価（初版から難易度 S → M）

**元ロードマップに Web Annotation の仕様は書かれていない。** §9「外部標準との関係」は JATS / TEI / MathML /
OpenMath / Markdown の 5 節のみで、言及は Phase 9 実装項目の 2 行だけ。§13.4 の round-trip 対象にも入っていない。
**仕様はこちらで確定させる必要がある**（着手前に設計概観へ §9b-4 節を新設する）。

確定させるべき設計判断:

- **座標を変換しない。** RFC 8118 の `viewrect`（左上原点）や `#xywh=` を出すには y 反転が必要だが、
  ①回転ページ（2,912 fragment）では page 寸法が回転後・bounds が回転前で `page_height - (y+h)` が負になる
  ②非回転ページでも **11,963 fragment（9 版）がページ矩形からはみ出す**（CropBox 原点・debt-19）。
  **これは debt-14 では直らない**（debt-14 は図 crop のクランプのみ）。
  したがって標準 `FragmentSelector` は `page=N` だけにし、生 bbox は名前空間つき独自 selector に載せる。
- **版を `Some("pdf")` に固定する。** TeX 版の `source_fragments` は実 DB でも 0 件、かつ
  `load_entry_lcir` は tex 優先なので、arXiv 論文で `--source` 無しに実行すると**成功扱いで空のファイルが出る**。
  `get_figures`（`document.rs:1137`）の先例に倣う。
- **`line` を除外する。** fragment は line 309,992 / block 130,904 / page 7,345 で line が 67%、
  しかも plain_text は親ブロックと重複（Markdown レンダラも捨てている）。除外後は版あたり
  min 10 / max 10,218 / 平均 970。**除外は打ち切りではなく意味的フィルタにする**（export は完全性が要件）。
  ただし 310k 件の座標を捨てるので `ExportWarningCode::LineFragmentsDropped`（severity=info）を足す。
- **IRI に `document_nodes.id` を使わない。** rowid なので再構築すると別の値になる。
  `(parent_id, ordinal)` は版内で一意（実 DB で重複 0 件）なので、根から辿った ordinal 経路が
  決定的な識別子になる。生の node_id は `lc:nodeId` として別に載せる。
- alt text を body に載せるなら `purpose` と `lc:origin` / `lc:model` を必ず付ける（§16・`confidence` は出さない）。

**9a のテストは golden fixture ではない**（`export/markdown.rs` の 27 テストはすべて手組み `LcirDocument` +
出力文字列へのインライン assert）。9b-4 も同じ方式で書く。

---

## 6. Phase 10（LLM・エージェント向け）— 10a/10b は実装済

**エージェント表面**: MCP サーバー 17 ツール / アプリ内チャット最大 20 ツール / CLI 14 サブコマンド。
文献本文の read ツールは `llm::tools::document` が正本で、MCP とチャットが同じ定義・同じ実行を共有する。

- **10a**（文脈バンドル `get_node_context`）= PR #68。詳細は設計概観「Phase 10a 実装状況」。
- **10b**（チャットへの LCIR 露出 + provenance + 根拠ジャンプ）= PR #69。同「Phase 10b 実装状況」。

10b の既知の限界（v1.0.0 でも残る）:

- **根拠チップは PDF 由来 LCIR 限定**。TeX 版に `source_fragments` は無く、`load_entry_lcir` は tex 優先。
  ロードマップ完了条件 (1) は **PDF 由来 LCIR について**満たす。tex → pdf の位置解決は post-1.0。
- **完了条件 (3)「数式を構造化表現として渡せる」は TeX 由来の `latex`（原文文字列）をもって満たしたと読む。**
  MathML / Content MathML は Phase 7（post-1.0）。v1.0.0 の「Phase 10 到達」判定はこの読みを前提にする。
- **一覧のゲートは spec 面のみ**（`lcir_readable` が false でも `execute_tool` は名前で実行できる）。
  読み取り専用なので意図的に許容する。なお **MCP サーバー側は `lcir_readable` でゲートしていない**
  （`mcp_server/mod.rs:334` が `DOCUMENT_TOOLS` を無条件ディスパッチ）。この非対称は doc に 1 行書く。
- **プロンプトキャッシュが無い。** ツール定義は 20 ツールで **26,959 文字 ≒ 6.7k〜7.5k トークン**（実測）が
  毎ターン再送される。LCIR 8 ツールを隠すと 9,283 文字まで落ちる。
  ただし **cache_control はモデル依存で無言に効かない**: 実 DB のチャットモデル `claude-haiku-4-5` の
  最小キャッシュ prefix は 4096 トークンで、LCIR 非表示時（tools+system ≒2.5k）は閾値未満になる。
  導入するなら会話履歴の末尾にも breakpoint を置き、`usage.cache_read_input_tokens` を 1 回実測してから
  「効いた」と言うこと。breakpoint 上限は 1 リクエスト 4 個。
- **`DEFAULT_MAX_TURNS = 12`**（`llm/chat.rs:19`）のまま。上限到達は `Ok(())` → `ChatStreamEvent::Done`
  （unit variant・理由を持たない）で正常終了と区別できない。**上げるより先に通知を入れる**こと。

**10c は post-1.0**（§0 の理由 3）。

---

## 7. v1.0.0 の前提（p0〜p4）

### v1.0.0-p0 — pdfium の Windows / Linux 同梱（実装済・実バンドル検証は未了）

`tauri.release-linux.conf.json` を新設し `bundle.resources` で `libpdfium.so` を同梱。
`release.yml` の Linux ジョブに取得 + SHA-256 検証を追加。Windows は CI 非対象なので VM 上の手動配置手順を
`RELEASE.md` §4 に記載。`bind_pdfium()` の探索候補を純関数 `library_search_dirs()` に切り出し、
Linux のリソースディレクトリ 3 通りを探索（単体テスト 6 本）。

**残る検証**: 実 `.deb` / `.AppImage` / `.msi` に入ることの確認。§2.6-5 と §9 を参照。

### v1.0.0-p1 — FTS 派生化

初版の記述は行番号も規模も実コードとずれていた（§10）。実態は次のとおり。

- seam `regenerate_page_fts_from_lcir` は **`ingestion/mod.rs:1317-1351`**、シグネチャは **`(pool, attachment_id)`**、
  **本番呼び出し元は 0 件**（テスト 2 箇所のみ）。設計概観 §8 の「(B) 化は差し替える 1 行」は撤回が要る。
- `fulltext` に**内容を書く**本番経路は **6 call site / 4 コードパス**:
  `extract_and_index` ×3（`lib.rs:736` add_attachment / `lib.rs:788` download_arxiv_pdf /
  `mcp_server/mod.rs:1006` clipper）/ `index_attachment` コマンド / `index_missing_attachments` /
  `run_ocr` の 2 箇所。
- FTS5 仮想表に列は足せない（実測で `virtual tables may not be altered` を確認）ので、
  初版の「破壊的 migration になるので採らない」は正しい。ただし**普通の側表なら非破壊**という
  第 3 の選択肢を初版は検討していない（§2.6-1）。

**初版に無い最重要の落とし穴**: pdfium の page 全文には Unicode マップの無いグリフ（リガチャ・数式）が
C0 制御文字として残る。`structure.rs:786 normalize_ws` が block/line からは落としているが、
**page ノードはこの正規化を通らない**。実測で非空 LCIR page 5,803 のうち **4,367 ページ（75.3%）**が
C0 制御文字を含む（`fulltext` 側は 11.6%）。trigram 索引なので `con\x02dition` は "condition" にヒットせず、
**素朴に派生化すると現状より検索が悪くなる**（debt-22）。
対策は「page の子 block を ordinal 順に連結する」（block は正規化済み・page 文字数の 98.4% をカバー）。

**派生化の実利は実測できる**（読み取り専用 SQL のみ・DB コピー不要）: ページ単位で 112 ページが新規索引、
2 ページが「LCIR 空だから既存行を残す」規則に依存、5,691 ページが重複。添付単位では
**att93（15p/23.8k字）と att94（42p/55.2k字）は pdf_extract が全滅・pdfium が成功**という実利がある。

**OCR 由来行の保護**は解けていない。`run_ocr` は全ページ OCR なら添付の索引を丸ごと置換するが、
テキスト層が壊れた PDF では pdfium も「壊れたテキスト」を非空で返すので「空ページは残す」規則では守れない。
`fulltext` には provenance を記録する場所が無く、**既存行が OCR 由来かは実 DB からも判定できない**。

**既存 138 添付に派生 FTS を行き渡らせる経路が無い**（`attachments_without_completed_lcir` は完了版のある
添付を除外し、`attachments_with_outdated_lcir` は版 bump 無しでは 0 件）。p1 に安価な再導出パス
（`rebuild_fulltext_fts_once` と同型の一度きり起動時パス + 手動ボタン）を持たせる。

### v1.0.0-p2 — LCIR の自動 build

初版の「`post_attach` への組み込み」は**存在しない関数**を指していた。事実上の共有 post-attach フックは
`db::fulltext::extract_and_index`（`db/fulltext.rs:67`）で、その doc コメント自身が
「全経路が同じ post-attach 索引を通るよう共有する」と書いている。刺す場所は既存の全文索引フックと同じ 3 箇所。

- PDF 添付が増える本番経路は **3 つ**（`add_attachment` / `download_arxiv_pdf` / クリッパー `spawn_pdf_job`）。
  CLI と MCP には添付を作る経路が無い。
- PDF の LCIR build の**手動経路は 2 つ**（設定→データの 2 ボタン + 詳細パネルの添付行ボタン・PR #61 で追加）。
- TeX の自動 build は **4 経路**（自動 3 + 手動 1）。`API_SPEC.md:442` の「3 つ」は
  `fetch_missing_arxiv_sources` が抜けている。

**排他が 3 フラグに分裂している。** `LCIR_BATCH_RUNNING` / `TEX_FETCH_RUNNING` / `VISION_ALT_TEXT_RUNNING` は
互いを見ず、相互排他は `SettingsModal.tsx:1077` の `disabled` 属性 1 行だけで担保されている（debt-24）。
自動 build は UI を通らないのでこの防壁を無効化する。
対策は `build_lcir_for_attachment` の内側に単一 `tokio::sync::Mutex` を置くこと
（`BACKUP_LOCK` と同じ `const_new` パターン。8 つの呼び出し口が自動的に 1 本に絞られる）。

**プロセス横断の排他が無い。** `acquire_gui_lock`（`lib.rs:50-64`）は `try_lock_exclusive` の成否を捨てている。
自動化するとこれが既定挙動になるので、成否を返して**ロックを取れたインスタンスだけがバックフィルを走らせる**
（10–15 行）。あわせて `gc_stale_asset_dirs` に mtime 猶予を入れる（debt-15）。

**起動時バックフィルの間引きはバックアップの前例が丸ごと流用できる**
（`run_backup_if_due` / settings KV / `static BACKUP_LOCK` / dev ビルド既定オフ / 起動時 spawn + interval）。
mtime 猶予のパターンは `sweep_backup_workdir` の `WORK_FILE_STALE_SECS = 60*60`。

**1 ラン上限は件数では最悪ケースを制御できない**（実測の分布が 1 添付 4,514 秒 vs 平均 2.1 秒と極端に偏る）。
経過時間で切るには添付の途中で止める必要があるが、`spawn_blocking` 内の pdfium 抽出はキャンセルできない
＝添付境界でしか止められない。

### v1.0.0-p3 — 既定 ON + 同意分離

- 判定式は `ingestion/mod.rs:29-36`。反転の本体は `== Some("1")`（:35）1 行だが、直上の doc コメントと
  `db/settings.rs:88-90` のキー説明も嘘になる。
- 既定 OFF 前提のテストは **8 本**（初版の「10 箇所程度」より少ない）。うち clipper.rs の 3 本は
  **tex_autofetch 分離を先にやれば無改造で通る**ので、要修正は 5 本に減る。
- **外部通信は 11 系統**あり、`lcir.enabled` に従うのは TeX 自動取得の 4 経路のみ
  （`clipper.rs:344` / `clipper.rs:423` / `lib.rs:1773` / `AddSheet.tsx:127`）。
  GitHub リリース確認は自動起動しない（設定→更新のボタンのみ）。
- **「明示 OFF」と「未設定」は区別できる。** `set_lcir_enabled` は `"0"`/`"1"` を必ず書き、
  他に書く本番経路は無く、migration に settings の seed も無い。
  `== Some("1")` を「値が `"0"` でなければ ON」に変えれば明示 OFF ユーザーは ON に化けない。
  ただし想定外値（`""` / `"false"`）を ON と誤読する暗黙の不変条件になるので、
  `Some("0")=>false / None=>true / Some(other)=>true` を網羅するテストと doc コメントで固定する。
- **`lcir.tex_autofetch.enabled` を既定 OFF にすると既存ユーザーの挙動が無言で退行する**
  （実 DB のユーザー本人が `lcir.enabled="1"` に該当）。起動時 1 回だけの冪等 backfill
  （`lcir.enabled == "1"` かつ tex_autofetch 未設定なら `"1"` を書く・`FTS_AUTHORS_V030_REBUILT_KEY` と同型）で救う。
- **p3 単独ではユーザーに何も変わらない**（p2 が無いと `lcir_readable` は false のまま）。p2 → p3 の順は必須。

### v1.0.0-p4 — superseded 版の GC + 容量可視化

**初版の「`DELETE FROM document_versions` だけで木ごと消える」は誤り。**
`document_versions.parent_version_id` は ON DELETE 指定が無く（NO ACTION）、
実 DB の superseded **145/145 が新版から参照されている**ので素朴な DELETE は FK エラーで 1 件も消えない
（scratchpad に実 migration を適用した DB で再現確認済み）。
同一 tx の pre-step `UPDATE document_versions SET parent_version_id = NULL WHERE parent_version_id IN (<削除対象>)`
が必須。**CASCADE 表は 4 表ではなく 9 表**（`document_nodes` / `source_fragments` / `math_expressions` /
`node_relations` / `symbols` / `symbol_occurrences` / `assets` / `node_assets` / `node_alt_texts`）。

CASCADE でない FK は 3 本: `symbols.scope_node_id` = SET NULL（同版内なので実害なし）、
`node_alt_texts.carried_from_version_id` = SET NULL（**provenance が壊れる** — schema コメントが
「NULL = この版で生成」と定義しているため carry 行が生成版を偽る）、`parent_version_id` = NO ACTION。

**GC が carry を壊さない条件（定式化）**: 版 v を削除して安全なのは
「v に紐づく `node_alt_texts` 行が 0 件」かつ「v が他の行の `carried_from_version_id` から参照されていない」とき。
前者が破れると (i) 課金済みで復旧不能な `llm_inference`（crop PNG は trash 済み・新版に page_crop が無いので
`figures_missing_alt_text` の対象にも戻らない）と (ii) 人間が書いた `user_edited` が無音で消える。

**タイミングが本質**（§2.3）。実 DB の削除対象 145 版は現時点で `node_alt_texts` 0 行 / `assets` 0 行 /
superseded を指す FTS 行 0 件 / node_id を含む `chat_messages` 0 件＝**述語がバグっても失うものが実測ゼロ**。
再構築後は carry に失敗した課金済み alt text を抱えた版が対象になる。

そのほかの実測:

- 回収見込みは約 **526MB**（`document_nodes` 262.4MB + `source_fragments` 173.1MB + 索引 5 本で DB の 83.2% を占め、
  そのうち 83% が superseded）。
- **GC しても DB ファイルは縮まない**（free page になるだけ）。UI は「使用中 / 再利用可」を分けて出し、
  「次のバックアップと次の再構築で回収される」と明記する。live DB への VACUUM は p4 に入れない。
- **バックアップは `VACUUM INTO` なので free page を運ばない** ＝ GC 後は live DB を VACUUM しなくても
  バックアップだけ即座に縮む。実測: 直近 zip の db.sqlite は raw 737MB → 圧縮 220.6MB。
  GC 後は圧縮 63〜70MB と見込め、1 世代あたり約 150MB・keep=14 で約 2.1GB の削減。
- **cascade delete は 2.21M+2.21M 行で約 27 秒**（実測外挿）。main pool は `busy_timeout` を明示しておらず
  sqlx 既定の 5 秒なので、単一 tx で保持すると別プロセスの書込が `SQLITE_BUSY` で落ちる。
  **版単位の tx に割る**（145 回・1 回約 0.18 秒）。
- 容量可視化の部品は既にある: `list_backups`（`BackupInfo{size_bytes}`）は Tauri に登録済みだが
  **フロントに呼び手が 1 つも無い**ので流用できる。`dbstat` は本番 DB で使える（726MB で数十秒）。
- GC と stats は **`lcir.enabled` で gate しない**（切った人ほど消したい）。

---

## 8. 積み残し債務（完了扱い Phase の中の未実装）

| id | 内容 | 難易度 | 後続の前提か |
|----|------|--------|--------------|
| debt-1 | `InlineMath` / `EquationGroup` を全経路で生成しない | M | Phase 7 の入力範囲を決める |
| debt-2 | `ListItem` / `Footnote` / `Citation` を生成しない（`List` は TeX のみ） | S〜M | 9b の構造品質 |
| debt-3 | `TextBlock` ノード型が未使用（実際の木は `document > page > block > line`） | S | — |
| debt-4 | JATS / HTML / LaTeXML 取込なし・手動 `.tex` 添付もスコープ外 | L | 9b と対称 |
| debt-5 | PDF 側の記号抽出なし（6b は TeX のみ）・記号スコープの厳密化 | L / M | 7c の曖昧性解消 |
| ~~debt-6~~ | ~~9a Markdown で figure が完全に落ちている~~ **解消**（2026-07-28） | S | 9b-1 |
| debt-7 | 8c の alt text の**手編集 UI なし**（`origin` 列だけ用意済み。実 DB の 888 件は全て `llm_inference`） | S | — |
| ~~debt-8~~ | ~~9a の欠落警告未実装~~ **解消**（2026-07-28・`export/warning.rs`） | S | 9b-0 |
| debt-9 | 回転ページは図領域を skip → **§4.4 で座標系は決着**。実害は 1 添付 5 ページ・figure 0 件 | M | 8d-8（切る候補） |
| debt-10 | 8b の longtable / tabu / siunitx S 列 / 表脚注 | M | — |
| debt-11 | 数式検索は trigram 部分一致のみ。TeX 版は node-FTS 対象外 | — | 7b の動機 |
| debt-12 | ローマ数字・全大文字の caption 取りこぼし → **v1.0.0 スコープ**（正当化は §8.1） | S | — |
| debt-13 | スキャン本 1 冊（att37）が figure の 43.7%・alt text の 58.9%・**crop 容量の 80%（444.5MB）**・**再構築時間の 94%（4,514 秒）**を占める | M | 8c/8a の費用対効果・p2 の最悪ケース |
| debt-14 | 図領域のクランプが CropBox 原点を無視 → **v1.0.0 スコープ**。実害は非ゼロ原点 12 版・クランプ痕跡 10 図・alt text 保持 6 件 | S | 8d-2 の前提 |
| **debt-15** | `gc_stale_asset_dirs` に mtime 猶予が無い。dev と配布版が同一 app data dir を共有しているので、両方が build すると互いの content_key ディレクトリを trash に送り合う | S | **#0 で解消する** |
| **debt-16** | `heal_missing_assets` が `assets.sha256` を UPDATE しても `node_alt_texts.source_asset_sha256` が追随しない。crop が 1 枚欠けた添付を build した時点で無言の carry 破壊が仕込まれ、数週間後の再構築で「8d-2 のバグ」と誤診される | S（~40 行） | 再構築 1 回の前提 |
| **debt-17** | 新規添付で `extract_and_index`（pdf_extract）の spawn と LCIR build が last-writer-wins レースになる。`index_attachment` は先頭で `DELETE FROM fulltext WHERE attachment_id=?` を無条件に打つので、pdf_extract が 0 字を返す個体（att93/att94）は **LCIR が正常でも検索から消える**。新規添付でしか起きないので既存 138 件の前後比較には現れない | M | **p1 の完了条件**（3 つの spawn を「足す」でなく「外す」） |
| **debt-18** | 同じ座標系の取り違えが `structure.rs:365-368` の `in_margin` 判定にもある。非ゼロ原点の PDF で判定帯がずれ、短い散文行が paragraph → unknown_block に降格する（vid 183 で実測 110 件 vs 正しい帯 51 件） | S | debt-14 と同根・別 PR |
| **debt-19** | **テキスト fragment 11,963 件（9 版）がページ矩形をはみ出す**（CropBox 原点・y 方向最大 +116pt / x 方向最大 +464pt）。**debt-14 では直らない** | M | 9b-4 の座標変換 |
| **debt-20** | `content_key` の `config_hash` が全経路 `""` 固定で、`RENDER_TARGET_WIDTH` 等を変えても content_key が動かない。pdfium バイナリの tag（chromium/7934）も content_key にも `metadata_json` にも入っていない ＝ **pdfium を上げると全 crop の sha256 が変わり alt text 888 件が全滅しうるのに、それが版として表現されない** | S（metadata に tag を足すのは 1 行） | 再構築の再現性 |
| **debt-21** | superseded 行が残る間、同一 content_key の再 build は UNIQUE 違反で必ず失敗する（`find_completed` が status で絞るので reuse に乗らない）。**古いバイナリで「旧版を再構築」を押すと全件失敗する**経路が実在する | S | p4 の GC が副作用で解消する |
| **debt-22** | page ノードの `plain_text` が `normalize_ws` を通らず C0 制御文字を含む（**非空 5,803 ページの 75.3%**。`fulltext` 側は 11.6%）。trigram 索引で語が割れる | S | **p1 の必須前提** |
| **debt-23** | 走り柱が caption と同一ブロックに融合する（「166 8 Staggered Model Fig. 8.3 …」）。`detect_caption` は先頭行しか見ないので caption にならない。素朴な緩和（先頭行以外にも当てる）は本文中の "Fig. 3" を誤認するので危険 | M | debt-12 とは別物 |
| **debt-24** | 長時間 LCIR ジョブの排他が 3 つの独立フラグに分裂し、相互排他は `SettingsModal.tsx:1077` の `disabled` 属性 1 行だけ。alt text ボタンは `lcirBatch` を見ていない | S〜M | **p2 の完了条件** |

`NodeKind` は 29 種定義されているが、生成経路を持つのは PDF 18 種 / TeX 22 種。
`ListItem` / `Footnote` / `Citation` / `InlineMath` / `EquationGroup` / `TextBlock` はどちらも生成しない。

### 8.1 debt-12 の正当化を差し替える

初版は debt-12 を「8d-7 の解決率上限 93% の実体」として位置づけていたが、**3 点とも実測と食い違う**。

- 「`Fig. 8.3` 形も落ちている」→ **再現しない。** `Fig. 8.3` は現行コードで通る（label_len=4 → 直後 6 文字に数字あり）。
  実 DB でも多段番号の figure_caption が 904 件中 400 件ある。落ちる実例は debt-23 の別問題。
- 「実測 60 件」→ **48 ブロック / 13 文書**（`TABLE <ローマ>` + 終端記号 47 + `Fig. <ローマ>` 1）。
- 「8d-7 の解決率上限の実体」→ **debt-12 単独では 8d-7 の解決辺は +0**。参照側の `take_ref_number`
  （`graph.rs:823-850`）が ASCII 数字（と付録形 "A.1"）しか読まないので、本文の "Table I" はそもそも
  参照として走査されない。参照側もローマ対応させて初めて +81 辺（1302→1383）だが、
  参照母数が 1428→1529 に増えるので**率は 91.18% → 90.45% に下がる**。

**新しい正当化**: `node_kind` の正しさ。`table_caption` が 68 → 約 115 件（+69%）になる。
図側にはほとんど影響しない（`FIG.`/`FIGURE`/`Fig.` で始まる block 級ノード 906 件のうち 892 件＝98.6% は
既に figure_caption に分類済み）。

**誤検出ガードは 1 個で足りる**: ローマ数字の直後に終端記号（`.` / `:`）を要求する。
実測でこれがローマ+終端記号 47 件（すべて caption）と「Table XIV shows the equivalence between…」1 件（本文）を
完全に分離する。「Table I」と「Table 1」の同一視は**不要**（ローマ数字 caption を持つ 12 版すべてで、
同じ版の `table_caption` ノード数 = 0 かつ本文の `Table <数字>` 参照 = 0）。
ローマ数字パーサは caption 専用の独立関数として書き、`parse_theorem_number` には触らない
（定理番号 "Theorem V.?" に波及して Phase 5 の分類を変えるため）。

---

## 9. リリース作業（v1.0.0 固有）

3 つの順序案がいずれも計画に入れていなかった項目。実装の外側に集中している。

- **`CHANGELOG.md`** — Keep a Changelog / SemVer 準拠で実際に維持されており、`## [Unreleased]` に
  Phase 10a/10b が既に積まれている。中間スコープ 10 項目ぶんの整理が要る
  （`docs/RELEASE.md` §7 のタグ前チェックリストにも項目がある）。
- **`docs/RELEASE.md` に v1.0.0 固有事項の節を追加** — §8/§9/§10 と同じ形式で。
  (a) `lcir.enabled` 既定 ON (b) 起動時バックフィル (c) migration 0021（採る場合）
  (d) pdfium が Windows / Linux で初同梱 (e)「旧版 N 件」を押すまで新しい図が出ないこと。
- **v0.10.0 → v1.0.0 の実アップグレード検証**（RELEASE.md §8-2 の前例）。ただし
  **開発機では既定 ON の経路を再現できない** — 実 DB の settings には `lcir.enabled="1"` が明示的に
  書かれているため、p3 が変える判定（未設定 → ON）を通らない。クリーンな app data dir での初回起動が要る。
- **p0 の実バンドル検証の実体** — Windows の pdfium は CI が同梱せず、VM 上で `curl` →
  `certutil -hashfile` → `src-tauri\pdfium\pdfium.dll` へ手動配置し `--config src-tauri/tauri.release-windows.conf.json`
  付きでビルドする。**`--config` を落とすと署名設定と pdfium 同梱が同時に消える**
  （過去に「signtool 更新」と誤診した前例）。インストーラ展開後に `pdfium.dll` の存在を確認する手順まで含める。
  Linux は §2.6-5。
- **Windows VM のリードタイム** — Azure VM は課金停止済みなので、起動 → SimplySign の対話ログイン →
  pdfium 配置 → ビルド → 署名 → 手動アップロードで半日規模。**リリース日程のクリティカルパス**。
- **`latest.json` は darwin エントリのみ** — Windows / Linux には v1.0.0 が auto-update で届かない。
  しかも両 OS にとって v1.0.0 は pdfium 初同梱版で、手動 DL しない限り LCIR も OCR も動かない
  （`bind_pdfium()` が両者の単一入口）。看板の到達率が OS で構造的に違うことと、
  更新通知（`check_latest_github_release` は全 OS で出る）での周知をリリースノートに書く。
- **Homebrew tap は publish 時の自動ワークフロー**（`release: published` で発火・プレリリースでは動かない）。
  必要なのは書き換えではなく、draft → publish の順序、`HOMEBREW_TAP_TOKEN` が生きていること、
  publish 後に tap のコミットと `brew info --cask lumencite` の確認。
  **トークン失効時は 403 で無言失敗**し、cask は `auto_updates true` なので brew ユーザーには
  「更新が来ない」ことすら見えない。
- **配布後検証**（RELEASE.md §6） — 別マシン / クリーンインストールでの Gatekeeper・SmartScreen・
  AppImage 起動・`dpkg -i`、そして **updater 経路**（旧版を入れて起動 → 通知 → 適用 → 再起動）。
  v1.0.0 は「updater で上がった直後の初回起動」でバックフィルが走る設計なので、この経路が最重要。
- **Chrome 拡張 0.2.0 を据え置くか** — クリッパー経路に触れるのは p2（`spawn_pdf_job` への build フック）と
  p3（tex_autofetch 分離で確認ポップアップの「tex」ラベルが出なくなる）。拡張側のコードを変えるかで
  前回 zip を流用してよいかが決まる。zip は CI が作らないので手動添付。
- **ゲート ②b のスコープ・トリアージ基準・修正時間のバッファ** — 対象がリポジトリ全体か v1.0.0 差分か
  （全体なら lib.rs 分割・N+1 など v1.0.0 と無関係の既存債務が大量に出る）、v1.0.0 ブロッカーと post-1.0 の
  切り分け基準、所要時間を先に決める。**前例では全体レビュー 1 回が本体 PR + followup 4 本を生んでいる。**
- **docs の一括点検** — v1.0.0 で `API_SPEC.md`（p1 の再導出コマンド / p3 の tex_autofetch /
  p4 の stats・GC / 9b-4 の新形式 / CLI の `--format` / `index_attachment` の意味変更）、
  `DATA_MODEL.md`（`fulltext` が LCIR 由来の派生索引になること / 0021 / GC 後の assets の provenance）、
  `SPEC.md`（自動 build の境界と上限 / 既定 ON / 容量表示と GC ボタン）が同時に変わる。
  ゲート ②b の入口条件に「3 本が実装と一致していること」を置く。
- **post-1.0 の非目標を明文化**（§0 末尾）。

### 実機 smoke の作法

```
cwd = src-tauri で cargo test --lib <name> -- --ignored --nocapture
LCIR_SMOKE_DB     : 実 DB（761MB）の**コピー**。$TMPDIR に置く
                    （Dropbox 配下に置くと 761MB が同期対象になり disk 91% 使用を悪化させる）
LCIR_SMOKE_APPDIR : 実 appdir を**読むだけ**。テストは一時ディレクトリに書く
LCIR_SMOKE_ATT    : 対象添付 id
LCIR_SMOKE_KEEP=1 : crop PNG を残して目視
```

`mode=ro` は WAL DB で `-shm` を作れず `SQLITE_CANTOPEN`(14) になることがある。読み取り専用の集計には
`immutable=1` を使う。p3 / 9b-4 の読み出し面には `mcp_lcir_tools_e2e` と `tex_extract_real_source` も
1 回ずつ回す（3 案とも `lcir_build_real_pdf` しか挙げていなかった）。

---

## 10. 初版（2026-07-28）から訂正した記述

| 初版の記述 | 実際（2026-07-30 実測） |
|---|---|
| p1 seam は `ingestion/mod.rs:1277-1306` | **1317-1351**（Phase 10a/10b で約 40 行ずれた） |
| 設計概観 §8「`regenerate_page_fts_from_lcir(pool, version_id)`」「(B) 化は差し替える 1 行」 | シグネチャは **`(pool, attachment_id)`**、本番呼び出し元は **0 件**。「1 行」は撤回が要る |
| p1「呼び出し元 5 箇所の書き換え」 | `fulltext` に内容を書く本番経路は **6 call site / 4 コードパス** |
| p2「PDF は完全に手動（設定→データのボタン）」 | 手動 **2 経路**（+ 詳細パネルの添付行ボタン・PR #61） |
| p2「TeX ソースだけ 3 経路」／`API_SPEC.md:442` | **4 経路**（`fetch_missing_arxiv_sources` が抜けている） |
| p2「`post_attach` への組み込み」 | **`post_attach` という関数は存在しない**。実体は `db::fulltext::extract_and_index` |
| p3「既定 OFF 前提のテスト 10 箇所程度」 | **8 本**（tex_autofetch 分離を先にやれば 5 本） |
| p4「`DELETE FROM document_versions` だけで木ごと消える」 | **誤り**。`parent_version_id` は NO ACTION で superseded 145/145 が参照されている。pre-step の UPDATE が必須。CASCADE 表も 4 表ではなく **9 表** |
| 8d-7「解決率の実測上限は約 93%」 | **91.18%（1302/1428）** |
| debt-12「`Fig. 8.3` 形も落ちている」「実測 60 件」「8d-7 の 93% の実体」 | **3 点とも食い違う**（§8.1） |
| §3「実測で効くのは 8d-2」（figure 0 件 69 版を一括で扱う） | caption を持つ 42 版のうち **半分（21 版・caption 211 件）は 8d-8 の担当** |
| §4「`export/mod.rs` は 73 行・追加は `pub mod` 1 行」 | HEAD で **118 行**。実際のグルーは Rust ~70 + TS ~30 + i18n |
| 9b-4「S / 120–200 + テスト 100–150」 | **M / 650–870 行**（削っても 450–550） |
| 「数百本で数十分規模」（`run_build_batch` の doc コメント由来の見積り） | **実測 4,797 秒（80 分）/ 138 PDF、うち 1 添付が 4,514 秒** |
| 8d-7「再構築は必須ではない。辺は次回 rebuild で付く」 | 結果として **pdfium 完了版 138/138 が既に outdated**、`refers_to_figure` は pdfium 側に 1 本も無い。「再構築 1 回」は選択肢ではなく確定した負債 |
| 設計概観 §6「**要検証**: 非ゼロ `/Rotate` で text bounds は回転前/後どちらか」 | **決着**: bounds は回転前 user space、`page.width()/height()` は回転適用後。pdf.js の viewport は両方吸収する |
| debt-14「段落の bbox は影響しない」 | 図については正しいが、同じ取り違えが `structure.rs:365-368` の `in_margin` にもある（debt-18）。さらに **テキスト bbox のページ矩形はみ出し 11,963 件 / 9 版は debt-14 では直らない**（debt-19） |

---

## 11. 実測値の基準（2026-07-30・読み取り専用）

再着手時はここを再計測して差し替えること。

| 項目 | 値 |
|------|-----|
| entries / attachments | 140 / 148（PDF 140・gzip 8） |
| `document_versions` | 291 行（pdfium: 0.6.0 completed 137 + cww 1 / 0.1.0 superseded 135 / 0.5.0 superseded 2。tex: 0.5.0 completed 2 + cww 6 / 0.1.0 sup 1 / 0.4.0 sup 7） |
| コード側の抽出器版 | pdfium `0.7.0` / tex `0.5.0` → **PDF 138/138 が outdated・TeX は 0 件** |
| `document_nodes` | 2,663,234 行（**superseded が 2,213,085 = 83%**） |
| `source_fragments` | 2,659,223 行（fragment_type 別: line 309,992 / block 130,904 / page 7,345 は生存版のみ） |
| DB ファイル | 761,237,504 B（726 MiB・freelist 0）。`document_nodes` 262.4MB + `source_fragments` 173.1MB + 索引 5 本で 83.2% |
| `assets` | 1,198 行 / 555,177,018 B（全て `image/png`・role は `page_crop` のみ）。実ファイル 531.8 MiB / content_key dir 69 個（stale 0） |
| `node_alt_texts` | 888 行・全件 `llm_inference`・全件 completed 版・**`carried_from_version_id` は全件 NULL**（carry は本番で一度も発火していない） |
| figure / figure_caption / caption_of / 未結合 caption | 1,198 / 904 / 262 / 642（うち Algorithm 10 件は対象外＝真の母数 632・556 件は「そのページに figure 0 件」） |
| figure 0 件の版 | 138 中 69（50.0%）。caption を持つのは 42 版（8d-8 担当 21 版 / 8d-2 担当 21 版） |
| 非ゼロ page box 原点の版 | 138 中 12（8.7%）。そこに figure 370 件・alt text 137 件。**クランプ痕跡は 10 図・うち alt text 6 件** |
| 回転ページ | 1 添付 5 ページのみ（att127・`/Rotate 90`）。figure 0 件・回転 skip warning 0 件 |
| LCIR page の C0 制御文字率 | 非空 5,803 ページの **75.3%**（`fulltext` 側は 11.6%） |
| 全ライブラリ再構築 | **4,797 秒（80 分）/ 138 PDF・7,345 ページ**。うち att37（527 ページのスキャン本）が 4,514 秒 = **94%**。残り 137 本は合計 ≒283 秒 |
| Vision alt text | 888 件 / 6,312 秒 ≒ **7.1 秒/図**。model は全件 `claude-sonnet-5`。単価 ≒$0.0068/図（総額 ≒$6 はメモリ由来でリポジトリからは未検証） |
| バックアップ | フル zip（`VACUUM INTO` + attachments 全体・差分も dedup も無し）・keep=14・24h 間隔。現在 5 本 × 831,617,064 B = 3.9 GiB。db.sqlite は raw 737MB → 圧縮 220.6MB |
| disk | 89 GB 空き（91% 使用） |
| テスト本数 | 967（PR #69 のコミットメッセージ）。`#[test]`/`#[sqlx::test]`/`#[tokio::test]` の静的個数は 973 |

**未確定**: `figure_caption` は別々の調査で 904 と 954 の 2 値が出た（集計条件の違いの可能性）。
904 は「生存 pdfium 版に限定」した値で 2 体が一致している。doc に確定値として書く前に再計測すること。

---

## 関連ドキュメント

- `docs/LCIR_design_overview.md` — 設計の決定版（実装済み Phase の詳細）。
  本書 §10 の指摘に沿って **2026-07-31 に 4 箇所を訂正済**:
  §6 の「要検証」（回転ページの bounds）→ 実測で決着 + debt-19 を追記 /
  §8 の「(B) 化は差し替える 1 行」→ 撤回（シグネチャも `(pool, attachment_id)` へ修正）/
  §10 モジュールツリーの `post_attach`（実在しない関数）を削除 /
  Phase 8a 実装状況の「XObjectForm 内画像は追わない」の根拠が未検証の仮説であることと、
  「tikz/pgf ベクター図はアセット 0 件が正当」が 8d-2 で撤回されることを追記
- `docs/LumenCite_machine_readable_document_roadmap.md` — 元ロードマップ（Phase ごとの実装項目・完了条件）
- `docs/RELEASE.md` — §4 に pdfium 同梱の手順。v1.0.0 固有事項の節は未作成（§9）
