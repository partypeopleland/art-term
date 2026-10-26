# 客戶端架構與模組責任

art-term 讓 Windows、Android 與 Ubuntu 終端機使用者連接既有 Runtime 的 Shell。Windows 與 Android 共用 Vue 畫面並提供共享桌面；Ubuntu 使用可在 SSH 下操作的文字介面。三者共用 Rust 核心，平台層負責檔案位置、憑證保護和呈現，共用核心維持遠端連線的身分與恢復規則。

## 來源目錄與平台邊界

Rust 核心、App 的原生整合與 Vue 畫面分開維護。前端放在 `apps/client`，因為同一個前端專案同時建置桌面與 Android App。桌面與手機各有畫面入口，共用的設定、Dashboard、主題、終端與狀態控制只保留一份。

```text
crates/
  art-term-core/                  共用連線、設定、認證與工作生命週期
  art-term-tui/                   Ubuntu Server／Desktop 終端客戶端
    src/main.rs                  art-term 命令與帳號金鑰入口
    src/backend.rs               Host、Shell 管理與分頁建立
    src/tab.rs                   每個分頁的連線、歷史及恢復
    src/terminal.rs              VT 終端模型、鍵盤編碼及查詢回覆
    src/ui.rs                    三區工作區、焦點、表單與分頁選擇
    src/layout.rs                共同區域尺寸與窄視窗配置
    src/preview.rs               直接 MCP 截圖與原始圖片解碼
    src/image_view.rs            有界圖片渲染與本機平移縮放
    src/display.rs               圖片模式與實際字格像素設定
    src/tty.rs                   本機終端模式與退出還原
    src/vault.rs                 私有帳號金鑰與儲存鎖
  art-term-gui/
    src/
      lib.rs                      Android 原生函式庫入口
      desktop/main.rs             桌面原生入口，目前支援 Windows
      shared/                     共用 Host、session、Screen 與搬移整合
      platform/
        windows/                  DPAPI、Windows 系統瀏覽器
        android/                  Android IPC 與 operator vault 整合
    gen/android/                  建置工具產生的 Android 工程
  tauri-plugin-mobile-*/          Android 憑證、語音與原生閱讀器

apps/client/
  src/
    main.ts                       選擇平台入口並組合原生 transport
    shared/
      components/                 共用畫面元件
      state/                      共用狀態與控制器
      terminal/                   終端介面、呈現與歷史控制
      transport/                  共用通訊契約、剪貼簿與連結操作
      theme/                      共用主題與配色
    desktop/                      桌面畫面與桌面 IPC adapter
    mobile/                       手機畫面、Android IPC、輸入與原生閱讀
  tests/                          兩種畫面及共用行為測試
  ui-preview/                     使用假資料的桌面與手機預覽
```

目錄按責任分組，並不代表每個目錄都能獨立建置。`shared/terminal` 保留現有桌面與手機終端策略，仍會使用 `mobile/` 的觸控捲動；共用 `TerminalPane` 也會依既有條件使用手機原生閱讀器。平台選擇仍集中於入口，這次整理不拆開終端實作，也不改變 IPC 或原生能力的判斷方式。

原生層由桌面入口與 Android 函式庫入口各自註冊模組。`#[path]` 指向共用與平台檔案，維持既有 Rust 模組名稱及呼叫關係。桌面 binary 仍名為 `art-term-gui`，Android 函式庫仍名為 `art_term_mobile`；bundle id、設定檔位置與憑證金鑰身分不因來源目錄搬移而改變。`gen/android` 由既有建置流程準備，不另複製成手工維護的 App。

Ubuntu 的交付目標是獨立 `art-term` 執行檔，可在 Server、Desktop 或 SSH 的終端機使用。它直接依賴 `art-term-core`，不載入 GUI crate、WebKit、GTK、D-Bus keyring 或本機網頁伺服器。受控端繼續使用相同 Runtime／helper，準備方式不因這個客戶端改變。用法見 [Ubuntu 終端機版](../platforms/linux.md)。

可從 repository 根目錄執行 `git ls-files apps/client/src crates/art-term-gui/src crates/art-term-core/src crates/art-term-tui/src` 查看受版本管理的實際來源檔。建置、測試與預覽命令使用 `apps/client`，例如 `npm --prefix apps/client run typecheck`；完整流程見[本地建置](../platforms/local-build.md)與[畫面預覽](../platforms/ui-preview.md)。

