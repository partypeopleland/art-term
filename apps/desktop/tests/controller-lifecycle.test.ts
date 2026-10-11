import { describe, expect, it } from 'vitest'
import { codingCliInput, createAppController } from '../src/state/controller'
import { FakeTerm, FakeTransport, deferred, makeHost, makeTarget } from './fake'

async function flush(): Promise<void> {
  for (let i = 0; i < 30; i++) await Promise.resolve()
}

const host = makeHost('test-host', [makeTarget('test-target', 'test-host', 'host')])
const group = { dto: host, expanded: true }

it('rejects CLI control characters and excessive input before remote write', () => {
  const cli = { id: 'assistant', name: 'Assistant', command: 'assistant', args: ['--model', 'a b'] }
  expect(codingCliInput(cli)).toBe("'assistant' '--model' 'a b'\r")
  expect(codingCliInput({ ...cli, args: ['bad\narg'] })).toBeNull()
  expect(codingCliInput({ ...cli, command: 'bad\rcommand' })).toBeNull()
  expect(codingCliInput({ ...cli, args: Array(33).fill('x') })).toBeNull()
  expect(codingCliInput({ ...cli, args: ['x'.repeat(1025)] })).toBeNull()
})

describe('mobile lifecycle in shared controller', () => {
  it('resumes a background-lost session on its original target once, without replay or launching Codex', async () => {
    const transport = new FakeTransport()
    const controller = createAppController(transport, { backgroundConnection: 'keep' })
    try {
      await controller.loadCatalog(); controller.openTarget(group, host.targets[0], true)
      const tab = controller.tabs[0]; controller.attachTerminal(tab, new FakeTerm()); await flush()
      const first = tab.sessionId!
      expect(tab.desktopEnabled).toBe(true)
      await controller.setHidden(true)
      expect(tab.shot).toBeNull()
      expect(transport.desktopCalls).toContainEqual({ sid: first, enabled: false })
      controller.sendInput(tab, 'must-not-replay\r')
      transport.feed(first, { bytes: [], exitCode: 255 }); await flush()
      await controller.setHidden(false); await flush()
      expect(transport.openCalls.map(c => c.targetId)).toEqual([host.targets[0].id, host.targets[0].id])
      expect(tab.status).toBe('live'); expect(tab.sessionId).not.toBe(first)
      expect(tab.desktopEnabled).toBe(true)
      expect(transport.sessions.get(tab.sessionId!)!.writes).toEqual([])
      transport.feed(tab.sessionId!, { bytes: [], exitCode: 255 }); await flush()
      expect(transport.openCalls).toHaveLength(2)
      await controller.closeTab(tab)
    } finally { controller.dispose() }
  })
  it('resumes a loss reported just after foregrounding and leaves healthy connections intact', async () => {
    const transport = new FakeTransport()
    const controller = createAppController(transport, { backgroundConnection: 'keep' })
    try {
      await controller.loadCatalog(); controller.openTarget(group, host.targets[0], false)
      const tab = controller.tabs[0]; controller.attachTerminal(tab, new FakeTerm()); await flush()
      const first = tab.sessionId!
      await controller.setHidden(true); await controller.setHidden(false)
      expect(transport.openCalls).toHaveLength(1)
      transport.feed(first, { bytes: [], exitCode: 255 }); await flush()
      expect(transport.openCalls).toHaveLength(2)
      await controller.closeTab(tab)
    } finally { controller.dispose() }
  })
  it('never automatically resumes an explicitly detached mobile tab', async () => {
    const transport = new FakeTransport()
    const controller = createAppController(transport, { backgroundConnection: 'keep' })
    try {
      await controller.loadCatalog(); controller.openTarget(group, host.targets[0], false)
      const tab = controller.tabs[0]; controller.attachTerminal(tab, new FakeTerm()); await flush()
      await controller.setHidden(true); await controller.detachTab(tab); await controller.setHidden(false); await flush()
      expect(transport.openCalls).toHaveLength(1); expect(tab.status).toBe('exited')
      await controller.closeTab(tab)
    } finally { controller.dispose() }
  })
  it('keep policy accepts an in-flight open after backgrounding without reopening', async () => {
    const transport = new FakeTransport()
    const gate = deferred<void>()
    const originalOpen = transport.open.bind(transport)
    transport.open = async (...args) => { await gate.promise; return originalOpen(...args) }
    const controller = createAppController(transport, { backgroundConnection: 'keep' })
    try {
      await controller.loadCatalog()
      controller.openTarget(group, host.targets[0], false)
      const tab = controller.tabs[0]
      controller.attachTerminal(tab, new FakeTerm())
      await controller.setHidden(true)
      gate.resolve()
      await flush()
      expect(tab.status).toBe('live')
      expect(tab.sessionId).toBe('s1')
      expect(transport.closeCalls).toEqual([])
      await controller.setHidden(false)
      expect(transport.openCalls).toHaveLength(1)
      await controller.closeTab(tab)
      expect(transport.closeCalls).toEqual(['s1'])
    } finally { controller.dispose() }
  })
  it('does not implicitly connect when a detached pane mounts after foregrounding', async () => {
    const transport = new FakeTransport()
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      controller.openTarget(group, host.targets[0], false)
      const tab = controller.tabs[0]
      await controller.setHidden(true)
      await controller.setHidden(false)
      controller.attachTerminal(tab, new FakeTerm())
      await flush()
      expect(tab.status).toBe('exited')
      expect(transport.openCalls).toEqual([])
      await controller.closeTab(tab)
    } finally { controller.dispose() }
  })
  it('detaches every local connection while retaining tabs and requires manual reconnect', async () => {
    const transport = new FakeTransport()
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      controller.openTarget(group, host.targets[0], false)
      const tab = controller.tabs[0]
      const term = new FakeTerm()
      controller.attachTerminal(tab, term)
      await flush()
      const first = tab.sessionId!
      await controller.setHidden(true)
      expect(transport.closeCalls).toEqual([first])
      expect(controller.tabs).toHaveLength(1)
      expect(term.disposed).toBe(false)
      expect(tab.status).toBe('exited')
      term.emitData('must-not-replay\r')
      controller.reopenTab(tab, false)
      expect(transport.openCalls).toHaveLength(1)
      await controller.setHidden(false)
      expect(transport.openCalls).toHaveLength(1)
      controller.reopenTab(tab, false)
      await flush()
      expect(tab.sessionId).not.toBe(first)
      expect(transport.sessions.get(tab.sessionId!)!.writes).toEqual([])
      await controller.closeTab(tab)
    } finally { controller.dispose() }
  })

  it('rejects a late open after a background/foreground cycle', async () => {
    const transport = new FakeTransport()
    const gate = deferred<void>()
    const originalOpen = transport.open.bind(transport)
    transport.open = async (...args) => { await gate.promise; return originalOpen(...args) }
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      controller.openTarget(group, host.targets[0], false)
      const tab = controller.tabs[0]
      controller.attachTerminal(tab, new FakeTerm())
      await controller.setHidden(true)
      await controller.setHidden(false)
      gate.resolve()
      await flush()
      expect(tab.sessionId).toBeNull()
      expect(tab.status).toBe('exited')
      expect(transport.closeCalls).toEqual(['s1'])
      await controller.closeTab(tab)
    } finally { controller.dispose() }
  })

  it('discards unsent paste when an in-flight write fails across detach and reconnect', async () => {
    const transport = new FakeTransport()
    const gate = deferred<void>()
    const originalWrite = transport.write.bind(transport)
    let blocked = true
    transport.write = async (sid, bytes) => {
      if (blocked) { await gate.promise; throw new Error('old connection failed') }
      return originalWrite(sid, bytes)
    }
    const controller = createAppController(transport)
    try {
      await controller.loadCatalog()
      controller.openTarget(group, host.targets[0], false)
      const tab = controller.tabs[0]
      const term = new FakeTerm()
      controller.attachTerminal(tab, term)
      await flush()
      term.emitData('x'.repeat(40_000) + '\r')
      await controller.detachTab(tab)
      controller.reopenTab(tab, false)
      await flush()
      blocked = false
      term.emitData('fresh\r')
      gate.resolve()
      await flush()
      expect(tab.writeBuf).toEqual([])
      expect(tab.errorMsg).toBe('')
      expect(transport.sessions.get(tab.sessionId!)!.writes.flat()).toEqual(Array.from(new TextEncoder().encode('fresh\r')))
      await controller.closeTab(tab)
    } finally { controller.dispose() }
  })
})
