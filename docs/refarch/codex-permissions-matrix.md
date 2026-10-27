# Codex 桌面工具權限與重連案例

每次先完成一個案例，再進下一個案例。Linux TUI、Android APK、Windows EXE 使用相同的行為案例，平台驅動與測試產物分開記錄。固定 MCP client 只驗證前置工具通道，以下權限結果由真正 Codex 取得。結果與來源、CLI 版本、批准政策、sandbox、程序身分及會話 ID 一起記錄，不能把不同種類的重連混為一談。

案例 ID 使用 L、A、W 加下表編號。結果為成功、可提出核准、正確拒絕、意外失敗／卡住，或明確 NOT_RUN。矩陣是待驗證契約，測試存在不代表已通過。

| 編號 | 啟動或操作 | 要確認的行為 |
|---|---|---|
| C01 | 先選 A，再以 --yolo 啟動真正 Codex | scoped adapter 能探索並呼叫安全測試工具，沒有 policy 矛盾或等待未能顯示的核准。 |
| C02 | 一般 codex，read-only＋never | 安全探索與有副作用工具分開判定，禁止核准時明確拒絕，不假報成功或重試卡住。 |
| C03 | 一般 codex，workspace-write＋never | 核對實際工具政策，不因工作區寫入權限推論已有桌面操作權限。 |
| C04 | 一般 codex，danger-full-access＋never | 核對 sandbox 與 MCP 批准政策的獨立性，明確分類成功或正確拒絕。 |
| C05 | 一般 codex，read-only＋on-request | 需要核准時真正顯示可處理的請求，只在測試端核准指定安全工具；拒絕時無上游副作用。 |
| C06 | 一般 codex，workspace-write＋on-request | 與 C05 對照，不自動修改使用者政策。 |
| C07 | 一般 codex，danger-full-access＋on-request | 核對可提出核准及核准後交付一次，不把完整存取當作所有 MCP 工具永久授權。 |
| C08 | Codex 已啟動且尚未選桌面，再選 A | 同一程序經既有 scoped adapter 取得 A，不重啟模型或注入新命令。 |
| C09 | 同一 Codex 由 A 切到 B | 新探索與呼叫屬於 B，舊 expected_target 不送到 B。 |
| C10 | 同一 Codex 清除或目標失效 | 明確拒絕，沒有誤用舊目標、上游呼叫或假 ready。 |
| C11 | 同一 Codex 的工作 WSS 暫斷再重連 | Codex PID／start 與會話保持，原 token／游標恢復，不重播輸入或默默變更批准政策。 |
| C12 | 關閉再開 App，接回仍執行的 Shell／Codex | 區分 App 重開與 Codex 重啟，原程序、工具選擇與批准政策保持。 |
| C13 | 結束 Codex 後以 resume 指定測試會話 | 新 PID、同對話，實際有效政策與 adapter wiring 可辨識，不依賴舊程序的記憶體授權。 |
| C14 | resume 時明確改為 --yolo | 尊重本次明確參數，仍核對 MCP 工具設定；不替使用者修改全域設定。 |

若某種政策本來就禁止操作，驗收重點是可辨識且不產生副作用的拒絕。修正應區分 App 的桌面指派、MCP 工具 metadata、Codex 的批准政策與 OS sandbox，不用預設全面放寬權限解決。

基準為 [共用功能矩陣](test-matrix.md)的 CLI 契約與真 Runtime／工具通道。官方參考為 [MCP 設定與工具批准模式](https://learn.chatgpt.com/docs/extend/mcp?surface=cli)及 [Codex 設定參考](https://learn.chatgpt.com/docs/config-file/config-reference)。CLI 的全域批准政策、sandbox 與每個 MCP 工具的批准模式分開記錄。

## 可執行檢查

共用批准標示檢查使用真正 stdio 協定，不呼叫模型：

```bash
python3 scripts/instance-mcp-regression/permissions.py
```

Linux TUI 使用 `scripts/tui-regression/codex_permissions.py`，每次以 `--case C01` 至 `--case C14` 執行一案。`--help` 列出 binary、官方 Runtime、tmux、Node、已編譯 history bundle、真正 Codex binary 與測試認證檔的必要參數。測試建立獨立 Runtime、工作 Shell、Codex 設定與安全桌面服務，透過真正 TUI 操作；不載入使用者的 plugins、hooks 或專案指引。認證只複製到自有的私有暫存目錄，測試結束移除。

Android APK 使用 `scripts/android-regression/codex_permissions.py`，相同的 `--binary` 參數在此指定 x86_64 debug fixture APK，並從 `ANDROID_HOME` 取得已安裝 SDK。Python 環境需同時安裝 TUI 與 Android regression 的 requirements。每次使用自建 AVD、真正 APK、原生 IPC 與官方 Runtime，透過真實觸控、終端鍵盤與 CDP 文字輸入操作；不替換 native 回覆。工具案例讀取實際終端內容，不把 native history 快照缺席當成無輸出，也不以這套案例宣稱歷史閱讀區已完整驗收。

Windows EXE 使用 `scripts/windows-regression/codex_permissions.py`。協調程式在 Linux 測試 Host 執行，Python 環境需安裝 TUI 與 Windows regression 的 requirements；`--help` 列出 Windows SSH、EXE、EdgeDriver、系統暫存根目錄與 Host 工具參數。Windows 的互動式桌面以自有排程工作啟動 EdgeDriver，以 CDP 文字輸入與鍵盤送出操作真正 xterm，保留 EXE／WebView2、Tauri IPC 與 DPAPI。自有 SSH loopback 通道連到測試 Host 的官方 Runtime 與安全桌面服務，真正 Codex 的認證留在 Host，不搬到 Windows。這是原生 App 測試，與一般 Windows runner 的 echo 協定 fixture 分開記錄。

`--bridge-source` 可明確指定原版 helper，與同一 APK 或 EXE 搭配重現修正前的錯誤。結果分別記錄 binary、helper 與測試驅動 hash，不能把原版 helper 的結果冒稱修正版通過。App 重開案例保留該案例資料；互相獨立的案例才使用新的測試資料。

結果保留工具實際回覆與上游請求、有效政策、Codex 版本、來源與 helper hash，以及 PID、程序 start time 與會話 ID。`SUCCESS`、`CORRECT_REFUSAL` 與失敗分開記錄。因錯誤假設或驅動無法操作而失敗的測試不算產品錯誤已重現。

C05 同一程序先批准一次，再拒絕下一次，核對拒絕後沒有上游呼叫。C11 在真正 Codex 的安全工具呼叫途中中斷自有工作 WSS，核對相同 token、非零恢復游標與 Runtime generation，再讓原工具完成並核對 App 顯示實際結果。既有的輸入傳送不確定保護仍保留；本案不要求靜默解除保護，也不透過另一條路徑注入新輸入。C12 關閉再開 App 並重新接回原 Shell，核對原 Codex PID、start 與桌面選擇 generation。C13 明確使用 read-only＋never resume 原會話，C14 明確以 --yolo resume，兩者都要求新程序與相同對話。

Codex 0.162.1 的預設工具政策下，C02、C03 的桌面派送會被 never 明確拒絕；C04 的完整存取＋never 會成功。C05、C06、C07 會顯示批准。這些結果不代表把所有 MCP 工具改為永久放行，也不能代替其他 Codex 版本的實際觀測。

## 平台驗證狀態

案例定義不代表平台已通過。各次執行保留上述來源與結果資料，平台完成紀錄由當次交付提供；尚未執行的平台保持 NOT_RUN。共用 bridge 變更不能代替 Android APK 或 Windows EXE 的原生測試。
