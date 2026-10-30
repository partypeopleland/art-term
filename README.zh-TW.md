# art-term

[English](README.md) · [下載 Windows、Android 與 Ubuntu 版](https://github.com/partypeopleland/art-term/releases/latest)

用 Windows 電腦、Android 手機或 Ubuntu 終端機，接著操作 Linux 上的終端與 coding CLI。指令和工具都在 Linux 主機執行；art-term 提供 Shell 分頁、歷史捲動，也能選配遠端桌面預覽。

Ubuntu 24.04 Server、Desktop 或 SSH 可使用 `art-term` 終端機客戶端，以文字介面管理同一套遠端 Host／Shell，無需圖形桌面。工作區分為 Host／Shell 側欄、實際 Screen 截圖預覽與多分頁終端，使用私有帳號金鑰直接啟動，不另設解鎖密碼。可用滑鼠選擇分頁及固定展開的連線樹，桌面工具狀態持續可見；快捷鍵可直接編輯 TOML 自訂。建置、保存與操作見 [Ubuntu 終端機版](docs/platforms/linux.md)。從 [Releases](https://github.com/partypeopleland/art-term/releases/latest) 下載 Linux x86_64 壓縮檔，解壓後執行 `./art-term`。

## 畫面預覽

Windows 與 Android 圖片是使用正式介面元件、搭配虛構示範資料的瀏覽器截圖，不含原生視窗外框、Android 系統鍵盤與裝置服務。

| Android 連線首頁 | Android 終端 |
| --- | --- |
| <img src="assets/screenshots/android-home.png" alt="Android 首頁，依 Computers、Agents 與 Screens 分類" width="300"> | <img src="assets/screenshots/android-terminal.png" alt="Android 終端，含 Shell 分頁與快捷鍵" width="300"> |

<img src="assets/screenshots/windows-home.png" alt="Windows 連線首頁" width="900">

想自己點看看，可在本機啟動[介面預覽](docs/platforms/ui-preview.md)。預覽使用示範回應，不會執行指令。

Linux TUI 在真正的終端機中擷取，Host、Shell 輸出與 Screen 均使用虛構示範資料。畫面包含連線樹、原生圖片預覽與 Shell 分頁。

<img src="assets/screenshots/linux-tui.png" alt="Linux TUI，左上為 Host 與 Shell 連線樹，左下為 Screen 圖片預覽，右側為兩個 Shell 分頁" width="900">

## 可以做什麼

- 同時開多個 Shell 分頁，建立新工作或接回既有工作。
- 上捲查看終端歷史，點擊網址用裝置瀏覽器開啟。
- 選擇 Screen 查看共享桌面。Windows 可用能拖曳、調整大小的放大面板；Android 支援全螢幕雙指縮放與平移。預覽圖片不提供遠端滑鼠、鍵盤操作。
- 用密碼加密匯出、匯入連線與 Screen 設定，搬到其他裝置。
- Android 可試用裝置端語音輸入。目前只在 Pixel 9 測試；離線使用需要語言資料，技術名詞可能辨識錯誤。

## 開始使用

1. 到 [Releases](https://github.com/partypeopleland/art-term/releases/latest) 下載 Windows ZIP、Android ARM64 APK 或 Ubuntu x86_64 壓縮檔。
2. 自行準備可連線的 [openab-pty Runtime](https://github.com/openabdev/openab-pty)，安裝、映像與部署方式請參考上游文件。
3. 在 art-term 新增其 HTTPS 根網址與管理憑證，建立或選擇 Shell。[使用前準備](docs/openab-host-setup.md)另列目前新建 Shell 所需的受控端支援檔。

art-term 依 Runtime 的 HTTPS API 與 WSS 終端協定連線，不依賴特定安裝方式。桌面預覽是選配，需要另外準備共享桌面與 instance-mcp；想使用的 coding CLI 要先裝在遠端 Shell 環境中。

## 系統怎麼連在一起

```text
Windows／Android 介面 -> Tauri 原生層 --+
Ubuntu 文字介面 ----------------------+-> 共用 Rust 核心
         |-- HTTPS / WSS --> openab-pty Runtime --> tmux Shell --> coding CLI
         |
         +-- 直接 MCP --> instance-mcp --> 共享桌面截圖
                                     ^
                    Shell 工具橋接 --+  （供 coding CLI 選配使用）
```

openab-pty 管理遠端 Shell 與終端傳輸，tmux 在主機保留工作與歷史。App 直接透過 MCP 取得 Screen 圖片；選配的 Shell 工具橋接，讓已設定的 coding 工具使用所選桌面。Windows 與 Android 共用連線核心，鍵盤、觸控與憑證保存則由各平台處理。詳見[系統架構](docs/refarch/architecture.md)與[桌面預覽](docs/refarch/desktop.md)。

## 自行建置與更多文件

想打包自己的版本，可讀[本地建置與自訂](docs/platforms/local-build.md)、[自動發布](docs/platforms/releases.md)，或從[文件入口](docs/INDEX.md)找需要的主題。

歡迎到 [Issues](https://github.com/partypeopleland/art-term/issues) 回報問題，請勿附上憑證或私人主機資訊。[MIT 授權](LICENSE) · [第三方說明](docs/third-party-notices.md)。
