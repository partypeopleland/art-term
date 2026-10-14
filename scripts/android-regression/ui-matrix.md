# APK UI 操作矩陣

此入口延伸既有 Android `Suite` 與 Windows/Android 共用的 TLS/WSS `Fixture`。在 Linux/KVM 的專用 Android 34 x86_64 AVD 安裝真 APK，使用 ADB 觸控／拖曳、CDP 鍵盤輸入、Android Keystore、系統選檔器與真 Rust 網路層。只有對端 Runtime 是模擬資料，不執行輸入的 shell 指令。

## 測試方法

以「介面 × 操作 × 狀態」盤點，再用狀態轉移、等價類、邊界值與相鄰功能組合挑選案例。每個 ID 是一個可重跑情境，有些包含一連串必要操作；不把其中每個斷言當成獨立 E2E，也不宣稱窮舉所有組合。機器可讀清單是 [ui-cases.json](ui-cases.json)，執行入口是 [ui_matrix.py](ui_matrix.py)。

| 介面／ID | 前置狀態與操作 | 預期結果／方法 |
|---|---|---|
| HOME-01、02 | 空資料刷新；有 Host 時切全部／Host／Agent | 空狀態提示、空匯出停用；分類即時切換，切回全部恢復。等價類 |
| HOST-01、04 | 新增中取消；刪除確認中取消 | 沒有寫入或刪除既有設定。取消路徑 |
| HOST-02 | 空白名稱、HTTP、非根網址、缺憑證、超過128 UTF-8 bytes | 顯示錯誤，仍無設定。等價類／邊界值 |
| HOST-03 | 新增→測試→編輯分類／名稱→重啟→刪最後一筆→重啟 | 管理認證成功；空憑證編輯保留金鑰；最後一筆不重現。狀態轉移 |
| HOST-05 | 錯誤憑證測試，修正後再測 | 失敗提示；修正可認證。負向→復原 |
| SHELL-01 | 空白、大寫、空格、特殊符號；輸入32字元後再加字元 | 錯誤提示且不建立；欄位上限32。等價類／邊界 |
| SHELL-02、03 | 建立→live→清單刷新→重複建立→返回→主入口 | 使用中／接管提示正確；衝突不替換 Shell；已有 live 分頁可回到原工作 |
| INPUT-01～03 | Composer 取消、不加 Enter、加 Enter，使用繁體字 | 取消不送；送出 bytes 精確；原生可見文字含繁體字 |
| INPUT-04、05 | Ctrl 開／關；啟動已設定 CLI（含空格參數） | 真輸入轉 control byte；CLI 參數界線正確。fixture 不執行 CLI |
| KEY-ESC/TAB/LEFT/UP/DOWN/RIGHT/SHIFTLEFT/CTRLC | live 時逐個觸控快速鍵 | PTY 收到規定的 escape/control bytes |
| KEY-IME | Android 軟鍵盤隱藏及顯示時，按四個方向鍵 | 系統 IME 最終狀態及操作期間的原生可見性事件均不反向切換；兩種起始狀態都須測到，事件觀察器也須收到刻意開／關鍵盤的訊號 |
| TAB-01～03 | 兩個工作切換；關其中一個；關最後一個再選連線 | 輸出不串線；只 detach、不刪遠端；空分頁導回列表 |
| TAB-04 | 切回第一分頁後，核對 active 標籤／遠端名稱並送 Composer 文字 | 畫面名稱一致；只送至選定 Shell，另一工作沒有該段輸入 |
| READ-01 | live 時直向↔橫向 | Android viewport 與 PTY 尺寸同步 |
| READ-02 | 模擬180行 `ls -al` 類輸出；實際手指拖曳 | 由可及性節點或 Android 實際 View hierarchy 確認可見原生 ScrollView／位置；截圖有位移；拖曳不送遠端輸入 |
| READ-03 | 上述歷史存在時，伺服器中斷 WSS | 同 token/cursor 恢復、不重送命令；原歷史仍可實際捲動 |
| READ-04 | 本機 detach→重新連線同一目標 | 新本機 binding、同遠端 Shell；新收到的原生 snapshot 仍含舊文字且可捲 |
| READ-05 | 停在歷史位置時伺服器追加輸出 | 畫面停在閱讀處，不跳到底。截圖比較 |
| READ-06 | 同一份歷史存在時，開關 Shell 清單並建立／切換另一分頁，再回原工作 | 原生歷史仍可實際拖曳。相鄰功能組合；與直向單一工作分開測 |
| OVERLAY-01 | 開 Composer、連線管理 | 原生閱讀層不擋操作；關閉後可回工作 |
| VOICE-01 | gear→hold／tap radio→語言 checkbox→gear | 文字預覽與設定區互換；原生閱讀區保持可見；選項保留 |
| VOICE-02、03 | 無辨識服務；切換設定後重啟 APK | 缺服務有 disabled/提示；設定跨重啟保留。這不是語音辨識成功案例 |
| PREVIEW-01 | 啟用→首圖→兩次30秒更新→手動更新→切分頁→停用 | 真 PNG 解碼；即時首圖與週期；分頁隔離；只清暫時 screenshot Shell |
| PREVIEW-02 | 桌面啟用時隱藏／顯示圖片 | 圖片與手動擷取控制隨可見狀態切換，桌面 enablement 保留 |
| LIFE-01 | 開預覽／Ctrl／未送草稿→Android HOME 超過30秒→返回 | 背景不擷取；清草稿與 Ctrl；保留 binding 且不重送內容 |
| SAF-01 | 真系統匯出、取消匯入、錯密碼、還原、重複匯入更新、重啟 | 加密不洩漏憑證；取消／錯密碼不寫入；duplicate 預設略過；還原後認證成功 |
| SAF-02 | 無選取／密碼不一致；metadata-only 匯出→新裝置等價狀態匯入補憑證 | 禁止無效匯出；JSON 不含憑證；補填前不能套用，補填後可認證 |
| UPDATE-01 | 同 debug APK install -r | 設定／Keystore 身分保留且可認證；不代表歷史 release 遷移 |

