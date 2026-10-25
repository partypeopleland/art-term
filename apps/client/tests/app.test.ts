import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import App from '../src/desktop/App.vue'
import type { AppController } from '../src/shared/state/controller'
import { ConnectionError } from '../src/shared/transport/types'
import { FakeTerm, FakeTransport, deferred, makeHost, makeProfile, makeTarget } from './fake'

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
  transport: FakeTransport
  terms: FakeTerm[]
}

function mountApp(transport: FakeTransport): Mounted {
  const terms: FakeTerm[] = []
  const wrapper = mount(App, {
    props: {
      transport,
      terminalFactory: () => {
        const t = new FakeTerm()
        terms.push(t)
        return t
      },
      controllerOptions: { idlePollMs: 0, autoCaptureMs: 30_000 }
    }
  })
  const controller = (wrapper.vm as unknown as { controller: AppController }).controller
  return { wrapper, controller, transport, terms }
}

// Runtime-only contract: the list is saved runtime Host profiles split by
// backend metadata (agents/computers); openable targets come only from
// per-profile runtimeSessions replies — opaque backend-issued ids.
const PROFILES = [
  makeProfile('hana', 'hana', { category: 'agent', agentName: 'hana', agentTool: 'Codex' }),
  makeProfile('beta', 'beta', { category: 'agent', agentName: 'beta', agentTool: 'Devin' }),
  makeProfile('gamma', 'gamma', { category: 'agent', agentName: 'gamma', agentTool: 'Kiro' }),
  makeProfile('nuc', 'nuc')
]

const AGENTS = makeHost('agents', [
  makeTarget('opq-agent-hana', 'agents', 'host', true, false, 'Codex'),
  makeTarget('opq-agent-beta', 'agents', 'host', true, false, 'Devin'),
  makeTarget('opq-agent-gamma', 'agents', 'host', true, false, 'Kiro')
])
const COMPUTERS = makeHost('computers', [
  makeTarget('opq-computer-nuc', 'computers', 'host', true, true, 'openab-pty')
])
// Targets carry display labels distinct from their opaque ids.
for (const [i, label] of ['hana', 'beta', 'gamma'].entries()) {
  AGENTS.targets[i].label = label
}
COMPUTERS.targets[0].label = 'nuc'
const agentsGroup = { dto: AGENTS, expanded: true }
const computersGroup = { dto: COMPUTERS, expanded: true }

function seedRuntime(transport: FakeTransport): void {
  transport.runtimeSessionsData.set('hana', makeHost('hana', [AGENTS.targets[0]]))
  transport.runtimeSessionsData.set('beta', makeHost('beta', [AGENTS.targets[1]]))
  transport.runtimeSessionsData.set('gamma', makeHost('gamma', [AGENTS.targets[2]]))
  transport.runtimeSessionsData.set('nuc', makeHost('nuc', [COMPUTERS.targets[0]]))
}

function key(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent('keydown', init)
}

