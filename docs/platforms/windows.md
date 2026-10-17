# Windows 平台

Windows 版同步 Android 的非平台專屬功能：分類主畫面、全頁設定、四種 Theme、版本與更新紀錄、獨立 Screens 及工作 Shell 的 MCP 選擇。Windows 保留硬體鍵盤、xterm、滑鼠與多分頁；語音留待下一階段，Android 原生閱讀器、觸控滑動與 IME 平台層不移植。

本輪在 Linux 開發及檢查，程式與臨時交接文件推到 `feat/windows-mobile-parity`，由另一台 Windows 電腦的 AI 接續建置與原生驗收。Linux 的型別、DOM、共用核心測試不能替代 Windows EXE、WebView2、DPAPI 或真 Runtime 操作。通過接續驗收前不合 main、不 tag、不發布。

```text
Windows App
  shared Dashboard / settings / homepage / themes / Screen picker
             |
       Tauri IPC -> Rust core -- HTTPS / WSS --> Runtime work Shell
             |                                tmux history + tools broker
             +-- native MCP screenshot ----------------> instance-mcp
             |
       local connections.toml + Windows DPAPI
```

`connections.toml` 是唯一設定來源。Host 與 Screens 是獨立集合；Windows DPAPI 保護管理憑證及 Screen token。明確選 Screen 後原生層直接向 MCP 取圖，不走舊截圖 Shell；後端保留的舊 Runtime 截圖相容路徑不代表新版 UI 已經驗收。Host 搬移使用[匯出／匯入](../refarch/connection-transfer.md)，目前不含 Screens；不能把另一 Windows 帳號的 DPAPI 密文直接當成可用憑證。

執行環境需要 Windows x64、Windows 10 1809 以上、WebView2 Runtime 與可達 Runtime／MCP 的網路；不需要 OpenSSH。EXE 尚無產品 Authenticode 簽章或自動更新，交付需保留整包並核對 manifest／hash。

## 畫面與平台操作

固定頂列提供品牌首頁與全站齒輪；設定／首頁可返回工作區。工作區左欄預設固定 200px，全螢幕時不隨視窗比例放大；可拖曳左右分隔線，或聚焦分隔線用方向鍵調寬。左欄以手機相同的直向 Computers／Agents／Screens 分類顯示數量、收合及本機連線狀態點，左下是目前分頁的桌面預覽；右側是多分頁連線與終端內容。預覽上方分隔線可拖曳調高，圖片等比例縮放、置中，以黑底補足空間。Shell 名稱按鈕明確 attach／接管，`＋` 建立新的歷史工作；不從分類推斷工具能力。

齒輪開全頁設定，左側分類為外觀、連線、畫面，右側只呈現所選內容，與品牌首頁使用一致的側欄及內容對齊。設定沿用手機共用元件：四主題即時套用、新增 Host／Agent、Host 匯入／匯出、既有連線直接編輯／測試／刪除，以及獨立 Screen 設定；不另開桌面專屬管理燈箱。離開分類會清除表單草稿與憑證；系統選檔器開啟期間暫停頁面切換。每個 live Shell 可透過固定 Screen 圖示獨立選擇、切換或取消工具；選擇後立即關閉選單，CLI 可繼續操作，預覽區顯示載入狀態直到收到圖片或錯誤，選定名稱顯示於預覽標題。來源與生命週期見[桌面](../refarch/desktop.md)。

設定預設放 EXE 旁，或以 `art-term-gui.exe --config C:/art-term-private/connections.toml` 指定可寫路徑。缺檔正常顯示空清單，不嵌入私人預設資料。沒有 Screen 保存 schema 5，有 Screen 保存 schema 6；首次桌面保存前私有備份與降版方式見[設定](../refarch/connections.md)。

切頁用 Alt+[／Alt+] 或 Ctrl+PageUp／PageDown；Ctrl+V 遵守 bracketed-paste，Ctrl+C 由遠端解讀。表單輸入不被終端快捷鍵攔截，關頁／關窗只做本機 detach。點終端 HTTP(S) 連結複製到剪貼簿。終端在設定／首頁遮蓋時仍繼續處理輸出，回工作區後重新 fit／focus，不重開連線。

