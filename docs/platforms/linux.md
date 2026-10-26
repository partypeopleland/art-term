# Ubuntu 終端機客戶端

`art-term` 在 Ubuntu Server、Ubuntu Desktop 或 SSH 的終端機中管理既有遠端 Host／Shell。候選套件以 Ubuntu 24.04 x86_64 為建置與相容目標，不能推定較舊版本也能執行。三區工作區、表單和多分頁終端不需要 X、Wayland、GTK、WebKit、瀏覽器或桌面 keyring。受控端使用相同 Runtime 與通用 helper，沒有 TUI 專用安裝機制。

## 啟動與保存

解開候選套件，在互動式終端執行 `./art-term`。也可將執行檔放入自己的 `~/.local/bin`。不要以 `sudo` 啟動，設定與金鑰應屬於實際操作帳號。

登入自己的 Unix 帳號後，執行 `art-term` 即可使用，不另設啟動或解鎖密碼。啟動需要互動式 TTY；`--help` 與 `--version` 可在管線中使用。

預設目錄是 `${XDG_CONFIG_HOME:-$HOME/.config}/art-term-tui`。`connections.toml` 保存 Host／Screen 與密文；`connections.toml.account-key-v1` 保存本機隨機帳號金鑰；`connections.toml.vault-lock` 拒絕其他程序同時寫入。空值或相對路徑的 XDG 設定使用 `$HOME/.config`。

```sh
art-term --config /home/user/private-art-term/connections.toml
```

指定其他檔案時，先準備屬於自己的安全父目錄。帳號金鑰與鎖檔使用相同檔名加上上述副檔名。預設目錄為 `0700`，資料檔為 `0600`，拒絕不安全權限、symlink、hardlink 與特殊檔案。

帳號金鑰由 Unix 帳號與檔案權限保護，沒有額外密碼包裝；取得該帳號權限的程序也能讀取金鑰。請備份整個設定目錄。已有設定卻遺失或損壞金鑰時，程式拒絕開啟，不建立替代金鑰。

舊版 `.vault-v1` 只有在 `connections.toml` 尚不存在且 metadata 格式正確時，才可自動改用新帳號金鑰；原檔保留不動。已有設定的舊密碼 vault 須另行遷移，啟動不會要求舊密碼或覆寫資料。跨裝置或與 Windows／Android 交換連線，使用加密匯出／匯入；搬移檔仍使用獨立的檔案密碼。GUI 平台密文不能直接複製來解密。詳見[憑證保存](../refarch/credential-storage.md)。

## 樹狀選單與滑鼠

正常視窗固定分為左上連線樹、左下桌面預覽、右側分頁終端。Computer、Agent、Screen 一直展開；用 ↑／↓ 選取、Enter 執行，或直接點選。樹上提供 New Host、New Agent、New Screen、各 Host 的 New Shell、現有 Shell 與 Refresh Shells，不必先切分類或記住不同情境的新增按鍵。遠端清單在背景載入，失敗顯示重試入口，不會自動建立或接管工作。

頂端 Connections、Preview、Terminal 可點選切換焦點。分頁標題可點選切換，旁邊的 `[x]` 只關閉本機分頁、保留遠端工作。選擇既有 Shell／接管與刪除遠端工作仍須確認。Actions 選單包含新增、編輯、刪除、測試、重新整理、匯入／匯出、歷史操作、復原及離開，均可用滑鼠或方向鍵與 Enter。

表單可點欄位、Save、Cancel；確認視窗提供 Yes／Cancel，預設不確認。視窗開啟時不會把滑鼠、輸入或貼上送進背後終端。滑鼠滾輪在連線樹移動選取，在終端使用共用歷史 scroll／follow，在說明與選單捲動各自內容。點終端只取得焦點，不把本機滑鼠報告當成遠端輸入。

窄視窗顯示目前焦點區域，仍保留可點的 Actions 與 Desktop 入口。遠端 PTY 尺寸只取終端內容區，不含邊框、側欄與分頁列。背景分頁繼續處理輸出，恢復使用原工作 token，不自動建立或重播輸入。歷史中輸入先回到即時位置，狀態不確定時停止送入。

## 自訂快捷鍵

首次啟動會在連線設定同目錄建立 `keybindings.toml`，一般是 `~/.config/art-term-tui/keybindings.toml`。使用 `--config` 時就在指定連線檔旁。直接用文字編輯器修改，重新啟動生效；不會覆寫已有的快捷鍵檔。

