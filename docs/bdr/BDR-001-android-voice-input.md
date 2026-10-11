# BDR-001：Android 語音輸入

- 版本：v5
- 日期：2026-10-11（v4：2026-10-11；v3：2026-10-11；v2：2026-10-11；v1：2026-10-07）
- 狀態：P2 已決定採用 Android 內建裝置端辨識；P3 已整合至 main，Owner 正在 Pixel 9（Android 17）使用 r3 APK，並回饋語音體驗良好。待觀察：Codex CLI 偶爾未自動送出、`⇧←` 在實際 Codex CLI 提問時的操作回饋，以及 API 34 中英語言切換尚未單獨驗證。Breeze ASR 25 保留為條件式備案。
- 定位：本專案的 BDR（Business Decision Record），記錄使用者問題、產品方向與取捨；後續推論引擎、原生介面與生命週期設計另以 ADR 記錄。
- 授權：v1 依使用者要求記錄語音輸入未來計畫。v2 依使用者 2026-10-11 決定，改為優先採用手機內建語音辨識，並將其列為 POC 第一階段。v3 記錄 Owner 於 Pixel 9 完成 P1a 的實測與觀察。v4 記錄 Owner 於 2026-10-11 決定採用裝置端辨識，並在錄音結束時直接送出文字與 Enter。v5 記錄 P3 已整合至 main、Owner 在 Pixel 9 使用 r3 APK 的回饋及待觀察事項。

## 使用者問題與目標

使用者在 Android APK 操作遠端終端與 coding agents 時，覺得手機輸入法難用，尤其是長篇中文需求、技術名詞及中英混用。希望透過語音轉文字降低打字負擔，並能看到實際送出的轉寫、控制送到哪個連線。

優先使用情境是向 agent 描述需求、補充問題與貼入文字。語音不取代方向鍵、Ctrl 組合鍵、密碼輸入或精確 shell 指令編輯；保留既有鍵盤輸入。

## 決策與理由

1. **採用 Android 系統內建語音辨識**：Android 的 `SpeechRecognizer` 讓 App 呼叫手機上已有的語音辨識服務（Pixel 上由 Google 提供）。App 不必自帶模型，APK 幾乎不會變大，也沒有另外下載與管理模型的負擔。
2. **只採用裝置端辨識**：Android 12（API 31）起可用 `createOnDeviceSpeechRecognizer` 建立只在手機上辨識的 recognizer；一般的 `createSpeechRecognizer` 可能把音訊送到服務供應商的伺服器，不符合「不默默上傳音訊」的原則，因此不作預設路線。裝置端語言包由系統下載與更新；App 只查詢可用性並在需要時請系統下載。
3. **Breeze ASR 25 改為備案**：若裝置端辨識在 Pixel 9 不支援繁體中文、品質不足，或無法保證離線，才進入自帶模型路線，以 MediaTek Research Breeze ASR 25 為優先候選。
4. **錄音開始與結束由使用者在 UI 決定**：設定可選「按住說話、放開結束」或「按一下開始、再按一下結束」。手機上的麥克風按鈕要大且好按；錄音中須以明確的色彩與介面變化呈現狀態。說話停頓不結束錄音，停頓處補全形逗號。
5. **錄音結束後直接送出**：辨識文字直接送到開始錄音時的終端分頁，並送出 Enter；App 顯示已送出的轉寫。本版不提供送出前編輯，編輯留待下一版。Owner 已知辨識錯字在 shell 中可能執行錯誤指令，並接受此取捨。
6. **語言設定**：預設 zh-TW。若裝置端辨識服務支援 API 34 語言切換，且 zh-TW、en-US 語言模型可用，啟用這兩種語言；API 34 中英切換尚未單獨實機驗證。
7. P2 已選定內建裝置端辨識；P3 已整合至 main，Owner 正在 Pixel 9 實際使用，待觀察事項列於文件狀態與結論。

服務直接收音時通常會在偵測到靜音後自行結束。Android 13（API 33）起可用 `EXTRA_SEGMENTED_SESSION` 要求分段工作階段；它的值是對應音訊來源 extra 的字串，不能用 Boolean 代替。搭配 `EXTRA_AUDIO_SOURCE` 時，辨識服務在停頓時回傳分段結果，音訊來源關閉才結束工作階段。Pixel 9 的實測結果與操作方式見下節。

內建辨識的優勢是零模型下載、由系統維護更新，且 Google 在 Pixel 上的語音輸入經過大量使用。代價是辨識品質、支援語言與離線能力由系統決定，App 無法調整模型或加入專案術語，Android 版本與 Google 服務更新也可能改變行為。P1a 的測試結果與限制列於下節；不能以「Gboard 平常能用」推定終端情境的準確度。

## P1a 實測結果（Pixel 9）

