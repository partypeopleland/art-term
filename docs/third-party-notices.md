# 第三方來源與授權

既有來源清冊沿用先前核對。Linux 新增的直接依賴授權依下載套件的 Cargo manifest 與 LICENSE 核對，未重做全部間接依賴的授權盤點。

- [openab-pty](https://github.com/openabdev/openab-pty)：MIT，Copyright (c) 2026 openabdev。Host runtime使用未修改的固定官方產物；來源與image／layer／binary digest記錄於scripts/openab-runtime/official_artifact.py。保留完整歸屬與許可文字於LICENSE。
- [openab-pty NOTICE](https://github.com/openabdev/openab-pty/blob/main/NOTICE)：repository code與聚合container image授權不同，映像內第三方CLI仍依其條款。Client不內嵌這些映像；另行散布Host映像須核對NOTICE。
- [instance-mcp](https://github.com/openabdev/instance-mcp)：MIT，Copyright (c) 2026 openabdev。Native Runtime helper 呼叫既有 MCP 桌面能力，不把該binary內嵌client。
- Vue、xterm.js、Tauri、Tokio、reqwest、rustls等依Cargo.lock及package-lock.json鎖定，各自license仍適用；此清冊不將依賴授權改為專案MIT。

Windows、APK 與 Linux 交付保留 LICENSE 與 THIRD-PARTY-NOTICES.md。

Runtime HTTPS／WSS 使用 rustls／ring；uuid 與 zeroize 為 Apache-2.0 OR MIT。原套件許可與歸屬仍適用。App 不再包含 russh／ssh-key 或呼叫系統 OpenSSH。

連線可攜檔使用 RustCrypto 的 argon2、aes-gcm 與 aes，以及 getrandom；原生檔案選擇／讀寫使用 tauri-plugin-dialog 與 tauri-plugin-fs。版本由 Cargo.lock 鎖定，各套件原有許可與歸屬仍適用。

## tmux 主機歷史支援

選配的 Linux 主機支援使用 tmux 3.6a；其 ISC／BSD 授權與所含來源通知需隨 binary 保存。若靜態連入 libevent 與 ncurses，亦需附上對應授權。私有 Node 22 應保留 Node 及其第三方通知。`tmux-history.cjs` 包含 xterm，請一併保存本專案與 xterm 的授權；APK 本身不內嵌 tmux 或 Node。

## Linux 桌面依賴

Linux 憑證保存使用 secret-service 5.2.0，授權為 MIT OR Apache-2.0。GIO Rust binding gio 0.18.4 為 MIT，負責開啟系統預設瀏覽器。GTK、WebKitGTK 與 Secret Service provider 由作業系統提供，不內嵌個人 keyring 或桌面服務。

下列 MIT 許可文字保留 secret-service 的歸屬，也保留 gio 套件隨附的許可條款。

```text
secret-service 5.2.0
Copyright (c) 2025 secret-service Developers

gio 0.18.4

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