describe('shared App with runtime profiles (no Tauri)', () => {
  it('shows a configured desktop CLI name and writes quoted words only on click', async () => {
    const transport = new FakeTransport(PROFILES)
    const target = { ...AGENTS.targets[0], codingCli: { id: 'dev-helper', name: 'Dev Helper', command: 'dev-helper', args: ['--label', "owner's workspace", '$(literal)', ''] } }
    const m = mountApp(transport)
    try {
      await flush()
      m.controller.openTarget({ dto: makeHost('hana', [target]), expanded: true }, target, false)
      await flush()
      const tab = m.controller.tabs[0], sid = tab.sessionId!
      expect(transport.sessions.get(sid)!.writes).toEqual([])
      const button = m.wrapper.find('[data-testid="launch-coding-cli"]')
      expect(button.text()).toBe('啟動 Dev Helper')
      await button.trigger('click'); await flush()
      expect(transport.sessions.get(sid)!.writes.map(b => new TextDecoder().decode(Uint8Array.from(b)))).toEqual(["'dev-helper' '--label' 'owner'\\''s workspace' '$(literal)' ''\r"])
      await m.controller.setHidden(true)
      m.controller.launchCodingCli(tab); await flush()
      expect(transport.sessions.get(sid)!.writes).toHaveLength(1)
    } finally { m.wrapper.unmount() }
  })
  it('sends the inline Codex command once from the header button', async () => {
    const transport = new FakeTransport()
    const target = { ...AGENTS.targets[0], codingCli: { id: 'codex', name: 'CODEX', command: 'codex', args: ['--yolo'] } }
    const m = mountApp(transport)
    try {
      await flush()
      m.controller.openTarget({ dto: makeHost('hana', [target]), expanded: true }, target, false)
      await flush()
      const sid = m.controller.tabs[0].sessionId!
      expect(transport.sessions.get(sid)!.writes).toEqual([])
      await m.wrapper.find('[data-testid="launch-coding-cli"]').trigger('click'); await flush()
      expect(transport.sessions.get(sid)!.writes.map(b => new TextDecoder().decode(Uint8Array.from(b))))
        .toEqual(['codex --yolo --no-alt-screen\r'])
    } finally { m.wrapper.unmount() }
  })
  beforeEach(() => {
    document.body.innerHTML = ''
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('loads the runtime profiles grouped by backend metadata; no auto-open', async () => {
    const t = new FakeTransport(PROFILES)
    seedRuntime(t)
    const { wrapper, controller, transport } = mountApp(t)
    await flush()

    expect(controller.runtimeProfiles.value.map((p) => p.id)).toEqual([
      'hana', 'beta', 'gamma', 'nuc'
    ])

    const text = wrapper.text()
    for (const name of ['hana', 'beta', 'gamma', 'nuc']) expect(text).toContain(name)
    for (const sub of ['Codex', 'Devin', 'Kiro']) expect(text).toContain(sub)
    // Stable runtime selector: the clickable row carries the display name.
    for (const name of ['hana', 'beta', 'gamma', 'nuc'])
      expect(wrapper.find(`[data-runtime-label="${name}"]`).exists()).toBe(true)

    // No autoconnect: nothing opened on its own.
    expect(transport.openCalls).toHaveLength(0)
    wrapper.unmount()
  })

  it('a profile-store error maps to the fixed safe text with no fallback catalog', async () => {
    const t = new FakeTransport([])
    t.runtimeHostsError = { kind: 'storage', message: 'do-not-render-this' }
    const { wrapper, controller } = mountApp(t)
    await flush()
    expect(controller.runtimeProfilesError.value).toContain('[STORAGE]')
    expect(controller.runtimeProfilesError.value).not.toContain('do-not-render-this')
    expect(controller.runtimeProfiles.value).toHaveLength(0)
    expect(wrapper.text()).not.toContain('do-not-render-this')
    wrapper.unmount()
  })

  it('an inline attach explicitly connects the single ready Shell', async () => {
    const t = new FakeTransport(PROFILES)
    seedRuntime(t)
    const { wrapper, controller, transport } = mountApp(t)
    await flush()

    await wrapper.find('[data-testid="runtime-nuc"] .attach-button').trigger('click')
    await flush()

    expect(transport.openCalls.map((c) => c.targetId)).toEqual(['opq-computer-nuc'])
    expect(controller.tabs).toHaveLength(1)
    expect(controller.tabs[0].hostId).toBe('nuc')
    expect(controller.tabs[0].status).toBe('live')
    wrapper.unmount()
  })

  it('a profile list refresh never rebinds an existing tab', async () => {
    const transport = new FakeTransport(PROFILES)
    seedRuntime(transport)
    const { wrapper, controller } = mountApp(transport)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()
    const original = controller.tabs[0]
    const originalSession = original.sessionId
    // A profile refresh (id/revision churn) must not touch the live tab.
    transport.runtimeHostsData = PROFILES.map((p) => ({ ...p, revision: p.revision + 1 }))
    await controller.loadCatalog()
    await flush()
    expect(original.targetId).toBe('opq-agent-hana')
    expect(original.sessionId).toBe(originalSession)
    expect(transport.closeCalls).toEqual([])
    wrapper.unmount()
  })

  it('structured open errors surface their kind-mapped message, no prompt', async () => {
    const t = new FakeTransport(PROFILES)
    t.openError = new ConnectionError('stateChanged', '')
    const { controller } = mountApp(t)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()
    expect(controller.tabs[0].status).toBe('error')
    expect(controller.tabs[0].errorMsg).toContain('已變更')
  })

  it('opens a shell tab, streams raw bytes, and backpressures poll on xterm.write', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport, terms } = mountApp(t)
    await flush()

    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()

    expect(transport.openCalls).toEqual([{ targetId: 'opq-agent-hana', rows: 24, cols: 80 }])
    const sid = transport.sessions.keys().next().value!
    const tab = controller.tabs[0]
    expect(tab.status).toBe('live')

    const term = terms[0]
    term.holdWrites = true
    transport.feed(sid, { bytes: [72, 105], exitCode: null })
    await flush()
    // Bytes were handed to xterm, but the write callback is still pending —
    // the next poll must NOT have been issued yet.
    expect(term.chunks).toHaveLength(1)
    expect(term.text()).toBe('Hi')
    expect(transport.sessions.get(sid)!.pollCount).toBe(1)

    term.releaseWrites()
    await flush()
    expect(transport.sessions.get(sid)!.pollCount).toBe(2)
  })

  it('drains background tabs while hidden', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport, terms } = mountApp(t)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[1], false)
    await flush()

    expect(controller.tabs).toHaveLength(2)
    expect(controller.activeKey.value).toBe('agents::opq-agent-beta')
    const s1 = transport.sessions.get('s1')!
    transport.feed('s1', { bytes: [98, 103], exitCode: null })
    await flush()
    // Tab 1 is hidden but its xterm instance still received the bytes.
    expect(terms[0].text()).toBe('bg')
  })

  it('serializes UTF-8 input and sends raw bytes', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport, terms } = mountApp(t)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()

    terms[0].emitData('中文 echo ✓\n')
    await flush()
    const written = transport.sessions.get('s1')!.writes.flat()
    const expected = Array.from(new TextEncoder().encode('中文 echo ✓\n'))
    expect(written).toEqual(expected)
  })

  it('intercepts Alt+[/Alt+] and Ctrl+PgUp/PgDn globally and via xterm handler; other keys pass', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport, terms } = mountApp(t)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[1], false)
    await flush()
    expect(controller.activeKey.value).toBe('agents::opq-agent-beta')

    window.dispatchEvent(key({ code: 'BracketLeft', altKey: true }))
    expect(controller.activeKey.value).toBe('agents::opq-agent-hana')
    window.dispatchEvent(key({ code: 'BracketRight', altKey: true }))
    expect(controller.activeKey.value).toBe('agents::opq-agent-beta')
    window.dispatchEvent(key({ code: 'PageUp', ctrlKey: true }))
    expect(controller.activeKey.value).toBe('agents::opq-agent-hana')
    window.dispatchEvent(key({ code: 'PageDown', ctrlKey: true }))
    expect(controller.activeKey.value).toBe('agents::opq-agent-beta')

    // The xterm custom key handler must swallow switch keys before they
    // become bytes, and let everything else through to the terminal.
    const interceptor = terms[1].keyInterceptor!
    expect(interceptor(key({ code: 'BracketLeft', altKey: true }))).toBe(false)
    expect(interceptor(key({ code: 'KeyA' }))).toBe(true)
    expect(interceptor(key({ code: 'KeyC', ctrlKey: true }))).toBe(true)

    // Switch keys produced no remote input.
    for (const s of transport.sessions.values()) expect(s.writes).toHaveLength(0)
  })

  it('reuses a live tab, removes a finished Shell, and opens a fresh tab on explicit reconnect', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport, terms } = mountApp(t)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false) // alive → reuse
    await flush()
    expect(controller.tabs).toHaveLength(1)
    expect(transport.openCalls).toHaveLength(1)

    transport.feed('s1', { bytes: [], exitCode: 0 })
    await flush()
    expect(controller.tabs).toHaveLength(0)
    expect(terms[0].disposed).toBe(true)

    controller.openTarget(agentsGroup, AGENTS.targets[0], false) // exited → reconnect same target
    await flush()
    expect(controller.tabs).toHaveLength(1)
    expect(transport.openCalls).toHaveLength(2)
    expect(transport.openCalls[1].targetId).toBe('opq-agent-hana')
    expect(controller.tabs[0].status).toBe('live')
    expect(controller.tabs[0].sessionId).toBe('s2')
    expect(terms[0].resetCount).toBe(0)
    expect(terms).toHaveLength(2)
  })

  it('enforces the 32-tab cap and closes tabs locally only', async () => {
    const many = makeHost(
      'agents',
      Array.from({ length: 34 }, (_, i) => makeTarget(`t${i}`, 'agents'))
    )
    const t = new FakeTransport(PROFILES)
    const { controller, transport } = mountApp(t)
    await flush()
    const group = { dto: many, expanded: true }
    for (const target of many.targets) controller.openTarget(group, target, false)
    await flush()
    expect(controller.tabs).toHaveLength(32)
    expect(controller.uiNotice.value).toContain('32')

    const tab = controller.tabs[0]
    const sid = tab.sessionId!
    await controller.closeTab(tab)
    expect(transport.closeCalls).toEqual([sid])
    expect(controller.tabs).toHaveLength(31)
    // EOF reclaims the backend handle once; removing the ended tab adds no close.
    const exited = controller.tabs.find((x) => x.status === 'live')!
    const sid2 = exited.sessionId!
    transport.feed(sid2, { bytes: [], exitCode: 0 })
    await flush()
    await controller.closeTab(exited)
    expect(transport.closeCalls.filter((id) => id === sid2)).toHaveLength(1)
  })

  it('enables desktop only after backend success and serializes toggles', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport } = mountApp(t)
    await flush()
    controller.openTarget(computersGroup, COMPUTERS.targets[0], true)
    await flush()
    const tab = controller.tabs[0]
    expect(tab.status).toBe('live')
    expect(transport.desktopCalls).toEqual([{ sid: 's1', enabled: true }])
    expect(tab.desktopEnabled).toBe(true)

    // Serialize toggles: a second request while busy is ignored.
    const gate = deferred<void>()
    transport.desktopGate = gate.promise
    void controller.setDesktop(tab, false)
    void controller.setDesktop(tab, true)
    await flush()
    expect(transport.desktopCalls).toHaveLength(2)
    gate.resolve()
    transport.desktopGate = null
    await flush()
    expect(tab.desktopEnabled).toBe(false)
  })

  it('captures immediately, then every 30s, only for the active tab; hide stops new captures', async () => {
    vi.useFakeTimers()
    const t = new FakeTransport(PROFILES)
    const { controller, transport } = mountApp(t)
    await flush()
    controller.openTarget(computersGroup, COMPUTERS.targets[0], true)
    await flush()
    const tab = controller.tabs[0]
    expect(tab.desktopEnabled).toBe(true)
    expect(transport.screenshotCalls).toEqual(['s1'])
    expect(tab.shot?.src).toBe('data:image/png;base64,aGk=')
    expect(tab.shot?.source).toBe('art-term-hands')

    await vi.advanceTimersByTimeAsync(30_000)
    await flush()
    expect(transport.screenshotCalls).toHaveLength(2)

    controller.setPreviewVisible(tab, false)
    await vi.advanceTimersByTimeAsync(60_000)
    await flush()
    expect(transport.screenshotCalls).toHaveLength(2)

    controller.setPreviewVisible(tab, true)
    await flush()
    expect(transport.screenshotCalls).toHaveLength(3)
  })

  it('guards stale screenshots across tabs (switch invalidates the in-flight capture)', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport } = mountApp(t)
    await flush()
    controller.openTarget(computersGroup, COMPUTERS.targets[0], true) // tab A + desktop
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false) // tab B, now active
    await flush()

    const tabA = controller.tabs.find((x) => x.targetId === 'opq-computer-nuc')!
    expect(tabA.shot).not.toBeNull()
    tabA.shot = null

    // Switching to A triggers an immediate capture; hold it in-flight, then
    // switch away before the response lands.
    const gate = deferred<void>()
    transport.screenshotGate = gate.promise
    controller.setActive(tabA.key)
    await flush()
    expect(transport.screenshotCalls.length).toBeGreaterThanOrEqual(2)

    controller.setActive('agents::opq-agent-hana')
    gate.resolve()
    transport.screenshotGate = null
    await flush()
    // The stale response was discarded; A was no longer active.
    expect(tabA.shot).toBeNull()
  })

  it('reports screenshot errors without blocking the shell', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport, terms } = mountApp(t)
    await flush()
    controller.openTarget(computersGroup, COMPUTERS.targets[0], true)
    await flush()
    const tab = controller.tabs[0]

    // Force the next screenshot to fail.
    const orig = transport.screenshot.bind(transport)
    transport.screenshot = async () => {
      throw new Error('provider timeout')
    }
    controller.manualCapture()
    await flush()
    expect(tab.shotError).toContain('provider timeout')
    expect(tab.status).toBe('live')
    transport.screenshot = orig

    // Terminal input still flows.
    terms[0].emitData('x')
    await flush()
    expect(transport.sessions.get('s1')!.writes.flat()).toEqual([120])
  })

  it('sends latest-wins serialized resizes for the live session', async () => {
    const t = new FakeTransport(PROFILES)
    const { controller, transport } = mountApp(t)
    await flush()
    controller.openTarget(agentsGroup, AGENTS.targets[0], false)
    await flush()
    const tab = controller.tabs[0]

    controller.requestResize(tab, 30, 100)
    controller.requestResize(tab, 31, 101)
    controller.requestResize(tab, 32, 102)
    await flush()
    const resizes = transport.sessions.get('s1')!.resizes
    expect(resizes.at(-1)).toEqual({ rows: 32, cols: 102 })
    expect(resizes.length).toBeLessThanOrEqual(3)
  })
})
