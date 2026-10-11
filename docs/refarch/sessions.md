# 分類連線與工作生命週期

App 必須區分「清單中的主機」、「遠端 Shell」與「本機分頁」，才能在設定變更或斷線後繼續操作正確的工作。Host／Agent 只是清單分類，不授予能力，也不是遠端工作的身分。App 使用已設定的 Runtime，不掃描主機或代為部署；設定來源見 [連線設定](connections.md)。

## 選主機之後連到哪個工作

```text
Host profile (id + revision + origin + capabilities)
         |
         +-> reuse local live tab, if present
         |
         +-> list remote Shells
                    |
                    +-> one ready Shell -------------------+
                    +-> other states -> Shell chooser      |
                                        |                  |
                             select / create / takeover    |
                                        |                  |
                                        +------------------+
                                                 |
                                          backend target
                                                 |
                                      binding -> tab + WSS
```

點名稱先重用同 Host 的本機 live runtime 分頁。沒有可重用分頁時，App 使用凍結的 Host id／revision 列一次 Shell：只有一個 ready Shell 就直接連線；零個、多個、已 attached 或未知狀態都顯示選擇介面。右側「Shell」永遠開管理清單。建立及接管都需使用者明確操作；結果不確定時先重新整理，不能自動重試建立而意外多開工作。

target 與 session ID 由 backend 發出，UI 不解析它們來組成遠端命令。binding 固定 Host 設定版本、Runtime 入口、Shell 名稱與官方 generation（工作世代）；綠點僅表示本機有已連線分頁，不是遠端在線探測。官方協定沒有 Runtime process 或 Shell incarnation 身分欄位，因此名稱與 generation 不能證明服務重啟後仍是原工作。短暫斷線沿用原 attach token，由 Runtime 驗證它是否仍能連接原工作；token 失效時停止自動恢復，交由使用者重新選擇。

桌面與 coding CLI 能力在 target／session 發出時一併凍結，來自明確 origin mapping，不從 Host／Agent 名稱或分類推斷。舊 Endpoint 桌面關聯只轉成原 origin 的 Native 路徑，不帶回 SSH 授權。

## 輸入、輸出與 ACK

```text
User input -> bounded input queue -> WSS -> remote Shell

remote Shell -> ordered output -> bounded output queue
                                          |
                                     xterm parse
                                          |
                       exact batch ACK via IPC
                                          |
                              native rendered cursor
                                          |
                           reconnect since -> Runtime

Gap -> clear pending input / resize -> reset emulator -> next output
```

輸入與輸出都有 bytes／事件數界限，單次 poll 最大 64 KiB。UTF-8 解碼保留跨 chunk 的未完整 bytes，背景 Tab 仍持續排空資料。ACK 是 App 對指定一批已解析輸出的本機確認：xterm parse 完成後，前端經 IPC 呼叫 native SessionHandle，更新本機已呈現游標並釋放該批 pending 輸出。它不是逐批發給 Runtime 的遠端 ACK frame；重連才使用該游標作為 since。關頁後晚到的 callback 不得 ACK。

Gap 表示輸出序列有缺段，不能把後續 bytes 當作完整連續畫面。App 先清 pending input／resize、重置 emulator，再處理補送資料，避免畫面與待送操作建立在已失效的狀態上。

## 斷線、背景與離開

短暫 transport 錯誤或 timeout 最多嘗試恢復三次，使用原 attach token 及本機 ACK 游標作為 `since`。token 只留在原生層受保護記憶體，不送進 WebView、日誌或設定檔。恢復不自動續期、建立或重啟 Shell，也不重播未送出或結果不確定的輸入。認證被拒、存活期限（TTL）到期、接管或 token 撤銷都會停止恢復，使用者需回到清單確認狀態。

使用者明確選取既有 Shell 時，App 才透過官方 `renew` 取得新 token；這會增加 generation 並使舊 token 失效。官方續期沒有本專案舊版的原子身分比對，因此不能在斷線恢復時只憑同名 Shell 自動續期。結果不確定的管理請求不自動重送。

Android 切到背景時保留健康終端連線，但停止圖片與恢復排程，清除憑證、composer、未送輸入／resize、輸入法組字及 Ctrl 狀態；回前景後才繼續恢復。系統回收或長時間 Doze 仍可能使連線中斷。主動 Detach 或關頁只讓本機脫離，不刪遠端 Shell，也不停止共享桌面。

平台操作見 [Windows](../platforms/windows.md)與 [Android](../platforms/android.md)，桌面觀察的範圍見 [桌面](desktop.md)。

## 附錄：實作與參考

[目標選擇](../../apps/desktop/src/state/runtime-entry.ts)、[分頁與輸出控制](../../apps/desktop/src/state/controller.ts)、[核心 session](../../crates/art-term-core/src/openab/session.rs)、[native session gates](../../crates/art-term-gui/src/runtime_sessions.rs)。
