# 使用前準備

art-term 是 [openab-pty](https://github.com/openabdev/openab-pty) 的連線客戶端。請先準備可連線的 Runtime；Linux Host 原生安裝見 [Host 安裝指南](openab-host-installation.md)，其他部署方式參考上游文件。art-term 依 HTTPS API 與 WSS 終端協定連線，不要求特定安裝方式。

## 連線前準備

1. 準備已安裝 art-term 的 Windows、Android 或 Ubuntu 裝置，確認可經私人網路連到 Runtime，例如使用 Tailscale。
2. 取得 Runtime 的 HTTPS 根網址與管理憑證。網址不含額外路徑、query 或 fragment；依下節選擇正常憑證驗證或開發模式。
3. 在 art-term 新增 Computer 或 Agent 連線，填入網址與管理憑證，測試連線後選擇 Shell。

目前在 art-term 新建 Shell 另需受控端的私有 tmux 3.6a 與讀頁支援檔，詳見[終端歷史主機準備](refarch/terminal-history.md#主機準備)。這是 art-term 的功能依賴，不能假定官方映像已經備齊；既有 Shell 可繼續連線。連線欄位與限制見[連線設定](refarch/connections.md)。

## 選擇 Runtime 憑證驗證方式

正常模式預設驗證伺服器憑證。Runtime 須提供 client 信任的 CA 所簽發、仍在有效期內且符合網址的憑證。使用 IP 網址時，憑證的 Subject Alternative Name（SAN）必須包含該 IP address。一般模式使用程式內建的 CA 清單，將自建 CA 匯入 Windows 或瀏覽器不會自動加入 art-term 的信任清單。

Windows／Android 正式版也提供 **設定 → 連線設定 → 開發模式：略過伺服器憑證驗證**，可供自簽憑證或 LAN IP 的開發環境使用。此設定預設關閉並會保存，變更套用至後續 Runtime 管理請求與新的終端連線。已建立的終端連線不會在途中更換驗證方式，重新連線時才套用。開發模式仍使用 HTTPS／WSS 加密與原有 token 授權，但不驗證伺服器身分，不必另外取得公開 CA 憑證或讓 client 匯入自建 CA。僅供開發測試使用。

Runtime 入口在兩種模式下都必須提供 TLS，網址仍須為 `https://`，不能改成 `http://` 或 `ws://`。此開關只作用於 Windows／Android 的 Runtime 連線，不影響 Screen 的 MCP 憑證驗證；Ubuntu 終端機版目前仍使用正常驗證。保存方式、略過範圍與生效時點見[開發模式設定](refarch/connections.md#runtime-開發模式)。

## 選用功能

要使用 coding CLI，先在遠端 Shell 環境安裝並登入該工具。桌面預覽另需裝置可達的 instance-mcp、其 URL／token 與桌面環境；在 art-term 新增獨立 Screen，再於 Shell 分頁選取。預覽不需要舊 Runtime 截圖 helper。

若要讓 Shell 與其中的 coding CLI 使用所選桌面，依[Shell 工具 bridge](refarch/runtime-desktop.md)準備通用支援程式。這份選擇只作用於該 Shell，不會自動配置 Discord agent。管理憑證與 Screen token 不貼到聊天中。

## 請 AI 協助檢查連線

```text
請依本專案 docs/openab-host-setup.md 檢查我的 openab-pty 是否可供 art-term 連線。
Host 原生安裝請讀 docs/openab-host-installation.md，其他部署方式參考
https://github.com/openabdev/openab-pty ，沿用我的既有環境。
保留現有檔案、憑證與服務；不要擅自升級或重啟。請告訴我應填的 HTTPS 網址、
適用的憑證驗證模式、如何安全取得管理憑證，以及新建 Shell 所需的支援檔是否齊備。
不要在對話中顯示憑證。
```

完整的 Host 工具、權限與輸入驗證見 [Host 安裝指南](openab-host-installation.md)；既有腳本與升級見 [Host 維護](platforms/linux-host-maintenance.md)。這些工具的適用環境不限制 art-term 的連線方式。
