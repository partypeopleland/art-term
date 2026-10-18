import { afterEach, describe, expect, it, vi } from 'vitest'
import { createAppController, type AppController } from '../src/state/controller'
import type { OutputDto, RuntimeOutputEvent, SessionDto } from '../src/transport/types'
import { FakeTerm, FakeTransport, deferred, makeHost, makeTarget } from './fake'

const target = makeTarget('runtime-issued-target', 'runtime-host', 'host')
const host = makeHost('runtime-host', [target])
const controllers: AppController[] = []
async function flush(): Promise<void> { for (let i = 0; i < 50; i++) await Promise.resolve() }
class RuntimeTransport extends FakeTransport {
  override runtimeAck = vi.fn(async (_session: string, _batch: number, _generation: number) => {})
  override runtimeRecover = vi.fn(async (_session: string) => {})
}
class RuntimeTerm extends FakeTerm {
  events: string[] = []
  historical: (boolean | undefined)[] = []
  override reset(): void { this.events.push('reset'); super.reset() }
  override async write(bytes: Uint8Array, historical?: boolean): Promise<void> {
    this.events.push(`write:${new TextDecoder().decode(bytes)}`)
    this.historical.push(historical)
    await super.write(bytes)
    this.events.push('parsed')
  }
}
async function setup(keep = false) {
  const transport = new RuntimeTransport()
  const controller = createAppController(transport, { idlePollMs: 0, ...(keep ? { backgroundConnection: 'keep' as const } : {}) })
  controllers.push(controller)
  await controller.loadCatalog()
  controller.openTarget({ dto: host, expanded: true }, target, false)
  const tab = controller.tabs[0]
  const term = new RuntimeTerm()
  controller.attachTerminal(tab, term)
  await flush()
  return { transport, controller, tab, term, session: tab.sessionId! }
}
function output(events: RuntimeOutputEvent[] = [], extras: Partial<NonNullable<OutputDto['runtime']>> = {}): OutputDto {
  return { bytes: [], exitCode: null, runtime: { batchId: 71, localGeneration: 9, events, ...extras } }
}
const historical: RuntimeOutputEvent = { kind: 'bytes', bytes: [0xe6, 0xbc, 0xa2], start: 13, end: 16, historical: true }
afterEach(() => { for (const controller of controllers.splice(0)) controller.dispose(); vi.restoreAllMocks() })

