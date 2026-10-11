import { addPluginListener, invoke, isTauri } from '@tauri-apps/api/core'
import { reactive } from 'vue'

export type VoicePhase = 'idle' | 'starting' | 'listening' | 'stopping'
export type VoiceMode = 'tap' | 'hold'
export type VoicePermission = 'granted' | 'denied' | 'prompt'

export interface VoiceSettings {
  mode: VoiceMode
  allowLanguageSwitch: boolean
}

export interface VoiceStatus {
  available: boolean
  sdkInt: number
  permission: VoicePermission
  installed: string[]
  supported: string[]
  languageSwitchSupported: boolean
}

export interface VoiceEvent {
  sessionId: string
  kind: 'state' | 'partial' | 'final' | 'error' | 'download'
  state?: VoicePhase
  text?: string
  languageSwitch?: boolean
  error?: string
  progress?: number
  downloadResult?: 'success' | 'scheduled' | 'error'
}

export interface VoicePlugin {
  status(): Promise<VoiceStatus>
  requestPermission(): Promise<{ permission: VoicePermission }>
  downloadLanguage(language: string): Promise<{ started: boolean }>
  start(args: { sessionId: string; language: string; allowLanguageSwitch: boolean }): Promise<{ ok: true; mechanism: 'segmented-audio-source' }>
  stop(args: { sessionId: string }): Promise<{ ok: boolean }>
  cancel(args: { sessionId: string }): Promise<{ ok: boolean }>
  listen(callback: (event: VoiceEvent) => void): Promise<() => void>
}

export interface VoiceTarget {
  tabId: string
  generation: number
  /** Must check tab identity, active page, live status, and generation. */
  isValid(): boolean
  /** Read xterm's current remote bracketed-paste mode for this tab. */
  bracketedPasteMode(): boolean
  send(text: string): void
}

export interface VoiceControllerState {
  pluginPresent: boolean
  recognizerAvailable: boolean
  permission: VoicePermission
  languageSwitchSupported: boolean
  phase: VoicePhase
  partialText: string
  sentText: string
  notice: string
  noticeCode: string
  downloadRequired: boolean
  downloadProgress: number | null
  settings: VoiceSettings
}

export interface VoiceControllerOptions {
  storage?: Pick<Storage, 'getItem' | 'setItem'>
  isTauri?: () => boolean
  makeSessionId?: () => string
}

export interface VoiceController {
  state: VoiceControllerState
  initialize(): Promise<void>
  updateSettings(settings: Partial<VoiceSettings>): void
  start(target: VoiceTarget | null): Promise<void>
  stop(): Promise<void>
  cancel(reason?: 'user' | 'background' | 'target' | 'unmount'): void
  downloadLanguage(): Promise<void>
  clearSentText(): void
  dispose(): void
}

export const VOICE_SETTINGS_KEY = 'art-term.mobile-voice.settings.v1'
export const DEFAULT_VOICE_SETTINGS: VoiceSettings = {
  mode: 'tap',
  allowLanguageSwitch: true
}

const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f\u2028\u2029]/g

export function sanitizeVoiceText(text: string): string {
  return text.replace(CONTROL_CHARACTERS, '').trim()
}

export function loadVoiceSettings(storage?: Pick<Storage, 'getItem'>): VoiceSettings {
  try {
    const actualStorage = storage ?? (typeof localStorage === 'undefined' ? undefined : localStorage)
    const saved = actualStorage?.getItem(VOICE_SETTINGS_KEY)
    if (!saved) return { ...DEFAULT_VOICE_SETTINGS }
    const parsed: unknown = JSON.parse(saved)
    if (!parsed || typeof parsed !== 'object') return { ...DEFAULT_VOICE_SETTINGS }
    const value = parsed as Partial<VoiceSettings>
    return {
      mode: value.mode === 'hold' ? 'hold' : 'tap',
      allowLanguageSwitch: typeof value.allowLanguageSwitch === 'boolean'
        ? value.allowLanguageSwitch
        : DEFAULT_VOICE_SETTINGS.allowLanguageSwitch
    }
  } catch {
    return { ...DEFAULT_VOICE_SETTINGS }
  }
}

