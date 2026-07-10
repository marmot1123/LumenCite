# コードベースレビュー（2026-07-11）

> **状態: 完了**
> この文書はレビュー結果と、将来の修正作業を中断・再開するためのチェックポイントを兼ねる。

## 1. レビュー基準

- 対象コミット: `1dd8b82` (`main` / `origin/main`)
- 対象規模: 約 4 万行（`src/`, `src-tauri/`, `extension/`）
- 観点: 正しさ、データ損失、セキュリティ、並行性、契約整合、性能、テスト不足
- 重要度:
  - **P0 Critical**: 通常操作で回復不能なデータ損失、または直ちに悪用可能な重大欠陥
  - **P1 High**: 主要機能の破損、秘密情報漏洩、重大な整合性・安全性違反
  - **P2 Medium**: 条件付きの不正動作、信頼性・性能問題、将来の障害要因
  - **P3 Low**: 限定的な不具合、保守性・品質ゲート不足

最終集計は **P0: 1件、P1: 9件、P2: 25件、P3: 4件**。最優先は、現役文献を
回復不能に削除できる CR-001 である。

## 2. 現在の進捗

- [x] リポジトリ構造、仕様、主要コマンド、CI の把握
- [x] データ層レビュー（DB、migration、BibTeX、backup、attachment）
- [x] インターフェース層レビュー（Tauri、CLI、MCP、LLM、OCR）
- [x] フロントエンド・ブラウザ拡張レビュー
- [x] frontend / Rust / extension の基準ビルド・テスト
- [x] frontend の `invoke` と Rust handler の静的照合
- [x] 各サブレビュー指摘の親レビューによる再検証
- [x] 重複指摘の統合、重要度の最終決定
- [x] 仕様書との不整合一覧の最終化
- [x] 最終レポート化

## 3. 実行済み検証

| 検証 | 結果 |
|---|---|
| `pnpm build` | 成功。main chunk 778.97 kB の警告あり |
| `(cd src-tauri && cargo test)` | 446 passed / 0 failed / 1 network test ignored |
| `(cd src-tauri && cargo clippy --all-targets --all-features)` | 成功、11 warnings（重複を含む） |
| `pnpm --filter lumencite-clipper test` | 3 files / 10 tests 成功 |
| `pnpm --filter lumencite-clipper build` | 成功 |
| `pnpm audit --prod --audit-level high` | 既知脆弱性なし |
| `cargo audit` | 4 vulnerabilities / 18 warnings。うち product で直接到達する `lopdf` high を P1 扱い |
| desktop / extension の英日翻訳キー比較 | 差分なし |
| frontend `invoke` / Rust handler 比較 | `get_setting`, `set_setting` のみ未実装・未登録 |

Rust テストは sandbox 内では localhost bind を使う 7 件が権限制約で失敗したが、
sandbox 外の再実行では全件成功した。実装上の失敗ではない。

## 4. 親レビューで確認済みの指摘

### CR-001: Trash 検索から現役エントリを完全削除できる

- **重要度: P0 Critical**
- 根拠:
  - `src/App.tsx:251-275` は検索時に `selectedView` / `viewName` を渡さない。
  - `src-tauri/src/db/entries.rs:216-220` と
    `src-tauri/src/db/fulltext.rs:198-200` は現役エントリだけを返す。
  - UI は Trash 状態を維持し、`src/App.tsx:397-413,1006-1027` は選択 ID や
    `entries` の ID を `bulk_purge` へ渡す。
  - `src-tauri/src/db/entries.rs:878-899` の purge 側にも
    `deleted_at IS NOT NULL` の防御条件がない。
- 影響: Trash 内で検索したユーザーが、検索結果を「削除済み」と信じて現役文献を
  回復不能に削除できる。メタデータ検索、全文検索、Empty Trash の経路が影響する。
- 推奨修正:
  1. 検索 API に view 条件を追加する。
  2. purge は DB 側で `deleted_at IS NOT NULL` を必須にする。
  3. Empty Trash は表示中 ID ではなく専用 backend command で実装する。
  4. Trash + metadata/fulltext search + purge の統合テストを追加する。

### CR-002: ツール承認設定と PDF 最終ページ保存が存在しない command を呼ぶ

