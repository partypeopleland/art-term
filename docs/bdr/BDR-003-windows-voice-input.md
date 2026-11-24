# BDR-003：Windows 語音輸入

- 版本：v1
- 日期：2026-11-24
- 狀態：已決定採用 Windows SAPI 裝置端 dictation；程式已整合，Windows 實機與麥克風驗證 NOT_RUN。
- 定位：記錄 Windows 終端語音輸入的方案取捨；操作流程與 Android 共用同一套前端 voice controller，平台差異只在辨識後端。

## 使用者問題與目標

Windows 桌面版需要與 Android 相同的語音輸入體驗：點按或按住錄音、清楚狀態與取消、顯示轉寫，結束後把單行文字送到錄音開始時仍有效的分頁並送出 Enter。優先情境仍是向 coding agent 描述需求與補充問題；語音不取代硬體鍵盤、方向鍵或精確 shell 編輯。

## 決策與理由

1. **採用 Windows SAPI dictation**：以 `robius-speech` 0.3.1 的 Windows 後端呼叫 SAPI in-process recognizer 與原生麥克風輸入。SAPI 是 unpackaged desktop app 可用的系統辨識服務，不需要 WinRT package identity，也不內嵌模型、不上傳錄音。
2. **只採用裝置端辨識**：SAPI recognizer 為系統已安裝的語音辨識語言；App 不提供雲端或上傳辨識服務。缺少語言或麥克風時回傳可理解錯誤，鍵盤輸入不受影響。
3. **預設 zh-TW**：前端固定以 `zh-TW` 啟動辨識，與 Android 相同。繁體中文辨識品質、中英混說表現取決於使用者安裝的 Windows 語音辨識語言；本 PR 未做實機語料驗證。
4. **沿用 Android 送出保護**：錄音開始鎖定分頁／窗格／連線世代；取消、切換焦點、關閉、斷線、重連或 App 進背景時丟棄晚到結果。文字送出後若目標失效，略過 Enter；已送出的文字不撤回。
5. **Windows 實機驗證另行安排**：CI 只驗證前端單元測試與可測邏輯；SAPI、麥克風權限與 zh-TW 語言包的實際行為標示 NOT_RUN。

## 限制與候選方案

- SAPI dictation 需要使用者已安裝對應 Windows 語音辨識語言；App 不會代為下載語言包，只提示至 Windows 設定安裝。
- 語音分段串接不沿用 Android 的停頓補全形逗號；Windows 以字元邊界補半形空格：CJK 相鄰不補，拉丁字詞、中英邊界與括號依邊界規則補空格，避免把停頓轉成標點。
- 中英混說由單一 `zh-TW` recognizer 處理；是否夠用必須以實機語料驗證，不能由 Android 的 `EXTRA_ENABLE_LANGUAGE_SWITCH` 行為推定。
- 若 SAPI 在目標 Windows 版本不可用或品質不足，候選方案是 WinRT `SpeechRecognizer`（需要 package identity 或改打包方式）、Windows.Media.SpeechRecognition 的自訂 grammar（不適合自由 dictation），或評估自帶模型；本版不加入上傳錄音服務。

## 參考

- [robius-speech](https://github.com/project-robius/robius) `crates/speech`：Windows 後端說明使用 SAPI dictation 與原生音訊輸入。
- [Android 語音輸入 BDR-001](BDR-001-android-voice-input.md)：共用操作流程與送出保護的來源決策。
- [Windows 平台](../platforms/windows.md)：桌面操作與驗收界線。
