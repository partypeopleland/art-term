# TUI 功能回歸

TUI 的功能移植必須有對應的實際操作測試。依[共同功能矩陣](../refarch/test-matrix.md)挑選案例，不能以「共用核心測試通過」或「桌面截圖成功」代替 TUI 到工具的完整流程。案例清單在 [cases.json](../../scripts/tui-regression/cases.json)；每次結果須附受測執行檔 SHA、來源差異／提交與 PASS／FAIL／NOT_RUN。

## 功能對照

| 案例 | TUI 必須驗證 | 執行入口 |
|---|---|---|
| T-CRUD | Host／Agent 建立、測試、空白憑證編輯、取消／刪除、重開與分類保存 | catalog.py |
| T-SECRET | 遮罩、私有帳號金鑰、加密保存、context／權限／金鑰遺失拒絕 | run.py、TUI vault／backend 與核心測試 |
| T-TERM | 建立／附加／外部接管、多分頁 Unicode／ANSI／輸入隔離、尺寸、只脫離與遠端刪除分開 | run.py |
| T-HISTORY | 共用 scroll／follow、閱讀中輸入與 VT 回覆、斷線原 token 恢復、不重播輸入 | run.py |
| T-WHEEL | 右側終端滾輪依遠端滑鼠模式分流；CLI 座標／修飾鍵、取消模式後歷史舊行與回到即時、分頁隔離 | wheel.py |
| T-SCREEN | Screen CRUD／revision／token、真圖色彩與區域、來源切換、延遲舊圖、錯誤恢復與停止 | run.py、圖片核心／預覽測試 |
| T-CONFIGURE | TUI 選擇配置正確工作 Shell，無 adapter 不假報 ready；截圖不可用或 SSH 終端未宣告 Sixel 時仍能配置及實際呼叫工具 | run.py 負向、official_runtime.py 正向 |
| T-TOOLS | 從該 Shell 裡晚啟動的正式 stdio client 實際探索／呼叫 HTTP 桌面工具，保留原參數及結果 | official_runtime.py |
| T-TARGET | 兩 Shell A／B 隔離、切換中 A 呼叫仍回 A、舊 expected_target 拒絕且不送 B | official_runtime.py |
| T-RESTORE | detach／重開後 current／verify 恢復；不重新 configure／clear，不改遠端 generation | official_runtime.py、共用 restore 核心測試 |
| T-LIFETIME | Stop 保留工具；Clear 只取消該 Shell；App 結束保留 broker；遠端工作刪除清理 broker | official_runtime.py |
| T-MOUSE | 分頁切換／關閉、焦點、樹與表單／確認／選單、滾輪、窄視窗取消、本機操作不送工作輸入 | catalog.py、run.py、draw hit-map 測試 |
| T-VIEWER | 點縮圖展開右側、X／再點收回、拖曳／滾輪／Fit／邊界、裁切原圖與背景輸出；操作不送遠端輸入／resize／額外截圖 | viewer.py |
| T-INPUT | 中文字寬、遠端與表單的可見實體游標、左側邊框穩定；系統組字另行人工驗證 | viewer.py |
| T-IMAGE | 慢截圖期間縮圖不閃說明、來源／停止／遮罩／resize 生命周期；原生 Sixel 編碼解碼與畫面邊界，停用時不取圖／不輸出圖片；持續取圖快於渲染時仍發布同來源／幾何的單調完成圖 | viewer.py、image_view／UI 測試 |
| T-IMAGE-DETECT | 本機／SSH 實際 DA 與尺寸回覆、負向／逾時／晚到分段回覆、查詢尺寸優先於系統預設值，舊設定不改寫 | viewer.py --mode policy |
| T-TERMINAL-REPORTS | 同一 EventStream 的回覆／鍵盤／貼上／滑鼠順序，分段、超長／錯誤回覆、ESC、resize、取消與 EOF 競態 | terminal_events.py |
| T-IMAGE-CLIENT | 真正本機 XTerm 內執行候選 TUI，檢查縮圖／右側展開的螢幕 RGB、字格內圖片細節與 X／遮罩清圖 | native_client.py |
| T-TRANSFER | 混合 Host／Screen 加密搬移、錯密碼不改檔、取消、重複略過／取代、重開後認證 | catalog.py、核心 transfer 測試 |
| T-KEYS | 直接編輯檔案後自訂前導鍵／直接鍵生效、停用舊鍵、錯誤設定指出路徑並保留原檔 | catalog.py、keybindings 測試 |
| T-RELEASE | 實際壓縮套件無 GUI 啟動、免密碼重開、滑鼠模式／TTY 還原、拒絕 debug CA、附預設快捷鍵範例 | 對當次候選 ELF 的獨立 smoke 與套件檢查 |

## 執行

在隔離且以 `/tmp/art-term-*` 為根的環境執行，勿使用既有連線、憑證、桌面或遠端工作作為測試資料。Python 需求在 [requirements.txt](../../scripts/tui-regression/requirements.txt)。

