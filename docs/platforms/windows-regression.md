# Windows 回歸測試：AI 執行指南

這份文件供 AI 在修改功能後選擇、補齊並執行測試，確認相鄰功能仍可使用。使用者只需準備 [Windows 開發工具](windows.md)、Python 3.12 以上、WebView2 Runtime，以及版本相符的 Microsoft Edge WebDriver；測試套件與執行細節由 AI 處理。受控端不需要為此安裝 Rust 或 Cargo，也不需要提供私人 Host。

給 AI 的工作範圍應包含要修改的功能與預期行為。AI 先讀本頁，從下方功能對照找既有保護，再決定需要增加哪個案例；不要以增加測試數量代替驗證具體行為。

## 各層測到什麼

跨平台操作案例及哪些項目仍需實機，統一見[功能測試矩陣](../refarch/test-matrix.md)。Android 真 APK 與 Linux 真官方 Runtime 分別依 [APK E2E](android-regression.md)、[Linux 驗收](linux-regression.md)執行；本頁保留 Windows／unit 操作細節。

```text
Frontend unit/component tests       Rust unit/integration tests
Vue/controller + test transport     real DPAPI / local TLS-WSS fixtures
                \                     /
                 +--- Windows regression entry ---+
                                                  |
                                     Native Windows E2E
                                     real EXE + WebView2
                                              |
                                         real Tauri IPC
                                              |
                                     DPAPI + Rust HTTPS/WSS
                                              |
                                     loopback protocol fixture
```

前端測試使用 Vitest／jsdom，控制時間、連線回覆與輸入事件，驗證畫面及狀態控制器。Rust 測試驗證保存、加密、協定及有界佇列；其中 Windows DPAPI 使用目前使用者的原生保護，TLS／WSS 測試使用本機伺服器。這些測試能精確重現錯誤條件，但本身不等於使用者已操作真 App。

原生端到端測試（E2E）啟動實際的 Windows EXE，透過 WebView2 操作畫面，保留真 Tauri IPC、Rust 網路層、DPAPI 與原生檔案選擇器。對端是模擬官方協定的本機服務，不啟動真正的遠端 Shell；它記錄收到的操作與資料，以核對重送、建立、續期及刪除行為。觀測程式只記錄真呼叫結果，不替換 IPC 回覆。

因此本機 E2E 通過代表 App 各層整合正常，不能單獨證明官方 Runtime 實際相容、Linux helper／instance-mcp、正式網路與 TLS，或 Android Keystore／觸控／背景正常。會影響上述邊界的功能仍須依 [Windows](windows.md)、[Android](android.md) 與 [Host 維護](linux-host-maintenance.md) 完成相應驗收。

## 準備與執行

以下指令均在 Windows 的專案根目錄執行。需要已安裝 Rust MSVC、C++ Build Tools、Node/npm、PowerShell 與 Python；正常使用 App 的人不需要這些開發工具。首次安裝相關工具依其官方文件，由 AI 先檢查現有版本並沿用。

Microsoft Edge WebDriver 必須匹配 App 使用的 **WebView2 Runtime**，不是只核對平常瀏覽的 Edge。AI 依 Microsoft 官方文件取得相符的 Windows x64 driver，放在專案內 ignored 工具目錄，或以 `-EdgeDriverPath` 明確指定。腳本不自動升級瀏覽器、不更改系統憑證。需要一個可互動的 Windows 桌面；執行前關閉自己開啟的 art-term 視窗，測試不會代關既有 App。

首次安裝專案測試依賴並執行全部檢查：

```powershell
.\scripts\test-windows-regression.ps1 -Prepare -EdgeDriverPath C:/tools/msedgedriver.exe
```

`C:/tools/msedgedriver.exe` 是範例，請改用實際下載路徑。`-Prepare` 執行前端鎖檔安裝，並在 `target/windows-regression/venv` 建立 Python 環境、依 `scripts/windows-regression/requirements.txt` 安裝固定版本套件。預設 driver 路徑是 `target/gui-tools/edge/msedgedriver.exe`；沒有此檔時必須指定自己的路徑。

依賴備妥後，日常完整回歸：

```powershell
.\scripts\test-windows-regression.ps1 -EdgeDriverPath C:/tools/msedgedriver.exe
```

預設 `-Suite All` 依序執行前端型別與測試、Rust 格式／workspace 測試／GUI 編譯檢查，接著建置前端與 debug fixture EXE，再執行原生 E2E。任何一步失敗就以非零結束，不會把後續未執行項目當成通過。

