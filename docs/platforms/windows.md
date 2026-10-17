# Windows 平台

Windows 版把遠端 Host 的 Shell 與桌面預覽放進本機 Tauri 視窗。Vue／xterm 負責畫面與輸入，Rust 共用核心負責連線、認證與 session 狀態；遠端程序仍在 Host 執行。這一頁說明如何從原始碼取得可執行交付，以及哪些結果仍須用真實 GUI 確認。

```text
Windows PC                                      Remote Host
+--------------------------+                    +------------------+
| Tauri GUI                |  HTTPS / WSS       | openab-pty       |
| Vue / xterm -> Rust core | -----------------> | Shell sessions   |
|                          |                    | Screenshot Shell |
| connections.toml         |                    +------------------+
| DPAPI-protected secrets  |
+--------------------------+
```

`connections.toml` 是這台裝置的唯一連線設定來源；Windows DPAPI 保護其中的管理憑證。終端資料透過 HTTPS／WSS 傳到 Runtime，新版共用桌面路徑改為原生層直接連獨立 Screen 的 instance-mcp，後端仍為未選 Screen 的 Windows 舊分頁保留 Runtime 專用截圖 Shell 相容流程；Windows 功能驗收暫停，不將 Android 改版當成 Windows 已驗收。搬移整個交付資料夾可保留程式與設定，但不能把另一個 Windows 帳號的 DPAPI 密文直接當成可用憑證；跨裝置 Host 請使用[連線匯出／匯入](../refarch/connection-transfer.md)，目前不含獨立 Screens。

執行環境需要 Windows x64、Windows 10 1809 以上、WebView2 Runtime 與可達 Runtime 的網路；原生連線不需要 OpenSSH。EXE 尚無產品 Authenticode 簽章或自動更新，交付時應一起搬移資料夾並核對版本與 hash。

## 平台操作差異

預設設定在 EXE 旁，或以 `art-term-gui.exe --config C:/art-term-private/connections.toml` 明示指定；路徑必須可寫。唯一來源與手動改檔規則見 [connections](../refarch/connections.md)。檔案不存在仍正常開空清單，可新增或匯入。

切頁使用 Alt+[／Alt+] 或 Ctrl+PageUp／PageDown；Ctrl+V 遵守 bracketed-paste，Ctrl+C 由遠端 Shell 解讀；關頁／關窗本機 detach。點終端 HTTP(S) 連結複製到本機剪貼簿。滾輪看歷史，互動程式可接收有界滑鼠回報。Shell 選擇與重連見 [sessions](../refarch/sessions.md)，桌面與 CLI 能力見 [desktop](../refarch/desktop.md)／[connections](../refarch/connections.md)。

## 工具鏈與建置

第一次 clone／fork、只想打包，或要更換名稱與圖示時，先依[本地建置與自訂版本](local-build.md)完成準備；本節補充開發檢查與平台驗收。

要請 AI 驗證功能，準備本節開發工具，以及 Python、WebView2 與版本相符的 Edge WebDriver，再直接貼上：

```text
請讀取本專案 docs/platforms/windows-regression.md，確認 Windows 測試依賴，
依這次修改找出相關功能與既有測試，補足必要的回歸案例，並在本機執行驗證。
保留我的連線資料與既有 App，只使用文件指定的隔離測試環境。
完成後告訴我測了什麼、結果，以及仍需實機驗證的部分。
```

AI 的測試分層、功能對照與操作細節見 [Windows 回歸測試](windows-regression.md)。

建置需要 PowerShell、Git、Rust MSVC、Visual Studio C++ Build Tools、Node/npm。前端檢查確認畫面程式可用，Rust 測試檢查共用核心，最後封裝把兩者組成同一份 Windows 交付。一般單元測試與 fixture（隔離的模擬環境）不需要私人 Host；它們不能驗證正式 Host 的 TLS 或真實網路。

```text
Vue / TypeScript ----> frontend checks / build --+
                                                +--> package --> dist/windows/
Rust core / Tauri ---> tests / GUI check --------+                 EXE + TOML + manifest
```

從 repository 根目錄依序執行：

```powershell
npm.cmd --prefix apps/desktop ci
npm.cmd --prefix apps/desktop run typecheck
npm.cmd --prefix apps/desktop test
npm.cmd --prefix apps/desktop run build
cargo fmt --all --check
cargo test --workspace --locked
cargo check -p art-term-gui --locked
.\scripts\package-windows.ps1
```

打包預設公開空設定，輸出 `dist/windows/art-term-0.1.23`；輸出需為新建或空目錄，保留既有交付。自訂交付 TOML：

```powershell
.\scripts\package-windows.ps1 -ConfigPath C:/art-term-private/connections.toml -OutputPath ./dist/windows/private-test-0.1.23
```

包內有 EXE、設定、README、LICENSE、第三方說明及 hash manifest。`ConfigPath` 只決定旁置 TOML，不嵌入 binary；私有 bootstrap 已退役，啟用會拒絕。`-SkipBuild` 沿用 binary，需另核對來源，不能代表新建置。腳本使用 Rust path remap 避免內嵌建置者 home／專案路徑。

## 驗證與交接

驗證應圍繞本次改動會影響的流程。設定變更要確認 HostStore 的 revision（保存版本）與 credential context（憑證所屬連線）仍相符，錯誤訊息不洩露秘密；連線變更要確認 TLS、原 attach token 與 binding。binding 是本機分頁對既定 Host／Shell 的關聯，重連沿用原 token，驗證失敗時停止而不改接同名工作。

終端改動還需確認有界 queue 與輸出 cursor：資料解析成功後才向本機核心送 ACK（已處理輸出的確認），收到 Gap（輸出歷史已缺失）時明確處理缺口。只有使用者選取既有工作才呼叫官方 renew；自動恢復使用原 token 與 since 游標，有期限與次數邊界，不重播先前輸入，過期 callback 也不能修改新連線。完整契約見 [session 與重連](../refarch/sessions.md)。`openab-fixture` 僅限 debug 隔離測試；release 禁止該 feature，不能用它略過正式 TLS。

可執行版驗收確認新增／編輯／刪除／測試、create/takeover、Unicode／resize、detach、renew、TTL、replacement、圖片及網路恢復。保存改動另驗證缺檔／空白／空清單、新增／匯入與重開；搬移契約見 [transfer](../refarch/connection-transfer.md)。核對受測 source manifest 與 binary hash；source／fixture 結果不替代真 GUI／正式 Host。

Windows 通過後才進 [Android](android.md)，兩平台必要檢查與 Owner 驗收後才合 main。簽章／金鑰位置見 [credential storage](../refarch/credential-storage.md)。

## 附錄：實作與參考

[scripts/package-windows.ps1](../../scripts/package-windows.ps1)、[crates/art-term-gui/src/main.rs](../../crates/art-term-gui/src/main.rs)。
