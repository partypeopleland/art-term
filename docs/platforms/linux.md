# Ubuntu 終端機客戶端

`art-term` 在 Ubuntu Server、Ubuntu Desktop 或 SSH 的終端機中管理既有遠端 Host／Shell。候選套件以 Ubuntu 24.04 x86_64 為建置與相容目標，不能推定較舊版本也能執行。文字清單、表單和多分頁終端不需要 X、Wayland、GTK、WebKit、瀏覽器或桌面 keyring。受控端使用相同 Runtime 與通用 helper，沒有 TUI 專用安裝機制。

## 啟動與保存

解開候選套件，在互動式終端執行 `./art-term`。也可將執行檔放入自己的 `~/.local/bin`。不要以 `sudo` 啟動，設定與金鑰應屬於實際操作帳號。

首次啟動輸入並確認新密碼，之後用相同密碼解鎖。密碼從控制終端讀取且不回顯，沒有 argv、環境變數或密碼檔案選項。啟動需要互動式 TTY；`--help` 與 `--version` 可在管線中使用。

預設目錄是 `${XDG_CONFIG_HOME:-$HOME/.config}/art-term-tui`。`connections.toml` 保存 Host／Screen 與密文；`connections.toml.vault-v1` 保存密碼加密的主金鑰；`connections.toml.vault-lock` 拒絕其他程序同時寫入。空值或相對路徑的 XDG 設定使用 `$HOME/.config`。

```sh
art-term --config /home/user/private-art-term/connections.toml
```

指定其他檔案時，先準備屬於自己的安全父目錄。key metadata 與鎖檔使用相同檔名加上上述副檔名。預設目錄為 `0700`，資料檔為 `0600`，拒絕不安全權限、symlink、hardlink 與特殊檔案。

請保留整個目錄並記住密碼。錯誤密碼、損壞或遺失 key metadata 不會重建金鑰，刪除檔案不能重設密碼。跨裝置或與 Windows／Android 交換連線，使用 App 的加密匯出／匯入。GUI 平台密文不能直接複製來解密。詳見[憑證保存](../refarch/credential-storage.md)。

## 主機、工作與分頁

主機清單提供新增、編輯、刪除及連線測試。選擇 Host 後列出遠端 Shell；建立、選擇既有工作和接管都是明確操作，重新整理清單不會自動建立或接管。原協定取得新 token 需續期，因此附著中的工作先要求接管確認。

每個本機分頁有自己的終端模型與工作身分，背景分頁繼續處理輸出。切換分頁不更換遠端工作；關閉分頁只脫離，遠端 Shell 繼續依 Runtime 生命週期運行。刪除遠端 Shell 是另外的確認操作。

新工作透過現有 helper 啟動 tmux 歷史功能。舊的一般 Shell 可連線，缺少歷史能力時明確顯示不可用。上捲與回到底部使用共同 scroll／follow 契約；歷史中輸入先回到即時位置，結果不確定時停止輸入，避免重送。斷線恢復沿用原工作 token，不自動新建或重播輸入。

終端處理 ANSI、Unicode、游標、替代畫面、尺寸變更及支援的終端查詢。快捷鍵以畫面內的 Help 為準，SSH／上層終端可能無法分辨部分修飾鍵。純文字版保留 Screen 設定與加密搬移資料，首版不提供圖形截圖預覽。

按 `Ctrl-]` 再按 `?` 開啟說明，`Ctrl-]` 再按 `q` 離開。前綴後按 `h` 回主機、`s` 列出 Shell、`n` 建立工作、`1`～`9` 選分頁、`p`／`l` 切換前後分頁、`w` 脫離目前分頁；連按兩次 `Ctrl-]` 將該按鍵送給遠端。一般 `Ctrl-C`／`Ctrl-D` 保留給遠端程式。主機頁按 `n` 新增、`e` 編輯、`t` 測試、Enter 列出 Shell；表單以 Tab 換欄、`Ctrl-S` 保存、Esc 取消。歷史使用前綴後的 `u`／`d` 上下捲動、`f` 回到底部。

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

終端模型單元測試不代替實際 PTY 操作，TLS／WSS fixture 也不代替真實受控端與人工驗收。回報分別記錄解鎖、主機與工作管理、雙分頁輸入輸出、resize、歷史／輸入排序、恢復及退出還原。測試使用自己的 HOME、憑證、loopback Runtime 與 PTY，不能操作使用者已連線的工作。

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
