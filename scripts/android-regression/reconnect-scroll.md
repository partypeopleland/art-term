# NUC 斷線後的歷史捲動測試（Linux）

這個測試使用真正的 Android APK、隔離模擬器與 NUC openab-pty Shell。它不修改 App 回傳值、不預填歷史紀錄，也不接管既有 Shell。

測試會建立隨機命名的 `scroll-test-…` Shell，在該 Shell 的 `/tmp/art-term-…` 專用目錄建立 160 個虛構檔案，執行 `ls` 與 `ls -al`。使用唯一檔名確認舊輸出仍出現在目前原生閱讀區的內容中，再以 ADB 真實拖曳及終端區域前後截圖差異確認畫面確實移動。觀察器只讀取真正 IPC 的完成結果，所有原生請求與回傳維持原樣。

## 檢查項目

1. 斷線前：原生 ScrollView 可捲動，舊輸出存在，拖曳不發送遠端輸入。
2. 網路中斷：只關閉專用模擬器的 Wi-Fi／行動網路，確認 App 確實離開已連線狀態，再恢復網路。檢查原連線身分與舊內容、捲動是否保留。若無法造成真斷線，測試會失敗，不能視為通過。
3. 手動脫離：透過 App「脫離」再「重新連線同一目標」，確認仍是同一遠端 Shell，但建立新的本機連線。清空測試觀察快取後，才檢查重新呈現的舊內容與捲動，避免把斷線前的畫面誤判成恢復成功。

這裡的「歷史」是先前同一遠端 Shell 的輸出；不代表刪除 Shell 後建立的新 Shell，或 App 資料清除後仍能恢復。這兩種情境不在本次測試範圍。

## 工具與執行

需要 Linux/KVM、Android SDK emulator／platform-tools／Build Tools 35.0.0，以及已安裝的 `system-images;android-34;default;x86_64`。使用新建的 AVD，絕不選取實體手機或既有模擬器。不需要更動 NUC 服務。

```bash
cd /home/art/projects/art-term
source target/android-tools/env.sh
uv venv target/android-regression/venv
uv pip install --python target/android-regression/venv/bin/python -r scripts/android-regression/reconnect-requirements.txt

# 只驗證模擬器可以啟動、識別並停止；不代表歷史測試通過。
target/android-regression/venv/bin/python scripts/android-regression/reconnect_scroll.py --emulator-smoke
```

另外可用 `--apk-smoke --apk <debug.apk>` 驗證APK安裝、啟動及WebView真原生bridge；不會連線到NUC，也不代表歷史測試通過。

真實測試需要可除錯的 x86_64 APK，以使用 WebView CDP 自動操作；不對正式 release APK 強行開啟除錯。

```bash
rustup target add x86_64-linux-android
python3 target/android-tools/prepare-linux.py
npm --prefix apps/client run build
cd crates/art-term-gui
node ../../apps/client/node_modules/@tauri-apps/cli/tauri.js android build --target x86_64 --debug --apk --split-per-abi --ci
cd ../..

target/android-regression/venv/bin/python scripts/android-regression/reconnect_scroll.py \
  --apk crates/art-term-gui/gen/android/app/build/outputs/apk/x86_64/debug/app-x86_64-debug.apk \
  --origin https://YOUR-NUC-RUNTIME.example.invalid \
  --credential-file /absolute/private/path/nuc-admin-token.txt
```

網址必須是可從模擬器連線且受 App 信任的 HTTPS 根網址。管理憑證檔必須只含 64 位十六進位 token，權限 0600；不要將 token 放在命令列、文件或 Git。測試不關閉 TLS 驗證，也不修改既有 App 的連線設定。

## 證據、失敗與清理

每次結果放在專案 ignored `target/android-regression/reconnect-…/`，包含 `result.json`、AVD 擁有權紀錄、建置以外的模擬器日誌與原生終端裁切截圖。結果只存測試判定、身分與虛構內容畫面，不輸出管理憑證。截圖差異是拖曳造成可見移動的證據，並非 OCR 證明某個字元已位於可見畫面；舊輸出存在另由當次原生 snapshot 確認。

測試結束會嘗試清除自己的虛構目錄、刪除自己的隨機遠端 Shell，並停止自己啟動的模擬器。原本的 Shell、服務、實體裝置與資料不會被清理。任何清理失敗都會記錄並讓總結果失敗，不能隱藏成成功。

`passed: true` 需要實際執行所有指定情境。工具可啟動、程式可編譯或看到連線成功，都不代表歷史保留通過。如果舊輸出不見，結果會如實記錄；測試不修改產品邏輯來讓檢查假通過。