預設 F1 開說明、F2 到連線樹、F3 到終端、F4 選桌面、F10 開 Actions。原本的 `Ctrl-]` 前導鍵仍可用；例如先按 `Ctrl-]`、放開、再按 `c` 選桌面。說明畫面顯示目前檔案設定的快捷鍵。

```toml
leader = "Ctrl+g"

[bindings]
hosts = ["Alt+h"]
terminal = ["F3"]
screens = ["F4", "leader+c"]
help = ["F9"]
close = ["leader+w"]
next = ["Alt+l"]
previous = ["Alt+p"]

[navigation]
save = ["Ctrl+s"]
cancel = ["Esc"]
```

`leader+c` 表示先按設定的 leader、放開，再按 c。`[bindings]` 每項可填多組鍵，`[]` 停用該操作的全域快捷鍵，仍可點選操作。未填的項目沿用預設；已產生的完整檔案及套件內 `keybindings.example.toml` 列出全部名稱。支援 Ctrl／Alt／Shift、單一字元、F1～F24、方向鍵、Tab、Enter 等。前導鍵連按兩次會把該鍵送進目前遠端終端。

`[navigation]` 增加本機選單與表單的導覽鍵，原有 Esc／Tab／Enter／Ctrl-S 等基本鍵保留；終端焦點的普通方向鍵繼續送給遠端。連線樹的基本 ↑／↓／Enter 保留可用。表單優先處理自己的按鍵，不觸發全域快捷鍵。格式錯誤、未知操作或重複快捷鍵會在啟動時指出檔案與錯誤原因，不靜默退回預設或覆寫原檔。

## 桌面設定、工具與預覽

連上工作 Shell 後，點右上 `[Desktop]` 或按 F4 選擇已保存的 Screen。即使桌面尚未產生截圖，也能先驗證可用的 MCP 工具並設定 Shell，預覽失敗不阻擋工具配置。這會透過與 Windows／Android 相同的受保護管理通道，配置「該 Shell」的桌面工具 broker；憑證不送進使用者工作終端。每個分頁獨立選擇，另一個 Shell 不會跟著改變。

終端標題持續顯示目前 Screen 與工具狀態。`Tools ready` 表示受控端確認該 Shell 的 broker 指派；`Tools unconfirmed` 表示尚未確認，預覽成功也不能代替工具設定成功。缺少通用工具或舊 Shell 沒有 broker 時，選單顯示處理提示；依共用安裝流程準備工具並建立新的工作 Shell，不能偷偷注入現有 AI 程序。這些狀態不代表每個 AI 模型已載入 MCP。

Stop 只停止本機預覽，既有 Shell 工具選擇保留；Resume preview 恢復取圖。Clear tools 是另一個需要確認的操作，只取消該 Shell 的桌面工具，不停止共享桌面、其他 Shell 或工作。關閉分頁、離開程式及 detach 也不清除遠端指派。再次連入會查詢既有 broker、核對目前設定與工具來源後恢復，不重新配置或改寫遠端 generation；無法辨識的外端指派要求使用者明確重選。

預覽直接呼叫共用 MCP `screenshot`，不經工作 PTY 傳圖。點左下圖片，會在右側工作區展開桌面，左側清單、縮圖與分頁保留。拖曳平移，滾輪以指標位置為中心縮放，範圍為適合視窗到 16 倍；`[Fit]` 還原。點右上 `[X]`、按 Esc 或再點左下圖片即可收回。縮放裁切原始截圖，不是放大縮圖；平移與縮放不向遠端送滑鼠、鍵盤、resize 或額外截圖請求。右側展開時，工作 Shell 仍處理背景輸出。

同一來源的圖片在切焦點、點擊及下次截圖等待期間保留，不閃回設定說明。換分頁、改來源、停止或發生錯誤時清圖；開表單或說明會收回右側圖片。僅目前可見、未停止且來源有效的分頁取圖。終端與表單將實體游標放在插入點，中文字以顯示寬度定位；這提供輸入法定位基礎，實際系統輸入法組字仍需對應終端程式驗收。

首次啟動在連線設定同目錄建立 `display.toml`，已有檔案不覆寫。預設 `image_mode = "auto"`，在啟動後向目前終端查詢 Sixel 能力與字格像素；本機與 SSH 使用相同判定，不用終端品牌白名單或 SSH 標記推論支援。畫面與鍵盤在偵測期間照常可用。約一秒內確認支援及有效尺寸後才顯示原生圖片，失敗則在左下說明原因。不使用字格拼圖，也沒有文字圖片備援。舊設定的 `compatible` 現在視為 `auto`，檔案仍保留原樣。