## 從畫面到遠端 Shell

```text
Windows / Android App                         Runtime Host
+------------------------+                    +---------------------+
| Vue UI + xterm          |                    | openab-pty          |
| tabs / input / preview  |                    | admin API / Shell   |
+-----------+------------+                    +---------+-----------+
            | restricted IPC                            ^
+-----------v------------+                              |
| Native GUI adapter     |                              |
| issued targets / gates |                              |
+-----------+------------+                              |
            |                                           |
+-----------v------------+                              |
| Rust core              |-------- HTTPS / WSS ---------+
| HostStore / session IO  |
+-------+----------+-----+
        |          |
 connections.toml  platform credential protection
                   (Windows DPAPI / Android Keystore)
```

Vue 管理分類、Shell 選擇、分頁與終端呈現。WebView 透過受存取控制清單（ACL）限制的程序間呼叫（IPC）取得安全資料物件；Runtime 管理憑證、session token 與已保存的 MCP token 留在 native 層；明確配置 AI 工具時，MCP token 經受保護握手送到受控端 adapter 行程記憶體。WebView 沒有 Runtime 外連、憑證讀回或任意遠端命令 API，因此畫面狀態不能直接指定另一個遠端工作。

native GUI adapter 驗證 Host 操作並發出 target。target 是後端核發的連線目標，凍結 Host id／revision、Runtime 入口、Shell 名稱、官方 generation 與能力。revision 辨識設定版本，generation 辨識工作世代；session binding 是實際連線與這些資料的綁定。官方協定不提供跨服務重啟的原子工作身分比對，App 以原 attach token 恢復短暫斷線，驗證失敗就停止並請使用者重新選擇。

Rust 核心讀取唯一 `connections.toml`，透過官方 HTTPS API 列出、建立或依使用者選擇續期 Shell，再以 WSS 連接已核發的 target。HTTPS 管理呼叫不接受 redirect、環境代理或降級，錯誤使用固定安全訊息。Runtime 使用未修改的官方產物；Ubuntu 終端介面同樣使用這條 HTTPS／WSS 路徑。

## Ubuntu 文字介面的狀態歸屬

```text
SSH / local TTY -> text UI -> backend -> shared HostStore / AdminClient
                                  |
                                  +-> tab actor -> shared SessionHandle -> WSS
                                           |
                                           +-> VT parser -> immutable screen -> text UI
                                           +-> shared HistoryChannel -> existing helper
                                  |
                                  +-> per-tab desktop worker
                                           +-> shared configure -> temporary control PTY -> Shell broker
                                           +-> shared instance_restore -> source/generation validation

text UI -> visible capture worker -> shared MCP screenshot -> decoded source frame
         -> latest render request -> bounded thumbnail + right viewer
                                  -> native Sixel only
```

每個本機分頁只有一個 actor 修改連線與 VT 終端模型。背景分頁同樣解析輸出並確認 exact batch；畫面可以略過中間繪製，不能略過遠端輸出。解析器完成一批內容後才呼叫核心 ACK，遇到 Gap 先重設終端狀態。遠端終端查詢的回覆送回來源分頁，不能因使用者切換分頁而送錯工作。

歷史捲動與使用者輸入依同一分頁排序。閱讀歷史時，輸入透過共同 helper 的 follow 操作回到即時畫面；結果不確定時拒絕重送。終端協定回覆仍直接回到原 PTY，不觸發 follow。歷史請求進行時仍處理輸出，避免遠端等待終端查詢回覆而互相卡住。重連沿用原 binding、token 及已解析游標，不建立新工作或重播輸入。關閉本機分頁只脫離連線，並清理該分頁自有的短命 helper。

文字介面讀取不可變畫面，透過有界命令入口發出操作。網路、搬移檔密碼雜湊與圖片處理不在畫面繪製迴圈執行。Host 及加密搬移沿用核心交易規則；帳號金鑰儲存持有程序間寫入鎖，避免兩個 TUI 同時修改同一份設定。啟動不要求密碼，金鑰由私有檔案與 Unix 帳號權限保護，見[憑證保存](credential-storage.md)。