describe('ordered runtime rendering and acknowledgement', () => {
  it('resets Gap before historical parsing and ACKs exact batch/generation only after callback', async () => {
    const { transport, tab, term, session } = await setup()
    term.holdWrites = true
    tab.writeBuf = [0x78]; tab.pendingSize = { rows: 22, cols: 80 }
    transport.feed(session, output([{ kind: 'gap', bytesDropped: 13, resumeOffset: 13 }, historical]))
    await flush()
    expect(term.events).toEqual(['reset', 'write:漢'])
    expect(term.historical).toEqual([true])
    expect(tab.writeBuf).toEqual([]); expect(tab.pendingSize).toBeNull()
    expect(transport.runtimeAck).not.toHaveBeenCalled()
    expect(transport.sessions.get(session)!.pollCount).toBe(1)
    term.releaseWrites(); await flush()
    expect(term.events).toEqual(['reset', 'write:漢', 'parsed'])
    expect(transport.runtimeAck).toHaveBeenCalledExactlyOnceWith(session, 71, 9)
    expect(transport.sessions.get(session)!.pollCount).toBe(2)
  })

  it('preserves event ordering across multiple historical/live chunks', async () => {
    const { transport, term, session } = await setup()
    transport.feed(session, output([
      historical,
      { kind: 'bytes', bytes: [65], start: 16, end: 17, historical: false },
      { kind: 'gap', bytesDropped: 3, resumeOffset: 20 },
      { kind: 'bytes', bytes: [66], start: 20, end: 21, historical: true }
    ]))
    await flush()
    expect(term.events).toEqual(['write:漢', 'parsed', 'write:A', 'parsed', 'reset', 'write:B', 'parsed'])
    expect(term.historical).toEqual([true, false, true])
    expect(transport.runtimeAck).toHaveBeenCalledExactlyOnceWith(session, 71, 9)
  })

  it.each(['close', 'detach', 'dispose'] as const)('does not ACK a late renderer callback after %s', async (action) => {
    const { transport, controller, tab, term, session } = await setup()
    term.holdWrites = true; transport.feed(session, output([historical])); await flush()
    if (action === 'close') await controller.closeTab(tab)
    else if (action === 'detach') await controller.detachTab(tab)
    else controller.dispose()
    term.releaseWrites(); await flush()
    expect(transport.runtimeAck).not.toHaveBeenCalled()
    expect(transport.closeCalls).toContain(session)
  })

  it('fails closed on ACK rejection and never replays queued input or reopens', async () => {
    const { transport, tab, term, session } = await setup()
    transport.runtimeAck.mockRejectedValueOnce(new Error('fixture ACK failure'))
    tab.writeBuf = Array.from(new TextEncoder().encode('uncertain-command\r'))
    transport.feed(session, output([historical])); await flush()
    expect(term.text()).toBe('漢'); expect(tab.status).toBe('error'); expect(tab.sessionId).toBeNull()
    expect(tab.writeBuf).toEqual([]); expect(transport.closeCalls).toEqual([session])
    expect(transport.sessions.get(session)!.writes).toEqual([])
    expect(transport.openCalls).toHaveLength(1); expect(transport.runtimeRecover).not.toHaveBeenCalled()
  })

  it('does not ACK empty batchId 0, while notice/TTL/uncertain flags remain visible', async () => {
    const { transport, tab, session } = await setup()
    transport.feed(session, output([], { batchId: 0 })); await flush()
    expect(transport.runtimeAck).not.toHaveBeenCalled()
    transport.feed(session, output([{ kind: 'notice', ephemeralWorkspace: true, teardownBestEffort: true }], { batchId: 72 })); await flush()
    expect(tab.notice).toContain('消失')
    transport.feed(session, output([{ kind: 'ttlWarning', secondsRemaining: 30, absolute: false }], { batchId: 73 })); await flush()
    expect(tab.notice).toContain('30 秒')
    tab.writeBuf = [65]
    transport.feed(session, output([], { batchId: 74, inputUncertain: true })); await flush()
    expect(tab.writeBuf).toEqual([]); expect(tab.notice).toContain('未自動重送')
  })
})

