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
| Windows | **v0.2.1 で Certum Open Source Code Signing（クラウド HSM/SimplySign）を導入**（§2）。CI 無人署名は SimplySign の GUI ログイン必須により不可と判明 → **一時 Windows VM で手動署名**。専用の常時起動マシンは不要 | （CI Secret なし。VM の SimplySign ログインで署名） |
| Linux | 不要（署名は使わない） | — |
| Tauri Updater (macOS) | **v0.2.0 で有効化**。ed25519 鍵で `latest.json` を検証 | `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |
| Tauri Updater (Windows) | **v0.2.1 では見送り**（手動 latest.json マージが macOS updater を壊すリスク）。Windows は手動 DL 更新 | — |
| 全 OS | リリース作成権限 | `GITHUB_TOKEN`（GitHub Actions が自動付与） |

> v0.2.0 で必要な GitHub Secrets は **macOS 関連 7 個 + Tauri Updater 署名鍵 2 個** の計 9 個。v0.2.1 では **CI 用の追加 Secret は不要**（Windows 署名は VM 手動）。

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

## 2. Windows 側準備（v0.1.0 スキップ → v0.2.1 で Certum Open Source を導入）

### 2-0. 経緯と CA 選定（2026-05-27 → 2026-06-11 更新）

v0.1.0 では Windows コード署名は **未署名で配布**（SmartScreen は「詳細情報 → 実行」で回避案内）。v0.2.1 で署名を導入する。CA は調査の結果 **Certum**（ポーランドの CA）を個人で取得する方針に決定した。当初 Standard Code Signing（€189〜209）を想定していたが、**LumenCite は MIT ライセンスで GitHub に公開済みの OSS のため、より安価な Certum Open Source Code Signing（€49 前後・クラウド版）を採用**することにした（2026-06-11 決定）。

- **DigiCert 等の OV は見送り**: 2023-06 の CA/Browser Forum 要件で秘密鍵は HSM 格納必須（`.pfx` DL 署名不可）。さらに OV は「組織」実在確認が必要で、**法人登記のない個人事業主だと検証が難航**（実際に DigiCert OV 申請が停滞）。
- **Azure Trusted Signing（現 Artifact Signing）も不可**: 安価（~$10/月・クラウド HSM）だが、公式 FAQ（2026-05 時点）で個人開発者は **米国・カナダのみ**対象。**日本の個人は対象外**。EV も発行しない。
- **Certum Open Source Code Signing（クラウド / SimplySign）を採用**:
  - 本人確認は **パスポート + 英語の住所証明のみ**（Standard と同じ）。加えて **OSS プロジェクトの証明**（リポジトリ URL + ライセンス）の提出が必要 → MIT 公開済みの本リポジトリで満たせる。
  - **D-U-N-S 番号不要**、**SimplySign（クラウド）でトークン輸入も不要**。費用 **€49 前後/年**（Standard の €189〜209 より大幅に安い）。
  - ⚠️ **証明書のサブジェクト名 (CN) は `Open Source Developer` + 本名**（例 `Open Source Developer, Motoki Seki`）になる。本名は載るので個人名義方針と整合。機能面（Authenticode 検証・SmartScreen 評価育成）は Standard と同等。
  - ⚠️ 購入時は必ず **「Open Source Code Signing — in the Cloud / SimplySign」** を選ぶ。検索で出る「€69 のセット」は**物理スマートカード+リーダー版**なので選ばない。
  - ⚠️ **用途は OSS に限定**される。将来 LumenCite をクローズドソース化・商用ライセンス化する場合は Standard への切り替えが必要。
  - 署名回数制限 5,000 回/月（実質無関係）。鍵長は RSA 3072-bit 以上。有効期間は 2026-02-27 以降 最大 459 日。
- SmartScreen は Open Source/OV 証明書では当初警告が出るが、DL 実績で評価が育つ（即時評価は EV のみ）。

参考: [Certum Open Source CS in the Cloud（商品）](https://certum.store/open-source-code-signing-on-simplysign.html) ／ [Certum 必要書類](https://support.certum.eu/en/code-signing-required-documents/) ／ [piers.rocks（Open Source 証明書 実体験・CN 表記）](https://piers.rocks/2025/10/30/certum-open-source-code-sign.html) ／ [Tauri v2 Windows 署名](https://v2.tauri.app/distribute/sign/windows/) ／ [defguard: Certum HSM + Tauri CI](https://defguard.net/blog/windows-codesign-certum-hsm/)

### 2-1. 取得・導入チェックリスト（Certum Open Source・クラウド）

クリティカルパスは Phase 2 の承認待ち（実日数 数日）。Phase 3〜6 は半日〜1 日程度。

**Phase 0 — 事前準備（手元作業）✅ 2026-06 取得済み**
- [x] パスポート（有効期限内）の顔写真ページを撮影
- [x] 英語の住所証明書（印字・ラテン文字・発行 13 ヶ月以内）を 1 つ。いずれか:
  - ゆうちょ銀行の英語版残高証明書（窓口で「英語・住所表記付き」を依頼）※**残高金額は審査に無関係**
  - 英語で出せる公共料金請求書（残高を見せたくない場合）
- [ ] **OSS プロジェクトの証明**を用意: 公開リポジトリ URL（<https://github.com/marmot1123/LumenCite>）と `LICENSE`（MIT）。申請者本人が関与していることが分かる状態にしておく
- [ ] 証明書に載せる氏名のローマ字を**パスポート表記と一致**させる（例 `Motoki Seki`）。CN は `Open Source Developer, <氏名>` になる点・この氏名が配布バイナリに埋まる点を最終確認
- [ ] クレジットカード（€49 前後の支払い用）

**Phase 1 — 購入（Certum）**
- [ ] [certum.eu / shop.certum.eu](https://shop.certum.eu/) でアカウント作成
- [ ] **「Open Source Code Signing — in the Cloud（SimplySign）」**1 年（€49 前後）を選択（USB トークン/スマートカード版でなくクラウド版）
- [ ] **個人（individual）**として申請（corporation を選ばない＝VAT ID 不要）→ カード支払い

**Phase 2 — 本人確認・アクティベーション**
- [ ] 証明書アクティベーション開始 → 鍵長 **RSA 3072-bit 以上**（4096-bit 可）を選択
- [ ] 本人確認方法 **Automatic Identity Verification（推奨）**
- [ ] スマホでパスポートのライブ確認（顔＋パスポート）
- [ ] パスポート画像＋英語住所証明をアップロード（指示によりパスワード付き zip をメール送付／`ccp@certum.pl` 宛の場合あり）
- [ ] **OSS プロジェクトの URL（GitHub）とライセンスを提出**（Open Source 版固有の追加要件）
- [ ] 申請者情報・証明書情報（氏名＝パスポート表記）を入力 → **承認待ち**（不備があると往復）

**Phase 3 — アクティベーション + シークレット取得（Mac＋スマホで完結）✅ 2026-06-17 完了**
- [x] SimplySign モバイルアプリ登録 / 証明書アクティベート（RSA 3072-bit 以上）
- [x] `otpauth://` シークレット取り出し（QR をデコード。`zbarimg` 等でオフライン）
- [x] 証明書(公開部分)を入手し拇印を算出: `openssl x509 -in cert.pem -noout -fingerprint -sha1` → `B4415786DBCFEEEFF9ECDEEB4FD3193F2EB7A9C9`（PEM は `~/Dropbox/secrets/Certum/`）

### 2-2. 署名アーキテクチャ: 一時 Windows VM で手動署名（2026-06-17 決定）

