# 本地建置與自訂版本

這份指南適用於 clone art-term 或建立 fork 後，想在 Windows 電腦打包 EXE／Android APK，或製作自己版本的開發者。範例預設從 repository 根目錄、使用 Windows PowerShell 7 執行；首次建置需要網路下載依賴，也需要足夠空間保存 Rust、SDK／NDK 與 Gradle 快取。

建置不需要維護者的私人設定、簽章金鑰或遠端 Host。App 初次開啟可使用空連線清單；實際使用終端前，再依[使用前準備](../openab-host-setup.md)設定自己的 Runtime。

## 1. 取得原始碼與準備工具

若要長期維護自己的修改，先在 GitHub fork，將下列 clone 網址換成自己的 repository；只是本地試用可直接使用本專案網址：

```powershell
git clone https://github.com/partypeopleland/art-term.git
Set-Location art-term
```

Windows x64 建置機需準備：

| 工具 | 用途與要求 |
|---|---|
| Git、PowerShell 7 | 取得原始碼及執行打包腳本 |
| Node.js 與 npm | 安裝並編譯 Vue 前端；目前鎖定的 Vite 要求 Node `^20.19.0` 或 `>=22.12.0`，請選符合範圍且仍受維護的版本 |
| Rust／Cargo／rustup | 使用 stable MSVC 工具鏈；Windows x64 對應 `x86_64-pc-windows-msvc` |
| Visual Studio C++ Build Tools | 安裝「使用 C++ 的桌面開發」，含 MSVC 與 Windows SDK，供 Rust／Tauri 連結 |
| WebView2 Runtime | 執行 Windows GUI；Windows 版需要 Windows 10 1809 以上 |

第三方安裝步驟見文末 Tauri 官方 prerequisites。安裝後重新開啟 PowerShell，先確認工具可執行，再安裝前端依賴：

```powershell
git --version
pwsh --version
node --version
npm.cmd --version
rustc --version
cargo --version
rustup show active-toolchain
npm.cmd --prefix apps/client ci
```

每一步成功後再繼續。Windows 打包腳本會自行執行 `npm ci`；Android 腳本只執行前端 build，因此首次建置 APK 前仍需上面的依賴安裝。之後更新原始碼、lockfile 有變動時，也要重新執行 `npm ci`。

## 2. 打包 Windows

工具準備完成後，以下命令會安裝前端依賴、編譯前端與 Rust release，並產生可攜資料夾：

```powershell
.\scripts\package-windows.ps1 -OutputPath ".\dist\windows\art-term-$(Get-Date -Format yyyyMMdd-HHmmss)"
```

輸出位置由命令最後的 `Windows package:` 顯示。包內包含 `art-term-gui.exe`、空的 `connections.toml`、README、授權檔與 `manifest.json`；交付時保留整個資料夾，放到可寫位置後執行 EXE。這個流程目前不產生安裝精靈，也不替 EXE 做 Authenticode 簽章。

`-OutputPath` 必須是新建或空目錄，避免蓋掉舊交付。省略此參數會依主 Tauri 設定的版本使用 `dist/windows/art-term-<version>`，第二次打包可能因此拒絕。平常不要加 `-SkipBuild`，它只重包既有 binary，不能保證包含最新修改。

公開交付使用預設空設定即可。私人 Windows 套件可用 `-ConfigPath` 指定自己的 TOML，但不要將含私人連線的套件當成公開版本；詳細設定與測試見 [Windows 平台](windows.md)。

## 3. 準備 Android 工具並試建 debug APK

完成第 1 節後執行：

```powershell
.\scripts\setup-android.ps1
.\scripts\build-android.ps1 -Target aarch64 -Debug
```

setup 下載獨立 JDK 17、Android SDK／NDK 及 Rust Android targets，將環境寫入 ignored `target/android-tools/environment.json`。首次安裝需由你閱讀並接受 Android SDK 授權；若安裝回報 license／下載錯誤，處理後重跑 setup，確認成功再建置。預設 SDK 在 `target/android-sdk`；不需要沿用原作者的 SDK 目錄或另外安裝 Android Studio。

`aarch64` 適用 ARM64 手機，x86_64 模擬器改用 `-Target x86_64`。Debug 流程不需要自建 release 金鑰；完成時會印出 generated Android 專案內的 APK 路徑，並不使用 release 的 `dist/android` 輸出位置。手機安裝後的觸控、鍵盤與背景操作仍需實測。