- **重要度: P1 High**
- 根拠:
  - `src/components/settings/ChatSettingsTab.tsx:468-480` と
    `src/components/detail/DetailView.tsx:108-136` は `get_setting` / `set_setting` を呼ぶ。
  - `src-tauri/src/lib.rs:2725-2832` に command 実装・登録がない。
  - `docs/API_SPEC.md:552-585` には存在する API として記載されている。
- 影響: 「自動実行」から「確認」に変更した UI が成功したように見えても保存されず、
  既定 auto の mutation が確認なしで継続する。PDF の最終ページも復元されない。
- 推奨修正: 任意キーを書ける汎用 setter ではなく、検証付きの用途別 command を追加し、
  保存失敗を UI に表示する。frontend invoke / backend handler の契約テストも追加する。

### CR-003: Web Clipper の PDF 自動取得に SSRF がある

- **重要度: P1 High**
- 根拠:
  - `extension/src/extract.ts:65-79` はページ管理下の `citation_pdf_url` を採用する。
  - `src-tauri/src/mcp_server/clipper.rs:222-229` は URL を無検証で job にする。
  - `src-tauri/src/download.rs:29-42` は既定 redirect policy で URL を GET し、
    PDF magic byte 検証はリクエスト後にしか行わない。
- 影響: 悪意あるページをクリップすると loopback、private、link-local 宛て GET や、
  redirect 経由の内部アクセスが発生する。
- 推奨修正: `http`/`https` 以外を拒否し、DNS 解決結果と各 redirect 先について
  loopback/private/link-local/unspecified を拒否する。直接・redirect SSRF テストを追加する。

### CR-004: 署名リリースへ未検証の `latest` pdfium を混入できる

- **重要度: P1 High（サプライチェーン）**
- 根拠: `.github/workflows/release.yml:86-95` は
  `bblanchon/pdfium-binaries/releases/latest` から tarball を取得し、version pin と checksum
  検証なしで `libpdfium.dylib` を署名対象へコピーする。
- 影響: upstream release の侵害・差し替え時に、第三者バイナリが LumenCite の署名・
  notarization を受けて配布される。
- 推奨修正: release version と SHA-256 を固定し、展開前に検証する。可能なら provenance
  検証または自前で再現可能ビルドを行い、GitHub Actions も commit SHA pin する。

### CR-017: 未信頼 PDF を処理する `lopdf` に既知の high 脆弱性がある

- **重要度: P1 High**
- 根拠: `cargo audit` が `lopdf 0.36.0` に
  `RUSTSEC-2026-0187`（深くネストした PDF object による stack overflow、CVSS 7.5）を検出。
  依存経路は `lumencite -> pdf-extract 0.9.0 -> lopdf 0.36.0`。
- 影響: 手動添付や外部取得した未信頼 PDF を全文索引すると、プロセス crash/DoS が
  発生し得る。`src-tauri/src/lib.rs:795-823,844-896` が実際の解析入口。
- 推奨修正: `lopdf >= 0.42.0` を含む `pdf-extract` へ更新する。更新不能なら別 parser、
  subprocess 分離、ページ/object 深さ制限を検討し、adversarial PDF regression test を追加する。

### CR-005: ORCID が一致しない同名著者を自動統合する

- **重要度: P1 High**
- 根拠: `src-tauri/src/db/authors.rs:136-154` は入力 ORCID の検索に失敗すると、
  ORCID の有無に関係なく正規化済み氏名だけで既存著者を返す。
- 影響: 既存の `John Smith` と、新しい ORCID を持つ別人の `John Smith` が同一人物へ
  統合され、新しい ORCID も保存されない。文献の著者関係が静かに破損する。
- 推奨修正: stable identifier が与えられた場合、identifier miss 後の氏名だけの統合を
  禁止し、新規作成または明示的な名寄せ確認へ回す。homonym regression test を追加する。

### CR-009: Windows で BibTeX 自動同期が2回目以降失敗する

- **重要度: P1 High**
- 根拠: `src-tauri/src/bibtex.rs:516-530` は固定名の一時ファイルを書き、
  `std::fs::rename(tmp, existing_destination)` で置換する。Windows の rename は既存宛先を
  上書きしない。