工作區左上固定展開 Computer／Agent／Screen 與工作／新增操作，左下顯示實際桌面圖片，右側保留分頁終端。滑鼠 hit region 與畫面一起產生，經同一操作入口執行；表單遮住下層控制，本機滑鼠不送入遠端。快捷鍵由連線設定旁的 `keybindings.toml` 解析，畫面提示讀取目前綁定。單一 layout 計算繪製與 PTY 內容尺寸；窄視窗顯示焦點區域。只有終端焦點且沒有 modal 時送出一般按鍵與貼上。

每個分頁的桌面狀態由 backend 擁有，有界入口啟動最多一個配置／恢復／取消工作，畫面只讀取非秘密狀態。工作 Shell 名稱、Host／Screen revision 與存活狀態在交換前後核對，憑證只經核心 raw／no-echo 管理通道。關閉分頁等待自有管理工作清理；停止預覽、detach 與 App 結束不清除 Shell broker。純 `instance_restore` 身分／generation 核對已移至核心，圖形與文字客戶端共用同一規則。

工具狀態以 broker configure／verify 的實際結果確認；預覽以獨立圖片 worker 確認。TUI 的來源前置驗證只初始化 MCP 認證與協定，不要求先截圖或工具清單成功，避免桌面尚未啟動時無法配置工具，也避免工具清單失敗使有效預覽消失。

每個穩定 TabId 各自選擇或停止預覽，只有當前可見分頁擷取。capture worker 沿用核心 `instance_mcp::screenshot`，依序完成請求與有界解碼，發布帶 sequence 的原始圖片。來源身分包含 TabId、generation、Screen id／revision，不包含焦點或區域尺寸。獨立 render worker 由原圖裁切、縮放，最多準備縮圖與右側展開圖兩個有界表面；身分再包含完整區域及 viewport，拒絕舊來源、舊圖序與舊幾何結果。每個 worker 只保留最新待處理要求並等待既有工作完成，不使用無限佇列或取消後脫離的工作。

UI 持有同一來源的最新圖片與本機 viewport，切焦點與平移縮放不清除來源，也不影響遠端 PTY 尺寸或送出輸入。來源切換、停止與錯誤清圖；表單與說明遮罩不顯示圖片。display 邊界以 Available／Unavailable 決定本機原生預覽；只有已驗證的字格像素能建立 Sixel renderer，沒有字格圖片備援。SSH、multiplexer、Linux 主控台或顯示設定失敗只停用預覽，不建立擷取與渲染 worker，既有 desktop broker 配置路徑照常執行。Sixel 以已知字格像素處理原生圖片，不向 stdin 發送圖片能力／字格像素查詢，也不修改 multiplexer。原生表面消失、換來源、改幾何與退出時清除圖片；繪製後單獨定位實體游標，避免以畫出的游標代替輸入法插入點。

## 受控端提供固定介面

受控端是獨立的受控點，內部實作由它負責。外部 App 遵循同一套介面；平台差異限於呈現與操作事件，不因某個平台建立專用協定或獨立安裝項。共同介面可透過能力宣告向後相容升級，升級同一個通用 helper。

歷史介面由同一個受控端 helper 提供。原生 viewport 的 `scroll / follow` 控制原工作畫面的閱讀位置，畫面沿既有 PTY 回傳；快照的 `viewport / older / ack / keep / close` 維持原有語意與相容性。共用 Rust 層管理工作身分、能力、通道、輸入排序及回覆驗證；App 只提出中立操作，不自行組合 tmux 指令或操作 socket。

Windows 的 `HostTerminalViewport` 管理捲動事件、保活與生命週期，使用原本的 xterm 呈現所有畫面。Android 的 `HostHistoryWindow` 管理快照、分頁、顯示確認及 1024 行窗口，由原生 View 呈現。共同介面以能力宣告向後相容演進，受控端內部實作保持集中，不另設平台專用 helper 或安裝流程。

## Android 畫面與原生閱讀器

手機介面以固定頂列統一導覽；軟體資訊及設定是頂列下方的內容頁，Main 與 Connect 是可水平拖曳的工作頁。頁面位移與指示色條共用同一滑動進度，因此手指停在中間時，畫面與色條位置一致。

```text
MobileApp (fixed brand / Main / Connect / global gear)
  |
  +-- AppHomepage -------- version + bundled release notes
  +-- ThemeSettings ------ theme + Host import/export + Host/Screen forms
  +-- workspace pager ---- shared page position + indicator progress
         |
         +-- Main: ConnectionDashboard -> HostConnections / saved Screens
         +-- Connect: tabs / terminal / Screen picker / voice / quick keys
                            |
                    NativeTerminalReader (Android native view)
                            |
                  horizontal gesture -> pager bridge

selected theme -> CSS variables + xterm palette + native reader palette
```