若手機已有相同 application ID、不同簽章的 App，Android 會拒絕覆蓋安裝。要讓自己的版本與原版共存，請先按第 5 節更換識別碼。不要以清除資料解決簽章不符；需要搬移連線時，先在原 App 做[加密匯出](../refarch/connection-transfer.md)。

## 4. 建立自己的 Android release 簽章

以下流程是**新 fork／新發行身分**的一次性設定。若已有自己發布的 APK，應還原原本的 key 與密碼；不要重建金鑰後期待能直接更新原 App。原作者的 key 不在 Git 裡，fork 或 clone 也不會取得它。

現行簽章腳本要求同一資料夾內有三個固定名稱：

| 檔案 | 內容 |
|---|---|
| `android-release.p12` | 以密碼保護的 PKCS#12 私鑰與憑證；alias 固定為 `art-term` |
| `android-release.dpapi` | 在建置帳號下，以 Windows DPAPI 加密的 keystore 密碼 |
| `certificate-sha256.txt` | 公開簽章憑證的 SHA-256 指紋，64 位十六進位文字 |

Alias 是 keystore 內部名稱，沿用 `art-term` 不會限制 App 的顯示名稱或 application ID。範例使用 `%LOCALAPPDATA%\example-terminal\signing`，你可以換成自己的專案名稱；之後 build 與 verify 都要傳相同 `-SigningRoot`。不傳時，腳本預設使用 `%LOCALAPPDATA%\art-term\signing`。

在已完成 Android setup 的同一 Windows 建置帳號執行下列區塊。它拒絕使用既有目錄，避免誤覆寫金鑰；請使用密碼管理器產生並另行保存強密碼。密碼以隱藏輸入取得，短暫經程序環境傳給 keytool，結束後移除環境變數，不寫入命令列或明文檔。

```powershell
$ErrorActionPreference = 'Stop'
$signingRoot = Join-Path $env:LOCALAPPDATA 'example-terminal/signing'
if (Test-Path -LiteralPath $signingRoot) {
    throw 'SigningRoot already exists; restore its original key or choose a new directory.'
}
$cfg = Get-Content ./target/android-tools/environment.json -Raw | ConvertFrom-Json
$keytool = Join-Path $cfg.javaHome 'bin/keytool.exe'
if (-not (Test-Path -LiteralPath $keytool)) { throw 'Run Android setup first.' }
$password = Read-Host 'New keystore password (save it in your password manager)' -AsSecureString
if ($password.Length -lt 6) { throw 'keytool requires at least 6 characters; use a strong password.' }
# 在建立私鑰前，先確認此 Windows 帳號能產生 DPAPI 密文。
$encryptedPassword = $password | ConvertFrom-SecureString
New-Item -ItemType Directory -Path $signingRoot | Out-Null
$keystore = Join-Path $signingRoot 'android-release.p12'
$certificate = Join-Path $signingRoot 'certificate.der'
$env:ARTTERM_NEW_KEY_PASSWORD = [PSCredential]::new('signing', $password).GetNetworkCredential().Password
try {
    & $keytool -genkeypair -storetype PKCS12 -keystore $keystore `
        -alias art-term -keyalg RSA -keysize 3072 -validity 10000 `
        -dname 'CN=Example Terminal Android Release' `
        -storepass:env ARTTERM_NEW_KEY_PASSWORD -keypass:env ARTTERM_NEW_KEY_PASSWORD
    if ($LASTEXITCODE -ne 0) { throw 'Key generation failed.' }
    & $keytool -exportcert -keystore $keystore -alias art-term `
        -storepass:env ARTTERM_NEW_KEY_PASSWORD -file $certificate
    if ($LASTEXITCODE -ne 0) { throw 'Certificate export failed.' }
    (Get-FileHash -LiteralPath $certificate -Algorithm SHA256).Hash.ToLowerInvariant() |
        Set-Content -LiteralPath (Join-Path $signingRoot 'certificate-sha256.txt') -Encoding ascii
    $encryptedPassword |
        Set-Content -LiteralPath (Join-Path $signingRoot 'android-release.dpapi') -Encoding ascii
} finally {
    Remove-Item Env:ARTTERM_NEW_KEY_PASSWORD -ErrorAction SilentlyContinue
    $password.Dispose()
}
```

