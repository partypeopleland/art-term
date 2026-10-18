import { describe, it, expect, vi } from 'vitest'
import { createAppController } from '../src/state/controller'
import { FakeTransport, FakeTerm, makeHost, makeTarget, deferred } from './fake'
import type { InstanceDesktop, InstanceSelection } from '../src/transport/types'
const HOSTS = [makeHost('host', [makeTarget('target', 'host')])]
const A: InstanceDesktop = { id: 'screen-a', revision: 1, name: 'Desktop A', mcpUrl: 'https://screen-a.example.test/mcp', tokenPresent: true }
const B: InstanceDesktop = { ...A, id: 'screen-b', name: 'Desktop B' }
async function flush() { for (let i = 0; i < 50; i++) await Promise.resolve() }
function setup() {
  const transport = new FakeTransport()
  const controller = createAppController(transport, { idlePollMs: 0 })
  transport.instanceSelect = vi.fn(async (_sid, id) => {
    const desktop = id === A.id ? A : B
    transport.shot = { ...transport.shot, desktopId: desktop.id, desktopRevision: desktop.revision }
    return { desktop, toolReady: false }
  })
  transport.instanceClear = vi.fn(async () => {})
  transport.instanceLaunch = vi.fn(async () => ({ desktop: A, toolReady: true }))
  controller.openTarget({ dto: HOSTS[0], expanded: true }, HOSTS[0].targets[0], false)
  const tab = controller.tabs[0]
  controller.attachTerminal(tab, new FakeTerm())
  return { controller, transport, tab }
}
describe('direct Screen selection', () => {
  it('previews a selected Screen independent of host desktop capability or tools readiness', async () => {
    const { controller: c, transport: t, tab } = setup()
    try {
      await flush(); tab.desktopAvailable = false
      await c.selectInstance(tab, A); await flush()
      expect(t.instanceSelect).toHaveBeenCalledWith(tab.sessionId, A.id, 1)
      expect(tab.instanceDesktop?.id).toBe(A.id)
      expect(tab.instanceToolReady).toBe(false)
      expect(tab.desktopEnabled).toBe(true)
      expect(t.screenshotCalls).toContain(tab.sessionId)
      expect(tab.desktopMsg).toBe('')
    } finally { c.dispose() }
  })
  it('clears the old image immediately and rejects a stale capture after target change', async () => {
    const { controller: c, transport: t, tab } = setup()
    try {
      await flush(); await c.selectInstance(tab, A); await flush()
      const gate = deferred<void>(); t.screenshotGate = gate.promise
      c.manualCapture(); await flush()
      await c.selectInstance(tab, B)
      expect(tab.shot).toBeNull()
      gate.resolve(); await flush()
      expect(tab.instanceDesktop?.id).toBe(B.id)
      await c.clearInstance(tab)
      expect(tab.shot).toBeNull(); expect(tab.instanceDesktop).toBeNull()
      expect(t.instanceClear).toHaveBeenCalledOnce()
    } finally { c.dispose() }
  })
  it('retains the selected Screen last frame when capture fails and clears the error on recovery', async () => {
    const { controller: c, transport: t, tab } = setup()
    try {
      await flush(); await c.selectInstance(tab, A); await flush()
      const oldFrame = tab.shot?.src
      expect(oldFrame).toBeTruthy()
      t.screenshot = vi.fn(async () => { throw new Error('capture failed') })
      c.manualCapture(); await flush()
      expect(tab.shot?.src).toBe(oldFrame)
      expect(tab.shotError).toContain('capture failed')
      t.screenshot = vi.fn(async () => ({ ...t.shot, base64: 'bmV3' }))
      c.manualCapture(); await flush()
      expect(tab.shotError).toBe('')
      expect(tab.shot?.src).not.toBe(oldFrame)
      await c.selectInstance(tab, B)
      expect(tab.instanceDesktop?.id).toBe(B.id)
    } finally { c.dispose() }
  })
  it('keeps the newer selection busy until its own enable finishes', async () => {
    const { controller: c, transport: t, tab } = setup()
    const a = deferred<void>(), b = deferred<void>(); let count = 0
    t.desktop = vi.fn(async () => { await (++count === 1 ? a.promise : b.promise); return { enabled: true, message: '' } })
    try {
      await flush(); const first = c.selectInstance(tab, A); await flush()
      const second = c.selectInstance(tab, B); await flush()
      a.resolve(); await first
      expect(tab.desktopBusy).toBe(true)
      b.resolve(); await second; await flush()
      expect(tab.instanceDesktop?.id).toBe(B.id); expect(tab.desktopBusy).toBe(false)
    } finally { a.resolve(); b.resolve(); c.dispose() }
  })
  it('rejects a valid image tagged with a different Screen identity', async () => {
    const { controller: c, transport: t, tab } = setup()
    try {
      await flush(); await c.selectInstance(tab, A); await flush()
      tab.shot = null; t.shot = { ...t.shot, desktopId: B.id }
      c.manualCapture(); await flush()
      expect(tab.shot).toBeNull()
    } finally { c.dispose() }
  })

  it('does not resurrect a tab from a late selection response', async () => {
    const { controller: c, transport: t, tab } = setup()
    try {
      await flush(); const gate = deferred<InstanceSelection>()
      t.instanceSelect = vi.fn(() => gate.promise)
      const pending = c.selectInstance(tab, A)
      await c.closeTab(tab); gate.resolve({ desktop: A, toolReady: true }); await pending
      expect(tab.instanceDesktop ?? null).toBeNull(); expect(t.desktopCalls).toHaveLength(0)
    } finally { c.dispose() }
  })
  it('does not require an App-launched coding CLI for a selected Screen', async () => {
    const { controller: c, transport: t, tab } = setup()
    try {
      await flush(); await c.selectInstance(tab, A); await flush()
      const term = tab.term as FakeTerm; const focused = term.focusCount
      c.launchCodingCli(tab); await flush()
      expect(t.instanceLaunch).not.toHaveBeenCalled()
      expect(t.sessions.get(tab.sessionId!)!.writes).toEqual([])
      expect(term.focusCount).toBe(focused); expect(tab.instanceToolReady).toBe(false)
    } finally { c.dispose() }
  })
})