`SETUP-TRANSFER` 是獨立匯入匯出群組的準備步驟，不算額外產品功能覆蓋。Terminal 的 CLI／preview 公開 metadata 目前仍沿用既有 Suite 的測試裝置種資料步驟；其餘設定透過 UI 建立。觀察用 IPC wrapper 原樣傳遞真請求與回覆，沒有假回傳值或 JS `click()`。

## 執行

需要 Linux/KVM、JDK17、Android SDK command-line tools/emulator/build-tools35，以及 `system-images;android-34;default;x86_64`。提供 **x86_64 debug、openab-fixture feature** 的 APK；測試 CA 僅安裝進自有 AVD，不關閉 TLS 驗證，不接受正式 APK 注入。

```bash
uv venv target/android-regression/venv
uv pip install --python target/android-regression/venv/bin/python \
  -r scripts/android-regression/ui-requirements.txt
# 若這台機器使用本地 Android 工具環境：
source target/android-tools/env.sh
target/android-regression/venv/bin/python scripts/android-regression/ui_matrix.py \
  --sdk "$ANDROID_HOME" \
  --apk crates/art-term-gui/gen/android/app/build/outputs/apk/x86_64/debug/app-x86_64-debug.apk
```

同一命令可加 `--focus history`、`--focus voice`、`--focus navigation`、`--focus keys` 或 `--focus regressions`，只跑宣告的情境及必要 UI 前置操作。`regressions` 包含導航／輸入目標／歷史及鍵盤狀態。結果註明 subset scope；子集通過不能當成完整矩陣通過。

Linux runner 以程序鎖序列化自有 AVD 的生命週期；並行啟動的命令會排隊，避免挑到同一個連接埠。ADB 不可用時也會回收自己建立的 Popen，保留失敗紀錄。

總時限1100秒。每個案例輸出 PASS／FAIL／BLOCKED／NOT_RUN；未到達的案例仍保留，不消失。失敗不隱藏；後續有獨立前置條件的群組繼續執行。每次結果另存 `target/android-regression/run-ui-*/cases.json`／`result.json`／截圖，附 APK hash、版本、source HEAD 與 runner hash。清理只作用於本次建立的 AVD、forward/reverse 與 fixture，不選現有手機。

案例曾由 agy 審查，採納首頁／取消／邊界／最後分頁／純設定搬移／設定持久化等缺口。針對審查指出的假陽性，文字除傳輸外也查原生可見文字；手動預覽使用不同的有效 PNG，要求新圖實際解碼；重連清除測試觀察後等新 snapshot，不能沿用舊觀察；無辨識服務的 disabled 狀態不算語音成功。

Android14 的 UIAutomator 可能省略剛替換的原生閱讀器。缺少可及性節點時，工具改查實際 Android View hierarchy：必須是這個 plugin 的原生 ScrollView，元件及父層均可見且非零尺寸，父層位置須符合截圖中的終端範圍。接著仍須實際拖曳、畫面位移、舊內容存在且不送遠端輸入。兩個階層都沒有元件，或手勢無效，仍會失敗；不再單憑可及性快照宣稱原生捲動退出。

## 範圍限制與待補案例

此 AVD 沒有 on-device SpeechRecognizer，故錄音 tap/hold、部分結果／最終文字、取消、權限拒絕與語言包下載列 BLOCKED；已操作的 radio/設定持久化單獨計算。Windows↔APK 雙向搬移仍使用既有 Windows 入口，Linux 列 BLOCKED。ARM64 release 簽章升級、廠牌中文組字、長時間 Doze 與實網列 BLOCKED。

本矩陣也尚未涵蓋：原生連結點按複製／長按邊界、alt-screen 與超大 buffer fallback、64筆 Host 容量、服務 draining/dead/延遲回覆、SAF 不合法／過大檔案、兩分鐘預覽過期、桌面端擷取錯誤／晚到圖片，以及每個 UI 尺寸的兩兩組合。這些是明確缺口，不能由既有單元測試或上述通過案例推定已完成 APK E2E。

READ-04 的 fixture 會按 attach cursor 重播遠端保留資料，因此結果只證明該協定下重新連線的 UI；不能推定沒有遠端 replay 的服務、App 被系統殺掉後仍有本機持久歷史。原先實際 NUC runner [reconnect_scroll.py](reconnect_scroll.py) 保留供部署驗證。
