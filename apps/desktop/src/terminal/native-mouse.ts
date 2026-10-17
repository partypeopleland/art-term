import { onBeforeUnmount, onMounted, watch } from 'vue'
import type { HostHistoryRequestFn } from './native'

/** Only controls tmux's native mouse lease. No output capture or rendering. */
export function useNativeMouse(enabled: () => boolean, session: () => string | null, request: () => HostHistoryRequestFn, notice: (text: string) => void) {
  let current: { request: HostHistoryRequestFn; stopped: boolean; busy: boolean; retry: number } | undefined
  let timer: ReturnType<typeof setInterval> | undefined
  let retryTimer: ReturnType<typeof setTimeout> | undefined
  let disposed = false
  let serial = Promise.resolve()
  function stop() {
    const owned = current
    current = undefined
    if (retryTimer) clearTimeout(retryTimer)
    if (!owned) return
    owned.stopped = true
    serial = serial.catch(() => {}).then(async () => { await owned.request({ op: 'close' }).catch(() => {}) })
  }
  function renew() {
    const owned = current
    if (!owned || owned.stopped || owned.busy || document.hidden || disposed) return
    owned.busy = true
    serial = serial.catch(() => {}).then(async () => {
      if (owned.stopped || disposed || document.hidden) return
      try {
        const reply = await owned.request({ op: 'terminalMouse', enabled: true })
        if (owned.stopped || disposed) return
        if (reply.kind === 'ack') { owned.retry = 0; return }
        if (reply.kind !== 'error' || reply.error !== 'busy') {
          notice('主機尚未支援原生終端捲動，請更新 tmux helper；即時輸入不受影響。')
          return
        }
        throw new Error('busy')
      } catch {
        if (!owned.stopped && !disposed && owned.retry++ < 2) retryTimer = setTimeout(renew, 300)
        else if (!owned.stopped && !disposed) notice('原生捲動控制暫時無法啟用，請切換分頁重試。')
      } finally { owned.busy = false }
    })
  }
  function sync() {
    stop()
    if (disposed || !enabled() || !session() || document.hidden) return
    current = { request: request(), stopped: false, busy: false, retry: 0 }
    renew()
  }
  watch(() => [enabled(), session(), request()], sync, { immediate: true })
  onMounted(() => { document.addEventListener('visibilitychange', sync); timer = setInterval(renew, 60000) })
  onBeforeUnmount(() => { disposed = true; stop(); if (timer) clearInterval(timer); document.removeEventListener('visibilitychange', sync) })
}
