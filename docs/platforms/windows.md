# Windows 平台

Windows 版同步 Android 的非平台專屬功能：分類主畫面、全頁設定、四種 Theme、版本與更新紀錄、獨立 Screens、工作 Shell 的 MCP 選擇與終端語音輸入。Windows 保留硬體鍵盤、xterm、滑鼠與多分頁；Android 原生閱讀器、觸控滑動與 IME 平台層不移植。

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

執行環境需要 Windows x64、Windows 10 1809 以上、WebView2 Runtime 與可達 Runtime／MCP 的網路；不需要 OpenSSH。EXE 尚無產品 Authenticode 簽章；應用程式內更新使用 Tauri updater 簽章（minisign），不是 Authenticode。交付需保留整包並核對 manifest／hash。

## 軟體更新

全站設定的「軟體更新」只在按下檢查時查詢 GitHub 最新 Release，不做啟動或背景自動檢查。畫面顯示目前與最新版本；確認後由 Rust 端重新檢查 `latest.json`，確認版本仍是使用者看到的那一版，下載 `art-term-<version>-windows-x64-setup.exe` 並以 Tauri updater 驗證 minisign signature，通過後才啟動 NSIS 安裝程式並由其重啟 App。檢查失敗、斷線或簽章驗證失敗都不會動到現有安裝；簽章公鑰寫在 `tauri.conf.json` 的 `plugins.updater.pubkey`，私鑰只存 repository secret，見[發佈流程](releases.md)。

NSIS 安裝會固定在執行中 EXE 的同目錄（portable ZIP 與已安裝版本皆同），因此 `connections.toml`、DPAPI 憑證與其他同目錄檔案留在原處，不會被搬到 `%LOCALAPPDATA%\Programs` 另建安裝目錄；發佈包內只附 `connections.example.toml` 範本，不會覆寫使用者的 `connections.toml`。

## 畫面與平台操作

固定頂列提供品牌首頁與全站齒輪；設定／首頁可返回工作區。工作區左欄預設固定 360px，全螢幕時不隨視窗比例放大；可拖曳左右分隔線，或聚焦分隔線用方向鍵調寬。左欄以手機相同的直向 Computers／Agents／Screens 分類顯示數量、收合及本機連線狀態點，左下是目前分頁的桌面預覽；右側是多分頁連線與終端內容。預覽上方分隔線可拖曳調高，圖片等比例縮放、置中，以黑底補足空間。Shell 名稱按鈕明確 attach／接管，`＋` 建立新的歷史工作；不從分類推斷工具能力。

點桌面預覽可開啟浮動視窗，拖曳標題移動，拖曳四邊或角落調整大小。寬高不小於開啟時的預設尺寸，且保持在應用程式內；應用程式暫時小於預設尺寸時，以可用空間為限。滑鼠左鍵單擊浮動視窗可切換填滿應用程式內部，再次單擊恢復原尺寸與位置，不會切換作業系統全螢幕。拖曳、圖片縮放／平移及關閉不會觸發切換。

齒輪開全頁設定，左側分類為外觀、連線，右側只呈現所選內容，與品牌首頁使用一致的側欄及內容對齊。設定沿用手機共用元件：四主題即時套用、新增 Host／Agent、Host 與 Screens 匯入／匯出、既有 Computer／Agent／Screen 在同一列表編輯／測試／刪除及同類型拖曳排序；不另開桌面專屬管理燈箱。離開分類會清除表單草稿與憑證；系統選檔器開啟期間暫停頁面切換。每個 live Shell 可透過固定 Screen 圖示獨立選擇、切換或取消工具；選擇後立即關閉選單，CLI 可繼續操作，預覽區顯示載入狀態直到收到圖片或錯誤，選定名稱顯示於預覽標題。來源與生命週期見[桌面](../refarch/desktop.md)。

設定預設放 EXE 旁，或以 `art-term-gui.exe --config C:/art-term-private/connections.toml` 指定可寫路徑。缺檔正常顯示空清單，不嵌入私人預設資料。沒有 Screen 保存 schema 5，有 Screen 保存 schema 6；首次桌面保存前私有備份與降版方式見[設定](../refarch/connections.md)。

