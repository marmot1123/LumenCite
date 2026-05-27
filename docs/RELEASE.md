# LumenCite リリース手順

v0.1.0 配布対象: **macOS (Apple Silicon + Intel)** / **Windows** / **Linux (AppImage / deb / rpm)**

このドキュメントは、開発者が手作業で行う必要があるリリース準備手順をまとめたものです。
コード変更（`tauri.conf.json`, `.github/workflows/release.yml`）はリポジトリ側に同梱済みなので、
ここに書いてある **外部サービスの登録 / 鍵生成 / GitHub Secrets の登録** が完了すれば自動リリースが動きます。

> 所要時間の目安: Apple Developer Program の承認に 24〜48 時間。
> **タグ付けの前に必ず先に着手すること**。

---

## 全体像 (v0.1.0)

| ターゲット | 必要なもの | 必要な GitHub Secret |
|---|---|---|
| macOS | Apple Developer ID Application 証明書 + notarytool | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `KEYCHAIN_PASSWORD` |
| Windows | **v0.1.0 は未署名配布**（SmartScreen は「詳細情報→実行」で回避）。**v0.2.1 で Certum 個人名義の署名を導入予定**（§2） | （取得後）コード署名用の Secret を追加 |
| Linux | 不要（署名は使わない） | — |
| Tauri Updater (macOS) | **v0.2.0 で有効化（macOS のみ）**。ed25519 鍵で `latest.json` を検証 | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| Tauri Updater (Windows) | **v0.2.1 へ送り**（コード署名と同時に導入） | — |
| 全 OS | リリース作成権限 | `GITHUB_TOKEN`（GitHub Actions が自動付与） |

> v0.2.0 で必要な GitHub Secrets は **macOS 関連 7 個 + Tauri Updater 署名鍵 2 個** の計 9 個。Windows 署名鍵は v0.2.1 で導入する。

---

## 1. Apple 側準備（macOS）

### 1-1. Apple Developer Program 加入

1. <https://developer.apple.com/programs/> から加入（年 USD 99 / 法人は別途）
2. 承認まで 24〜48 時間。`Team ID`（10 文字英数字）を控える

### 1-2. Developer ID Application 証明書の発行

1. ローカル macOS で **Keychain Access > 証明書アシスタント > 認証局に証明書を要求** で `.certSigningRequest` を生成（個人 Mac の Login Keychain に秘密鍵を保存）
2. <https://developer.apple.com/account/resources/certificates> > **+** > **Developer ID Application** を選択し、CSR をアップロードして `.cer` をダウンロード
3. ダブルクリックで Login Keychain に登録 → 「証明書」カテゴリで `Developer ID Application: <Name> (<TeamID>)` が見えれば成功
4. Keychain Access で同証明書を右クリック → **書き出し...** → `.p12` 形式で保存（パスワードを設定）
5. `signingIdentity` 名（例: `Developer ID Application: Motoki Marumo (XXXXXXXXXX)`）を控える

### 1-3. App-Specific Password の発行（notarytool 用）

1. <https://appleid.apple.com/account/manage> > **App-Specific Passwords** > **Generate Password**
2. ラベル例: `lumencite-notarytool`、表示されたパスワードを控える

### 1-4. .p12 を Base64 化して GitHub Secrets に登録

```sh
base64 -i certificate.p12 -o cert-base64.txt
pbcopy < cert-base64.txt    # クリップボードへコピー
```

GitHub の Settings > Secrets and variables > Actions > **New repository secret** で以下を登録:

