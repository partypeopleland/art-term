# 工作 Shell 的記憶體 MCP 工具 bridge

Owner 本次範圍是 App 連入的工作 Shell，以及該 Shell 裡的 coding CLI；暫不包含 Discord Agent，也不使用整台 Host 共用的工具選擇。Android 選定 Screen 的截圖仍由 App 原生層直接向 MCP 取得。工具路徑則由每個新工作 Shell 自己持有 broker，讓先選 Screen、後啟動 CLI 的操作也能成立。

本文描述本次整合設計；安裝、真 Runtime／PTY 及手機驗收須依當次結果確認，不能以舊 standalone adapter 或固定 client 成功推定現場已完成。

## Broker 先於 Bash 與 CLI 存在

```text
new tmux work Shell (binding = full th2-... name)
    |
    +-- Shell-owned supervisor broker (initially no target)
           |
           +-- interactive bash -> generic Shell tool client
           |                    -> coding CLI -> scoped stdio proxy
           |                                           |
           +-- owner-only socket <---------------------+
           |         ^
           |         +-- temporary raw/no-echo configure Shell <--- App
           |
           +-- memory-only selected target/token -- MCP HTTP --> instance-mcp

App native -- direct screenshot -------------------------------> instance-mcp
```

新 tmux 工作建立時先啟動 supervisor，再啟動互動 Bash。broker 綁定完整工作 Shell 名稱，不含 Runtime generation，因為 renew 不替換原 Shell 程序；不是 Host 全域服務，不另外建立常駐 Runtime Shell。不同工作 Shell 的選擇隔離，可以同時指向相同 Screen，沒有獨占桌面或重設瀏覽器。

broker 初始沒有上游授權，未選擇時工具拒絕呼叫且不連上游。App 配置成功後，URL／token 只存在此 broker 的行程記憶體。晚啟動的 Shell client 或 CLI stdio proxy 連同一個 broker；CLI stdio EOF 只斷開該 client，不清除 Shell 的選擇。明確 clear 拒絕後續工具呼叫；Bash 結束、SIGTERM／SIGHUP 或 tmux／Runtime 結束時清記憶體、移除 socket，不留下孤兒 broker。

Unix socket 位於 owner-only 0700 runtime 目錄，驗證 owner、mode 及非 symlink，client 只連相符 binding；同 UID 程序仍是信任邊界。通用程式、非秘密 binding 與 stdio 定義可在磁碟／argv／環境中，選定上游 URL、token 及認證 header 不得放入這些位置、工作 tty、history 或 log。已移除的 NUC／Hana 固定上游設定及 token 環境不可重新引入。

## App 配置與來源核對

App 以 `exec python3` 啟動短命 configure Shell；接收端先關 echo／canonical mode，完成後才印 nonce READY。App 等 READY 再送有界 length-prefixed base64 JSON frame，最多 16 KiB、10 秒；token 為可見 ASCII、最多 1024 bytes。configure 只回非秘密結果，結束、失敗或取消都清理自建管理 Shell，不把秘密輸入使用者工作終端。

配置回報的 Screen id／revision／選擇 generation 核對後，才能確認 Shell 接入；不存在 broker 不得假報 ready。generation 是工具選擇版本，不是 Runtime generation；過期 rollback／clear 不能撤銷較新選擇。App 重啟或接管時先查詢該 Shell broker 的現有指派，核對 Screen id／revision、當前憑證與上游；已刪除或改版的指派會清除，不能沿用失效認證。一般 Main 切換、App 背景、關頁、detach 與關閉 App 只停本機取圖及連線；明確取消工具才 clear。Shell 程序結束時釋放其記憶體指派。

## Shell 與 coding CLI 使用同一選擇

通用 Shell client 提供 current／list／call 類操作，CLI 透過 scoped stdio proxy 使用同一 broker。固定 MCP 工具仍為 `current_target`、`list_target_tools`、`call_target_tool`；call 必須帶 `expected_target`，不符就回 `isError`，不能改送新目標。list 保留 cursor，list／call 在單次操作開始固定來源；切換中的舊呼叫留在原上游完成並標來源，新呼叫才用新選擇。

