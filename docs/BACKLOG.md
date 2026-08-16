# 実装したい機能（v1.0.0 以降）

LCIR の post-1.0 項目は `LCIR_REMAINING_PHASES.md` §0 が正本。
**このファイルは LCIR 以外の UX 機能**を置く。

各項目の「現状」は 2026-08-10 に `51a5c26` の実コードを読んで確かめた事実で、
doc やコメントの記述ではない。着手時には行番号を関数名で grep し直すこと。

---

## B-1 エントリ一覧にコメント列を出す

コメント（`entries.notes`）の先頭を一覧で読めるようにする。

### 現状

- コメントの実体は **`entries.notes`**（`abstract` / `summary` とは別列）。
- 一覧の列は 7 つで固定 ── タイトル / 著者 / 雑誌 / 年 / 種別 / タグ / 追加日
  （i18n の `entriesTable.col*`）。
- `EntriesTable.tsx`（402 行）は列を配列で持たず、`widths` オブジェクト
  （`title` / `authors` / `journal` / `year` / `venue` / `tags` / `added`）の
  キーごとに固定幅の `div` を並べる。列はドラッグでリサイズできる。

### 実装の要点

列 1 本の追加は 3 か所（`widths` のキー / セルの `div` / `ColumnHeader`）で足りる。
**保存済みの列幅**を持っている場合は、新しいキーが無い既存の保存値に既定値を補う経路が要る。

### 決めること

- 先頭何文字を出すか。改行はどう畳むか（`notes` は複数行になりうる）。
- 列の既定幅と、既定で表示するか隠すか（7 列でも横幅は既に厳しい）。
- ソート対象にするか（現在 `sortable` はタイトル・追加日など一部のみ）。
- 空のときの見え方。

---

## B-2 コメントを全文検索の対象にする

現在、コメントは検索に一切かからない。

### 現状

`entries_fts` の列は **5 つだけ**（`migration 0002_entries_fts.sql`）。

```sql
CREATE VIRTUAL TABLE entries_fts USING fts5(
    title, authors_text, tags_text, abstract_text, identifiers,
    tokenize = 'trigram'
)
```

**`notes` は入っていない。`summary`（LLM 要約）も入っていない。**
投入は `db::entries::sync_entries_fts`（`entries.rs:79`）の 1 か所。

### 実装の要点

⚠ **FTS5 は `ALTER TABLE ... ADD COLUMN` ができない。** 列を増やすには
仮想テーブルを作り直して全件を入れ直す必要がある。

前例がそのまま使える ── v0.3.0 で `authors_text` の合成規則を変えたときの
`rebuild_authors_fts_once`（起動時に 1 回だけ走り、`settings` のフラグ
`fts.authors_v030_rebuilt` で冪等にする形）。同型の 1 回きり再構築を足せばよい。
実ライブラリは entries が 130 件規模なので所要は秒オーダー。

`tokenize = 'trigram'` なので、日本語で書いたコメントもそのまま引ける。

### 決めること

- **独立列にするか `abstract_text` に混ぜるか。** 独立列にすると
  `bm25(entries_fts)` の重み付けと、検索結果に「どこがヒットしたか」を
  出すときの分岐が変わる。混ぜると実装は軽いが由来が消える。
- `summary` も同時に入れるか（同じ作り直しの機会に乗せられる）。
- migration 番号は **v1.0.0 の後**（`0021` 以降）。v1.0.0 は migration 0 件で通す方針。
- 既存の LIKE フォールバック経路（`entries.rs:204` 付近）にも同じ列を足すこと。

---

## B-3 PDF をドラッグ&ドロップしてエントリを作る

PDF をアプリに落とすと、中の DOI / arXiv ID を読んでエントリを作り、その PDF を添付する。

### 現状

- **OS のファイルドロップを受けるハンドラがコード上に 1 つも無い。**
  `onDragDropEvent` / `tauri://drag-drop` / `file-drop` はフロント・Rust とも 0 件。
  いま存在する DnD は「エントリ行 → コレクション / タグ / ビュー」の**アプリ内**の
  もので、WKWebView で HTML5 DnD が動かないためポインタイベントで実装されている。
- PDF の入口は **`pickAndAttachPdf`（`src/lib/attachments.ts:10`）→ `pick_pdf_file`
  （ファイルダイアログ）→ `add_attachment` の 1 本だけ**で、
  **呼ぶには既存のエントリが要る**（詳細パネル / リーダーから）。
  「PDF からエントリを作る」入口は存在しない。
- 識別子から書誌を取る経路は既にある ── `metadata::fetch_by_doi`（CrossRef）/
  arXiv API / Open Library。arXiv ID の正規化は `metadata::normalize_arxiv_id`。
  **ただし本文テキストから DOI を抜き出すヘルパは無い**（新規）。
- 重複検出の下地もある（`doi_canonical` / `arxiv_canonical` / `isbn_canonical` の
  UNIQUE 索引・CR-019）。