`ConnectionDashboard` 管分類、數量與收合，`HostConnections` 管每筆主機的 Shell 清單、明確 attach／接管與建立工作。頁面切換不改變本機分頁與遠端工作 binding；既有 target 核發、Rust 核心和 HTTPS／WSS 協定保持原有責任。

Android 終端閱讀器是覆蓋在 WebView 終端位置的原生 View，水平手勢需透過 bridge 回報共用 pager，不能只依賴 Vue pointer 事件。閱讀器仍負責垂直歷史捲動；頁面、設定、資訊與語音燈箱的可見狀態控制原生 View 是否呈現，避免遮住 Vue 內容。Theme 保存於裝置本機，變更時同步頁面樣式、xterm 與原生閱讀器配色。

瀏覽器預覽沿用手機元件，但以獨立入口注入記憶體 transport、假語音與瀏覽器終端；正式入口使用原生 transport、裝置端辨識及原生閱讀器。預覽不接真實 Runtime，不參與 APK 打包，詳見[手機畫面預覽](../platforms/ui-preview.md)。

## Windows 共用工作區與歷史閱讀

Windows App 接入同一套分類 Dashboard、全頁 settings、homepage、四主題及 Screen picker，保留 Host 管理、硬體鍵盤與 xterm。設定／首頁遮蓋時終端元件保持掛載並處理輸出，返回後 fit／focus，表單正常輸入不被終端快捷鍵攔截。語音與 Android 原生閱讀／手勢 plugin 不移植。

Windows 的即時畫面與歷史都使用原本的 xterm 實例。上捲呼叫共同歷史通道的原生 viewport 操作，tmux 沿原 PTY 回傳較早畫面；下捲到底自然接續即時輸出。字型、主題、游標與文字選取維持原終端，沒有另外的閱讀覆蓋層。一般 normal buffer 仍使用 xterm 本機 scrollback；受控工作才使用共同端點的原生閱讀能力。

```text
Windows wheel -> HostTerminalViewport -> shared history channel
                                              | scroll / follow
                                              v
                                         Host helper -> tmux work pane
                                                            |
Windows xterm <--------------- original PTY -----------------+

Android touch -> HostHistoryWindow -> same history channel
                                              | snapshot operations
Android native View <------------ bounded rows/epoch --------+
```

共用原生連線層將使用者輸入與 viewport 操作排序，閱讀時先恢復即時位置，再交付輸入；終端自動查詢回覆則直接沿原 PTY 回傳，不改變閱讀位置。控制租期只清理自有閱讀狀態，關閉或程序死亡不終止工作 Shell。

Windows EXE／WebView2 與 Android 原生 View 的操作仍須分別驗證；平台呈現測試不能取代受控端契約測試。

## 輸出完成後才確認

```text
connections snapshot -> HTTPS list / create / explicit renew
                                      |
                                issued target
                                      |
Runtime output -> WSS binding -> bounded queue -> xterm parse
                                      ^                |
                                      |     exact batch ACK via IPC
                                      |                v
                               native rendered cursor
                                      |
                           reconnect with since -> Runtime
```

有界佇列限制等待處理的 bytes 與事件數，避免終端或背景分頁無限累積資料。xterm 完成一批輸出的解析後，控制器才經 IPC 確認 exact batch，由 native SessionHandle 更新本機已呈現游標。這個 ACK（已處理確認）不是每批送給 Runtime 的遠端 frame；恢復連線時才以已呈現游標作為 since，向 Runtime 請求後續輸出。不能因收到資料就提前確認。若 Runtime 告知 Gap（輸出已缺段），App 清理待送輸入並重置顯示，再處理可補送的輸出。短暫斷線只恢復原 binding，不重建 Shell，也不重播結果不確定的輸入。完整生命週期見 [連線與工作](sessions.md)。

核心同時提供尺寸／大小界限及 PNG／JPEG 驗證。Screen 與 Host 是同一 TOML 中兩個獨立集合；原生層直接連 Screen 的 instance-mcp 擷取圖片，不經 PTY，不建立截圖 Shell。Screen 由每個 live Shell 分頁明確選取，與舊 Host 桌面能力欄位無關。原生後端另外保留未選 Screen 的 Windows 舊分頁 Runtime 截圖相容路徑；Android／Windows 新選擇始終直接 MCP，不退回該路徑。

