# Android 客戶端

Android 版讓手機操作與 Windows 相同的 Runtime Host 與 Shell，沿用 Rust 的原生 HTTPS／WSS 核心；平台層處理觸控、輸入法、背景生命週期與本機憑證保護。開發先在 Windows 確認共用功能，再用同一分支完成 APK 與手機操作驗收。

```text
Shared Vue UI + Rust core
          |
          v
Android APK -- HTTPS / WSS --> Runtime --> Shell / Desktop
    |
    +--> operator Keystore --> encrypted credential --> connections.toml
    |
    +--> touch / IME / foreground-background lifecycle
```

連線設定只保存在本機 `connections.toml`，管理憑證由 operator Keystore（此 App 在 Android 使用的金鑰儲存）保護。APK 不包含私人連線；新手機可在 App 新增或匯入。觸控與背景處理會影響使用者能否持續操作，所以桌面測試與 APK 建置成功都不能取代手機驗收。

目前 package 為 `net.missart.artterm`，minSdk 24／targetSdk 36，release 為 ARM64、16KB page alignment；版本 0.1.23／code 1023。更新必須使用原簽章，才能維持既有 App 與 Keystore 的連續性。

## 操作與背景

連線頁提供全部／Host／Agent 清單切換與管理入口。「新增 openab-pty 連線」在獨立表單保存名稱、分類、HTTPS 根網址及管理憑證；各裝置獨立保存。點已存Host先重用本機live分頁，否則只有一個ready Shell直接連，其餘狀態進選擇介面；右側「Shell」永遠開管理清單。建立／接管都需明確操作，結果不確定先重新整理。分類與能力見 [設定](../refarch/connections.md)。

窄畫面有觸控方向鍵、Ctrl latch與中文IME處理。單指上下滑動、放手慣性及細捲動軌道用於終端區；互動程式依手勢開始位置接收有界滑鼠事件，未回報歷史範圍時不捏造絕對進度。點HTTP(S)網址複製至本機剪貼簿，滑動仍可捲動。

前景使用 `FLAG_KEEP_SCREEN_ON` 保持螢幕，`onPause` 清除，不取得 wake lock 或變更全域設定。切到其他 App 時，健康連線可以保留，但桌面截圖暫停，Host 表單憑證、輸入編輯區（composer）、未送 input／resize、IME 組字與 Ctrl 狀態會清除，避免回前景後送出舊操作。

```text
Foreground -- onPause --> Background -- resume --> Foreground
 input + screenshots      keep healthy link       recover original binding
                          clear pending input      within bounded attempts
                          stop screenshots         no input replay
```

binding 指本機分頁與原 Runtime／session 的關聯，回前景只在有限次數與期限內恢復這份關聯，不改接其他工作。取消組字後，晚到的 `compositionend`／echo 會被擋住，直到新組字或明確新輸入，避免中文輸入被補送。Doze、系統回收與實機網路仍可能造成斷線，這套流程不保證無限保活。

一般桌面觀看只做readiness與圖片觀察，停用／關頁不停止共享服務；宣告 coding_cli 的目標才顯示該 CLI 名稱按鈕；點擊才依設定的 command／args 啟動。桌面預覽沿用現有 operator credential，透過 Runtime 專用 Shell 傳回圖片，細節見 [桌面預覽](../refarch/desktop.md)。

## 語音輸入

Android 終端分頁提供麥克風按鈕，可在語音設定選擇「點一下開始、再點一下結束」或「按住說話、放開結束」。首次使用會要求麥克風權限；拒絕後仍可使用鍵盤。App 使用 Android 裝置端語音辨識，不提供雲端辨識備援；辨識服務及語言包由 Android 管理，缺少所需語言包時可由系統下載。Pixel 9 已在飛航模式確認繁體中文辨識可用，但其他裝置的離線能力與辨識品質取決於其系統服務和已安裝模型。

結束錄音後，App 會清理換行與控制字元，先把辨識文字送到開始錄音時的分頁，再於約 150 ms 後單獨送出 Enter；沒有送出前編輯步驟。錄音期間取消、目標失效或 App 進入背景時不送辨識結果；若文字已送出後目標失效或 App 進入背景，則略過 Enter，已送出的文字不會撤回。專有名詞、路徑及 shell 指令的辨識可能不準，錯字可能直接執行，因此語音不適合輸入需要精確控制的命令。手機鍵列的 `⇧←` 會送出終端 Shift+Left 序列，可用於 Codex CLI 提問時的游標／選取操作；實際效果由終端程式決定。

## Windows 建置

要請 AI 驗證 APK 操作，先準備本節建置工具、Python 與可用的 Android 模擬器，再貼上：

```text
請讀 docs/refarch/test-matrix.md 與 docs/platforms/android-regression.md，
使用專用 Android 模擬器執行真 APK E2E，依這次修改補足必要案例。
保留我的手機和既有 App 資料，回報各案例結果與仍需實機確認的部分。
```

