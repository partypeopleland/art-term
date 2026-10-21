// Fixed display text for runtime Host management failures. The backend
// returns HostError {kind,message} where kind is a serialized ApiError; the
// transport preserves it via ConnectionError/asConnectionError. We key on
// the exact kind only and never render the backend message, so a malformed
// or hostile payload cannot inject URLs, tokens, or instructions into the UI.
import { asConnectionError } from './types'

const HOST_KIND_TEXT: Record<string, string> = {
  auth: '[AUTH] 主機管理認證被拒絕：請編輯 Host，重新貼上管理 token 本身（不要加 Bearer 前綴）。',
  tls: '[TLS] HTTPS 憑證驗證失敗：請確認網址主機名稱與伺服器憑證相符，且裝置日期與時間正確。',
  transport: '[TRANSPORT] 無法連到服務：請確認裝置已連上 Tailnet，且網址使用服務的 HTTPS 主機名稱。',
  timeout: '[TIMEOUT] 主機回應逾時：請確認網路連線後重試。',
  gatewayUnavailable: '[GATEWAY_UNAVAILABLE] 主機入口暫時無法連到服務：請檢查主機服務與轉送設定後重試。',
  throttled: '[THROTTLED] 主機暫時限制請求：請稍後重試。',
  capacity: '[CAPACITY] 主機或本機資料已達容量限制。',
  sessionMissing: '[SESSION_MISSING] 遠端 Shell 已不存在。',
  sessionDead: '[SESSION_DEAD] 遠端 Shell 已結束。',
  upgradeRequired: '[UPGRADE_REQUIRED] 主機 runtime 版本過舊，請升級後再試。',
  replacement: '[REPLACEMENT] 遠端 runtime 或 Shell 已更換：請重新選擇。',
  protocol: '[PROTOCOL] 主機回應格式不符：請確認使用完整 HTTPS 主機網址與受支援的 runtime 版本。',
  stateChanged: '[STATE_CHANGED] Host 設定已變更：請關閉後重新開啟再試。',
  storage: '[STORAGE] 無法讀寫受保護的管理憑證，原有資料已保留。'
}

/**
 * Display text for a Host save/list/delete/test rejection: a known
 * structured kind maps to its fixed message; anything else (plain errors,
 * unknown or malformed envelopes) returns the caller's generic fallback.
 * The backend message is never rendered.
 */
export function hostErrorText(e: unknown, fallback: string): string {
  const conn = asConnectionError(e)
  return conn && Object.hasOwn(HOST_KIND_TEXT, conn.kind) ? HOST_KIND_TEXT[conn.kind] : fallback
}
