import { attachMobileProfile, showMobileMain } from './mobile-actions'
// Runtime-agent binding retirement: a typed terminal-kind rejection
// (stateChanged/replacement/…) on a runtime-agent tab closes only that
// tab, reloads the saved profile list exactly once, and surfaces manual
// reselection — never auto-connecting a fresh opaque id or disturbing
// unrelated tabs.
import { describe, it, expect, vi, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import App from '../src/desktop/App.vue'
import MobileApp from '../src/mobile/MobileApp.vue'
import { createAppController } from '../src/shared/state/controller'
import type { AppController } from '../src/shared/state/controller'
import { ConnectionError } from '../src/shared/transport/types'
import type { RuntimeHostDto } from '../src/shared/transport/types'
import { FakeTerm, FakeTransport, deferred, makeHost, makeProfile, makeTarget } from './fake'
import { FakeMobileTransport, makeCatalogTarget, makeGroup } from './mobile.fake'

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

const PROFILES: RuntimeHostDto[] = [
  makeProfile('hana', 'hana', { category: 'agent', agentName: 'hana', agentTool: 'Codex' }),
  makeProfile('beta', 'beta', { category: 'agent', agentName: 'beta', agentTool: 'Devin' }),
  makeProfile('nuc', 'nuc')
]
const gHana = makeHost('hana', [makeTarget('opq-hana', 'hana', 'host', true, false, 'Codex')])
const gBeta = makeHost('beta', [makeTarget('opq-beta', 'beta', 'host', true, false, 'Devin')])
const gNuc = makeHost('nuc', [makeTarget('opq-nuc', 'nuc', 'host')])
gHana.targets[0].label = 'hana'
gBeta.targets[0].label = 'beta'
gNuc.targets[0].label = 'nuc'
const HANA = { dto: gHana, expanded: true }
const BETA = { dto: gBeta, expanded: true }
const NUC = { dto: gNuc, expanded: true }

function runtimeTransport(): FakeTransport {
  const t = new FakeTransport(PROFILES)
  t.runtimeSessionsData.set('hana', gHana)
  t.runtimeSessionsData.set('beta', gBeta)
  t.runtimeSessionsData.set('nuc', gNuc)
  return t
}

function countProfileLoads(transport: FakeTransport): () => number {
  let calls = 0
  const orig = transport.runtimeHosts.bind(transport)
  transport.runtimeHosts = async () => {
    calls++
    return orig()
  }
  return () => calls
}

function mobileRuntime(): FakeMobileTransport {
  const t = new FakeMobileTransport(PROFILES)
  t.runtimeSessionsData.set('hana', makeGroup('hana', [makeCatalogTarget('opq-hana', 'hana', 'hana', 'Codex')], 'hana'))
  t.runtimeSessionsData.set('beta', makeGroup('beta', [makeCatalogTarget('opq-beta', 'beta', 'beta', 'Devin')], 'beta'))
  t.runtimeSessionsData.set('nuc', makeGroup('nuc', [makeCatalogTarget('opq-nuc', 'nuc', 'nuc', 'openab-pty')], 'nuc'))
  return t
}

describe('runtime-agent binding retirement (shared controller)', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('stateChanged on an agent target closes only that tab, frees queued work, and refreshes once', async () => {
    const transport = runtimeTransport()
    const catalogs = countProfileLoads(transport)
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      // An unrelated live tab keeps running through the whole recovery.
      controller.openTarget(BETA, BETA.dto.targets[0], false)
      controller.attachTerminal(controller.tabs[0], new FakeTerm())
      await flush()
      const live = controller.tabs[0]
      expect(live.status).toBe('live')

      transport.openError = new ConnectionError('stateChanged', 'runtime state changed; refresh the target list')
      controller.openTarget(HANA, HANA.dto.targets[0], false)
      const stale = controller.tabs[1]
      controller.attachTerminal(stale, new FakeTerm())
      stale.writeBuf = [1, 2, 3]
      stale.pendingSize = { rows: 40, cols: 120 }
      await flush()

      expect(controller.tabs).toHaveLength(1)
      expect(controller.tabs[0]).toBe(live)
      expect(live.status).toBe('live')
      expect(live.sessionId).toBe('s1')
      expect(stale.closed).toBe(true)
      expect(stale.writeBuf).toEqual([])
      expect(stale.pendingSize).toBeNull()
      // Exactly one bounded profile reload for this failure.
      expect(catalogs()).toBe(2)
      expect(controller.selectionRequest.value).toBe(1)
      expect(controller.uiNotice.value).toContain('hana')
      expect(controller.uiNotice.value).toContain('重新選擇')
      // The failed open was the only open; nothing auto-connects.
      expect(transport.openCalls.map((c) => c.targetId)).toEqual(['opq-beta', 'opq-hana'])
    } finally {
      controller.dispose()
    }
  })

  it('a reconnect that hits the ended binding closes the tab and its ended session is released once', async () => {
    const transport = runtimeTransport()
    const catalogs = countProfileLoads(transport)
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      controller.openTarget(HANA, HANA.dto.targets[0], false)
      const tab = controller.tabs[0]
      controller.attachTerminal(tab, new FakeTerm())
      await flush()
      expect(tab.status).toBe('live')
      // Session ends; EOF already released s1 once.
      transport.feed('s1', { bytes: [], exitCode: 0 })
      await flush()
      expect(tab.status).toBe('exited')
      expect(transport.closeCalls).toEqual(['s1'])

      transport.openError = new ConnectionError('stateChanged', 'runtime state changed')
      controller.reopenTab(tab, false)
      await flush()
      expect(tab.closed).toBe(true)
      expect(controller.tabs).toHaveLength(0)
      // The terminal rejection did not open or close another session.
      expect(transport.closeCalls).toEqual(['s1'])
      expect(catalogs()).toBe(2)
      expect(controller.selectionRequest.value).toBe(1)
    } finally {
      controller.dispose()
    }
  })

  it('a freshly listed opaque id connects only after an explicit selection — never auto-opened', async () => {
    const transport = runtimeTransport()
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      transport.openError = new ConnectionError('stateChanged', 'runtime state changed')
      controller.openTarget(HANA, HANA.dto.targets[0], false)
      controller.attachTerminal(controller.tabs[0], new FakeTerm())
      await flush()
      expect(controller.tabs).toHaveLength(0)
      expect(transport.openCalls).toHaveLength(1)

      // The user re-selects through a fresh session list; the new opaque id
      // opens only on the explicit pick.
      transport.openError = null
      const fresh = { ...HANA.dto.targets[0], id: 'opq-hana.r1' }
      controller.openTarget(HANA, fresh, false)
      controller.attachTerminal(controller.tabs[0], new FakeTerm())
      await flush()
      expect(transport.openCalls.map((c) => c.targetId)).toEqual(['opq-hana', 'opq-hana.r1'])
      expect(controller.tabs[0].status).toBe('live')
      expect(controller.uiNotice.value).toBe('')
    } finally {
      controller.dispose()
    }
  })

  it('generic open failures never trigger the retirement path', async () => {
    const transport = runtimeTransport()
    const catalogs = countProfileLoads(transport)
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      // transport kind on an agent target: plain error, tab retained.
      transport.openError = new ConnectionError('transport', 'network unreachable')
      controller.openTarget(HANA, HANA.dto.targets[0], false)
      controller.attachTerminal(controller.tabs[0], new FakeTerm())
      await flush()
      expect(controller.tabs[0].status).toBe('error')
      expect(controller.tabs).toHaveLength(1)
      expect(catalogs()).toBe(1)
      expect(controller.selectionRequest.value).toBe(0)

      // stateChanged on a non-agent (plain computer) tab keeps its error
      // semantics — no auto-close, no reload.
      transport.openError = new ConnectionError('stateChanged', 'Host settings changed')
      controller.openTarget(NUC, NUC.dto.targets[0], false)
      controller.attachTerminal(controller.tabs[1], new FakeTerm())
      await flush()
      expect(controller.tabs[1].status).toBe('error')
      expect(controller.tabs[1].errorMsg).toContain('Host settings changed')
      expect(controller.tabs).toHaveLength(2)
      expect(catalogs()).toBe(1)
      expect(controller.selectionRequest.value).toBe(0)
    } finally {
      controller.dispose()
    }
  })

  it('a stale rejection landing after a manual close refreshes nothing', async () => {
    const transport = runtimeTransport()
    const catalogs = countProfileLoads(transport)
    const gate = deferred<void>()
    const origOpen = transport.open.bind(transport)
    transport.open = async (...args) => {
      await gate.promise
      return origOpen(...args)
    }
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      controller.openTarget(HANA, HANA.dto.targets[0], false)
      const tab = controller.tabs[0]
      controller.attachTerminal(tab, new FakeTerm())
      await controller.closeTab(tab)
      gate.reject(new ConnectionError('stateChanged', 'runtime state changed'))
      await flush()
      expect(catalogs()).toBe(1)
      expect(controller.selectionRequest.value).toBe(0)
      expect(controller.uiNotice.value).toBe('')
    } finally {
      controller.dispose()
    }
  })

  it('a disposed controller never mutates state from a late terminal rejection', async () => {
    const transport = runtimeTransport()
    const catalogs = countProfileLoads(transport)
    const gate = deferred<void>()
    const origOpen = transport.open.bind(transport)
    transport.open = async (...args) => {
      await gate.promise
      return origOpen(...args)
    }
    const controller = createAppController(transport)
    await controller.loadCatalog()
    controller.openTarget(HANA, HANA.dto.targets[0], false)
    const tab = controller.tabs[0]
    controller.attachTerminal(tab, new FakeTerm())
    controller.dispose()
    gate.reject(new ConnectionError('stateChanged', 'runtime state changed'))
    await flush()
    expect(catalogs()).toBe(1)
    expect(controller.selectionRequest.value).toBe(0)
    expect(tab.closed).toBe(false)
  })

  it('a failed profile reload surfaces the safe error without a retry loop', async () => {
    const transport = runtimeTransport()
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      transport.openError = new ConnectionError('stateChanged', 'runtime state changed')
      transport.runtimeHosts = () => Promise.reject(new Error('fixture-admin private detail'))
      controller.openTarget(HANA, HANA.dto.targets[0], false)
      controller.attachTerminal(controller.tabs[0], new FakeTerm())
      await flush()
      expect(controller.tabs).toHaveLength(0)
      expect(controller.runtimeProfilesError.value).toContain('無法讀取已儲存的')
      expect(controller.runtimeProfilesError.value).not.toContain('fixture-admin')
      expect(controller.catalogLoading.value).toBe(false)
      expect(controller.uiNotice.value).toContain('重新選擇')
      // A single load attempt — failure is visible, never retried here.
    } finally {
      controller.dispose()
    }
  })

  it('a newer profile load wins over a late recovery snapshot', async () => {
    const transport = runtimeTransport()
    const controller = createAppController(transport)
    const first = deferred<RuntimeHostDto[]>()
    const second = deferred<RuntimeHostDto[]>()
    const newer = [...PROFILES, makeProfile('extra', 'extra')]
    try {
      await controller.loadCatalog()
      transport.openError = new ConnectionError('stateChanged', 'runtime state changed')
      let calls = 0
      transport.runtimeHosts = () => (++calls === 1 ? first.promise : second.promise)
      controller.openTarget(HANA, HANA.dto.targets[0], false)
      controller.attachTerminal(controller.tabs[0], new FakeTerm())
      await flush()
      expect(calls).toBe(1)
      // Manual refresh supersedes the in-flight recovery load.
      void controller.loadCatalog()
      await flush()
      expect(calls).toBe(2)
      second.resolve(newer)
      await flush()
      first.resolve(PROFILES) // late, stale snapshot — must be dropped
      await flush()
      expect(controller.runtimeProfiles.value).toHaveLength(4)
      expect(controller.catalogLoading.value).toBe(false)
    } finally {
      controller.dispose()
    }
  })
})

