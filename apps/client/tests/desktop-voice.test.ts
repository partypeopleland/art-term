import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import App from '../src/desktop/App.vue'
import VoiceBar from '../src/desktop/VoiceBar.vue'
import { DESKTOP_VOICE_SETTINGS_KEY, createDesktopVoiceController } from '../src/desktop/voice'
import { type VoiceController, type VoiceEvent, type VoicePlugin, type VoiceStatus, type VoiceTarget } from '../src/shared/voice'
import type { AppController } from '../src/shared/state/controller'
import { FakeTerm, FakeTransport, makeHost, makeProfile, makeTarget, deferred } from './fake'

vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })

const readyStatus: VoiceStatus = {
  available: true,
  permission: 'granted',
  installed: ['zh-TW'],
  supported: ['zh-TW'],
  languageSwitchSupported: false
}

function mockPlugin(overrides: Partial<VoicePlugin> = {}) {
  let callback: ((event: VoiceEvent) => void) | null = null
  const plugin: VoicePlugin = {
    status: vi.fn(async () => ({ ...readyStatus })),
    requestPermission: vi.fn(async () => ({ permission: 'granted' as const })),
    start: vi.fn(async () => ({ ok: true, mechanism: 'windows-sapi' })),
    stop: vi.fn(async () => ({ ok: true })),
    cancel: vi.fn(async () => ({ ok: true })),
    listen: vi.fn(async (listener: (event: VoiceEvent) => void) => {
      callback = listener
      return () => {}
    }),
    ...overrides
  }
  return {
    plugin,
    emit(event: VoiceEvent) { callback?.(event) },
    sessionId() {
      const args = vi.mocked(plugin.start).mock.calls.at(-1)?.[0]
      if (!args) throw new Error('voice session not started')
      return args.sessionId
    }
  }
}

function mockTarget(tabId = 'pane/one', generation = 3) {
  const send = vi.fn()
  const target: VoiceTarget = {
    tabId,
    generation,
    isValid: () => true,
    bracketedPasteMode: () => false,
    send
  }
  return { target, send }
}

