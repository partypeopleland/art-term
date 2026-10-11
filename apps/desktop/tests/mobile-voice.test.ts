import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import VoiceButton from '../src/mobile/VoiceButton.vue'
import {
  createVoiceController,
  DEFAULT_VOICE_SETTINGS,
  sanitizeVoiceText,
  VOICE_SETTINGS_KEY,
  type VoiceController,
  type VoiceEvent,
  type VoicePlugin,
  type VoiceStatus,
  type VoiceTarget
} from '../src/mobile/voice'

const readyStatus: VoiceStatus = {
  available: true,
  sdkInt: 35,
  permission: 'granted',
  installed: ['zh-TW'],
  supported: ['zh-TW', 'en-US'],
  languageSwitchSupported: true
}

function mockPlugin(overrides: Partial<VoicePlugin> = {}) {
  let callback: ((event: VoiceEvent) => void) | null = null
  const unsubscribe = vi.fn()
  const plugin: VoicePlugin = {
    status: vi.fn(async () => ({ ...readyStatus })),
    requestPermission: vi.fn(async () => ({ permission: 'granted' as const })),
    downloadLanguage: vi.fn(async () => ({ started: true })),
    start: vi.fn(async () => ({ ok: true as const, mechanism: 'segmented-audio-source' as const })),
    stop: vi.fn(async () => ({ ok: true })),
    cancel: vi.fn(async () => ({ ok: true })),
    listen: vi.fn(async (listener: (event: VoiceEvent) => void) => {
      callback = listener
      return unsubscribe
    }),
    ...overrides
  }
  return {
    plugin,
    unsubscribe,
    emit(event: VoiceEvent) { callback?.(event) }
  }
}

function mockTarget(tabId = 'tab/one', generation = 7) {
  let current = true
  let currentGeneration = generation
  let bracketedPasteMode = false
  const send = vi.fn()
  const target: VoiceTarget = {
    tabId,
    generation,
    isValid: () => current && currentGeneration === generation,
    bracketedPasteMode: () => bracketedPasteMode,
    send
  }
  return {
    target,
    send,
    switchAway() { current = false },
    reconnect() { currentGeneration++ },
    setBracketedPasteMode(enabled: boolean) { bracketedPasteMode = enabled },
    restore() { current = true }
  }
}

const controllers: VoiceController[] = []
const wrappers: VueWrapper[] = []

async function readyController(plugin: VoicePlugin, options: Parameters<typeof createVoiceController>[1] = {}) {
  const voice = createVoiceController(plugin, {
    isTauri: () => true,
    makeSessionId: () => 'nonce-test',
    ...options
  })
  controllers.push(voice)
  await voice.initialize()
  return voice
}

function sessionId(plugin: VoicePlugin): string {
  const args = vi.mocked(plugin.start).mock.calls.at(-1)?.[0]
  if (!args) throw new Error('語音工作階段尚未啟動')
  return args.sessionId
}

function mountVoiceButton(voice: VoiceController, target: VoiceTarget) {
  const wrapper = mount(VoiceButton, { props: { voice, target } })
  wrappers.push(wrapper)
  return wrapper
}

function dispatchPointer(element: Element, type: string, pointerId: number): void {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.defineProperties(event, {
    pointerId: { value: pointerId },
    pointerType: { value: 'touch' },
    button: { value: 0 }
  })
  element.dispatchEvent(event)
}

beforeEach(() => {
  localStorage.clear()
})

afterEach(() => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount())
  controllers.splice(0).forEach(voice => voice.dispose())
  vi.useRealTimers()
  vi.restoreAllMocks()
  localStorage.clear()
})