完整保留 MCP `content`／`isError`／`structuredContent` 與來源 metadata，包括圖片；App 直接截圖避免 PTY 傳圖，不表示工具結果全面過濾圖片。工具呼叫最多 60 秒、list 最多 15 秒、JSON／SSE 最多 16 MiB。URL／DNS／TLS／proxy 與取消規則和[直接預覽](desktop.md#原生-mcp-傳輸)相同，不重播有副作用的呼叫，也不宣稱取消能撤銷上游已開始的操作。

新工作 Shell 的專用 rc 先載入一般 `~/.bashrc`，再提供 Shell 工具及 scoped Codex stdio 接線。使用者可照常輸入 Codex，不需要 App 額外啟動按鈕；CLI 不會自動執行。通用 MCP 定義只帶程式路徑與 binding，不含上游憑證，保留使用者參數及一般設定，包括直接輸入 `codex --yolo`，不修改全域 Codex、Hana 或 Discord 配置。其他 CLI 只在確認支援的配置方式後整合，不宣稱所有引擎或既有 AI 能熱插入。Devin 不自動接線，也不代寫其 project／global 設定；其他支援標準 MCP 的 CLI 可依下一節，用各自原生配置一次登記通用入口。

## 給其他 CLI 的標準 MCP 入口

在已具備 broker 的 App 工作 Shell 執行：

```bash
art-term-screen mcp-config
```

它輸出可登記到其他 MCP client 的非秘密 `command`／`args`：command 是 `python3`，args 是本機 `instance_bridge.py` 完整安裝路徑、`stdio`、`--current-shell`。不含固定 Shell binding、上游 URL、token 或 header。依該 CLI 的原生 MCP 設定方式登記一次，之後從 App 工作 Shell 啟動的 client 才能解析當前 Shell；不是所有 CLI 都會自動發現設定，也不承諾零配置。Codex 保留上述 scoped 自動接線與使用者參數。

`stdio --current-shell` 只接已存在的 App Shell broker，不退回獨立 adapter，不掃描整台 Host 的 grants。它驗證 `ART_TERM_SCREEN_BINDING`，或在 SDK 過濾環境時沿同 UID 祖先程序解析；最多 32 層、每層讀取 262 KiB、總期限 1 秒，核對 UID、parent pid、start time，且 socket peer 必須是已驗證祖先中的活 broker。找不到唯一可信來源就拒絕，不借用其他 Shell 的選擇。

標準 `/tmp` 或 `/run/user/<uid>` 路徑可在 SDK 過濾環境後解析；自訂 runtime 路徑仍需保留 `XDG_RUNTIME_DIR`。離開原祖先鏈、被重新掛接或獨立背景啟動的 client 可能拒絕；不要改成 Host-wide 掃描或寫固定認證繞過。這些解析與配置契約不代表所有 CLI 或真 AI 模型已驗收。

## 安裝與既有連線

使用者帳號的通用程式位於 `~/.local/share/art-term/tools/`。在已完成 tmux 主機準備的受控端 checkout，以工作 Shell 的一般使用者執行：

```bash
bash scripts/install-instance-tools.sh --with-tmux-wrapper
```

安裝 `instance_bridge.py`、`instance-shell.py`、`instance-shell.bash`，另將第四個通用 helper 安裝為持久化的 `~/.local/share/art-term/tools/tmux-shell.py`；不帶旗標則只安裝 tools 三檔，不新增此建立入口。App 新建 Shell 優先使用 tools 的 wrapper，缺少時退回原有版本 wrapper。既有 `openab-pty/tmux-v2`、其 image symlink 及 tmux binary 保持原處，不修改映像或要求 Pod 重啟。腳本驗證 owner／非 symlink 及不可由其他帳號寫入，檢查 Python／Bash 語法，以原子替換安裝為 0700 並輸出 SHA256。被替換的既有檔先備份為同目錄 `<原檔名>.pre-shell-screen-<UTC時間>-<隨機識別>`，權限 0600；內容相同不另建備份。不保存私人上游設定、不重啟既有 Shell、Pod 或 Runtime，也不更改全域 CLI／Discord 配置。本次穩定介面為 `instance_bridge.py shell --binding th2-<key>` 啟動 supervisor；`stdio --binding <binding>` 連現有 supervisor，舊 standalone 模式仍供相容。Shell rc 提供 `art-term-screen current`／`list`／`call`；底層 `shell-command --binding <binding> current|list|call|mcp-config` 的 call 從 stdin 讀 `{name, arguments, expected_target}` JSON。先讀 current／list 再帶該來源呼叫，不將 token 放入呼叫資料。上述是目前交付介面，不套用舊 CLI-owned adapter 的生命週期。

既有正在執行的 Shell／CLI 不注入啟動定義、不偷偷輸入命令。helper 安裝後需建立一次新連線，才有 Shell-owned broker；缺 helper 的工作仍可用普通 Bash，但工具不可假報已接入。若一般 `~/.bashrc` 已定義 Codex alias／function，rc 保留它而不覆蓋，因此會繞過接線；rc 載入後執行 `command codex`、絕對路徑，或再次定義同名 alias／function，可能繞過 scoped wrapper；自訂啟動器需明確確認其接線，不承諾每種用法自動接入。

tmux 畫面、歷史與 job control 保持原責任；supervisor 透過自有 PTY 維持 Bash 的 job control、resize 與前景程序群組；pipe-death guard 在 supervisor 被 SIGKILL 時清理其自有工作 session。這不是額外常駐 Runtime Shell，tmux 仍管理畫面與歷史。須驗證 Ctrl+C 返回 Bash、退出狀態與 kill 清理。舊 Windows 未選 Screen 分頁的 Runtime 截圖相容路徑保留，Android 新 Screen 不依賴它；新工具設計不使用舊 history_journal 或 Host 共用代理。

## 附錄：參考

[桌面選擇](desktop.md)、[終端歷史](terminal-history.md)、[測試矩陣](test-matrix.md)、[通用程式](../../scripts/instance_bridge.py)。固定 client、CLI argv fixture 及真 Runtime／PTY 驗證不等於真 AI 模型、所有 CLI 或 Owner 手機已實測。
