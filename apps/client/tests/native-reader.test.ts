import { afterEach, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import NativeTerminalReader from '../src/mobile/NativeTerminalReader.vue'
import { NativeHistoryKey, type NativeHistoryEvent, type NativeHistorySnapshot } from '../src/shared/terminal/native'
import type { TerminalHandle } from '../src/shared/terminal/handle'
import { Terminal } from '@xterm/xterm'
import { createNativeBufferProjection } from '../src/shared/terminal/native-buffer'
import { createMobileInput } from '../src/mobile/input'
vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => { wrappers.splice(0).forEach(w => w.unmount()); vi.useRealTimers() })

function fixture(reject = false, historyRequest?: (value: Record<string, unknown>) => Promise<unknown>) {
  let receive!: (event: NativeHistoryEvent) => void
  let supported = true
  const records = [{ id: 1, text: 'existing', styles: [] }]
  const terminal: TerminalHandle = {
    rows: 24, cols: 80, readStyledText: () => ({ records, supported }), setNativeReaderActive: vi.fn(),
    write: vi.fn(async () => {}), onData: vi.fn(), setKeyInterceptor: vi.fn(), fit: vi.fn(), focus: vi.fn(), reset: vi.fn(), dispose: vi.fn()
  }
  const bridge = {
    open: vi.fn(async (_value: NativeHistorySnapshot, cb: (e: NativeHistoryEvent) => void) => { receive = cb; if (reject) throw Error('unavailable') }),
    update: vi.fn(async (_value: NativeHistorySnapshot) => {}), close: vi.fn(async () => {})
  }
  const wrapper = mount(NativeTerminalReader, { props: { terminal, visible: true }, attrs: { historyRequest }, global: { provide: { [NativeHistoryKey as symbol]: () => bridge } } })
  vi.spyOn(wrapper.element, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, width: 300, height: 500 } as DOMRect)
  wrappers.push(wrapper)
  return { wrapper, terminal, bridge, event: (e: Partial<NativeHistoryEvent> & { kind: NativeHistoryEvent['kind'] }) => receive({ readerId: bridge.open.mock.calls[0][0].readerId, ...e }), emit: (kind: NativeHistoryEvent['kind'], id = bridge.open.mock.calls[0][0].readerId) => receive({ readerId: id, kind }), records, unsupported: () => { supported = false }, supported: () => { supported = true } }
}

it('loads only the Host viewport until a real upward gesture requests an older page', async () => {
  vi.useFakeTimers()
  const hostRows = (start: number, end: number) => Array.from({ length: end - start }, (_, i) => ({ id: start + i + 1, text: `HOST-${start + i + 1}`, styles: [] }))
  const request = vi.fn(async (value: Record<string, unknown>) => value.op === 'viewport'
    ? { kind: 'page', instance: 'a'.repeat(32), epoch: 1, columns: 80, rows: 24, rowCount: 1000, start: 976, end: 1000, hasOlder: true, records: hostRows(976, 1000) }
    : value.op === 'older'
      ? { kind: 'page', instance: 'a'.repeat(32), epoch: 1, columns: 80, rows: 24, rowCount: 1000, start: 720, end: 976, hasOlder: true, records: hostRows(720, 976) }
      : { kind: 'ack', instance: 'a'.repeat(32), op: value.op, epoch: 1 })
  const f = fixture(false, request)
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].records).toEqual(hostRows(976, 1000))
  expect(request.mock.calls.some(([r]) => r.op === 'older')).toBe(false)
  f.event({ kind: 'viewport', sourceEpoch: `${'a'.repeat(32)}:1`, anchorId: 977, nearTop: true, atBottom: false })
  await flushPromises()
  expect(request.mock.calls.find(([r]) => r.op === 'older')?.[0]).toMatchObject({ instance: 'a'.repeat(32), epoch: 1, before: 976, limit: 256 })
  expect(f.bridge.update.mock.calls.at(-1)![0].records).toEqual(hostRows(720, 1000))
  expect(f.bridge.update.mock.calls.at(-1)![0].followLatest).toBe(false)
  expect(f.terminal.write).not.toHaveBeenCalled()
})

it('uses xterm during a Host alternate screen and resumes Host history when it ends', async () => {
  vi.useFakeTimers()
  let alternate = false
  const request = vi.fn(async (r: Record<string, unknown>) => r.op !== 'viewport'
    ? { kind: 'ack', instance: 'a'.repeat(32), epoch: 1, op: r.op }
    : alternate ? { kind: 'error', error: 'unsupported' }
      : { kind: 'page', instance: 'a'.repeat(32), epoch: 1, columns: 80, rows: 1, rowCount: 1, start: 0, end: 1, hasOlder: false, records: [{ id: 1, text: 'Host normal screen', styles: [] }] })
  const f = fixture(false, request)
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].visible).toBe(true)
  alternate = true; f.unsupported()
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].visible).toBe(false)
  expect(f.terminal.setNativeReaderActive).toHaveBeenLastCalledWith(false)
  alternate = false; f.supported()
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].records[0].text).toBe('Host normal screen')
  expect(f.bridge.update.mock.calls.at(-1)![0].visible).toBe(true)
})

