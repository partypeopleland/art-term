# Ubuntu 桌面平台

Ubuntu 版使用與 Windows 相同的桌面畫面、Rust 核心和 Runtime 契約。主畫面分類、設定、四種主題、多分頁終端、歷史捲動、Screens 及連線匯入／匯出沿用共用實作。Linux 平台層負責可寫設定位置、憑證保護與系統瀏覽器；受控端不需要額外的 Linux client 專用 helper。

目前套件目標為 Ubuntu 24.04 x86_64，格式為 `.deb`。建置成功、虛擬顯示器中的原生測試與使用者在實際桌面上的人工驗收是不同階段，不能互相替代。特定候選版本的檢查結果記錄在對應 Issue。

## 桌面與金鑰環境

App 需要圖形桌面、GTK 3、WebKitGTK 4.1，以及使用者 session D-Bus 上的 Secret Service。Ubuntu 的 GNOME Keyring 可提供持久金鑰集合；其他實作也須提供可解鎖的 persistent default collection。套件推薦安裝 `fonts-noto-cjk`，避免精簡系統缺少中文字型而顯示方框。只有 SSH 或 TTY 的環境沒有可顯示 App 的桌面，安裝套件不會自動建立遠端桌面。

從桌面登入後開啟 **art-term**，或在該桌面的終端執行 `art-term-gui`。首次開啟時，keyring 可能要求解鎖。連線清單初始為空，從設定新增 Host 或 Screen。App 不包含私人連線資料，也不把安裝目錄當成可寫設定來源。

終端 HTTP(S) 網址由 GIO 交給系統預設瀏覽器。桌面預設瀏覽器需已完成關聯設定；App 不透過 shell 拼接網址。快捷鍵、滑鼠、xterm 與工作區配置沿用[桌面操作](windows.md#畫面與平台操作)。Android 的語音、原生閱讀器與觸控 pager 保留在手機平台。

## 設定與憑證位置

預設位置依編譯後的 Tauri App identifier 區分，目前為 `net.missart.artterm.desktop`。

| 資料 | 預設位置 |
|---|---|
| 唯一連線來源 | `${XDG_CONFIG_HOME:-$HOME/.config}/net.missart.artterm.desktop/connections.toml` |
| 金鑰使用 marker | `${XDG_DATA_HOME:-$HOME/.local/share}/net.missart.artterm.desktop/operator-key-used-v1` |
| 主金鑰 | 使用者 Secret Service 的 persistent default collection |

空值或相對路徑的 XDG 變數採用 `$HOME` 下的預設目錄。App 專用目錄為 `0700`，設定檔與 marker 為 `0600`。檔案須屬於目前使用者，不能是 symlink、hardlink 或其他特殊檔案。

使用 `--config` 可指定另一個可寫 TOML，仍沿用同一個使用者與 App 的金鑰。先準備屬於自己的目錄，且不允許其他使用者寫入。例如：

```sh
art-term-gui --config /home/user/private-art-term/connections.toml
```

Secret Service 只保存主金鑰。Host 管理憑證與 Screen token 以 AES-256-GCM 密文保存在 TOML，驗證情境綁定資料的身分與版本。marker 只記錄 key 指紋，不是金鑰備份。keyring 不可用、未解鎖、key 重複或遺失時，設定讀寫會失敗並保留原資料，不退回明文儲存。初始化與修復規則見[憑證保存](../refarch/credential-storage.md)。

跨裝置使用[連線匯出／匯入](../refarch/connection-transfer.md)。Windows DPAPI、Android Keystore 與 Linux keyring 的密文不能直接互用，單獨複製 TOML 不會搬移解密能力。卸載前保留需要的加密匯出檔；刪除 key 或 marker 不是修復損壞的方式。

## 建置 Debian 套件

在 Ubuntu 24.04 x86_64 安裝 Git、Rust、Node/npm，以及 [Tauri Linux 開發依賴](https://v2.tauri.app/start/prerequisites/#linux)。建置需要 GTK 3 與 WebKitGTK 4.1 的開發套件。Secret Service 在執行 App 時使用，純編譯不需要登入圖形桌面或解鎖個人 keyring。

從 repository 根目錄執行：

```sh
bash scripts/package-linux.sh --output-dir ./dist/linux/ubuntu-candidate
```

打包使用 Linux 專屬 Tauri 設定，輸出 `.deb`，不套用 Windows NSIS 目標或旁置設定。交付目錄包含 `.deb`、README、LICENSE、第三方說明、`manifest.json` 與 `SHA256SUMS`。manifest 記錄平台、版本、檔案與雜湊；候選來源提交另由交付回報記錄，安裝前核對兩者。套件以 Ubuntu 24.04 為相容目標，不能由建置成功推定較舊發行版也可執行。

開發基本檢查從 repository 根目錄執行：

```sh
npm --prefix apps/client ci
npm --prefix apps/client run typecheck
npm --prefix apps/client test
npm --prefix apps/client run build
cargo fmt --all -- --check
cargo check -p art-term-core --locked
cargo test -p art-term-core --locked
cargo test -p art-term-core --doc --locked
cargo test -p art-term-gui --bin art-term-gui --locked
```

原生驗證使用隔離的 HOME、D-Bus、持久 keyring 與圖形顯示環境，避免接觸既有設定或登入中的桌面。至少涵蓋 Host 與 Screen 保存後重啟、key 遺失時拒絕替換、設定搬移、實際 WebKitGTK 畫面及預設瀏覽器啟動。受控端 Runtime 與 helper 的獨立驗證另見 [Linux 回歸測試](linux-regression.md)。

桌面驗證腳本需要 Python 3、`dbus-daemon`、`gnome-keyring-daemon`、`gdbus`、`Xvfb`、`xauth`、`xdotool` 與 `WebKitWebDriver`（Ubuntu 的 `webkit2gtk-driver` 套件）。它會建立自己的顯示器、session bus 和 keyring，測試畫面、Host／Screen 操作、重啟保存、瀏覽器及剪貼簿。輸出目錄必須尚未存在；截圖和紀錄保留於該目錄，臨時使用者資料與程序會清除。

```sh
python3 scripts/linux-regression/client.py \
  --binary ./target/release/art-term-gui \
  --output-dir /tmp/art-term-client-results
```

也可先解開 `.deb`，將 `--binary` 指向其中的 `usr/bin/art-term-gui`，驗證實際交付程式。這項測試不會連線真實受控端，也不代替 Runtime Shell／歷史操作或使用者人工驗收。

## 附錄：實作與參考

[桌面入口](../../crates/art-term-gui/src/desktop/main.rs)、[Linux 平台層](../../crates/art-term-gui/src/platform/linux/mod.rs)、[Linux 打包設定](../../crates/art-term-gui/tauri.linux.conf.json)、[打包入口](../../scripts/package-linux.sh)、[Secret Service 規格](https://specifications.freedesktop.org/secret-service/latest-single/)、[XDG 目錄規格](https://specifications.freedesktop.org/basedir/latest/)、[Tauri Debian 打包](https://v2.tauri.app/distribute/debian/)。
