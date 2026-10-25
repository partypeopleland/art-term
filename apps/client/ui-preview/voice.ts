import type { VoiceEvent, VoicePlugin } from '../src/mobile/voice'

/** Browser-only voice state for rendering the actual Android controls. No microphone access. */
export function createPreviewVoicePlugin(): VoicePlugin {
  let receive: ((event: VoiceEvent) => void) | null = null
  let sessionId: string | null = null
  let timer: ReturnType<typeof setTimeout> | undefined
  function clear(): void { clearTimeout(timer); timer = undefined; sessionId = null }
  return {
    async status() { return { available: true, sdkInt: 36, permission: 'granted', installed: ['zh-TW'], supported: ['zh-TW', 'en-US'], languageSwitchSupported: true } },
    async requestPermission() { return { permission: 'granted' } },
    async downloadLanguage() { return { started: false } },
    async listen(callback) { receive = callback; return () => { clear(); receive = null } },
    async start(args) {
      clear()
      sessionId = args.sessionId
      timer = setTimeout(() => {
        if (sessionId !== args.sessionId) return
        receive?.({ sessionId: args.sessionId, kind: 'state', state: 'listening' })
        receive?.({ sessionId: args.sessionId, kind: 'partial', text: '這裡顯示語音辨識文字（示範）' })
      }, 100)
      return { ok: true, mechanism: 'segmented-audio-source' }
    },
    async stop(args) {
      if (sessionId !== args.sessionId) return { ok: false }
      clearTimeout(timer)
      timer = setTimeout(() => {
        if (sessionId !== args.sessionId) return
        receive?.({ sessionId: args.sessionId, kind: 'final', text: '這是語音輸入示範' })
        sessionId = null
      }, 100)
      return { ok: true }
    },
    async cancel() { clear(); return { ok: true } }
  }
}
