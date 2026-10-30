import { attachMobileProfile, showMobileMain } from './mobile-actions'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import MobileApp from '../src/mobile/MobileApp.vue'
import type { AppController } from '../src/shared/state/controller'
import { ConnectionError } from '../src/shared/transport/types'
import { FakeMobileTransport, makeCatalogTarget, makeGroup, makeProfile } from './mobile.fake'
import { FakeTerm, deferred } from './fake'
import type { RuntimeHostDto } from '../src/shared/transport/types'
import { NativeKeyboardKey, type NativeKeyboardListener } from '../src/shared/terminal/native'

// jsdom lacks ResizeObserver; TerminalPane observes its element.
vi.stubGlobal(
  'ResizeObserver',
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
)

async function flush(times = 40): Promise<void> {
  for (let i = 0; i < times; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

interface Mounted {
  wrapper: ReturnType<typeof mount>
  controller: AppController
  transport: FakeMobileTransport
  terms: FakeTerm[]
}

function mountMobile(transport: FakeMobileTransport, keyboard?: NativeKeyboardListener): Mounted {
  const terms: FakeTerm[] = []
  const wrapper = mount(MobileApp, {
    props: {
      transport,
      terminalFactory: () => {
        const t = new FakeTerm()
        terms.push(t)
        return t
      },
      controllerOptions: { idlePollMs: 0, autoCaptureMs: 30_000 }
    },
    global: { provide: keyboard ? { [NativeKeyboardKey as symbol]: keyboard } : {} },
    attachTo: document.body
  })
  const vm = wrapper.vm as unknown as { controller: AppController }
  return { wrapper, controller: vm.controller, transport, terms }
}

/**
 * The shipped runtime profile list: agents = saved runtime Host profiles
 * whose backend metadata declares a configured runtime_agents match;
 * computers = every other runtime profile. Openable targets are opaque
 * backend-issued ids from the per-profile session list.
 */
const PROFILES: RuntimeHostDto[] = [
  makeProfile('hana', 'hana', { category: 'agent', agentName: 'hana', agentTool: 'Codex' }),
  makeProfile('beta', 'beta', { category: 'agent', agentName: 'beta', agentTool: 'Devin' }),
  makeProfile('gamma', 'gamma', { category: 'agent', agentName: 'gamma', agentTool: 'Kiro' }),
  makeProfile('nuc', 'nuc')
]

function seed(t: FakeMobileTransport): FakeMobileTransport {
  const hana = makeCatalogTarget('opq-a1', 'hana', 'hana', 'Codex')
  t.runtimeSessionsData.set('hana', makeGroup('hana', [hana], 'hana'))
  t.runtimeSessionsData.set('beta', makeGroup('beta', [makeCatalogTarget('opq-a2', 'beta', 'beta', 'Devin')], 'beta'))
  t.runtimeSessionsData.set('gamma', makeGroup('gamma', [makeCatalogTarget('opq-a3', 'gamma', 'gamma', 'Kiro')], 'gamma'))
  t.runtimeSessionsData.set(
    'nuc',
    makeGroup('nuc', [makeCatalogTarget('opq-c1', 'nuc', 'nuc', 'Ubuntu 24.04.4 LTS', 'host', true)], 'nuc')
  )
  return t
}

function runtime(): FakeMobileTransport {
  return seed(new FakeMobileTransport(PROFILES))
}

describe('manual coding CLI launch', () => {
  it('keeps launch controls absent and sends only manually typed terminal input', async () => {
    const m = mountMobile(runtime())
    try {
      await flush(); await tapTarget(m, 'hana')
      const tab = m.controller.tabs[0], first = tab.sessionId!
      expect(m.wrapper.find('[data-testid="launch-coding-cli"]').exists()).toBe(false)
      expect(m.transport.sessions.get(first)!.writes).toEqual([])
      m.controller.sendInput(tab, 'codex\r'); await flush()
      expect(m.transport.sessions.get(first)!.writes.map(bytes => new TextDecoder().decode(Uint8Array.from(bytes)))).toEqual(['codex\r'])
    } finally { m.wrapper.unmount() }
  })
})

/** Attach the listed Shell for the named profile. */
async function tapTarget(m: Mounted, label: string): Promise<void> {
  await attachMobileProfile(m.wrapper, label)
  await flush()
}

const SCREEN = { id: 'screen-test', revision: 1, name: 'Test Screen', mcpUrl: 'https://screen.example.test/mcp', tokenPresent: true }
async function chooseMobileScreen(m: Mounted): Promise<void> {
  Object.assign(m.transport, {
    instanceDesktops: async () => [SCREEN],
    instanceSelect: async () => ({ desktop: SCREEN, toolReady: false }),
    instanceClear: async () => {}
  })
  m.transport.shot = { ...m.transport.shot, desktopId: SCREEN.id, desktopRevision: SCREEN.revision }
  await m.controller.loadInstanceDesktops()
  await m.wrapper.get('[data-testid="desktop-toggle"]').trigger('click')
  await flush()
  await m.wrapper.get('[data-testid="choose-screen-screen-test"]').trigger('click')
  await flush()
}

function compEvent(type: string, data = ''): Event {
  const e = new Event(type, { bubbles: true })
  Object.assign(e, { data })
  return e
}

function setHidden(value: boolean): void {
  const spy = vi.spyOn(document, 'hidden', 'get').mockReturnValue(value)
  document.dispatchEvent(new Event('visibilitychange'))
  spy.mockRestore()
}

describe('mobile App runtime connection list (no Tauri)', () => {
  beforeEach(() => {
    document.body.innerHTML = ''
  })
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('home lists saved runtime agents and computers directly — no keys, no discovery', async () => {
    const t = runtime()
    const { wrapper, controller } = mountMobile(t)
    await flush()

    // Category groups with names + tool subtitles.
    expect(wrapper.find('[data-testid="cat-agents"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="cat-computers"]').exists()).toBe(true)
    expect(wrapper.findAll('.connection-section h2').map(heading => heading.text())).toEqual(['Computers', 'Agents', 'Screens'])
    expect(wrapper.find('[data-testid="cat-screens"]').text()).toContain('尚未新增 Screen')
    expect(wrapper.find('[data-testid="cat-screens"] .attach-button').exists()).toBe(false)
    const text = wrapper.text()
    for (const name of ['hana', 'beta', 'gamma', 'nuc']) expect(text).toContain(name)
    for (const sub of ['Codex', 'Devin', 'Kiro', 'openab-pty'])
      expect(text).toContain(sub)

    // No onboarding form / public key on the home page.
    expect(wrapper.find('[data-testid="add-host"]').exists()).toBe(false)
    expect(wrapper.find('[data-testid="pubkey"]').exists()).toBe(false)
    expect(text).not.toContain('SHA256:identityfp')

    // Read-only Shell lists load automatically, without opening a session.
    expect(t.openCalls).toHaveLength(0)
    expect(controller.runtimeProfilesError.value).toBe('')
    wrapper.unmount()
  })

  it('keeps all dashboard sections visible with no profiles or remote queries', async () => {
    const t = new FakeMobileTransport()
    const { wrapper } = mountMobile(t)
    await flush()
    expect(wrapper.find('[data-testid="cat-computers"]').text()).toContain('尚未新增主機連線')
    expect(wrapper.find('[data-testid="cat-agents"]').text()).toContain('尚未新增工作助理')
    expect(wrapper.find('[data-testid="cat-screens"]').exists()).toBe(true)
    expect(t.openCalls).toHaveLength(0)
    wrapper.unmount()
  })

  it('profile-list failure shows the fixed error and retry reloads', async () => {
    const t = new FakeMobileTransport()
    t.runtimeHostsError = { kind: 'storage', message: 'duplicate id fixture-admin' }
    const { wrapper, controller } = mountMobile(t)
    await flush()
    expect(wrapper.find('[data-testid="runtime-profiles-error"]').text()).toContain('[STORAGE]')
    expect(wrapper.text()).not.toContain('fixture-admin')
    expect(controller.runtimeProfiles.value).toHaveLength(0)

    t.runtimeHostsError = null
    seed(t).runtimeHostsData.push(...PROFILES)
    await controller.loadCatalog()
    await flush()
    expect(wrapper.text()).toContain('hana')
    wrapper.unmount()
  })

  it('explicitly attaching an Agent opens its listed ready Shell', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()

    await tapTarget(m, 'hana')
    expect(t.openCalls).toEqual([{ targetId: 'opq-a1', rows: 24, cols: 80 }])
    expect(m.controller.tabs).toHaveLength(1)
    const tab = m.controller.tabs[0]
    expect(tab.status).toBe('live')
    expect(tab.targetId).toBe('opq-a1')
    expect(m.wrapper.get('[data-testid="runtime-status-hana"]').classes()).toContain('on')
    m.wrapper.unmount()
  })

  it('explicitly attaching a Computer opens its listed Shell', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    expect(t.openCalls).toEqual([{ targetId: 'opq-c1', rows: 24, cols: 80 }])
    expect(m.controller.tabs[0].hostId).toBe('nuc')
    expect(m.controller.tabs[0].runtime).toBe(true)
    m.wrapper.unmount()
  })

  it.each([
    ['transport', 'network unreachable'],
    ['target', 'no Ready Shell'],
    ['capacity', 'session limit reached']
  ] as const)('open rejection kind %s shows an error on the tab', async (kind, msg) => {
    const t = runtime()
    t.openErrors.set('opq-a1', new ConnectionError(kind, msg))
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'hana')
    expect(m.controller.tabs[0].status).toBe('error')
    expect(m.controller.tabs[0].errorMsg).toContain(msg)
    expect(m.controller.tabs[0].errorMsg).not.toContain('[object Object]')
    m.wrapper.unmount()
  })

  it('an unknown structured kind surfaces its message', async () => {
    const t = runtime()
    // A future/new kind must not be flattened to [object Object].
    t.openErrors.set('opq-a1', { kind: 'maintenance', message: '目標維護中' })
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'hana')
    expect(m.controller.tabs[0].errorMsg).toBe('目標維護中')
    m.wrapper.unmount()
  })

  it('a plain-string rejection surfaces the message verbatim', async () => {
    const t = runtime()
    t.openErrors.set('opq-a1', new Error('socket closed by peer'))
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'hana')
    expect(m.controller.tabs[0].status).toBe('error')
    expect(m.controller.tabs[0].errorMsg).toContain('socket closed by peer')
    m.wrapper.unmount()
  })

  it('cross-profile tabs run independent sessions; background output drains hidden tabs', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()

    await tapTarget(m, 'hana')
    expect(m.controller.tabs).toHaveLength(1)

    // Back to the list and connect the computer — tabs keep living.
    await showMobileMain(m.wrapper)
    await flush()
    await tapTarget(m, 'nuc')
    await flush()

    expect(m.controller.tabs).toHaveLength(2)
    expect(t.openCalls.map((c) => c.targetId)).toEqual(['opq-a1', 'opq-c1'])

    // The hidden first tab still drains its own session bytes.
    t.feed('m1', { bytes: [65], exitCode: null })
    t.feed('m2', { bytes: [66], exitCode: null })
    await flush()
    expect(m.terms[0].text()).toBe('A')
    expect(m.terms[1].text()).toBe('B')
    m.wrapper.unmount()
  })

  it('App switching retains the same runtime session and accepts input on return', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    const tab = m.controller.tabs[0]

    setHidden(true)
    await flush()

    expect(t.closeCalls).toEqual([])
    expect(tab.status).toBe('live')
    expect(tab.sessionId).toBe('m1')
    expect(m.controller.tabs).toHaveLength(1)

    m.controller.sendInput(tab, 'hidden-input')
    expect(t.sessions.get('m1')!.writes).toHaveLength(0)
    setHidden(false)
    await flush()
    m.controller.sendInput(tab, 'continued\r')
    await flush()
    expect(tab.status).toBe('live')
    expect(tab.sessionId).toBe('m1')
    expect(t.openCalls).toHaveLength(1)
    expect(t.sessions.get('m1')!.writes.flat()).toEqual(Array.from(new TextEncoder().encode('continued\r')))
    m.wrapper.unmount()
  })

  it('backgrounding discards unsent input while retaining an already accepted write', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    const tab = m.controller.tabs[0]

    const gate = deferred<void>()
    t.writeGate = gate.promise
    m.controller.sendInput(tab, 'stuck')
    m.controller.sendInput(tab, 'must-not-replay\r')
    await flush()

    setHidden(true)
    await flush()
    setHidden(false)
    await flush()
    m.controller.reopenTab(tab, false)
    await flush()
    gate.resolve()
    t.writeGate = null
    await flush()

    expect(tab.status).toBe('live')
    expect(t.sessions.get('m1')!.writes.flat()).toEqual(
      Array.from(new TextEncoder().encode('stuck'))
    )
    expect(t.openCalls).toHaveLength(1)
    expect(t.closeCalls).toEqual([])
    m.wrapper.unmount()
  })

  it('App background pauses screenshots without releasing desktop or the session', async () => {
    vi.useFakeTimers()
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    const tab = m.controller.tabs[0]
    await chooseMobileScreen(m)
    expect(t.screenshotCalls).toEqual(['m1'])
    setHidden(true)
    await flush()
    await vi.advanceTimersByTimeAsync(60_000)
    expect(t.screenshotCalls).toEqual(['m1'])
    expect(tab.desktopEnabled).toBe(true)
    expect(t.closeCalls).toEqual([])
    setHidden(false)
    await flush()
    expect(t.screenshotCalls).toEqual(['m1', 'm1'])
    expect(tab.sessionId).toBe('m1')
    m.wrapper.unmount()
  })

  it('desktop enable + 30s preview only for the active tab; hide stops captures', async () => {
    vi.useFakeTimers()
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    const tab = m.controller.tabs[0]

    const desktopBtn = m.wrapper.find('[data-testid="desktop-toggle"]')
    expect(desktopBtn.attributes('aria-label')).toBe('選擇 Screen')
    expect(desktopBtn.attributes('aria-pressed')).toBe('false')
    await chooseMobileScreen(m)
    expect(t.desktopCalls).toEqual([{ sid: 'm1', enabled: true }])
    expect(m.wrapper.find('[data-testid="desktop-toggle"]').attributes('aria-pressed')).toBe('true')
    expect(tab.desktopEnabled).toBe(true)
    expect(t.screenshotCalls).toEqual(['m1'])
    expect(tab.shot?.src).toBe('data:image/png;base64,aGk=')

    await vi.advanceTimersByTimeAsync(30_000)
    await flush()
    expect(t.screenshotCalls).toHaveLength(2)

    // Hide via the panel corner toggle (the only preview control), then
    // show again — the panel header stays mounted while the desktop is
    // enabled so the toggle is always reachable.
    await m.wrapper.find('[data-testid="preview-visibility"]').trigger('click')
    await flush()
    expect(tab.previewVisible).toBe(false)
    await vi.advanceTimersByTimeAsync(60_000)
    await flush()
    expect(t.screenshotCalls).toHaveLength(2)

    await m.wrapper.find('[data-testid="preview-visibility"]').trigger('click')
    await flush()
    expect(tab.previewVisible).toBe(true)
    // Showing the image captures immediately, then the 30s cycle resumes.
    expect(t.screenshotCalls).toHaveLength(3)
    await vi.advanceTimersByTimeAsync(30_000)
    await flush()
    expect(t.screenshotCalls).toHaveLength(4)
    m.wrapper.unmount()
  })

  it('keeps enlarged desktop drags in the image, then restores workspace swipes after close', async () => {
    const m = mountMobile(runtime())
    try {
      await flush(); await tapTarget(m, 'nuc'); await chooseMobileScreen(m)
      await m.wrapper.get('[data-testid="preview-enlarge"]').trigger('click')
      await flush()
      expect(m.wrapper.find('[data-testid="mobile-preview-zoom"]').exists()).toBe(true)
      const viewport = m.wrapper.get('[data-testid="zoom-viewport"]').element as HTMLElement
      viewport.setPointerCapture = vi.fn()
      const image = viewport.querySelector('img')!
      Object.defineProperties(image, { naturalWidth: { value: 1600 }, naturalHeight: { value: 900 } })
      image.dispatchEvent(new Event('load'))
      const pointer = (target: Element, type: string, id: number, x: number, primary = true) => {
        const event = new Event(type, { bubbles: true, cancelable: true })
        Object.assign(event, { pointerId: id, clientX: x, clientY: 200, button: 0, pointerType: 'touch', isPrimary: primary })
        target.dispatchEvent(event)
      }
      pointer(viewport, 'pointerdown', 1, 100)
      pointer(viewport, 'pointerdown', 2, 200, false)
      pointer(viewport, 'pointermove', 2, 280, false)
      await nextTick()
      expect(image.getAttribute('style')).toContain('scale(1.8)')
      pointer(viewport, 'pointerup', 2, 280, false)
      pointer(viewport, 'pointermove', 1, 300)
      pointer(viewport, 'pointerup', 1, 300)
      await flush()
      expect(m.wrapper.find('[data-testid="mobile-preview-zoom"]').exists()).toBe(true)
      expect(m.wrapper.get('[data-testid="terminal-to-list"]').attributes('aria-current')).toBeUndefined()
      await m.wrapper.get('[aria-label="關閉放大檢視"]').trigger('click')
      await flush()
      expect(m.wrapper.find('[data-testid="mobile-preview-zoom"]').exists()).toBe(false)
      vi.useFakeTimers()
      const root = m.wrapper.get('.m-app').element
      pointer(root, 'pointerdown', 3, 100)
      pointer(root, 'pointermove', 3, 300)
      pointer(root, 'pointerup', 3, 300)
      await vi.advanceTimersByTimeAsync(200)
      await flush()
      expect(m.wrapper.get('[data-testid="terminal-to-list"]').attributes('aria-current')).toBe('page')
    } finally { m.wrapper.unmount(); vi.useRealTimers() }
  })

  it('a stale screenshot never lands on a backgrounded tab (generation guard)', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc') // tab A, desktop-capable
    const tabA = m.controller.tabs[0]

    await chooseMobileScreen(m)
    expect(tabA.shot).not.toBeNull()
    tabA.shot = null

    // Open an agent — the active tab switches.
    await showMobileMain(m.wrapper)
    await flush()
    await tapTarget(m, 'gamma')
    await flush()
    const tabB = m.controller.tabs[1]

    // Switch to A → immediate capture held in flight; switch away before it lands.
    const gate = deferred<void>()
    t.screenshotGate = gate.promise
    m.controller.setActive(tabA.key)
    await flush()
    expect(t.screenshotCalls.length).toBeGreaterThanOrEqual(2)
    m.controller.setActive(tabB.key)
    gate.resolve()
    t.screenshotGate = null
    await flush()
    expect(tabA.shot).toBeNull()
    m.wrapper.unmount()
  })

  it('suppresses only a hidden soft keyboard before quick-key touches', async () => {
    let visibility!: (visible: boolean) => void
    const m = mountMobile(runtime(), async receive => { visibility = receive; return async () => {} })
    try {
      await flush(); await tapTarget(m, 'nuc')
      const suppress = vi.fn()
      Object.assign(m.terms[0], { suppressSoftKeyboard: suppress })
      visibility(false)
      await m.wrapper.get('[data-testid=k-left]').trigger('pointerdown')
      await m.wrapper.get('[data-testid=k-left]').trigger('click')
      expect(suppress).toHaveBeenCalledOnce()
      visibility(true)
      await m.wrapper.get('[data-testid=k-right]').trigger('pointerdown')
      await m.wrapper.get('[data-testid=k-right]').trigger('click')
      expect(suppress).toHaveBeenCalledOnce()
      expect(m.transport.sessions.get('m1')!.writes.flat()).toEqual([0x1b,0x5b,0x44,0x1b,0x5b,0x43])
    } finally { m.wrapper.unmount() }
  })

  it('sends quick keys without requesting terminal keyboard focus', async () => {
    const m = mountMobile(runtime())
    try {
      await flush(); await tapTarget(m, 'nuc')
      const focus = vi.spyOn(m.terms[0], 'focus')
      for (const id of ['esc', 'tab', 'left', 'up', 'down', 'right', 'shiftleft', 'ctrlc', 'enter']) {
        await m.wrapper.get(`[data-testid="k-${id}"]`).trigger('click')
      }
      await flush()
      expect(focus).not.toHaveBeenCalled()
      expect(m.transport.sessions.get('m1')!.writes.flat()).toContain(0x1b)
      await m.wrapper.get('[data-testid="k-keyboard"]').trigger('click')
      expect(focus).toHaveBeenCalledOnce()
    } finally { m.wrapper.unmount() }
  })

  it('keybar sends exact bytes and controller detach keeps the tab', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    const flat = () => t.sessions.get('m1')!.writes.flat()

    expect(m.wrapper.findAll('.keybar button').map(button => button.attributes('data-testid'))).toEqual([
      'k-tab', 'k-esc', 'k-shiftleft', 'k-up', 'k-down', 'k-left', 'k-right',
      'k-enter', 'k-ctrlc', 'k-keyboard'
    ])
    expect(m.wrapper.find('[data-testid="k-shiftleft"]').text()).toBe('⇧←')

    for (const id of ['esc', 'tab', 'up', 'down', 'left', 'right', 'shiftleft', 'ctrlc'] as const) {
      await m.wrapper.find(`[data-testid="k-${id}"]`).trigger('click')
    }
    await flush()
    expect(flat()).toEqual([
      0x1b, 0x09,
      0x1b, 0x5b, 0x41,
      0x1b, 0x5b, 0x42,
      0x1b, 0x5b, 0x44,
      0x1b, 0x5b, 0x43,
      0x1b, 0x5b, 0x31, 0x3b, 0x32, 0x44,
      0x03
    ])

    // Detach is a local detach — no bytes are sent, remote session survives,
    // and the tab keeps its target for a one-tap reconnect.
    await m.controller.detachTab(m.controller.tabs[0])
    await flush()
    expect(m.controller.tabs).toHaveLength(1)
    expect(m.controller.tabs[0].status).toBe('exited')
    expect(t.closeCalls).toContain('m1')
    m.wrapper.unmount()
  })

  it('hardware Ctrl continues through the terminal input adapter', async () => {
    const m = mountMobile(runtime())
    try {
      await flush(); await tapTarget(m, 'nuc')
      expect(m.terms[0].keyInterceptor!(new KeyboardEvent('keydown', { key: 'c', ctrlKey: true }))).toBe(true)
      m.terms[0].emitData('\x03'); await flush()
      expect(m.transport.sessions.get('m1')!.writes.flat()).toEqual([0x03])
    } finally { m.wrapper.unmount() }
  })

  it('IME composition commits once even when fragments repeat', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    const host = m.wrapper.find('.term-host').element

    host.dispatchEvent(compEvent('compositionstart'))
    m.terms[0].emitData('中') // stray intermediate fragment
    m.terms[0].emitData('中') // xterm's own commit emission
    host.dispatchEvent(compEvent('compositionend', '中'))
    await flush()

    const written = t.sessions.get('m1')!.writes.flat()
    expect(written).toEqual(Array.from(new TextEncoder().encode('中')))
    m.wrapper.unmount()
  })

  it('compositionend without data falls back to the held fragment once', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')
    const host = m.wrapper.find('.term-host').element

    host.dispatchEvent(compEvent('compositionstart'))
    m.terms[0].emitData('日')
    host.dispatchEvent(compEvent('compositionend', ''))
    await flush()
    expect(t.sessions.get('m1')!.writes.flat()).toEqual(
      Array.from(new TextEncoder().encode('日'))
    )
    m.wrapper.unmount()
  })

  it('terminal paste sends multiline UTF-8 bytes without injecting Enter', async () => {
    const m = mountMobile(runtime())
    try {
      await flush(); await tapTarget(m, 'nuc')
      m.terms[0].emitData('echo 一\necho 二'); await flush()
      expect(m.transport.sessions.get('m1')!.writes.flat()).toEqual([...new TextEncoder().encode('echo 一\necho 二')])
      expect(m.controller.tabs[0].status).toBe('live')
    } finally { m.wrapper.unmount() }
  })

  it('no-tabs fallback returns to the list and a tap opens a new tab', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')

    // Detach leaves one exited tab; closing it triggers the no-tabs cover.
    await m.controller.detachTab(m.controller.tabs[0])
    await flush()
    const tab = m.controller.tabs[0]
    await m.controller.closeTab(tab)
    await flush()
    expect(m.controller.tabs).toHaveLength(0)

    await m.wrapper.find('[data-testid="no-tabs-targets"]').trigger('click')
    await flush()
    const list = m.wrapper.find('[data-testid="catalog-card"]')
    expect(list.exists()).toBe(true)
    expect(list.text()).toContain('hana')

    await attachMobileProfile(m.wrapper, 'hana')
    await flush()
    expect(t.openCalls.at(-1)?.targetId).toBe('opq-a1')
    expect(m.controller.tabs).toHaveLength(1)
    m.wrapper.unmount()
  })

  it('unmount detaches every live session locally and disposes terminals', async () => {
    const t = runtime()
    const m = mountMobile(t)
    await flush()
    await tapTarget(m, 'nuc')

    m.wrapper.unmount()
    await flush()
    expect(t.closeCalls).toEqual(['m1'])
    expect(m.terms[0].disposed).toBe(true)
    t.feed('m1', { bytes: [65], exitCode: null })
    await flush()
    expect(m.terms[0].chunks).toHaveLength(0)
  })
})