**預覽停用不影響 Screen 選擇、Shell 桌面工具配置或 AI 呼叫工具。** 不會為無法顯示的預覽額外擷取圖片。SSH 本身不是圖片限制；由你眼前的終端回覆能力，並實際繪製圖片。不支援 Sixel 的終端、沒有回覆或尺寸無效時才停用預覽，不能僅因連線是 SSH 而拒絕。

尺寸優先採用終端字格像素回覆，沒有有效回覆時才在偵測截止後使用終端實際回報的文字區像素與行列數計算，必須能整除。程式不使用 SSH 的系統像素欄位作圖片幾何依據，因為它可能是預設值。設定的 `cell_width`、`cell_height` 只作最後備援，必須填實際值；不能取代 Sixel 能力證據或覆蓋有效的終端尺寸回覆。`image_mode = "sixel"` 同樣必須先確認能力。設定錯誤只停用預覽並保留原檔，Host／Shell 照常可用。

每次啟動只偵測一次。一般視窗大小變動會重新配置圖片與終端區域；改變終端字型或字級後須重啟 art-term，重新取得字格像素。每個圖片區最多 160×80 字格所占的像素範圍，大視窗會置中顯示。

Sixel 編碼已內建，不需安裝外部轉圖程式。能力與尺寸回覆、鍵盤、滑鼠及貼上共用同一個 EventStream；偵測結束後的晚到回覆仍會被辨識，不會送入遠端 Shell。程式不修改終端或 multiplexer 設定；multiplexer 若沒有提供足夠回覆，會停用預覽。Kitty／iTerm2 協定未啟用。圖片操作只控制本機檢視，不操作遠端桌面。可在 `keybindings.toml` 的 `[bindings]` 自行配置 `viewer_toggle`、`viewer_fit`，預設未綁定。詳見[桌面流程](../refarch/desktop.md)。

## 建置與驗證

在 Ubuntu 24.04 x86_64 準備 Rust、C 編譯／連結工具、Python 3 與 Git。從 repository 根目錄執行：

```sh
cargo build -p art-term-tui --bin art-term --release --locked
./target/release/art-term --help
bash scripts/package-tui.sh --output-dir ./dist/tui/ubuntu-candidate
```

打包輸出 `.tar.gz`、manifest 與 SHA-256 清單。壓縮檔內含執行檔、說明、LICENSE、第三方說明及新增終端依賴授權，不含私人設定。只建置 `art-term-tui` 不會編譯 GUI crate。

```sh
cargo fmt --all -- --check
cargo check -p art-term-core --locked
cargo test -p art-term-core --locked
cargo test -p art-term-core --doc --locked
cargo test -p art-term-tui --locked
cargo tree -p art-term-tui
```

終端模型單元測試不代替實際 PTY 操作，TLS／WSS fixture 也不代替真實受控端與人工驗收。回報分別記錄免密碼啟動、三區焦點與桌面預覽、主機與工作管理、雙分頁輸入輸出、resize、歷史／輸入排序、恢復及退出還原。測試使用自己的 HOME、憑證、loopback Runtime 與 PTY，不能操作使用者已連線的工作。

以隔離 Python 環境安裝 `scripts/tui-regression/requirements.txt` 後，可重跑真實 PTY 與 loopback TLS／WSS 驗證。測試用 CA 僅存在 debug fixture feature，release 禁止啟用。

```sh
cargo build -p art-term-tui --features openab-fixture --locked
python3 scripts/tui-regression/run.py \
  --binary ./target/debug/art-term \
  --output-dir /tmp/art-term-tui-results
```

輸出目錄必須尚未存在。腳本保留驗證紀錄並清理自己的 PTY、測試資料與 fixture；正式候選套件另用不含 fixture feature 的 release 建置。

## 附錄：實作與參考

[TUI 入口](../../crates/art-term-tui/src/main.rs)、[共用核心](../../crates/art-term-core/src/openab/mod.rs)、[打包腳本](../../scripts/package-tui.sh)、[Ratatui](https://docs.rs/ratatui/0.30.2/ratatui/)、[tui-term](https://docs.rs/tui-term/0.3.4/tui_term/)、[終端歷史](../refarch/terminal-history.md)。

完整功能對照、案例 ID 與實際 TUI／官方 Runtime 執行入口見 [TUI 功能回歸](tui-regression.md)。
