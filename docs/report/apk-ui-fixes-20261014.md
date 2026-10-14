# APK UI 回歸修正與重跑（2026-10-14）

先將既有工作提交為 `bd4dd85f4894fefba29fd29515be82c71f7bc0cc`，再修正快速鍵的鍵盤狀態問題及閱讀區測試誤判。本次完整真 APK UI 矩陣為 **45 PASS、0 FAIL、3 BLOCKED、0 NOT_RUN**；48 筆案例包含一筆安裝前置案例，並不代表所有裝置與狀態組合皆已覆蓋。逐案結果見 [JSON](apk-ui-fixes-20261014-cases.json)，案例設計見 [操作矩陣](../../scripts/android-regression/ui-matrix.md)。

## 修正內容

快速鍵原本呼叫終端 focus；移除此呼叫後，Android WebView 在終端 textarea 仍保持焦點的情況下，觸碰工具列仍可能重新打開 IME。現在透過原生 WindowInsets 的 IME 可見狀態與事件，在鍵盤隱藏時觸碰快速鍵，將終端 inputmode 設為 none；使用者明確要求輸入時恢復 text 並重新取得焦點。上下左右與 Ctrl 直接處理按鍵，不主動取得終端焦點。鍵盤原本開啟時保持開啟，原本隱藏時保持隱藏。初始化狀態查詢不會覆蓋較新的原生事件，卸載時解除監聽。

READ-06 的原判定需更正：切換列表／分頁後，Android 14 的 UIAutomator 有時漏列原生 ScrollView，但實際 Android View hierarchy 中閱讀區仍可見，真實拖曳也能捲動。先前將缺少 accessibility 節點判定為閱讀功能失效，屬於測試誤判。現在優先使用 accessibility 節點；若缺失，必須確認實際原生閱讀 View、可見父容器及位置，再驗證舊內容、實際 ADB 拖曳、畫面像素變化與未送出遠端輸入。兩種 hierarchy 都找不到閱讀區，或拖曳無效，仍判定失敗。本次 READ-06 的拖曳畫面變化比例為 0.25279。未修改原生捲動演算法、終端紀錄投影或 fit 行為。

測試工具新增 AVD 生命週期鎖，避免同時執行的命令爭用模擬器埠；啟動查詢逾時會在既定期限內重試，失敗時回收自己啟動的程序。診斷紀錄只觀察原生回覆，不替換 IPC 結果。先前診斷及失敗結果保留，未計入通過案例。

## 驗證證據

| 驗證 | 結果 |
|---|---|
| 完整 APK UI 矩陣 | 45 PASS、3 BLOCKED；包含歷史輸出、斷線與重連、列表／分頁切換、語音設定、快速鍵、預覽、背景恢復、旋轉及系統選檔等 |
| 額外 keys 範圍 | 3 PASS；含兩個 UI 前置案例，不另累加完整矩陣總數 |
| 快速鍵狀態補強 | 四個方向 × 鍵盤開／關，共八種；檢查最終 Android IME 狀態及期間原生可見事件，無相反狀態；觀察器須先實際收到開與關事件才能通過 |
| 前端單元測試 | 22 檔、268 項通過 |
| 靜態檢查 | 前端 typecheck、Python compile、git diff --check 通過 |
| 清理 | App 停止、fixture 停止、測試自建 AVD 停止、測試所建 forwards/reverses 移除；結束後 adb 無裝置 |

完整執行結果：`target/android-regression/run-ui-dbf0be4cc869/result.json`。鍵盤事件補強：`target/android-regression/run-ui-082492c102a0/result.json`。兩次使用相同 APK；補強僅修改測試觀察器，因此未重跑其餘完整案例。

APK 為 v0.1.24、x86_64 debug、openab-fixture，Android API 34 模擬器。SHA256：`4a48cf6029e670ada180b1177786cca4e4595f9dd63d8536a923e787caac613e`。UI 實際操作安裝的 APK，後端使用既有 TLS/WSS fixture，未 mock 原生 IPC。這次未更新版本號、發布正式手機 APK 或驗證真 NUC Shell。

三筆 BLOCKED 分別為：無辨識服務的 AVD 無法驗證完整語音辨識流程；Linux 無法執行 Windows 雙向搬移；ARM64 正式簽章覆蓋升級、OEM 鍵盤與 Doze 需對應實體裝置。它們未計入 PASS。本次為本機可重跑驗證，尚未建立 GitHub Actions 必過門檻。

[舊報告](apk-ui-20261014.md)保留原始執行結果，並標註 READ-06 分類更正；應以本報告評估目前狀態。
