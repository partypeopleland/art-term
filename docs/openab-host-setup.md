# 使用前準備

0.1.26 起，新建 Shell 另需私有 tmux 3.6a 與讀頁支援檔；完成 Runtime 安裝後依[終端歷史主機準備](refarch/terminal-history.md#主機準備)設定。既有 Shell 可繼續連線。

art-term 讓 Windows 電腦或 Android 手機連到 Linux 受控端，操作終端與 coding CLI。先準備以下依賴，再把下面的 prompt 貼給能操作受控端的 AI，由它讀取安裝文件並完成設定。

## 需要哪些依賴

| 位置／用途 | 需要準備 |
|---|---|
| 受控端 | Linux 主機、一般使用者帳號，以及 Bash、systemd、Git、curl、Python 3、CA 憑證等基本工具。預設安裝支援 Ubuntu 22.04／24.04 x86_64。 |
| 網路 | 受控端與 Windows／手機已安裝並登入 Tailscale，能透過同一個私人網路（Tailnet）互通。安裝方式見 [Tailscale 官網](https://tailscale.com/download)。 |
| 終端服務 | 官方 openab-pty，由你的 AI 依本專案文件安裝與設定。 |
| 使用裝置 | 已安裝 art-term 的 Windows 電腦或 Android 手機。 |
| coding CLI（選配） | 你要使用的 CLI 已在受控端安裝並登入。 |
| 桌面預覽（選配） | 手機／電腦可達的 instance-mcp、其 URL／token 與桌面環境；在 App 的 Screens 獨立保存。只用終端就不需要。 |
| Codex 桌面工具（選配） | 受控端同帳號可達相同 instance-mcp，安裝通用 Shell broker／工具 client；新工作 Shell 及其中的 Codex 共用選擇，Discord Agent 暫不包含。 |

**受控端不需要為 art-term 安裝 Rust、Cargo 或 Docker。** 安裝流程直接使用官方執行檔；所選 CLI 若另有依賴，交由 AI 依該工具的文件處理。

## 把這段貼給你的 AI

AI 需要能讀取本專案檔案，並能操作你指定的 Linux 受控端。以下使用專案相對路徑；若 AI 找不到專案，先告訴它專案放在哪裡。

```text
請協助我把 Linux 受控端設定成可供 art-term 連線使用。

請先讀取本專案的 docs/openab-host-installation.md，依該文件檢查依賴、安裝官方 openab-pty，並驗證結果。

請先確認目標主機與要執行 Shell 的一般使用者帳號。保留既有檔案、憑證和服務；若已安裝，先檢查並沿用。需要升級或重啟、可能中斷現有工作時，先告訴我影響。

我先需要終端連線；桌面預覽等我提出需求時再處理。缺少必要依賴或權限時，請只說明需要我補充什麼。

完成後，請告訴我在 art-term 要填的連線網址、如何在自己的終端安全取得管理憑證，以及如何確認連線成功。不要把憑證貼到對話中。
```

完成後，在 art-term 新增連線，填入 AI 提供的網址，再依指示取得並填入管理憑證。開啟 Shell 並確認能操作受控端，即可開始使用。

桌面不需要啟用舊 Runtime 截圖 helper。使用者在全站齒輪新增 Screen，於 live Shell 分頁明確選擇；App 直接取圖。需要讓已連入的 Shell 及其中的 coding CLI 操作相同桌面時，請 AI 依[工具 bridge](refarch/runtime-desktop.md)準備通用程式，安裝後建立一次新連線；先選 Screen 再啟動 Codex 也可使用，不需要 App 額外啟動入口。其他支援標準 MCP 的 CLI 可用 `art-term-screen mcp-config` 取得通用入口，依各自設定登記一次；不需固定桌面 token 或 Shell binding。此 Shell 整合仍須依當次交付驗證，不把安裝完成當成實機通過。Screen token 不貼聊天、不填入受控端永久設定；Host 匯出／匯入不包含 Screen。
