# LumenCite 機械可読論文フォーマット実装ロードマップ

## 1. 文書の目的

本書は、LumenCite に保存される論文全文を、単なる全文検索用テキストではなく、数式・図・論文構造を保持した機械可読形式へ段階的に移行するための実装ロードマップである。

現在の LumenCite では、PDFium を介して PDF から抽出した全文を SQLite FTS5 に保存している。この方式は全文検索には有効である一方、以下の情報を十分に保持できない。

- 節、段落、定理、証明、数式、図、表などの論理構造
- 数式の構文構造と意味
- PDF 上の位置と抽出結果の対応
- OCR・数式認識・構造認識の信頼度
- 抽出器や変換器のバージョン
- 図中の軸、データ系列、ノード、エッジなどの意味情報
- 後から抽出精度が改善した場合の再処理可能性

本ロードマップでは、FTS5 を引き続き検索インデックスとして活用しつつ、論文の正規化表現を独立した中間形式として保存する。

---

## 2. 基本方針

### 2.1 三層構造に分離する

LumenCite 内の論文データを、次の三層に分離する。

1. **原資料層**
   - PDF
   - arXiv TeX ソース
   - 出版社 JATS XML
   - HTML + MathML
   - 補助ファイル、画像、表データ

2. **正規化文書層**
   - LumenCite 独自の中間表現
   - 仮称: **LCIR — LumenCite Document Intermediate Representation**
   - 文書構造、数式、図、参照関係、出典、信頼度を保持する

3. **派生インデックス層**
   - SQLite FTS5
   - ベクトル検索
   - 数式検索
   - 記号検索
   - LLM 用チャンク
   - Markdown / HTML / JATS エクスポート

原資料と LCIR を正本とし、FTS5 やベクトル埋め込みは再生成可能な派生データとして扱う。

---

### 2.2 PDF 抽出結果を唯一の正解とみなさない

PDF は最終表示形式であり、論理構造や数式の意味を完全には保持していない場合がある。

取得可能な表現には、原則として次の優先順位を設定する。

1. 出版社 JATS XML
2. arXiv などの TeX ソース
3. HTML + MathML
4. タグ付き PDF
5. 通常 PDF のテキスト・レイアウト解析
6. 画像 PDF の OCR

複数の表現が存在する場合は、一つに上書きせず、由来の異なる表現として併存させる。

---

### 2.3 数式を単一形式に統一しない

数式については、用途の異なる複数表現を保持する。

- PDF 上の元領域
- 元 TeX または推定 LaTeX
- Presentation MathML
- Content MathML
- OpenMath
- 検索用正規化文字列
- 数式構文木
- 記号参照情報
- 推定信頼度

Presentation MathML は数式の表示・構文構造に適している。Content MathML や OpenMath は意味表現に適するが、PDF から自動推定した意味は不確実であるため、必ず推定結果として扱う。

---

### 2.4 文書を型付きノードの木またはグラフとして保存する

論文を一つの巨大な文字列ではなく、型付きノードの集合として表現する。

初期段階で想定するノード型:

- `document`
- `front_matter`
- `abstract`
- `section`
- `subsection`
- `heading`
- `paragraph`
- `list`
- `list_item`
- `definition`
- `theorem`
- `lemma`
- `proposition`
- `corollary`
- `remark`
- `example`
- `proof`
- `display_math`
- `inline_math`
- `equation_group`
- `figure`
- `figure_caption`
- `table`
- `table_caption`
- `footnote`
- `citation`
- `bibliography`
- `bibliography_entry`
- `code_block`
- `unknown_block`

認識に確信がない場合は、誤った型を確定するより `unknown_block` と信頼度を保存する。

---

## 3. 目標アーキテクチャ

```text
PDF / TeX / JATS / HTML
          │
          ▼
Source Ingestion
          │
          ▼
LCIR Document Version
  ├── document tree
  ├── source fragments
  ├── math representations
  ├── figures and assets
  ├── symbols and references
  ├── provenance
  └── confidence scores
          │
          ├── FTS5 page index
          ├── FTS5 semantic-node index
          ├── vector index
          ├── mathematical index
          ├── Markdown / HTML
          ├── JATS XML
          └── LLM retrieval chunks
```

LCIR は論文の意味構造を保持する内部標準であり、特定の抽出器、検索エンジン、LLM、外部規格に依存しないものとする。

---

## 4. LCIR の設計原則

