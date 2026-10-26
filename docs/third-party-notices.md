# 第三方來源與授權

來源清冊沿用既有核對，本輪只更新repo內複用及散布範圍，未重新查核外部授權。

- [openab-pty](https://github.com/openabdev/openab-pty)：MIT，Copyright (c) 2026 openabdev。Host runtime使用未修改的固定官方產物；來源與image／layer／binary digest記錄於scripts/openab-runtime/official_artifact.py。保留完整歸屬與許可文字於LICENSE。
- [openab-pty NOTICE](https://github.com/openabdev/openab-pty/blob/main/NOTICE)：repository code與聚合container image授權不同，映像內第三方CLI仍依其條款。Client不內嵌這些映像；另行散布Host映像須核對NOTICE。
- [instance-mcp](https://github.com/openabdev/instance-mcp)：MIT，Copyright (c) 2026 openabdev。Native Runtime helper 呼叫既有 MCP 桌面能力，不把該binary內嵌client。
- Vue、xterm.js、Tauri、Tokio、reqwest、rustls等依Cargo.lock及package-lock.json鎖定，各自license仍適用；此清冊不將依賴授權改為專案MIT。

Windows、APK 與 TUI 交付保留 LICENSE 與 THIRD-PARTY-NOTICES.md。

Runtime HTTPS／WSS 使用 rustls／ring；uuid 與 zeroize 為 Apache-2.0 OR MIT。原套件許可與歸屬仍適用。App 不再包含 russh／ssh-key 或呼叫系統 OpenSSH。

連線可攜檔使用 RustCrypto 的 argon2、aes-gcm 與 aes，以及 getrandom；原生檔案選擇／讀寫使用 tauri-plugin-dialog 與 tauri-plugin-fs。版本由 Cargo.lock 鎖定，各套件原有許可與歸屬仍適用。

## tmux 主機歷史支援

選配的 Linux 主機支援使用 tmux 3.6a；其 ISC／BSD 授權與所含來源通知需隨 binary 保存。若靜態連入 libevent 與 ncurses，亦需附上對應授權。私有 Node 22 應保留 Node 及其第三方通知。`tmux-history.cjs` 包含 xterm，請一併保存本專案與 xterm 的授權；APK 本身不內嵌 tmux 或 Node。

## 終端機客戶端

文字介面使用 Ratatui 0.30.2、其 core／widgets／crossterm backend、Crossterm 0.29.0、tui-term 0.3.4 與 vt100 0.16.2，皆為 MIT。隱藏密碼輸入使用 rpassword 7.5.4 與 rtoolbox 0.0.6，為 Apache-2.0。實際來源版本由 Cargo.lock 鎖定。

上述新增依賴的完整授權文字從各已下載套件核對並保存在 `crates/art-term-tui/licenses`，TUI 壓縮套件原樣附帶該目錄。既有共用核心依賴仍適用原有授權；此段不表示已重新稽核整個相依圖。
