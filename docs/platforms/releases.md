# GitHub 自動建置與 Release

Git push 只會上傳原始碼；GitHub 不會自行執行本機打包腳本。此專案的 `Build and release` workflow 在推送 `v*` 版本標籤後，用 GitHub 的 Windows runner 建置 EXE 和已簽章的 ARM64 APK，再建立 GitHub Release。Release 本文取自該 tag 中的 `docs/report/<version>.md`，發布前必須完成對應更新說明。成果不需要加入 Git。

```text
push v0.1.32 --+--> Windows EXE + ZIP -----------+
              +--> Android APK + verification --+--> GitHub Release + SHA256SUMS
```

一般推送 `main` 不發布版本。在 Actions → Build and release → Run workflow 選擇 `main`，可先建置並從該次執行的 Artifacts 下載；選擇分支試跑不建立 Release。第一次正式發布成功後，可從 repository 的 Releases 下載；若首頁右欄仍未顯示，於首頁 About 的齒輪設定勾選 Releases。

同一 App 主版本的 Android／Windows 發布須維持既有受支援 Host 的基本通訊；新增能力使用向後相容的宣告。破壞相容性的更新須提升主版本並提供遷移說明，詳見[Runtime 契約](../refarch/connections.md#app-與既有-host-的版本相容)。發布檢查須涵蓋既有 Runtime 與缺少選用能力的情境，不以更新 Host 作為每次 App 更新的前置步驟。

## 首次設定 Android 簽章

在 repository → Settings → Secrets and variables → Actions → New repository secret，設定以下三個 repository secrets。現有腳本使用 PKCS12、alias `art-term`，store 與 key 使用同一密碼。

| Secret | 內容 |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | 自己的 `android-release.p12` 的 Base64 內容 |
| `ANDROID_KEYSTORE_PASSWORD` | 該 keystore 的密碼 |
| `ANDROID_CERTIFICATE_SHA256` | 已獨立核對的簽章 SHA256，64 個十六進位字元，不含冒號 |

每個維護者可建立自己的金鑰，不需要取得上游專案的私鑰。建立一次後備份，之後的版本持續使用同一把；每次打包換新金鑰，會導致使用者無法直接覆蓋更新。不同簽章的 APK 不能覆蓋更新原 App；若要改用新簽章，先在舊 App 匯出必要連線，移除舊 App 後安裝新版並匯入，移除會清除 App 本機資料。要讓 fork 與上游同時安裝，則需另外調整 application ID 及相關原生程式／打包設定。

此上游專案於 2026-10-12 建立新的固定 release 金鑰，已設定 repository Secrets，不沿用先前本機 APK 的簽章。新公開憑證的 SHA256 為：

```text
ea9d73da8793b21fd4d4ae6050c0d934c4fec8073adc39d3e8e5e8cd3e4c311e
```

這是公開指紋，可以版控；私鑰與密碼不在 repository。Fork 應填入自己的憑證指紋，workflow 沒有鎖定上方的上游指紋。

### Fork 建立新的金鑰

安裝 JDK 後，可在 Windows PowerShell 建立 PKCS12 金鑰；命令會互動詢問新密碼與憑證資料。金鑰存於 repository 外，不要覆寫已有金鑰：

```powershell
$signingRoot = Join-Path $env:LOCALAPPDATA 'art-term/signing'
New-Item -ItemType Directory -Force $signingRoot | Out-Null
$keystore = Join-Path $signingRoot 'android-release.p12'
if (Test-Path $keystore) { throw 'Keystore already exists; reuse and back it up instead.' }
keytool -genkeypair -keystore $keystore -storetype PKCS12 -alias art-term -keyalg RSA -keysize 3072 -validity 10000
if ($LASTEXITCODE -ne 0) { throw 'Keystore generation failed' }
keytool -list -v -keystore $keystore -storetype PKCS12 -alias art-term
```

將列出的 SHA256 去掉冒號後填入 `ANDROID_CERTIFICATE_SHA256`，將剛設定的密碼填入 `ANDROID_KEYSTORE_PASSWORD`。Base64 可直接傳給自己 fork 的 Secret：

```powershell
$repo = 'YOUR-ACCOUNT/art-term' # 改成自己的 fork
[Convert]::ToBase64String([IO.File]::ReadAllBytes($keystore)) |
    gh secret set ANDROID_KEYSTORE_BASE64 --repo $repo
if ($LASTEXITCODE -ne 0) { throw 'Keystore secret upload failed' }
```

### 沿用既有 Windows 簽章材料

本機腳本的原檔預設在 `%LOCALAPPDATA%/art-term/signing`。既有的 `android-release.dpapi` 只能由原 Windows 使用者解密，不能直接搬給 GitHub runner。Workflow 會在 runner 以 Secret 密碼重新建立暫用 DPAPI 檔，完成後移除簽章材料。不要將金鑰、密碼或 Base64 貼到原始碼、Issues 或 Actions 日誌。

如已在家中原 Windows 帳號登入 GitHub CLI，可在 PowerShell 用下列命令直接傳入 Secrets，不把內容印出來：

```powershell
$signingRoot = Join-Path $env:LOCALAPPDATA 'art-term/signing'
$repo = 'partypeopleland/art-term'
[Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $signingRoot 'android-release.p12'))) |
    gh secret set ANDROID_KEYSTORE_BASE64 --repo $repo
if ($LASTEXITCODE -ne 0) { throw 'Keystore secret upload failed' }
$secure = Get-Content (Join-Path $signingRoot 'android-release.dpapi') | ConvertTo-SecureString
$credential = [PSCredential]::new('signing', $secure)
$credential.GetNetworkCredential().Password | gh secret set ANDROID_KEYSTORE_PASSWORD --repo $repo
if ($LASTEXITCODE -ne 0) { throw 'Password secret upload failed' }
([IO.File]::ReadAllText((Join-Path $signingRoot 'certificate-sha256.txt'))).Trim() |
    gh secret set ANDROID_CERTIFICATE_SHA256 --repo $repo
if ($LASTEXITCODE -ne 0) { throw 'Certificate secret upload failed' }
$credential = $null
$secure = $null
```

Repository 必須允許 GitHub Actions 執行。Workflow 僅在發布 job 要求 `contents: write`，使用 GitHub 自動提供的 `GITHUB_TOKEN`，不需要另外建立發布用 PAT。Android 工具準備步驟會接受 SDK licenses，並沿用專案既有 SDK／NDK 與打包腳本。

## 發布版本

先將 workflow、腳本與設定變更 commit 並 push 至 `main`。目前 App 版本為 `0.1.32`，發布可執行：

```sh
git tag -a v0.1.32 -m "Release 0.1.32"
git push origin v0.1.32
```

標籤必須指向包含 workflow 的 commit。後續發布先同步修改 `crates/art-term-gui/tauri.conf.json` 與 `crates/art-term-gui/tauri.android.conf.json` 的 `version`，並增加 Android 設定的 `bundle.android.versionCode`（目前為 `1063`）。Commit、push 後，再推送對應的新標籤，例如 `v0.1.32`。Workflow 會拒絕與 App 版本不一致的標籤。

成功後 Release 附件包含：

- `art-term-<version>-windows-x64.zip`：建議 Windows 使用者下載，含 EXE、空設定、操作說明與授權檔。
- `art-term-<version>-windows-x64.exe`：同一份獨立執行檔，需要 WebView2 Runtime；完整交付請選 ZIP。EXE 尚無 Authenticode 簽章。
- `art-term-<version>-android-arm64.apk`：使用 repository Secrets 指定的固定金鑰簽署、可安裝的 ARM64 Android 版。
- `SHA256SUMS.txt`：上述檔案的 SHA256。

0.1.23 修正首次乾淨建置時，Gradle 將 `tauri` 誤當成 Node 檔案的問題；準備腳本會明確指定 Node 與 Tauri CLI 的路徑，並要求 Tauri 按 ABI 分開封裝，讓 APK 產物路徑與取檔、驗證流程一致。Android 另以空資源清單覆寫桌面設定，避免將 Windows 的連線範本嵌入 APK。

## 失敗與驗證範圍

到 Actions 點選該次執行，查看紅色步驟的日誌。缺少 Android Secrets 時，Windows job 仍會獨立建置，成功後可從該次執行的 Artifacts 取得成果，但不會發布缺少 APK 的 Release。補好 Secrets 後可對原執行選擇 Re-run all jobs。

已存在的同名 Release 不會被覆寫；要交付新的程式碼，增加版本並建立新標籤。若流程在上傳附件時遇到網路錯誤，先檢查 Releases 是否已有部分成果，再決定修復該版或發布新版。

Workflow 執行正式編譯、APK 簽章／版本／ABI／16KB alignment 與既有封裝檢查，不代表通過 GUI、模擬器或手機實測。首次上線仍須確認 GitHub runner 的實際建置結果，功能驗收見[測試矩陣](../refarch/test-matrix.md)。

## 附錄：實作與參考

- [Release workflow](../../.github/workflows/release.yml)
- [Windows 建置](windows.md)、[Android 建置與簽章](android.md)
- [GitHub Release CLI](https://cli.github.com/manual/gh_release_create)
- [Tauri Android 簽章](https://v2.tauri.app/distribute/sign/android/)