```text
connections.toml: Host profiles + independent encrypted Screen profiles

App native -- direct MCP screenshot --------------------> instance-mcp -> desktop
                                                              ^
Runtime work Shell -> tmux pane -> Shell-owned broker -- MCP --+
                                      |       ^
                                 owned PTY    | owner-only socket
                                      |       |
                                    bash -> Shell client / CLI stdio proxy
                                              |
App -- temporary raw/no-echo configure Shell -> broker memory target
```



本次設計由新 tmux 工作 Shell 先啟動 supervisor broker，再以自有 PTY 啟動 Bash，保留 job control／resize 並以 pipe-death guard 清理自有子行程；App 透過短命管理 Shell 配置目前選擇。Shell client 與 coding CLI 的 scoped stdio proxy 使用同一 broker；其他 MCP client 可登記非秘密 `stdio --current-shell` 通用入口，以受限同 UID 祖先核對解析當前活 broker，不掃描 Host 或寫固定 binding／認證，不需要 App 專用 Codex 入口，不使用 Host 共用選擇，也不接入 Discord Agent。管理通道只傳有界配置 frame、回非秘密結果，不轉送圖片。broker 回傳完整上游 MCP 工具結果；固定 `current_target`／`list_target_tools`／`call_target_tool` 以來源版本避免切換時串線。預覽成功與工具就緒分別核對，不能以其中之一替代另一個。CLI EOF 不清除 Shell 選擇，clear／Shell 結束停止工具；既有 Shell 不注入啟動定義，安裝 helper 後需建立新連線。設計與待驗收邊界見[桌面](desktop.md)及[工具 bridge](runtime-desktop.md)。

## tmux 保存工作與按需讀頁

新建 Shell 由 App 的單一「建立並連線」入口啟動主機私有 tmux。Runtime 繼續負責 HTTPS／WSS、工作 token 與即時輸入輸出；tmux 負責保存終端畫面、原生換行與有限歷史。讀頁輔助程序透過同一 Runtime 的獨立短命 Shell 回傳指定範圍，沒有新增伺服器 API。

```text
App --create--> Runtime work Shell --wrapper--> private tmux work pane
 |                   |                              |
 |<-- live WSS ------+                              | native history in RAM
 |                                                  |
 +-- viewport / older request --> Runtime helper ----+
 |                                      | native copy-mode backing grid
 +<-- bounded rows / epoch --------------+ capture requested rows only
 |
 Android native reader: viewport first, upward gesture -> older page
```

每個工作使用自己的 socket，tmux 與系統版本並存。快照讀取最多維持兩份 tmux 原生閱讀快照；Android 首次只讀當前畫面，上捲才最多取得 256 行，閱讀窗口最多 1024 行。Windows 原生閱讀直接由 tmux 管理有限的 backing history，不在 App 重新轉譯歷史。歷史版面與軟換行由 tmux 提供，輔助程序不先擷取完整歷史再切頁。畫面確認已顯示後才釋放舊快照，前插舊頁與新輸出均保留閱讀位置。生命週期、限制與主機準備見[終端歷史](terminal-history.md)。

## 平台層負責什麼

Windows 使用目前使用者的 DPAPI 保護管理憑證，Android 使用既有 operator Keystore；共用 CredentialContext 維持憑證與設定身分的綁定。唯一 TOML 的原子保存、外部修改核對與舊格式相容由 HostStore 負責，見 [設定](connections.md)與 [憑證保存](credential-storage.md)。保留的 bootstrap 模組僅供舊測試相容，正式啟動不呼叫，私有 seed 建置會被拒絕。

Android adapter 處理觸控、輸入法組字、快速鍵與背景生命週期。背景時清除敏感輸入和待送操作，健康終端連線可繼續保留；這些平台差異不改變核心 binding 規則，詳見 [Android](../platforms/android.md)。邊界的決策理由見 [ADR-001](../adr/ADR-001-client-boundaries.md)，尚未實作的候選見 [BDR-002](../bdr/BDR-002-future-directions.md)。

## 附錄：實作與參考

[GUI 註冊與 ACL 入口](../../crates/art-term-gui/src/lib.rs)、[前端控制器](../../apps/client/src/shared/state/controller.ts)、[Rust openab 核心](../../crates/art-term-core/src/openab/mod.rs)。
