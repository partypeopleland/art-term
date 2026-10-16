import { showMobileMain } from './mobile-actions'
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import App from '../src/App.vue'
import MobileApp from '../src/mobile/MobileApp.vue'
import type { AppController } from '../src/state/controller'
import type { HostDto, RuntimeHostDto, TargetDto } from '../src/transport/types'
import { FakeTerm, FakeTransport, deferred, makeTarget } from './fake'
import { FakeMobileTransport } from './mobile.fake'

// jsdom lacks ResizeObserver; TerminalPane observes its element.
vi.stubGlobal(
  'ResizeObserver',
  class {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
)

async function flush(times = 50): Promise<void> {
  for (let i = 0; i < times; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

const profile: RuntimeHostDto = {
  id: 'runtime-profile-1',
  revision: 7,
  name: 'nuc',
  baseUrl: 'https://runtime.example.test',
  credentialPresent: true
}
const profileB: RuntimeHostDto = {
  id: 'runtime-profile-2',
  revision: 3,
  name: 'hana',
  baseUrl: 'https://runtime-b.example.test',
  credentialPresent: true
}

function runtimeTarget(id: string, hostId: string, phase: string, running = true, label = id): TargetDto {
  return { ...makeTarget(id, hostId, 'host', running), phase, label }
}

function sessionGroup(p: RuntimeHostDto, targets: TargetDto[]): HostDto {
  return { id: p.id, label: p.name, desktopAvailable: false, targets }
}

const ready = runtimeTarget('runtime-target-1', profile.id, 'ready', true, 'shell-one')
const readyGroup = sessionGroup(profile, [ready])

function runtimeTransport(store: RuntimeHostDto[], listed: HostDto = readyGroup) {
  return Object.assign(new FakeTransport(), {
    runtimeHosts: vi.fn(async () => store),
    runtimeSessions: vi.fn(async (_id: string, _revision: number) => listed),
    runtimeCreateSession: vi.fn(async () => ready)
  })
}

function mobileRuntimeTransport(store: RuntimeHostDto[], listed: HostDto = readyGroup) {
  return Object.assign(new FakeMobileTransport(), {
    runtimeHosts: vi.fn(async () => store),
    runtimeSessions: vi.fn(async (_id: string, _revision: number) => listed),
    runtimeCreateSession: vi.fn(async () => ready)
  })
}

const wrappers: ReturnType<typeof mount>[] = []
function mountApp<T extends FakeTransport>(transport: T) {
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
  wrappers.push(wrapper)
  const controller = (wrapper.vm as unknown as { controller: AppController }).controller
  return { wrapper, controller, transport, terms }
}

function mountMobile<T extends FakeMobileTransport>(transport: T) {
  const terms: FakeTerm[] = []
  const wrapper = mount(MobileApp, {
    props: {
      transport,
      terminalFactory: () => {
        const t = new FakeTerm()
        terms.push(t)
        return t
      },
      controllerOptions: { idlePollMs: 0 }
    },
    attachTo: document.body
  })
  wrappers.push(wrapper)
  const controller = (wrapper.vm as unknown as { controller: AppController }).controller
  return { wrapper, controller, transport, terms }
}

function setVisibility(hidden: boolean): void {
  Object.defineProperty(document, 'hidden', { configurable: true, value: hidden })
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: hidden ? 'hidden' : 'visible' })
  document.dispatchEvent(new Event('visibilitychange'))
}

function statusDot(wrapper: ReturnType<typeof mount>, id = profile.id) {
  return wrapper.get(`[data-testid="runtime-status-${id}"]`)
}

beforeEach(() => {
  document.body.innerHTML = ''
  setVisibility(false)
})
afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  setVisibility(false)
  vi.restoreAllMocks()
})

