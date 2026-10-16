# 手機畫面預覽

手機畫面預覽讓你在另一個 SSH 工作階段啟動服務，用一般瀏覽器討論及修改 Vue 畫面，不必設定瀏覽器的裝置模擬，也不必建置 APK。預設手機尺寸是 390 × 844 CSS 像素；外框依瀏覽器可用空間自動縮放，內部仍維持真正的手機 viewport，因此窄畫面樣式、`vh` 和對話框高度都以手機尺寸計算。

## 啟動與開啟

第一次先從 repository 根目錄安裝前端依賴；已安裝者可略過：

```bash
npm --prefix apps/desktop ci
```

在 SSH 中切到 repository 根目錄，執行並保持程序運作：

```bash
npm run preview:ui
```

服務預設只監聽主機本機的 `127.0.0.1:1421`。在你的電腦建立 SSH tunnel（將 `user@server` 換成平常登入的主機）：

```bash
ssh -N -L 1421:127.0.0.1:1421 user@server
```

再用電腦瀏覽器開啟 **http://localhost:1421/**。不用開 DevTools 或 Android emulator。若 1421 已被使用，先停止舊的預覽服務，或用 `npm run preview:ui -- --port 1422` 啟動另一個埠，tunnel 與網址也改為相同埠。

需要同區網手機直接開啟時，可改用：

```bash
npm run preview:ui -- --host 0.0.0.0
```

手機開 `http://主機區網IP:1421/`，網路與防火牆需允許此埠。這是開發服務，僅供你信任的網路使用。手機也會看到自動縮放的預覽框；若要讓手機瀏覽器直接用自身 viewport 顯示 App，可開 `http://主機區網IP:1421/?phone`。

## 看畫面與修改草稿

預設「正式手機元件」直接使用 Android 的 `MobileApp.vue`、共用連線表單、分頁與終端樣式。固定頂列含品牌、Main／Connect 滑動條及全站齒輪；點品牌看版本與更新紀錄，點齒輪看外觀及連線設定。Main 依序顯示 Computers、Agents 和尚未開放的 Screens；分類可收合，右側顯示數量。主機旁的既有 Shell 按鈕可模擬 attach，多筆預設收合；`＋` 模擬建立工作。新增連線表單及匯入／匯出入口集中在設定頁，預覽新增的設定只更新假資料。

情境選單可切換示範連線、空清單和讀取失敗；「重設」還原初始資料。切換畫面或情境也會重設，重新整理不保留新增的連線。四種全站 Theme 及語音操作偏好則保存在瀏覽器本機。

調整既有畫面可直接編輯 `apps/desktop/src/mobile/MobileApp.vue` 或 `apps/desktop/src/components/` 裡的對應元件；儲存後 Vite 會即時更新瀏覽器，正式 APK 也會沿用這份修改。部分 script 修改會使元件重新掛載，回到初始狀態。

想自由試新設計時，把畫面選成「我的草稿」，編輯 `apps/desktop/ui-preview/Draft.vue`。這份獨立 Vue 元件可自行改版面、放假資料或加簡單按鈕互動，儲存後立即看到。確認草稿後，再請 AI 整合到正式元件並接上功能；草稿本身不會進入 APK。討論時可提供畫面／情境名稱與想調整的位置，截圖也可以。

## 假資料與實機界線

所有 Host、Shell、測試連線與輸入回應都來自 iframe 內的記憶體，不讀取本機連線、不對 Runtime 發出請求、不保存管理憑證。終端只回顯輸入及固定示範訊息，不執行指令；匯入／匯出模擬取消，網址複製不寫入電腦剪貼簿。Connect 底部以假語音狀態呈現語音按鈕、文字預覽、錄音／預覽設定與 `openab-pty:…` 連線資訊；即使還沒有 attach，也能查看這部分排版。假語音不開麥克風，也不呼叫 Android 辨識服務。Android 原生歷史閱讀器及遠端桌面不在此預覽提供。

預覽適合確認排版、配色、表單與一般導覽。Android 系統鍵盤、語音、原生歷史手勢、系統選檔、安全區域、背景連線及真實 Runtime 行為仍需 APK 驗收。流程是瀏覽器調整畫面、確認設計，再建 APK 做實機測試。

## 與正式建置的關係

```text
npm run preview:ui -> separate Vite root -> browser shell
                                            |
                                            v
                                     390 x 844 iframe
                                      /           \
                         real MobileApp          Draft.vue
                               |
                        in-memory transport

normal APK build -> src/main.ts -> real native transport
```

預覽使用獨立 Vite 設定與入口，不修改正式 `src/main.ts` 的原生組裝。預覽設定替換 Tauri 與剪貼簿介接；手機元件以可選參數關閉原生閱讀器，改用瀏覽器 xterm，APK 預設仍使用 Android 原生閱讀器。正式 `npm --prefix apps/desktop run build` 不會打包預覽入口或草稿。

## 附錄：實作與參考

[預覽設定](../../apps/desktop/vite.preview.config.ts)、[預覽外框](../../apps/desktop/ui-preview/Preview.vue)、[假資料 transport](../../apps/desktop/ui-preview/transport.ts)、[草稿](../../apps/desktop/ui-preview/Draft.vue)、[Android 實機驗收](android-regression.md)。
