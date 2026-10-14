# APK UI 回歸測試

在 Linux/KVM 專用 Android 模擬器安裝真 APK，透過 ADB 操作介面。後端使用既有 TLS/WSS fixture，原生 IPC、Keystore 與系統選檔仍實際執行。案例依「介面 × 操作 × 狀態」設計，涵蓋正常、取消、錯誤復原、邊界值與相鄰功能組合。

## 覆蓋範圍

- 首頁分類、Host 新增／編輯／刪除、欄位驗證與重啟保存。
- Shell 建立、衝突提示、清單返回、分頁切換與關閉。
- 多行與中文輸入、CLI 啟動、Ctrl 與快速鍵、虛擬鍵盤狀態。
- 原生歷史拖曳、新輸出保留閱讀位置、斷線恢復與重新連線、切換列表與分頁後捲動。
- 語音設定與文字預覽切換、模式保存、缺少辨識服務時的提示。
- 桌面預覽啟用／停用、週期與手動更新、分頁隔離。
- 旋轉、背景返回、系統選檔、連線匯入／匯出與覆蓋安裝。

可執行案例以 [ui-cases.json](ui-cases.json) 為準，入口為 [ui_matrix.py](ui_matrix.py)，不另維護重複的逐案文件。

## 執行

需要 Linux/KVM、JDK17、Android SDK command-line tools、emulator、build-tools35 與 `system-images;android-34;default;x86_64`。提供啟用 `openab-fixture` feature 的 x86_64 debug APK。測試僅操作自建模擬器，不使用現有手機。

```bash
uv venv target/android-regression/venv
uv pip install --python target/android-regression/venv/bin/python \
  -r scripts/android-regression/ui-requirements.txt
source target/android-tools/env.sh
target/android-regression/venv/bin/python scripts/android-regression/ui_matrix.py \
  --sdk "$ANDROID_HOME" \
  --apk crates/art-term-gui/gen/android/app/build/outputs/apk/x86_64/debug/app-x86_64-debug.apk
```

可加 `--focus history|voice|navigation|keys|regressions` 選擇單一範圍（擇一填入）；子集通過不代表完整測試通過。執行結果與截圖輸出至忽略版控的 `target/android-regression/`，用於當次排錯，不另外建立歷史文件。

## 限制

此模擬器沒有完整語音辨識服務，無法涵蓋實際辨識與語言包下載。Windows 雙向搬移使用既有 Windows 測試入口；ARM64 正式簽章升級、廠牌鍵盤與長時間省電仍需對應實機驗證。

尚未覆蓋全部介面組合，例如原生連結複製、alternate buffer／超大緩衝 fallback、Host 容量上限、異常或過大匯入檔及逾時圖片。fixture 的重連會重播遠端保留資料，不能據此推定 App 被系統殺掉後仍有本機持久歷史。真服務驗證另見 [重連與歷史閱讀](reconnect-scroll.md)。
