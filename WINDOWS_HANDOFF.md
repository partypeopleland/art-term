# Windows 開發交接

版本：`0.1.29`；開發分支：`feat/windows-mobile-parity`。本輪接續 `8bc8650`，修復上捲時切換成另一個歷史畫面、字型與亮度不同的問題。

## 原終端直接捲動

即時輸出與歷史都由原本的 xterm 實例呈現。上捲閱讀較早內容，持續輸出不推動閱讀位置，下捲到底自然接續最新輸出；移除桌面歷史覆蓋層及「返回即時」按鈕。閱讀中輸入、貼上或啟動 CLI，透過共同原生連線層先回到即時位置再交付輸入。結果不明的輸入不重送，避免重複執行。

受控端仍提供單一固定介面。共用 helper／Rust 歷史通道向後相容新增能力宣告與 `scroll / follow`，操作工作本身的閱讀位置，畫面沿原 PTY 回傳。沒有 Windows 專用 helper 或服務；Android 的 `viewport / older / ack / keep / close` 與原生快照閱讀保持相容。

原生捲動需要升級同一個通用歷史 helper；舊 helper 明確回報不支援，不改用另一個閱讀器。全螢幕 alternate-screen 程式暫不提供歷史，退出後可再使用。一般 xterm normal buffer 保留本機 scrollback。關頁、隱藏及程序死亡只清理自有閱讀狀態，不結束工作 Shell。

同一 pane 已有其他讀者或手動閱讀狀態時不接管。若其他同帳號程序繞過固定介面，直接改動私有 tmux 的內部模式，無法保證辨識模式世代。未知閱讀狀態先阻擋輸入，確認安全後才放行。

Screens 匯入／匯出、全頁設定、四主題與每分頁桌面工具選擇保留。語音與模型留待下一階段。

## 驗證與交付

- Windows 前端 349 項測試、型別檢查及正常建置通過；GUI Rust 29 項測試通過，含瀏覽器入口的三項驗證。
- 共用歷史核心 22 項、codec 1 項及真官方 Runtime 16 項通過，涵蓋原五操作、連續輸出、輸入順序、異常結束與 120 秒清理。
- Windows 真 EXE／WebView2 接隔離官方 Runtime，8 項原生驗收通過：首次小幅上捲、超過千行、同一視窗／樣式／Unicode、閱讀保持、自然追尾、輸入與 Backspace 不重送、快捷 CLI 入口及退出關分頁。原生測試沒有觀察到 terminal protocol reply；該項由共用後端與前端測試保護。
- Android 自建 AVD 驗證真正 IPC 與原生閱讀 View 的 HTTP／HTTPS 點擊，系統網頁處理程式確實開啟；大小寫 scheme、錯誤網址、滑動／長按及不傳送 PTY 輸入通過。Windows 瀏覽器啟動以注入入口測試，沒有操作 Owner 的實際瀏覽器。
- 計畫與程式碼獨立審查通過。前兩次 Windows 驗收及首次 Android 驗收的 harness 誤判均保留失敗紀錄，修正測量後完整重跑通過，沒有因此弱化產品條件。

證據在 Linux `target/inline-history/`、`target/inline-history-native-20261018/`、`target/terminal-web-links/` 及 artpc `target/windows-regression/inline-history-*`。ignored 證據不隨 git pull 搬移，應依正式測試入口重跑。

NUC／Hana 的同一通用 helper 已升級為 SHA256 `bfcf0fd3d72e14545e3915c22593056dc6606f1573af2f55ed0c40163ac28906`，Node22／tmux3.6a 及檔案回讀通過。NUC 原檔備份名稱為 `tmux-history.cjs.pre-shared-viewport-bfcf0fd3d72e`；Hana 原目錄指標備份為 `tmux-v2.pre-shared-viewport-bfcf0fd3d72e`，新檔位於持久化使用者目錄，原 image 檔案保持 `7426f75d…`。沒有重啟服務、Pod 或既有工作。

不操作 Owner 正在執行的 App 或既有工作。新套件放在獨立目錄，更換 EXE 前先關閉 App，保留私人 `connections.toml`；既有遠端工作可重新 attach。

OS 中文候選選字、實際 AI 推論本輪未驗收；Android 候選 APK 原簽章與封裝驗證通過，Owner 已確認本版可發布。本輪沒有重跑完整 All suite；既有 Host-only golden vector 失敗及全量 Rust 格式差異紀錄保留。

## 接續方式

先讀[文件入口](docs/INDEX.md)、[架構](docs/refarch/architecture.md)、[終端歷史](docs/refarch/terminal-history.md)、[Windows](docs/platforms/windows.md)與[回歸測試](docs/platforms/windows-regression.md)。Windows 專屬工作在 artpc，共用工作在 Linux；同步分支使用 ff-only，保留未提交內容及私人資料。

正常打包使用 PowerShell 7，輸出必須是新目錄：

```powershell
$env:CARGO_BUILD_JOBS = '1'
pwsh -NoProfile -File scripts/package-windows.ps1 -OutputPath dist/windows/new-package
```

2026-10-18 Owner 確認本版可發布，接續合併 main 並以 `v0.1.29` 發布。正式套件從 [GitHub Release](https://github.com/partypeopleland/art-term/releases/tag/v0.1.29) 取得；以下保留候選包資訊供追溯。

## APK 候選

Android 0.1.28／code1056，原簽章與 16KB 對齊驗證通過；不包含私人連線及 fixture。NAS：`\\art.ds\files\my-cli-release\art-term-0.1.28-build1056-android-arm64.apk`。SHA256：`182bd8d3c7ba81d7b697e63a0d054784ad9b6942218e088e15f662ce20c96987`。Owner 已確認此版本可發布；正式版為 0.1.29／code1058。

## Windows 修正版套件

artpc：`D:\code\github\temp\art-term\dist\windows\original-terminal-history-web-links-20261018`。正常 release 建置，公開空白設定；五個交付檔案的 manifest／長度／SHA 均通過，沒有 fixture selector。EXE SHA256：`34a947c882237e6184cec7fba8d876540142a37dadce15c846e8cb975ae5085c`。更換既有安裝時，先關閉舊 App，只更換 EXE／說明檔並保留自己的 `connections.toml`。