Windows 在連線內容區直接用滑鼠滾輪閱讀，無需開啟另一面板。文字隨可用寬度自動換行，不出現水平捲軸；換行、拖曳寬度與補頁都以實際行識別及像素偏移保留閱讀位置。共用 HostHistoryWindow 先讀當前頁，Windows 再背景預載最多三批舊頁，每批最多 256 行；快取綁定同一快照，捲動時直接取用，不改顯示中的內容或追尾狀態。顯示窗口最多 1024 行、預載快取最多 768 行。新輸出不拉走閱讀點，捲到底部才恢復追尾。畫面完成後 ACK、閱讀中 keep、關閉或替換 session 時 close；設定／首頁與背景暫停新讀取並保留位置；切換分頁釋放 reader，切回重新讀目前頁，與手機一致。原 xterm 持續解析與收發、保留硬體鍵盤／resize，點一般內容回到輸入焦點；不改 Host mouse off、不載入 Android plugin。不支援主機歷史時仍可操作原終端。普通／舊 Shell 的可用界線、退出 Bash 結束工作及私有 wrapper 準備見[終端歷史](../refarch/terminal-history.md)。

## 工具鏈與新建置

第一次 clone／fork 或自訂名稱／簽章時，先讀[本地建置](local-build.md)。Windows 建置需要 PowerShell、Git、Rust MSVC、Visual Studio C++ Build Tools、Node/npm；原生 E2E 另需 Python 3.12、WebView2 與匹配的 Edge WebDriver。

從 Windows 的 repository 根目錄執行：

```powershell
npm.cmd --prefix apps/desktop ci
.\scripts\test-windows-regression.ps1 -Prepare -EdgeDriverPath C:/tools/msedgedriver.exe
.\scripts\package-windows.ps1
```

driver 路徑換成實際位置。回歸入口的 suite、失敗判定及隔離資料見[Windows 回歸測試](windows-regression.md)。打包腳本重新建置前端與 release EXE，預設輸出 `dist/windows/art-term-<目前版本>`；目錄必須不存在或為空，不覆蓋舊交付。需要明確輸出位置時：

```powershell
.\scripts\package-windows.ps1 -OutputPath ./dist/windows/parity-handoff-new
```

包內有 EXE、公開空 TOML、README、LICENSE、第三方說明與 hash manifest。私有 TOML 可用 `-ConfigPath C:/art-term-private/connections.toml` 旁置，不嵌入 binary；公開交付不含私人憑證。`-SkipBuild` 只沿用既有 EXE，不能代表這個分支的新建置或新功能已通過。腳本保留 Rust path remap，避免內嵌建置者路徑。

## 原生驗收與交接

先核對交接文件所在 commit 與分支、保留私人資料，再用隔離 TOML／WebView profile 操作真 EXE。至少驗證 Host／Screen DPAPI CRUD、重開、匯入保留 Screens、每分頁 Screen 選擇／切換／clear、首頁／主題／設定、硬體鍵盤與 Unicode、resize、直接滾輪歷史、Bash exit 關頁、原 token 恢復及不重播輸入。release 建置不得啟用 `openab-fixture`。

本機 fixture 能保護真 IPC、DPAPI 及畫面整合，仍不能代替正式 Runtime／Tailnet／TLS、現場 MCP 或真 AI CLI。接續 AI 應回報 source commit、工具版本、命令與 exit code、result／log 路徑、EXE manifest／hash，明確分開 PASS／FAIL／NOT_RUN。保留既有 App、連線及遠端工作，不以清除資料排錯。

本輪只交付開發分支及臨時 `WINDOWS_HANDOFF.md`，不建立新 release note、不建 APK、不合 main 或發布。後續發布由 Owner 在 Windows 實測後另行決定。

## 附錄：實作與參考

[Windows App](../../apps/desktop/src/App.vue)、[打包入口](../../scripts/package-windows.ps1)、[回歸入口](../../scripts/test-windows-regression.ps1)、[工作生命週期](../refarch/sessions.md)。
