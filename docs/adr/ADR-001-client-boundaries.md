# ADR-001：原生 Runtime 客戶端邊界

狀態：已採用。2026-10-11 更新連線保存決策；原生 Runtime 邊界延續既有設計。

## 要解決的問題

Windows 與 Android 都要管理同一種遠端 Shell。若各平台分別處理認證、重連與終端輸出，兩端容易在斷線或工作被取代時產生不同結果。因此兩平台共用 Rust 連線核心，平台層只處理作業系統提供的能力。

```text
Windows UI ----+                         +--> Windows DPAPI
               |                         |
               v                         |
            Tauri IPC --> Shared Rust core --> HTTPS / WSS --> Runtime
               ^                         |
               |                         |
Android UI ----+                         +--> Android Keystore
```

UI 透過 Tauri IPC（受權限限制的程序間呼叫）發出操作。共用核心保存連線身分、持有憑證並連接 Runtime；Windows DPAPI 與 Android Keystore 則負責裝置上的憑證保護。UI 可以顯示與操作分頁，但不能讀回已保存的管理憑證或 session token。

## 共用核心擁有連線生命週期

核心統一驗證設定、HTTPS 來源地址與遠端工作身分。每個已連線分頁固定綁定當時的 Host 設定版本、Runtime 程序及 Shell 執行個體，避免重連時意外接到同名的新工作。

終端輸出由有界佇列承接，畫面解析完成後才向本機核心回報 ACK（已消費輸出的確認）；若輸出已超出可恢復範圍，使用 Gap 明確表示缺口。斷線只有限次恢復原工作，不重播未確認的輸入，因為重送可能讓指令執行兩次。平台 adapter 沿用這些規則；完整生命週期見[連線與工作](../refarch/sessions.md)。

這個分工讓兩平台共享相同的認證、續期和失敗處理。平台差異仍存在，例如 Android 的背景限制，但不另建一套連線核心。

## 連線保存只有一個來源

0.1.18 起，名稱、分類、能力與平台保護後的憑證都存在 `connections.toml`。移除該檔後重新開啟，App 顯示空清單，不從別處補回已刪除的連線。

原決策曾允許一次性的私有 bootstrap（建置時預先提供連線）。這部分已被單一 TOML 取代：正式啟動不再呼叫 bootstrap，建置也拒絕啟用它。這樣使用者看到的設定檔就能解釋 App 中的連線來源。欄位與升級方式見[連線設定](../refarch/connections.md)。

保存位置改變不代表可以更換金鑰。Windows 保留 DPAPI 的保護內容，Android 保留 operator Keystore 的金鑰識別、命名空間、使用標記與附加驗證資料（AAD）。這些資料共同決定舊密文能否解開；清除舊介面時不得刪除使用者資料或重建金鑰。詳見[憑證保存](../refarch/credential-storage.md)。

## 桌面沿用 Runtime 的身分邊界

桌面預覽使用同一 Runtime 的 HTTPS 管理入口。能否觀看由明確設定授予，建立連線目標時就固定，後續不能靠改顯示名稱把分頁綁到另一台主機。

觀看只取得既有共享桌面的截圖，關閉預覽不停止主機服務。App 直接連獨立桌面服務仍是[未來提案](../bdr/BDR-002-future-directions.md)，需要另行設計可信入口與觀看授權。目前元件關係見[架構](../refarch/architecture.md)與[桌面預覽](../refarch/desktop.md)。