模擬器會實際執行 APK、Keystore 與系統選檔器；ARM64 release、硬體金鑰、特定鍵盤與長時間省電仍須手機驗收。完整範圍見[測試矩陣](../refarch/test-matrix.md)。

先完成 [Windows 檢查](windows.md)，再於 Windows 建置 Android APK。JDK 17、Android SDK／NDK 與 Rust Android target 分別提供 Java、原生 Android 與 Rust 交叉編譯工具。首次安裝工具並確認 debug 建置：

```powershell
.\scripts\setup-android.ps1
.\scripts\build-android.ps1 -Target aarch64 -Debug
```


setup 產生 ignored `target/android-tools/environment.json`；SDK 預設在 `target/android-sdk`，NDK 為 `28.2.13676358`。SDK license 由建置者接受，既有工具環境可沿用。Tauri init 產生 `gen/android`，prepare 再設定安全 manifest、版本與工具鏈，因此這些產物應由既有腳本準備。

原簽章release與驗證：

```powershell
.\scripts\build-android.ps1 -Target aarch64 -OutputApk ./dist/android/art-term-0.1.23-aarch64.apk
.\scripts\verify-android.ps1 -Apk ./dist/android/art-term-0.1.23-aarch64.apk
```


APK 不嵌入連線；首次開啟為空清單，由 App 新增或匯入，原子寫回 `<noBackupFilesDir>/openab-operator-v1/connections.toml`。不再支援建置時 ConfigPath 或私有 bootstrap。與 Windows 共用 schema 5 及核心規則，各裝置獨立保存。

簽章材料在 repo 外，預設 `%LOCALAPPDATA%/art-term/signing`，包含 PKCS12 store（alias `art-term`）、由原 Windows 帳號 DPAPI 保護的密碼 blob，以及已獨立確認的 certificate SHA256。build／sign／verify 使用相同 SigningRoot 或 ExpectedCertificate，才能確認這份 APK 與已安裝版本屬於同一簽章。沒有 store 時不自動生成替代 key，密碼也不能放在命令列或 Git。

驗收先確認同簽章升級保留 operator Keystore、Host revision（保存版本）與 credential context（憑證所屬連線），再操作 Host 增改刪測、建立／列出／接管 Shell、Unicode／IME、detach 與 TTL（存活期限）。輸出 ACK 是已處理資料的確認，Gap 是歷史輸出的缺口；兩者與背景後恢復原 binding、無輸入重播的規則見 [session 與重連](../refarch/sessions.md)。手機觸控、虛擬鍵盤與長時間背景需實機測試。保存限制見 [執行時檔案](../refarch/credential-storage.md)。語音的決策與待觀察項目見 [BDR-001](../bdr/BDR-001-android-voice-input.md)。

## 桌面與既有憑證

桌面一律使用 Native runtime-mcp 與原 Runtime 管理憑證，沒有 SSH 密碼授權視窗。舊 endpoint／agent 桌面關聯遷移到原 HTTPS origin。APK 同簽章升級保留 operator Keystore；0.1.17 的舊 Host JSON 保留但不載入，升級前請用舊 App 匯出，再於新版匯入。既有 schema 5 TOML 保留 id／revision；舊 SSH identity／pin 檔留在原處，App 不再讀寫，不清資料排錯。

## 搬移連線與手機驗收

連線「管理」支援多選匯出／匯入，使用 Android 系統文件選擇器。含管理憑證的檔案以使用者密碼整份加密，匯入時重新以本機 operator Keystore 保護；不依賴來源手機金鑰或私人建置。詳見[操作與格式](../refarch/connection-transfer.md)。

實機需驗證文件建立／開啟與取消、選檔返回、解鎖中切背景及預覽逾時；再確認含憑證匯入後，終端及原設定桌面可用，重開 App 仍有效。也需實際執行手機匯出→自行搬移→Windows 匯入。前端手機尺寸測試、Rust 測試與已簽章 APK 均不取代這些驗收；不要清除既有 App 資料來模擬新手機。
## 附錄：實作與參考

[scripts/build-android.ps1](../../scripts/build-android.ps1)、[scripts/verify-android.ps1](../../scripts/verify-android.ps1)、[apps/desktop/src/mobile/MobileApp.vue](../../apps/desktop/src/mobile/MobileApp.vue)、[crates/tauri-plugin-mobile-vault/android/src/main/java/net/missart/artterm/vault/MobileVaultPlugin.kt](../../crates/tauri-plugin-mobile-vault/android/src/main/java/net/missart/artterm/vault/MobileVaultPlugin.kt)、[crates/tauri-plugin-mobile-voice/android/src/main/java/net/missart/artterm/voice/MobileVoicePlugin.kt](../../crates/tauri-plugin-mobile-voice/android/src/main/java/net/missart/artterm/voice/MobileVoicePlugin.kt)。
