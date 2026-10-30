# 使用前準備

art-term 是 [openab-pty](https://github.com/openabdev/openab-pty) 的連線客戶端。請先自行準備可連線的 Runtime；安裝、映像建置與部署方式以該專案的文件為準。art-term 依 HTTPS API 與 WSS 終端協定連線，不要求特定安裝方式。

## 連線前準備

1. 準備已安裝 art-term 的 Windows、Android 或 Ubuntu 裝置，確認可經私人網路連到 Runtime，例如使用 Tailscale。
2. 取得 Runtime 的可信 HTTPS 根網址與管理憑證。網址不含額外路徑、query 或 fragment；art-term 不提供略過 TLS 驗證的選項。
3. 在 art-term 新增 Computer 或 Agent 連線，填入網址與管理憑證，測試連線後選擇 Shell。

目前在 art-term 新建 Shell 另需受控端的私有 tmux 3.6a 與讀頁支援檔，詳見[終端歷史主機準備](refarch/terminal-history.md#主機準備)。這是 art-term 的功能依賴，不能假定官方映像已經備齊；既有 Shell 可繼續連線。連線欄位與限制見[連線設定](refarch/connections.md)。

## 選用功能

要使用 coding CLI，先在遠端 Shell 環境安裝並登入該工具。桌面預覽另需裝置可達的 instance-mcp、其 URL／token 與桌面環境；在 art-term 新增獨立 Screen，再於 Shell 分頁選取。預覽不需要舊 Runtime 截圖 helper。

若要讓 Shell 與其中的 coding CLI 使用所選桌面，依[Shell 工具 bridge](refarch/runtime-desktop.md)準備通用支援程式。這份選擇只作用於該 Shell，不會自動配置 Discord agent。管理憑證與 Screen token 不貼到聊天中。

## 請 AI 協助檢查連線

```text
請依本專案 docs/openab-host-setup.md 檢查我的 openab-pty 是否可供 art-term 連線。
安裝與部署請參考 https://github.com/openabdev/openab-pty ，沿用我的既有環境。
保留現有檔案、憑證與服務；不要擅自升級或重啟。請告訴我應填的 HTTPS 網址、
如何安全取得管理憑證，以及新建 Shell 所需的支援檔是否齊備。不要在對話中顯示憑證。
```

若維護者選用本專案既有的 Linux 原生安裝腳本，可查[選用安裝工具指南](openab-host-installation.md)與[Host 維護](platforms/linux-host-maintenance.md)。這些工具的適用環境不限制 art-term 的連線方式。