### 4.1 バージョン管理可能であること

すべての LCIR 文書にスキーマバージョンを持たせる。

```json
{
  "schema": "https://lumencite.dev/schema/document-ir/0.1",
  "schema_version": "0.1.0"
}
```

破壊的変更を行う場合はマイグレーション処理を用意する。

---

### 4.2 再現可能であること

抽出結果ごとに以下を記録する。

- 入力ファイルの SHA-256
- MIME type
- 抽出器名
- 抽出器バージョン
- モデル名・モデルバージョン
- 設定値
- 実行日時
- 実行環境
- 親となる文書バージョン
- 手動修正の有無

---

### 4.3 原文と推定を区別すること

各データに由来を付ける。

例:

- `publisher_source`
- `tex_source`
- `pdf_text_layer`
- `ocr`
- `layout_model`
- `math_recognition`
- `llm_inference`
- `user_edited`

推定結果には信頼度を付ける。

```json
{
  "value": "theorem",
  "origin": "layout_model",
  "confidence": 0.82
}
```

---

### 4.4 PDF 上の位置に戻れること

すべてのテキストブロック、数式、図、表について、可能な限り PDF 上の位置を保持する。

```json
{
  "page": 5,
  "bbox": {
    "x": 72.4,
    "y": 181.0,
    "width": 468.2,
    "height": 95.6
  }
}
```

座標系は文書全体で統一し、原点、単位、回転、ページサイズを明示する。

---

### 4.5 欠損を許容すること

すべての表現が必ず取得できるとは限らない。

例:

- LaTeX はあるが MathML がない
- MathML はあるが意味表現がない
- 図の画像はあるが SVG がない
- 定理らしいブロックだが型を確定できない

欠損値を正常な状態として扱い、無理に推定結果で埋めない。

---

## 5. 推奨データモデル

### 5.1 `document_versions`

論文の各抽出・変換結果を管理する。

```sql
CREATE TABLE document_versions (
    id TEXT PRIMARY KEY,
    attachment_id TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    source_mime_type TEXT NOT NULL,
    extractor_name TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    parent_version_id TEXT,
    extraction_status TEXT NOT NULL,
    metadata_json TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (parent_version_id) REFERENCES document_versions(id)
);
```

想定する `extraction_status`:

- `pending`
- `processing`
- `completed`
- `completed_with_warnings`
- `failed`
- `superseded`

---

### 5.2 `document_nodes`

文書の論理構造を保存する。

```sql
CREATE TABLE document_nodes (
    id TEXT PRIMARY KEY,
    document_version_id TEXT NOT NULL,
    parent_id TEXT,
    node_kind TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    plain_text TEXT,
    language TEXT,
    confidence REAL,
    origin TEXT,
    payload_json TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (document_version_id) REFERENCES document_versions(id),
    FOREIGN KEY (parent_id) REFERENCES document_nodes(id)
);
```

`payload_json` にはノード型固有の情報を保存する。

例:

- 節番号
- 定理番号
- 数式番号
- 図番号
- 箇条書き種別
- 引用形式
- 行内要素
- スタイル情報

---

### 5.3 `source_fragments`

LCIR ノードと PDF 上の領域を対応付ける。

```sql
CREATE TABLE source_fragments (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL,
    page_number INTEGER NOT NULL,
    x REAL NOT NULL,
    y REAL NOT NULL,
    width REAL NOT NULL,
    height REAL NOT NULL,
    rotation REAL DEFAULT 0,
    reading_order INTEGER,
    fragment_type TEXT,
    FOREIGN KEY (node_id) REFERENCES document_nodes(id)
);
```

一つの段落や証明が複数ページにまたがる場合、複数レコードを持たせる。

---

### 5.4 `math_expressions`

数式の複数表現を保存する。

```sql
CREATE TABLE math_expressions (
    id TEXT PRIMARY KEY,
    node_id TEXT NOT NULL,
    display_mode TEXT NOT NULL,
    equation_label TEXT,
    latex TEXT,
    presentation_mathml TEXT,
    content_mathml TEXT,
    openmath_json TEXT,
    normalized_text TEXT,
    ast_json TEXT,
    semantic_status TEXT,
    confidence REAL,
    origin TEXT,
    FOREIGN KEY (node_id) REFERENCES document_nodes(id)
);
```

`semantic_status` の例:

- `not_attempted`
- `surface_only`
- `inferred`
- `verified`
- `source_provided`

---