- 影響: 最初の同期後、文献を更新しても `.bib` が更新されず、LaTeX 側が古い引用情報を使う。
  固定 `.tmp` 名は同時手動同期と debounce 同期でも競合する。
- 推奨修正: 一意な同一ディレクトリ temp file と、Windows 対応の atomic replace を使い、
  同期処理を直列化する。Windows CI で複数回上書きテストを追加する。

### CR-010: Chat の Stop 後にも書込み tool が実行され得る

- **重要度: P1 High**
- 根拠: `src-tauri/src/llm/chat.rs:112-129` は LLM 呼出し前にしか cancel を確認せず、
  streaming 完了後は `:132-190` で assistant 永続化、承認登録、自動承認 tool 実行へ進む。
- 影響: ユーザーが停止した後に `add_tag`、`update_notes` 等が実行される。停止後に登録された
  confirm-required call は、既に cancel が承認待ちを掃除した後なので永久待機にもなり得る。
- 推奨修正: provider stream に cancellation token と idle timeout を渡し、永続化・承認登録・
  tool 実行の各直前に fail-closed で再確認する。

### CR-012: 外部 MCP の API key 等を SQLite と frontend に平文で保持する

- **重要度: P1 High**
- 根拠:
  - `src/components/settings/ChatSettingsTab.tsx:158-172` は任意 env 値を入力させる。
  - `src-tauri/src/lib.rs:1495-1514` は env を含む config JSON を `settings` へ保存する。
  - `src-tauri/src/lib.rs:1467-1492` は値を frontend にそのまま返す。
  - `docs/DATA_MODEL.md:320` は API key を平文 settings に置かない契約を明記する。
- 影響: 外部 MCP 用 token/API key が DB backup、診断用 DB コピー、WebView compromise から漏れる。
- 推奨修正: env の秘密値を keychain reference に分離し、一覧 API は key 名と設定有無だけを返す。
  既存 plaintext 値の migration と credential rotation 案内も必要。

### CR-015: 複数 PDF 間でハイライトが混在する

- **重要度: P1 High**
- 根拠: `src-tauri/migrations/0005_highlights.sql:3-17` と対応 DTO は highlight を
  `entry_id + page` だけで識別する。`src/components/detail/DetailView.tsx:212-219` は entry の
  全 highlight を読み、選択中 attachment に関係なく表示する。
- 影響: primary PDF 3ページ目の highlight が supplement PDF 3ページ目にも現れ、編集・削除も
  元 PDF を区別できない。v0.8 の複数 PDF 機能で注釈の意味が破損する。
- 推奨修正: `attachment_id` を schema/API/UI に追加し、既存行を primary attachment へ移行する。
  attachment 切替、削除、同一ページ番号の migration test を追加する。

監査で同時に検出された `quick-xml 0.39.3` の high 2件は `plist`/build・platform 経路で、
現時点では未信頼 XML の直接入力面を確認できていないため P2 の依存更新課題とする。
`rsa 0.9.10` は lockfile 内の非使用 `sqlx-mysql` 経路で、現在の SQLite build には含まれない。

## 5. P2 Medium の指摘

