# GUI 架構與模組責任

art-term 讓 Windows 與 Android 使用者連接既有 Runtime 的 Shell，並選擇觀看主機的共享桌面。兩平台共用 Vue 介面控制器與 Rust 核心；平台層負責檔案位置、憑證保護和原生操作，共用核心維持遠端連線的身分與恢復規則。

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

Rust 核心讀取唯一 `connections.toml`，透過官方 HTTPS API 列出、建立或依使用者選擇續期 Shell，再以 WSS 連接已核發的 target。HTTPS 管理呼叫不接受 redirect、環境代理或降級，錯誤使用固定安全訊息。Runtime 使用未修改的官方產物；這條路徑不需要本專案伺服器補丁，也不恢復已退役的 SSH terminal、CLI／TUI 或 gateway。

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

核心同時提供尺寸／大小界限及 PNG／JPEG 驗證。Screen 與 Host 是同一 TOML 中兩個獨立集合；原生層直接連 Screen 的 instance-mcp 擷取圖片，不經 PTY，不建立截圖 Shell。Screen 由每個 live Shell 分頁明確選取，與舊 Host 桌面能力欄位無關。原生後端另外保留未選 Screen 的 Windows 舊分頁 Runtime 截圖相容路徑；Android 新選擇始終直接 MCP，不退回該路徑。

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

每個工作使用自己的 socket，tmux 與系統版本並存。輔助程序最多維持兩份 tmux 原生閱讀快照；首次只讀當前畫面，上捲才最多取得 256 行，App 閱讀窗口最多 1024 行。歷史版面與軟換行由 tmux 提供，輔助程序不先擷取完整歷史再切頁。畫面確認已顯示後才釋放舊快照，前插舊頁與新輸出均保留閱讀位置。生命週期、限制與主機準備見[終端歷史](terminal-history.md)。

## 平台層負責什麼

Windows 使用目前使用者的 DPAPI 保護管理憑證，Android 使用既有 operator Keystore；共用 CredentialContext 維持憑證與設定身分的綁定。唯一 TOML 的原子保存、外部修改核對與舊格式相容由 HostStore 負責，見 [設定](connections.md)與 [憑證保存](credential-storage.md)。保留的 bootstrap 模組僅供舊測試相容，正式啟動不呼叫，私有 seed 建置會被拒絕。

Android adapter 處理觸控、輸入法組字、快速鍵與背景生命週期。背景時清除敏感輸入和待送操作，健康終端連線可繼續保留；這些平台差異不改變核心 binding 規則，詳見 [Android](../platforms/android.md)。邊界的決策理由見 [ADR-001](../adr/ADR-001-client-boundaries.md)，尚未實作的候選見 [BDR-002](../bdr/BDR-002-future-directions.md)。

## 附錄：實作與參考

[GUI 註冊與 ACL 入口](../../crates/art-term-gui/src/lib.rs)、[前端控制器](../../apps/desktop/src/state/controller.ts)、[Rust openab 核心](../../crates/art-term-core/src/openab/mod.rs)。
