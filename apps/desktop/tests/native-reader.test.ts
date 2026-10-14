import { afterEach, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import NativeTerminalReader from '../src/components/NativeTerminalReader.vue'
import { NativeHistoryKey, type NativeHistoryEvent, type NativeHistorySnapshot } from '../src/terminal/native'
import type { TerminalHandle } from '../src/terminal/handle'
vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => { wrappers.splice(0).forEach(w => w.unmount()); vi.useRealTimers() })

function fixture(reject = false) {
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
  const wrapper = mount(NativeTerminalReader, { props: { terminal, visible: true }, global: { provide: { [NativeHistoryKey as symbol]: () => bridge } } })
  vi.spyOn(wrapper.element, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, width: 300, height: 500 } as DOMRect)
  wrappers.push(wrapper)
  return { wrapper, terminal, bridge, emit: (kind: NativeHistoryEvent['kind'], id = bridge.open.mock.calls[0][0].readerId) => receive({ readerId: id, kind }), records, unsupported: () => { supported = false } }
}

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