### 5.5 `assets`

図、画像、SVG、表データなどを管理する。

```sql
CREATE TABLE assets (
    id TEXT PRIMARY KEY,
    document_version_id TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    width INTEGER,
    height INTEGER,
    metadata_json TEXT,
    FOREIGN KEY (document_version_id) REFERENCES document_versions(id)
);
```

大きなバイナリを SQLite BLOB に保存するかファイルシステムに保存するかは別途判断する。初期段階では、ファイルシステム保存と SHA-256 による参照を推奨する。

---

### 5.6 `node_assets`

ノードとアセットの関係を保存する。

```sql
CREATE TABLE node_assets (
    node_id TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (node_id, asset_id, role),
    FOREIGN KEY (node_id) REFERENCES document_nodes(id),
    FOREIGN KEY (asset_id) REFERENCES assets(id)
);
```

`role` の例:

- `original`
- `page_crop`
- `vector`
- `thumbnail`
- `ocr_source`
- `plot_data`
- `supplementary`

---

### 5.7 `node_relations`

ノード間の型付き関係を保存する。

```sql
CREATE TABLE node_relations (
    id TEXT PRIMARY KEY,
    document_version_id TEXT NOT NULL,
    from_node_id TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    to_node_id TEXT NOT NULL,
    confidence REAL,
    origin TEXT,
    metadata_json TEXT,
    FOREIGN KEY (from_node_id) REFERENCES document_nodes(id),
    FOREIGN KEY (to_node_id) REFERENCES document_nodes(id)
);
```

想定する関係:

- `cites`
- `refers_to_equation`
- `refers_to_figure`
- `refers_to_table`
- `defines_symbol`
- `uses_symbol`
- `proves`
- `depends_on`
- `caption_of`
- `footnote_of`
- `continues`
- `alternative_representation_of`

---

### 5.8 `symbols`

論文内の記号定義を管理する。

```sql
CREATE TABLE symbols (
    id TEXT PRIMARY KEY,
    document_version_id TEXT NOT NULL,
    surface_form TEXT NOT NULL,
    normalized_form TEXT,
    description TEXT,
    symbol_type TEXT,
    defined_at_node_id TEXT,
    scope_node_id TEXT,
    semantic_json TEXT,
    confidence REAL,
    origin TEXT,
    FOREIGN KEY (document_version_id) REFERENCES document_versions(id)
);
```

例:

```json
{
  "surface_form": "U",
  "description": "time evolution operator",
  "symbol_type": "unitary_operator",
  "semantic_json": {
    "domain": "ell2(Z2; C2)"
  }
}
```

---

### 5.9 `symbol_occurrences`

数式・本文中の記号出現を定義へ関連付ける。

```sql
CREATE TABLE symbol_occurrences (
    id TEXT PRIMARY KEY,
    symbol_id TEXT,
    node_id TEXT NOT NULL,
    local_offset_json TEXT,
    surface_form TEXT NOT NULL,
    confidence REAL,
    origin TEXT,
    FOREIGN KEY (symbol_id) REFERENCES symbols(id),
    FOREIGN KEY (node_id) REFERENCES document_nodes(id)
);
```

---

## 6. LCIR JSON の概念例

```json
{
  "schema": "https://lumencite.dev/schema/document-ir/0.1",
  "schema_version": "0.1.0",
  "document_id": "doc-2408.09578",
  "version_id": "version-001",
  "source": {
    "type": "application/pdf",
    "sha256": "example",
    "extractor": {
      "name": "lumencite-pdf",
      "version": "0.1.0"
    }
  },
  "nodes": [
    {
      "id": "sec-1",
      "kind": "section",
      "ordinal": 1,
      "title": "Introduction",
      "children": ["p-1", "eq-1"]
    },
    {
      "id": "p-1",
      "kind": "paragraph",
      "ordinal": 1,
      "content": [
        {
          "type": "text",
          "text": "We define the time evolution operator by "
        },
        {
          "type": "math_ref",
          "target": "eq-1"
        }
      ],
      "source_fragments": [
        {
          "page": 2,
          "bbox": [71.2, 412.8, 466.1, 43.2]
        }
      ]
    },
    {
      "id": "eq-1",
      "kind": "display_math",
      "ordinal": 2,
      "label": "(2.1)",
      "math": {
        "latex": "U=S_2C_2S_1C_1",
        "presentation_mathml": "<math>...</math>",
        "content_mathml": null,
        "openmath": null,
        "normalized_text": "eq(U,mul(S_2,C_2,S_1,C_1))"
      },
      "confidence": {
        "layout": 0.99,
        "latex": 0.81,
        "semantics": null
      }
    }
  ]
}
```

