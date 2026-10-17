# Android 客戶端

Android 版讓手機操作與 Windows 相同的 Runtime Host 與 Shell，沿用 Rust 的原生 HTTPS／WSS 核心；平台層處理觸控、輸入法、背景生命週期與本機憑證保護。目前以 Android APK 開發及驗收為主，Windows 功能開發暫停。

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

目前 package 為 `net.missart.artterm`，minSdk 24／targetSdk 36，release 為 ARM64、16KB page alignment；本版為 0.1.27／code 1050。更新必須使用原簽章，才能維持既有 App 與 Keystore 的連續性。

## 操作與背景

頂端固定顯示可點擊的 `>_ art-term`、Main／Connect 滑動條與全站齒輪。Main 向左滑進 Connect，Connect 向右滑回 Main；點文字也能切換。頁面拖曳時，底下的指示色條跟著可視範圍移動，頂列保持固定。點品牌圖示或名稱進入軟體資訊首頁，顯示目前版本、近期更新重點與可展開的完整更新文件。

Main 依序顯示 Computers（主機連線）、Agents（工作助理）與 Screens。分類圖示及兩行標題保留，數量在最右側；點分類可收合內容。Screens 列出實際保存的獨立 instance-mcp 桌面，與 Host 設定分開。主機左側灰點表示沒有本機已連線分頁，綠點表示此裝置已有已連線分頁，並非遠端在線探測。

主機右側列出既有 Shell 名稱按鈕；沒有 Shell 時留空。多筆預設收合為「連線數:03」這類摘要，點摘要或左側主機名稱展開；展開後第一列改成第一個 Shell 名稱，其餘等寬按鈕往下排列，再點主機名稱收合。點 Shell 名稱才 attach，已由別處連線的 Shell 標示「接管」。右側 `＋` 建立新的 tmux 工作並連線；不在結果不確定時自動重試。分類與能力見[設定](../refarch/connections.md)。

齒輪直接進入滑動條下方的設定頁，重複點擊仍留在設定。外觀提供科技感、深黑、柔光、懷舊四種 Theme，選取立即套用所有手機頁面、終端與 Android 原生歷史閱讀器，並保存本機偏好。下方「連線設定」集中 Host 匯入、匯出及新增表單；分類可選 Screen，另有獨立 Screen 編輯／刪除區；主畫面不再提供新增設定或管理按鈕。新增表單保存名稱、分類、HTTPS 根網址與管理憑證，各裝置獨立保存。

Connect 有分頁時才顯示分頁列，狀態由分頁前的小圓點呈現；尚未選擇連線時不顯示提示區。快速鍵依序為 Tab、Esc、`⇧←`、↑、↓、←、→、ENTER、CTRL+C、鍵盤，不另提供 Ctrl latch、Detach 或輸入編輯區。Shell 正常結束時自動清除該本機分頁；沒有其他分頁時回到「選擇連線」，未連線頁也可右滑回 Main。關閉分頁仍只中斷本機連線，不刪除遠端工作。

終端單指上下滑動、放手慣性及細捲動軌道用於閱讀；Android 原生歷史閱讀器將水平頁面手勢交回共用滑動控制器，使 Connect 內也能右滑回 Main。互動程式依手勢開始位置接收有界滑鼠事件，未回報歷史範圍時不捏造絕對進度。點 HTTP(S) 網址複製至本機剪貼簿，滑動仍可捲動。

前景使用 `FLAG_KEEP_SCREEN_ON` 保持螢幕，`onPause` 清除，不取得 wake lock 或變更全域設定。切到其他 App 時，健康連線可以保留，但桌面截圖暫停，Host 表單憑證、未送 input／resize、IME 組字與待送輸入狀態會清除，避免回前景後送出舊操作。

```text
Foreground -- onPause --> Background -- resume --> Foreground
 input + screenshots      keep healthy link       recover original binding
                          clear pending input      within bounded attempts
                          stop screenshots         no input replay
```

binding 指本機分頁與原 Runtime／session 的關聯，回前景只在有限次數與期限內恢復這份關聯，不改接其他工作。取消組字後，晚到的 `compositionend`／echo 會被擋住，直到新組字或明確新輸入，避免中文輸入被補送。Doze、系統回收與實機網路仍可能造成斷線，這套流程不保證無限保活。

每個 live Shell 分頁可明確選 Screen，App 原生層直接向其 instance-mcp 取圖，不透過 Runtime 截圖 Shell。預覽標題顯示 Screen 名稱，圖片下方省略時間與工具狀態；本次工具範圍是 App 連入的工作 Shell 及該 Shell 裡的 coding CLI，不使用 Host 共用選擇，也暫不接入 Discord Agent。新 Shell 的 supervisor 在 Bash／CLI 前存在，App 選 Screen 後，Shell client 及稍後啟動的 scoped Codex 使用同一記憶體選擇；可照常輸入 `codex --yolo`，不需要 App 額外 Codex 入口，不修改全域 CLI 或恢復固定上游憑證。其他支援 MCP 的 CLI 可依原生設定登記 `art-term-screen mcp-config` 輸出的通用入口一次；不宣稱全部零配置。既有 Shell／CLI 不注入命令，helper 安裝後需建立新連線；CLI EOF 不清除 Shell 選擇，Shell 結束清除 broker。預覽成功不等於工具已驗收，實際安裝、PTY／Runtime 及手機結果另行確認。停止預覽不清除工具憑證；「取消桌面工具」才 clear 並停圖，兩者都不停止共享服務。細節見[桌面預覽](../refarch/desktop.md)。

## 語音輸入

