# Windows 開發交接

本輪在 `feat/windows-mobile-parity` 接續既有桌面 UI 與原生 xterm 修正，基準為 `e249f203b14258cea2f55c1094d9147b68675c3f`。Windows 專屬開發與驗證在 artpc；共用連線搬移程式在 Linux 開發後同步。Owner 授權 AI 基本驗收後 commit／push 開發分支；本輪不合 main、不 tag、不發布 APK。

已完成手機非平台專屬功能同步：分類主畫面、全頁設定、四種 Theme、首頁與更新紀錄、Screens、每個工作 Shell 的桌面工具選擇。語音與模型留待下一階段。Windows 終端使用單一 xterm：normal buffer 保留 5000 行，tmux alternate 使用原生 mouse／copy-mode；移除舊文字覆蓋層與 snapshot 輪詢。

匯入／匯出現在包含 instance-mcp Screens，可只選 Screens 或混合 Host／Agent。密碼加密檔包含 token；公開檔不帶 token。Host-only 保留 v1，相容原格式；含 Screen 使用 v2。匯入先預覽重複項目，預設略過，明確選更新才改動；在目的 Windows 重新以 DPAPI 保存。

## 基本驗收結果

- Windows 前端型別、332 個 unit/component 測試、Vite build 通過。
- Windows debug fixture EXE 建置及 GUI Rust 24 個測試通過。
- 真 Windows WebView2 → 隔離官方 Runtime → 私有 tmux：首次 34px 上滾進 copy-mode 並上移兩行、回到底部、中文輸入與 Backspace、Bash exit 自動關閉分頁，五項通過。中文 IPC 到畫面約 40ms，Backspace 約 27ms。
- 真 Windows 原生選檔器：Screens-only／混合加密匯出、新設定匯入、DPAPI 重開、匯入 token 實際 MCP 驗證、重複略過／明確更新、零 Host 仍可匯出，全部通過。單一 xterm 的小幅滾輪、閱讀時輸出不跳回、回底部跟隨亦通過。
- 真 Linux tmux 兩 helper 交錯、舊 owner 清理與 Python 被終止的鎖恢復通過。獨立計畫與來源審查已通過。

原生證據：`target/windows-regression/run-f81eff14abbe4d6093059c7334006906/result.json`、`real-runtime-native-04/result.json`、`final-{typecheck,unit,build,debug-build,gui-tests}.log`。這些是 artpc ignored 本機證據，Git pull 後不會取得；可依正式測試入口重新執行。測試使用隔離設定與工作，未改正式 Runtime 或使用者連線。

## 已知限制

完整核心測試為 101 通過、1 個既有 Host-only golden vector 失敗；原始基準 codec 與現有 Host-only 輸出 byte 相同，既有 vector 未修改。全量 cargo fmt --check 仍有格式差異，沒有把完整 All suite 宣稱為通過。真 Codex 0.162.1 已在自有空工作目錄顯示 composer；測試等待條件仍要求舊版 footer，逾時後安全退出，Codex composer 中文／Backspace 未執行，未送推論 prompt。OS 中文候選選字、實際 AI 推論／長時間對話、Android APK／Keystore 此輪未驗收。

原生 tmux 捲動需要新版 helper READY 的 `terminalMouse` 能力；舊 helper 仍可輸入並提示更新。此次只驗證隔離候選 helper，未部署至正式受控端。每個可見 active 終端續租控制狀態，隱藏／切頁釋放；helper 的檔案鎖與 owner 檢查保護原 mouse／copy-mode／WheelUpPane 設定。

## 接續與重建

先讀 [文件入口](docs/INDEX.md)、[Windows](docs/platforms/windows.md)、[回歸測試](docs/platforms/windows-regression.md) 與 [連線搬移](docs/refarch/connection-transfer.md)。保留本機私人 TOML／DPAPI 憑證及未提交內容，不以 reset／clean 排錯。

在 Windows repo 根目錄使用 PowerShell 7：

```powershell
git pull --ff-only origin feat/windows-mobile-parity
npm.cmd --prefix apps/desktop run typecheck
npm.cmd --prefix apps/desktop test -- --run
$env:CARGO_BUILD_JOBS = '1'
cargo test -p art-term-gui --locked
pwsh -NoProfile -File scripts/package-windows.ps1 -OutputPath dist/windows/new-package
```

正常打包重新建置 release EXE，不能用 SkipBuild 代替來源驗證。公開包只用 `config/connections.example.toml`，不帶測試憑證或私人設定。不要同時執行多個 GUI／Cargo 工作。測試需互動式 Windows 桌面；SSH Session 0 的 WebView2 啟動逾時不代表功能測試通過或失敗。

## 本次交付

artpc 正常 release 打包通過（未使用 SkipBuild、未啟用 openab-fixture），套件位於 `dist/windows/native-xterm-screen-transfer-20261018`。EXE SHA-256：`528ee16f7afaea5360b664de80861c11a378ce90362ed9d8f0bef836a5d2e65e`。已逐一核對 manifest 的檔案長度與 SHA，connections.toml 與公開 example 完全一致。建置日誌：`target/windows-regression/final-package.log`。
