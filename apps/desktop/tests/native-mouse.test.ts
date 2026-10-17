import { mount, flushPromises } from '@vue/test-utils'
import { defineComponent, ref } from 'vue'
import { afterEach, expect, it, vi } from 'vitest'
import { useNativeMouse } from '../src/terminal/native-mouse'
import type { HostHistoryRequest, HostHistoryReply } from '../src/terminal/native'
import { deferred } from './fake'

const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => { wrappers.splice(0).forEach(wrapper => wrapper.unmount()); vi.useRealTimers() })
function setup(request = vi.fn(async (r: HostHistoryRequest): Promise<HostHistoryReply> => ({ kind: 'ack', instance: 'a'.repeat(32), op: r.op }))) {
  const enabled = ref(true), notice = vi.fn()
  const wrapper = mount(defineComponent({ setup() { useNativeMouse(() => enabled.value, () => 'session', () => request, notice); return () => null } }))
  wrappers.push(wrapper)
  return { wrapper, enabled, request, notice }
}
it('only leases native mouse control and never polls displayed text', async () => {
  vi.useFakeTimers()
  const f = setup(); await flushPromises()
  await vi.advanceTimersByTimeAsync(59000)
  expect(f.request.mock.calls.map(([r]) => r)).toEqual([{ op: 'terminalMouse', enabled: true }])
  await vi.advanceTimersByTimeAsync(1000); await flushPromises()
  expect(f.request.mock.calls.map(([r]) => r)).toEqual([{ op: 'terminalMouse', enabled: true }, { op: 'terminalMouse', enabled: true }])
  f.enabled.value = false; await flushPromises()
  expect(f.request.mock.calls.at(-1)?.[0]).toEqual({ op: 'close' })
})
it('serializes close after a pending enable and never revives an inactive owner', async () => {
  const pending = deferred<HostHistoryReply>()
  const request = vi.fn(async (r: HostHistoryRequest): Promise<HostHistoryReply> => r.op === 'terminalMouse' ? pending.promise : { kind: 'ack', instance: 'a'.repeat(32), op: r.op })
  const f = setup(request); await flushPromises()
  f.enabled.value = false; await flushPromises()
  expect(request).toHaveBeenCalledTimes(1)
  pending.resolve({ kind: 'ack', instance: 'a'.repeat(32), op: 'terminalMouse' }); await flushPromises()
  expect(request.mock.calls.map(([r]) => r.op)).toEqual(['terminalMouse', 'close'])
  expect(f.notice).not.toHaveBeenCalled()
})
it('keeps unsupported helpers out of the output path without reverting to a text overlay', async () => {
  const request = vi.fn(async (): Promise<HostHistoryReply> => ({ kind: 'unsupported' }))
  const f = setup(request); await flushPromises()
  expect(f.notice).toHaveBeenCalledWith(expect.stringContaining('更新 tmux helper'))
  expect(request).toHaveBeenCalledTimes(1)
})


it('releases a still-selected TerminalPane lease when props.active hides it under settings or a picker', async () => {
  vi.useFakeTimers()
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  const { default: TerminalPane } = await import('../src/components/TerminalPane.vue')
  const { TerminalFactoryKey } = await import('../src/terminal/handle')
  const { FakeTerm } = await import('./fake')
  const { reactive } = await import('vue')
  const term = new FakeTerm()
  const tab = reactive({ key: 'selected-tab', sessionId: 'session', status: 'live', runtime: {}, closed: false, term: null })
  const request = vi.fn(async (_tab: unknown, r: HostHistoryRequest): Promise<HostHistoryReply> => ({ kind: 'ack', instance: 'a'.repeat(32), op: r.op }))
  const controller = { activeKey: ref('selected-tab'), historyRequest: request, requestResize: vi.fn(), attachTerminal: vi.fn() }
  const wrapper = mount(TerminalPane, {
    props: { tab: tab as never, controller: controller as never, active: true, nativeScroll: true },
    global: { provide: { [TerminalFactoryKey as symbol]: () => term } }
  })
  wrappers.push(wrapper)
  await flushPromises()
  expect(request.mock.calls.map(([, r]) => r.op)).toEqual(['terminalMouse'])
  await wrapper.setProps({ active: false }); await flushPromises()
  expect(controller.activeKey.value).toBe('selected-tab')
  expect(request.mock.calls.map(([, r]) => r.op)).toEqual(['terminalMouse', 'close'])
  await vi.advanceTimersByTimeAsync(120000); await flushPromises()
  expect(request.mock.calls.map(([, r]) => r.op)).toEqual(['terminalMouse', 'close'])
  await wrapper.setProps({ active: true }); await flushPromises()
  expect(request.mock.calls.map(([, r]) => r.op)).toEqual(['terminalMouse', 'close', 'terminalMouse'])
})