| ID | 問題と影響 | 根拠 | 推奨修正 |
|---|---|---|---|
| CR-006 | 著者 API の write/read contract が複数箇所で欠落する。`author_ids` は無視され、structured author の ORCID 以外の identifier は新規作成時に失われ、merge 後の ORCID 二重表現も同期されない。一覧/detail の `Author.identifiers` は常に空で、TS の `EntryDetail.journal` も runtime に存在しない。 | `db/authors.rs:24-36,228-289,464-485`; `db/entries.rs:371-380,730-745`; `models.rs:33,140-166,258`; `src/types.ts:278,322-344` | 入力優先順位を定義して ID と structured input を同一 transaction で保存する。summary/full DTO を分離し、Rust/TS/API contract test を追加する。 |
| CR-007 | 3階層以上の collection が名称順によって tree から消える。DB 行は残るが UI/CLI から辿れない。 | `db/collections.rs:21-39` | ID map と adjacency list から再帰的に構築し、深さ3以上と名前順 permutation のテストを追加する。 |
| CR-008 | attachment の DB・実ファイル・全文索引 lifecycle が非原子的。index 削除エラーと物理削除エラーを無視し、同名保存は TOCTOU のため、orphan text/file や2行で1ファイル共有が起こる。 | `lib.rs:285-290,637-660,729-738`; `download.rs:82-103,165-188`; `db/fulltext.rs:6-30` | exclusive create + path UNIQUE、DB trigger/transaction、rename-to-trash と永続 retry queue を導入する。delete/index race test を追加する。 |
| CR-011 | CLI は MCP probe の全失敗を「GUI停止」と解釈し、既定で MCP server が無効でも live DB へ直接 write する。UI refresh がなく、警告も明示 `--force` 時だけ。 | `cli/write.rs:39-89,118-145` | MCP 設定と独立した single-instance IPC/lock で GUI 生存を判定し、停止を証明できない場合は fail closed または `--force` を要求する。 |
| CR-013 | 生成 MCP command/config に bearer token を埋め込み、shell history や JSON/TOML に残す。stdio shim は同一 binary なのに env token を必須とする。 | `lib.rs:2331-2405`; `mcp_shim.rs:92-107` | shim は keychain から token を取得する。direct HTTP client 向けは漏洩リスクと file permission を明記し、shell history に残らない設定方法を提供する。 |
| CR-014 | chat runtime と承認 state に競合がある。同一 session の並行 send は cancel flag を上書きし、approval は provider 管理の `call_id` だけで keying、timeout なし。session を切替えると frontend は pending approval を消して復元不能。 | `lib.rs:1202-1263,1312-1315,1639-1667`; `src/chat/store.ts:91-105`; `src/chat/messages.ts:152-180` | session ごとに単一 run を保証し、`(session_id, call_id)` key、timeout、pending 状態照会または切替禁止を実装する。 |
| CR-016 | PDF を `Vec<u8>` の JSON IPC で全量転送し、frontend は全ページ canvas を DPR 解像度で同時描画する。大規模 PDF で数百MBからGBの memory/IPC 負荷になる。 | `lib.rs:743-754`; `PdfPane.tsx:143-170,254-299`; `PdfViewer.tsx:223-233,296-325` | binary/custom protocol、ページ virtualization、viewport 外 canvas 破棄、memory budget を導入する。 |
| CR-018 | 仕様の「全データ JSON」は `EntryDetail[]` だけで、highlight/chat/settings/audit/fulltext/attachment path・本体を含まず restore もない。SQLite backup も明記どおり DB のみで attachment 本体を保護しない。 | `docs/SPEC.md:52-54`; `lib.rs:1758-1793`; `backup.rs:20-49`; `SettingsModal.tsx:709-780` | JSON を「entry metadata export」と明記するか、versioned archive + attachment + import/round-trip を実装する。完全 backup の範囲を UI で明示する。 |
| CR-019 | DOI/arXiv/ISBN/ORCID の canonicalization と uniqueness が不統一で、表記揺れや同時 clip により duplicate が作られる。旧形式 arXiv category 消失、末尾 `vN` の残留もある。 | `db/entries.rs:1009-1027`; `metadata.rs:340-353`; `db/authors.rs:157-185`; migrations `0001` | canonical column を migration で追加し、write 時正規化 + partial UNIQUE を DB で保証する。既存行を migrate/dedup する。 |
| CR-020 | metadata client は DOI の URL encode、共通 timeout、全経路の `error_for_status` が不足し、arXiv Atom を substring parse して XML entity を復号しない。 | `metadata.rs:14-29,191-253,340-379`; `orcid.rs:28-50,92` | bounded shared client、正規 URL builder、XML parser、malformed/status/timeout/entity test を追加する。 |
| CR-021 | 一覧・検索は無制限かつ1文献あたり複数 query、全文検索は同一 entry を page hit ごとに再取得する。大規模 library で latency が線形以上に増える。 | `db/entries.rs:201-204,409-460,687-692`; `db/fulltext.rs:234-243`; migration `0001:103-110` | pagination、batch association load、entry summary cache、reverse index を追加し benchmark を持つ。 |
| CR-022 | 複数の DB/file 操作が check-then-write または分割 transaction。tag 同時作成、chat scope、export snapshot、backup 同時実行で失敗・部分状態・混在出力が起こる。 | `db/tags.rs:20-30`; `db/chat.rs:309-321`; `bibtex.rs:259-290`; `backup.rs:28-47,105-123` | atomic upsert、単一 transaction/read snapshot、backup mutex と exclusive name、retention error reporting を導入する。 |
| CR-023 | MCP HTTP server は request を直列処理し、clip metadata 待ちが全 traffic を止める。config 保存後の起動失敗、audit best-effort、token/enable lifecycle にも非原子性がある。 | `mcp_server/mod.rs:282,552-570,634-799`; `clipper.rs:280-313`; `lib.rs:1495-1515,2262-2306` | authenticated request の bounded concurrency、duplicate precheck、config/lifecycle serialization、mutation+audit の原子化、bind 失敗 rollback を実装する。 |
| CR-024 | chat の entry scope は fulltext search にしか適用されず、`get_entry` と mutation tool は任意 ID を操作できる。 | `llm/tools/search.rs:109-165`; `llm/tools/mutate.rs:274-640` | 全 entry read/write tool の共通 scope guard を作る。retrieval-only scope が意図なら UI/仕様名を変更する。 |
| CR-025 | frontend は古い detail DTO を完全 update に再送し、非同期応答も selection 世代を確認しない。chat/MCP mutation 後は entries だけ reload され、detail/tags/collections が stale のまま。 | `src/App.tsx:286-342,521-560,690-716` | patch command + revision/optimistic lock、request sequence guard、mutation 種別付き統一 cache invalidation を導入する。 |
| CR-026 | LLM/notes Markdown が外部 image を読み、通常 link は app WebView を遷移できる。CSP は無効。raw HTML XSS は確認されていないが tracking と将来の injection 防御がない。 | `MathMarkdown.tsx:24-30`; `tauri.conf.json:22-24` | `a`/`img` custom renderer、scheme/domain policy、opener 経由、厳格 CSP、local font を導入する。 |
| CR-027 | 複数 PDF で OCR は常に最初の attachment を対象にし、reader/clipper 追加 PDF は自動全文索引されない。仕様の「添付成功後に自動索引」と不一致。 | `DetailView.tsx:150-182`; `llm/tools/ocr.rs:67-87`; `mcp_server/mod.rs:889-917`; `docs/SPEC.md:284-287` | OCR/index API を `attachment_id` 指定に統一し、全添付経路で共通 post-attach indexing job を呼ぶ。 |
| CR-028 | 実装されていない、または表示名と動作が違う UI がある。PDF search/note/pen、Header Download/More、beta update channel、Chat/Detail からの global command が該当。 | `PdfToolbar.tsx:52-56,127-176`; `Header.tsx:85-95`; `SettingsModal.tsx:515,664-672`; `CommandPalette.tsx:60-61`; `App.tsx:967-983` | 実装まで disable/非表示にし、global action は library 遷移または overlay 常時 mount、palette は backend 全文献検索にする。 |
| CR-029 | extension の localhost `fetch` に timeout がなく、hung request で `busy`/testing が永久に残る。古い badge timer が新しい結果を消す race もある。 | `extension/src/api.ts:26-57`; `extension/src/background.ts:9-22,29-76` | `AbortController` deadline、timer cancel/世代番号、pair/clip 多重実行防止を追加する。 |
| CR-030 | desktop frontend の unit/E2E/lint と通常 PR CI がない。447 Rust test と extension test は release workflow でも実行されず、今回の P0 と command 未登録を検出できなかった。 | `package.json`; `.github/workflows/release.yml:35-149`; `.github/workflows/` | PR workflow に `pnpm build`、Rust test/clippy、extension test/build、invoke-handler contract test、主要 Playwright/Tauri E2E を追加する。 |
| CR-031 | `quick-xml 0.39.3` に `RUSTSEC-2026-0194/0195`（各 CVSS 7.5）がある。現在の経路は `plist -> quick-xml` で未信頼 XML の直接入力は未確認だが、lockfile は脆弱。 | `Cargo.lock`; `cargo tree -i quick-xml@0.39.3` | `quick-xml >= 0.41.0` を含む `plist`/Tauri へ更新し、`cargo audit` を CI gate にする。 |
| CR-032 | BibTeX は organization author を literal 保護せず、citation key は structured family/reading を使わないため round-trip や日本語著者で不正確。 | `bibtex.rs:357-360,625-632,670-694` | `is_organization` を `{{...}}` で出力し、`family_name`/`reading_family` 優先の共通 key generator と round-trip test を追加する。 |
| CR-033 | OpenAI/Anthropic/ORCID の外部通信に connect/idle timeout が不足し、SSE の error/terminal marker 不在を成功扱いできる。 | `llm/openai.rs:33-60`; `llm/anthropic.rs:36-63`; `orcid.rs:28-50` | provider 共通 timeout/cancel policy、error event parse、正常 terminal marker 必須化、EOF regression test を追加する。 |
| CR-034 | Summary sheet を閉じても有料 LLM request は backend で継続し、再実行で並走する。notes 保存は await されず失敗を表示しない。 | `SummarySheet.tsx:28-105`; `App.tsx:852-855` | request ID/cancel command、単一実行 guard、await 可能な保存 callback と error state を追加する。 |
| CR-035 | 外部 MCP tool 名の `mcp_<id>_<tool>` 連結は collision と provider 命名制約違反を起こし、routing が HashMap 順序依存になる。 | `mcp/mod.rs:151-177,389-408` | reversible encoding、長さ/文字検証、collision-checked registry、deterministic routing を実装する。 |