`certificate.der` 是另存的公開憑證，供計算指紋使用。若中途失敗，保留目錄檢查錯誤，不要直接刪除已產生的私鑰。請另外妥善備份 `.p12`、原始密碼與憑證指紋；不要提交私鑰／密碼，也不要只備份 `.dpapi`。DPAPI 檔綁定 Windows 帳號與環境，換電腦／帳號時須以原密碼在新環境重新產生；`.p12` 與憑證身分維持不變。

還原原 `.p12` 與指紋到選定目錄後，可用下列方式重存密碼。先確保密碼來自同一 keystore 的備份；接下來簽章若失敗，應核對備份，不要重建 key：

```powershell
$signingRoot = Join-Path $env:LOCALAPPDATA 'example-terminal/signing'
$password = Read-Host 'Password of the restored keystore' -AsSecureString
try {
    $password | ConvertFrom-SecureString |
        Set-Content -LiteralPath (Join-Path $signingRoot 'android-release.dpapi') -Encoding ascii
} finally { $password.Dispose() }
```

之後每次 release，從根目錄執行：

```powershell
$signingRoot = Join-Path $env:LOCALAPPDATA 'example-terminal/signing'
$apk = ".\dist\android\example-terminal-$(Get-Date -Format yyyyMMdd-HHmmss)-aarch64.apk"
.\scripts\build-android.ps1 -Target aarch64 -SigningRoot $signingRoot -OutputApk $apk
.\scripts\verify-android.ps1 -Target aarch64 -SigningRoot $signingRoot -Apk $apk
```

Build 完成編譯、ZIP 對齊與簽章；verify 另外核對憑證、package／版本、ABI、manifest 與 16KB native alignment。兩個命令都成功才是可交付候選，之後仍需安裝到手機驗證。要讓本地與 CI 產物能互相升級，兩邊須使用同一份自己的 release key。GitHub Actions 的 secrets 與發行設定依 [Release 指南](releases.md)在自己的 fork 另外建立，不能複製 Windows DPAPI 檔就直接供 CI 使用。

## 5. 改成自己的名稱、識別碼、圖示與版本

以下以虛構的 `Example Terminal`／`com.example.terminal` 為例，實際發布時使用自己的名稱與唯一識別碼。先完成設定，再初始化 Android 專案及建置。這些位置目前沒有統一的「重新命名」指令，單改 repository 名稱或 APK 檔名不會改變安裝身分。

| 想調整的項目 | 需要同步修改的位置與效果 |
|---|---|
| App 名稱與視窗標題 | `crates/art-term-gui/tauri.conf.json` 的 `productName`、`app.windows[].title`；Android 設定 `tauri.android.conf.json` 的視窗 title 也要修改（productName 沿用主設定） |
| 畫面文字 | `apps/client/index.html` 的 title、`apps/client/src/mobile/MobileApp.vue` 的標題、`apps/client/src/main.ts` 的啟動錯誤文字；Windows 包裝 README 文字在 `scripts/package-windows.ps1` |
| Windows 識別碼 | 主 Tauri 設定的 `identifier`，例如 `com.example.terminal.desktop` |
| Android 安裝識別碼 | Android Tauri 設定的 `identifier`，例如 `com.example.terminal`；並將 `scripts/prepare-android.ps1` 內 MainActivity 的 Kotlin `package` 與輸出路徑由 `net/missart/artterm` 改為 `com/example/terminal` |
| APK 驗證期待值 | `scripts/verify-android.ps1` 的 package metadata 檢查及最後輸出物件的 `applicationId` 必須跟著自己的識別碼更新；保留其他安全檢查 |
| Android 回歸測試識別碼 | `scripts/android-regression/device.py` 的 `PACKAGE` 與 `scripts/test-android-regression.ps1` 的 package metadata 檢查也要改成自己的識別碼，讓測試啟動及驗證正確的 App |
| 版本 | 同步修改兩份 Tauri 設定的 `version`，並遞增 Android 設定的 `bundle.android.versionCode`；prepare 與 verify 已從 Android 設定讀取版本，不需再改腳本中的版本字串 |
| 輸出名稱 | 使用 `-OutputPath`／`-OutputApk`；預設目錄與 APK 檔名會讀取 Tauri 版本，但仍使用 `art-term` 前綴。Windows EXE 仍叫 `art-term-gui.exe`，不由 productName 自動更名 |

更換 application ID 會建立另一個 App，原 App 的資料不會自動出現；用連線匯出／匯入搬移即可。維持同一 application ID 發行更新時，繼續使用原 release key，並增加版本號。

