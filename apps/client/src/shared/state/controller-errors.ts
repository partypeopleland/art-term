import { asConnectionError } from '../transport/types'

export function errText(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e)
  const oneLine = msg.replace(/\s+/g, ' ').trim()
  return oneLine.length > 300 ? `${oneLine.slice(0, 300)}…` : oneLine
}

/** Kind → frontend-safe Chinese fallback when the backend message is empty. */
export const KIND_FALLBACK: Record<string, string> = {
  upgradeRequired: '主機 runtime 需要升級',
  replacement: '遠端 runtime 或 Shell 已更換，請重新選擇',
  ttlExpired: '遠端 Shell 已到期',
  takenOver: '此 Shell 已由另一個連線接管',
  renewed: '此 Shell 的連線憑證已更新',
  operatorKilled: '遠端 Shell 已被管理者結束',
  exited: '遠端 Shell 已結束',
  missing: '遠端 Shell 已不存在',
  slowConsumer: '輸出佇列已滿，請重新連線',
  retryExhausted: '自動重連次數已用完，請手動重新連線',
  busy: '連線操作正在進行，請稍後再試',
  cancelled: '本機已中斷連線',
  inputUncertain: '輸入是否送達尚未確認，未自動重送',
  internal: '遠端 runtime 發生內部錯誤',
  gatewayUnavailable: '主機入口暫時無法連到服務，請檢查主機服務與轉送設定後重試',
  protocol: '遠端 runtime 資料格式不符',
  auth: 'Shell 認證被拒絕',
  tls: 'WSS 憑證驗證失敗',
  capacity: '連線佇列或 runtime 容量已達上限',
  throttled: '主機暫時限制請求，請稍後重試',
  transport: '無法連線到目標，請檢查網路',
  timeout: 'Shell 連線中斷或逾時，請確認網路後重新連線',
  target: '目標無法使用或不存在',
  stateChanged: '連線設定已變更，請重新整理後再試'
}

/**
 * Runtime lifecycle-terminal kinds for a declared runtime-agent tab —
 * the backend SessionError camelCase serialization. For these kinds the
 * issued binding can never be reused and an automatic reconnect would
 * silently reach a different runtime, so the tab retires with a catalog
 * refresh and a manual-reselect notice instead.
 */
export const RUNTIME_TERMINAL_KINDS = new Set([
  'replacement',
  'missing',
  'exited',
  'ttlExpired',
  'takenOver',
  'renewed',
  'operatorKilled',
  'stateChanged'
])

/**
 * Display text for any session rejection: a structured ConnectionError
 * keeps its sanitized backend message (never "[object Object]"), with a
 * per-kind Chinese fallback; anything else is flattened by errText.
 */
export function connErrText(e: unknown): string {
  const conn = asConnectionError(e)
  if (conn) {
    const msg = conn.message.replace(/\s+/g, ' ').trim()
    return (msg.length > 300 ? `${msg.slice(0, 300)}…` : msg) ||
      KIND_FALLBACK[conn.kind] || '連線失敗'
  }
  return errText(e)
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