```sh
cargo build -p art-term-tui --features openab-fixture --locked
python3 scripts/tui-regression/run.py --binary /absolute/target/debug/art-term --output-dir /tmp/art-term-tui-terminal-result
python3 scripts/tui-regression/viewer.py --binary /absolute/target/debug/art-term --output-dir /tmp/art-term-tui-viewer-result
python3 scripts/tui-regression/catalog.py --binary /absolute/target/debug/art-term --output-dir /tmp/art-term-tui-catalog-result
python3 scripts/tui-regression/official_runtime.py --binary /absolute/target/debug/art-term --output-dir /tmp/art-term-tui-tools-result
```

輸出目錄必須不存在。run.py、catalog.py、viewer.py 操作真正的 TUI／PTY，對端是隔離的 TLS／WSS 協定 fixture；它故意回報零 adapter，適合驗證未就緒提示，不能證明工具可用。

official_runtime.py 操作相同 TUI，經已釘選、未修改的官方 Runtime、真 tmux／Shell supervisor、正式 stdio adapter 與 HTTP A／B 測試服務。固定 client 從實際工作 Shell 啟動並真的呼叫工具；測試不 mock 產品 broker 路由。需要 tmux、Node 與前端建置依賴，以建置共同 paged history helper。可用 `--history-bundle`、`--node`、`--runtime-binary` 指定對應實際來源及經驗證產物；不能藉此換成測試替身。

工具案例先驗證圖片可用但工具清單失敗時仍可預覽且不假報工具 ready；再讓截圖不可用但保留工具服務，驗證 TUI 仍可配置及實際執行工具，再恢復圖片確認預覽恢復。這能防止「先有桌面才能設定開桌面的工具」的循環依賴。連線清單更新、背景請求、分頁關閉與資料 revision 邊界由精確單元測試補強。

EventStream 範例與 Python 驅動直接操作真正的輸入事件流，兩種 Unix backend 分別執行；包含 20 次關閉 PTY 的競態回歸。它不測圖片渲染。

```sh
cargo build -p art-term-tui --example terminal_events --locked
python3 scripts/tui-regression/terminal_events.py --binary /absolute/target/debug/examples/terminal_events --output-dir /tmp/art-term-terminal-events-result
python3 scripts/tui-regression/viewer.py --mode policy --binary /absolute/target/debug/art-term --output-dir /tmp/art-term-detection-result
```

`native_client.py` 另外需要 Xvfb、支援 Sixel 的 XTerm 390，以及 `requirements-native-client.txt` 的 Python 套件。這些是驗證工具，不是 art-term 的執行依賴。它建立自己的顯示伺服器與終端，執行真正的候選 TUI，從該終端視窗讀取螢幕 RGB，不把解碼的 Sixel 像素當成螢幕截圖。

```sh
python3 scripts/tui-regression/native_client.py \
  --binary /absolute/target/debug/art-term \
  --xterm /absolute/path/to/xterm \
  --output-dir /tmp/art-term-native-client-result
```

預設驗證新建 `auto` 設定，記錄實際終端版本、啟動參數、終端查詢回覆的字格像素及受測 ELF SHA。`--library-path` 可指定隔離驗證套件的函式庫位置。不要以顯示協定測試或其他終端版本的結果代替這項驗證。本機與 SSH 的支援／不支援／逾時偵測由 viewer 案例驗證；官方 Runtime 另驗證未宣告 Sixel 時工具仍可呼叫。實際 SSH 終端繪圖須獨立記錄，不能用環境標記或模擬回覆代替。

測試程序須在失敗時也關閉自建 PTY／官方 Runtime／tmux／broker／固定 client，移除自建暫存與測試 HOME。結果可保留少量紀錄，勿保存私人憑證或整個建置環境。

可獨立執行終端滾輪案例：

```sh
python3 scripts/tui-regression/wheel.py \
  --binary /absolute/target/debug/art-term \
  --output-dir /tmp/art-term-wheel-result
```

Codex 等程式啟用滑鼠協定時，右側終端滾輪沿既有 PTY 傳給程式；未啟用時使用共用主機歷史介面。預覽圖片的滾輪仍只縮放圖片。此 fixture 案例不等於實際 Codex 模型驗收。

## 驗收界線

真實 TUI、協定 fixture、官方 Runtime／工具流程與候選 ELF smoke 分別報告，不能互相取代。Windows DPAPI／WebView／系統選檔器、Android Keystore／SAF／旋轉／語音／IME 是平台專屬案例，不宣稱已在 TUI 執行。原生 Windows／APK、實際 AI 模型／Codex 與私人現場 MCP 仍須另外明確驗證；固定 client 成功不等於模型已載入工具。

viewer.py 的 committed CJK／游標座標不等於作業系統輸入法的 composition 驗收；Sixel 編碼／PTY 協定驗證也不等於實際終端已畫出圖片。兩者須明列人工驗證狀態。產品不提供字格圖片模式；測試中的像素或字格座標檢查是對原生 Sixel 資料的檢視，不是產品圖片備援。

同一 Shell 已有相同 binding 的未結束本機分頁時，再次選取會切回該分頁，保留即時畫面或歷史閱讀位置，不 renew 或新增分頁。run.py 驗證背景分頁切回、helper 不重建與輸入不重播；official_runtime.py 以正式 Runtime／helper 驗證切回及閱讀位置。關頁後接回與外部 client 接管仍分開驗證，不能互相替代。已結束、不同 Host 設定版本或不同工作 generation 不直接重用。
