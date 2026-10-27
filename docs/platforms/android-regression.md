# Android APK 原生 E2E：AI 執行指南

這份文件說明如何在 Windows 的專用 Android 模擬器操作真正的 APK，驗證畫面、Tauri IPC、Rust HTTPS／WSS、Android Keystore 與系統文件選擇器的整合。使用者準備 [Android 建置工具](android.md)及可用的 Android 模擬器即可；工具檢查、案例選擇與結果由 AI 處理。功能與跨平台保護範圍集中在[測試矩陣](../refarch/test-matrix.md)。

Linux/KVM 的日常 APK UI 回歸使用[介面操作矩陣](../../scripts/android-regression/ui-matrix.md)，延伸同一套 Suite／協定 fixture，涵蓋語音設定 swap、原生歷史捲動、重連、快速鍵與 Android 軟鍵盤狀態；不依賴 Windows EXE。本文以下保留既有 Windows＋APK 跨裝置流程。

0.1.27 的固定頂列、設定頁與行內連線使用新版入口 `scripts/android-regression/ui_refresh.py`，以專用 Linux 模擬器執行真 APK。它另涵蓋未連線頁右滑、儲存燈箱偏好後重新進入 Connect，以及正式 Shell-exited 訊號後清除終端並往返滑動；也涵蓋首頁更新文件、設定與 Keystore 保存、明確建立工作、原生／WebView 滑動、快速鍵、原生主題與背景／系統選檔返回；實際語音辨識及 ARM64 手機觸控仍須實機確認。舊操作矩陣部分入口選擇器尚未配合新介面調整，不能直接把舊矩陣視為本版完整驗收。

```bash
source target/android-tools/env.sh
target/android-regression/venv/bin/python scripts/android-regression/ui_refresh.py \
  --sdk "$ANDROID_HOME" \
  --apk crates/art-term-gui/gen/android/app/build/outputs/apk/x86_64/debug/app-x86_64-debug.apk
```

本流程已在 Android 15／API 35 模擬器實際操作 APK，涵蓋六組 Android 案例及一組雙向跨平台案例。測試 APK 是 x86_64 debug＋`openab-fixture`，不是更新私人手機的 ARM64 release APK。測試使用真正的 Android 系統與產品程式，但模擬器不具有實體手機的 ARM64、硬體 Keystore、安全硬體與各廠牌省電行為。每次執行是否通過，仍以該次完整結果與清理狀態判定。