function errorCode(error: unknown): string {
  const raw = typeof error === 'string'
    ? error
    : error instanceof Error
      ? error.message
      : String(error)
  const match = raw.toUpperCase().match(/PERMISSION_DENIED|NOT_AVAILABLE|LANGUAGE_UNAVAILABLE|BACKGROUND_CANCELLED|BUSY/)
  return match?.[0] ?? 'UNKNOWN'
}

function errorMessage(code: string): string {
  switch (code) {
    case 'PERMISSION_DENIED': return '麥克風權限遭拒，請至系統設定開啟。'
    case 'NOT_AVAILABLE': return '此裝置不支援語音輸入。'
    case 'LANGUAGE_UNAVAILABLE': return '尚未安裝繁體中文語言包。'
    case 'BACKGROUND_CANCELLED': return 'App 進入背景，錄音已取消。'
    case 'BUSY': return '語音輸入正在使用中，請稍後再試。'
    case 'EMPTY_RESULT': return '沒有辨識到文字，請再試一次。'
    case 'TARGET_INVALID': return '分頁已切換或連線已變更，語音結果未送出。'
    case 'TARGET_INVALID_AFTER_TEXT': return '分頁已切換或連線已變更，文字已送出但 Enter 未送出。'
    case 'BACKGROUND_CANCELLED_AFTER_TEXT': return 'App 進入背景，文字已送出但 Enter 未送出。'
    case 'ENTER_CANCELLED_AFTER_TEXT': return '送出已取消；文字已送出但 Enter 未送出。'
    default: return '語音輸入失敗，請再試一次。'
  }
}

function makeSessionId(tabId: string, generation: number, nonce: string): string {
  return `voice:${encodeURIComponent(tabId)}:g${generation}:${nonce}`
}

