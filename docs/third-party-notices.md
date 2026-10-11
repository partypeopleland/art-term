# 第三方來源與授權

來源清冊沿用既有核對，本輪只更新repo內複用及散布範圍，未重新查核外部授權。

- [openab-pty](https://github.com/openabdev/openab-pty)：MIT，Copyright (c) 2026 openabdev。Host runtime使用未修改的固定官方產物；來源與image／layer／binary digest記錄於scripts/openab-runtime/official_artifact.py。保留完整歸屬與許可文字於LICENSE。
- [openab-pty NOTICE](https://github.com/openabdev/openab-pty/blob/main/NOTICE)：repository code與聚合container image授權不同，映像內第三方CLI仍依其條款。Client不內嵌這些映像；另行散布Host映像須核對NOTICE。
- [instance-mcp](https://github.com/openabdev/instance-mcp)：MIT，Copyright (c) 2026 openabdev。Native Runtime helper 呼叫既有 MCP 桌面能力，不把該binary內嵌client。
- Vue、xterm.js、Tauri、Tokio、reqwest、rustls等依Cargo.lock及package-lock.json鎖定，各自license仍適用；此清冊不將依賴授權改為專案MIT。

Windows／APK交付保留LICENSE與THIRD-PARTY-NOTICES.md。

Runtime HTTPS／WSS 使用 rustls／ring；uuid 與 zeroize 為 Apache-2.0 OR MIT。原套件許可與歸屬仍適用。App 不再包含 russh／ssh-key 或呼叫系統 OpenSSH。

連線可攜檔使用 RustCrypto 的 argon2、aes-gcm 與 aes，以及 getrandom；原生檔案選擇／讀寫使用 tauri-plugin-dialog 與 tauri-plugin-fs。版本由 Cargo.lock 鎖定，各套件原有許可與歸屬仍適用。
