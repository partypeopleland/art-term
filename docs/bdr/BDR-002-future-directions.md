# BDR-002：未來方向與保護條件

- 狀態：提案／未排程，未啟動實作、部署或效能改版。
- 來源：整併原 roadmap 與 future-work 的候選需求；語音完整決策保留在 [BDR-001](BDR-001-android-voice-input.md)。

## App 獨立桌面觀看入口

2026-10-09 記錄的需求是讓 Agent 繼續使用 Host instance-mcp 操作瀏覽器，而 App 直接觀看同一 Host 桌面，避免經 Agent Pod 轉送圖片。這與目前已提供的 [Native Runtime 共享截圖](../refarch/desktop.md) 不同；現行能力仍經明確 Runtime origin 的管理 API，獨立觀看入口尚未實作。

提案希望把觀看與 Agent 操作分成兩條路徑，兩者指向同一桌面：

```text
Proposed: independent viewing (not implemented)

App   -- viewing authorization --> Host desktop service --+
                                                          +--> Same desktop
Agent -- operation authorization --> Host instance-mcp ---+
```

這樣觀看者可以取得畫面，而不必持有 Agent 操作桌面的權限。這張圖表達產品期望的分工，尚未決定實際觀看協定或網址；目前 App 仍透過 Runtime 取得截圖。

後續需先確認可信 Tailnet HTTPS、觀看與 Agent 操作的獨立授權、精確 Host／桌面綁定、合理的新增連線預填，避免手機手打長 token。預填不得自動儲存或連線；Agent PTY 入口不能推定為桌面 MCP 入口，操作 credential 不能交給觀看介面。公開例子用虛構值，私人資料透過既有受控設定或匯入方式提供，不恢復私有 bootstrap。endpoint／port／授權格式需在實作階段查核，目前不新增服務、路由或部署。

## 其他候選

| 候選 | 先取得的證據 | 必須保護 |
|---|---|---|
| Android 本地語音（已整合） | P3 已整合 main，Owner 正在 Pixel 9 使用 r3 APK，並回饋語音體驗良好。待觀察：Codex CLI 偶爾未自動送出、`⇧←` 的實際提問操作回饋、API 34 中英語言切換尚未單獨驗證。Breeze ASR 25 維持條件式備案。詳 [BDR-001](BDR-001-android-voice-input.md) | 綁定錄音開始時的 tab／session generation；錄音中取消、目標失效或背景時不送結果；文字送出後目標失效或背景時略過 Enter，已送文字不撤回；移除換行及控制字元 |
| 多分頁與耗電量測 | 1／4／16 Tab 延遲、CPU、手機電量與背景行為 | queue bounds、UTF-8、backpressure、ACK 與 exit 順序 |
| Host preflight／可攜性 | 版本、相依、能力及可回復安裝 | 依實際授權操作、保留既有憑證與工作 |
| Web 入口／provider 擴充 | server auth、credential storage、transport、多使用者隔離 | 不把本機信任 guard 當 server 授權，不建立第二套不一致核心 |

上述沒有完成日期或效能結論。正式選型、技術設計與驗收依後續明示範圍進行；現行架構見 [architecture](../refarch/architecture.md)。
