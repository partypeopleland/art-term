# Windows 接續建置與驗收

> 2026-10-18 Owner 修正體驗要求後，本分支已改為單一工作區：左側約三分之一是手機相同的直向清單、左下桌面預覽，右側為多分頁終端。Main／Connect 與額外「歷史」按鈕已移除，連線內容直接滾輪讀取舊頁，到底追尾。下文保留原始接力背景；其中舊版畫面與歷史面板操作，以 [Windows 現況](docs/platforms/windows.md)及[回歸指南](docs/platforms/windows-regression.md)為準。發布限制不變，仍不合 main、tag、發布或建 APK。

本文件是 `feat/windows-mobile-parity` 的臨時交接工單，受測來源為**此交接文件所在 commit**。請在另一台 Windows 電腦接續；本輪只 commit／push 開發分支，不 merge main、不 tag、不發布，不新增 release note 或建 APK。語音留下一階段。

## 來源與資料保護

先讀 `docs/INDEX.md`、`docs/platforms/windows.md`、`docs/platforms/windows-regression.md` 及本文件。確認目前 checkout、branch、HEAD、dirty／untracked，保留私人 TOML、DPAPI 憑證、既有 App 與工作。只在工作目錄乾淨或已安全保存自身變更時切換：

```powershell
git status --short --branch
git fetch origin
git switch feat/windows-mobile-parity
git pull --ff-only origin feat/windows-mobile-parity
git rev-parse HEAD
```

若分支不存在、已偏離或本機有變更，不用 reset／clean／force；先核對來源。記錄接到的 commit，之後測試及 EXE 必須來自同一來源；若修復後推新 commit，回報新的受測 commit。請保留 `scripts/android-regression/history_proxy_fixture.py`、`proxy_pressure.py` 等不屬此任務的未追蹤檔案。

## 本機已做與尚未做

開發環境是 Linux。本輪實際結果：

- `npm --prefix apps/desktop run typecheck`、`npm --prefix apps/desktop test`（29 檔／332 tests）、`npm --prefix apps/desktop run build` 通過，包含 Android 共用回歸、Windows 畫面與 9 項歷史面板測試。
- `cargo test -p art-term-core --locked instance_mcp` 3/3、`cargo test -p art-term-core --locked openab::history` 17/17 通過。
- Windows runner 五個 Python 檔 `py_compile` 通過；隔離 fixture smoke 兩次通過，驗證 JSON／SSE PNG、TLS／WSS、bootstrap、READY/configure/clear、resize、4004 與自有 threads/sockets 清理。
- 全 core suite 為 96 通過／1 既有失敗，Rust 格式檢查仍有基線差異，詳見下一節。沒有將這兩項寫成 PASS。

Linux 證據保存在開發機 ignored 的 `target/windows-parity/`，不隨 Git 交付；接手者需在 Windows 重新取得自己的 log，不能假定本機已存在這些結果。

本輪 **NOT_RUN：Windows MSVC／release EXE 建置、真 WebView2 GUI、DPAPI、原生選檔器、Windows 對官方 Runtime／現場 MCP 及使用者實測**。Linux DOM 或 fixture 通過不取代這些。Windows 歷史面板使用共用 HostHistoryWindow，不載 Android plugin；語音、手機滑動及 Android IME 不在驗收範圍。

## 已知基線檢查問題

本輪未修改 Rust／Cargo 來源，與基準 main `2b656ed` 一致。本機 `cargo fmt --all --check` 在既有 Rust 檔案報格式差異；`cargo test -p art-term-core --locked` 的 `openab::transfer::tests::public_strict_roundtrip_and_private_vector` 仍有既有 golden vector 失敗。前輪已在未修改的 main 重現該測試，本輪保留失敗斷言，沒有更動匯出格式或向量。

若 Windows 的 All 入口停在這些檢查，記錄原始結果、與接到的 main 基線比較，不能宣稱完整回歸通過或直接改預期值。可另外執行下列命令取得原生 E2E 結果，兩份結果分開回報；這不解除原失敗，也不授權發布。

```powershell
.\scripts\test-windows-regression.ps1 -Prepare -Suite E2E -EdgeDriverPath C:/tools/msedgedriver.exe
```

## Windows 工具與完整回歸

需要 Windows x64、PowerShell、Git、Rust MSVC、Visual Studio C++ Build Tools、Node/npm、Python 3.12 以上、WebView2 Runtime 與匹配 WebView2 的 Edge WebDriver。沿用既有工具，driver 不只對照一般 Edge 的版本；實際依賴說明見 Windows 回歸文件。

使用專用隔離資料，不拿私人連線當 fixture。先自行正常關閉既有 art-term 視窗，腳本不會代關。從 repository 根目錄執行：

```powershell
npm.cmd --prefix apps/desktop ci
.\scripts\test-windows-regression.ps1 -Prepare -EdgeDriverPath C:/tools/msedgedriver.exe
```