## 6. P3 Low の指摘

| ID | 指摘 | 根拠 / 修正 |
|---|---|---|
| CR-036 | Added sort は comparator が値を設定せず常に同順。 | `src/App.tsx:375-381`; `created_at` を比較する。 |
| CR-037 | dark theme は accent を amber 固定し、PDF別windowは theme/language を同期しない。localStorage 値も型castだけ。 | `useTheme.ts:50-57`; `pdf-viewer.tsx:1-23`; enum 検証と window 間同期を追加する。 |
| CR-038 | API docs と実装に `attach_ocr_text`、updater command、`abstract`/`abstract_`、return type の drift があり、CLI help は write 対応後も read-only と表示する。相対時刻等の hard-coded UI text と HTML `lang` 固定も残る。 | `docs/API_SPEC.md:125-150,540-585`; `cli/mod.rs:55-60`; docs/type generation と i18n lint を導入する。 |
| CR-039 | Clippy は重複 attribute、同一 if branch、不要 `Ok(?)` 等 11 warnings。 | `cargo clippy --all-targets --all-features`; baseline を直して CI では warning deny を段階導入する。 |

## 7. 推奨する修正順

1. **即時 hotfix**: CR-001。検索 scope と purge の backend 防御を同時に直す。
2. **security release**: CR-017、CR-003、CR-002、CR-010、CR-012、CR-004。
3. **data integrity release**: CR-005、CR-009、CR-015、CR-008、CR-019。
4. **reliability**: CR-014、CR-011、CR-023、CR-025、CR-027、CR-030。
5. 残りの P2/P3 を subsystem ごとに処理する。

## 8. 良好だった点

- Rust は 446 test が成功し、DB migration、FTS、citation key、MCP auth/write gate に広い coverage がある。
- MCP/clipper は loopback bind、Bearer 認証、body size 上限、default-off gate を持つ。
- 公開 MCP から destructive `delete_entry` を除外し、provider API key は OS keychain に保存する。
- PDF download は 50MB 上限、timeout、HTTP status、`%PDF-` magic byte を検証する。
- Tauri updater は署名検証を有効化し、placeholder public key guard がある。
- extension 権限は `activeTab`、`scripting`、`storage`、loopback host に限定され、常駐 content script はない。
- desktop/extension の日本語・英語 translation key は一致している。
- npm production dependencies には監査時点で既知脆弱性がなかった。

## 9. 再開情報と変更状況

修正作業を再開する場合は、対象コミットを確認し、CR-001 から ID 順ではなく
「推奨する修正順」に従う。各修正で仕様更新、回帰テスト、関連する frontend/backend
両方の検証を同じ変更に含める。

レビュー開始前からの未追跡ファイル: `AGENTS.md`。

レビューで追加したファイル: `docs/CODE_REVIEW_2026-07-11.md`。

レビュー対象の実装コードは変更していない。