describe('mobile voice controller', () => {
  it('sends plain text first and Enter after 150ms while preventing an overlapping recording', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()

    await voice.start(target.target)
    expect(voice.state.phase).toBe('listening')
    expect(mock.plugin.start).toHaveBeenCalledWith({
      sessionId: expect.stringContaining('voice:tab%2Fone:g7:nonce-test'),
      language: 'zh-TW',
      allowLanguageSwitch: true
    })
    expect(sessionId(mock.plugin)).toContain('voice:tab%2Fone:g7:')

    const id = sessionId(mock.plugin)
    mock.emit({ sessionId: id, kind: 'partial', text: '顯示中的逐字稿' })
    expect(voice.state.partialText).toBe('顯示中的逐字稿')
    await voice.stop()
    expect(voice.state.phase).toBe('stopping')
    vi.useFakeTimers()
    mock.emit({ sessionId: id, kind: 'final', text: '  echo\n  hi\u0001\u0085\u001b ' })

    expect(target.send).toHaveBeenCalledOnce()
    expect(target.send).toHaveBeenCalledWith('echo  hi')
    expect(voice.state.phase).toBe('stopping')
    expect(voice.state.sentText).toBe('')
    await voice.start(mockTarget('tab/two').target)
    expect(mock.plugin.start).toHaveBeenCalledOnce()
    await vi.advanceTimersByTimeAsync(149)
    expect(target.send).toHaveBeenCalledOnce()
    await vi.advanceTimersByTimeAsync(1)
    expect(target.send).toHaveBeenNthCalledWith(2, '\r')
    expect(voice.state.phase).toBe('idle')
    expect(voice.state.sentText).toBe('已送出：echo  hi')
  })

  it('wraps text in bracketed-paste markers when xterm mode is enabled', async () => {
    vi.useFakeTimers()
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()
    target.setBracketedPasteMode(true)
    await voice.start(target.target)
    mock.emit({ sessionId: sessionId(mock.plugin), kind: 'final', text: '貼上文字' })

    expect(target.send).toHaveBeenNthCalledWith(1, '\x1b[200~貼上文字\x1b[201~')
    await vi.advanceTimersByTimeAsync(150)
    expect(target.send).toHaveBeenNthCalledWith(2, '\r')
    expect(voice.state.sentText).toBe('已送出：貼上文字')
  })

  it.each(['switch', 'reconnect', 'background'] as const)(
    'does not send Enter after %s invalidates the target during the delay',
    async reason => {
      vi.useFakeTimers()
      const mock = mockPlugin()
      const voice = await readyController(mock.plugin)
      const target = mockTarget()
      await voice.start(target.target)
      mock.emit({ sessionId: sessionId(mock.plugin), kind: 'final', text: '延遲送出' })
      expect(target.send).toHaveBeenCalledOnce()
      expect(target.send).toHaveBeenCalledWith('延遲送出')

      if (reason === 'switch') target.switchAway()
      else if (reason === 'reconnect') target.reconnect()
      else voice.cancel('background')
      await vi.advanceTimersByTimeAsync(150)

      expect(target.send).toHaveBeenCalledOnce()
      expect(voice.state.phase).toBe('idle')
      expect(voice.state.notice).toContain('文字已送出但 Enter 未送出')
    }
  )

  it('treats a bare idle event as terminal and ignores later results for that session', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()

    await voice.start(target.target)
    const id = sessionId(mock.plugin)
    mock.emit({ sessionId: id, kind: 'partial', text: '未完成逐字稿' })
    mock.emit({ sessionId: id, kind: 'state', state: 'idle' })
    mock.emit({ sessionId: id, kind: 'final', text: '不可送出的結果' })

    expect(voice.state.phase).toBe('idle')
    expect(voice.state.partialText).toBe('')
    expect(target.send).not.toHaveBeenCalled()
  })

  it('ignores delayed starting and listening events after stopping begins', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()

    await voice.start(target.target)
    const id = sessionId(mock.plugin)
    await voice.stop()
    mock.emit({ sessionId: id, kind: 'state', state: 'starting' })
    mock.emit({ sessionId: id, kind: 'state', state: 'listening' })

    expect(voice.state.phase).toBe('stopping')
  })

  it('drops late results when the user switches pages or the captured tab reconnects', async () => {
    for (const invalidate of ['switch', 'reconnect'] as const) {
      const mock = mockPlugin()
      const voice = await readyController(mock.plugin)
      const target = mockTarget()
      await voice.start(target.target)
      const id = sessionId(mock.plugin)
      if (invalidate === 'switch') target.switchAway()
      else target.reconnect()
      mock.emit({ sessionId: id, kind: 'final', text: 'not sent' })
      expect(target.send).not.toHaveBeenCalled()
      expect(voice.state.notice).toContain('未送出')
      voice.dispose()
      controllers.splice(controllers.indexOf(voice), 1)
    }
  })

  it('cancels without sending, including cancellation caused by backgrounding', async () => {
    for (const reason of ['user', 'background'] as const) {
      const mock = mockPlugin()
      const voice = await readyController(mock.plugin)
      const target = mockTarget()
      await voice.start(target.target)
      const id = sessionId(mock.plugin)
      voice.cancel(reason)
      await flushPromises()
      mock.emit({ sessionId: id, kind: 'final', text: 'must be discarded' })
      expect(mock.plugin.cancel).toHaveBeenCalledWith({ sessionId: id })
      expect(target.send).not.toHaveBeenCalled()
      expect(voice.state.notice).toContain(reason === 'background' ? '背景' : '取消')
      voice.dispose()
      controllers.splice(controllers.indexOf(voice), 1)
    }
  })

  it('handles the native BACKGROUND_CANCELLED event without sending final text', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()
    await voice.start(target.target)
    mock.emit({ sessionId: sessionId(mock.plugin), kind: 'error', error: 'BACKGROUND_CANCELLED' })

    expect(target.send).not.toHaveBeenCalled()
    expect(voice.state.phase).toBe('idle')
    expect(voice.state.notice).toContain('背景')
  })

  it('does not send empty transcripts or events from another session', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()
    await voice.start(target.target)
    const id = sessionId(mock.plugin)
    mock.emit({ sessionId: 'stale-session', kind: 'final', text: 'stale' })
    expect(target.send).not.toHaveBeenCalled()
    mock.emit({ sessionId: id, kind: 'final', text: ' \n\u0000\u0085 ' })
    expect(target.send).not.toHaveBeenCalled()
    expect(voice.state.notice).toContain('沒有辨識到文字')
  })

  it('offers a language-pack download after the recognizer reports LANGUAGE_UNAVAILABLE', async () => {
    const mock = mockPlugin({ start: vi.fn().mockRejectedValue('LANGUAGE_UNAVAILABLE') })
    const voice = await readyController(mock.plugin)
    const target = mockTarget()
    await voice.start(target.target)
    expect(voice.state.notice).toContain('語言包')
    expect(voice.state.downloadRequired).toBe(true)

    const wrapper = mountVoiceButton(voice, target.target)
    await wrapper.get('[data-testid="voice-download"]').trigger('click')
    await flushPromises()
    expect(mock.plugin.downloadLanguage).toHaveBeenCalledWith('zh-TW')
    mock.emit({ sessionId: '', kind: 'download', progress: 100, downloadResult: 'success' })
    expect(voice.state.downloadRequired).toBe(false)
    expect(voice.state.notice).toContain('已下載')
  })

  it('requests permission and reports a denial without starting recognition', async () => {
    const mock = mockPlugin({
      status: vi.fn(async () => ({ ...readyStatus, permission: 'prompt' as const })),
      requestPermission: vi.fn(async () => ({ permission: 'denied' as const }))
    })
    const voice = await readyController(mock.plugin)
    await voice.start(mockTarget().target)
    expect(mock.plugin.requestPermission).toHaveBeenCalledOnce()
    expect(mock.plugin.start).not.toHaveBeenCalled()
    expect(voice.state.notice).toContain('權限遭拒')
  })

  it('interrupts a pending permission prompt when stopped or cancelled', async () => {
    for (const interrupt of ['stop', 'cancel'] as const) {
      let grantPermission!: (result: { permission: 'granted' }) => void
      const requestPermission = vi.fn(() => new Promise<{ permission: 'granted' }>(resolve => {
        grantPermission = resolve
      }))
      const mock = mockPlugin({
        status: vi.fn(async () => ({ ...readyStatus, permission: 'prompt' as const })),
        requestPermission
      })
      const voice = await readyController(mock.plugin)
      const startPromise = voice.start(mockTarget().target)

      expect(voice.state.phase).toBe('starting')
      expect(mock.plugin.start).not.toHaveBeenCalled()
      if (interrupt === 'stop') await voice.stop()
      else voice.cancel()
      expect(voice.state.phase).toBe('idle')

      grantPermission({ permission: 'granted' })
      await startPromise
      expect(mock.plugin.start).not.toHaveBeenCalled()
      expect(voice.state.phase).toBe('idle')
      voice.dispose()
      controllers.splice(controllers.indexOf(voice), 1)
    }
  })

  it('persists mode and language-switch settings in localStorage', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    expect(voice.state.settings).toEqual(DEFAULT_VOICE_SETTINGS)
    voice.updateSettings({ mode: 'hold', allowLanguageSwitch: false })
    expect(JSON.parse(localStorage.getItem(VOICE_SETTINGS_KEY) ?? '{}')).toEqual({ mode: 'hold', allowLanguageSwitch: false })

    const reopened = createVoiceController(mock.plugin, { isTauri: () => true })
    controllers.push(reopened)
    expect(reopened.state.settings).toEqual({ mode: 'hold', allowLanguageSwitch: false })
  })

  it('does not probe or expose the native plugin outside the Tauri mobile shell', async () => {
    const mock = mockPlugin()
    const voice = createVoiceController(mock.plugin, { isTauri: () => false })
    controllers.push(voice)
    await voice.initialize()
    expect(mock.plugin.listen).not.toHaveBeenCalled()
    expect(mock.plugin.status).not.toHaveBeenCalled()
    expect(voice.state.pluginPresent).toBe(false)
  })

  it('sanitizes all newlines and C0/C1 control characters', () => {
    expect(sanitizeVoiceText(' a\r\nb\u0000\u007f\u0085\u2028\u001bc ')).toBe('abc')
  })
})