P1a 已於 2026-10-11 在 Pixel 9（Android 17）完成。以下分開記錄 Owner 真人語音觀察與 TTS 合成語料結果；TTS 指標只驗證音訊輸入流程與分類表現，不代表真人語音準確度。

### 裝置端語言與模型下載

Pixel 9 的 Android System Intelligence 語音辨識服務已安裝 `zh-TW`、`cmn-Hant-TW`、`zh-Hant-TW` 與 `en-US`。Android 14（API 34）以上可用 `triggerModelDownload(Intent, Executor, ModelDownloadListener)` 請系統下載語言模型並接收進度；在 Android 17 上使用舊的單參數 overload 曾造成服務端錯誤與斷線。每次查詢語言支援都建立新的 recognizer，避免重用已失效的服務連線。

### Owner 真人語音與 UI 控制

Owner 在飛航模式下實測麥克風辨識仍可用，主觀感受與連網時相近。一般中文口語可辨識；英文專名常出錯，例如 Devin 辨成「第一本」、Claude 辨成「cl 扣」、Codex 辨成「code 跟 kilome」。這是少量真人語音的 Owner 觀察，尚非代表性準確度測試。Owner 對整體操作表示「大致上非常滿意」，「按著說話」與「點按切換」兩種控制方式都可用。

App 以 `AudioRecord` 自行錄音，透過 pipe 將 PCM 音訊交給裝置端 recognizer，並在 Intent 同時設定 `EXTRA_AUDIO_SOURCE` 與字串值 `EXTRA_SEGMENTED_SESSION = EXTRA_AUDIO_SOURCE`。Owner 共 5 次實測皆使用分段工作階段，未發生接續；中間停頓超過 5 秒仍能繼續辨識。工作階段由 UI 結束音訊來源來收尾；按下結束到全文完成約 0.38–0.47 秒。

這些測試也釐清三個容易誤用的地方：服務自行收音時可能因停頓而結束；把 `EXTRA_SEGMENTED_SESSION` 設為 Boolean 不是有效請求；用重建 recognizer 接續曾發生 `ERROR_SERVER_DISCONNECTED` 並漏字。P1a 採用 App 控制音訊來源與分段工作階段後，五次真人操作均未走接續路徑。

Gboard 語音可在 WebView 文字欄使用；使用者須允許 Gboard 使用麥克風，並先切換到中文輸入法才能辨識中文。

### TTS 音訊檔結果（非代表性）

36 句 TTS 合成語料須以接近即時速度送入，並在語音後補靜音尾段。Pixel 9 的 `onResults` bundle 為空，辨識文字只出現在 partial 回呼；POC 因此以最後一筆 partial 作為音訊檔辨識結果。

| 類別 | CER |
|---|---:|
| 純中文 | 1.06% |
| 長句 | 3.70% |
| 術語 | 33.52% |
| 中英混用 | 43.11% |
| 指令 | 47.90% |
| 路徑 | 58.75% |

術語錯誤率為 82.69%；3 筆靜音語料皆未誤出文字。音訊結束到結果回傳的延遲 p50 約 103 ms、p95 約 132 ms。這些是 TTS 音訊檔路徑的結果，不可與真人麥克風測得的 0.38–0.47 秒結束延遲混為一談，也不能代表真人語音的錯字比例。

### P1a 結論與後續階段

