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
| Windows | **v0.1.0 では未署名配布**（SmartScreen 警告が出るが「詳細情報→実行」で回避可能） | — |
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

## 2. Windows 側準備（v0.1.0 ではスキップ）

v0.1.0 では Windows のコード署名は **行わない**。理由:

- OV / EV 証明書はコストと運用負荷（特に EV はハードウェアトークン必須）が大きい
- 初回ダウンロードでは OV であっても SmartScreen 警告が出るため、CTA を「詳細情報 → 実行」で案内すれば未署名と体感的に大差ない
- DL 実績が貯まってから判断するほうが投資対効果が読める

未署名の `.msi` インストーラがそのまま GitHub Releases に上がる。SmartScreen 警告のユーザー向け回避手順は README / リリースノートに記載済み。

将来 OV / EV を導入する場合は、本ドキュメントの旧版（git log 参照）の手順 + `.github/workflows/release.yml` の Windows セクションを復活させる。

---

## 3. Tauri Updater（v0.2.0 で macOS のみ有効化）

v0.2.0 で **macOS のみ** auto-updater を有効化した。Windows updater はコード署名と同時に v0.2.1 へ送る（未署名のままでは updater が検証で弾かれるため）。

有効化手順（v0.2.0 で実施済み・記録用）:

1. リポジトリ直下で `pnpm tauri signer generate -w ~/.tauri/lumencite-updater.key`
2. 出力された公開鍵を `tauri.conf.json` の `plugins.updater.pubkey` にコピーし、`active: true` に変更（旧 `"REPLACE_WITH_TAURI_SIGNER_PUBKEY"` を置換）
3. GitHub Secrets に `TAURI_SIGNING_PRIVATE_KEY`（秘密鍵全文）と `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` を登録
4. `.github/workflows/release.yml` の tauri-action ステップに上記 2 つの env を渡し、`with: includeUpdaterJson: true` に変更。**macOS ジョブのみ updater asset を生成**し、Windows ジョブは未署名 `.msi` のみ出力（`latest.json` の windows セクションは省略 or 空）
5. **秘密鍵は 1Password 等で別途保管**（紛失すると永久に updater 互換性が切れる）

エンドポイントは GitHub Releases の `latest.json` を参照する設定で既に入っている (`tauri.conf.json` 参照)。

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
