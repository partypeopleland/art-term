# 功能測試矩陣

這張表回答「目前哪些操作受到測試保護，以及改功能時要驗證什麼」。單元測試負責精確的失敗情境；原生 E2E 操作真正的 Windows EXE 或 Android APK，確認產品各層能一起運作；Linux 測試再檢查真正的官方 Runtime 與 helper。三者的通過條件不同，不能互相取代。

以下舊流程記錄屬於當時的 Runtime 截圖 helper 設計，不能代表直接 MCP 版本已通過。直接 MCP 新增驗收門檻列在下一節；每次執行仍需記錄實際 PASS／FAIL／BLOCKED／NOT_RUN。

2026-10-11 已分別驗證 Windows 四組原生流程、Android 六組流程與一組雙向跨平台搬移案例，以及 Linux 真 Runtime／helper、一次既有 MCP 擷取和 53 個 Host tests。下面列出已有案例的保護範圍；每次修改後，仍需以對應入口產出的結果、來源版本與清理狀態判定該次是否通過，不能沿用舊版本的通過紀錄。

## 直接 MCP 與獨立 Screens 的驗收門檻

下表列出本次應驗證的行為，不宣稱已執行；以當次結果與版本判定。固定 client 代替 AI 模型，但必須走正式產品 adapter、HTTP 與工具執行，不能 mock 產品路由。

| 範圍 | 必須驗證 |
|---|---|
| Screen 保存與遷移 | 原生加密 CRUD／重開、同 URL 保留 token、改 URL 必須重填、id／revision／context 拒絕混用；schema 5 啟動不改檔，首筆 Screen 前私有備份、有 Screen 寫 schema 6；Host CRUD／匯入不丟桌面 |
| MCP HTTP 傳輸 | 真 JSON／SSE、initialize／initialized／session、JSON-RPC id／error／isError、PNG／JPEG、尺寸／16 MiB／逾時；TLS、DNS pinning、HTTP Tailnet／loopback、拒 redirect／環境 proxy、session 清理 |
| App 原生截圖 | 直接 MCP screenshot，不經 Runtime PTY 或依賴 tools/list；切換／背景／關頁拒舊回覆；圖片來源 Screen id／revision 一致，定時／手動／停止保持終端 |
| 分頁與 UI | 真 APK／Keystore：Main Screens 清單與數量、齒輪 Screen 新增／編輯／刪除、每 live Shell 獨立選擇／切換、舊 desktop_transport 不控制入口、PreviewPanel 來源及工具狀態一致 |
| Adapter 工具行為 | 固定 client 經正式 stdio adapter→真測試 HTTP A／B 上游；工具探索保留 cursor，原名稱／參數與完整 content／isError／structuredContent 保留；A→B、不同比例圖片、兩 adapter 共用 A 且只切其中一個 |
| 選擇版本 | current_target／list 回來源，call 的 expected_target 不符拒絕；進行中 A 呼叫留 A、新呼叫用 B，不把舊結果顯示成 B；取消不宣稱撤銷上游副作用 |
| 真官方 Runtime configure | Rust 原生 configure→未修改官方 Runtime 管理 PTY→adapter→固定 client→A／B；raw／關 echo READY 握手，token 不在 replay／history／cmdline／environ／log；管理 Shell 刪除與名額恢復；renew 後仍可 configure／clear |
| 工具生命週期 | 無 adapter 不假報 ready、auth／timeout／isError、clear／EOF 清記憶體／socket、背景／晚到回應、App 重啟／takeover 不沿用未核對選擇；owner／0700／非 symlink 及同 UID 邊界 |
| CLI 契約 | 確定性檢查 Codex -c stdio command／args 不含 URL／token、timeout 75秒、其他 CLI 不猜語法、不對既有程式偷偷輸入；此項不等於真 Codex／AI 已測 |
| 回歸 | tmux 歷史按需讀頁、輸入／ACK／重連、固定滑動頂列／四主題／語音與鍵列、Host 搬移保留獨立 Screens、無私人預填 |