Android 終端底部提供語音輸入按鈕、辨識文字預覽與無外框齒輪。點語音齒輪時隱藏麥克風，原列維持相同高度，改為並排呈現「錄音方式」與「文字預覽方式」。錄音可選點一下開始／再點一下結束，或按住說話／放開結束；文字可選按鈕右側預覽，或在畫面中央以較大燈箱顯示。沒有辨識內容時預覽保持空白，偏好會保存於本機。已儲存燈箱偏好不會在連線時自動彈出；開始錄音、手動開啟或切換燈箱模式時才顯示。

首次使用會要求麥克風權限；拒絕後仍可使用鍵盤。App 使用 Android 裝置端語音辨識，不提供雲端辨識備援；辨識服務及語言包由 Android 管理，缺少所需語言包時可由系統下載。支援的 Android 14（API 34）以上服務預設允許 zh-TW／en-US 語言切換，介面不另提供開關；實際支援仍取決於裝置辨識服務及模型。Pixel 9 曾在飛航模式確認繁體中文辨識可用，但其他裝置的離線能力與品質取決於系統服務和已安裝模型。

結束錄音後，App 會清理換行與控制字元，先把辨識文字送到開始錄音時的分頁，再於約 150 ms 後單獨送出 Enter；沒有送出前編輯步驟。錄音期間取消、目標失效或 App 進入背景時不送辨識結果；若文字已送出後目標失效或 App 進入背景，則略過 Enter，已送出的文字不會撤回。專有名詞、路徑及 shell 指令的辨識可能不準，錯字可能直接執行，因此語音不適合輸入需要精確控制的命令。手機鍵列的 `⇧←` 會送出終端 Shift+Left 序列，可用於 Codex CLI 提問時的游標／選取操作；實際效果由終端程式決定。

## Windows 建置

第一次 clone／fork、需要建立自己的 release 簽章，或要修改 Android 識別碼時，先讀[本地建置與自訂版本](local-build.md)。下列建置命令假設已完成該指南的前端依賴安裝；維護者的簽章材料不隨原始碼提供。

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


APK 不嵌入連線；首次開啟為空清單，由 App 新增或匯入，原子寫回 `<noBackupFilesDir>/openab-operator-v1/connections.toml`。不再支援建置時 ConfigPath 或私有 bootstrap。與 Windows 共用 schema 5／6 保存模型及核心規則，各裝置獨立保存。

簽章材料在 repo 外，預設 `%LOCALAPPDATA%/art-term/signing`，包含 PKCS12 store（alias `art-term`）、由原 Windows 帳號 DPAPI 保護的密碼 blob，以及已獨立確認的 certificate SHA256。build／sign／verify 使用相同 SigningRoot 或 ExpectedCertificate，才能確認這份 APK 與已安裝版本屬於同一簽章。沒有 store 時不自動生成替代 key，密碼也不能放在命令列或 Git。

驗收先確認同簽章升級保留 operator Keystore、Host revision（保存版本）與 credential context（憑證所屬連線），再操作 Host 增改刪測、建立／列出／接管 Shell、Unicode／IME、detach 與 TTL（存活期限）。輸出 ACK 是已處理資料的確認，Gap 是歷史輸出的缺口；兩者與背景後恢復原 binding、無輸入重播的規則見 [session 與重連](../refarch/sessions.md)。手機觸控、虛擬鍵盤與長時間背景需實機測試。保存限制見 [執行時檔案](../refarch/credential-storage.md)。語音的決策與待觀察項目見 [BDR-001](../bdr/BDR-001-android-voice-input.md)。

## 桌面與既有憑證

桌面使用獨立 Screen 的平台加密 token，沒有 SSH 密碼授權視窗。舊 desktop_transport 僅保留 Host 相容解析，不產生 Screen 或啟用截圖轉送。APK 同簽章升級保留 operator Keystore；0.1.17 的舊 Host JSON 保留但不載入，升級前請用舊 App 匯出，再於新版匯入。既有 schema 5 TOML 保留 id／revision；舊 SSH identity／pin 檔留在原處，App 不再讀寫，不清資料排錯。

## 搬移連線與手機驗收

齒輪設定頁的「連線設定」支援多選匯出／匯入，使用 Android 系統文件選擇器。含管理憑證的檔案以使用者密碼整份加密，匯入時重新以本機 operator Keystore 保護；不依賴來源手機金鑰或私人建置。詳見[操作與格式](../refarch/connection-transfer.md)。

實機需驗證文件建立／開啟與取消、選檔返回、解鎖中切背景及預覽逾時；再確認含憑證匯入後，終端可用、本機獨立 Screens 保留，重開 App 仍有效。Host 搬移目前不含 Screen；需另驗證 Screen CRUD、重開及每分頁選擇。也需實際執行手機匯出→自行搬移→Windows 匯入。前端手機尺寸測試、Rust 測試與已簽章 APK 均不取代這些驗收；不要清除既有 App 資料來模擬新手機。
## 附錄：實作與參考

[scripts/build-android.ps1](../../scripts/build-android.ps1)、[scripts/verify-android.ps1](../../scripts/verify-android.ps1)、[apps/desktop/src/mobile/MobileApp.vue](../../apps/desktop/src/mobile/MobileApp.vue)、[crates/tauri-plugin-mobile-vault/android/src/main/java/net/missart/artterm/vault/MobileVaultPlugin.kt](../../crates/tauri-plugin-mobile-vault/android/src/main/java/net/missart/artterm/vault/MobileVaultPlugin.kt)、[crates/tauri-plugin-mobile-voice/android/src/main/java/net/missart/artterm/voice/MobileVoicePlugin.kt](../../crates/tauri-plugin-mobile-voice/android/src/main/java/net/missart/artterm/voice/MobileVoicePlugin.kt)。