describe('mobile voice button', () => {
  it('uses tap-to-toggle by default and presents partial and sent text', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()
    const wrapper = mountVoiceButton(voice, target.target)
    const button = wrapper.get('[data-testid="voice-mic"]')

    await button.trigger('click')
    await flushPromises()
    expect(mock.plugin.start).toHaveBeenCalledOnce()
    expect(button.attributes('aria-pressed')).toBe('true')
    expect(button.text()).toContain('錄音中 00:00')
    const id = sessionId(mock.plugin)
    mock.emit({ sessionId: id, kind: 'partial', text: '即時辨識' })
    await flushPromises()
    expect(wrapper.get('[data-testid="voice-partial"]').text()).toBe('即時辨識')

    await button.trigger('click')
    await flushPromises()
    expect(mock.plugin.stop).toHaveBeenCalledWith({ sessionId: id })
    vi.useFakeTimers()
    mock.emit({ sessionId: id, kind: 'final', text: '完成文字' })
    await flushPromises()
    expect(button.text()).toContain('辨識處理中…')
    await vi.advanceTimersByTimeAsync(150)
    await flushPromises()
    expect(wrapper.get('[data-testid="voice-status"]').text()).toBe('已送出：完成文字')
  })

  it.each(['pointerup', 'pointercancel'])('%s ends press-and-hold recording with pointer capture', async endEvent => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    voice.updateSettings({ mode: 'hold' })
    const target = mockTarget()
    const wrapper = mountVoiceButton(voice, target.target)
    const element = wrapper.get('[data-testid="voice-mic"]').element as HTMLButtonElement & { setPointerCapture: (pointerId: number) => void }
    const capturePointer = vi.fn<(pointerId: number) => void>()
    element.setPointerCapture = capturePointer

    dispatchPointer(element, 'pointerdown', 19)
    await flushPromises()
    expect(capturePointer).toHaveBeenCalledWith(19)
    expect(mock.plugin.start).toHaveBeenCalledOnce()
    dispatchPointer(element, endEvent, 19)
    await flushPromises()
    expect(mock.plugin.stop).toHaveBeenCalledOnce()
  })

  it('waits for native start to settle before stopping a quickly released hold', async () => {
    let finishStart!: (value: { ok: true; mechanism: 'segmented-audio-source' }) => void
    const start = vi.fn(() => new Promise<{ ok: true; mechanism: 'segmented-audio-source' }>(resolve => { finishStart = resolve }))
    const mock = mockPlugin({ start })
    const voice = await readyController(mock.plugin)
    voice.updateSettings({ mode: 'hold' })
    const wrapper = mountVoiceButton(voice, mockTarget().target)
    const element = wrapper.get('[data-testid="voice-mic"]').element as HTMLButtonElement & { setPointerCapture: (pointerId: number) => void }
    element.setPointerCapture = vi.fn<(pointerId: number) => void>()

    dispatchPointer(element, 'pointerdown', 23)
    expect(voice.state.phase).toBe('starting')
    dispatchPointer(element, 'pointerup', 23)
    expect(voice.state.phase).toBe('stopping')
    expect(mock.plugin.stop).not.toHaveBeenCalled()
    finishStart({ ok: true, mechanism: 'segmented-audio-source' })
    await flushPromises()
    expect(mock.plugin.stop).toHaveBeenCalledOnce()
  })

  it('saves both preferences through the settings panel', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const wrapper = mountVoiceButton(voice, mockTarget().target)
    await wrapper.get('[data-testid="voice-settings-toggle"]').trigger('click')
    await wrapper.get('input[name="voice-mode"][value="hold"]').setValue(true)
    await wrapper.get('.language-switch input[type="checkbox"]').setValue(false)

    expect(voice.state.settings).toEqual({ mode: 'hold', allowLanguageSwitch: false })
    expect(JSON.parse(localStorage.getItem(VOICE_SETTINGS_KEY) ?? '{}')).toEqual({ mode: 'hold', allowLanguageSwitch: false })
  })

  it('locks recording-mode changes until the active recording ends', async () => {
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const wrapper = mountVoiceButton(voice, mockTarget().target)
    await wrapper.get('[data-testid="voice-mic"]').trigger('click')
    await flushPromises()
    await wrapper.get('[data-testid="voice-settings-toggle"]').trigger('click')

    const modeGroup = wrapper.get('.voice-settings fieldset').element as HTMLFieldSetElement
    expect(modeGroup.disabled).toBe(true)
    expect(voice.state.settings.mode).toBe('tap')
  })

  it('keeps the gear panel and status readable on narrow mobile widths', async () => {
    const mock = mockPlugin({ status: vi.fn(async () => ({ ...readyStatus, languageSwitchSupported: false })) })
    const voice = await readyController(mock.plugin)
    const wrapper = mountVoiceButton(voice, mockTarget().target)
    await wrapper.get('[data-testid="voice-settings-toggle"]').trigger('click')
    expect(wrapper.get('.voice-settings-hint').text()).toContain('不支援')
    expect(wrapper.get('[data-testid="voice-mic"]').element.getAttribute('aria-label')).toBe('開始語音輸入')
  })

  it('clears the sent transcript confirmation after five seconds', async () => {
    vi.useFakeTimers()
    const mock = mockPlugin()
    const voice = await readyController(mock.plugin)
    const target = mockTarget()
    const wrapper = mountVoiceButton(voice, target.target)
    await wrapper.get('[data-testid="voice-mic"]').trigger('click')
    await flushPromises()
    const id = sessionId(mock.plugin)
    mock.emit({ sessionId: id, kind: 'final', text: '五秒提示' })
    await flushPromises()
    await vi.advanceTimersByTimeAsync(150)
    await flushPromises()
    expect(wrapper.get('[data-testid="voice-status"]').text()).toBe('已送出：五秒提示')
    await vi.advanceTimersByTimeAsync(5000)
    expect(wrapper.get('[data-testid="voice-status"]').text()).not.toContain('已送出')
  })
})
