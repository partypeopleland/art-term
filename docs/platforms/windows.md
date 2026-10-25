# Windows 平台

Windows 版同步 Android 的非平台專屬功能：分類主畫面、全頁設定、四種 Theme、版本與更新紀錄、獨立 Screens 及工作 Shell 的 MCP 選擇。Windows 保留硬體鍵盤、xterm、滑鼠與多分頁；語音留待下一階段，Android 原生閱讀器、觸控滑動與 IME 平台層不移植。

Windows 專屬開發、建置與 GUI 檢查在 Windows 電腦執行，共用功能沿用相同的 Rust 核心與受控端契約。每次建置與驗收須核對來源提交、套件 manifest 與 hash。真 Codex 與作業系統中文候選選字仍須分別有實際驗證，不能由型別、DOM 或共用核心測試推定。

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

`connections.toml` 是唯一設定來源。Host 與 Screens 是獨立集合；Windows DPAPI 保護管理憑證及 Screen token。明確選 Screen 後原生層直接向 MCP 取圖，不走舊截圖 Shell；後端保留的舊 Runtime 截圖相容路徑不代表新版 UI 已經驗收。Host 與 Screens 可分別或混合[匯出／匯入](../refarch/connection-transfer.md)，Screen token 僅隨密碼加密檔搬移；不能把另一 Windows 帳號的 DPAPI 密文直接當成可用憑證。

執行環境需要 Windows x64、Windows 10 1809 以上、WebView2 Runtime 與可達 Runtime／MCP 的網路；不需要 OpenSSH。EXE 尚無產品 Authenticode 簽章或自動更新，交付需保留整包並核對 manifest／hash。

## 畫面與平台操作

固定頂列提供品牌首頁與全站齒輪；設定／首頁可返回工作區。工作區左欄預設固定 200px，全螢幕時不隨視窗比例放大；可拖曳左右分隔線，或聚焦分隔線用方向鍵調寬。左欄以手機相同的直向 Computers／Agents／Screens 分類顯示數量、收合及本機連線狀態點，左下是目前分頁的桌面預覽；右側是多分頁連線與終端內容。預覽上方分隔線可拖曳調高，圖片等比例縮放、置中，以黑底補足空間。Shell 名稱按鈕明確 attach／接管，`＋` 建立新的歷史工作；不從分類推斷工具能力。

點桌面預覽可開啟浮動視窗，拖曳標題移動，拖曳四邊或角落調整大小。寬高不小於開啟時的預設尺寸，且保持在應用程式內；應用程式暫時小於預設尺寸時，以可用空間為限。滑鼠左鍵單擊浮動視窗可切換填滿應用程式內部，再次單擊恢復原尺寸與位置，不會切換作業系統全螢幕。拖曳、圖片縮放／平移及關閉不會觸發切換。

齒輪開全頁設定，左側分類為外觀、連線、畫面，右側只呈現所選內容，與品牌首頁使用一致的側欄及內容對齊。設定沿用手機共用元件：四主題即時套用、新增 Host／Agent、Host 與 Screens 匯入／匯出、既有連線直接編輯／測試／刪除，以及獨立 Screen 設定；不另開桌面專屬管理燈箱。離開分類會清除表單草稿與憑證；系統選檔器開啟期間暫停頁面切換。每個 live Shell 可透過固定 Screen 圖示獨立選擇、切換或取消工具；選擇後立即關閉選單，CLI 可繼續操作，預覽區顯示載入狀態直到收到圖片或錯誤，選定名稱顯示於預覽標題。來源與生命週期見[桌面](../refarch/desktop.md)。

設定預設放 EXE 旁，或以 `art-term-gui.exe --config C:/art-term-private/connections.toml` 指定可寫路徑。缺檔正常顯示空清單，不嵌入私人預設資料。沒有 Screen 保存 schema 5，有 Screen 保存 schema 6；首次桌面保存前私有備份與降版方式見[設定](../refarch/connections.md)。

切頁用 Alt+[／Alt+] 或 Ctrl+PageUp／PageDown；Ctrl+V 遵守 bracketed-paste，Ctrl+C 由遠端解讀。表單輸入不被終端快捷鍵攔截，關頁／關窗只做本機 detach。點終端 HTTP(S) 網址會開啟系統預設瀏覽器；即時畫面與歷史均可使用，拖曳選取不會開啟網址。終端在設定／首頁遮蓋時仍繼續處理輸出，回工作區後重新 fit／focus，不重開連線。

Windows 的即時畫面與歷史使用同一個 xterm 視窗。上捲時，受控端沿原連線呈現較早內容；向下捲到底便自然接續最新輸出，不切換閱讀畫面、不改變字型或亮度，也沒有「返回即時」按鈕。文字選取、複製、連結與全站主題都由原終端處理。

原生連線層統一處理閱讀中的輸入：打字、貼上、快捷鍵與 coding CLI 啟動會先回到即時位置，再交給 CLI。新版需要同一通用歷史 helper 宣告原生 viewport 能力；舊 helper 仍可供手機快照使用，但不能讓新版桌面假裝已能原生捲動。升級既有 helper 不需要更換 Runtime、重啟 Pod 或重新建立仍活著的 tmux 工作，也不增加 Windows 專用安裝項。細節與相容性見[終端歷史](../refarch/terminal-history.md)。

## 工具鏈與新建置

第一次 clone／fork 或自訂名稱／簽章時，先讀[本地建置](local-build.md)。Windows 建置需要 PowerShell、Git、Rust MSVC、Visual Studio C++ Build Tools、Node/npm；原生 E2E 另需 Python 3.12、WebView2 與匹配的 Edge WebDriver。

從 Windows 的 repository 根目錄執行：

```powershell
npm.cmd --prefix apps/client ci
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

檢查結果、套件來源與未驗證項目記錄於對應 Issue。進行中的臨時交接檔在驗收並合併後移除，持續適用的操作與限制整理到正式主題文件。使用者實機回饋與 AI 基本檢查分開記錄；合併與正式發布依各自門檻處理，不能把 AI 基本檢查寫成使用者已驗收。

## 附錄：實作與參考

[Windows App](../../apps/client/src/desktop/App.vue)、[打包入口](../../scripts/package-windows.ps1)、[回歸入口](../../scripts/test-windows-regression.ps1)、[工作生命週期](../refarch/sessions.md)。