describe('runtime recovery keeps the original binding and emulator', () => {
  it('can recover later independent drops after healthy live output was rendered and ACKed', async () => {
    let now = 0
    vi.spyOn(Date, 'now').mockImplementation(() => now)
    const { transport, tab, term, session } = await setup()
    for (let attempt = 1; attempt <= 4; attempt++) {
      transport.feed(session, output([], { batchId: 0, ended: 'slowConsumer' })); await flush()
      expect(tab.sessionId).toBe(session)
      expect(tab.status).toBe('live')
      expect(transport.runtimeRecover).toHaveBeenCalledTimes(attempt)
      now += 31000
      transport.feed(session, output([{ kind: 'bytes', bytes: [64 + attempt], start: attempt - 1, end: attempt, historical: false }], { batchId: attempt })); await flush()
    }
    expect(term.text()).toBe('ABCD')
    expect(term.resetCount).toBe(0)
    expect(transport.openCalls).toHaveLength(1)
    expect(transport.sessions.get(session)!.writes).toEqual([])
  })
  it.each(['ttlExpired', 'takeover', 'operatorKill', 'sessionEnded', 'replacement', 'capacity', 'protocol'])('does not automatically recover typed %s termination', async (reason) => {
    const { transport, tab, session } = await setup()
    transport.feed(session, output([], { batchId: 0, ended: reason })); await flush()
    expect(tab.status).toBe('error'); expect(tab.sessionId).toBeNull()
    expect(transport.runtimeRecover).not.toHaveBeenCalled(); expect(transport.openCalls).toHaveLength(1)
    expect(transport.closeCalls).toEqual([session])
  })

  it.each(['transport', 'slowConsumer'])('recovers %s on the same id/emulator and stops after three recovery attempts', async (reason) => {
    const { transport, tab, term, session } = await setup()
    transport.feed(session, output([historical])); await flush()
    const writes = transport.sessions.get(session)!.writes
    for (let attempt = 1; attempt <= 3; attempt++) {
      tab.writeBuf = [0x78]
      transport.feed(session, output([], { batchId: 0, ended: attempt === 2 ? 'timeout' : reason, inputUncertain: true })); await flush()
      expect(transport.runtimeRecover).toHaveBeenCalledTimes(attempt)
      expect(tab.sessionId).toBe(session); expect(tab.term).toBe(term); expect(tab.status).toBe('live')
      expect(tab.writeBuf).toEqual([]); expect(term.resetCount).toBe(0); expect(term.text()).toBe('漢')
    }
    transport.feed(session, output([], { batchId: 0, ended: reason })); await flush()
    expect(transport.runtimeRecover.mock.calls).toEqual([[session], [session], [session]])
    expect(transport.openCalls).toHaveLength(1); expect(writes).toEqual([])
    expect(tab.status).toBe('error'); expect(transport.closeCalls).toEqual([session])
  })

  it('recovery failure requires explicit action and never creates/reopens automatically', async () => {
    const { transport, tab, term, session } = await setup()
    transport.runtimeRecover.mockRejectedValueOnce({ kind: 'replacement', message: '' })
    transport.feed(session, output([], { batchId: 0, ended: 'transport' })); await flush()
    expect(tab.status).toBe('error'); expect(tab.sessionId).toBeNull()
    expect(transport.runtimeRecover).toHaveBeenCalledExactlyOnceWith(session)
    expect(transport.openCalls).toHaveLength(1); expect(term.resetCount).toBe(0)
  })

  it('mobile keep policy defers recovery while hidden and resumes same binding on foreground', async () => {
    const { transport, controller, tab, term, session } = await setup(true)
    transport.feed(session, output([historical])); await flush()
    await controller.setHidden(true); controller.sendInput(tab, 'background-never-replay\r')
    transport.feed(session, output([], { batchId: 0, ended: 'transport' })); await flush()
    expect(transport.runtimeRecover).not.toHaveBeenCalled()
    await controller.setHidden(false); await flush()
    expect(transport.runtimeRecover).toHaveBeenCalledExactlyOnceWith(session)
    expect(tab.sessionId).toBe(session); expect(tab.term).toBe(term); expect(term.text()).toBe('漢')
    expect(term.resetCount).toBe(0); expect(transport.openCalls).toHaveLength(1)
    expect(transport.sessions.get(session)!.writes).toEqual([])
  })

  it('closed tab invalidates a late successful recovery', async () => {
    const { transport, controller, tab, session } = await setup()
    const pending = deferred<void>(); transport.runtimeRecover.mockReturnValueOnce(pending.promise)
    transport.feed(session, output([], { batchId: 0, ended: 'transport' })); await flush()
    await controller.closeTab(tab); pending.resolve(); await flush()
    expect(controller.tabs).toHaveLength(0); expect(tab.sessionId).toBeNull()
    expect(transport.openCalls).toHaveLength(1)
  })
  it('restores native preview only after background recovery reattaches the work handle', async () => {
    const { transport, controller, tab, session } = await setup(true)
    await controller.setDesktop(tab, true); await flush()
    await controller.setHidden(true)
    expect(transport.desktopCalls.at(-1)).toEqual({ sid: session, enabled: false })
    transport.desktopCalls = []
    const pending = deferred<void>()
    transport.runtimeRecover.mockReturnValueOnce(pending.promise)
    transport.feed(session, output([], { batchId: 0, ended: 'transport' })); await flush()
    await controller.setHidden(false); await flush()
    expect(transport.desktopCalls).toEqual([])
    pending.resolve(); await flush()
    expect(transport.desktopCalls).toEqual([{ sid: session, enabled: true }])
    expect(tab.sessionId).toBe(session)
    expect(tab.status).toBe('live')
    expect(transport.openCalls).toHaveLength(1)
  })
})


it('forwards generated terminal replies independently while ordinary user FIFO is held', async () => {
  const { transport, controller, tab, term } = await setup()
  const pending = deferred<void>()
  const writes: { text: string; reply: boolean }[] = []
  transport.write = async (_sid, bytes, terminalReply) => {
    writes.push({ text: new TextDecoder().decode(Uint8Array.from(bytes)), reply: !!terminalReply })
    if (!terminalReply) await pending.promise
  }
  controller.sendInput(tab, 'user-one')
  controller.sendInput(tab, 'user-two')
  term.emitData('\x1b[1;1R', true)
  await flush()
  expect(writes).toEqual([{ text: 'user-one', reply: false }, { text: '\x1b[1;1R', reply: true }])
  pending.resolve(); await flush()
  expect(writes.at(-1)).toEqual({ text: 'user-two', reply: false })
})
