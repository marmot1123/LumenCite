# LumenCite リリース手順

v0.1.0 配布対象: **macOS (Apple Silicon + Intel)** / **Windows** / **Linux (AppImage / deb / rpm)**

このドキュメントは、開発者が手作業で行う必要があるリリース準備手順をまとめたものです。
コード変更（`tauri.conf.json`, `.github/workflows/release.yml`）はリポジトリ側に同梱済みなので、
ここに書いてある **外部サービスの登録 / 鍵生成 / GitHub Secrets の登録** が完了すれば自動リリースが動きます。

> 所要時間の目安: Apple Developer Program の承認に 24〜48 時間、Windows 証明書発行に 1〜10 営業日（OV / EV）。
> **タグ付けの前に必ず先に着手すること**。

---

## 全体像

| ターゲット | 必要なもの | 必要な GitHub Secret |
|---|---|---|
| macOS | Apple Developer ID Application 証明書 + notarytool | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`, `KEYCHAIN_PASSWORD` |
| Windows | コード署名証明書（OV または EV） | `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` |
| Linux | 不要（署名は使わない） | — |
| Tauri Updater | 署名鍵ペア | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| 全 OS | リリース作成権限 | `GITHUB_TOKEN`（GitHub Actions が自動付与） |

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

## 2. Windows 側準備

### 2-1. コード署名証明書の取得

選択肢:
- **OV (Organization Validation) 証明書** — 安価（年 USD 80〜200）。最初は SmartScreen 警告が出るが、ダウンロード実績が貯まると消える
- **EV (Extended Validation) 証明書** — 高価（年 USD 300〜500）+ ハードウェアトークン必須。SmartScreen 警告が即時に出ない

主な発行元: DigiCert, Sectigo, SSL.com など。v0.1.0 では **OV 推奨**（コスト優先）。

> EV を選んだ場合はハードウェア HSM 上に秘密鍵が保管されるため GitHub Actions での自動署名は困難。
> その場合は手動で `signtool sign` を走らせるか、AzureKeyVault などのクラウド HSM を併用する。
> 以下は OV ベースの手順。

### 2-2. .pfx を Base64 化して GitHub Secrets に登録

発行元から `.pfx` ファイル + パスワードが届く。

```sh
base64 -i cert.pfx -o cert-pfx-base64.txt
```

GitHub Secrets:

| Name | 値 |
|---|---|
| `WINDOWS_CERTIFICATE` | base64 化した pfx |
| `WINDOWS_CERTIFICATE_PASSWORD` | pfx のパスワード |

ワークフロー側で `cert.pfx` に書き戻してから `tauri build` に渡す。

`tauri.conf.json` の `bundle.windows.certificateThumbprint` は使わない（pfx パス指定方式を採る）。

---

## 3. Tauri Updater 署名鍵

`tauri-plugin-updater` は、更新バイナリの真正性を ed25519 署名で検証する。**秘密鍵は厳重に管理**。

### 3-1. 鍵生成

リポジトリ直下で:

```sh
pnpm tauri signer generate -w ~/.tauri/lumencite-updater.key
```

- パスワードを設定（空も可だが GitHub Actions に渡しにくくなるので設定推奨）
- 公開鍵が `~/.tauri/lumencite-updater.key.pub` に出力される
- 公開鍵の内容を `src-tauri/tauri.conf.json` の `plugins.updater.pubkey` にコピー（リポジトリにコミット OK）

### 3-2. GitHub Secrets に登録

| Name | 値 |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | `~/.tauri/lumencite-updater.key` の中身（全文） |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 3-1 で設定したパスワード |

> **秘密鍵を失うと、過去の updater 署名と互換性のある新バイナリを発行できなくなる**。
> 1Password 等のパスワードマネージャに別途保管しておくこと。

---

## 4. updater エンドポイントの準備

GitHub Releases にバイナリをアップロードする方式を採用。

- `tauri-action` が自動的に `latest.json` を生成して Release アセットにアップロードする
- `tauri.conf.json` の `plugins.updater.endpoints` に以下を設定（既に設定済み）:

```json
"endpoints": [
  "https://github.com/motoki317/lumencite/releases/latest/download/latest.json"
]
```

> `motoki317/lumencite` の部分は実際のリポジトリ owner/name に合わせて要編集。
> プライベートリポジトリの場合は `https://github.com/.../releases/download/v{{current_version}}/latest.json` 形式 + S3 や Cloudflare R2 等のホスティングが必要。

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

1. macOS arm64 / macOS x64 / Windows x64 / Linux x64 の 4 ターゲットで `tauri build` を並列実行
2. 各バイナリを署名 + macOS は notarize
3. `latest.json` を生成して updater 用 ed25519 鍵で署名
4. GitHub Release を作成し、すべてのアセットをアップロード

エラー時はワークフロー画面のログを確認。よくあるトラブル:

| 症状 | 対処 |
|---|---|
| `errSecInternalComponent` | `KEYCHAIN_PASSWORD` 未設定 or 値が間違っている |
| `Notarization failed` | `APPLE_PASSWORD` は **通常パスワードではなく App-Specific Password** を使う |
| `signtool: timestamp server error` | `timestampUrl` を `http://timestamp.digicert.com` に変更 |
| `Failed to sign with updater key` | `TAURI_SIGNING_PRIVATE_KEY` に改行が欠落していないか確認 |

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
- [ ] Windows コード署名証明書 発注 & 受領
- [ ] `pnpm tauri signer generate` で updater 鍵ペア生成
- [ ] `tauri.conf.json` の `plugins.updater.pubkey` を実値に置換
- [ ] `tauri.conf.json` の `plugins.updater.endpoints` のリポジトリパスを実値に置換
- [ ] GitHub Secrets 12 個を登録（上記表参照）
- [ ] 試しに `v0.1.0-rc.1` タグでドライランしてワークフローを通す
- [ ] 各 OS でインストール検証

---

## 関連

- `tauri.conf.json` — bundle / updater 設定
- `.github/workflows/release.yml` — 自動リリースワークフロー
- <https://tauri.app/distribute/sign/macos/> — Tauri 公式 macOS 署名ドキュメント
- <https://tauri.app/distribute/sign/windows/> — Tauri 公式 Windows 署名ドキュメント
- <https://tauri.app/plugin/updater/> — Tauri Updater プラグイン公式
