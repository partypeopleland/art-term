# Runtime 桌面預覽

App 的桌面按鈕讓使用者確認遠端共享畫面的目前狀態。它沿用工作 Shell 所屬的 Runtime 入口與管理憑證，另外建立一個截圖專用 Shell，呼叫 Host 上的 helper，圖片經同一條 Runtime 連線回到 App。截圖指令不送進使用者正在操作的 Shell 或 coding CLI；畫面仍是主機的共享桌面，並非各個 Shell 專屬，也不是可直接操作的遠端桌面。

桌面能力只取自唯一 `connections.toml`，不由 Host／Agent 分類推斷。兩種分類都可設定 `desktop_transport="runtime-mcp"`，入口使用原有 `openab_pty_url`。舊 endpoint／Agent 桌面關聯轉為相同 origin 的 Native 路徑，不改用另一個入口。

## 圖片如何回到 App

```text
App native layer              official openab-pty          Host
      |                              |
      +-- existing work WSS -------->+-- work Shell -> coding CLI
      |
      +-- create temporary Shell --->+-- screenshot Shell
      +-- attach, send launcher ---->|          |
      |                              |          v
      |                              |     local helper -> instance-mcp
      |                              |          |
      |<---- framed JSON + image ----+----------+
      |
      +-- delete own temporary Shell
      v
active Tab preview
```

官方 Runtime 負責建立 Shell 和傳送輸入／輸出；圖片由本專案 helper 取得，不需要修改 Runtime 或安裝自訂桌面 API。App 使用專用回應標記辨識圖片 JSON，處理終端回顯、換行與分段輸出，驗證完整 PNG／JPEG 及解碼後最多 8 MiB 的界限。若輸出缺段、連線中斷、回應不完整或逾時，就清除該次結果並顯示錯誤，不把部分圖片當成成功。

每次觀察會短暫多占一個 Runtime Shell 名額；容量不足只使桌面預覽失敗，工作終端繼續使用。App 隱藏自己管理的截圖 Shell，完成、失敗或取消後清理自己的通道，不刪使用者工作。網路中斷可能使遠端刪除暫時失敗，不能把本機關頁等同遠端已刪除；恢復連線後的清理以本機保留的自建通道紀錄為限。

App 只持有 Runtime 管理憑證。MCP（Model Context Protocol）是 helper 與桌面服務之間的協定，其 URL 和 token 留在主機端受保護設定，不放進 PTY 指令、回顯或 App。準備方式見 [桌面 helper](runtime-desktop.md)。

## 手動與自動觀察

按桌面按鈕後先確認 helper 可擷取，再立即取得圖片；active Tab 每 30 秒更新一次，也可手動更新或隱藏圖片。失敗清除舊截圖，停用／關頁取消本次擷取與後續排程；切到背景停止圖片更新。這些操作不停止共享桌面或 instance-mcp，也不終止工作 Shell。

helper 直接呼叫既有桌面 screenshot，不用 browser 截圖替代，不列舉可能卡住的瀏覽器工具。所有觀看者看到同一桌面；App 不安裝、啟停或輪替共享服務。桌面為選配，純終端連線不需要 helper。

App 直接連獨立桌面入口仍是 [未來計畫](../bdr/BDR-002-future-directions.md)。目前圖片透過官方 Runtime 的專用 Shell 回傳。

## 舊設定的相容邊界

舊 SSH 欄位只在讀取設定時作有界相容輸入，解析後丟棄；App 沒有 SSH executor、pin IO 或帳密授權。沒有可轉換 Runtime origin 的 SSH-only endpoint 保留中繼資料並產生「桌面不可用」警告，不阻止其他有效 Runtime。既有私鑰、pins、known_hosts、DPAPI 與 operator Keystore 不刪除或重建。

## 附錄：實作與參考

[桌面能力 mapping](../../crates/art-term-core/src/runtime_preview.rs)、[native session 與預覽](../../crates/art-term-gui/src/runtime_sessions.rs)、[預覽畫面](../../apps/desktop/src/components/PreviewPanel.vue)、[官方 client contract](https://github.com/openabdev/openab-pty/blob/b82a36620a76c84a67bc6a7ab316cbc60a30e1a9/runtime/CLIENT-CONTRACT.md)。