把 driver 改為實際路徑。Windows 新 runner 已適配 Main／設定、inline 工作選擇、新 th2 工作與直接 MCP fixture，**仍待 Windows 首次執行**；Linux fixture smoke 不代表真 Windows 成功。All 入口依序執行前端型別／測試、Rust 格式／workspace／GUI 檢查、debug fixture EXE 建置與真原生 E2E。保存 `target/windows-regression/checks-*/result.json`、原生 `run-*/result.json` 及各步 log；失敗先診斷，不放寬斷言／假造 IPC／清除使用者資料。只跑 Unit 或 E2E 必須明列缺少的另一層。

若新 UI／平台差異使案例失敗，修產品或相符 fixture，保留真 EXE／IPC／DPAPI／Rust 網路與原生 picker 邊界。單元 fixture 不執行真 AI 模型，不寫成真 Codex 已測。新版原生 runner 自動覆蓋四組 CRUD／終端／Screens／搬移；Screens 以不同的 2×2 PNG 完整 bytes 核對來源，這輪不代表大圖壓力測試。歷史面板的真 Host 分頁／ACK／keep／close、四主題與頁面實際布局／焦點、約 30 秒自動截圖、持續背景／晚回覆及真 AI broker 都另列人工 NOT_RUN，不能因 runner 成功而一併標 PASS。

## 建立這個分支的新 EXE 包

原生回歸完成後，以新的空輸出目錄重新建置 release：

```powershell
.\scripts\package-windows.ps1 -OutputPath ./dist/windows/parity-handoff-new
```

腳本重新安裝前端鎖檔依賴、建置前端及 release EXE。若目錄已存在且非空，換新名稱並保留舊包。**不要使用 `-SkipBuild` 或舊 EXE 冒充本分支建置**；release 不得帶 `openab-fixture`。預設包內只放公開空設定；需要測私有設定時另用自己的可寫設定路徑，不能把秘密提交或放公開交付。

核對包內 `art-term-gui.exe`、`connections.toml`、README、LICENSE、第三方說明及 `manifest.json`。記錄版本、EXE SHA256 與來源 commit；同版本字串不代表同一份來源。

## 原生人工驗收重點

用隔離 TOML 及允許測試的 Runtime／Screen；不得擅自重啟共用服務或使用者正在跑的 Shell。

- 固定品牌首頁、Main／Connect、全頁齒輪設定；Windows 版本及完整更新文件可讀。
- Computers／Agents／Screens 的數量、收合、本機狀態點；既有工作 attach／接管及 `＋` 新歷史工作。Host 管理的編輯／刪除／測試入口仍可達。
- 四種 Theme 覆蓋畫面、對話框、即時 xterm 及新分頁；設定／首頁開啟期間仍接收輸出，返回能 fit／focus。表單正常文字及快捷鍵不誤送到終端。
- Host／Screen 使用真 DPAPI 增改刪及重開；同 URL 空 token 編輯保留認證、改 URL 需重填；schema 5／6、私有備份、Host 匯入保留 Screens；搬移檔明示不含 Screens。原生匯出／匯入、取消、錯密碼都不毀資料。
- 每 live 分頁明確選 Screen、A／B 切換、停止預覽、取消工具及兩分頁隔離；圖片直接 MCP，舊回覆不覆蓋新來源，不依賴 Host 舊 desktop_transport。預覽成功不等於工具已接入。保持當前預覽不操作，觀察至少兩次自動更新及約 30 秒間隔；切 Main／設定／背景後應停止新截圖，返回恢復原先顯示狀態。
- 「歷史」按鈕的獨立彩色面板：先當前頁、「較早紀錄／較新紀錄／最新內容／返回終端」可用，向上每批最多 256 行、窗口最多 1024 行；ACK／keep／close 正確，開關面板不改 xterm 硬體鍵盤、resize 或原 Host mouse off。普通／舊 Shell 不假造歷史能力。
- Unicode、Ctrl+V／Ctrl+C、分頁快捷鍵、resize、多分頁不串線；暫時斷線沿原 token／游標恢復，不建立／renew／重播輸入。退出 Codex 返回 Bash；退出 Bash 結束工作、對應分頁關閉，讀頁 snapshot 不讓工作假存活。
- 正式 Runtime 新工作用已安裝的持久化工具 wrapper；Screen 選擇供該 Shell／coding CLI 使用，clear／Shell exit、CLI EOF 保留選擇等依 `docs/refarch/runtime-desktop.md` 驗證。缺 helper 或既有工作未載 broker 須清楚呈現，不能偷偷注入命令。真 AI／現場服務另需授權，不以固定 client 假稱通過。

## 回報及停止點

回報來源 branch／commit、工具版本、實際命令／exit code、結果與 log 路徑、release 包路徑／manifest／EXE hash，逐項區分 PASS／FAIL／BLOCKED／NOT_RUN。記錄仍需 Owner 驗收的項目，排除秘密、私人截圖與完整終端內容。

必要修復可提交並推送同一開發分支，保留新來源與重測範圍；不得 force push、合 main、tag、發 Release 或新增託管 CI。交給 Owner 在 Windows 使用後另行決定發布。
