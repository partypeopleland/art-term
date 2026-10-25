import { afterEach, expect, it, vi } from 'vitest'
import { HostTerminalViewport, type HostHistoryReply, type HostHistoryRequest } from '../src/shared/terminal/native'
import { deferred } from './fake'
afterEach(() => vi.useRealTimers())
const view = (held = false, available = true): HostHistoryReply => ({ kind: 'view', instance: 'a', op: 'scroll', held, available, offset: held ? 2 : 0 })
const flush = async () => { for (let i = 0; i < 15; i++) await Promise.resolve() }
it('coalesces fractional wheel deltas while prewarm is in flight, bounded to 256 lines', async () => {
  const pending = deferred<HostHistoryReply>()
  const calls: HostHistoryRequest[] = []
  const request = vi.fn(async (r: HostHistoryRequest) => { calls.push(r); return calls.length === 1 ? pending.promise : view(true) })
  const host = new HostTerminalViewport(request)
  host.start(); host.scroll(-.4); host.scroll(-.6); host.scroll(-500)
  expect(calls).toEqual([{ op: 'scroll', lines: 0 }])
  pending.resolve(view()); await flush()
  expect(calls[1]).toEqual({ op: 'scroll', lines: -256 })
  await host.close()
})
it('retries busy but treats inner alternate unavailability as transient', async () => {
  vi.useFakeTimers()
  const request = vi.fn(async (): Promise<HostHistoryReply> => ({ kind: 'error', error: 'busy' }))
  const host = new HostTerminalViewport(request)
  host.start(); await flush()
  request.mockImplementation(async () => view(false, false))
  await vi.advanceTimersByTimeAsync(150); await flush()
  expect(host.unsupported).toBe(false); expect(host.available).toBe(false)
  request.mockImplementation(async () => view(true))
  host.scroll(-2); await flush(); expect(host.held).toBe(true)
  await host.close()
})
it('closes immediately to cancel pending control, and waits before reopening another session', async () => {
  const pending = deferred<HostHistoryReply>(), closed = deferred<HostHistoryReply>()
  const calls: HostHistoryRequest[] = []
  const host = new HostTerminalViewport(async r => { calls.push(r); return r.op === 'close' ? closed.promise : pending.promise })
  host.start(); host.scroll(-2)
  const finish = vi.fn(); const closing = host.close().then(finish)
  expect(calls.map(r => r.op)).toEqual(['scroll', 'close'])
  pending.resolve(view(true)); await flush(); expect(finish).not.toHaveBeenCalled()
  closed.resolve({ kind: 'ack', instance: 'a', op: 'close' }); await closing
  expect(host.held).toBe(false); expect(calls).toHaveLength(2)
})
it.each(['direct', 'error'])('permanently stops and releases old capability via %s wire form', async form => {
  const request = vi.fn(async (): Promise<HostHistoryReply> => form === 'direct' ? { kind: 'unsupported' } : { kind: 'error', error: 'unsupported' })
  const host = new HostTerminalViewport(request)
  host.start(); await flush(); host.scroll(-2); await flush()
  expect(request).toHaveBeenCalledTimes(2); expect(request.mock.calls[1]).toEqual([{ op: 'close' }]); expect(host.unsupported).toBe(true)
  await host.close()
})


it('retains sub-row wheel fractions and lets opposite tiny gestures cancel', async () => {
  const requests: HostHistoryRequest[] = []
  const host = new HostTerminalViewport(async request => { requests.push(request); return view(true) })
  host.start(); await flush()
  host.scroll(-.4); host.scroll(.4); await flush()
  expect(requests).toHaveLength(1)
  host.scroll(-34 / 20); await flush()
  expect(requests[1]).toEqual({ op: 'scroll', lines: -1 })
  host.scroll(-.35); await flush()
  expect(requests[2]).toEqual({ op: 'scroll', lines: -1 })
  await host.close()
})