SQLite を主要ストレージとして利用しつつ、デバッグ、エクスポート、テスト、交換のために LCIR JSON を生成できるようにする。

---

## 7. FTS5 の位置づけ

FTS5 は正本ではなく、LCIR から生成される検索インデックスとする。

### 7.1 ページ単位インデックス

既存のページ単位全文検索を維持する。

用途:

- PDF ビューア上の該当ページへの移動
- 現在の検索 UX の維持
- LCIR 導入前の文献との互換性

```sql
CREATE VIRTUAL TABLE page_fulltext_fts USING fts5(
    attachment_id UNINDEXED,
    page_number UNINDEXED,
    content
);
```

---

### 7.2 意味ブロック単位インデックス

段落、定理、証明、数式、図キャプションなどを単位として検索する。

```sql
CREATE VIRTUAL TABLE document_nodes_fts USING fts5(
    node_id UNINDEXED,
    document_version_id UNINDEXED,
    node_kind UNINDEXED,
    plain_text,
    math_text,
    symbol_text
);
```

用途:

- LLM Retrieval-Augmented Generation
- 定理・定義・証明単位の検索
- 文脈を壊さないチャンク取得
- 図・数式・引用への参照

---

### 7.3 数式検索用派生表現

数式ごとに複数の検索表現を生成する。

例:

```text
raw_latex:
\widehat U(k)=S_2(k)C_2S_1(k)C_1

unicode_linear:
Û(k) = S₂(k) C₂ S₁(k) C₁

identifier_tokens:
U k S_2 C_2 S_1 C_1

operator_tokens:
equality function_application multiplication

structural_tokens:
eq(apply(hat(U),k),mul(apply(S_2,k),C_2,apply(S_1,k),C_1))
```

将来的には以下を検討する。

- 変数名を無視した α 正規化
- 添字正規化
- 可換演算の正規化
- 数式 AST 部分木検索
- 型情報を使った検索
- 記号定義を考慮した検索

---

## 8. 図の機械可読化方針

図を単なる画像として保存せず、複数表現の集合として扱う。

```json
{
  "id": "figure-3",
  "caption": "Probability distribution at time t.",
  "mentions": ["paragraph-42", "paragraph-51"],
  "representations": {
    "original": "asset:sha256:...",
    "page_crop": "asset:sha256:...",
    "svg": "asset:sha256:...",
    "ocr_text": "...",
    "alt_text": "...",
    "scene_graph": null,
    "plot_data": null
  }
}
```

### 8.1 初期段階で保存する情報

- PDF 内の元画像ストリーム
- ページから切り出した画像
- PDF 上の位置
- 図番号
- キャプション
- 本文中の参照位置
- OCR テキスト
- 抽出器と信頼度

### 8.2 ベクトル図

可能な場合は SVG を保存する。

ただし、PDF から抽出した SVG が描画命令の集合にすぎない場合があるため、SVG を意味表現とはみなさない。

### 8.3 プロット

将来的には次の構造を保存する。

```json
{
  "figure_type": "plot",
  "axes": [
    {
      "orientation": "x",
      "label": "time",
      "unit": "s"
    },
    {
      "orientation": "y",
      "label": "probability",
      "unit": null
    }
  ],
  "series": [
    {
      "label": "simulation",
      "points": [[0, 0.1], [1, 0.22], [2, 0.37]]
    }
  ]
}
```

### 8.4 ダイアグラム

```json
{
  "figure_type": "diagram",
  "nodes": [],
  "edges": [],
  "labels": []
}
```

### 8.5 表

表は画像ではなく、セル構造として保存する。

保持対象:

- 行・列
- ヘッダ
- `rowspan`
- `colspan`
- 単位
- 脚注
- セル座標
- 表キャプション

---

## 9. 外部標準との関係

### 9.1 JATS XML

論文交換形式として第一候補とする。

用途:

- 出版社 XML のインポート
- 論文構造を保持したエクスポート
- 数式の TeX / MathML 併存
- 図、表、参考文献、メタデータの交換

LCIR と JATS の完全な一対一対応は要求しない。LCIR 固有の信頼度、PDF 座標、抽出履歴などは JATS 外に保持してよい。