it('ACKs only the epoch actually drawn and keeps that epoch when a gesture races a newer candidate', async () => {
  vi.useFakeTimers()
  let current = 1
  const instance = 'a'.repeat(32)
  const rows = (start: number, end: number) => Array.from({ length: end - start }, (_, i) => ({ id: start + i + 1, text: `ROW-${start + i + 1}`, styles: [] }))
  const request = vi.fn(async (r: Record<string, unknown>) => {
    const epoch = r.op === 'viewport' ? current : Number(r.epoch)
    const rowCount = epoch === 1 ? 1000 : 1200
    if (r.op === 'viewport' || r.op === 'older') {
      const end = r.op === 'older' ? Number(r.before) : rowCount
      const start = end - (r.op === 'older' ? Number(r.limit) : 24)
      return { kind: 'page', instance, epoch, columns: 80, rows: 24, rowCount, start, end, hasOlder: true, records: rows(start, end) }
    }
    return { kind: 'ack', instance, epoch, op: r.op }
  })
  const f = fixture(false, request)
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  expect(request.mock.calls.some(([r]) => r.op === 'ack')).toBe(false)
  f.event({ kind: 'position', sourceEpoch: `${instance}:1` }); await flushPromises()
  expect(request.mock.calls.find(([r]) => r.op === 'ack')?.[0]).toMatchObject({ instance, epoch: 1 })
  current = 2
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].sourceEpoch).toBe(`${instance}:2`)
  expect(request.mock.calls.some(([r]) => r.op === 'ack' && r.epoch === 2)).toBe(false)
  f.event({ kind: 'reading', sourceEpoch: `${instance}:1` }); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].sourceEpoch).toBe(`${instance}:1`)
  expect(f.bridge.update.mock.calls.at(-1)![0].records).toEqual(rows(976, 1000))
  f.event({ kind: 'position', sourceEpoch: `${instance}:2` }); await flushPromises()
  expect(request.mock.calls.some(([r]) => r.op === 'ack' && r.epoch === 2)).toBe(false)
  f.event({ kind: 'viewport', sourceEpoch: `${instance}:1`, anchorId: 977, nearTop: true, atBottom: false }); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].records).toEqual(rows(720, 1000))
})

it('forgets expired offered epochs even when their display ACK failed', async () => {
  vi.useFakeTimers()
  const instance = 'a'.repeat(32)
  let epoch = 1
  const request = vi.fn(async (r: Record<string, unknown>) => r.op === 'viewport'
    ? { kind: 'page', instance, epoch, columns: 80, rows: 1, rowCount: 1, start: 0, end: 1, hasOlder: false, records: [{ id: 1, text: `epoch-${epoch}`, styles: [] }] }
    : { kind: 'error', error: 'busy' })
  const f = fixture(false, request)
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  f.event({ kind: 'position', sourceEpoch: `${instance}:1` }); await flushPromises()
  for (const next of [2, 3]) {
    epoch = next
    await vi.advanceTimersByTimeAsync(121000); await flushPromises()
    f.event({ kind: 'position', sourceEpoch: `${instance}:${epoch}` }); await flushPromises()
  }
  epoch = 4
  await vi.advanceTimersByTimeAsync(121000); await flushPromises()
  f.event({ kind: 'reading', sourceEpoch: `${instance}:2` }); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].sourceEpoch).toBe(`${instance}:4`)
  // The currently drawn epoch remains available while its newer candidate waits.
  f.event({ kind: 'reading', sourceEpoch: `${instance}:3` }); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].sourceEpoch).toBe(`${instance}:3`)
})

it('waits for the first Host viewport rather than displaying a different local page during a busy recovery', async () => {
  vi.useFakeTimers()
  let viewportCalls = 0
  const request = vi.fn(async (r: Record<string, unknown>) => r.op !== 'viewport'
    ? { kind: 'ack', instance: 'a'.repeat(32), epoch: 1, op: r.op }
    : ++viewportCalls === 1 ? { kind: 'error', error: 'busy' }
      : { kind: 'page', instance: 'a'.repeat(32), epoch: 1, columns: 80, rows: 1, rowCount: 1, start: 0, end: 1, hasOlder: false, records: [{ id: 1, text: 'Host viewport', styles: [] }] })
  const f = fixture(false, request)
  await flushPromises()
  expect(f.bridge.open).not.toHaveBeenCalled()
  await vi.advanceTimersByTimeAsync(500); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)![0].records[0].text).toBe('Host viewport')
})

it('focuses only native taps for the active reader and never sends touch as terminal input', async () => {
  vi.useFakeTimers()
  const f = fixture()
  await vi.advanceTimersByTimeAsync(250); await flushPromises()
  expect(f.terminal.setNativeReaderActive).toHaveBeenLastCalledWith(true)
  f.emit('tap', 'other-reader')
  f.emit('visibility')
  expect(f.terminal.focus).not.toHaveBeenCalled()
  f.emit('tap')
  expect(f.terminal.focus).toHaveBeenCalledTimes(1)
  expect(f.terminal.write).not.toHaveBeenCalled()
  await f.wrapper.setProps({ visible: false }); await flushPromises()
  f.emit('tap')
  expect(f.terminal.focus).toHaveBeenCalledTimes(1)
  expect(f.bridge.update.mock.calls.at(-1)?.[0]).toMatchObject({ visible: false, records: f.records })
  expect(f.terminal.setNativeReaderActive).toHaveBeenLastCalledWith(false)
})

