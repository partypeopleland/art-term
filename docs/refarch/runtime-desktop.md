# 官方 Runtime 的選配桌面 helper

桌面預覽由本專案的 helper 提供；openab-pty 使用未修改的官方產物，只負責建立 Shell 與傳送終端資料。一般終端不需要桌面服務。要使用預覽，Host 管理者先準備既有 instance-mcp，再安裝 helper 與本機受保護設定；App 只需在同一連線宣告 `desktop_transport="runtime-mcp"`。

## 為何使用獨立 Shell

```text
App -> official openab-pty -> work Shell -> user / coding CLI
               |
               +-> temporary screenshot Shell
                              |
                         preview.py
                              | protected local settings
                         mcp_preview.py
                              |
                         instance-mcp -> shared desktop
                              |
App <---- PTY / WSS <---- framed JSON + image_base64
```

工作 Shell 可能正在執行互動 CLI，因此 App 不能把截圖指令直接打入該終端。每次觀察先建立專用 Shell、連上 WSS，再送固定 launcher 命令；完成後刪除自己建立的 Shell。這會短暫多占一個名額，容量上限仍由官方 Runtime 設定決定，不需要本專案的配額或多人協調服務。

PTY 是資料流，prompt、指令回顯與換行都可能混入輸出。launcher 用每次隨機產生的 nonce（一次性辨識碼）標記開始與結束；App 只接受同次完整 JSON，逐段組裝並限制輸出大小。Gap、逾時、中斷或缺少結束標記都讓該次觀察失敗；圖片解碼後最多 8 MiB，必須是有效 PNG／JPEG。

## Host 準備

安裝與既有服務升級見 [Host 維護](../platforms/linux-host-maintenance.md)；首次使用見 [使用前準備](../openab-host-setup.md)，交由 AI 依[安裝操作指南](../openab-host-installation.md)執行。官方 binary 原生執行，NUC 的 Shell 仍可使用原使用者的檔案與 CLI，不以容器取代主機環境。

桌面設定包含 MCP URL 與既有 token，應放在自己擁有、其他帳號不能寫入的目錄，檔案權限為 0600。下列是虛構範例，真實 token 以編輯器填入私人檔案，不貼在命令列或日誌：

```json
{"mcp_url":"http://192.0.2.10:8795/mcp","mcp_token":"example-replace-in-private-file"}
```

沿用初始化入口的明確選配：

```bash
bash scripts/init-host.sh --enable-desktop "$HOME/.config/art-term/desktop.json"
```

launcher 位於 `$HOME/.local/share/art-term/openab-pty/preview.py`，與 `mcp_preview.py` 一起安裝。官方 Shell 的環境會過濾變數，因此 launcher 自行讀取受保護設定，再啟動 helper；不能假設 MCP token 會自動出現在新 Shell。App 傳入的只有固定操作與 nonce，不傳 URL 或 MCP 憑證。

官方 format 4 啟用桌面不需要重建或重啟 Runtime。管理者要清除主機上的桌面設定，可執行 `bash scripts/init-host.sh --rollback-desktop`；這會保留空的受保護 `desktop.env`，讓舊版遷移後仍引用該檔案的服務能正常啟動，再次啟用時可填入設定。啟用交易被中斷時改用 `--recover-desktop` 依紀錄復原，不刪交易檔繞過檢查。App 的「停用桌面」則只停止本機預覽，不清除主機設定。

既有 Host／Hana 的設定與管理憑證需要保留。Hana 繼續使用原本的工作環境，將原受管理 MCP 設定提供給同一 launcher；更新 Runtime 可能中斷 Shell，部署前須確認工作停點。此 helper 不安裝或更動 instance-mcp、共享桌面或 Tailnet。

## 一次觀察的責任

`status` 會實際取得一張有界截圖再丟棄，因此可用狀態代表當下能擷取，並非只確認 HTTP 服務存在。`screenshot` 回傳同次取得的圖片；兩者都直接呼叫 instance-mcp 的桌面 screenshot，不以瀏覽器截圖代替，也不列舉可能觸發其他上游查詢的工具。

helper 使用 Python 標準函式庫，協商支援的 MCP 協定版本（2024-11-05、2025-03-26），不接受未知版本作為成功。它不使用環境代理、重新導向或無界重試。MCP token 留在主機程序內；錯誤只回傳固定非秘密訊息。

App 負責建立、取消與清理截圖通道，Host helper 負責有限時間內完成擷取，官方 Runtime 負責 Shell 生命週期。網路不可達時不能保證遠端 DELETE 立即成功；App 保留自建通道的待清理狀態，不為清理而接管或刪除一般工作。共享桌面本身不由 App 停用或關頁動作關閉。

操作與本機生命週期見 [桌面預覽](desktop.md)，終端恢復限制見 [連線與工作](sessions.md)。

## 附錄：實作與參考

[初始化腳本](../../scripts/init-host.sh)、[MCP helper](../../scripts/openab-runtime/mcp_preview.py)、[官方產物說明](https://github.com/openabdev/openab-pty/tree/b82a36620a76c84a67bc6a7ab316cbc60a30e1a9#images)、[官方 client contract](https://github.com/openabdev/openab-pty/blob/b82a36620a76c84a67bc6a7ab316cbc60a30e1a9/runtime/CLIENT-CONTRACT.md)。
