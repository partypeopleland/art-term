import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import App from '../src/App.vue'
import CategoryList from '../src/components/CategoryList.vue'
import MobileApp from '../src/mobile/MobileApp.vue'
import type { AppController } from '../src/state/controller'
import type { HostDto, RuntimeHostDto, RuntimeHostInput, TargetDto } from '../src/transport/types'
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
const issuedTarget: TargetDto = { ...makeTarget('issued-shell-1', profile.id, 'host'), phase: 'detached', label: 'shell-one' }
const sessionGroup: HostDto = { id: profile.id, label: profile.name, desktopAvailable: false, targets: [issuedTarget] }

function runtimeTransport(store: RuntimeHostDto[]) {
  return Object.assign(new FakeTransport(), {
    runtimeHosts: vi.fn(async () => store),
    runtimeSaveHost: vi.fn(async (input: RuntimeHostInput) => {
      const saved: RuntimeHostDto = {
        id: input.id ?? profile.id,
        revision: (input.id ? store.find((p) => p.id === input.id)?.revision ?? 0 : 0) + 1,
        name: input.name,
        baseUrl: input.baseUrl,
        credentialPresent: true
      }
      store.splice(0, store.length, ...store.filter((p) => p.id !== saved.id), saved)
      return saved
    }),
    runtimeDeleteHost: vi.fn(async (id: string, _expectedRevision: number) => {
      store.splice(0, store.length, ...store.filter((p) => p.id !== id))
    }),
    runtimeTestHost: vi.fn(async () => ({ sessionCount: 0, draining: false, teardownBestEffort: true, continuityVerified: false })),
    runtimeSessions: vi.fn(async () => sessionGroup),
    runtimeCreateSession: vi.fn(async () => issuedTarget),
    runtimeCreateHistorySession: vi.fn(async () => issuedTarget)
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

async function button(wrapper: ReturnType<typeof mount>, label: string) {
  const node = wrapper.findAll('button').find((item) => item.text() === label)
  expect(node, label).toBeDefined()
  await node!.trigger('click')
}

async function dialogButton(wrapper: ReturnType<typeof mount>, label: string) {
  const node = wrapper
    .get('[data-testid="runtime-connection-dialog"]')
    .findAll('button')
    .find((item) => item.text() === label)
  expect(node, label).toBeDefined()
  await node!.trigger('click')
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

describe('saved runtime Computers in the shared catalog', () => {
  it('save→close settings→saved Computer appears in the outer computers category', async () => {
    const store: RuntimeHostDto[] = []
    const transport = runtimeTransport(store)
    const { wrapper } = mountApp(transport)
    await flush()
    expect(wrapper.find('[data-runtime-label]').exists()).toBe(false)

    await wrapper.get('[data-testid="desktop-settings"]').trigger('click')
    await flush()
    await wrapper.get('[data-testid="name"]').setValue('nuc')
    await wrapper.get('[data-testid="base-url"]').setValue('https://runtime.example.test')
    await wrapper.get('[data-testid="credential"]').setValue('fixture-admin')
    await wrapper.get('form').trigger('submit')
    await flush()
    await wrapper.get('[data-testid="desktop-workspace"]').trigger('click')
    await flush()

    const cat = wrapper.get('[data-testid="cat-computers"]')
    const entry = cat.get('[data-runtime-label="nuc"]')
    expect(entry.text()).toContain('nuc')
    expect(cat.text()).toContain('Computers')
    // Local metadata only — never advertised as online before a probe.
    expect(cat.get('[data-testid^="runtime-status-"]').classes()).not.toContain('on')
    expect(transport.runtimeHosts.mock.calls.length).toBeGreaterThanOrEqual(3)
  })

  it('initial boot lists stored Shells read-only; explicit attach uses the issued target', async () => {
    const store: RuntimeHostDto[] = [profile]
    const transport = runtimeTransport(store)
    const { wrapper, transport: t } = mountApp(transport)
    await flush()

    expect(wrapper.find('[data-runtime-label="nuc"]').exists()).toBe(true)
    expect(t.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, 7)
    expect(t.runtimeCreateSession).not.toHaveBeenCalled()
    expect(t.openCalls).toEqual([])
    expect(wrapper.text()).toContain('shell-one')

    // Explicit connect emits the backend-issued target unchanged.
    await wrapper.get('.attach-button').trigger('click')
    await flush()
    expect(t.openCalls).toEqual([{ targetId: 'issued-shell-1', rows: 24, cols: 80 }])
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
  })

  it('a failed profile read surfaces a safe fixed error without leaking payload', async () => {
    const transport = runtimeTransport([])
    transport.runtimeHosts = vi.fn(async () => {
      throw { kind: 'storage', message: 'fixture-admin https://evil.fixture' }
    })
    const { wrapper } = mountApp(transport)
    await flush()
    const err = wrapper.get('[data-testid="runtime-profiles-error"]')
    expect(err.text()).toContain('[STORAGE]')
    expect(wrapper.text()).not.toContain('fixture-admin')
    expect(wrapper.text()).not.toContain('evil.fixture')
    expect(wrapper.find('[data-runtime-label]').exists()).toBe(false)
  })

  it('unknown profile-read failures fall back to generic text without leaking payload', async () => {
    const transport = runtimeTransport([])
    transport.runtimeHosts = vi.fn(async () => {
      throw new Error('fixture-admin Bearer private detail')
    })
    const { wrapper } = mountApp(transport)
    await flush()
    const err = wrapper.get('[data-testid="runtime-profiles-error"]')
    expect(err.text()).toContain('無法讀取已儲存的 Computer 設定')
    expect(wrapper.text()).not.toContain('fixture-admin')
    expect(wrapper.text()).not.toContain('Bearer')
  })

  it('a synchronous profile-read failure surfaces the safe error', async () => {
    const transport = runtimeTransport([])
    transport.runtimeHosts.mockImplementationOnce(() => { throw { kind: 'storage', message: 'fixture-secret' } })
    const { wrapper } = mountApp(transport)
    await flush()
    expect(wrapper.get('[data-testid="runtime-profiles-error"]').text()).toContain('[STORAGE]')
    expect(wrapper.text()).not.toContain('fixture-secret')
  })

  it('a stale list reply never resurrects a deleted profile', async () => {
    const first = deferred<RuntimeHostDto[]>()
    const second = deferred<RuntimeHostDto[]>()
    const transport = runtimeTransport([])
    transport.runtimeHosts = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)
    const { controller } = mountApp(transport)
    await flush()
    // Profile deleted; the fresh reload resolves empty before the older
    // pending reply lands — generation fencing must keep it empty.
    void controller.loadCatalog()
    second.resolve([])
    await flush()
    first.resolve([profile])
    await flush()
    expect(controller.runtimeProfiles.value).toEqual([])
  })

  it('deleting a profile removes its entry after the changed reload', async () => {
    const store: RuntimeHostDto[] = [profile]
    const transport = runtimeTransport(store)
    const { wrapper, controller } = mountApp(transport)
    await flush()
    expect(wrapper.find('[data-runtime-label="nuc"]').exists()).toBe(true)
    await transport.runtimeDeleteHost(profile.id, profile.revision)
    await controller.loadCatalog()
    await flush()
    expect(wrapper.find('[data-runtime-label="nuc"]').exists()).toBe(false)
    expect(controller.runtimeProfiles.value).toEqual([])
  })

  it('bounds saved runtime entries to 64', async () => {
    const many = Array.from({ length: 65 }, (_, i) => ({ ...profile, id: `p-${i}`, name: `host-${i}` }))
    const transport = runtimeTransport(many)
    const { wrapper, controller } = mountApp(transport)
    await flush()
    expect(controller.runtimeProfiles.value).toHaveLength(64)
    expect(wrapper.findAll('[data-runtime-label]')).toHaveLength(64)
  })

  it('backgrounding the inline list prevents a late create from connecting', async () => {
    const transport = runtimeTransport([profile])
    const pending = deferred<TargetDto>()
    transport.runtimeCreateHistorySession.mockReturnValueOnce(pending.promise)
    const { wrapper, transport: t } = mountApp(transport)
    await flush()
    await wrapper.get('.new-connection').trigger('click')
    await flush()
    setVisibility(true)
    await flush()
    expect(wrapper.find('[data-testid="runtime-connection-dialog"]').exists()).toBe(false)
    pending.resolve(issuedTarget)
    await flush()
    setVisibility(false)
    await flush()
    expect(t.openCalls).toEqual([])
    expect(t.runtimeCreateHistorySession).toHaveBeenCalledExactlyOnceWith(profile.id, 7)
  })

  it('closing host management preserves the existing live session', async () => {
    const transport = runtimeTransport([profile])
    const { wrapper, transport: t, controller } = mountApp(transport)
    await flush()
    // An existing session stays untouched by dialog lifecycle.
    controller.openTarget({ dto: sessionGroup, expanded: true }, issuedTarget, false)
    await flush()
    await wrapper.get('[data-testid="desktop-settings"]').trigger('click')
    await wrapper.get('[data-testid="desktop-manage-hosts"]').trigger('click')
    await flush()
    await button(wrapper, '關閉')
    await flush()
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
    expect(controller.tabs).toHaveLength(1)
    expect(controller.tabs[0].status).toBe('live')
    expect(t.openCalls).toEqual([{ targetId: 'issued-shell-1', rows: 24, cols: 80 }])
  })
})

describe('mobile shared runtime catalog entries', () => {
  function mobileTransport(store: RuntimeHostDto[]) {
    const t = new FakeMobileTransport()
    return Object.assign(t, {
      runtimeHosts: vi.fn(async () => store),
      runtimeSaveHost: vi.fn(async () => profile),
      runtimeDeleteHost: vi.fn(async () => {}),
      runtimeTestHost: vi.fn(async () => ({ sessionCount: 0, draining: false, teardownBestEffort: true, continuityVerified: false })),
      runtimeSessions: vi.fn(async () => sessionGroup),
      runtimeCreateSession: vi.fn(async () => issuedTarget),
    runtimeCreateHistorySession: vi.fn(async () => issuedTarget)
    })
  }

  it('mobile lists saved Shells read-only and attaches only on explicit connect', async () => {
    const transport = mobileTransport([profile])
    const { wrapper, transport: t, controller } = mountMobile(transport)
    await flush()
    expect(wrapper.get('[data-runtime-label="nuc"]').text()).toContain('openab-pty')
    expect(t.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, 7)
    expect(t.openCalls).toEqual([])
    await wrapper.get('.attach-button').trigger('click'); await flush()
    expect(t.openCalls).toEqual([{ targetId: 'issued-shell-1', rows: 24, cols: 80 }])
    expect(wrapper.get('.term-page').isVisible()).toBe(true)
    expect(controller.tabs[0].targetId).toBe('issued-shell-1')
    expect(controller.tabs[0].hostId).toBe(profile.id)
  })

  it('mobile background invalidates an in-flight inline listing without attaching', async () => {
    const transport = mobileTransport([profile])
    const pending = deferred<HostDto>()
    transport.runtimeSessions.mockReturnValueOnce(pending.promise)
    const { wrapper, transport: t } = mountMobile(transport)
    await flush()
    expect(wrapper.find('.attach-button').exists()).toBe(false)
    setVisibility(true); await flush()
    pending.resolve(sessionGroup); await flush()
    expect(t.openCalls).toEqual([])
    expect(wrapper.find('.attach-button').exists()).toBe(false)
    setVisibility(false); await flush()
    expect(t.openCalls).toEqual([])
    expect(wrapper.find('.attach-button').exists()).toBe(true)
  })

  it('mobile profile-read failure surfaces a safe fixed error', async () => {
    const transport = mobileTransport([])
    transport.runtimeHosts = vi.fn(async () => {
      throw { kind: 'auth', message: 'fixture-admin' }
    })
    const { wrapper } = mountMobile(transport)
    await flush()
    const err = wrapper.get('[data-testid="runtime-profiles-error"]')
    expect(err.text()).toContain('[AUTH]')
    expect(wrapper.text()).not.toContain('fixture-admin')
  })
})

it('filters the shared connection list without opening or creating sessions', async () => {
  const wrapper = mount(CategoryList, { props: { catalogLoading: false, runtimeProfiles: [profile, { ...profile, id: 'agent', category: 'agent', name: 'Agent fixture' }] } }); wrappers.push(wrapper)
  await wrapper.get('[data-testid=runtime-category-filter]').setValue('agent')
  expect(wrapper.find('[data-testid=runtime-runtime-profile-1]').exists()).toBe(false)
  expect(wrapper.find('[data-testid=runtime-agent]').exists()).toBe(true)
  await wrapper.get('[data-testid=runtime-category-filter]').setValue('host')
  expect(wrapper.find('[data-testid=runtime-agent]').exists()).toBe(false)
  expect(wrapper.find('[data-testid=runtime-runtime-profile-1]').exists()).toBe(true)
  expect(wrapper.emitted('runtime-open')).toBeUndefined(); expect(wrapper.emitted('runtime-shell')).toBeUndefined()
})