調查有界修改時可單獨執行：

```powershell
.\scripts\test-windows-regression.ps1 -Suite Unit
.\scripts\test-windows-regression.ps1 -Suite E2E -EdgeDriverPath C:/tools/msedgedriver.exe
```

`Unit` 包含前端與 Rust 的單元／元件／協定整合檢查；`E2E` 仍會重新確認前端及 EXE 建置，不沿用來源不明的舊產物。只跑其中一層時，報告必須寫明範圍，不能寫成完整回歸通過。需要使用另一個已備妥的 Python 環境時，可指定 `-PythonPath`，其中必須安裝相同 requirements。

## 原生 E2E 的四個流程

| 流程 | 主要操作與必須成立的結果 |
|---|---|
| 連線保存 | 缺 TOML 正常開空清單；新增／測試／改名與分類；空白憑證編輯仍保留原憑證；重開後 id、revision、分類及能力保持；刪最後一筆後仍能重開空清單。 |
| 終端與工作 | 由畫面建立及選擇 Shell，真 xterm 輸入與呈現 Unicode；resize、兩分頁不串線；暫時斷線後沿用 token／游標，不建立或 renew，也不重播輸入；關頁只 detach。 |
| 桌面預覽 | 專用截圖 Shell 傳回可解碼的大圖；首圖、實際等待 30 秒自動更新、手動更新、停用清圖；工作 Shell 不收到截圖命令，只有自建暫時 Shell 被刪除，終端繼續可用。 |
| 加密搬移 | 原生選檔器匯出含憑證加密檔；在另一份空設定匯入；錯密碼不改設定，重新解鎖後套用並重開可連；相同網址預設略過，明確選取更新才變更 revision。 |

測試不安裝或執行 coding CLI；只驗證 App 依明確能力設定送出預期命令。桌面 fixture 傳回真 PNG，走完整 framing、Rust 驗證與畫面解碼；不是把任意 bytes 當成截圖。它也不執行 Linux 的 preview helper，helper 自身的驗證仍屬 Host 測試範圍。

## 功能與既有測試對照

路徑均相對於專案根目錄。表內用測試名稱作定位，詳細測試原始碼連結集中在附錄。

| 修改範圍 | 先檢查的既有案例 | 需要連帶驗證 |
|---|---|---|
| 唯一 TOML／CRUD／revision | core `openab/tests.rs`：`single_toml_empty_sources_and_no_json_fallback`、`single_toml_external_changes_refuse_stale_operations`、`concurrent_mutations_cannot_accept_two_old_revisions` | 原生 CRUD、重開、匯入；不要只測成功新增。 |
| Windows 憑證與能力保存 | GUI `dpapi.rs`：`native_protection_roundtrip_and_context_binding`、`native_host_store_reload_edit_delete_and_corruption`；`runtime_hosts.rs`：`category_update_preserves_local_and_exact_origin_catalog_capabilities` | 真 DPAPI、空白憑證編輯、分類／CLI／桌面 metadata 保存、錯 context 拒絕。 |
| 搬移加密與原子套用 | core `openab/transfer_tests.rs`：`reject_untrusted_files_and_authenticated_changes`、`atomic_batch_metadata_reload_stale_and_failure`；GUI `runtime_transfer.rs`：`bounded_ttl_window_binding_busy_cancel_late_and_commit_grant` | 原生 picker roundtrip、錯密碼、重複 origin、取消／背景／過期；Android SAF 另作實機驗證。 |
| Runtime list／建立／續期 | core `openab/tests.rs`：`native_https_official_list_does_not_require_custom_fields`、`explicit_official_renew_is_one_post_without_custom_conditions`；前端 `runtime-sessions.test.ts` | 使用者選取與自動恢復的區分；管理請求結果不確定時不自動重送。 |
| 輸入／ACK／Gap／恢復 | core `openab/tests.rs`：`native_wss_historical_gap_ack_and_live_input_are_byte_exact`、`native_same_retained_token_resumes_from_ack_without_admin_renew`；前端 `runtime-stream.test.ts` | 真 xterm、分頁隔離、同 token 恢復、無輸入重播；ACK 是本機 parse 完成確認，不是遠端 frame。 |
| 圖片／截圖通道 | core `openab/screenshot.rs`：`decode_screenshot_validates_envelope_and_preserves_wire_bytes`、`decode_status_accepts_only_an_explicit_boolean` 與 framing tests；`openab/tests.rs`：`preview_cancel_during_attach_deletes_only_created_owned_shell`、`preview_burst_larger_than_terminal_queue_is_acknowledged_without_slow_consumer` | 有效圖片與 envelope 錯誤、原生大圖與工作 Shell 隔離；只測 burst bytes 不代表圖片解碼通過。 |
| 自動預覽／分頁／關閉 | 前端 `app.test.ts`：`captures immediately, then every 30s, only for the active tab; hide stops new captures`；GUI `runtime_sessions.rs`：`in_flight_screenshot_is_discarded_after_disable_or_close_native` | 原生首圖／auto／manual／停用；晚到 callback 不寫入新分頁。 |
| 終端滾動／手機輸入與背景 | 前端 `terminal-wheel.test.ts`、`mobile-input.test.ts`、`mobile-runtime-ipc.test.ts`、`runtime-stream.test.ts` | 保留 IME、背景取消、原 binding 恢復；Windows 的手機尺寸／模擬事件不等於 Android 實機。 |

