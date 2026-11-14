# 客戶端控制模組的責任邊界

本頁說明 TUI、原生 GUI 與 Vue 控制器如何分工。三端仍沿用既有 Runtime 協定、Session binding 與平台能力；這次拆檔只調整原始碼歸屬，不改對外命令或資料格式。

```text
Vue 畫面 → controller → Transport → GUI command 入口
              │                     └→ runtime_sessions
              ├→ preview                 ├→ lifecycle
              ├→ terminal-io             ├→ history
              ├→ errors                  ├→ preview
              └→ types                   └→ instance

TUI run → App → backend / tab / core
            ├→ tree / settings / input
            ├→ events / preview
            └→ render
```

## 模組責任

| 模組 | 擁有的決策 | 不負責 |
|---|---|---|
| `apps/client/src/shared/state/controller.ts` | 分頁、連線、輸入與回復流程；組合畫面可用的操作 | 截圖排程細節、錯誤文案表、型別定義 |
| `controller-preview.ts` | Screen 選擇的世代隔離、桌面開關、單一進行中截圖、圖片有效性 | 建立 Shell、送出終端輸入 |
| `controller-terminal-io.ts` | 一般輸入、終端協定回覆與 resize 的序列化及有界佇列 | 畫面焦點與 Screen 選擇 |
| `controller-errors.ts` | 將後端錯誤轉成有界且安全的畫面文字 | 連線狀態變更 |
| `controller-types.ts` | 分頁、控制器與選項契約 | 執行時操作 |
| `crates/art-term-gui/src/shared/runtime_sessions.rs` | 共用狀態型別、錯誤對應、Tauri command 入口 | 各操作流程的實作 |
| `runtime_sessions/lifecycle.rs` | target/session 建立、資料輸入輸出、重連、關閉 | Screen 或截圖操作 |
| `runtime_sessions/history.rs` | 歷史通道與 helper 的生命週期 | 一般 Shell 輸入 |
| `runtime_sessions/preview.rs` | 預覽路由、綁定核對、截圖授權與競態隔離 | Screen 設定與工具配置 |
| `runtime_sessions/instance.rs` | Screen 選擇、工具配置與清除 | 截圖內容呈現 |
| `crates/art-term-tui/src/ui.rs` | App 狀態、資料型別與頂層事件迴圈 | 各類操作和繪圖細節 |
| `ui/app_*.rs` | 依樹狀連線、設定、輸入、背景結果、預覽與狀態維護分工 | 後端網路協定 |
| `ui/render.rs` | 從 App 讀取狀態、繪圖、游標與 hit region | 啟動網路操作或修改遠端工作 |

## 測試保護

- GUI 的 `runtime_sessions_tests.rs` 維持原本 20 個測試入口；驗證目標仍是 native session 的身份、路由、預覽與清理契約。
- TUI 的 `ui_tests.rs` 與 `ui/tests/` 維持原本 26 個測試入口；覆蓋表單、樹狀選擇、繪圖、預覽與游標。測試專用的 `terminal_size` helper 留在測試模組。
- 前端既有 controller、Screen 選擇、預覽與 Runtime 測試仍從公開 `createAppController` 入口驗證可觀察行為。拆檔後，各模組不新增另一套狀態來源；世代與取消判斷仍共用原分頁狀態。

重構的檢查重點是：測試名稱與案例數不變、原 API 與錯誤文字不變、編譯與既有測試通過。新增需求應針對單一行為寫獨立案例，再修改負責該行為的模組。