**CI 自動署名は断念した。** Certum SimplySign はクラウド HSM の鍵を呼び出すのに **SimplySign Desktop（トレイ常駐 GUI）への対話ログインが必須**で、ヘッドレス/CLI ログイン手段が公式に存在しない。GitHub ホストランナーの非対話セッションでは GUI が描画されず（`rc.2` で実証：プロセスは起動するがウィンドウ列挙に出ず、SendKeys 不発・証明書がストアに現れない）、**無人 CI 署名は構造的に不可**。SSL.com eSigner CKA や Azure Trusted Signing のような CI 向け無人署名アダプタを Certum は提供していない。

→ **macOS（署名+notarize）と Linux は CI、Windows は一時 VM で手動署名**する分担にした。専用の常時起動マシンは不要で、VM はリリース時だけ起動すればよい。

**Phase 4 — Windows VM の用意（初回のみ）**
- [ ] Apple Silicon なら [UTM](https://mac.getutm.app/)（無料）＋ Windows 11 ARM、または Parallels 等
- [ ] VM に SimplySign Desktop（[files.certum.eu](https://files.certum.eu/software/SimplySignDesktop/Windows/) の 64-bit `.msi`）、Rust + Node + pnpm + Tauri ビルド前提一式、Git をインストール
- [ ] SimplySign モバイルアプリは手元のスマホで OK（VM 側には不要。ログイン時に OTP を入力）

**Phase 4-2 — VM の起動と停止（2026-08-12 のコスト削減以降・毎回必要）**

実運用では Azure の Windows 11 VM（`lumencite-win` / リソースグループ `lumencite-siging` / japaneast）を使っている。**`az vm deallocate` してもディスクと公開 IP の課金は止まらない**（マネージドディスクは電源状態と無関係に「確保した容量」へ、Standard SKU の公開 IPv4 はどこにも紐付いていなくても課金される）。実測では停止中の定常費 ¥4,268/月 のうち 96% がこの 2 つだった。そこで 2026-08-12 に **公開 IP を削除**し、OS ディスクを `Premium_LRS`(P10) → `StandardSSD_LRS`(E10) に落として ¥1,553/月 にした。

このため **署名のたびに公開 IP を作り直して NIC に付ける必要がある**。サブネットは `defaultOutboundAccess` 未設定で NAT Gateway も無いので、公開 IP が無いと RDP が繋がらないだけでなく**送信方向のインターネット接続も無い**（SimplySign はクラウド署名なので致命的）。

```sh
RG=lumencite-siging; NIC=lumencite-win563_z1

# 起動前 — 公開 IP を作って NIC に付ける（元と同じ Standard / Static / zone 1）
az network public-ip create -g $RG -n lumencite-win-ip \
  --sku Standard --allocation-method Static --version IPv4 --zone 1
az network nic ip-config update -g $RG --nic-name $NIC -n ipconfig1 \
  --public-ip-address lumencite-win-ip
az vm start -g $RG -n lumencite-win
az vm list -d -o table    # 新しい IP を確認（毎回変わる）→ FreeRDP の接続先に使う

# 署名が終わったら — 必ず 3 つとも戻す
az vm deallocate -g $RG -n lumencite-win
az network nic ip-config update -g $RG --nic-name $NIC -n ipconfig1 --remove publicIPAddress
az network public-ip delete -g $RG -n lumencite-win-ip
```

- **RDP がつながらないときの第一容疑者は NSG**。`lumencite-win-nsg` の受信規則 `RDP`（priority 300 / TCP 3389）は送信元を **VM 作成時の自宅グローバル IP に固定**してある。ISP 側で変わっていると弾かれるので、`curl -s ifconfig.me` で現在の IP を確認し `az network nsg rule update -g $RG --nsg-name lumencite-win-nsg -n RDP --source-address-prefixes <現在の IP>` で更新する（**現在値はリポジトリに書かない**。`az network nsg rule show` で引く）。
- OS ディスクの SKU 変更は **deallocated 中しかできない**。Premium に戻したくなったら停止中に `az disk update --sku` する。
- 自動シャットダウン（DevTestLab スケジュール・21:00 JST）は有効のまま。ただしこれは**戻し忘れの保険**であって、停止してもディスクと IP の課金は止まらない。
- 月 ¥2,000 の予算アラート `lumencite-monthly-2000jpy` を設定済み（実績 80%/100% と予測 100% でメール通知）。**通知するだけで課金は止まらない。**

**Phase 5 — リリースごとの Windows 署名手順（VM 上）**
0. [ ] **VM を起動する。公開 IP を作り直してから起動すること**（Phase 4-2。忘れると RDP も外向き通信も繋がらない）
1. [ ] CI（タグ push）が **macOS + Linux のドラフトリリース**を生成するのを待つ（§5 参照）
2. [ ] VM で対象タグを `git checkout`（ローカルに前回ビルドの差分が残っていれば `git stash` か `git checkout -- <file>` で退避してから）。**updater 秘密鍵 (`TAURI_SIGNING_PRIVATE_KEY`) は不要** — Windows オーバーレイ `tauri.release-windows.conf.json` が `createUpdaterArtifacts: false` を設定しており updater 成果物（`.sig`）を生成しないため。署名はコード署名証明書（Certum）だけで完結する
3. [ ] **SimplySign Desktop を起動しログイン**（ユーザーID + スマホ OTP）。証明書がストアに載る（PIN キャッシュ 3h・セッション 2h）
4. [ ] **pdfium.dll を `src-tauri\pdfium\pdfium.dll` へ配置する** ── **§4 の PowerShell ブロックをそのまま使う**。
   Windows は CI が pdfium を取得しないので、ここが唯一の配置点。**v1.0.0 が Windows にとっての初同梱**
   （v0.10.0 の `tauri.release-windows.conf.json` には `resources` が無かった）なので、前回 VM 作業の踏襲は効かない。
   ⚠ 展開は必ず `tmp-pdfium\` へ隔離する（ルートで展開すると追跡ファイルの `LICENSE` を壊し、
   それがインストーラのライセンス頁に載る）。配置後に `git status --short` が clean であることを見る。
   置き忘れた場合は**ビルドが `ResourcePathNotFound` で落ちてインストーラが 1 つも出来ない**
   （`bundle.resources` の非 glob エントリは `tauri-build` が実在を要求する）ので、
   ここは機械が止めてくれる。**黙って壊れるのは次の手順で `--config` を落としたときだけ**
   ── 署名設定と `resources` が同時に消え、**未署名で pdfium も入っていない**インストーラが出来上がる
5. [ ] 署名込みでビルド:
   ```pwsh
   pnpm install --frozen-lockfile
   pnpm tauri build --config src-tauri/tauri.release-windows.conf.json
   ```
   `tauri.release-windows.conf.json` の `certificateThumbprint`（拇印）でバンドル時に `signtool` 署名された `.msi` / `*-setup.exe` が生成される。同オーバーレイは `createUpdaterArtifacts: false` なので **updater 成果物（`.sig`）は生成されない**（Windows auto-updater 見送りのため。Phase 6 参照）
6. [ ] `signtool verify /pa /v <生成された .exe/.msi>` で署名を確認
7. [ ] **インストーラを展開して `pdfium.dll` が入っていることを確認**（`--config` を落とした場合は
   ビルドが通ってしまうので、**その取りこぼしが目に見えるのはここが最初で最後**）
8. [ ] 生成された **署名済み `.msi`/`.exe`** をドラフトリリースへアップロード（`gh release upload <tag> <files> --clobber`）。
   ⚠ **バージョンを明示して拾う**（`bundle\{msi,nsis}\` に前回リリースの成果物が残っていると、
   アルファベット順で古い方を掴む。§9-5 参照）

> **⚠️ 署名トラブルシューティング（v0.8.0 で遭遇・次回も再発しうる）**
> - **`failed to bundle project: failed to run …\x64\signtool.exe` は「exe が無い/壊れた」とは限らない**。tauri は signtool の **non-zero 終了も stderr を握りつぶして**この文言に丸める。切り分けは (1) `signtool.exe /?` が usage を出す＝起動は健全、を確認し、(2) **tauri と同じ sign コマンドを手打ち**して実エラーを見る:
>   ```pwsh
>   & "C:\Program Files (x86)\Windows Kits\10\bin\<ver>\x64\signtool.exe" sign /v /fd sha256 /sha1 B4415786DBCFEEEFF9ECDEEB4FD3193F2EB7A9C9 /tr http://time.certum.pl/ /td sha256 <exe>
>   ```
>   実体はたいてい `SignTool Error: No certificates were found…` ＝ **SimplySign Desktop 未ログインで証明書がストア未載**。ログイン後 `Get-ChildItem Cert:\CurrentUser\My | ? Thumbprint -eq 'B4415786DBCFEEEFF9ECDEEB4FD3193F2EB7A9C9'` が 1 件出れば OK。
> - **「signtool のアップデート」表示は実は SimplySign Desktop の更新**（signtool 単体は自己更新しない）。この更新が署名基盤を壊すことがある。SimplySign Desktop が起動しないときは **VM 再起動 → 残プロセス kill → exe 直叩きでエラー確認 → 再インストール** の順。
> - signtool は **x64（64bit）が正**（エミュ x64 ツールチェインに一致・tauri が叩くパスも `\x64\`。32bit 版は不要）。

**Phase 6 — Windows auto-updater（v0.2.1 では見送り）**
- v0.2.1 は **署名済みインストーラの配布まで**とし、**Windows auto-updater は見送る**。理由: updater を有効化するには VM 生成物の windows エントリ（url + `.sig` の中身 + version + pub_date）を CI 生成の `latest.json` に手動マージする必要があり、誤ると**稼働中の macOS updater を壊すリスク**がある。Windows は当面「このページから手動 DL で更新」。
- 将来 Windows updater を入れる場合は、CI 向け無人署名できる CA（例: Azure Trusted Signing / SSL.com eSigner）への移行か、セルフホスト Windows runner の常設とセットで検討する。

**Phase 7 — 配布・確認**
- [ ] 署名済み `.msi`/`.exe` を別マシンで `signtool verify /pa /v` 再確認
- [ ] SmartScreen 警告は DL 実績で評価が育つ。必要なら [Microsoft へ file submission](https://www.microsoft.com/en-us/wdsi/filesubmission)
- [ ] README の「Windows 未署名」記述を更新（CHANGELOG は対応済み）

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

> ⚠️ **既知の事故（v0.1.0）と再発防止:** v0.1.0 は `plugins.updater.pubkey` が**プレースホルダ `REPLACE_WITH_TAURI_SIGNER_PUBKEY` のままタグ付け**されて出荷された（実鍵は commit `a74fd86` で投入＝v0.2.0 が初出）。`active:false` は Tauri v2 では無視され、v0.1.0 も「アップデートを確認」UI を同梱しているため、ユーザーが押すと updater がプレースホルダを base64 デコードして `Invalid symbol 95, offset 7.`（byte 95=`_`・index 7＝`REPLACE_` の `_`）で失敗する。バイナリに鍵がコンパイル済みのため**遠隔修正は不可**＝v0.1.0 ユーザーは手動 DL で乗り換えるしかない。**再発防止として `release.yml` に「pubkey がプレースホルダなら build を fail させる」ガードを追加済み**（タグ build の最初に検証）。

---

## 4. pdfium（OCR / LCIR 用ネイティブライブラリ）

**OCR**（スキャン PDF の Vision 文字起こし）と **LCIR 抽出**（`lumencite-pdfium`）は、どちらも実行時に
**pdfium 動的ライブラリ**を必要とする（`pdfium-render` がロード）。両者は
`ingestion::pdf::pdfium::bind_pdfium()` という単一の入口を共有するため、**pdfium が同梱されていない OS では
LCIR も動かない**。`lcir.enabled` を既定 ON にする前提として 3 OS すべてに同梱する。

取得元は [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries)。
**サプライチェーン対策（CR-004）としてタグを固定し、展開前に SHA-256 を検証する**（`latest` は使わない）。

### 同梱先と探索パス

| OS | 取得アセット | 置き場所 | 同梱の仕組み | 実行時の探索先 |
|----|--------------|----------|--------------|----------------|
| macOS | `pdfium-mac-univ.tgz` | `src-tauri/pdfium/libpdfium.dylib` | `tauri.release-macos.conf.json` の `bundle.macOS.frameworks` | `.app/Contents/Frameworks/`（`<exe>/../Frameworks`） |
| Linux | `pdfium-linux-x64.tgz` | `src-tauri/pdfium/libpdfium.so` | `tauri.release-linux.conf.json` の `bundle.resources` | `<exe>/../lib/<name>`（deb/rpm は `/usr/lib/<name>/`、AppImage は `$APPDIR/usr/lib/<name>/`）。`<name>` は配布形態により productName / crate 名 / バイナリ名のいずれかになるため **3 つとも探索する** |
| Windows | `pdfium-win-x64.tgz`（`bin/pdfium.dll`） | `src-tauri/pdfium/pdfium.dll` | `tauri.release-windows.conf.json` の `bundle.resources` | exe と同じディレクトリ |

探索順は `bind_pdfium()` のドキュメントコメントを参照（単体テストで固定してある）。
Tauri のリソース配置規則そのものは `tauri-utils` の `resource_dir_from` に一致させている。

- **macOS と Linux は `release.yml` が CI で自動取得する**（`Download and verify pdfium (macOS)` /
  `(Linux)` ステップ）。**base の `tauri.conf.json` には同梱設定を入れない**
  （ライブラリ不在で `cargo build` / `tauri dev` が壊れるため。オーバーレイは `--config` でのみマージする）。
- **Windows は CI 非対象**（Certum SimplySign の対話ログイン要件・§2 参照）なので、VM 上で手動配置する:

  ⚠ **展開先を必ず作業ディレクトリに隔離する。** この tarball は**トップレベルに `LICENSE` を含む**
  （bblanchon/pdfium-binaries の MIT。pdfium 本体の BSD は `licenses/` の下）ので、
  リポジトリルートで素の `tar -xzf` を打つと**追跡ファイルの `LICENSE` が上書きされる**。
  `tauri.conf.json` は `"licenseFile": "../LICENSE"` を設定しており、この LICENSE は
  **WiX / NSIS のライセンス頁に埋め込まれる** ── つまり LumenCite 自身の MIT 表示が消えた
  インストーラが出来上がる。CI 側（`release.yml`）は同じ tarball を `tmp-pdfium/` に隔離してから
  展開しているので、この事故は Windows の手動手順にだけ存在する。

  ```powershell
  # VM 上、リポジトリルートで（タグとハッシュの正本は .github/pdfium.env）
  $tag = "chromium/7934"      # = PDFIUM_TAG
  $sha = "c2c05e752ef41a1af21ad24f7f09e75e6e24c3d2cf84bbc88f11efa42edd341c"   # = PDFIUM_SHA256_WINDOWS
  Invoke-WebRequest -Uri "https://github.com/bblanchon/pdfium-binaries/releases/download/$tag/pdfium-win-x64.tgz" -OutFile pdfium.tgz
  # 展開の**前に**照合して止める（CI は `sha256sum -c -` で同じことをしている）
  $got = ((certutil -hashfile pdfium.tgz SHA256)[1] -replace '\s','')
  if ($got -ne $sha) { throw "pdfium SHA256 mismatch: $got" }
  New-Item -ItemType Directory -Force tmp-pdfium, src-tauri\pdfium | Out-Null
  tar -xzf pdfium.tgz -C tmp-pdfium        # ← -C を落とすと LICENSE を壊す
  Copy-Item tmp-pdfium\bin\pdfium.dll src-tauri\pdfium\pdfium.dll -Force
  Remove-Item -Recurse -Force tmp-pdfium, pdfium.tgz
  git status --short                        # ← LICENSE が modified なら復元してからビルドする
  ```

  そのうえで `pnpm tauri build --config src-tauri/tauri.release-windows.conf.json` を実行する。
  **`--config` を忘れると署名設定も pdfium 同梱も両方落ちる**ので、生成物の中に `pdfium.dll` があることを
  インストーラ展開後に確認すること。
  （`curl` は PowerShell 5.1 では `Invoke-WebRequest` のエイリアスで `-fL -o` を解釈しないため、
  上の例では `Invoke-WebRequest` を直接使っている。`src-tauri/pdfium/` と `tmp-pdfium/` のうち
  gitignore 済みなのは前者だけなので、後片付けまでを 1 ブロックにしてある。）

- **Linux は同梱を CI が自動で検査する**（v1.0.0-p0）。同梱が落ちてもアプリは起動でき、
  ユーザーが PDF を開いた瞬間に初めて「pdfium library not found」になるため、目視では捕まらない。
  - `release.yml` の `Verify pdfium is bundled (Linux)` が **毎リリース** `.deb` / `.rpm` / `.AppImage` を
    展開し、`libpdfium.so` が `bind_pdfium()` の探索候補にあることを確かめる
    （`scripts/verify_linux_bundle.sh`）。⚠ tauri-action はバンドルとアップロードが 1 ステップなので、
    この検査はアップロード**後**に走る。落ちてもドラフトには成果物が残るので、**公開せずに破棄すること**。
  - `linux-bundle-verify.yml` は**実際に .deb をインストールし .AppImage を展開して、その中で
    `bind_pdfium()` を呼ぶ**（`scripts/verify_linux_bundle_runtime.sh`）。手動実行と、
    探索候補・同梱設定・pdfium 版に触る PR で走る。**タグを打つ前に確かめたいときはこちらを手動実行する。**
- **ローカル開発で試す**: 各 OS 用アセットを展開し、上表の「置き場所」へ置く（`bind_pdfium` は
  カレントの `pdfium/` も探すので `src-tauri/` で `pnpm tauri dev` すれば拾う）。未配置でも OCR / LCIR 以外は動く。
- `src-tauri/pdfium/` は gitignore 済み（バイナリは非コミット）。
- **pdfium の版と検証ハッシュの正本は `.github/pdfium.env` の 1 ファイル**（3 OS 分をまとめて持つ）。
  `release.yml` と `linux-bundle-verify.yml` は両方これを `source` するので、**更新はこのファイルだけ**。
  値は `curl -fsSL <url> | shasum -a 256` で求める（GitHub API の asset digest とも一致する）。
  上の PowerShell 例に出てくる期待値も同じファイルの `PDFIUM_SHA256_WINDOWS` と揃えること。
  ⚠ 以前は同じ値がワークフロー 2 ファイルに複製されており、この手順が片方しか挙げていなかったため、
  **pdfium を上げると検証ワークフローだけ古い版を掴んだまま緑になる**穴があった。複製を戻さないこと。

---

## 5. リリース手順（実運用）

事前準備が整ったら、リリースは以下のフローで進める。**版 bump も main 直コミットにしない**
（このプロジェクトは docs 1 行でも PR 経由。過去ログに直コミットが混ざっているが根拠にしない）:

```sh
# 1. 版を上げるブランチを切る
git switch -c release/v1.0.0

# 2. 版を 4 か所で一致させる（Cargo.lock は lumencite パッケージの version 行だけ手で直す。
#    cargo を走らせると無関係な依存まで churn する）
#      package.json / src-tauri/Cargo.toml / src-tauri/tauri.conf.json / src-tauri/Cargo.lock
#    あわせて CHANGELOG.md の [Unreleased] を [X.Y.Z] - YYYY-MM-DD へ切り出し、
#    末尾の compare リンクも張り直す（§12-3）

# 3. add は対象ファイルを明示する（`git add -A` は使わない。レビュー用エージェントの残骸を
#    巻き込んだ前例がある）。**doc も同じ PR に載せる**ので、実際の一覧は毎回変わる
#    ── `git status --short` を目で見て、意図したファイルだけを並べること
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock \
        CHANGELOG.md docs/... README.md
git commit -m "chore(release): bump version to 1.0.0"

# 4. PR を出し、**CI が緑になるのを待ってから**マージする
git push -u origin release/v1.0.0     # ← 先に push する（upstream が無いと gh が対話プロンプトに落ちる）
gh pr create --fill
gh pr checks --watch                  # ci.yml と linux-bundle-verify の**両方**が緑になるまで待つ
gh pr merge --squash --delete-branch
#  ⚠ `&&` で繋がない。このリポジトリは **auto-merge 無効（`allow_auto_merge:false`）で
#    必須チェックも未設定**なので、`gh pr merge` は CI の起動前に即座に通ってしまう。
#    同じ理由で `--auto` も使えない（`enablePullRequestAutoMerge` が拒否される）。
#    `linux-bundle-verify` は 7〜12 分かかるので、待たずにマージすると §12-3 の
#    「`linux-bundle-verify` が緑」を原理的に満たせない。

# 5. マージ後の main にタグを打つ
git switch main && git pull --ff-only
git tag v1.0.0
git push origin v1.0.0      # ← タグだけを push する（`--tags` はローカルの全タグを送る）
```

タグプッシュで `.github/workflows/release.yml` が起動し:

1. **macOS universal (arm64 + x86_64) と Linux x64 の 2 ターゲット**で `tauri build` を並列実行
   - macOS は `macos-14` (Apple Silicon) ランナー上で `--target universal-apple-darwin` を指定し、`lipo` で 1 つの `.app` / `.dmg` に統合
   - 旧 `macos-13` (Intel) ランナーは GitHub 側の供給不足で恒常的に queue 待ちが長いため使わない
   - **Windows は CI 非対象**（Certum SimplySign の対話ログイン要件・§2-2）。署名済みインストーラは
     リリース担当が VM で作ってドラフトへ手動添付する
2. **macOS だけ**署名 + notarize（Linux は無署名で配布 ── OS ごとの分担は冒頭の「全体像」表）
3. `latest.json` を生成して updater 用 ed25519 鍵で署名。**`includeUpdaterJson` は macOS ジョブのみ true**
   なので `latest.json` は darwin エントリだけを持つ（＝ auto-update が届くのは macOS だけ）
4. **ドラフト**の GitHub Release を作成し、そのジョブのアセットをアップロードする
   （`releaseDraft: true` / `prerelease: false`。公開は §7 の**（タグ後）**項目）
5. **Linux ジョブが `Verify pdfium is bundled (Linux)` で成果物を展開し、`libpdfium.so` が
   実行時の探索先に入っていることを確かめる**（v1.0.0-p0）

⚠ **5 が赤いドラフトは公開しない。** tauri-action はバンドルとアップロードが 1 ステップなので、
この検査はアップロード**後**にしか置けず、落ちても成果物はドラフトに残る。失敗すると
ワークフローがドラフトのタイトルを `DO NOT PUBLISH — Linux pdfium verify failed …` に書き換えるので、
**その印が付いたドラフトは破棄して原因を直してからタグを打ち直す**。

⚠ **この印は当てにしすぎない。** 条件は「Linux ジョブが**何かで**落ちたら」なので、
pdfium とは無関係な失敗（apt・ビルド・アップロード）でも同じ文言が付き、**原因を誤って名指しする**。
逆に、ドラフトが作られる前に落ちた場合は `gh release edit` 自体が失敗し、
ログに `::warning::` が 1 行出るだけで印は付かない。**タグ後は必ず run を開いて、
どのジョブがどのステップで落ちたかを目で見ること。**

⚠ この検査は「同梱されているか」までで、**実行時に `bind_pdfium()` が実際に掴むか**は見ていない。
そちらは `linux-bundle-verify` ワークフロー（.deb を入れて中でプローブを走らせる）の担当で、
タグでは自動起動しない。**探索候補・同梱設定・productName・pdfium の版に触ったリリースでは、
タグを打つ前に手動実行しておくこと。**
ただし `linux-bundle-verify` が作るのは `--bundles deb,appimage` の 2 つだけなので、
**`.rpm` に対する検査はタグ時の `release.yml` が初回**になる（rpm だけで落ちたらドラフトを破棄して打ち直す）。

⚠ **`release.yml` の `workflow_dispatch` をタグ前のドライランに使わない。** `tagName` が
`github.ref_name` なので、`main` から起動すると **`main` という名前のドラフトリリース**を作りにいく。
ドライランするならタグを打ってから `--ref <tag>` で起動する（＝実質ドライランにならない）。
rc タグでのドライランについては §7 を参照。

エラー時はワークフロー画面のログを確認。よくあるトラブル:

| 症状 | 対処 |
|---|---|
| `errSecInternalComponent` | `KEYCHAIN_PASSWORD` 未設定 or 値が間違っている |
| `Notarization failed` | `APPLE_PASSWORD` は **通常パスワードではなく App-Specific Password** を使う |
| `User interaction is not allowed` | keychain unlock 失敗。`KEYCHAIN_PASSWORD` の再確認 |
| notarize `HTTP status code: 403. A required agreement is missing or has expired` | **Apple Developer の規約承諾切れ**（コード/Secrets は無関係）。Account Holder で <https://developer.apple.com/account> にログインし更新版 License Agreement を承諾 → `gh run rerun <run-id> --failed` で macOS ジョブのみ再実行（新タグ不要）。v0.4.0 で実際に発生。 |

---

## 6. 配布後の検証

各 OS で別マシン（クリーンインストール環境推奨）から:

- **macOS**: `.dmg` をマウント → アプリをドラッグ → 初回起動で警告なく開けば成功（Gatekeeper / notarization 通過）
- **Windows**: インストーラ実行で SmartScreen が出ない（EV）または「詳細情報」から実行できる（OV）
- **Linux**: AppImage を実行 / `sudo dpkg -i lumencite_*.deb` 実行
- **Updater**: 旧バージョンを入れて起動 → アップデート通知 → 適用 → 新バージョンで再起動
- **pdfium（Windows / Linux・v1.0.0 以降）**: 実インストールで **PDF を 1 本「新しく添付して」**
  全文索引が付くことを見る。⚠ **既にある PDF を開くだけでは検査にならない** ── ビューアは webview で
  pdfium を通らず、索引と LCIR の入口は「添付」と起動時バックフィルだから、pdfium が無くても
  既存の索引がそのまま表示されて緑に見える。上の 3 つはアプリが起動するかまでしか見ていない
  （`bind_pdfium()` が LCIR と Vision OCR の単一入口）。ログに `pdfium library not found` が出ないこと。
  Linux は CI が同梱を検査するが、**Windows は自動検査が無い**（§2-2 Phase 5 の手順 7 が最後の歯止め。
  ただし DLL の置き忘れ自体はビルドが落ちるので、素通りするのは `--config` を落とした場合だけ）
- **既定 ON のバックフィル（v1.0.0 以降）**: **旧版を入れて updater で上げた直後の初回起動**で
  `LCIR backfill:` のログが出て、放っておくと LCIR を持つ論文が増えることを見る。
  リリースビルドでは起動 60 秒後に始まる。これが「既定 ON」経路の本番で、**開発機では再現できない**（§12-2）

---

## 7. 毎回のリリースチェックリスト（タグ前 / タグ後・初出は v0.1.0）

**版固有の追加項目は各版の節に置く**（v1.0.0 は §12-3。**§12-3 は全項目がタグ前**）。ここは版に依らないもの。

- [x] Apple Developer Program 加入完了（初回のみ）
- [x] Developer ID Application 証明書 発行 & ローカル登録（初回のみ）
- [x] App-Specific Password 発行（初回のみ）
- [x] GitHub Secrets: **CI が使うのは 9 個**（`APPLE_*` × 6 + `KEYCHAIN_PASSWORD` + `TAURI_SIGNING_PRIVATE_KEY` +
      `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`）。加えて publish 時の tap 自動更新に **`HOMEBREW_TAP_TOKEN`**（§11-1）。
      `CERTUM_*` の 4 個は VM 手動署名の控えで **CI は使わない**
- [ ] `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.lock` の version が
      4 つとも一致し、**打とうとしているタグ名とも一致**している（機械的な検査は無い。ここが唯一の歯止め）
- [ ] **`tauri.conf.json` の `plugins.updater.pubkey` が本物の鍵**（`REPLACE_…` プレースホルダでない）— v0.1.0 はこれを取りこぼして updater が壊れた。`release.yml` のガードでも検証されるが、タグ前に目視確認
- [ ] `CHANGELOG.md` の `[Unreleased]` を `[X.Y.Z] - YYYY-MM-DD` へ切り出し、末尾の compare リンクも張り直した
- [ ] ~~試しに `vX.Y.Z-rc.N` タグでドライランしてワークフローを通す~~ **v0.2.1 を最後に実施していない**
      （以後 v0.3.0〜v0.10.0 の 8 リリースは本番タグ直行）。用途は毎回「署名まわりを潰す」ことで、
      v0.1.0〜v0.2.0 は Apple 署名（rc.1〜rc.4）、v0.2.1 は Windows の CI 無人署名が不可だと実証した rc.2。
      パイプラインが安定した今は、CI / 署名 / 同梱設定に触ったときだけ検討する。
      **やる場合は `release.yml` が `prerelease: false` 固定なので rc タグでもプレリリース扱いにならない**
      ── 絶対に publish せず、ドラフトと rc タグを事後に消すこと（使用済み rc タグは 1 本も残っていない）
- [ ] pdfium の版・探索候補・同梱設定に触ったリリースなら、**タグ前に `linux-bundle-verify` を手動実行**（§4）
- [ ] **（タグ後）Linux ジョブの `Verify pdfium is bundled (Linux)` が緑**（赤ければドラフトを破棄。タイトルに
      `DO NOT PUBLISH` の印が付く。⚠ このステップはタグ push でしか発火せず、`release.yml` の
      `workflow_dispatch` を代用に使ってはいけない ── §5 の ⚠ 参照）
- [ ] **（タグ後）**各 OS でインストール検証（macOS: Gatekeeper 通過 / Windows: SmartScreen「詳細情報→実行」/ Linux: AppImage 起動）
- [ ] **（タグ後）**ドラフトリリースの公開（GitHub UI から手動で「Publish release」）

---

## 8. v0.3.0 リリースの固有事項（multilingual authors）

v0.3.0 は authors テーブルを多言語名・読み仮名・国際識別子・団体著者対応に拡張する（migration 0009 + `author_identifiers` テーブル）。リリース時に注意すべき運用差分:

### 8-1. 初回起動時の FTS ワンショット再構築

- v0.3.0 では `entries_fts.authors_text` に `name_original`（漢字 / ハングル / キリル）と読み仮名（`reading_family` / `reading_given`）を合成するよう変更した。**既存ライブラリの FTS は古い合成のままなので、アップグレード後の初回起動で全 entry の FTS を一度だけ再構築する**（`src-tauri/src/lib.rs` の setup で `rebuild_authors_fts_once` を呼ぶ。`settings` テーブルの `fts.authors_v030_rebuilt` フラグで冪等化 — 2 回目以降は no-op）。
- 大規模ライブラリでは初回起動が数秒ブロックしうる。クラッシュではないので進捗が気になる場合のみ将来スプラッシュ表示を検討（v0.3.0 では未実装）。

### 8-2. アップグレード検証（必須）

クリーンインストールに加え、**v0.2.x の既存ライブラリを引き継いだ状態**で必ず確認する:

- [ ] v0.2.1 で作成した DB を残したまま v0.3.0 を起動 → migration 0009 が通り、起動時に FTS 再構築ログ（`entries_fts: rebuilt for v0.3.0 authors schema`）が一度だけ出る
- [ ] 日本語著者を含む entry を `関` / `せき` / `Seki` のいずれで検索してもヒットする
- [ ] AuthorEditor で著者フィールド編集・identifier 追加・同名著者マージが動く
- [ ] ORCID 「Fetch from ORCID」で given/family/identifier が埋まる
- [ ] 2 回目の起動では FTS 再構築が走らない（フラグ機能の確認）

### 8-3. auto-updater の配信順

- v0.2.1 は既に Latest として公開済み。v0.3.0 タグを publish すると `latest.json` が v0.3.0 を指すようになり、macOS の v0.2.x ユーザーへ自動配信される（通常の進行で問題なし）。
- updater 公開鍵 `98449F75…` は v0.1.0 以降の全リリースで不変。**鍵を変えると旧版ユーザーが署名検証に失敗して更新できなくなる**ため、`tauri.conf.json` の `pubkey` は絶対に変更しない。

---

## 9. v0.4.0 リリースの固有事項（entry types 拡張 + MCP サーバー） ✅ 2026-06-29 公開完了

v0.4.0 は文献種別を 6→19（Zotero 準拠）に拡張し、LumenCite 自身を **MCP サーバー**として公開する（Phase 1 read / Phase 2 gated write + 監査ログ / Phase 3 Claude Desktop stdio shim + バルク write）。

### 9-1. アップグレード差分（低リスク）

- 新規 migration は **0010_mcp_audit_log** のみ（`mcp_audit_log` テーブルを追加するだけの加算的変更）。**FTS 再構築は不要**（v0.3.0 の `authors_v030_rebuilt` のような one-shot 処理は無い）。
- entry types 拡張は **マイグレーション不要**（既存 `entry_type` 列の許容値を増やすだけ・既存 BibTeX キー据置）。
- 依存追加: `reqwest` に `blocking` feature（Phase 3 shim 用）。`tiny_http`（MCP HTTP サーバー）。CI ビルドへの追加設定は不要。

### 9-2. アップグレード検証

- [ ] v0.3.x の既存 DB を残したまま v0.4.0 を起動 → migration 0010 が通り、既存データは無傷
- [ ] 新しい文献種別（例 `thesis` / `dataset`）で entry を作成・表示できる
- [ ] 設定 → MCP サーバーを有効化 → Claude Code / Claude Desktop スニペットが生成される
- [ ] （任意）実 Claude Desktop から read/write を end-to-end 確認（**macOS のみ検証済み**。Windows の stdio 継承は未検証）

### 9-3. MCP サーバーのセキュリティ留意

- サーバーは **localhost バインド + インストール毎の Bearer トークン**（キーチェーン保管）。write は既定 off の一括ゲート。`delete_entry` は常に非公開。
- Claude Desktop スニペットの `command` は **アプリの絶対パスを埋め込む**。配布物は **/Applications 設置前提**（移動・translocation でパスが無効化する旨は設定 UI で警告表示済み）。

### 9-4. auto-updater の配信順

- v0.3.0 が Latest として公開済み。v0.4.0 タグを publish すると `latest.json` が v0.4.0 を指し、macOS の v0.3.x ユーザーへ自動配信される。
- updater 公開鍵 `98449F75…` は不変。`tauri.conf.json` の `pubkey` は**絶対に変更しない**（[[§8-3]] 参照）。

### 9-5. Windows VM 署名で v0.4.0 に踏んだ実運用メモ（次回も再発しうる）

§2-2 Phase 5 の手順に加えて、今回ハマった点と回避策:

> **⚠️ 2026-08-12 以降、接続先 IP は毎回変わる。** コスト削減で公開 IP を削除したため、署名のたびに作り直す（§2-2 Phase 4-2）。下記の FreeRDP 手順に入る前に IP の作成と NIC への付け替えを済ませ、`az vm list -d -o table` で出た新しい IP を接続先にすること。

- **VM への接続は FreeRDP を使う（JIS キーボードのため）**。macOS の「Windows App」(旧 Microsoft Remote Desktop) は JIS の**キーボード種別(101/106)を送れず**、VM 上で Shift+2 が `"` でなく `@` になる（i8042prt レジストリも `IgnoreRemoteKeyboardLayout` も RDP では効かない）。`brew install freerdp` の **`sdl-freerdp`**（`xfreerdp` は X11 依存で macOS では `$DISPLAY` エラー）で `/kbd:layout:0x00000411 /kbd:type:7 /kbd:subtype:2` を明示送出すると JIS が通る（**Shift+2=`"` で成功判定**）。JIS の `\`/`|`/`_`（¥キー・右Shift左の「ろ」キー）は FreeRDP が転送しないことがあるが、**コマンドはクリップボード貼り付け**（FreeRDP 既定で有効）と PowerShell の `/`(スラッシュ)パスで回避できる。
- **アップロードで古いバージョンの成果物を拾わない**。VM の `src-tauri/target/release/bundle/{msi,nsis}/` に**前回リリースの古い `.msi`/`.exe` が残る**ことがあり、`Get-ChildItem *.msi | Select -First 1` はアルファベット順で**古い方**を拾う（v0.4.0 で 0.2.1 を誤アップロード→`gh release delete-asset` で削除して再アップロードした）。**アップロード前に bundle を clean するか、`*<version>*` でバージョンを明示**して拾うこと。
- **signtool を PATH に通す**。Windows SDK の `C:\Program Files (x86)\Windows Kits\10\bin\<ver>\x64`（v0.4.0 では `10.0.26100.0`）を `[Environment]::SetEnvironmentVariable('Path', "$old;$dir", 'User')` で永続追加（`setx` は 1024 文字で PATH を破壊しうるので使わない）。
- 接続ワンライナー（VM パスワード平文を含むため**リポジトリには置かない**）はローカルの `~/bin/lumencite-vm-rdp.sh`（chmod 700）に保管。**このスクリプトは接続先 IP を固定で持っているので、2026-08-12 以降は毎回そこを書き換える**（旧 IP `20.210.105.233` は解放済みで、もう存在しない）。

## 10. v0.5.0 リリースの固有事項（Web クリッパー） ✅ 2026-07-03 公開完了

Web クリッパー（Chrome 拡張 + `/clipper` ローカル API）は main へマージ済み（PR #20、arXiv 実機 E2E 確認済み）。リリース前の残作業:

### 10-1. 残作業（機能）

- [x] **更新通知**（v0.5.0 同梱）: `check_latest_github_release` コマンド追加。GitHub API で全 OS「新版あり」判定 → 更新タブで updater `check()` と並行実行し、アプリ内更新不可なら「Releases を開く」通知バナー表示（DL/インストールはせず外部ブラウザで開くだけ・`latest.json`/署名鍵不要）。`memory/project_update_delivery.md` の計画どおり
- [x] **Codex（OpenAI CLI）向け MCP スニペット**（同梱）: `get_mcp_server_config_snippet` に `"codex"` arm 追加（`~/.codex/config.toml` の `[mcp_servers.lumencite]` TOML・既存 `--mcp-stdio` shim 流用・Windows パスの `\` を TOML エスケープ）＋設定 UI にスニペット表示。Codex 実機で end-to-end 疎通確認済み（read/write）

### 10-2. 拡張の配布（新規手順）

- [ ] `pnpm --filter lumencite-clipper build` → **`extension/dist` の中身**（`manifest.json` を **zip ルート**に）を `lumencite-clipper-<manifest version>.zip` に固めて **GitHub Releases に添付**する（v1 は load-unpacked 配布。README「Browser extension」節にインストール手順あり）。⚠️ **この zip は CI が生成しない**（リリースのアセット一覧に出ない）ので、**拡張を変えたリリースでは publish 前に手動添付を必ず確認**（v0.8.0 であわや漏れ→検証で捕捉）。⚠ **拡張を変えていないリリースでも、前回と同じ zip を毎回添付する**
  ── リリース頁 1 枚で拡張まで配れる状態を保つため（v0.6.0 / v0.7.0 は添付しておらず、その 2 版だけ
  リリース頁から拡張を入手できない。v0.8.0 以降は未変更のまま毎回添付している）。§12-3 の項目と同じ運用
- [ ] 拡張のバージョン（`extension/manifest.json` / `extension/package.json`、現在 **0.2.0**）はアプリと独立採番。API 互換を壊す変更をしたときだけ上げる（v0.5.0=0.1.0 → v0.8.0 取得整備で 0.2.0）
- Chrome Web Store 公開は DL 実績を見て別途判断（審査用に権限は最小: `activeTab`/`scripting`/`storage` + `http://127.0.0.1/*` のみ）

### 10-3. アップグレード差分（低リスク）

- **migration 追加なし**。新設定キーは `clipper.enabled`（既定 off）のみ。既存ユーザーは何もしなければ挙動不変（サーバー起動条件が「MCP **or** クリッパー」になっただけ）
- リポジトリは pnpm workspace 化（`pnpm-workspace.yaml` + `extension/`）。**アプリのビルド手順は不変**（`pnpm tauri build`）。CI に拡張のビルド/テストを足すなら `pnpm --filter lumencite-clipper build/test`

### 10-4. 検証チェックリスト

- [ ] クリッパー無効（既定）のままアプリを起動 → HTTP サーバーは MCP 設定に従来どおり従う（回帰なし）
- [ ] 有効化 → 接続コード → 拡張ペアリング → arXiv クリップ（preprint 種別＋著者＋PDF 添付）→ 再クリップで duplicate
- [ ] トークン再生成 → 拡張が 401 UX（再ペアリング誘導）になる
- [ ] 既存 MCP クライアント（Claude Code）の疎通が従来どおり

### 10-5. 既知の制約（v1 として許容・記録）

- arXiv API（export.arxiv.org）はレート制限・遅延が頻発 → 10 秒でフォールバック（拡張が送る著者＋preprint 種別で品質を維持。`API_SPEC.md`「メタデータ解決の規則」参照）
- トークンは MCP サーバーと共有。再生成でペアリングが切れる（専用トークン分離は将来）
- Chrome の Private Network Access 政策変化で localhost fetch に `targetAddressSpace: "local"` が必要になる可能性
- 重複ヒット時は PDF を添付しない（既存エントリに PDF が無くても）

---

## 11. Homebrew tap（macOS・自前 tap で配布）

macOS ユーザーは `brew install --cask` でも導入できる。公式 homebrew-cask は notability 要件（自己申請は ★225 / fork 90 / watch 90 未満は却下）を満たせないため、**自前 tap** で配布する。

- **tap リポジトリ**: [`marmot1123/homebrew-lumencite`](https://github.com/marmot1123/homebrew-lumencite)（別リポジトリ・public）。cask は `Casks/lumencite.rb`。
- **インストール**（利用者側）:
  ```sh
  brew tap marmot1123/lumencite
  brew trust marmot1123/lumencite   # Homebrew 6.0+ はサードパーティ tap に trust が必須
  brew install --cask lumencite
  ```
- cask は universal `.dmg`（`LumenCite_<version>_universal.dmg`）を GitHub Releases から取得。`auto_updates true`（アプリ内 Tauri updater と併用）/ `depends_on macos: :big_sur`。

### 11-1. リリースごとの cask 更新（自動）

`.github/workflows/update-homebrew-tap.yml` が **リリース publish 時（`release: published`）に発火**し、universal `.dmg` の sha256 を計算して tap の cask の `version` / `sha256` を書き換え push する。draft → publish の運用なので publish 時点で `.dmg` は必ず存在する。プレリリースでは動かない。

- **前提 Secret（初回のみ・必須）**: LumenCite リポジトリの Settings → Secrets and variables → Actions に **`HOMEBREW_TAP_TOKEN`** を登録する。値は tap リポジトリ `marmot1123/homebrew-lumencite` に `contents: write` できる **PAT**（fine-grained PAT を homebrew-lumencite だけにスコープ、`Contents: Read and write` 推奨。classic なら `repo` スコープ）。デフォルトの `GITHUB_TOKEN` は別リポジトリへ push できないため必須。未登録だと "Checkout tap" ステップが 403 で失敗する。
- **手動再実行**: 取りこぼした時は Actions から `Update Homebrew tap` を `workflow_dispatch`（`tag=vX.Y.Z`）で再実行できる。
- **フォールバック（手動）**: cask の `version` と `sha256`（`shasum -a 256 <dmg>`）を書き換えて push するだけ。`brew bump-cask-pr` でも可。

### 11-2. 動作確認

- **publish 直後に Actions の `Update Homebrew tap` の run を開いて色を見る。** `HOMEBREW_TAP_TOKEN` は
  PAT なので**期限切れが repo からは見えず**、失効していると "Checkout tap" が 403 で落ちる。
  cask は `auto_updates true` なので、**brew 利用者からは「更新が来ない」ことすら見えない**
- publish 後、tap リポジトリに `lumencite X.Y.Z` コミットが入っているか
- `brew update && brew info --cask lumencite` が新バージョンを表示するか
- 必要なら `brew style Casks/lumencite.rb` / `brew audit --cask --online marmot1123/lumencite/lumencite`

---

## 12. v1.0.0 リリースの固有事項（LCIR 完成） ✅ 2026-08-17 公開完了

看板は **LCIR の完成**（Phase 9a/10 到達 + `lcir.enabled` の既定 ON）。
スコープと実装順序の正本は `docs/LCIR_REMAINING_PHASES.md`（§2 順序 / §9 リリース作業）。

### 12-1. アップグレード差分（**この版だけ「高」**）

| 項目 | 内容 |
|---|---|
| **`lcir.enabled` が既定 ON** | 判定は「`"0"` でなければ ON」。**明示的に切った人だけが OFF のまま残り、未設定（＝新規と、一度も触っていない既存）は全部 ON になる**。したがって v0.10.0 まで LCIR に触らずに使ってきたユーザーでも、更新後は起動時バックフィルが動きはじめる |
| **起動時バックフィル** | リリースビルドでは起動 60 秒後に始まり、以後 **10 分ごとに「走ってよいか」を叩いて実際に走るのは 1 時間に 1 回**（`POLL_INTERVAL` と `AUTO_INTERVAL_SECS` は別物 ── 混同すると負荷を 6 倍に見積もる）。1 ラン 5 分の時間予算・添付境界で譲る・他バッチとバックアップ中は stand down（かつての「別インスタンス起動中は走らない」ゲートは、②c C-01 で第2インスタンスが起動自体を拒否するようになったため削除された）。**「updater で上がった直後の初回起動」がこの経路の本番**なので、配布後検証（§6）でここを必ず通す |
| **migration** | **0 件**。v1.0.0 は 1 本も足していない（p1 の出どころ記録は settings キーに置いた）。したがって配布版 v0.10.0 との相互起動でスキーマは壊れない |
| **pdfium が Windows / Linux で初同梱** | 両 OS にとって v1.0.0 は pdfium 初同梱版で、**手動 DL しない限り LCIR も OCR も動かない**（`bind_pdfium()` が両者の単一入口）。Windows の DLL は CI が同梱しないので **§2-2 Phase 5 の手順 4（実コマンドは §4）**で手動配置し、**`--config src-tauri/tauri.release-windows.conf.json` を落とさない**（落とすと署名設定と pdfium 同梱が同時に消える）。配置は `tmp-pdfium\` へ隔離して展開すること（ルート展開は追跡ファイルの `LICENSE` を壊す・§4） |
| **抽出器版 0.6.0 → 0.14.0** | ⚠ **起点は 0.6.0**（v0.10.0 が出荷したのはこの値。0.7.0 は v0.10.0 タグの**後**の PR #67 で入ったので、0.7.0 で作られた LCIR を持つ利用者は存在しない）。**PDF 由来の**既存 LCIR は**全件が「旧版」になる**（TeX 由来は次行）。自動では作り直さない（版 bump は再構築を誘発しない設計）ので、設定 → データの「旧版の LCIR を現行版へ再構築」を押すまで新しい図は出ない。⚠ **押す前に件数は出ない**（対象数を返すコマンドが無く、`lcir_storage_stats` が返すのは superseded 版の数＝別物）。件数が見えるのは実行中の進捗と完了メッセージだけ。**押した場合のコスト**（実測: 138 本で約 20 分 + 図の説明が有効なら約 30 分・≈$0.80）と**押さない場合に失うもの**（図領域 **1,198 → 1,629 ＝ +431**・caption ペア 662）を**リリースノートに両方書く**。⚠ **図の増分に 2 つの数字があるので、どちらの意味で使うかを決めてから書く** ── **431** は「**v0.10.0 が出荷した 0.6.0 の LCIR（1,198）との総数差**」＝押さない人が失う総量。**427** は「8d-2 / 8d-8 + クリップ修正**だけ**の寄与」（1,202 → 1,629）で、残る +4 は debt-14 のクランプ修正ぶん。CHANGELOG は**クランプ修正を別項目に分けている**ので項目ごとに 427 と +4 を使い分けており、それで無矛盾。**1 つの数字で「押さないと失うもの」を言うときは 431** |
| **TeX 由来の LCIR は据え置き** | `TEX_EXTRACTOR_VERSION` は 0.5.0 のまま。「既存の LCIR は全件が旧版」は **PDF 由来のものだけ**の話で、arXiv の TeX ソースから作った表現は現行のままで再構築の対象にならない（この版の**抽出器側の**改善は PDF 側だけ ── p1〜p4 や排他まわりは両方に効く） |

### 12-2. アップグレード検証

- **既定 ON の経路は開発機では再現できない。** 実 DB の settings には `lcir.enabled="1"` が明示的に
  書かれているので、p3 が変える判定（未設定 → ON）を通らない。**クリーンな app data dir での初回起動**が要る
  （`--config` で identifier を差し替える手が使える。ただし Keychain は共有なので課金操作に注意）。
  ⚠ **この手で見えるのは「未設定 → ON」という設定既定値の判定まで。** dev ビルドは
  **起動時バックフィルが既定で無効**（`LUMENCITE_LCIR_BACKFILL=1` を付けないと
  `LCIR backfill: disabled in dev build` を出して降りる）・**起動時バックアップも既定で無効**
  （`LUMENCITE_STARTUP_BACKUP=1`）なので、そこまで見たければ環境変数を付ける。
  バックフィル本体の本番検証は次の bullet（リリースビルド + updater）が担当する。
- v0.10.0 を入れて起動 → v1.0.0 へ updater で更新 → 初回起動でバックフィルが走ること・
  既存の LCIR が旧版として再構築の対象になること（⚠ **押す前に件数は出ないので、押して進捗の
  分母を見るか DB の `extractor_version` を数えて確かめる**）・全文検索が壊れていないことを見る（§8-2 の前例）。

### 12-3. タグ前チェックリスト（v1.0.0 固有・§7 に足すもの）

- [ ] **ゲート ②c（Codex レビュー）を通した**（`LCIR_REMAINING_PHASES.md` §2.24。②a/②b と**ベンダを跨ぐ** 3 段目）
- [ ] `CHANGELOG.md` の `## [Unreleased]` を `## [1.0.0] - YYYY-MM-DD` へ切り出した
- [ ] **`CHANGELOG.md` 末尾の compare リンクを張り直した** — `[Unreleased]: compare/v1.0.0...HEAD` と
      `[1.0.0]: compare/v0.10.0...v1.0.0` を足す。⚠ **比較元を引き写さないこと**（v0.2.0 で止まっていた定義を
      v1.0.0 の PR-4 で全版ぶん張り直した。同じ間違いは切り出しのたびに再発しうる）
- [ ] 版が **4 ファイル**で一致（`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` /
      `src-tauri/Cargo.lock`。`Cargo.lock` は `lumencite` パッケージの version 行だけ手で直す）
- [ ] **`linux-bundle-verify` が緑**（探索候補・同梱設定・検証資材が v0.10.0 から変わっているので §5 の条文に該当する）。
      **版 bump の PR は `src-tauri/tauri.conf.json` を触るので、この検査は PR で自動的に走る** ──
      main 直コミットで済ませるとトリガが消えるので、その場合は `gh workflow run linux-bundle-verify.yml --ref main` を手動実行する
- [ ] **クリーンな app data dir で「未設定 → ON」の判定を 1 回見た**（§12-2 の 1 つ目。この版だけ「高」と
      呼んでいる変更なのに、ここまでチェック項目が無かった）
- [ ] Windows VM のリードタイムを日程に載せた（起動 → SimplySign 対話ログイン → pdfium 配置 → ビルド →
      署名 → 手動アップロードで半日規模。**公開 IP は毎回作り直す** — §2-2 Phase 4-2）
- [ ] `latest.json` が darwin エントリのみであること、したがって Windows / Linux には
      **更新通知**（`check_latest_github_release`・全 OS で出る）でしか届かないことをリリースノートに書いた
      （**pdfium 初同梱と併せて**「手動 DL しないと LCIR も OCR も動かない」まで書く）
- [ ] Chrome 拡張 0.2.0 を据え置くか決めた（**据え置きでも前回 zip を手動添付する**。CI は zip を作らない・§10-2）
- [ ] `[1.0.0]` の冒頭に**版の要約段落**を書いた（直近 3 版が守っている形式。看板 / migration 0 件 / 拡張据え置き）
- [ ] **タグを打つ直前に CHANGELOG の日付を見直した** ── 過去 11 版は**公開日（JST）**を書いている。
      Windows 署名で半日ずれるので、bump PR を書いた日のまま日付をまたぎやすい

---

## 関連

- `tauri.conf.json` — bundle / updater 設定
- `.github/workflows/release.yml` — 自動リリースワークフロー
- `.github/workflows/update-homebrew-tap.yml` — publish 時に Homebrew tap の cask を自動更新
- <https://tauri.app/distribute/sign/macos/> — Tauri 公式 macOS 署名ドキュメント
- <https://tauri.app/distribute/sign/windows/> — Tauri 公式 Windows 署名ドキュメント
- <https://tauri.app/plugin/updater/> — Tauri Updater プラグイン公式