| Name | 値 |
|---|---|
| `APPLE_CERTIFICATE` | base64 化した p12 |
| `APPLE_CERTIFICATE_PASSWORD` | p12 のパスワード |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <Name> (<TeamID>)` |
| `APPLE_ID` | Apple ID メールアドレス |
| `APPLE_PASSWORD` | 1-3 で発行した App-Specific Password |
| `APPLE_TEAM_ID` | 10 文字 Team ID |
| `KEYCHAIN_PASSWORD` | ワークフロー内で一時 keychain を作るための任意の文字列（推奨: openssl rand -base64 24） |

---

## 2. Windows 側準備（v0.1.0 スキップ → v0.2.1 で Certum 個人名義を導入）

### 2-0. 経緯と CA 選定（2026-05-27 決定）

v0.1.0 では Windows コード署名は **未署名で配布**（SmartScreen は「詳細情報 → 実行」で回避案内）。v0.2.1 で署名を導入する。CA は調査の結果 **Certum を「個人名義」で取得**する方針に決定した。

- **DigiCert 等の OV は見送り**: 2023-06 の CA/Browser Forum 要件で秘密鍵は HSM 格納必須（`.pfx` DL 署名不可）。さらに OV は「組織」実在確認が必要で、**法人登記のない個人事業主だと検証が難航**（実際に DigiCert OV 申請が停滞）。
- **Azure Trusted Signing（現 Artifact Signing）も不可**: 安価（~$10/月・クラウド HSM）だが、公式 FAQ（2026-05 時点）で個人開発者は **米国・カナダのみ**対象。**日本の個人は対象外**。EV も発行しない。
- **Certum 個人名義が現実解**: 個人として申請でき、本人確認はパスポート + 英語の住所証明のみ。**D-U-N-S 番号不要**、**SimplySign（クラウド）でトークン輸入も不要**。費用 **€189/年**（概ね 3〜3.5 万円・為替次第）。証明書サブジェクトに本名が出る点だけ許容が必要。
  - 屋号を載せたい場合は D-U-N-S 番号（東京商工リサーチ、¥3,300、約 1 週間）が追加で必要。
  - 公開リポジトリなら Certum Open Source Code Signing（年 €30 前後）も候補（サブジェクト表記に制約）。
- SmartScreen は個人/OV 証明書では当初警告が出るが、DL 実績で評価が育つ（即時評価は EV のみ）。

参考: [Certum 必要書類](https://support.certum.eu/en/code-signing-required-documents/) ／ [blog.craftz.dog（個人名義実例）](https://blog.craftz.dog/6b64c9ec806b/) ／ [Tauri v2 Windows 署名](https://v2.tauri.app/distribute/sign/windows/) ／ [defguard: Certum HSM + Tauri CI](https://defguard.net/blog/windows-codesign-certum-hsm/)

### 2-1. 取得・導入チェックリスト（Certum 個人名義）

クリティカルパスは Phase 2 の承認待ち（実日数 数日）。Phase 3〜6 は半日〜1 日程度。

**Phase 0 — 事前準備（手元作業）**
- [ ] パスポート（有効期限内）の顔写真ページを撮影
- [ ] 英語の住所証明書（印字・ラテン文字・発行 13 ヶ月以内）を 1 つ。いずれか:
  - ゆうちょ銀行の英語版残高証明書（窓口で「英語・住所表記付き」を依頼）※**残高金額は審査に無関係**
  - 英語で出せる公共料金請求書（残高を見せたくない場合）
- [ ] 証明書に載せる氏名のローマ字を**パスポート表記と一致**させる（例 `Motoki Seki`）。この氏名・住所が配布バイナリに埋まる点を最終確認
- [ ] クレジットカード（€189 支払い用）

**Phase 1 — 購入（Certum）**
- [ ] [certum.eu / shop.certum.eu](https://shop.certum.eu/) でアカウント作成
- [ ] 「Standard Code Signing — in the Cloud（SimplySign）」1 年（€189）を選択（USB トークン版でなくクラウド版）
- [ ] **個人（individual）**として申請（corporation を選ばない＝VAT ID 不要）→ カード支払い

**Phase 2 — 本人確認・アクティベーション**
- [ ] 証明書アクティベーション開始 → 鍵長 **4096-bit** を選択
- [ ] 本人確認方法 **Automatic Identity Verification（推奨）**
- [ ] スマホでパスポートのライブ確認（顔＋パスポート）
- [ ] パスポート画像＋英語住所証明をアップロード（指示によりパスワード付き zip をメール送付の場合あり）
- [ ] 申請者情報・証明書情報（氏名＝パスポート表記）を入力 → **承認待ち**（不備があると往復）

**Phase 3 — SimplySign セットアップ**
- [ ] SimplySign モバイルアプリを導入・登録（署名時の OTP/認証用）
- [ ] SimplySign Desktop を Windows にインストール（証明書を Windows 証明書ストア / `signtool` から見えるようにする）
- [ ] 発行済み証明書がクラウドプロファイルに見えることを確認

**Phase 4 — ローカル署名テスト（Windows 実機 / VM）**
- [ ] `signtool sign /tr http://time.certum.pl/ /td sha256 /fd sha256 /a test.exe`
- [ ] `signtool verify /pa /v test.exe` が成功
- [ ] 証明書ストアの**拇印（thumbprint）**を控える（`certmgr.msc` / `Cert:\CurrentUser\My`）