P1a 階段的隔離測試顯示，Pixel 9 的裝置端繁體中文與飛航模式辨識可用；真人中文口語可辨識，但英文術語、路徑及指令偏弱。後續 P3 已整合至 main，Owner 正在 Pixel 9（Android 17）使用 r3 APK，並回饋語音體驗良好。實際使用仍有待觀察事項，詳見[現況與重新檢視條件](#現況與重新檢視條件)。P2 已決定採用內建裝置端辨識；P1b（Breeze ASR 25）目前不需啟動。若試用中術語錯誤造成的修改負擔不可接受，再評估術語校正或 Breeze 等方案，不預設數值門檻。

## 產品操作流程（P3 已整合）

```text
麥克風（點按／按住） --> 裝置端辨識 --錄音結束--> 單行文字 --約 150 ms--> Enter --> 錄音開始時的分頁
                                                                  |
                                                                  +--> App 顯示已送出的轉寫
錄音中取消／目標失效／App 進背景 ----------------------------------------> 不送辨識結果
文字已送出後目標失效／App 進背景 ----------------------------------------> 略過 Enter；文字不撤回
```

Breeze ASR 25 僅為條件式備案，目前不在 P3 主路徑中。

錄音一開始就綁定當時的分頁與工作世代（session generation）。辨識完成後直接送出，不經可編輯草稿；App 顯示已送出的轉寫，讓使用者看見實際送出的內容。辨識錯字在 shell 中可能執行錯誤指令，這項風險由 Owner 知情接受；本版以目標綁定、目標失效時停止後續送出及單行文字清理降低誤送範圍。

1. 使用者在終端分頁點選大型麥克風按鈕，首次使用才要求錄音權限；拒絕後仍可打字。
2. zh-TW 為預設語言。API 34 裝置若辨識服務支援語言切換，且 zh-TW、en-US 模型可用，可啟用這兩種語言；API 34 中英切換尚未單獨實機驗證。所需裝置端語言模型尚未安裝時，提示使用者由系統下載；沒有可用的裝置端辨識時，停用語音並說明原因，不改走雲端。
3. 設定可選「點按開始／再點按結束」或「按住說話／放開結束」。錄音中以明確的色彩與介面變化顯示狀態，也提供取消操作。停頓不會結束錄音，停頓處補全形逗號；使用者結束錄音後才取得最終結果。
4. 辨識結果送出前移除換行與控制字元，先把單行文字送到錄音開始時的分頁；約 150 ms 後再單獨送出一個結尾 `\r`（即 Enter）。App 顯示已送出的轉寫。本版不提供送出前編輯，編輯留待下一版。
5. 目標分頁及 session generation 必須仍有效；若錄音期間目標已關閉、重連或失效，或使用者取消／App 進入背景，不送出辨識結果。辨識文字已送出後若目標失效或 App 進入背景，略過後續 Enter 並告知使用者；已送出的文字無法撤回。晚到的辨識結果不得改送其他分頁。

## 首階段邊界與整合方向

- 限 Android APK，先驗證 Pixel 9；Windows、持續聽寫、背景錄音、喚醒詞與聲控命令不列入首階段。語音只屬 Android 平台層與 UI，不改變兩平台共用的核心規則。
- 原生層以 `AudioRecord` 自行錄音，透過 pipe 將 PCM 提供給 `SpeechRecognizer`，並以分段工作階段讓 UI 控制音訊來源的開始與結束；WebView 負責錄音操作與結果顯示，只接收文字結果，不提供送出前草稿。
- 切換其他 App／鎖屏時停止錄音並取消本次辨識；Runtime 沿用目前背景保留政策，兩者生命週期分開。
- 不預設保存錄音、辨識歷史或把內容寫入 log；密碼／enrollment 畫面不啟用語音。
- 若改走 Breeze 備案，另需模型獨立下載與管理（大小、版本、來源、校驗、進度、磁碟空間、刪除），不將大型權重放進 APK。

P3 已整合至 main，並由 Owner 在 Pixel 9（Android 17）使用 r3 APK。整合保留上述平台邊界與送出保護；目前仍觀察 Codex CLI 偶爾未自動送出、`⇧←` 在實際 Codex CLI 提問時的操作回饋，以及 API 34 中英語言切換。

## 候選與取捨

| 候選 | 本次定位 | 取捨／待確認 |
|---|---|---|
| Android 裝置端 `SpeechRecognizer` | P2 已決定採用；P3 已整合至 main，Owner 使用中 | Pixel 9 已實測 zh-TW、飛航模式辨識及 UI 控制錄音；Owner 使用 r3 APK 後回饋語音良好。英文術語、路徑與指令偏弱；偶爾未自動送出及 API 34 中英切換仍待觀察。 |
| 使用 Gboard 語音輸入 | 非產品路線的對照 | 已確認可在 WebView 文字欄使用；Gboard 須有麥克風權限並先切換到中文輸入法。離線與否仍由 Gboard 設定決定，App 無法保證。 |
| Breeze ASR 25（自帶模型） | 備案 | 台灣華語與中英混用方向符合；large-v2 架構約需下載 1–2GB 量化權重，手機資源與等待時間待測。 |
| 小型 Whisper 或 SenseVoice | 備案對照 | 若走自帶模型且 25 成本過高，以同一組語料比較。 |
| Breeze ASR 26 | 暫不列入 | 先前討論偏向台語需求；如新增該需求再核對。 |
| 一般 `SpeechRecognizer`／雲端（ChatGPT 類 App、Groq 等託管 Whisper） | 不採用，待另議 | 音訊離開手機、需網路，部分需 API key 與費用；若要採用，另明示資料流與授權。 |

自帶模型路線先調查 sherpa-onnx 的 ONNX 路線，whisper.cpp 作為其他候選；引擎支援、權重來源、大小、hash、量化誤差與授權在進入該路線時確認。

## 後續驗證與選型門檻

| 階段 | 工作 | 必須留下的結果 |
|---|---|---|
| P0：定義體驗 | 整理台灣華語、中英混用、專案名、路徑與指令的代表語句與正確文字，包含短句／較長句與靜音。 | 語句與正確文字、音訊來源與保存方式、評分方法；測試前訂等待時間與準確度門檻。 |
| P1a：內建辨識隔離實驗（已完成） | Pixel 9 查詢語言並觸發下載，以真人麥克風及 TTS 音訊檔測試裝置端辨識，並確認 Gboard 語音可用；不接正式 Runtime 輸入。 | zh-TW 與飛航模式可用；Owner 5 次 UI 控制錄音皆走分段工作階段，停頓逾 5 秒未中斷；真人口語觀察與非代表性 TTS CER 見上節。 |
| P1b：自帶模型實驗（條件式） | 若 P3 試用中術語錯誤造成的改字負擔不可接受，再於 Pixel 9 評估 Breeze ASR 25 和至少一個小模型。 | 權重大小、冷啟動、p50/p95、峰值記憶體、CER、術語錯誤、靜音幻覺、溫度與耗電；不預設數值門檻。 |
| P2：正式選型（已完成） | Owner 決定採用 Android 內建裝置端辨識，Breeze ASR 25 維持條件式備案。 | 本 BDR 記錄選定路線、理由、直接送出 Enter 的風險取捨及保護條件；P3 已整合至 main。 |
| P3：整合與實機使用（已整合，Owner 使用中） | 整合裝置端辨識、手機錄音控制與狀態、語言設定、單行文字清理；錄音結束後直接送文字與 Enter 到錄音開始時仍有效的分頁，並顯示已送出的轉寫。 | Owner 在 Pixel 9（Android 17）使用 r3 APK，回饋語音良好。待觀察：Codex CLI 偶爾未自動送出、`⇧←` 的實際提問操作回饋、API 34 中英語言切換未單獨驗證。 |

具體數值門檻尚未決定，不以模擬器成功當成 Pixel 9 使用體驗驗收。TTS 準確度只能驗證音訊流程，不代表真人語音；目前 Pixel 9 的 r3 使用回饋是 Owner 的實機觀察，不代表其他裝置或廣泛語料的準確度。

## 現況與重新檢視條件

P1a 已完成；Pixel 9 的裝置端繁體中文與飛航模式辨識可用，UI 控制的分段收音通過 Owner 5 次操作測試。TTS 顯示純中文結果良好，但術語、中英混用、指令與路徑的錯誤偏多；TTS 指標不代表真人語音準確度。P2 已決定採用內建裝置端辨識；P3 已整合至 main，Owner 正在 Pixel 9（Android 17）使用 r3 APK，並回饋語音體驗良好。

目前待觀察：Codex CLI 中偶爾未自動送出；`⇧←` 尚待實際遇到 Codex CLI 提問時回饋；API 34 中英語言切換尚未單獨驗證。這些觀察不改變已整合功能與裝置端辨識決策；若術語錯誤造成的修改負擔不可接受，再評估術語校正或 Breeze。若後續需求轉為台語、即時聽寫、多裝置，或需要雲端辨識，重新檢視本決策。

未來計畫入口：[未來方向](BDR-002-future-directions.md)。Android 語音使用方式與平台限制見 [Android](../platforms/android.md)。

## 附錄：實作與參考

以下為選型、規畫與 P1a 結果核對的來源；P3 實機使用現況與待觀察事項列於本文。

- [Android `SpeechRecognizer`](https://developer.android.com/reference/android/speech/SpeechRecognizer)：`createOnDeviceSpeechRecognizer`（API 31）、`checkRecognitionSupport` 與 `triggerModelDownload(Intent)`（API 33）、`triggerModelDownload(Intent, Executor, ModelDownloadListener)`（API 34）。
- [Android `RecognitionSupport`](https://developer.android.com/reference/android/speech/RecognitionSupport)：裝置端已安裝／可下載／線上語言清單。
- [Android `RecognizerIntent`](https://developer.android.com/reference/android/speech/RecognizerIntent)：`EXTRA_LANGUAGE`、`EXTRA_AUDIO_SOURCE` 與 `EXTRA_SEGMENTED_SESSION`（API 33；後者須以字串指定同一 Intent 中設定的 session 類型；實際支援取決於 recognizer 實作）；`EXTRA_ENABLE_LANGUAGE_SWITCH` 與 `EXTRA_LANGUAGE_SWITCH_ALLOWED_LANGUAGES`（API 34；需要支援該功能的辨識服務及對應語言模型）。
- [Breeze ASR 25 官方模型卡](https://huggingface.co/MediaTek-Research/Breeze-ASR-25)：2026-10-11 核對 `model.safetensors` 約 3.1GB。
- [sherpa-onnx 官方專案](https://github.com/k2-fsa/sherpa-onnx)
- P1a 實測證據：POC 任務紀錄（Pixel 9 手動測試與 TTS 語料批次結果）。