Rust crate／library 名稱與 vault、voice plugin 的 Java namespace 可以先保留，它們不是 App 安裝識別碼。全面改內部名稱還會牽涉 Cargo、native library、Rust plugin 註冊及 Kotlin 路徑；不要只對整個 repo 做字串替換，尤其不要任意更動憑證格式與儲存識別字。

`crates/art-term-gui/gen/android` 是 ignored 產物。若曾以舊識別碼初始化，先保存其中自行做過的原生修改，再將該目錄搬到 repo 內 ignored `target/` 的新備份目錄，讓下一次初始化重新產生。不要只修改 generated Gradle 或 MainActivity；build 每次會執行 prepare，版本與 Activity 內容會再次寫入。

### 產生自己的圖示

準備有透明通道的正方形 PNG 或 SVG，例如自行新增的 `branding/app-icon.png`，並保留原始圖檔在自己的 fork。為了讓 Android 圖示也寫入正確專案，先完成上面的識別碼修改與 SDK setup，再在根目錄執行：

```powershell
$cfg = Get-Content ./target/android-tools/environment.json -Raw | ConvertFrom-Json
$env:JAVA_HOME = $cfg.javaHome
$env:ANDROID_HOME = $cfg.androidHome
$env:ANDROID_USER_HOME = $cfg.androidUserHome
$env:NDK_HOME = $cfg.ndkHome
$env:PATH = "$env:JAVA_HOME/bin;$env:PATH"
Push-Location crates/art-term-gui
try {
    node ../../apps/client/node_modules/@tauri-apps/cli/tauri.js android init --ci --skip-targets-install
    if ($LASTEXITCODE -ne 0) { throw 'Android init failed.' }
    node ../../apps/client/node_modules/@tauri-apps/cli/tauri.js icon ../../branding/app-icon.png
    if ($LASTEXITCODE -ne 0) { throw 'Icon generation failed.' }
} finally { Pop-Location }
```

這會產生桌面 `icons/` 與 Android 專案內的圖示，Windows 設定目前使用 `icons/icon.ico`。只製作 Windows 版時，可略過 Android 環境／init，直接在 `crates/art-term-gui` 執行上述 icon 命令。Android generated 目錄不提交 Git，因此換機、fresh clone 或重新初始化後，要再執行 icon 命令，才能在新 APK 使用自訂圖示。保留並提交自己的原始圖與桌面 icons。

## 6. 檢查與交付

只想本地試用時，先確認 Windows 包或 APK 成功產出並可開啟；修改程式後，依 [Windows 開發檢查](windows.md)及 [Android 平台驗收](android.md)執行受影響的測試。成功編譯與簽章驗證不代表已測過真實 Host、手機輸入或長時間背景。

發布前確認名稱、圖示、版本、application ID 與簽章都是自己的預期值；保留 LICENSE 與第三方說明，公開包內只使用空設定。`target/`、`dist/`、`node_modules/` 及簽章材料不應提交。App 的使用設定與匯出檔也不要夾帶進自己的 Git repository。

常見失敗可依原因處理：缺 `node_modules` 先執行 `npm ci`；Windows 輸出非空就換新 `-OutputPath`；SDK 缺套件或授權時重新完成 setup；DPAPI 無法解密時回到原建置帳號或以原密碼重新建立密碼檔；fork 後 verify 仍期待上游 package，就同步第 5 節的識別碼期待值；版本不符則核對 Tauri 設定與本次產物，不移除驗證步驟。

## 附錄：實作與參考

- 本專案腳本：[Windows package](../../scripts/package-windows.ps1)、[Android setup](../../scripts/setup-android.ps1)、[Android build](../../scripts/build-android.ps1)、[Android prepare](../../scripts/prepare-android.ps1)、[Android sign](../../scripts/sign-android.ps1)、[Android verify](../../scripts/verify-android.ps1)。
- 設定與依賴：[Windows Tauri](../../crates/art-term-gui/tauri.conf.json)、[Android Tauri](../../crates/art-term-gui/tauri.android.conf.json)、[前端 lockfile](../../apps/client/package-lock.json)。
- 官方工具文件：[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)、[Tauri icons](https://v2.tauri.app/develop/icons/)、[JDK 17 keytool](https://docs.oracle.com/en/java/javase/17/docs/specs/man/keytool.html)、[PowerShell ConvertFrom-SecureString](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.security/convertfrom-securestring)、[Android app signing](https://developer.android.com/studio/publish/app-signing)。