function randomNonce(): string {
  try {
    if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') return crypto.randomUUID()
  } catch { /* use the local fallback below */ }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`
}

export function createTauriVoicePlugin(): VoicePlugin {
  return {
    status: () => invoke<VoiceStatus>('plugin:mobile-voice|status'),
    requestPermission: () => invoke<{ permission: VoicePermission }>('plugin:mobile-voice|requestPermission'),
    downloadLanguage: language => invoke<{ started: boolean }>('plugin:mobile-voice|downloadLanguage', { language }),
    start: args => invoke('plugin:mobile-voice|start', args),
    stop: args => invoke('plugin:mobile-voice|stop', args),
    cancel: args => invoke('plugin:mobile-voice|cancel', args),
    listen: async callback => {
      const listener = await addPluginListener<VoiceEvent>('mobile-voice', 'voice', callback)
      return () => { void listener.unregister() }
    }
  }
}

export function createVoiceController(plugin: VoicePlugin, options: VoiceControllerOptions = {}): VoiceController {
  const storage = options.storage
  const state = reactive<VoiceControllerState>({
    pluginPresent: false,
    recognizerAvailable: false,
    permission: 'prompt',
    languageSwitchSupported: false,
    phase: 'idle',
    partialText: '',
    sentText: '',
    notice: '',
    noticeCode: '',
    downloadRequired: false,
    downloadProgress: null,
    settings: loadVoiceSettings(storage)
  })

  let initialized: Promise<void> | null = null
  let disposeListener: (() => void) | null = null
  let disposed = false
  let active: { sessionId: string; target: VoiceTarget } | null = null
  let downloadPending = false
  let pendingStop: string | null = null
  let pendingEnter: { target: VoiceTarget; text: string } | null = null
  let pendingEnterTimer: ReturnType<typeof setTimeout> | null = null
  let operationGeneration = 0

  const clearPendingEnter = () => {
    if (pendingEnterTimer !== null) clearTimeout(pendingEnterTimer)
    pendingEnterTimer = null
    pendingEnter = null
  }

  const clearNotice = () => {
    state.notice = ''
    state.noticeCode = ''
    state.downloadRequired = false
  }

  const showError = (code: string) => {
    state.noticeCode = code
    state.notice = errorMessage(code)
    state.downloadRequired = code === 'LANGUAGE_UNAVAILABLE'
    state.sentText = ''
  }

  const handleEvent = (event: VoiceEvent) => {
    if (event.kind === 'download') {
      state.downloadProgress = typeof event.progress === 'number' ? Math.max(0, Math.min(100, event.progress)) : state.downloadProgress
      if (event.downloadResult === 'success' || event.downloadResult === 'scheduled') {
        state.downloadProgress = null
        downloadPending = false
        state.downloadRequired = false
        state.noticeCode = ''
        state.notice = event.downloadResult === 'success'
          ? '繁體中文語言包已下載。'
          : '語言包下載已排程，完成後即可使用。'
      } else if (event.downloadResult === 'error') {
        state.downloadProgress = null
        downloadPending = false
        showError('UNKNOWN')
      }
      return
    }

    const session = active
    if (!session || event.sessionId !== session.sessionId) return
    if (event.kind === 'state' && event.state) {
      if (state.phase === 'stopping' && (event.state === 'starting' || event.state === 'listening')) return
      if (state.phase === 'listening' && event.state === 'starting') return
      if (event.state === 'idle') {
        active = null
        pendingStop = null
        state.phase = 'idle'
        state.partialText = ''
      } else {
        state.phase = event.state
      }
      if (event.languageSwitch !== undefined) state.languageSwitchSupported = event.languageSwitch
    } else if (event.kind === 'partial') {
      state.partialText = event.text ?? ''
    } else if (event.kind === 'final') {
      const text = sanitizeVoiceText(event.text ?? '')
      active = null
      pendingStop = null
      state.partialText = ''
      if (!text) {
        state.phase = 'idle'
        showError('EMPTY_RESULT')
      } else if (!session.target.isValid()) {
        state.phase = 'idle'
        showError('TARGET_INVALID')
      } else {
        const payload = session.target.bracketedPasteMode()
          ? `\x1b[200~${text}\x1b[201~`
          : text
        try {
          session.target.send(payload)
        } catch {
          state.phase = 'idle'
          showError('UNKNOWN')
          return
        }
        clearNotice()
        state.phase = 'stopping'
        const pending = { target: session.target, text }
        pendingEnter = pending
        pendingEnterTimer = setTimeout(() => {
          if (pendingEnter !== pending) return
          pendingEnter = null
          pendingEnterTimer = null
          if (disposed || !pending.target.isValid()) {
            state.phase = 'idle'
            showError('TARGET_INVALID_AFTER_TEXT')
            return
          }
          try {
            pending.target.send('\r')
            state.phase = 'idle'
            state.sentText = `已送出：${text}`
            clearNotice()
          } catch {
            state.phase = 'idle'
            showError('UNKNOWN')
          }
        }, 150)
      }
    } else if (event.kind === 'error') {
      active = null
      pendingStop = null
      state.phase = 'idle'
      state.partialText = ''
      showError(errorCode(event.error ?? 'UNKNOWN'))
    }
  }

  const stopSession = async (sessionId: string) => {
    try {
      const result = await plugin.stop({ sessionId })
      if (!result.ok && active?.sessionId === sessionId) {
        void plugin.cancel({ sessionId }).catch(() => undefined)
        active = null
        pendingStop = null
        state.phase = 'idle'
        state.partialText = ''
        showError('UNKNOWN')
      }
    } catch (error) {
      if (active?.sessionId === sessionId) {
        void plugin.cancel({ sessionId }).catch(() => undefined)
        active = null
        pendingStop = null
        state.phase = 'idle'
        state.partialText = ''
        showError(errorCode(error))
      }
    }
  }

  const controller: VoiceController = {
    state,
    initialize() {
      if (initialized) return initialized
      initialized = (async () => {
        // MobileApp is imported only after the existing gui_platform Android check.
        // This guard keeps browser-based unit tests from probing a native plugin.
        if (disposed || !(options.isTauri ?? isTauri)()) return
        try {
          const unsubscribe = await plugin.listen(handleEvent)
          if (disposed) {
            unsubscribe()
            return
          }
          disposeListener = unsubscribe
          const status = await plugin.status()
          if (disposed) return
          state.pluginPresent = true
          state.recognizerAvailable = status.available
          state.permission = status.permission
          state.languageSwitchSupported = status.languageSwitchSupported
          if (!status.available) showError('NOT_AVAILABLE')
        } catch {
          disposeListener?.()
          disposeListener = null
          state.pluginPresent = false
        }
      })()
      return initialized
    },
    updateSettings(patch) {
      state.settings = { ...state.settings, ...patch }
      try {
        const actualStorage = storage ?? (typeof localStorage === 'undefined' ? undefined : localStorage)
        actualStorage?.setItem(VOICE_SETTINGS_KEY, JSON.stringify(state.settings))
      } catch { /* Keep the in-memory setting if storage is unavailable. */ }
    },
    async start(target) {
      if (disposed || !state.pluginPresent || state.phase === 'starting' || state.phase === 'listening' || state.phase === 'stopping') return
      if (!target || !target.isValid()) {
        showError('TARGET_INVALID')
        return
      }
      if (!state.recognizerAvailable) {
        showError('NOT_AVAILABLE')
        return
      }
      clearNotice()
      state.sentText = ''
      state.partialText = ''
      state.phase = 'starting'
      const currentOperation = ++operationGeneration
      if (state.permission !== 'granted') {
        try {
          const result = await plugin.requestPermission()
          state.permission = result.permission
        } catch {
          state.permission = 'denied'
        }
        if (disposed || currentOperation !== operationGeneration) return
        if (state.permission !== 'granted') {
          state.phase = 'idle'
          showError('PERMISSION_DENIED')
          return
        }
      }
      if (!target.isValid()) {
        state.phase = 'idle'
        showError('TARGET_INVALID')
        return
      }
      const nonce = options.makeSessionId?.() ?? randomNonce()
      const sessionId = makeSessionId(target.tabId, target.generation, nonce)
      active = { sessionId, target }
      try {
        await plugin.start({ sessionId, language: 'zh-TW', allowLanguageSwitch: state.settings.allowLanguageSwitch })
        if (active?.sessionId === sessionId) {
          if (pendingStop === sessionId) {
            pendingStop = null
            void stopSession(sessionId)
          } else if (state.phase === 'starting') state.phase = 'listening'
        }
      } catch (error) {
        if (active?.sessionId === sessionId) {
          active = null
          pendingStop = null
          state.phase = 'idle'
          state.partialText = ''
          showError(errorCode(error))
        }
      }
    },
    async stop() {
      const session = active
      if (!session && state.phase === 'starting') {
        operationGeneration++
        state.phase = 'idle'
        state.partialText = ''
        return
      }
      if (!session || (state.phase !== 'starting' && state.phase !== 'listening')) return
      const starting = state.phase === 'starting'
      state.phase = 'stopping'
      if (starting) {
        pendingStop = session.sessionId
        return
      }
      await stopSession(session.sessionId)
    },
    cancel(reason = 'user') {
      const session = active
      const hadPendingEnter = pendingEnter !== null
      const wasActive = !!session || state.phase !== 'idle' || hadPendingEnter
      if (!wasActive) return
      operationGeneration++
      clearPendingEnter()
      active = null
      pendingStop = null
      state.phase = 'idle'
      state.partialText = ''
      state.sentText = ''
      state.downloadRequired = false
      if (session) void plugin.cancel({ sessionId: session.sessionId }).catch(() => undefined)
      if (reason === 'background') showError(hadPendingEnter ? 'BACKGROUND_CANCELLED_AFTER_TEXT' : 'BACKGROUND_CANCELLED')
      else if (reason === 'target') showError(hadPendingEnter ? 'TARGET_INVALID_AFTER_TEXT' : 'TARGET_INVALID')
      else if (reason === 'user') {
        if (hadPendingEnter) showError('ENTER_CANCELLED_AFTER_TEXT')
        else {
          state.noticeCode = ''
          state.notice = '錄音已取消。'
        }
      }
      // Unmount cancellation is silent because the containing surface is leaving.
    },
    async downloadLanguage() {
      if (!state.downloadRequired || downloadPending) return
      downloadPending = true
      state.downloadProgress = 0
      state.notice = '正在下載繁體中文語言包…'
      try {
        const result = await plugin.downloadLanguage('zh-TW')
        if (!result.started && downloadPending) {
          downloadPending = false
          state.downloadProgress = null
          showError('UNKNOWN')
        }
      } catch {
        downloadPending = false
        state.downloadProgress = null
        showError('UNKNOWN')
      }
    },
    clearSentText() {
      state.sentText = ''
    },
    dispose() {
      if (disposed) return
      disposed = true
      controller.cancel('unmount')
      disposeListener?.()
      disposeListener = null
    }
  }
  return controller
}