it('updates the same native surface on append and hides it for unsupported buffers', async () => {
  vi.useFakeTimers()
  const f = fixture()
  await vi.advanceTimersByTimeAsync(250); await flushPromises()
  f.records.push({ id: 2, text: 'append', styles: [] })
  await vi.advanceTimersByTimeAsync(250); await flushPromises()
  expect(f.bridge.open).toHaveBeenCalledTimes(1)
  expect(f.bridge.update.mock.calls.at(-1)?.[0].records).toHaveLength(2)
  f.unsupported()
  await vi.advanceTimersByTimeAsync(250); await flushPromises()
  expect(f.bridge.update.mock.calls.at(-1)?.[0].visible).toBe(false)
  expect(f.terminal.setNativeReaderActive).toHaveBeenLastCalledWith(false)
  f.wrapper.unmount(); wrappers.pop(); await flushPromises()
  expect(f.bridge.close).toHaveBeenCalled()
})

it('keeps original xterm scrolling available when native open fails', async () => {
  vi.useFakeTimers()
  const f = fixture(true)
  await vi.advanceTimersByTimeAsync(250); await flushPromises()
  expect(f.terminal.setNativeReaderActive).toHaveBeenLastCalledWith(false)
  expect(f.terminal.setNativeReaderActive).not.toHaveBeenCalledWith(true)
  expect(f.bridge.close).toHaveBeenCalled()
})

it('loads older native pages only after a user viewport event and holds that page during live output', async () => {
  const term = new Terminal({ cols: 80, rows: 2, scrollback: 5000 })
  const projection = createNativeBufferProjection(term)
  const write = (value: string) => new Promise<void>(resolve => term.write(value, resolve))
  await write(Array.from({ length: 2000 }, (_, n) => `line-${n}`).join('\r\n'))
  vi.useFakeTimers()
  let receive!: (e: NativeHistoryEvent) => void
  const bridge = {
    open: vi.fn(async (_s: NativeHistorySnapshot, cb: typeof receive) => { receive = cb }),
    update: vi.fn(async (_s: NativeHistorySnapshot) => {}), close: vi.fn(async () => {})
  }
  const real: TerminalHandle = {
    cols: 80, rows: 2, readStyledText: window => projection.read(window),
    setNativeReaderActive: vi.fn(), focus: vi.fn(), fit: () => {},
    write: bytes => new Promise<void>(resolve => term.write(bytes, resolve)),
    onData: cb => { term.onData(text => cb(text)) }, setKeyInterceptor: cb => term.attachCustomKeyEventHandler(cb),
    reset: () => term.reset(), dispose: () => { projection.dispose(); term.dispose() }
  }
  const terminal = createMobileInput().wrap(document.createElement('div'), real)
  const wrapper = mount(NativeTerminalReader, { props: { terminal, visible: true }, global: { provide: { [NativeHistoryKey as symbol]: () => bridge } } })
  vi.spyOn(wrapper.element, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, width: 300, height: 500 } as DOMRect)
  wrappers.push(wrapper)
  try {
    await vi.advanceTimersByTimeAsync(1000); await flushPromises()
    const snapshot = () => bridge.update.mock.calls.at(-1)![0]
    expect(snapshot().records).toHaveLength(256)
    expect(snapshot().records[0].id).toBe(1745)
    receive({ readerId: snapshot().readerId, kind: 'position', nearTop: true })
    await flushPromises()
    expect(snapshot().records).toHaveLength(256)
    const viewport = (nearTop: boolean, nearBottom = false, atBottom = false) => receive({
      readerId: snapshot().readerId, kind: 'viewport', nearTop, nearBottom, atBottom,
      anchorId: snapshot().records[0].id
    })
    viewport(true); await flushPromises()
    expect(snapshot().records).toHaveLength(512)
    expect(snapshot().records[0].id).toBe(1489)
    expect(snapshot().followLatest).toBe(false)
    viewport(true); await flushPromises()
    viewport(true); await flushPromises()
    viewport(true); await flushPromises()
    expect(snapshot().records).toHaveLength(1024)
    expect(snapshot().records[0].id).toBe(721)
    const ids = snapshot().records.map(r => r.id)
    const pending = write('\r\nnew-live-output')
    await vi.advanceTimersByTimeAsync(1000); await pending; await flushPromises()
    expect(snapshot().records.map(r => r.id)).toEqual(ids)
    expect(snapshot().records.some(r => r.text === 'new-live-output')).toBe(false)
    viewport(false, true); await flushPromises()
    expect(snapshot().records).toHaveLength(1024)
    expect(snapshot().records[0].id).toBe(977)
  } finally { projection.dispose(); term.dispose() }
})