## 新功能或修正的測試計畫

動手前在該任務紀錄寫清楚「使用者怎麼觸發、原本結果、預期結果、可能一起受影響的功能」。由上表選擇相關測試層，先確認既有案例是否已能攔住回歸，再補最小缺口。

修 bug 時，盡可能先建立能在修改前重現問題的案例，確認失敗原因正是該問題，再修改實作讓它通過。新功能至少測主要成功流程，以及會破壞保存、認證、取消或工作隔離的相關失敗情境；不要把所有可能邊界一律列為必做。

測試與產品程式分工不同：單元測試可控制時間與回覆；E2E 必須保留真 native 邊界。若需要新增測試入口，先判斷是否能沿用現有 UI 或 fixture，不能為了讓測試成功而弱化 TLS、換用 fake IPC、刪除失敗斷言或把正式資料帶入測試。

完成後執行受影響的檢查與必要完整回歸，記錄受測來源、命令、結果、未執行平台及原因。修改過的測試本身也需審查，尤其是 fixture 是否真能使錯誤版本失敗。既有功能若仍由本機 fixture 無法代表的遠端／平台能力支撐，明列相應實機驗收；不把這些限制寫成已通過。

## 結果與失敗處理

統一腳本在 `target/windows-regression/checks-*/result.json` 保存選定 suite、各步 exit code 及 log 路徑；原生 runner 的每次結果留在獨立 `run-*/result.json`。這些都是 ignored 本機產物，不提交憑證、測試影像、WebView profile 或建置快取。

先看失敗步驟與 log，再依原因處理：缺工具／driver 不符是環境問題；編譯失敗是來源或工具鏈問題；畫面操作、協定次數、保存內容或圖片解碼不符合預期才是對應測試失敗。不能以「環境不穩」概括所有 assertion，也不能只重跑直到偶然通過。修正後只重跑受影響範圍，必要檢查成功便收尾。

每次原生測試使用自己的 TOML、WebView profile、臨時 TLS CA 與虛構憑證；CA 只交給 debug `openab-fixture`，不安裝至 Windows 信任儲存區。正式 release 禁止此 feature。測試只回收自己建立的程序與 picker，失敗也不能關閉使用者的既有 App 或刪除舊資料。清理前先確認程序已結束，再依實際 run 目錄處理，不對整個 target 作廣泛刪除。

## GitHub Actions 的使用決定

本專案目前為 private。GitHub 雖提供 Windows runner，私有倉庫使用託管 runner 有免費額度，超額可能計費；不符合本輪「不得因免費額度用完而付費」條件，因此不建立 Actions workflow，驗收以 Windows 本機成功執行為準。未來條件變更時再重新確認官方計費規則，不能直接把這套測試接上託管 runner。相關條件見 [GitHub Actions 計費](https://docs.github.com/en/billing/concepts/product-billing/github-actions)與 [GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。

## 附錄：實作與參考

[統一測試入口](../../scripts/test-windows-regression.ps1)、[原生 runner](../../scripts/windows-regression/run.py)、[本機協定 fixture](../../scripts/windows-regression/fixture.py)、[Windows DPAPI tests](../../crates/art-term-gui/src/dpapi.rs)、[核心協定 tests](../../crates/art-term-core/src/openab/tests.rs)、[搬移 tests](../../crates/art-term-core/src/openab/transfer_tests.rs)、[截圖 decoder](../../crates/art-term-core/src/openab/screenshot.rs)、[前端 tests](../../apps/desktop/tests)。

[Microsoft：以 Edge WebDriver 測試 WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/webdriver)。
