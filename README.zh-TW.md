# art-term

[English](README.md) · [下載 Windows 與 Android 版](https://github.com/partypeopleland/art-term/releases/latest)

用 Windows 電腦或 Android 手機，接著操作 Linux 上的終端與 coding CLI。指令和工具都在 Linux 主機執行；art-term 提供 Shell 分頁、歷史捲動，也能選配遠端桌面預覽。

## 畫面預覽

以下是使用正式介面元件、搭配虛構示範資料的瀏覽器截圖。預覽不含原生視窗外框、Android 系統鍵盤與裝置服務。

| Android 連線首頁 | Android 終端 |
| --- | --- |
| <img src="assets/screenshots/android-home.png" alt="Android 首頁，依 Computers、Agents 與 Screens 分類" width="300"> | <img src="assets/screenshots/android-terminal.png" alt="Android 終端，含 Shell 分頁與快捷鍵" width="300"> |

<img src="assets/screenshots/windows-home.png" alt="Windows 連線首頁" width="900">

想自己點看看，可在本機啟動[介面預覽](docs/platforms/ui-preview.md)。預覽使用示範回應，不會執行指令。

## 可以做什麼

- 同時開多個 Shell 分頁，建立新工作或接回既有工作。
- 上捲查看終端歷史，點擊網址用裝置瀏覽器開啟。
- 選擇 Screen 查看共享桌面。Windows 可用能拖曳、調整大小的放大面板；Android 支援全螢幕雙指縮放與平移。預覽圖片不提供遠端滑鼠、鍵盤操作。
- 用密碼加密匯出、匯入連線與 Screen 設定，搬到其他裝置。
- Android 可試用裝置端語音輸入。目前只在 Pixel 9 測試；離線使用需要語言資料，技術名詞可能辨識錯誤。

## 開始使用

1. 到 [Releases](https://github.com/partypeopleland/art-term/releases/latest) 下載 Windows ZIP 或 Android ARM64 APK。
2. 準備已安裝官方 openab-pty Runtime 的 Linux 主機，讓裝置能透過私人網路連線，例如 Tailscale。詳見[使用前準備](docs/openab-host-setup.md)。
3. 在 App 新增 Runtime 網址與管理憑證，建立或選擇 Shell。想使用的 coding CLI 要先裝在主機上。

桌面預覽是選配，需要另外準備共享桌面與 instance-mcp。App 不會代裝遠端服務或 coding 工具。

## 系統怎麼連在一起

```text
Windows／Android App
  Vue 介面 + 終端
         |
  Tauri 原生層 + 共用 Rust 核心
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