**Phase 5 — LumenCite ビルドへ組み込み**
- [ ] ローカルビルド署名（最短）: `src-tauri/tauri.conf.json` の `bundle.windows` に
  ```json
  "windows": {
    "certificateThumbprint": "<Phase4の拇印>",
    "digestAlgorithm": "sha256",
    "timestampUrl": "http://time.certum.pl/"
  }
  ```
  （SimplySign Desktop で認証済みのマシンならストア経由で署名される）
- [ ] CI（GitHub Actions）で自動署名する場合は `bundle.windows.signCommand`（`%1` が対象ファイル）でカスタム署名コマンドを指定。クラウド HSM を CI で回すのは難所なので defguard の記事を参照
- [ ] **Windows updater 同時有効化**: `release.yml` の `includeUpdaterJson` を Windows ジョブでも `true`（現状 macOS のみ）→ `latest.json` に windows エントリ追加。updater 署名は**既存の ed25519 鍵を流用**（Authenticode とは別物、両方必要。§3 参照）

**Phase 6 — 配布・確認**
- [ ] 署名済み `.msi`/`.exe` を Release に添付し `signtool verify` で再確認
- [ ] SmartScreen 警告が出る場合は DL 実績で評価が育つ。必要なら [Microsoft へ file submission](https://www.microsoft.com/en-us/wdsi/filesubmission)
- [ ] 本ドキュメント §6 と README/CHANGELOG の「Windows 未署名」記述を更新

---

## 3. Tauri Updater（v0.2.0 で macOS のみ有効化）

v0.2.0 で **macOS のみ** auto-updater を有効化した。Windows updater はコード署名と同時に v0.2.1 へ送る（未署名のままでは updater が検証で弾かれるため）。

実施状況（v0.2.0）:

- ✅ 鍵生成済み: `~/.tauri/lumencite-updater.key`（**空パスワード**）。公開鍵は `tauri.conf.json` の `plugins.updater.pubkey` に設定済み、`active: true`。`bundle.createUpdaterArtifacts: true`。
- ✅ `release.yml`: `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` を tauri-action に渡し、`includeUpdaterJson` は **macOS ジョブのみ true**（`latest.json` は darwin エントリのみ → macOS だけ auto-update）。
- ⏳ **リリース担当が手作業で必要**:
  1. **秘密鍵を 1Password 等にバックアップ**（`~/.tauri/lumencite-updater.key`。紛失すると永久に updater 互換性が切れる）。
  2. GitHub Secrets を 2 つ登録:
     - `TAURI_SIGNING_PRIVATE_KEY` = `~/.tauri/lumencite-updater.key` の中身全文（`cat ~/.tauri/lumencite-updater.key`）
     - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` = 空文字（空パスワードのため）

エンドポイントは GitHub Releases の `latest.json` を参照する設定で既に入っている (`tauri.conf.json` 参照)。Windows / Linux は updater 非対象（手動 DL。Windows は v0.2.1 で署名と同時に対応）。

---

## 4. pdfium（OCR 用ネイティブライブラリ）

OCR（スキャン PDF の Vision 文字起こし）は実行時に **pdfium 動的ライブラリ**を必要とする（`pdfium-render` がロード）。

- **配布 (.dmg)**: `release.yml` の macOS ジョブが [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) の `pdfium-mac-univ.tgz` を取得し `src-tauri/pdfium/libpdfium.dylib` に置く。`tauri.release-macos.conf.json`（`--config` でマージ）の `bundle.macOS.frameworks` で `.app/Contents/Frameworks/` に同梱され、`bind_pdfium` がそこを探す。**base の `tauri.conf.json` には frameworks を入れない**（dylib 不在で `cargo build`/`tauri dev` が壊れるため）。
- **ローカル開発で OCR を試す**: `pdfium-mac-univ.tgz` を展開し `lib/libpdfium.dylib` を `src-tauri/pdfium/libpdfium.dylib` に置く（`bind_pdfium` がカレント `pdfium/` も探す）。未配置でも OCR 以外は動く。
- `src-tauri/pdfium/` は gitignore 済み（バイナリは非コミット）。Windows / Linux の pdfium 同梱は OCR を各 OS で配布する段階で別途対応。

---

## 5. リリース手順（実運用）

事前準備が整ったら、リリースは以下のフローで自動化:

```sh
# 1. バージョンを上げる
# package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json の version を一致させる

# 2. コミット & タグ付け
git add -A
git commit -m "Release v0.1.0"
git tag v0.1.0
git push origin main --tags
```

タグプッシュで `.github/workflows/release.yml` が起動し:

1. macOS universal (arm64 + x86_64) / Windows x64 / Linux x64 の 3 ターゲットで `tauri build` を並列実行
   - macOS は `macos-14` (Apple Silicon) ランナー上で `--target universal-apple-darwin` を指定し、`lipo` で 1 つの `.app` / `.dmg` に統合
   - 旧 `macos-13` (Intel) ランナーは GitHub 側の供給不足で恒常的に queue 待ちが長いため使わない
2. 各バイナリを署名 + macOS は notarize
3. `latest.json` を生成して updater 用 ed25519 鍵で署名
4. GitHub Release を作成し、すべてのアセットをアップロード

エラー時はワークフロー画面のログを確認。よくあるトラブル:

| 症状 | 対処 |
|---|---|
| `errSecInternalComponent` | `KEYCHAIN_PASSWORD` 未設定 or 値が間違っている |
| `Notarization failed` | `APPLE_PASSWORD` は **通常パスワードではなく App-Specific Password** を使う |
| `User interaction is not allowed` | keychain unlock 失敗。`KEYCHAIN_PASSWORD` の再確認 |

---

## 6. 配布後の検証

各 OS で別マシン（クリーンインストール環境推奨）から:

- **macOS**: `.dmg` をマウント → アプリをドラッグ → 初回起動で警告なく開けば成功（Gatekeeper / notarization 通過）
- **Windows**: インストーラ実行で SmartScreen が出ない（EV）または「詳細情報」から実行できる（OV）
- **Linux**: AppImage を実行 / `sudo dpkg -i lumencite_*.deb` 実行
- **Updater**: 旧バージョンを入れて起動 → アップデート通知 → 適用 → 新バージョンで再起動

---

## 7. v0.1.0 リリースに向けた現時点のチェックリスト

- [ ] Apple Developer Program 加入完了
- [ ] Developer ID Application 証明書 発行 & ローカル登録
- [ ] App-Specific Password 発行
- [ ] GitHub Secrets **7 個**を登録（`APPLE_*` × 6 + `KEYCHAIN_PASSWORD`）
- [ ] `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` の version が一致
- [ ] `CHANGELOG.md` に v0.1.0 エントリを追記
- [ ] 試しに `v0.1.0-rc.1` タグでドライランしてワークフローを通す
- [ ] 各 OS でインストール検証（macOS: Gatekeeper 通過 / Windows: SmartScreen「詳細情報→実行」/ Linux: AppImage 起動）
- [ ] ドラフトリリースの公開（GitHub UI から手動で「Publish release」）

---

## 関連

- `tauri.conf.json` — bundle / updater 設定
- `.github/workflows/release.yml` — 自動リリースワークフロー
- <https://tauri.app/distribute/sign/macos/> — Tauri 公式 macOS 署名ドキュメント
- <https://tauri.app/distribute/sign/windows/> — Tauri 公式 Windows 署名ドキュメント
- <https://tauri.app/plugin/updater/> — Tauri Updater プラグイン公式
