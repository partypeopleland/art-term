# Windows 開發交接

分支：`feat/windows-mobile-parity`。本輪接續 `c2a00c9` 修復 Windows 滾輪無法讀歷史。該交付改用專用滑鼠控制，卻只在隔離環境準備新版 helper，實際 NUC／Hana 的既有 helper 不支援，造成歷史功能失效。

## 受控端的固定契約

Owner 明確要求：受控端是獨立受控點，提供固定介面；外部各平台遵循同一套方法。平台差異只留在呈現與操作事件，不能為 Windows 另開機制，再要求受控端配合安裝。

歷史統一使用既有 `viewport / older / ack / keep / close`，共用 Rust 通道與 `HostHistoryWindow`。Android 原生 View 維持原流程；Windows 即時輸入輸出用 xterm，上捲時以桌面呈現元件按需讀取同一份快照，保留閱讀 anchor 與 1024 行窗口，回到底部返回即時畫面。已撤回專用控制路徑；沒有桌面專用預載模型，也沒有 live 快照輪詢。

正式受控端未修改或重啟。恢復共用介面後的 helper 建置 SHA-256 為 `7426f75dfa46ef32471c48cf30056b8fa622e48678361fd9f85f47b45b8511c3`，與 NUC／Hana 原有檔案相同，不需要更新 helper 或 image。

Screens 匯入／匯出、新裝置重新保護 token、全頁設定、主題與桌面工具選擇均保留；語音及模型留待下一階段。

## 驗證

- 共用核心既有歷史協定 16 項測試通過。
- 使用正式已安裝舊 helper，在隔離官方 Runtime 驗證 viewport／older／ACK／keep、快照讀取與 Bash exit 結束工作，通過。只清理自己建立的測試工作。
- Windows 前端 338 個測試、型別檢查及 Vite 建置通過；Windows GUI Rust 24 個測試通過。
- Windows Edge headless 實際滾輪：首個 -34px 手勢觸發快照舊頁、閱讀時保持、回到底部回到 live xterm，以及既有側欄／Screen／設定流程，全部通過。
- 目前 Owner 正使用 App，新版原生 EXE 的 GUI E2E 未執行；headless 是預覽 fixture，不能當成原生 IPC／DPAPI／實際 Host 的端到端驗收。獨立計畫、整體來源與 busy 重試增量審查均通過；正常 release 建置及套件雜湊核對亦已通過。

Windows 測試證據保留於 `target/windows-regression/shared-snapshot-*`；Linux 共用協定證據於 `target/native-scroll-rollout-20261018/`。ignored 證據不隨 git pull 搬移，應依正式測試入口重跑。

Owner 現有 Windows App 與工作連線保持原狀，不為測試終止或控制它。新套件交付到獨立目錄；更換 EXE 需先自行結束目前 App，私人 connections.toml 保留，既有遠端工作可重新 attach。

## 限制與接續

OS 中文候選選字、實際 AI 推論、Android APK／Keystore 本輪未驗收。原有 Host-only golden vector 失敗與全量 Rust 格式差異仍保留，不能宣稱完整 All suite 通過。舊的原生 mouse 測試結果是歷史紀錄，不能用來代表本次快照介面修復。

先讀 [文件入口](docs/INDEX.md)、[架構](docs/refarch/architecture.md)、[終端歷史](docs/refarch/terminal-history.md)、[Windows](docs/platforms/windows.md) 與 [回歸測試](docs/platforms/windows-regression.md)。保留私人 TOML／DPAPI 資料及未提交內容。Windows 專屬工作在 Windows；共用工作在 Linux，同步分支使用 ff-only，禁止覆蓋其他工作。

正常打包使用 PowerShell 7，輸出必須是新目錄：

```powershell
$env:CARGO_BUILD_JOBS = '1'
pwsh -NoProfile -File scripts/package-windows.ps1 -OutputPath dist/windows/new-package
```

本輪授權基本驗收後 commit／push 開發分支；不合 main、不 tag、不發布 APK。

## 修正版套件

artpc：`dist/windows/shared-snapshot-history-20261018`。正常 release 重建（無 SkipBuild／fixture feature），使用公開空白設定。EXE SHA-256：`dc8f0914e002e07f46ac9acf76c6b34529ddc01ff891d01773ba95087cbc0838`。已逐檔核對 manifest 的長度與 SHA；建置日誌：`target/windows-regression/shared-snapshot-package-clean.log`。更換既有安裝時只換 EXE 與說明檔，保留自己的 connections.toml；目前執行中的舊 App 需先關閉。