ARM64 手機、現場網路／MCP、實際 Codex CLI 與 AI 模型各自需要明確結果；固定 client E2E、命令契約或舊 helper 案例不替代它們。無授權不操作 Owner 手機或既有共享服務。

## 既有操作保護（歷史 helper 流程）

| 功能／風險 | Windows 原生 E2E | Android APK 原生 E2E | Linux／單元測試補強 |
|---|---|---|---|
| 連線新增、測試、改名、分類、刪除後重開 | W-CRUD：缺檔空清單、增改刪、重開保存 | A-CRUD：同操作並經 Android Keystore 保存 | TOML 唯一來源、revision／外部變更與並行衝突 |
| 編輯時沒有重填密碼，不應失去原憑證 | W-CRUD：空白憑證編輯後原認證可用 | A-CRUD：編輯後及重開後原認證可用 | DPAPI context 綁定、錯 schema／Host／origin／revision 拒絕 |
| 終端輸入與多個工作不串線 | W-TERM：真 xterm Unicode、resize、兩分頁隔離 | A-TERM：Unicode composer、觸控按鍵、resize、分頁隔離 | L-RUNTIME：真 shell Unicode／stty resize；stream unit驗byte／ACK／Gap |
| 從工作頁啟動已設定的 coding CLI | W-TERM：按啟動按鈕，核對送出的命令與含空格參數 | A-TERM：相同按鈕與命令檢查 | 只驗證命令組裝與傳送；fixture 不執行真的 coding CLI |
| 暫時斷線不重播命令，也不擅自接管工作 | W-TERM：原 token／游標恢復，不 renew、不重送輸入 | A-TERM：同 token／游標恢復及輸入不重播 | L-RUNTIME：官方 Runtime 原 grant 恢復、generation不變、不重播舊輸出 |
| 預覽立即出圖，定時／手動更新正常 | W-PREVIEW：大 PNG 真解碼、首圖／30秒／手動／停用 | A-PREVIEW：真 APK 圖片解碼及相同更新操作 | L-PREVIEW：真 helper→真 PTY 大圖；decoder/framing unit驗錯誤資料 |
| 截圖不能污染或刪除使用者工作 | W-PREVIEW：工作 Shell 無截圖命令，只刪自建暫時 Shell | A-PREVIEW：同隔離與清理檢查 | L-PREVIEW：兩個真 PTY 隔離；取消／晚到回覆 unit保護 |
| 手機切背景後不送出舊操作 | 不套用 Android 生命週期 | A-LIFECYCLE：真 Home／回前景，清未送內容、停止新截圖、保留原 binding | mobile輸入／visibility／取消與背景狀態 unit |
| 匯出／匯入與錯密碼不能毀掉設定 | W-TRANSFER：真 Windows 選檔器、加密／解鎖、錯密碼、duplicate | A-SAF：真 Android 系統選檔器、取消、加密／解鎖、duplicate | transfer加密、防篡改、原子套用、TTL／取消／late result unit |
| Windows 與 APK 互相搬移憑證 | X-TRANSFER：Windows匯出→APK匯入；APK匯出→Windows匯入 | 同一案例的 Android SAF 與 Keystore 端 | 新裝置建立自己的credential identity，重開後兩端原生認證成功 |
| 同簽章重裝保留設定與金鑰 | 由W-CRUD重開保存覆蓋，非installer升級測試 | A-UPDATE：同一debug APK `install -r` 後仍可認證 | 不等於所有歷史版遷移或release原簽章升級 |
| Linux Host 安裝、啟用桌面與失敗復原 | App側連線測試不能取代Host交易測試 | 同左 | L-HOST：installer／upgrade／desktop／helper既有案例；假systemd/Tailnet adapter |
| 現場 MCP 確實能擷取畫面 | App對本機fixture不能證明現場MCP | 同左 | L-LIVE-MCP：選配既有MCP的一次真截圖，只記大小/hash與當次結果 |

W- 前綴對應四組 Windows 原生流程。A- 前綴對應六組 APK 流程；X-TRANSFER 是跨裝置流程。L- 前綴區分真 Runtime、真 helper、Host fixture 與選配現場 MCP；每列內有多個斷言，不把斷言數當作獨立 E2E 數量。