interface MountedMobile {
  wrapper: ReturnType<typeof mount>
  controller: AppController
  transport: FakeMobileTransport
  terms: FakeTerm[]
}

function mountMobile(transport: FakeMobileTransport): MountedMobile {
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
    attachTo: document.body
  })
  const vm = wrapper.vm as unknown as { controller: AppController }
  return { wrapper, controller: vm.controller, transport, terms }
}

async function tapTarget(m: MountedMobile, label: string): Promise<void> {
  await attachMobileProfile(m.wrapper, label)
  await flush()
}

describe('mobile runtime-agent retirement flow', () => {
  afterEach(() => {
    document.body.innerHTML = ''
    vi.restoreAllMocks()
  })

  it('closes the stale tab, lands on the refreshed list with the notice', async () => {
    const t = mobileRuntime()
    t.openErrors.set('opq-hana', new ConnectionError('stateChanged', 'runtime state changed'))
    let catalogs = 0
    const orig = t.runtimeHosts.bind(t)
    t.runtimeHosts = async () => {
      catalogs++
      return orig()
    }
    const m = mountMobile(t)
    try {
      await flush()
      expect(catalogs).toBe(1)

      await tapTarget(m, 'hana')
      await flush()

      // Back on the list page: the stale tab is gone, the notice shows.
      expect(m.wrapper.find('[data-testid="catalog-card"]').exists()).toBe(true)
      expect(m.wrapper.find('.term-page').isVisible()).toBe(false)
      const notice = m.wrapper.find('[data-testid="list-notice"]')
      expect(notice.exists()).toBe(true)
      expect(notice.text()).toContain('hana')
      expect(m.controller.tabs).toHaveLength(0)
      expect(m.controller.selectionRequest.value).toBe(1)
      // Exactly one recovery reload.
      expect(catalogs).toBe(2)
      // No automatic connection to a replacement.
      expect(t.openCalls.map((c) => c.targetId)).toEqual(['opq-hana'])
    } finally {
      m.wrapper.unmount()
    }
  })

  it('still navigates to the list when other tabs stay live', async () => {
    const t = mobileRuntime()
    t.openErrors.set('opq-beta', new ConnectionError('stateChanged', 'runtime state changed'))
    const m = mountMobile(t)
    try {
      await flush()
      await tapTarget(m, 'nuc')
      expect(m.controller.tabs[0].status).toBe('live')
      const keep = m.controller.tabs[0]

      await showMobileMain(m.wrapper)
      await flush()
      await tapTarget(m, 'beta')
      await flush()

      expect(m.controller.tabs).toHaveLength(1)
      expect(m.controller.tabs[0]).toBe(keep)
      expect(keep.status).toBe('live')
      expect(keep.sessionId).toBe('m1')
      expect(m.wrapper.find('[data-testid="list-notice"]').exists()).toBe(true)
      expect(t.openCalls.map((c) => c.targetId)).toEqual(['opq-nuc', 'opq-beta'])
    } finally {
      m.wrapper.unmount()
    }
  })

  it('a failed connection still offers a manual refresh and same-target reconnect', async () => {
    const t = mobileRuntime()
    const m = mountMobile(t)
    try {
      await flush()
      await tapTarget(m, 'nuc')
      const tab = m.controller.tabs[0]
      expect(tab.status).toBe('live')
      t.feed('m1', { bytes: [], exitCode: null, runtime: { batchId: 0, localGeneration: 1, events: [], ended: 'auth' } })
      await flush()

      const text = m.wrapper.find('.term-page').text()
      expect(text).toContain('主機狀態為準')
      let catalogs = 0
      const orig = t.runtimeHosts.bind(t)
      t.runtimeHosts = async () => {
        catalogs++
        return orig()
      }
      await m.wrapper.find('[data-testid="refresh-targets"]').trigger('click')
      await flush()
      expect(catalogs).toBe(1)
      expect(m.wrapper.find('[data-testid="catalog-card"]').exists()).toBe(true)
      expect(m.wrapper.find('.term-page').isVisible()).toBe(false)
      // A failed connection remains available for explicit retry.
      expect(m.wrapper.find('.done .btn').text()).toContain('重新連線同一目標')
    } finally {
      m.wrapper.unmount()
    }
  })
})

