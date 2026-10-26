# 獨立 Screens 與直接 MCP 桌面預覽

Screen 是保存在 App 的獨立 instance-mcp 桌面入口，與 Computers／Agents 的 Runtime Host 分開。App 原生層直接向選定的 instance-mcp 呼叫 `screenshot`，圖片不經工作 PTY、Runtime 截圖 Shell 或 coding AI 轉送。僅使用遠端 Shell 時不需要設定 Screen；同一 Screen 可供多個分頁及其他使用者共同觀看，不提供獨占、租約或桌面重設。

## Ubuntu 終端機預覽

TUI 的左上固定展開 Computer／Agent／Screen 樹狀選單，左下顯示桌面圖片，右側保留分頁與終端。方向鍵／Enter 或滑鼠執行清單操作；Actions 提供編輯／刪除等入口，分頁可點選切換與關閉。token 輸入遮罩，空白編輯仍須符合核心 URL 與 revision 規則。

每個分頁透過右上 Desktop 或可自訂的快捷鍵選擇 Screen。TUI 先透過共用 MCP 初始化驗證目前來源與認證，沿用受保護 configure 通道配置該工作 Shell 的 broker，不以截圖成功作為工具設定或恢復的前提；分別顯示預覽與工具狀態；沒有 adapter 或無法確認時不顯示 Tools ready。重新連入沿用下方共同的 current／verify／來源核對恢復流程，不改寫既有指派。Stop 只停本機取圖；Clear tools 才明確取消該 Shell 工具，關頁與程式結束不取消。背景分頁不擷取。點左下縮圖可在右側展開桌面，以拖曳與滾輪控制本機平移縮放；`[Fit]` 還原縮放，`[X]` 或再次點縮圖收回右側展開。操作不送入遠端桌面或改動 Shell 尺寸；圖片模式與終端相容限制見 [Ubuntu 終端機客戶端](../platforms/linux.md#桌面設定工具與預覽)。

預覽沿用核心直接 MCP `screenshot` 與相同的 URL、token、TLS、圖片驗證規則。PNG／JPEG 解碼後保留原圖，只以 Sixel 原生圖片呈現，不用彩色字格拼圖。本機顯示須有支援的終端與實際字格像素；SSH、multiplexer、Linux 主控台或顯示條件不足時只停用預覽，Shell 桌面工具配置與 AI 呼叫不受影響。不發送圖片能力或字格像素的 stdin 查詢，不修改 multiplexer。編碼器內建，不需要外部轉圖套件；Kitty／iTerm2 未啟用。圖片不提供遠端桌面輸入。

capture worker 在背景依序完成擷取與解碼，獨立 render worker 最多處理一個已加入等待結束的工作，只保留最新待處理要求。來源變更先清圖，既有請求仍正常完成 MCP session 清理；TabId、generation、Screen id／revision 排除舊來源，sequence、區域與 viewport 排除舊呈現結果。解碼受 PNG／JPEG、8 MiB 圖片、4096 像素邊長與 64 MiB 配置上限限制；縮圖與展開表面各最多 160×80 字格，原生表面各最多 4 百萬輸出像素。錯誤清除舊圖並顯示安全訊息。操作見 [Ubuntu 終端機版](../platforms/linux.md)。

## 共用預覽與工具路徑

```text
App native -- direct screenshot ---------------------> instance-mcp -> desktop
                                                           ^
Shell client / coding CLI stdio proxy -> Shell broker ------+
                                             ^
App -- temporary raw/no-echo configure Shell -+
```

預覽與 AI 工具入口是兩個狀態。預覽成功只代表 App 可擷取圖片，不能當成 AI 已載入 MCP。Shell client 與 coding CLI 的標準 MCP stdio proxy 經此 Shell 的記憶體 broker 探索與呼叫同一上游；它完整回傳 MCP `content`、`isError` 與 `structuredContent`，另外帶來源 metadata。App 避開 PTY 傳圖，不表示過濾 AI 工具的圖片結果或只讓 AI 收到文字提示。

## Windows／Android 建立、選擇與停止

Main 的 Screens 列出實際儲存的桌面與數量，可像其他分類收合。全站齒輪的新增連線分類可選 Screen，另有獨立 Screen 編輯／刪除區。名稱、MCP URL 與 token 保存規則見[連線設定](connections.md)，不預填私人入口或憑證。

每個 live Shell 分頁都能開 Screen 選單，與 Host 的舊 `desktop_transport` 無關。第一次可選任意 Screen，再次可切換或停止預覽；清單共用，指派只作用於該工作 Shell。切換先清除舊圖，後端依 Screen id／revision 與本次請求核對來源，關頁、背景、切換或設定修改後的晚到圖片不得重新顯示。App 重開或接管工作後查詢該 Shell 的現有指派，以當前本機憑證實際擷取成功後恢復預覽。跨裝置匯入設定後，本機 Screen ID 可不同；新版受控端會回報既有指派的 URL，App 只在本機找到唯一相同 URL 的 Screen 時重新驗證並恢復。恢復只讀取既有指派，不改寫 Shell 的 Screen 身分、憑證或 generation；工具就緒另以既有 broker 的憑證實際查詢工具清單並核對 generation。無法辨識外端指派、版本不符或預覽驗證失敗時保留 broker 狀態並顯示錯誤，不顯示舊圖或改選其他 Screen。較舊的存活 Shell 若不回報 URL，相同本機 ID／revision 仍可恢復；外端 ID 則須使用者明確重新選擇 Screen，不需為此重啟工作 Shell。

預覽標題顯示所選 Screen，圖片下方不顯示擷取時間或工具接入說明，以保留畫面空間；擷取與操作錯誤仍顯示。立即擷取、定時與手動更新只在當前可見分頁進行；停止預覽只停止取圖，並不清除已配置的 AI 工具。「取消桌面工具」才向該 Shell broker 送 clear，清除其 token 並停止預覽。關頁、detach、App 關閉只釋放本機連線與預覽；Shell supervisor 結束時釋放其記憶體指派；CLI stdio EOF 不清除 Shell 選擇。所有操作都不停止共享桌面、瀏覽器、instance-mcp 或使用者工作 Shell。

## 原生 MCP 傳輸

App 原生層保存及解密 token，WebView 只接收非秘密 metadata 與已驗證圖片，不能讀回已存 token。每次操作協商 MCP `initialize`、`notifications/initialized` 與 `Mcp-Session-Id`，接受有界 JSON／SSE，核對 JSON-RPC id、error、`isError`、PNG／JPEG 與尺寸。預覽直接呼叫 screenshot，不以 `tools/list` 成功作為前置。

URL 不允許帳密、query 或 fragment。HTTPS 使用正常憑證驗證；HTTP 僅允許 loopback 或 Tailnet 位址 `100.64.0.0/10`、`fd7a:115c:a1e0::/48`。每次新連線解析並固定到驗證過的位址，不接受 redirect 或環境 proxy。App 與 adapter 使用同一規則。JSON／SSE 最多 16 MiB，截圖上限 20 秒；每次操作結束 best-effort DELETE MCP session，不因失敗重播工具呼叫。圖片及回應另受格式、大小與尺寸界限保護。

工具取消依逾時／關閉連線收尾；協定可用時 best-effort 發送 `notifications/cancelled`。取消不能保證撤銷上游已開始的操作。錯誤訊息不回顯 token 或完整私人設定。

## Windows／Android 選擇供 Shell 與 coding CLI 使用

Owner 最新範圍是 App 連入的工作 Shell，以及它裡面的 coding CLI；Discord Agent 暫不接入，整台 Host 共用的 broker 也不在範圍。新工作 Shell 在 Bash／CLI 啟動前就持有未配置的 supervisor broker，因此可先選 Screen，再由 Shell 命令或稍後啟動的 coding CLI 使用相同選擇。CLI 結束不清除 Shell 選擇，明確取消工具或 Shell 結束才拒絕新工具呼叫。

App 不新增 Codex 專用啟動按鈕，不自動執行 CLI。新 Shell 的 scoped wrapper 提供 Codex stdio 接線，不修改主機全域配置或恢復固定上游 token。既有正在執行的 Shell／AI 不注入命令；helper 安裝後需建立新連線。其他支援標準 MCP 的 CLI 可用 `art-term-screen mcp-config` 取得無 URL／token／固定 binding 的通用 stdio 入口，依各自原生設定登記一次；啟動時只解析當前 App Shell 的活 broker，不掃描 Host。不宣稱所有 CLI 零配置或既有 AI 熱插入。完整邊界見[Shell 工具 bridge](runtime-desktop.md)。

這是本次整合設計；實際安裝、PTY／Runtime 行為與手機驗收依當次結果確認，預覽成功不等於 Shell／CLI 工具已驗收。

## 相容與驗收邊界

舊 `desktop_transport` 繼續解析以保留 Host 相容，但不決定新版 Android／Windows 的 Screen 按鈕。兩平台明確選取 Screen 後只走直接 MCP，不啟用舊 `preview.py` 轉送路徑。原生後端仍保留未選 Screen 的 Windows 舊分頁相容路徑，依其既有 Host mapping 透過 Runtime 專用截圖 Shell 取圖；這不是新版 Screen 流程的前置或備援。Windows 已同步共用 Screen 清單／設定／選單，原生 EXE／DPAPI 與現場 Runtime 驗收由 Windows 電腦接續，不能把 Linux 或舊相容程式碼當成已驗收。既有 schema 5 不會自行產生 Screen。獨立 Screen 可透過連線匯出／匯入搬移設定，但來源 ID／revision 不隨檔案搬移；備份與降版條件見[設定遷移](connections.md#舊設定與版本升級)。

固定 client 的 MCP E2E 能驗證產品路由、完整工具結果及官方 Runtime configure 通道，不能代表真的 AI 模型或 Codex CLI 已實測。App／手機與現場 MCP 的驗收各自列出實際結果，不以舊截圖 helper 的通過紀錄代替。範圍見[測試矩陣](test-matrix.md)。

## 附錄：實作與參考

[預覽元件](../../apps/client/src/shared/components/PreviewPanel.vue)、[TUI 預覽](../../crates/art-term-tui/src/preview.rs)、[憑證保護](credential-storage.md)。