### 9.2 TEI XML

GROBID などの解析結果を受け取る入力形式として利用する。

用途:

- PDF 構造解析ツールとの連携
- 既存 TEI コーパスのインポート
- 人文学系文書への将来対応

### 9.3 MathML

- Presentation MathML: 数式の表示・構文構造
- Content MathML: 数式の意味構造

### 9.4 OpenMath

演算子や数学的対象の意味を明示するために利用する。

すべての PDF 数式を OpenMath に変換することは初期目標としない。

### 9.5 Markdown

Markdown は正本とせず、人間および LLM 向けの派生ビューとする。

---

## 10. 段階的実装ロードマップ

# Phase 0: 設計準備

## 目的

既存実装を壊さずに LCIR を導入できる境界を確定する。

## 実装項目

- 現在の PDF 抽出パイプラインの整理
- 現在の FTS5 スキーマの整理
- 添付ファイルと文献エントリの関係確認
- LCIR モジュールの責務定義
- 座標系の仕様決定
- UUID または安定 ID の生成規則決定
- スキーマバージョニング方針の決定
- 原資料・正本・派生データの区別をコード上に導入
- 実験用 feature flag の追加

## 成果物

- Architecture Decision Record
- LCIR 0.1 の JSON Schema
- SQLite マイグレーション案
- 既存データから LCIR への移行方針
- サンプル論文 5〜10 件のテストコーパス

## 完了条件

- 同一 PDF から同一の文書バージョン ID を再現できる
- 新機能を無効化した場合に既存挙動が変化しない
- LCIR の最小 JSON を schema validation できる

---

# Phase 1: ページ・テキストブロック・出典管理

## 目的

現在のページ全文テキストを、出典と位置情報を持つ再処理可能なデータへ移行する。

## 実装項目

- `document_versions`
- `document_nodes`
- `source_fragments`
- 原 PDF の SHA-256 保存
- PDFium バージョン保存
- ページサイズ、回転、座標系の保存
- ページごとのテキストブロック抽出
- ブロック、行、文字列の読み順保存
- ページ FTS の LCIR 由来再生成
- 抽出失敗・警告ログの保存

## ノード型

- `document`
- `page`
- `text_block`
- `line`
- `unknown_block`

## 完了条件

- 現在の全文検索機能が維持される
- 検索結果から PDF 上の該当領域をハイライトできる
- FTS5 を削除しても LCIR から再構築できる
- 抽出器のバージョン違いを別文書バージョンとして保存できる

---

# Phase 2: 論理構造認識

## 目的

ページ境界ではなく、節・段落・参考文献などの意味ブロックを生成する。

## 実装項目

- 見出し認識
- 段落結合
- 複数ページにまたがる段落の結合
- abstract の認識
- section hierarchy の構築
- figure caption / table caption の認識
- bibliography 領域の認識
- bibliography entry の分割
- footnote の認識
- 意味ブロック単位 FTS5 の追加
- 構造認識信頼度の保存

## ノード型

- `abstract`
- `section`
- `heading`
- `paragraph`
- `figure_caption`
- `table_caption`
- `footnote`
- `bibliography`
- `bibliography_entry`

## 完了条件

- ページをまたぐ段落を一つのノードとして取得できる
- 見出し階層がツリーとして保存される
- LLM に段落単位でコンテキストを渡せる
- ノードから元 PDF の領域へ戻れる
- 認識に失敗したブロックが失われず `unknown_block` として残る

---

# Phase 3: 数式の表層構造

## 目的

数式を通常テキストから分離し、検索・表示・再認識可能な形で保存する。

## 実装項目

- 行内数式領域の検出
- 独立数式領域の検出
- 数式番号の抽出
- 数式と本文参照の関連付け
- LaTeX 認識器との統合
- Presentation MathML 生成
- 数式画像または PDF crop の保存
- 数式認識信頼度の保存
- 数式検索用正規化表現の生成
- 数式 FTS カラムの追加

## ノード型

- `inline_math`
- `display_math`
- `equation_group`

## 完了条件

- 数式を本文文字列とは独立して取得できる
- 数式番号から該当数式へ移動できる
- LaTeX と Presentation MathML を併存できる
- 認識結果と元 PDF 領域を比較できる
- 認識器を変更して再処理しても旧結果を保持できる

---

# Phase 4: TeX・JATS・HTML の直接取り込み

## 目的

PDF より高品質な原資料を利用可能にする。