describe('Windows shell runtime-agent retirement surface', () => {
  it('closes only the stale tab and keeps the sidebar list refreshed', async () => {
    const t = runtimeTransport()
    const terms: FakeTerm[] = []
    const wrapper = mount(App, {
      props: {
        transport: t,
        terminalFactory: () => {
          const x = new FakeTerm()
          terms.push(x)
          return x
        },
        controllerOptions: { idlePollMs: 0, autoCaptureMs: 30_000 }
      }
    })
    try {
      const controller = (wrapper.vm as unknown as { controller: AppController }).controller
      await flush()
      await wrapper.get('[data-testid="runtime-beta"] .attach-button').trigger('click')
      await flush()
      expect(controller.tabs[0].status).toBe('live')

      t.openError = new ConnectionError('stateChanged', 'runtime state changed')
      await flush()
      await wrapper.get('[data-testid="runtime-hana"] .attach-button').trigger('click')
      await flush()

      // The failed tab is gone; the unrelated live tab is untouched.
      expect(controller.tabs).toHaveLength(1)
      expect(controller.tabs[0].targetId).toBe('opq-beta')
      expect(controller.tabs[0].status).toBe('live')
      expect(controller.selectionRequest.value).toBe(1)
      // The notice surfaces in the Windows statusbar too.
      expect(wrapper.text()).toContain('已更換')
      expect(wrapper.text()).toContain('重新選擇')
      expect(t.openCalls.map((c) => c.targetId)).toEqual(['opq-beta', 'opq-hana'])
    } finally {
      wrapper.unmount()
    }
  })
})