另有 [APK UI 操作矩陣](../../scripts/android-regression/ui-matrix.md)與 [Linux runner](../../scripts/android-regression/ui_matrix.py)，細分介面／操作／狀態的 ID，重用上述 Android Suite。新增原生拖曳、兩種重連、語音設定切換、軟鍵盤顯示不變、取消／錯誤／邊界與最後分頁等 UI 情境；每次結果獨立記錄 PASS／FAIL／BLOCKED／NOT_RUN，測試存在不代表產品已通過。

## 哪些是真實環境

| 層級 | 真實部分 | 模擬或尚未涵蓋部分 |
|---|---|---|
| 前端／Rust unit與整合 | 產品狀態控制、核心、Windows原生DPAPI；部分真本機TLS/WSS | 控制時間、協定回覆與錯誤，不代表真App操作 |
| Windows原生E2E | EXE、WebView2、IPC、DPAPI、原生選檔器、Rust網路層 | Runtime由loopback協定fixture模擬，不執行遠端shell |
| APK原生E2E | 模擬器內Android系統、APK、WebView、IPC、Keystore、SAF、生命週期與Rust網路層 | x86_64 debug APK、固定硬體鍵盤／CDP文字輸入；不測軟鍵盤開關與組字。對端為同一協定fixture，不等於實體ARM64手機 |
| Linux整合 | 未修改的官方binary、真PTY、真preview/MCP helper、檔案權限與程序清理 | 預設MCP為有效圖片fixture；Host交易的systemd/Tailnet用adapter |
| 選配現場MCP | 既有服務的當次截圖擷取 | 不代表App到正式Tailnet/TLS的整條連線、長期穩定或全部桌面工具 |

Windows／APK 的 debug fixture CA 只限定測試建置，仍實際驗證 TLS；release 禁止該 feature。測試不修改 openab-pty，也不以真機已登入的私人連線當測試資料。

## 修改功能時怎麼使用

先從第一張表找修改範圍與相鄰功能，再由 AI 選擇對應 unit 與原生流程。修 bug 時盡可能先用案例重現，再修正產品；新增案例應攔得住該種退步，不能只增加測試數量。具體測試函式與 unit 檔案對照在 [Windows 回歸測試](../platforms/windows-regression.md)。

使用者可以直接貼給 AI：

```text
請先讀 docs/refarch/test-matrix.md，依這次功能修改找出受影響的測試。
沿用既有案例，必要時補回歸測試，再依 Windows、Android、Linux 的操作文件執行。
保留我的手機、連線設定與既有服務，只使用專用測試裝置和隔離資料。
完成後列出案例 ID、實際通過／失敗／未執行、受測版本，以及仍需實機確認的項目。
```

執行入口分別是 [Windows](../platforms/windows-regression.md)、[Android APK](../platforms/android-regression.md)、[Linux](../platforms/linux-regression.md)。目前以 Android APK 為主，Windows 功能驗收暫停；Linux 驗證官方 Runtime／adapter 與 Host 側，不能代替 APK 操作或手機驗收。

## 仍需另外驗收

ARM64 release原簽章更新、硬體Keystore、各廠牌中文IME／觸控手感、長時間Doze／OEM回收與手機實網，仍需相應實體手機。正式Runtime／Tailnet／TLS與現場桌面環境也需按部署範圍驗證。自動測試能提早攔住已建模的退步，不能保證所有環境與未設計情境都沒有問題。

## 附錄：案例實作

[Windows四流程](../../scripts/windows-regression/run.py)、[Android與跨裝置流程](../../scripts/android-regression/run.py)、[Linux流程](../../scripts/linux-regression/run.py)、[核心協定tests](../../crates/art-term-core/src/openab/tests.rs)、[憑證DPAPI tests](../../crates/art-term-gui/src/dpapi.rs)、[搬移tests](../../crates/art-term-core/src/openab/transfer_tests.rs)、[前端tests](../../apps/desktop/tests)。
