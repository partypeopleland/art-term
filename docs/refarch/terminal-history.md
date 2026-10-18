# tmux 終端歷史與按需捲動

新建 Shell 統一使用主機上的私有 tmux，保留有限主機歷史。Windows 與 Android 都遵循同一套歷史快照介面，先取當前畫面，向上才分批取較早紀錄。Windows 即時畫面使用 xterm，歷史以桌面元件呈現；Android 以原生 View 呈現。即時輸入輸出仍走官方 Runtime。

## 閱讀與恢復

```text
建立並連線 -> Runtime 工作 Shell -> 私有 tmux / bash
                                          |
App 要求歷史 -> 讀頁 helper -> tmux 原生閱讀快照 -> 當前畫面
App 向上捲動 -> 同一快照指定範圍擷取            -> 最多 256 行
App 確認畫面已顯示 -> 保留顯示快照、釋放前一份快照
```

每個工作使用獨立 socket 與工作目錄，不使用個人的 tmux 設定。輔助程序透過 tmux 3.6a 的原生 copy-mode 複製閱讀 backing grid，最多保留兩份（已顯示與待顯示）；捕捉只讀本次要求的列及必要的一列換行邊界，不先讀全部歷史。回傳的 epoch 是這份閱讀快照的識別，較早頁必須屬於同一份快照，避免持續輸出造成頁面重疊或漏列。

Android 原生閱讀窗口最多 1024 行；上捲前插舊頁、持續輸出、開關鍵盤及視窗尺寸改變時，依原生列識別與像素位置維持閱讀點。停在底部才跟隨最新輸出。畫面呈現完成才確認快照，避免尚未顯示的頁面取代使用者正在閱讀的資料。

Windows 上捲時以 `DesktopHistoryReader` 呈現共用 `HostHistoryWindow` 的歷史快照，使用與 Android 相同的 `viewport / older / ack / keep / close`。即時 PTY 仍持續交給 xterm；閱讀時維持行識別與像素位置，向下回到最新內容後返回即時畫面。畫面可選取、複製及呈現文字樣式，歷史窗口最多 1024 行。

受控端 helper 提供固定的歷史介面，統一處理 tmux、工作驗證、快照擷取與清理。外部平台只提出讀取、分頁、確認、保活和關閉要求；Windows 不需要額外的原生滑鼠能力，也不改受控端 mouse／copy-mode／WheelUpPane 設定。

Android 與 Windows 共用快照與分頁規則，平台差異僅在呈現及操作事件。兩平台均在資料實際顯示後確認，並只關閉自己持有的讀頁程序。Windows GUI 與 Android 原生 View 仍須分別驗證平台呈現；共用協定通過不代表所有平台操作均已驗收。

新建工作的生命週期以原始 Shell 為準：退出 Bash 時會結束該工作的私有 tmux session，閱讀快照不會讓已退出的工作繼續存活，App 因此能關閉對應分頁。退出 Codex 等前景程式回到 Bash，或只釋放閱讀快照，都不會結束工作或影響其他分頁。

關閉分頁只脫離工作並清理自己的讀頁程序。App 強制結束時，未收到有效請求的輔助程序在 120 秒後清理自身；新 App 可立即建立新的讀頁程序接回仍活著的工作。此流程不會終止其他分頁的輔助程序。

工作顯示名稱自動產生為 `s-` 加八位十六進位字元，重新連線維持相同名稱。Runtime 使用完整的 `th2-` 加 24 位識別，不能用短顯示名稱推測工作身分。既有一般 Shell 或早期 `th-` 工作仍可連線，但不自動轉成新協定；需要新式歷史時請建立新 Shell，再於其中恢復自己的 CLI 工作。

## 新 Shell 的桌面工具生命週期

本次 Screen 整合在新 tmux 工作中先啟動 Shell-owned supervisor，再啟動互動 Bash，binding 使用完整 `th2-` 工作名稱。Shell client 及其 coding CLI 共用該 Shell 的記憶體選擇；不是 Host 全域服務。CLI EOF 不清除選擇，明確 clear 拒絕新工具呼叫，Shell／tmux／Runtime 結束則移除 socket 並清記憶體。App 背景或切 Main 只停止取圖。