直接 MCP／Screens 版本須另覆蓋原生 Keystore 的桌面 CRUD／重開、每分頁選擇／切換、背景／晚到圖片、工具狀態及 Host 匯入保留桌面。舊截圖 helper 案例或 APK 建置成功不代表這些項目已通過；完整門檻見[測試矩陣](../refarch/test-matrix.md#直接-mcp-與獨立-screens-的驗收門檻)。固定 client 的工具 E2E 不代表真 AI 模型已測。

## 依賴與資料流

需要 Windows、Python 3.12 以上、Git、Node/npm、Rust Android target、JDK 17、Android SDK／NDK，以及 `system-images;android-35;google_apis;x86_64`。沿用專案既有 `target/android-tools/environment.json`；模擬器須通過實際 `emulator -accel-check`，不能單憑系統資訊欄位判斷虛擬化不可用。另需至少 8 GiB 可用磁碟供新測試裝置，建置快取也需要額外空間。

```text
Windows test runner
   | ADB: specific owned emulator serial
   +--> Android emulator --> real APK / WebView
   |                            | real Tauri IPC
   |                            +--> Keystore / encrypted TOML
   |                            +--> Android system document picker
   |                            +--> Rust HTTPS / WSS
   |                                       |
   +<------------ adb reverse <------------+
   |
   +--> loopback TLS protocol fixture / real PNG
   |
   +--> Windows EXE / DPAPI / native file picker
            encrypted file exchange with APK
```

按鈕透過 Android 系統的實際觸控操作：UIAutomator 取得 WebView 的原生範圍，再以網頁目標位置換算螢幕座標，每次只送出一次 ADB 點擊，並確認目標收到可信任的 click。CDP 負責文字輸入、畫面定位與真實回覆觀察，不偽造 IPC。Android Home／回前景、安裝與系統選檔也由 ADB／UIAutomator 操作。中文文字注入可以驗證 Unicode 傳送，不能據此宣稱所有中文輸入法的組字行為已通過。

專用模擬器固定啟用硬體鍵盤，並關閉「使用硬體鍵盤時仍顯示軟鍵盤」，讓自動觸控採用固定的輸入環境；入口會讀回核對設定，並寫入裝置紀錄。按鈕仍透過真觸控操作，文字仍經真輸入事件與產品程式處理。這組案例不涵蓋軟鍵盤遮擋、開關或組字，這些仍須另以相應手機／輸入法驗收。

協定 fixture 不執行遠端 shell 命令；它記錄建立、輸入、續期、恢復及刪除操作，提供可解碼的大 PNG。真官方 Runtime／Linux helper 的驗收由 [Linux 測試](linux-regression.md)補足。APK 的測試 CA 只放在自己私有資料目錄，僅 debug fixture feature 使用；不裝系統憑證、不略過 TLS 檢查，release 仍禁止此 feature。

## 執行流程

以下從專案根目錄執行。先依 [Windows 回歸測試](windows-regression.md)完成 Windows 真 App 檢查，保留相同來源的 debug fixture EXE 及版本相符 EdgeDriver，供跨裝置搬移案例使用。

```powershell
.\scripts\test-android-regression.ps1 -Prepare
```

入口建置前端及完整 APK、檢查 package／ABI／debug 狀態，建立全新的專用 AVD，再執行原生案例。AVD 使用獨立資料目錄、無 snapshot、背景執行；每條 ADB 命令指定 serial 並核對對應 AVD。測試結束只關閉自己建立的裝置，不關共用 ADB server、不操作使用者手機或既有 AVD。

若已備妥 Python 環境、Windows EXE 或 EdgeDriver，可明示 `-PythonPath`、`-WindowsExe`、`-EdgeDriverPath`。預設 Python 位於 `target/windows-regression/venv/Scripts/python.exe`，Windows EXE 為 `target/debug/art-term-gui.exe`，EdgeDriver 為 `target/gui-tools/edge/msedgedriver.exe`。

只建測試 APK 可用 `-BuildOnly`。要在修測試腳本後沿用 APK，可用 `-SkipBuild`；入口會核對 APK hash 與建置來源清單，來源變更時拒絕沿用。建置成功不等於 E2E 通過。Windows 無符號連結權限時，入口沿用既有建置方法，以已成功編譯的原生庫完成 Gradle 組裝，不更改全域開發者模式。

## 案例與結果

案例包含連線新增／測試／編輯／刪除與重開保存、Unicode 終端／觸控按鍵／分頁／恢復、大圖預覽／自動與手動更新、真背景切換、系統選檔器加密搬移，以及同一 debug APK 重裝後保留資料。跨裝置案例分別驗證 Windows 匯出→APK 匯入與 APK 匯出→Windows 匯入。每項具體條件見[測試矩陣](../refarch/test-matrix.md)。

統一入口輸出在 `target/android-regression/checks-*/result.json`，原生案例結果在 `run-*/result.json`，裝置歸屬與清理在 `environment-*/owner.json`。保留受測 APK／來源 hash、系統與 WebView 版本、案例結果及清理狀態；任何必要步驟失敗都以非零結束。`BuildOnly` 只表示建置通過，不能當成原生測試通過。

失敗時先讀該步驟的 log／result。WebView 尚未就緒、選檔器找不到節點、協定斷言失敗須分別診斷，不能直接放寬檢查或把假的回覆改成成功。所有資料屬測試裝置；重開與重裝保存案例期間不能清除 App 資料來掩蓋問題。含測試密碼的 UI dump／截圖也留在 ignored target，不提交原始裝置資料或圖片。

## 尚需實機確認的邊界

這套測試不替代 ARM64 release 原簽章更新、硬體 Keystore、各廠牌鍵盤／中文組字、長時間 Doze／OEM 回收、手機 Tailnet 網路或真觸控手感。`install -r` 案例只證明同一 debug APK 保留測試資料，不代表所有歷史版本遷移均可用。修改上述平台行為時，仍依 [Android](android.md)完成對應實機驗收。

## 附錄：實作與參考

[測試入口](../../scripts/test-android-regression.ps1)、[專用 AVD 管理](../../scripts/android-regression/environment.py)、[APK runner](../../scripts/android-regression/run.py)、[Android 憑證與 fixture CA](../../crates/art-term-gui/src/platform/android/android_operator.rs)。

[Android Emulator 命令列](https://developer.android.com/studio/run/emulator-commandline)、[硬體加速](https://developer.android.com/studio/run/emulator-acceleration)、[ADB](https://developer.android.com/tools/adb)、[WebView 遠端除錯](https://developer.chrome.com/docs/devtools/remote-debugging/webviews)。

## Linux 模擬器：NUC 斷線與歷史捲動

針對原生閱讀區新增獨立測試入口，涵蓋真正拖曳、網路中斷後恢復，以及手動脫離再接回同一遠端 Shell。每次建立新的測試 AVD 與專用 Shell，不接管現有裝置或工作。使用 `ls`／`ls -al` 列出測試專用的虛構檔案，透過當次原生內容與畫面變化判斷舊輸出與捲動是否保留。

執行前需準備可除錯的 x86_64 APK、可用的 NUC HTTPS 網址與 repo 外的管理憑證檔。工具啟動測試與真實歷史測試分開回報，沒有連線證據時不得宣稱歷史通過。完整命令、證據與清理方式見[NUC 歷史捲動測試](../../scripts/android-regression/reconnect-scroll.md)。

## 終端網址

`web_links.py` 在全新自建 AVD 安裝 debug x86_64 APK，透過真正的原生 IPC 與閱讀 View 點擊驗證 HTTP(S) 的系統開啟、錯誤網址拒絕及滑動／長按不開啟網址。測試只操作自己的模擬器，不連既有手機。瀏覽器 Intent 的路徑可能被 Android 診斷輸出隱藏，畫面測試核對 scheme／host；原始 URL 傳遞另由連結與平台邊界測試保護。

```bash
source target/android-tools/env.sh
target/android-regression/venv/bin/python scripts/android-regression/web_links.py \
  --sdk "$ANDROID_HOME" \
  --apk crates/art-term-gui/gen/android/app/build/outputs/apk/x86_64/debug/app-x86_64-debug.apk
```

## 真正 Codex 的批准與工作生命週期

[Codex 權限矩陣](../refarch/codex-permissions-matrix.md)的 Android 入口為 `codex_permissions.py`。它與 Linux TUI 共用同一組 14 個行為案例，使用真正 APK 的連線、Screen 選擇、鍵盤及 App 重開操作，並在自有官方 Runtime 上執行真正 Codex。工具回覆、上游請求、程序 PID／start、會話與有效政策分開記錄；原版與修正版 helper 可用同一 APK 比較。這些案例不宣稱實體 ARM64、歷史閱讀或所有 UI 矩陣都已通過。