describe('profile-list recovery completion fences', () => {
  it('refreshes immediately despite a delayed local close and never navigates again on close completion', async () => {
    const t = runtimeTransport()
    const catalogs = countProfileLoads(t)
    const c = createAppController(t)
    const closing = deferred<void>()
    try {
      await c.loadCatalog()
      c.openTarget(HANA, HANA.dto.targets[0], false)
      const tab = c.tabs[0]
      c.attachTerminal(tab, new FakeTerm()); await flush()
      // Hold a native close while the reconnect receives a terminal reply.
      tab.status = 'exited'
      const originalClose = t.close.bind(t)
      t.close = async (id) => { await closing.promise; await originalClose(id) }
      t.openError = new ConnectionError('stateChanged', 'runtime state changed')
      c.reopenTab(tab, false); await flush()
      expect(tab.closed).toBe(true)
      expect(c.tabs).toHaveLength(0)
      expect(catalogs()).toBe(2)
      expect(c.selectionRequest.value).toBe(1)
      c.dispose()
      closing.resolve(); await flush()
      expect(c.selectionRequest.value).toBe(1)
      expect(catalogs()).toBe(2)
    } finally { closing.resolve(); c.dispose() }
  })

  it('drops late profile-load completion and rejects new loads after disposal', async () => {
    const t = runtimeTransport()
    const gate = deferred<RuntimeHostDto[]>()
    let calls = 0
    t.runtimeHosts = () => { calls++; return gate.promise }
    const c = createAppController(t)
    const pending = c.loadCatalog()
    expect(c.catalogLoading.value).toBe(true)
    c.dispose()
    expect(c.catalogLoading.value).toBe(false)
    gate.resolve(PROFILES); await pending
    await c.loadCatalog()
    expect(c.runtimeProfiles.value).toEqual([])
    expect(c.catalogLoading.value).toBe(false)
    expect(calls).toBe(1)
  })

  it('removes outdated choices on refresh failure and allows an explicit retry', async () => {
    const t = runtimeTransport()
    const c = createAppController(t)
    try {
      await c.loadCatalog()
      t.runtimeHosts = () => Promise.reject(new Error('fixture-admin private'))
      await c.refreshTargets()
      expect(c.runtimeProfiles.value).toEqual([])
      expect(c.runtimeProfilesError.value).toContain('無法讀取已儲存的')
      expect(c.runtimeProfilesError.value).not.toContain('fixture-admin')
      t.runtimeHosts = () => Promise.resolve(PROFILES)
      await c.refreshTargets()
      expect(c.runtimeProfiles.value).toHaveLength(3)
      expect(c.runtimeProfilesError.value).toBe('')
      expect(t.openCalls).toHaveLength(0)
    } finally { c.dispose() }
  })
})