切頁用 Alt+[／Alt+] 或 Ctrl+PageUp／PageDown。終端聚焦時 Alt+C 複製選取文字、Alt+V 貼上剪貼簿純文字；滑鼠反白放開後也會自動複製並短暫顯示「已複製」。Ctrl+C 一律交給遠端作中斷，Ctrl+V 不作為本功能的貼上鍵。Alt+C／Alt+V 因此不再傳到遠端作 Meta 快捷鍵。貼上需授權桌面 App 讀取本機文字剪貼簿；Android 仍只有寫入權限。

遠端啟用 bracketed paste 時，多行文字透過其框架送入，不額外送 Enter。未啟用時，對每個換行先送 Readline 的 Ctrl+V 字面插入鍵，再送原換行；已在 Bash/Readline 驗證貼上當下不執行，仍需由使用者另按 Enter。這是 Shell 專用相容方法，其他遠端程式可能不支援相同按鍵，不能保證都不執行。表單輸入不被終端快捷鍵攔截，關頁／關窗只做本機 detach。中文輸入法組字期間，候選視窗不會把整個終端畫面捲動，過長的組字內容在終端內自行換行。點終端 HTTP(S) 網址會開啟系統預設瀏覽器；即時畫面與歷史均可使用，拖曳選取不會開啟網址。終端在設定／首頁遮蓋時仍繼續處理輸出，回工作區後重新 fit／focus，不重開連線。

終端下方的語音列提供與 Android 相同的點按／按住錄音、狀態、取消與轉寫顯示。辨識由 Windows SAPI 裝置端 dictation 提供，預設 `zh-TW`，需要使用者已安裝對應 Windows 語音辨識語言與可用麥克風；缺少語言、麥克風或辨識引擎時顯示原因並停用語音，鍵盤仍可輸入。錄音開始鎖定目前分頁、焦點窗格與連線世代；切換分頁／窗格、關閉、斷線、重連或 App 進背景會取消並丟棄晚到結果。辨識文字清理控制字元後先送文字，約 150 ms 後送 Enter；文字已送出後目標失效則略過 Enter。方案取捨與實機驗證狀態見 [BDR-003](../bdr/BDR-003-windows-voice-input.md)。

Windows 的即時畫面與歷史使用同一個 xterm 視窗。上捲時，受控端沿原連線呈現較早內容；向下捲到底便自然接續最新輸出，不切換閱讀畫面、不改變字型或亮度，也沒有「返回即時」按鈕。文字選取、複製、連結與全站主題都由原終端處理。

live 終端可用窗格右鍵選單或鍵盤快捷鍵向右、向下分割畫面（右鍵選單在未分割的單一窗格上同樣可用）。預設快捷鍵為 Shift+Alt+=（向右，`+` 即 Shift+=，比對以按鍵位置為準）、Shift+Alt+-（向下）、Shift+Alt+X（關閉目前焦點窗格），可在全域設定「快捷鍵」頁重新指定。每個分割窗格是獨立連線：按下分割會直接在同一台主機建立一條新連線（等同左側主機列的「+」），不再出現未連線窗格或連線選擇介面；各窗格有自己的 session、鍵盤焦點與輸入，點窗格切換焦點、拖曳分隔線調整大小、右鍵選單可再分割或關閉。Ctrl+Alt+方向鍵依顯示順序循環切換連線窗格焦點，Ctrl+Shift+W 關閉焦點窗格。明確關閉窗格會先刪除該窗格對應的遠端 Shell，成功後才移除窗格；刪除失敗會保留窗格並提示重試。若仍有其他連線窗格存活，原始窗格關閉或其 Shell 結束時由第一個存活窗格接手頂部分頁，不會連坐關閉其他窗格；全部窗格結束才關閉整個分頁。新增或刪除 Shell 後，左側連線清單會自動重新整理。分頁的關閉與背景斷線仍採本機 detach，不刪除遠端 Shell。

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
