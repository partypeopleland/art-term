# BDR-002：未來方向與保護條件

- 狀態：其餘候選未排程；獨立桌面入口已由 Owner 決定採用直接 MCP 設計，實際驗收依當次交付結果。
- 來源：整併原 roadmap 與 future-work 的候選需求；語音完整決策保留在 [BDR-001](BDR-001-android-voice-input.md)。

## App 獨立桌面觀看入口

2026-10-09 提出的獨立觀看需求，於 2026-10-15 由 Owner 決定採用直接 instance-mcp，取代先前「App 不持有 MCP 操作憑證」的提案限制。App 原生層保存平台加密 Screen token 並直接取得截圖；受控端 AI 經記憶體 stdio adapter 操作相同上游。Host 與 Screen 各自獨立，純終端不需要桌面。

```text
App native -- screenshot ------------------> instance-mcp -> same desktop
AI CLI ---- stdio memory-only adapter -----> instance-mcp -> desktop/tools
```

Screen 選擇限定本機分頁，多個 Shell 可共用上游，不實施獨占或重設桌面。WebView 不讀回 token；明確接入工具才配置 adapter，不自動中斷既有 AI。預覽成功不代表工具接入。完整 MCP 結果保留，避免 PTY 傳圖與工具內容過濾是兩件事。

目前主題以[桌面](../refarch/desktop.md)、[工具 bridge](../refarch/runtime-desktop.md)及[連線設定](../refarch/connections.md)為準。沒有私人預填或 bootstrap；現場部署、真 Codex 使用與手機驗收必須各自有實際結果，不能沿用舊 helper 的驗收或以固定 client 冒稱真 AI 已測。

## 其他候選

| 候選 | 先取得的證據 | 必須保護 |
|---|---|---|
| Android 本地語音（已整合） | P3 已整合 main，Owner 正在 Pixel 9 使用 r3 APK，並回饋語音體驗良好。待觀察：Codex CLI 偶爾未自動送出、`⇧←` 的實際提問操作回饋、API 34 中英語言切換尚未單獨驗證。Breeze ASR 25 維持條件式備案。詳 [BDR-001](BDR-001-android-voice-input.md) | 綁定錄音開始時的 tab／session generation；錄音中取消、目標失效或背景時不送結果；文字送出後目標失效或背景時略過 Enter，已送文字不撤回；移除換行及控制字元 |
| 多分頁與耗電量測 | 1／4／16 Tab 延遲、CPU、手機電量與背景行為 | queue bounds、UTF-8、backpressure、ACK 與 exit 順序 |
| Host preflight／可攜性 | 版本、相依、能力及可回復安裝 | 依實際授權操作、保留既有憑證與工作 |
| Web 入口／provider 擴充 | server auth、credential storage、transport、多使用者隔離 | 不把本機信任 guard 當 server 授權，不建立第二套不一致核心 |

上述沒有完成日期或效能結論。正式選型、技術設計與驗收依後續明示範圍進行；現行架構見 [architecture](../refarch/architecture.md)。