describe('mobile finished Shell cleanup', () => {
  it.each(['exit-code', 'runtime-exited', 'runtime-missing', 'runtime-operatorKilled', 'recovery-exited', 'recovery-missing', 'ack-exited'] as const)('returns Connect to target selection after %s', async (ending) => {
    const m = mountMobile(runtime())
    try {
      await flush(); await tapTarget(m, 'nuc')
      const tab = m.controller.activeTab.value!
      const session = tab.sessionId!
      expect(m.wrapper.find('[data-testid="no-tabs"]').exists()).toBe(false)
      if (ending === 'exit-code') m.transport.feed(session, { bytes: [], exitCode: 0 })
      else {
        const recovering = ending.startsWith('recovery-')
        if (recovering) m.transport.recoverError = new ConnectionError(ending.slice('recovery-'.length) as 'exited' | 'missing', 'Shell 已結束')
        if (ending === 'ack-exited') m.transport.runtimeAck = async () => { throw new ConnectionError('exited', 'Shell 已結束') }
        m.transport.feed(session, { bytes: [], exitCode: null, runtime: { batchId: ending === 'ack-exited' ? 1 : 0, localGeneration: 1, events: [], ended: recovering ? 'transport' : ending.slice('runtime-'.length) } })
      }
      await flush()
      expect(m.controller.tabs).toHaveLength(0)
      expect(m.wrapper.find('[data-testid="no-tabs"]').exists()).toBe(true)
      expect(m.wrapper.find('.term-host').exists()).toBe(false)
      expect(m.wrapper.get('#terminal-panel').isVisible()).toBe(true)
      expect(m.terms[0].disposed).toBe(true)
    } finally { m.wrapper.unmount() }
  })
})
