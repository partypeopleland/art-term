# ADR-001：原生 Runtime 客戶端邊界

狀態：已採用。2026-10-17 依 Owner 的直接 instance-mcp 決策更新桌面邊界。2026-10-26 依 Owner 的 Ubuntu Server 需求新增文字客戶端；Runtime 終端邊界與 tmux 歷史延續既有設計。

## 要解決的問題

Windows、Android 與 Ubuntu 終端機都要管理同一種遠端 Shell。若各平台分別處理認證、重連與終端輸出，容易在斷線或工作被取代時產生不同結果。因此客戶端共用 Rust 連線核心，平台層處理作業系統提供的能力與呈現。

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

Ubuntu 的 `art-term` 是直接在 TTY／SSH 執行的 Rust 程式，文字 UI 透過程序內的有界命令與不可變畫面溝通，不使用 WebView 或 Tauri IPC。每個分頁 actor 直接組合既有 core session／history API，vault 以密碼保護本機主金鑰。文字 UI 不取得已保存憑證，也不新增受控端協定。第一版保留既有 GUI 整合，避免為新增終端客戶端同時改動穩定的 Windows／Android。

## 共用核心擁有連線生命週期

核心統一驗證設定、HTTPS 來源地址與遠端工作身分。每個已連線分頁固定綁定當時的 Host 設定版本、Runtime 程序及 Shell 執行個體，避免重連時意外接到同名的新工作。

終端輸出由有界佇列承接，畫面解析完成後才向本機核心回報 ACK（已消費輸出的確認）；若輸出已超出可恢復範圍，使用 Gap 明確表示缺口。斷線只有限次恢復原工作，不重播未確認的輸入，因為重送可能讓指令執行兩次。平台 adapter 沿用這些規則；完整生命週期見[連線與工作](../refarch/sessions.md)。

這個分工讓兩平台共享相同的認證、續期和失敗處理。平台差異仍存在，例如 Android 的背景限制，但不另建一套連線核心。

## 連線保存只有一個來源

0.1.18 起，名稱、分類、能力與平台保護後的憑證都存在 `connections.toml`。移除該檔後重新開啟，App 顯示空清單，不從別處補回已刪除的連線。

原決策曾允許一次性的私有 bootstrap（建置時預先提供連線）。這部分已被單一 TOML 取代：正式啟動不再呼叫 bootstrap，建置也拒絕啟用它。這樣使用者看到的設定檔就能解釋 App 中的連線來源。欄位與升級方式見[連線設定](../refarch/connections.md)。

保存位置改變不代表可以更換金鑰。Windows 保留 DPAPI 的保護內容，Android 保留 operator Keystore 的金鑰識別、命名空間、使用標記與附加驗證資料（AAD）。這些資料共同決定舊密文能否解開；清除舊介面時不得刪除使用者資料或重建金鑰。詳見[憑證保存](../refarch/credential-storage.md)。

## 桌面改用獨立 Screen 身分

Owner 於 2026-10-15 決定由 App 原生層直接持有平台加密的 instance-mcp token 並取得截圖，取代先前 App 只持有 Runtime 憑證、由 Host helper 轉送的限制。Screen 與 Host 各自保存 id／revision，Screen 選擇屬於本機分頁，不由 Runtime origin 或舊 desktop_transport 推定。

WebView 仍不能讀回已保存 token。App 直接向 MCP 截圖，受控端 AI 則由使用者明確啟動的 stdio adapter 使用同一選擇；App 以關 echo 的短命管理通道傳配置，URL／token 只存 adapter 行程記憶體。adapter 本身是 broker，不另建常駐 Shell 或逃離 Runtime kill domain。同 UID 程序仍在信任邊界內。

預覽成功不等於 AI 工具接入，正在執行且未載 adapter 的 AI 也不能宣稱熱插入。工具回傳完整 MCP content／isError／structuredContent；直接截圖避開 PTY，不代表全面過濾工具的圖片內容。停止預覽、取消工具與關閉共享服務分開，App 不重設共用桌面。未選 Screen 的 Windows 舊分頁保留原生 Runtime 截圖相容路徑，Android 新選取 Screen 不使用它。設計及限制見[架構](../refarch/architecture.md)、[桌面預覽](../refarch/desktop.md)與[工具 bridge](../refarch/runtime-desktop.md)。