### 実装の段取り

1. Tauri 2 の webview drag-drop イベントを購読する。
   `tauri.conf.json` の `app.windows[].dragDropEnabled` は既定 true で、
   **これが HTML5 の drop イベントをブロックしている側**でもある。どちらの経路で
   受けるかを最初に決める。
2. 落ちてきた PDF からテキストを取る（pdfium と pdf_extract は既に依存にある）。
3. 識別子を拾う。**埋め込みメタデータ（XMP / DocInfo）に DOI が入っていることが
   あるので、本文の正規表現より先にそちらを見る価値がある。**
4. 既存の取得経路（CrossRef / arXiv / Open Library）に載せる。
5. エントリを作って `add_attachment` → 既存の `ingest_new_pdf_attachment` に乗る。
   ここから先は全文索引と LCIR の自動 build が既に配線済み（v1.0.0-p2）。

### 決めること

- **識別子が取れなかったとき**どうするか（手動入力へフォールバック / タイトル行の
  推定で CrossRef を検索 / そのまま「添付だけ」で作る）。
- 複数ファイルの一括ドロップを許すか。許すなら arXiv・CrossRef へのレート配慮が要る
  （既存の e-print 一括取得は 3 秒スロットル）。
- 重複したときの見せ方。`AddSheet` には既に「同じ識別子の文献が既にあります /
  既存を表示 / 重複してでも追加」の UI がある（`addSheet.identifier.duplicateWarn`）ので、
  それを再利用できる。
- ドロップ先（一覧全体か専用ゾーンか）と、PDF 以外を落とされたときの扱い。
- `.bib` を落としたら BibTeX インポートに回す、まで広げるか。

---

## B-4 リリース基盤の穴（v1.0.0 のタグ前検証で見つけた・post-1.0）

タグを止める理由にはならないが、**次のリリースまでに塞いでおくと安い**もの。
いずれも 2026-08-16 に実ファイル・実ワークフローで確かめた。

### B-4-1 タグ名とアプリ版の一致を機械が見ていない

`release.yml` は updater の pubkey がプレースホルダでないことは検査するが、
**`v1.0.0` というタグと `tauri.conf.json` の `version` が一致するかは誰も見ていない**。
版 bump を忘れてタグを打つと、**CI は緑のまま「中身が前の版のドラフト」が出来上がる**。
`latest.json` も古い版を指すので、macOS の updater には「新しい版が無い」ように見える。

**実装の要点**: pubkey ガード（`release.yml` の `Guard against placeholder updater pubkey` ステップ）の隣に、
`startsWith(github.ref, 'refs/tags/')` のとき `jq -r .version src-tauri/tauri.conf.json` と
`${GITHUB_REF_NAME#v}` を比べて不一致なら fail する数行を足すだけ。
**⚠ ここを触るとリリース経路そのものが壊れうる**ので、タグ直前ではなく版と版の間に入れる。

### B-4-2 `DO NOT PUBLISH` の印が原因を誤って名指しする

ドラフトのタイトルを書き換えるステップの条件は
`failure() && matrix.platform == 'ubuntu-22.04' && startsWith(github.ref, 'refs/tags/')` で、
**Linux ジョブの任意の失敗**で発火する。apt の失敗でも「Linux pdfium verify failed」と付く。
逆に**ドラフトが作られる前**に落ちると `gh release edit` 自体が失敗し、
`::warning::` が 1 行出るだけで**印は付かない**。

**実装の要点**: verify ステップに `id:` を付けて `steps.<id>.conclusion == 'failure'` で絞るか、
文言を `DO NOT PUBLISH — Linux job failed` に一般化する。当面は運用（タグ後に run を必ず目視）で埋める。

### B-4-3 `.rpm` はタグを打つまで一度も作られない

`linux-bundle-verify` が作るのは `--bundles deb,appimage` の 2 つだけなので、
**`.rpm` に対する同梱検査はタグ時の `release.yml` が初回**になる。
rpm だけで落ちるとドラフトを破棄してタグを打ち直すことになる。

**実装の要点**: `linux-bundle-verify.yml` の `--bundles` に `rpm` を足す（実行時間は伸びる）。
このファイル自身が同ワークフローの `paths` に載っているので、変更 PR で自動的に回る。

### B-4-4 `linux-bundle-verify` の `paths` に `src-tauri/Cargo.toml` が無い

pdfium の探索候補の 1 つは `env!("CARGO_PKG_NAME")` から来る（`ingestion/pdf/pdfium.rs`）ので、
**crate 名を変えると探索先が変わるのに検査は走らない**。
v1.0.0 では crate 名は不変（`lumencite`）なので実害は無い。
足すときは、同ファイル冒頭の対応表に理由を「答えを変えうる」ではなく
**「候補の 1 つが crate 名から来る」**と書くこと。
