# AI 文件入口

這是 art-term 的主題知識庫。第一次接手可從[架構](refarch/architecture.md)理解 App、Runtime 與遠端 Shell 的關係，再依問題選讀下表。每篇直接解釋運作方式、設計理由與限制；正文連到相關主題，文末附錄供維護者追溯實作與資料來源。

理解系統只需讀相關主題，不必一次載入全部文件或重新查證整個實作。

| 任務／問題 | 先讀 |
|---|---|
| 模組責任、信任邊界、資料流 | [架構](refarch/architecture.md) |
| 唯一 TOML、欄位、能力、舊版相容 | [連線設定](refarch/connections.md) |
| Host／Agent 分類、Shell 選擇、ACK、重連 | [連線與工作生命週期](refarch/sessions.md) |
| 匯出／匯入、加密格式、新裝置還原 | [連線搬移](refarch/connection-transfer.md) |
| DPAPI／Keystore、金鑰遺失、保存與清理 | [憑證與檔案](refarch/credential-storage.md) |
| App 共享桌面預覽與 binding | [桌面](refarch/desktop.md) |
| 官方 Runtime 專用截圖 Shell 與 Host helper | [Runtime 桌面 helper](refarch/runtime-desktop.md) |
| Clone／fork 後本地打包、自有簽章與品牌調整 | [本地建置與自訂版本](platforms/local-build.md) |
| Windows 工具鏈、操作差異、打包與驗收 | [Windows](platforms/windows.md) |
| GitHub Actions、EXE／APK 自動建置、簽章 Secrets 與 Release | [自動發布](platforms/releases.md) |
| 功能回歸、unit／原生 E2E、測試計畫與本機執行 | [Windows 回歸測試](platforms/windows-regression.md) |
| 哪些操作受保護、跨平台案例與驗收界線 | [功能測試矩陣](refarch/test-matrix.md) |
| 真 APK／模擬器／Keystore／系統選檔／跨裝置搬移 | [Android APK 原生 E2E](platforms/android-regression.md) |
| Linux 真官方 Runtime／helper／Host fixture驗收 | [Linux 回歸測試](platforms/linux-regression.md) |
| Android 工具鏈、觸控／背景、原簽章與驗收 | [Android](platforms/android.md) |
| 使用者：必要依賴與交給 AI 的安裝 prompt | [使用前準備](openab-host-setup.md) |
| AI：檢查受控端、安裝 Runtime 與驗證連線 | [AI 安裝操作指南](openab-host-installation.md) |
| Host 進階安裝、排查、升級、復原與移除 | [Host 維護](platforms/linux-host-maintenance.md) |
| 原生 Runtime 邊界的決策理由 | [ADR-001](adr/ADR-001-client-boundaries.md) |
| Android 語音輸入現況、決策與待觀察事項 | [BDR-001](bdr/BDR-001-android-voice-input.md) |
| 未排程候選及保護條件 | [BDR-002](bdr/BDR-002-future-directions.md) |
| 版本差異、遷移與本版驗證限制 | [0.1.19 release note](report/0.1.19.md)（前一版 [0.1.18](report/0.1.18.md)） |
| 文件寫法、拆篇與載入原則 | [文件規則](steering/documentation.md) |
| 第三方散布義務 | [第三方說明](third-party-notices.md) |

`refarch/`、`platforms/` 與 Host 頁解釋目前行為；ADR／BDR 記錄決策理由與提案，並標明實作狀態。`report/` 記版本差異及驗證範圍。歷史測試不代表目前實機驗收。
