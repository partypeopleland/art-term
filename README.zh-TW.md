# art-term

[English](README.md)

art-term 是給開發者在 Windows 電腦或 Android 手機上連線操作 Linux Shell 與 coding CLI 的 App。Shell 和工具都在遠端 Linux Host 執行；App 負責管理連線並呈現終端。

## 功能

- 多個 Shell 分頁與有限度恢復：短暫斷線後可恢復原工作階段；不重送結果不確定的輸入。
- 選配的共享桌面預覽：顯示 Host 共享桌面的截圖，不是可操作的遠端桌面。
- Windows 與 Android 共用連線核心，並可用密碼加密的連線檔案匯出／匯入，將設定搬到其他裝置。
- 實驗中的 Android 語音輸入：使用裝置端辨識；裝置有可用語言資料時可離線辨識。停頓不會結束錄音，完成後會把文字送到開始錄音的分頁。目前只在 Pixel 9 測試；英文技術名詞、指令與路徑可能辨識錯誤。

## 運作方式

```text
Windows／Android App
   | HTTPS：管理 Shell；WSS：傳送終端與截圖資料
   v
官方 openab-pty 遠端 Runtime
   +--> 工作 Shell --> 命令／coding CLI
   +--> 暫時截圖 Shell --> Host helper --> 選配的 instance-mcp
                                                   |
                                              共享桌面
```

art-term 與 openab-pty 是獨立專案。App 與官方 openab-pty Runtime 配合，由 Runtime 管理遠端 Shell 並傳送終端資料。使用選配桌面預覽時，Host 還需要 helper 與 instance-mcp。App 不會安裝這些遠端服務或 coding CLI。

## 開始使用

準備一台 Linux Host，讓 Windows 電腦或 Android 手機可透過同一個私人 Tailnet（例如 Tailscale）連線，並先設定好官方 openab-pty Runtime。再於 App 填入 Runtime 網址與管理憑證。coding CLI 是選配工具，使用前須先安裝在 Host。要使用桌面預覽，還需準備共享桌面、Host helper 與 instance-mcp。

## 文件與建置

自動建置 Windows EXE、Android APK 並發布 GitHub Release，請見[自動發布設定](docs/platforms/releases.md)。推送版本標籤即可觸發；Android 使用保存在 GitHub Actions Secrets 的固定簽章金鑰，fork 維護者可建立自己的金鑰。

目前詳細文件以繁體中文撰寫。可從[使用前準備](docs/openab-host-setup.md)或[文件入口](docs/INDEX.md)開始，並參考[架構](docs/refarch/architecture.md)、[Shell 工作階段與恢復](docs/refarch/sessions.md)、[桌面預覽](docs/refarch/desktop.md)、[連線搬移](docs/refarch/connection-transfer.md)、[Windows 建置指南](docs/platforms/windows.md)、[Android 建置指南](docs/platforms/android.md)及[Android 語音輸入決策](docs/bdr/BDR-001-android-voice-input.md)。

## Issues 與授權

Issues 開放，歡迎回報問題。請勿貼上憑證、主機位址或其他私人資訊。本專案採 MIT 授權；第三方元件仍適用各自的授權條款。詳見 [LICENSE](LICENSE) 與[第三方說明](docs/third-party-notices.md)。