async function flush(times = 40) {
  for (let i = 0; i < times; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

const wrappers: VueWrapper[] = []
const controllers = new Set<VoiceController>()

beforeEach(() => {
  localStorage.clear()
  document.body.innerHTML = ''
})

afterEach(() => {
  wrappers.splice(0).forEach(wrapper => wrapper.unmount())
  controllers.forEach(voice => voice.dispose())
  controllers.clear()
  vi.useRealTimers()
  vi.restoreAllMocks()
  localStorage.clear()
})

function mountVoiceBar(voicePlugin: VoicePlugin) {
  const voice = createDesktopVoiceController(voicePlugin, { isTauri: () => true })
  controllers.add(voice)
  void voice.initialize()
  const target = mockTarget()
  const wrapper = mount(VoiceBar, { props: { voice, target: target.target } })
  wrappers.push(wrapper)
  return { voice, wrapper, target }
}

function dispatchPointer(element: Element, type: string, pointerId: number): void {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.defineProperties(event, {
    pointerId: { value: pointerId },
    pointerType: { value: 'mouse' },
    button: { value: 0 }
  })
  element.dispatchEvent(event)
}

describe('desktop voice bar', () => {
  it('supports tap mode and shows partial plus sent transcript', async () => {
    const mock = mockPlugin()
    const { wrapper } = mountVoiceBar(mock.plugin)
    await flush()
    const mic = wrapper.get('[data-testid="desktop-voice-mic"]')
    await mic.trigger('click')
    await flushPromises()
    expect(mock.plugin.start).toHaveBeenCalledWith(expect.objectContaining({ language: 'zh-TW' }))
    const id = mock.sessionId()
    mock.emit({ sessionId: id, kind: 'partial', text: '逐字稿' })
    await flushPromises()
    expect(wrapper.get('[data-testid="desktop-voice-partial"]').text()).toBe('逐字稿')
    vi.useFakeTimers()
    mock.emit({ sessionId: id, kind: 'final', text: '完成文字' })
    await flushPromises()
    await vi.advanceTimersByTimeAsync(150)
    await flushPromises()
    expect(wrapper.get('[data-testid="desktop-voice-status"]').text()).toBe('已送出：完成文字')
  })

  it('supports hold mode with pointer down/up and waits for native start before stopping', async () => {
    localStorage.setItem(DESKTOP_VOICE_SETTINGS_KEY, JSON.stringify({ mode: 'hold', allowLanguageSwitch: true }))
    const gate = deferred<{ ok: boolean; mechanism: string }>()
    const mock = mockPlugin({ start: vi.fn(() => gate.promise) })
    const { wrapper } = mountVoiceBar(mock.plugin)
    await flush()
    const element = wrapper.get('[data-testid="desktop-voice-mic"]').element as HTMLButtonElement & { setPointerCapture: (id: number) => void }
    element.setPointerCapture = vi.fn()

    dispatchPointer(element, 'pointerdown', 7)
    expect(mock.plugin.start).toHaveBeenCalledOnce()
    dispatchPointer(element, 'pointerup', 7)
    expect(mock.plugin.stop).not.toHaveBeenCalled()
    gate.resolve({ ok: true, mechanism: 'windows-sapi' })
    await flushPromises()
    expect(mock.plugin.stop).toHaveBeenCalledOnce()
  })

  it('persists the recording mode under the desktop settings key', async () => {
    const mock = mockPlugin()
    const { voice, wrapper } = mountVoiceBar(mock.plugin)
    await flush()
    await wrapper.get('[data-testid="desktop-voice-settings-toggle"]').trigger('click')
    await wrapper.get('input[name="desktop-voice-mode"][value="hold"]').setValue(true)
    expect(voice.state.settings.mode).toBe('hold')
    expect(JSON.parse(localStorage.getItem(DESKTOP_VOICE_SETTINGS_KEY) ?? '{}')).toMatchObject({ mode: 'hold' })
    expect(localStorage.getItem('art-term.mobile-voice.settings.v1')).toBeNull()
  })
})

describe('desktop app voice target', () => {
  function setup(voicePlugin: VoicePlugin) {
    const transport = new FakeTransport([makeProfile('host', 'My computer')])
    const target = makeTarget('shell', 'host', 'host', true, false)
    transport.runtimeSessionsData.set('host', makeHost('host', [target]))
    const terms: FakeTerm[] = []
    const wrapper = mount(App, {
      props: {
        transport,
        terminalFactory: () => {
          const term = new FakeTerm()
          terms.push(term)
          return term
        },
        controllerOptions: { idlePollMs: 0 },
        voicePlugin
      },
      attachTo: document.body
    })
    wrappers.push(wrapper)
    const controller = (wrapper.vm as unknown as { controller: AppController }).controller
    return { wrapper, controller, transport, terms, target }
  }

  it('sends the transcript to the focused live pane and then Enter', async () => {
    const mock = mockPlugin()
    const { controller, transport, target } = setup(mock.plugin)
    await flush()
    controller.openTarget({ dto: makeHost('host', [target]), expanded: true }, target, false)
    await flush()
    const tab = controller.tabs[0]
    expect(tab.status).toBe('live')

    const mic = document.querySelector('[data-testid="desktop-voice-mic"]') as HTMLButtonElement
    expect(mic).toBeTruthy()
    mic.click()
    await flushPromises()
    expect(mock.plugin.start).toHaveBeenCalledOnce()
    const id = mock.sessionId()
    mock.emit({ sessionId: id, kind: 'partial', text: '你好' })
    vi.useFakeTimers()
    mock.emit({ sessionId: id, kind: 'final', text: '你好 世界' })
    await flushPromises()
    const sid = tab.sessionId!
    expect(transport.sessions.get(sid)!.writes).toEqual([Array.from(new TextEncoder().encode('你好 世界'))])
    await vi.advanceTimersByTimeAsync(150)
    await flushPromises()
    expect(transport.sessions.get(sid)!.writes.at(-1)).toEqual([13])
    expect(document.querySelector('[data-testid="desktop-voice-status"]')?.textContent).toBe('已送出：你好 世界')
  })

  it('drops a late transcript after pane focus moves to a new split', async () => {
    const mock = mockPlugin()
    const { controller, transport, target } = setup(mock.plugin)
    await flush()
    controller.openTarget({ dto: makeHost('host', [target]), expanded: true }, target, false)
    await flush()
    const tab = controller.tabs[0]
    const mic = document.querySelector('[data-testid="desktop-voice-mic"]') as HTMLButtonElement
    mic.click()
    await flushPromises()
    const id = mock.sessionId()

    await controller.splitPane(tab, tab.key, 'right')
    await flush()
    expect(mock.plugin.cancel).toHaveBeenCalledWith({ sessionId: id })
    mock.emit({ sessionId: id, kind: 'final', text: '不應送出' })
    await flushPromises()
    expect(transport.sessions.get(tab.sessionId!)!.writes).toEqual([])
  })

  it('shows the native status detail when recognition is unavailable', async () => {
    const mock = mockPlugin({
      status: vi.fn(async () => ({
        ...readyStatus,
        available: false,
        permission: 'denied' as const,
        detail: '找不到可用的麥克風輸入裝置，請檢查 Windows 麥克風設定。'
      }))
    })
    const { controller, target } = setup(mock.plugin)
    await flush()
    controller.openTarget({ dto: makeHost('host', [target]), expanded: true }, target, false)
    await flush()
    expect(document.querySelector('[data-testid="desktop-voice-status"]')?.textContent).toContain('麥克風')
    const mic = document.querySelector('[data-testid="desktop-voice-mic"]') as HTMLButtonElement
    expect(mic.disabled).toBe(true)
  })
})