describe('runtime entry quick connect', () => {
  it('exactly one running ready Shell connects directly — actual ids, no dialog, no create', async () => {
    const transport = runtimeTransport([profile])
    const { wrapper, controller, transport: t } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()

    expect(t.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, profile.revision)
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
    expect(t.runtimeCreateSession).not.toHaveBeenCalled()
    expect(t.openCalls).toEqual([{ targetId: 'runtime-target-1', rows: 24, cols: 80 }])
    expect(controller.tabs).toHaveLength(1)
    expect(controller.tabs[0].hostId).toBe(profile.id)
    expect(controller.tabs[0].targetId).toBe('runtime-target-1')
    expect(controller.tabs[0].runtime).toBe(true)
    expect(controller.tabs[0].status).toBe('live')
  })

  it('zero Shells opens the chooser seeded without a second list call', async () => {
    const transport = runtimeTransport([profile], sessionGroup(profile, []))
    const { wrapper, transport: t } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()

    const dialog = wrapper.get('[data-testid="runtime-connection-dialog"]')
    expect(dialog.text()).toContain('nuc 的 Shell')
    expect(t.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, profile.revision)
    expect(t.openCalls).toEqual([])
    expect(t.runtimeCreateSession).not.toHaveBeenCalled()
  })

  it('multiple Shells open the chooser; explicit connect uses the backend target unchanged', async () => {
    const second = runtimeTarget('runtime-target-2', profile.id, 'ready', true, 'shell-two')
    const transport = runtimeTransport([profile], sessionGroup(profile, [ready, second]))
    const { wrapper, transport: t } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()

    const dialog = wrapper.get('[data-testid="runtime-connection-dialog"]')
    expect(dialog.text()).toContain('shell-one')
    expect(dialog.text()).toContain('shell-two')
    expect(t.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, profile.revision)
    expect(t.openCalls).toEqual([])

    const connectButtons = dialog.findAll('button').filter((b) => b.text() === '連線')
    expect(connectButtons).toHaveLength(2)
    await connectButtons[1].trigger('click')
    await flush()
    expect(t.openCalls).toEqual([{ targetId: 'runtime-target-2', rows: 24, cols: 80 }])
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
  })

  it('a single attached Shell keeps the explicit takeover chooser', async () => {
    const attached = runtimeTarget('runtime-target-att', profile.id, 'attached', true, 'shell-busy')
    const transport = runtimeTransport([profile], sessionGroup(profile, [attached]))
    const { wrapper, transport: t } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()

    const dialog = wrapper.get('[data-testid="runtime-connection-dialog"]')
    expect(dialog.text()).toContain('接管連線')
    expect(t.openCalls).toEqual([])
    expect(t.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, profile.revision)
  })

  it('non-running or unknown phases always open the chooser', async () => {
    for (const listed of [
      sessionGroup(profile, [runtimeTarget('rt-stopped', profile.id, 'ready', false)]),
      sessionGroup(profile, [runtimeTarget('rt-mystery', profile.id, 'detached')])
    ]) {
      const transport = runtimeTransport([profile], listed)
      const { wrapper, transport: t } = mountApp(transport)
      await flush()
      await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
      await flush()
      expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(true)
      expect(t.openCalls).toEqual([])
      wrapper.unmount()
      wrappers.splice(wrappers.indexOf(wrapper), 1)
    }
  })

  it('an existing live runtime tab for the same host id is reused with no remote query', async () => {
    const readyB = runtimeTarget('runtime-target-b', profileB.id, 'ready', true, 'shell-b')
    const transport = runtimeTransport([profile, profileB])
    transport.runtimeSessions.mockImplementation((id: string) =>
      Promise.resolve(id === profile.id ? readyGroup : sessionGroup(profileB, [readyB]))
    )
    const { wrapper, controller, transport: t } = mountApp(transport)
    await flush()

    // Quick-connect both profiles; hana's tab is now the active one.
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()
    await wrapper.get('[data-runtime-label="hana"]').trigger('click')
    await flush()
    expect(controller.tabs).toHaveLength(2)
    expect(controller.activeKey.value).toBe(controller.tabs[1].key)

    // Re-clicking nuc reactivates its live tab by exact host id — no new
    // list request, no dialog, no extra open.
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()

    expect(t.runtimeSessions).toHaveBeenCalledTimes(2)
    expect(t.openCalls).toHaveLength(2)
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
    expect(controller.activeKey.value).toBe(controller.tabs[0].key)
  })

  it('a stale profile revision or deletion retires an in-flight quick result', async () => {
    for (const mutate of ['revision', 'delete'] as const) {
      const transport = runtimeTransport([profile])
      const pending = deferred<HostDto>()
      transport.runtimeSessions.mockReturnValueOnce(pending.promise)
      const { wrapper, controller, transport: t } = mountApp(transport)
      await flush()

      await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
      await flush()
      controller.runtimeProfiles.value = mutate === 'delete' ? [] : [{ ...profile, revision: 8 }]
      pending.resolve(readyGroup)
      await flush()

      expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
      expect(t.openCalls).toEqual([])
      wrapper.unmount()
      wrappers.splice(wrappers.indexOf(wrapper), 1)
    }
  })

  it('backgrounding and unmount discard an in-flight quick result', async () => {
    const transport = runtimeTransport([profile])
    const pending = deferred<HostDto>()
    transport.runtimeSessions.mockReturnValueOnce(pending.promise)
    const { wrapper, transport: t } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()
    setVisibility(true)
    pending.resolve(readyGroup)
    await flush()
    setVisibility(false)
    await flush()
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
    expect(t.openCalls).toEqual([])

    // Unmount mid-flight: the late reply must never connect.
    const transport2 = runtimeTransport([profile])
    const pending2 = deferred<HostDto>()
    transport2.runtimeSessions.mockReturnValueOnce(pending2.promise)
    const mounted = mountApp(transport2)
    await flush()
    await mounted.wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()
    mounted.wrapper.unmount()
    wrappers.splice(wrappers.indexOf(mounted.wrapper), 1)
    pending2.resolve(readyGroup)
    await flush()
    expect(transport2.openCalls).toEqual([])
  })

  it('double clicks stay one bounded request; reselection supersedes the older one', async () => {
    const transport = runtimeTransport([profile])
    const pending = deferred<HostDto>()
    transport.runtimeSessions.mockReturnValueOnce(pending.promise)
    const { wrapper, transport: t } = mountApp(transport)
    await flush()
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()
    expect(t.runtimeSessions).toHaveBeenCalledTimes(1)
    pending.resolve(readyGroup)
    await flush()
    expect(t.openCalls).toEqual([{ targetId: 'runtime-target-1', rows: 24, cols: 80 }])

    const pendingA = deferred<HostDto>()
    const readyB = runtimeTarget('runtime-target-b', profileB.id, 'ready', true, 'shell-b')
    const transport2 = runtimeTransport([profile, profileB])
    transport2.runtimeSessions.mockImplementation((id: string) =>
      id === profile.id ? pendingA.promise : Promise.resolve(sessionGroup(profileB, [readyB]))
    )
    const mounted = mountApp(transport2)
    await flush()
    await mounted.wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await mounted.wrapper.get('[data-runtime-label="hana"]').trigger('click')
    pendingA.resolve(readyGroup)
    await flush()
    // The superseded A reply landed but applied nothing.
    expect(mounted.wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
    expect(mounted.transport.openCalls).toEqual([{ targetId: 'runtime-target-b', rows: 24, cols: 80 }])
  })

  it('the Shell button always opens the manager and cancels a pending quick request', async () => {
    const transport = runtimeTransport([profile])
    const pending = deferred<HostDto>()
    transport.runtimeSessions.mockReturnValueOnce(pending.promise)
    const { wrapper, transport: t } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await wrapper.get(`[data-testid="runtime-shell-${profile.id}"]`).trigger('click')
    await flush()

    const dialog = wrapper.get('[data-testid="runtime-connection-dialog"]')
    expect(dialog.text()).toContain('nuc 的 Shell')
    // Chooser performed its own current list after cancelling the quick one.
    expect(t.runtimeSessions).toHaveBeenCalledTimes(2)
    pending.resolve(readyGroup)
    await flush()
    expect(t.openCalls).toEqual([])
  })

  it('a list failure shows fixed safe text — no raw error or credential material', async () => {
    const transport = runtimeTransport([profile])
    transport.runtimeSessions.mockRejectedValueOnce(new Error('Bearer fixture-secret-detail http trace'))
    const { wrapper, controller } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()

    expect(controller.uiNotice.value).toBe('無法列出 Shell，請確認連線與 runtime 版本。')
    expect(wrapper.text()).not.toContain('fixture-secret-detail')
    expect(wrapper.text()).not.toContain('Bearer')
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
  })

  it('a pending quick click shows a small loading hint and blocks repeat clicks', async () => {
    const transport = runtimeTransport([profile])
    const pending = deferred<HostDto>()
    transport.runtimeSessions.mockReturnValueOnce(pending.promise)
    const { wrapper, transport: t } = mountApp(transport)
    await flush()

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()
    const entry = wrapper.get(`[data-testid="runtime-${profile.id}"]`)
    expect(entry.text()).toContain('查詢中…')
    const main = wrapper.get('[data-runtime-label="nuc"]')
    expect(main.attributes('disabled')).toBeDefined()
    await main.trigger('click')
    expect(t.runtimeSessions).toHaveBeenCalledTimes(1)
    pending.resolve(readyGroup)
    await flush()
    expect(t.openCalls).toHaveLength(1)
  })
})

describe('local connected green dot', () => {
  it('lights only for a live runtime tab of the exact host id and extinguishes on detach/close', async () => {
    const transport = runtimeTransport([profile])
    const { wrapper, controller } = mountApp(transport)
    await flush()

    // Saved metadata alone never lights the dot.
    expect(statusDot(wrapper).classes()).not.toContain('on')

    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()
    expect(statusDot(wrapper).classes()).toContain('on')
    expect(statusDot(wrapper).attributes('title')).toBe('已連線')

    await controller.detachTab(controller.tabs[0])
    await flush()
    expect(statusDot(wrapper).classes()).not.toContain('on')
  })

  it('a second live runtime tab for the same host keeps the dot when one closes', async () => {
    const second = runtimeTarget('runtime-target-2', profile.id, 'ready', true, 'shell-two')
    const transport = runtimeTransport([profile], sessionGroup(profile, [ready, second]))
    const { wrapper, controller } = mountApp(transport)
    await flush()

    // Open both Shells through the explicit chooser.
    await wrapper.get(`[data-testid="runtime-shell-${profile.id}"]`).trigger('click')
    await flush()
    const connectButtons = () =>
      wrapper
        .get('[data-testid="runtime-connection-dialog"]')
        .findAll('button')
        .filter((b) => b.text() === '連線')
    await connectButtons()[0].trigger('click')
    await flush()
    await wrapper.get(`[data-testid="runtime-shell-${profile.id}"]`).trigger('click')
    await flush()
    await connectButtons()[1].trigger('click')
    await flush()
    expect(controller.tabs).toHaveLength(2)

    await controller.detachTab(controller.tabs[0])
    await flush()
    expect(statusDot(wrapper).classes()).toContain('on')

    await controller.detachTab(controller.tabs[1])
    await flush()
    expect(statusDot(wrapper).classes()).not.toContain('on')
  })

  it('a same-name other profile never lights this dot; connecting/error states stay dark', async () => {
    const sameName: RuntimeHostDto = { ...profile, id: 'runtime-profile-9', revision: 2 }
    const sameNameTarget = runtimeTarget('runtime-target-9', sameName.id, 'ready', true, 'shell-nine')
    const transport = runtimeTransport([profile, sameName])
    transport.runtimeSessions.mockImplementation((id: string) =>
      Promise.resolve(id === sameName.id ? sessionGroup(sameName, [sameNameTarget]) : readyGroup)
    )
    const { wrapper, controller, transport: t } = mountApp(transport)
    await flush()

    // Same display name, different host id — lights only its own dot.
    await wrapper.get('[data-testid="runtime-runtime-profile-9"] [data-runtime-label]').trigger('click')
    await flush()
    expect(controller.tabs[0].runtime).toBe(true)
    expect(statusDot(wrapper).classes()).not.toContain('on')
    expect(statusDot(wrapper, sameName.id).classes()).toContain('on')

    // An in-flight connect stays dark until live.
    const gate = deferred<void>()
    const realOpen = t.open.bind(t)
    t.open = async (targetId: string, rows: number, cols: number) => {
      await gate.promise
      return realOpen(targetId, rows, cols)
    }
    await wrapper.get('[data-testid="runtime-runtime-profile-1"] [data-runtime-label]').trigger('click')
    await flush()
    expect(controller.tabs[1].status).toBe('connecting')
    expect(statusDot(wrapper).classes()).not.toContain('on')

    gate.resolve()
    await flush()
    expect(controller.tabs[1].status).toBe('live')
    expect(statusDot(wrapper).classes()).toContain('on')
  })
})

describe('mobile inline attachment', () => {
  it('loads ready Shells read-only and attaches only after an explicit action', async () => {
    const transport = mobileRuntimeTransport([profile], readyGroup)
    const { wrapper, controller, transport: t } = mountMobile(transport)
    await flush()
    expect(t.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, profile.revision)
    expect(t.openCalls).toEqual([])
    await wrapper.get('.attach-button').trigger('click'); await flush()
    expect(t.openCalls).toEqual([{ targetId: 'runtime-target-1', rows: 24, cols: 80 }])
    expect(wrapper.get('.term-page').isVisible()).toBe(true)
    expect(controller.tabs[0].hostId).toBe(profile.id)
    await showMobileMain(wrapper)
    expect(statusDot(wrapper).classes()).toContain('on')
  })

  it('attaching the same inline Shell reuses the live local tab', async () => {
    const transport = mobileRuntimeTransport([profile])
    const { wrapper, controller, transport: t } = mountMobile(transport)
    await flush()
    await wrapper.get('.attach-button').trigger('click'); await flush()
    expect(controller.tabs[0].status).toBe('live')
    await showMobileMain(wrapper)
    const listings = t.runtimeSessions.mock.calls.length
    await wrapper.get('.attach-button').trigger('click'); await flush()
    expect(t.runtimeSessions).toHaveBeenCalledTimes(listings)
    expect(t.openCalls).toHaveLength(1)
    expect(controller.tabs).toHaveLength(1)
    expect(wrapper.get('.term-page').isVisible()).toBe(true)
  })

  it('expands multiple inline Shells and requires explicit takeover selection', async () => {
    const attached = runtimeTarget('attached-shell', profile.id, 'attached', true, 'busy-shell')
    const transport = mobileRuntimeTransport([profile], sessionGroup(profile, [ready, attached]))
    const { wrapper, transport: t } = mountMobile(transport)
    await flush()
    expect(t.openCalls).toEqual([])
    expect(wrapper.get('.connection-summary').text()).toContain('02')
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click'); await flush()
    const choices = wrapper.findAll('.attach-button')
    expect(choices).toHaveLength(2)
    expect(choices[1].text()).toContain('接管')
    expect(t.openCalls).toEqual([])
    await choices[1].trigger('click'); await flush()
    expect(t.openCalls.map(call => call.targetId)).toEqual([attached.id])
  })
})

describe('a later user selection retires pending quick connect', () => {
  it('reusing a live local Computer cancels a different pending Computer query', async () => {
    const transport = runtimeTransport([profile, profileB])
    const { wrapper, controller } = mountApp(transport)
    await flush()
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    await flush()
    const live = controller.tabs[0]
    const gate = deferred<HostDto>()
    transport.runtimeSessions.mockImplementation(() => gate.promise)
    await wrapper.get('[data-runtime-label="hana"]').trigger('click')
    await flush()
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    gate.resolve(sessionGroup(profileB, [runtimeTarget('runtime-target-b', profileB.id, 'ready')]))
    await flush()
    expect(transport.openCalls).toHaveLength(1)
    expect(controller.activeKey.value).toBe(live.key)
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
  })

  it('selecting a Shell through another profile chooser prevents a late quick result', async () => {
    const other = runtimeTarget('runtime-target-b', profileB.id, 'ready', true, 'shell-b')
    const transport = runtimeTransport([profile, profileB])
    const gate = deferred<HostDto>()
    transport.runtimeSessions.mockImplementation((id: string) =>
      id === profile.id ? gate.promise : Promise.resolve(sessionGroup(profileB, [other]))
    )
    const { wrapper } = mountApp(transport)
    await flush()
    await wrapper.get('[data-runtime-label="nuc"]').trigger('click')
    // The Shell button cancels the pending nuc query and opens hana's chooser.
    await wrapper.get(`[data-testid="runtime-shell-${profileB.id}"]`).trigger('click')
    await flush()
    const dialog = wrapper.get('[data-testid="runtime-connection-dialog"]')
    await dialog.findAll('button').find((b) => b.text() === '連線')!.trigger('click')
    gate.resolve(readyGroup)
    await flush()
    expect(transport.openCalls.map((call) => call.targetId)).toEqual([other.id])
  })

  it('an inline selection prevents a late read-only result from switching targets', async () => {
    const other = runtimeTarget('runtime-target-b', profileB.id, 'ready', true, 'shell-b')
    const transport = mobileRuntimeTransport([profile, profileB])
    const gate = deferred<HostDto>()
    transport.runtimeSessions.mockImplementation((id: string) => id === profile.id ? gate.promise : Promise.resolve(sessionGroup(profileB, [other])))
    const { wrapper, controller } = mountMobile(transport)
    await flush()
    expect(transport.openCalls).toEqual([])
    await wrapper.get(`[data-testid="runtime-${profileB.id}"] .attach-button`).trigger('click')
    gate.resolve(readyGroup); await flush()
    expect(transport.openCalls.map(call => call.targetId)).toEqual([other.id])
    expect(controller.activeTab.value?.targetId).toBe(other.id)
  })
})