## 実装項目

- arXiv TeX ソース取り込み
- TeX ファイル依存関係の解決
- LaTeXML などによる構造化変換
- JATS XML インポート
- HTML + MathML インポート
- 複数表現の優先順位管理
- PDF と TeX/JATS の節・数式・図の対応付け
- source-provided と inferred の区別

## 完了条件

- 同一論文の PDF と TeX を別表現として保持できる
- TeX 由来の数式を PDF 認識結果より優先できる
- 出版社 JATS から節、数式、図、表、参考文献を取り込める
- 原資料を切り替えても同一文献エントリに紐付く

---

# Phase 5: 定理・定義・証明構造

## 目的

数学・物理論文に特有の論理構造を機械可読化する。

## 実装項目

- theorem-like environment の認識
- 定理番号の抽出
- definition / theorem / lemma / proposition / corollary の分類
- proof の開始・終了認識
- 定理と証明の関連付け
- 定理間参照の抽出
- 定理・定義専用検索
- LLM 用 theorem-context 生成

## ノード型

- `definition`
- `theorem`
- `lemma`
- `proposition`
- `corollary`
- `remark`
- `example`
- `proof`

## 完了条件

- 定理と証明を独立ノードとして取得できる
- 「定理 2.3 の証明」を一つの問い合わせで取得できる
- PDF 上の型判定が不確実な場合、信頼度付きで保存される
- TeX 由来環境名を優先できる

---

# Phase 6: 記号・参照グラフ

## 目的

数式の表面表現だけでなく、論文中の記号定義と使用関係を表現する。

## 実装項目

- 記号候補の抽出
- “let”, “define”, “denote” などの定義文認識
- 記号定義ノードとの関連付け
- 記号のスコープ推定
- 同一表記の異なる意味の分離
- 数式内記号出現の参照解決
- equation / figure / theorem reference の解決
- citation graph との統合
- relation confidence の保存

## 完了条件

- 記号を選択して定義位置へ移動できる
- 同一記号の異なるスコープを区別できる
- 数式を構成する主要記号の説明を取得できる
- 推定された意味と原文由来情報が明確に分離される

---

# Phase 7: 数式意味表現

## 目的

数式の構文木と意味表現を段階的に導入する。

## 実装項目

- 数式 AST の定義
- 演算子・関数・識別子の分類
- Content MathML 変換
- OpenMath 変換
- 型情報の推定
- α 正規化
- 部分式検索
- 数式類似度検索
- 記号定義を利用した曖昧性解消
- ユーザーによる意味確認・修正 UI

## 注意事項

PDF から意味を完全に復元できると仮定しない。

例として `AB` が以下のどれかは文脈なしには確定できない。

- 数の積
- 行列積
- 作用素積
- 関数適用
- テンソル積の省略表記

意味表現には必ず `semantic_status` と `confidence` を付ける。

## 完了条件

- 数式 AST を保存・再生成できる
- 変数名の違いを無視した検索ができる
- 意味推定が未確定であることを UI と API で表現できる
- ユーザー修正を上書きせず、新しい provenance として保存できる

---

# Phase 8: 図・表の機械可読化

## 目的

図、プロット、ダイアグラム、表を複数表現で保存する。

## 実装項目

- 図領域の抽出
- 元画像ストリームの保存
- ページ crop の保存
- SVG 抽出
- OCR
- キャプションとの関連付け
- 本文中の figure reference 解決
- plot / diagram / photo / table の分類
- プロット軸・凡例・系列の抽出
- ダイアグラムの node-edge 表現
- 表のセル構造抽出
- alt text 生成
- 図表抽出信頼度の保存

## 完了条件

- 図番号から画像、キャプション、本文参照を取得できる
- 表をセル単位で検索・コピーできる
- プロットの軸ラベルと凡例を取得できる
- 元画像、SVG、OCR、構造化データを併存できる

---

# Phase 9: 外部エクスポートと相互運用

## 目的

LCIR を外部形式へ変換し、ベンダーロックインを避ける。

## 実装項目

- LCIR JSON エクスポート
- JATS XML エクスポート
- TEI XML エクスポート
- Markdown エクスポート
- HTML + MathML エクスポート
- Web Annotation 互換領域注釈
- JSON-LD または RDF への将来拡張
- スキーマ互換性テスト

## 完了条件