歷史仍由原 tmux pane 與讀頁 helper 管理，supervisor 以自有 PTY 維持 Bash 前景程序群組及 resize，pipe-death guard 清理 supervisor 非正常結束後的自有 session，不增加常駐 Runtime Shell、不改 namespace／歷史 manifest；Ctrl+C、job control、resize、退出狀態與 kill 清理須另經真 PTY／Runtime 驗證。Screen 工具安裝另將建立 wrapper 放在持久化 `~/.local/share/art-term/tools/tmux-shell.py`，App 新建時優先使用，缺少時退回原有版本 wrapper；原 tmux-v2 binary／讀頁 helper 及可能指向 image 的 symlink 不改動，不需要更新映像或重啟 Pod。安裝不重啟既有工作；已執行的 Shell 不注入新啟動定義，需建立新連線，缺 helper 時仍可普通 Bash 相容。設計、工具介面與驗收界線見[Shell 工具 bridge](runtime-desktop.md)。

## 保存界線

tmux 的歷史上限設定為 5000 行。這是有限的主機記憶體歷史，tmux 可能批次淘汰較早列，因此不保證每次恰好保留 5000 行。工作仍活著時，App 重開或 Runtime 輸出緩衝區出現 Gap，可重新讀取 tmux 當前畫面及仍保留的歷史；不保證已被 tmux 淘汰的內容。

Host 重開、Runtime 工作到期或被刪除、tmux 工作退出、程式主動清除歷史，都不保證可以恢復。全螢幕 alternate-screen 程式的內容也不等於一般終端歷史。右上角 Codex 按鈕使用 `codex --yolo --no-alt-screen`，讓 Codex 使用一般終端畫面；此按鈕只在使用者按下時執行，重連不會再次啟動。

## 主機準備

目前主機支援為 Linux；需要 Python 3、私有 Node 22、tmux 3.6a 與以下兩個專案輔助檔。官方 Runtime 不需要修改或重啟。首次安裝 Runtime 仍依[主機安裝指南](../openab-host-installation.md)；本節只補上 tmux 支援。

```text
$HOME/.local/share/art-term/openab-pty/
  node                       私有 Node 22 執行檔
  tmux-v2/
    tmux                     私有 tmux 3.6a 執行檔
    tmux-shell.py            建立工作
    tmux-history.cjs         按需讀頁
```

在專案 checkout 先建置自帶依賴的 helper；Host 不需要 `node_modules`。建置端需要符合專案要求的 Node 與 npm：

```bash
npm --prefix apps/desktop ci
node scripts/openab-runtime/build-tmux-history.mjs --paged
```

從可信的官方來源準備與 Host 架構相符的 Node 22 及 tmux 3.6a。此版本驗證的 tmux 原始碼為官方 `tmux-3.6a.tar.gz`，SHA256 為 `b6d8d9c76585db8ef5fa00d4931902fa4b8cbe8166f528f44fc403961a3f3759`。編譯需 libevent、ncurses 與 bison；可在隔離建置目錄準備依賴，將產物放在上述私有位置，並核對動態函式庫可用及 `tmux -V`。不要替換系統 tmux 或連接使用者既有 tmux server。

由同一 Runtime 執行帳號安裝，目錄與執行檔權限均為 `0700`；來源與產物授權說明一併保存。若已有私有 Node，先核對版本與來源，不能覆寫。以下範例假設 Node 與私有 tmux 已準備好，僅安裝 helper，目的檔已存在時停止：

```bash
python3 - <<'PY'
import os
from pathlib import Path
os.umask(0o077)
base = Path.home() / '.local/share/art-term/openab-pty/tmux-v2'
base.mkdir(parents=True, exist_ok=True, mode=0o700)
for source, name in [
    ('scripts/openab-runtime/tmux-v2/tmux-shell.py', 'tmux-shell.py'),
    ('target/tmux-history/tmux-v2/tmux-history.cjs', 'tmux-history.cjs'),
]:
    with (base / name).open('xb') as output:
        output.write(Path(source).read_bytes())
    (base / name).chmod(0o700)
PY
```

安裝後用 App 建立一個專用測試工作，分批輸出編號行、上捲讀取舊頁、關頁後重連並核對彩色與行序；只清理這個測試工作。若新建失敗，核對執行帳號、私有路徑、版本、執行權限與函式庫。不要以重啟共享 Runtime 或刪除舊工作作為排查第一步。Windows Host 安裝及其他 tmux 版本尚未驗收。

## 附錄：實作與參考

[主機 wrapper](../../scripts/openab-runtime/tmux-v2/tmux-shell.py)、[讀頁 helper](../../scripts/openab-runtime/tmux-v2/tmux-history.ts)、[協定驗證](../../crates/art-term-core/src/openab/history.rs)、[App 原生閱讀器](../../apps/desktop/src/components/NativeTerminalReader.vue)、[工作退出與歷史快照回歸](../../scripts/instance-mcp-regression/history_exit.py)。