- 一つの論文を LCIR JSON と JATS XML に出力できる
- エクスポート後に主要構造が失われない
- 不完全な情報を無理に標準形式へ変換しない
- LCIR 固有情報が失われる場合に警告を出せる

---

# Phase 10: LLM・エージェント向け利用

## 目的

LCIR を LLM にとって高品質な研究情報基盤として利用する。

## 実装項目

- ノード単位のチャンク生成
- 定理 + 前提定義 + 証明のコンテキスト生成
- 数式参照を展開したチャンク生成
- 図キャプションと本文説明を統合したチャンク生成
- provenance 付き回答生成
- PDF ページ・領域への引用リンク
- embedding の再生成管理
- モデル変更時のインデックス再構築
- 文献横断の記号・定理・引用グラフ

## 完了条件

- LLM 回答から根拠ノードと PDF 領域へ移動できる
- ページ境界で文脈が切れない
- 数式をプレーンテキストだけでなく構造化表現として渡せる
- AI 推定部分を回答中で識別できる

---

## 11. 推奨実装順序

最初の実用的なリリースでは、以下の順序を推奨する。

1. 文書バージョンと provenance
2. PDF 座標付きテキストブロック
3. 段落・見出し・参考文献
4. 意味ブロック単位 FTS5
5. 独立数式と数式番号
6. LaTeX + Presentation MathML
7. TeX / JATS 取り込み
8. 定理・定義・証明
9. 記号・参照グラフ
10. 図・表の構造化
11. Content MathML / OpenMath
12. 高度な数式検索と研究グラフ

Content MathML、OpenMath、図の意味解析は重要だが、最初から完全実装を目指さない。まずは原資料、位置、構造、由来を失わない基盤を作る。

---

## 12. 既存データの移行方針

既存 FTS5 データを破壊的に変更しない。

### 移行手順

1. 既存添付 PDF ごとに `document_versions` を作成
2. 既存ページ全文を `page` または `text_block` ノードとして取り込む
3. 既存 FTS5 を legacy index として維持
4. 新しい LCIR 由来インデックスを並行運用
5. 検索結果品質を比較
6. 十分な互換性が確認できた後、新インデックスを既定にする
7. legacy index は再生成可能になった時点で削除候補とする

既存データに PDF 座標がない場合は、再抽出可能な PDF のみ後から座標付き LCIR を生成する。

---

## 13. テスト戦略

### 13.1 テストコーパス

以下を含む小規模な固定コーパスを用意する。

- 1 段組論文
- 2 段組論文
- arXiv TeX が取得できる論文
- JATS XML が取得できる論文
- 数式が多い数学論文
- 図が多い物理論文
- 表が多い論文
- スキャン PDF
- 日本語論文
- 複数ページにまたがる定理・証明
- Appendix を含む論文
- Supplementary material を含む論文

### 13.2 Golden File Test

同じ入力から生成される LCIR JSON を固定し、差分を検査する。

抽出器の更新で差分が生じる場合は、意図された改善か回帰かを確認する。

### 13.3 Schema Validation

すべての LCIR JSON を JSON Schema で検証する。

### 13.4 Round-trip Test

可能な範囲で次を検査する。

```text
JATS → LCIR → JATS
TEI → LCIR → TEI
LCIR → Markdown
LCIR → HTML + MathML
```

完全一致ではなく、主要な意味構造が保持されることを検証する。

### 13.5 Search Regression Test

既存全文検索と新しいノード検索について、代表的なクエリ結果を比較する。

### 13.6 Coordinate Test

検索結果やノードから PDF 上の正しい領域をハイライトできることを検証する。

---

## 14. パフォーマンスとストレージ

### 14.1 遅延処理

すべての解析を PDF 登録時に同期実行しない。

推奨:

- 基本全文抽出は即時
- 構造解析はバックグラウンドジョブ
- 数式認識は必要時またはアイドル時
- 図意味解析はオンデマンド
- embedding はモデルごとに非同期生成

### 14.2 キャッシュ

抽出結果は以下をキーにキャッシュする。

- source SHA-256
- extractor name
- extractor version
- configuration hash

### 14.3 大容量データ

以下は SQLite 外に保存することを検討する。

- 元 PDF
- 高解像度 page crop
- SVG
- OCR 中間画像
- 数式画像
- 補助ファイル

SQLite には相対パス、SHA-256、MIME type、メタデータを保存する。

---

## 15. セキュリティとプライバシー

- ローカル文献を外部 API に送信する場合は明示的な同意を得る
- 抽出器ごとにローカル実行・クラウド実行を区別する
- API に送信したページ、数式、図を監査ログに記録できるようにする
- 機密文献ではクラウド処理を禁止できる設定を用意する
- 一時ファイルを安全に削除する
- 文献データと embedding のエクスポート範囲を明示する

---

## 16. 非目標

初期段階では以下を目標にしない。

- あらゆる PDF の完全な論理構造復元
- すべての数式の意味の自動確定
- すべての図から元データを完全復元
- 任意の TeX マクロの完全展開
- JATS、TEI、OpenMath への完全な可逆変換
- AI 推定結果を人手確認なしで真実として扱うこと
- 一つの万能フォーマットへの統一

---

## 17. 実装上の重要な判断事項

着手前または Phase 0 で以下を決定する。

1. LCIR の主ストレージを SQLite 正規化テーブルとするか、JSON blob と併用するか
2. バイナリアセットを SQLite BLOB に保存するか、ファイルシステムに保存するか
3. ノード ID を UUID とするか、内容由来の安定 ID とするか
4. PDF 座標系をどの規則に統一するか
5. document version の差分管理を行うか、完全スナップショットとするか
6. 抽出ジョブのキューをどのように実装するか
7. ユーザー修正と再抽出結果をどうマージするか
8. TeX/JATS/PDF 間の対応付けをどの粒度で行うか
9. Rust 側の型定義と JSON Schema のどちらを一次仕様とするか
10. LCIR を公開仕様にする時期

---

## 18. 推奨モジュール構成

```text
src/
  document_ir/
    mod.rs
    schema.rs
    node.rs
    relation.rs
    source.rs
    math.rs
    figure.rs
    symbol.rs
    validation.rs

  ingestion/
    pdf/
      mod.rs
      pdfium.rs
      layout.rs
      coordinates.rs
    tex/
      mod.rs
    jats/
      mod.rs
    tei/
      mod.rs
    html/
      mod.rs

  indexing/
    page_fts.rs
    node_fts.rs
    math_index.rs
    vector_index.rs

  export/
    lcir_json.rs
    markdown.rs
    html.rs
    jats.rs
    tei.rs

  jobs/
    extraction.rs
    math_recognition.rs
    figure_analysis.rs
    embedding.rs

  storage/
    document_versions.rs
    document_nodes.rs
    assets.rs
    relations.rs
```

Rust の型は `serde` によるシリアライズを前提とし、未知のフィールドを可能な限り保持できる設計が望ましい。

---

## 19. 最初のマイルストーン案

### Milestone A: Reproducible PDF Extraction

- PDF SHA-256
- document version
- extractor version
- page text
- source fragment
- 再生成可能な page FTS

### Milestone B: Semantic Text Blocks

- heading
- paragraph
- bibliography
- figure caption
- node FTS
- PDF ハイライト

### Milestone C: Mathematical Surface Representation

- display math
- equation labels
- LaTeX
- Presentation MathML
- 数式検索用文字列

### Milestone D: Source-aware Ingestion

- arXiv TeX
- JATS
- HTML + MathML
- 複数表現の優先順位

### Milestone E: Mathematical Knowledge Structure

- theorem
- proof
- definition
- symbol
- reference graph

### Milestone F: Figure and Table Intelligence

- figure assets
- OCR
- SVG
- table cells
- plot metadata
- diagram graph

---

## 20. 最終到達像

LumenCite が目指すべき内部構造は、単なる PDF 全文データベースではなく、次の性質を持つ研究文書基盤である。

- 元資料へ常に戻れる
- 抽出結果を再現できる
- PDF 上の位置を失わない
- 数式を複数表現で保持できる
- 定理、証明、定義、図、表を独立オブジェクトとして扱える
- 記号の定義と使用関係を追跡できる
- AI 推定と原文由来情報を区別できる
- FTS5、ベクトル検索、数式検索を再生成できる
- JATS、TEI、MathML、OpenMath などの標準と接続できる
- 将来の LLM や研究エージェントが利用しやすい
- 人間にとっても検証可能である

最優先事項は、高度な意味理解を一度に実現することではない。

**原資料、位置、構造、由来、信頼度を失わずに保存できる基盤を先に作ること**が重要である。その基盤があれば、数式認識、図解析、記号解決、LLM、知識グラフなどの技術が今後改善した際にも、既存文献を再処理しながら LumenCite を継続的に進化させられる。
