import { afterEach, expect, it, vi } from 'vitest'
import { HostHistoryWindow, type HostHistoryPage, type HostHistoryReply } from '../src/shared/terminal/native'

afterEach(() => vi.useRealTimers())

const page: HostHistoryPage = {
  kind: 'page', instance: 'a'.repeat(32), epoch: 1, columns: 80, rows: 1,
  rowCount: 1, start: 0, end: 1, hasOlder: false,
  records: [{ id: 1, text: 'Host viewport', styles: [] }]
}

it('backs off temporary failures up to 30 seconds and resets the interval after recovery', async () => {
  vi.useFakeTimers(); vi.setSystemTime(0)
  const request = vi.fn(async (): Promise<HostHistoryReply> => ({ kind: 'error', error: 'unavailable' }))
  const host = new HostHistoryWindow(request)
  await host.refresh()
  for (const [attempt, time] of [1000, 3000, 7000, 15000, 31000, 61000, 91000].entries()) {
    vi.setSystemTime(time - 1); await host.refresh()
    expect(request).toHaveBeenCalledTimes(attempt + 1)
    vi.setSystemTime(time); await host.refresh()
    expect(request).toHaveBeenCalledTimes(attempt + 2)
  }
  expect(host.unsupported).toBe(false)
  request.mockResolvedValue(page)
  vi.setSystemTime(121000); await host.refresh()
  expect(host.visiblePage).toEqual(page)
  vi.setSystemTime(121499); await host.refresh()
  expect(request).toHaveBeenCalledTimes(9)
  vi.setSystemTime(121500); await host.refresh()
  expect(request).toHaveBeenCalledTimes(10)
})

it('preserves an existing Host page during a transport failure and resumes on the next retry', async () => {
  vi.useFakeTimers(); vi.setSystemTime(0)
  const request = vi.fn(async (): Promise<HostHistoryReply> => page)
  const host = new HostHistoryWindow(request)
  await host.refresh()
  request.mockRejectedValueOnce(new Error('connection interrupted'))
  vi.setSystemTime(500); await host.refresh()
  expect(host.visiblePage).toEqual(page)
  vi.setSystemTime(1500); await host.refresh()
  expect(request).toHaveBeenCalledTimes(3)
  expect(host.unsupported).toBe(false)
})

it('stops retrying only when the Host explicitly reports unsupported capability', async () => {
  vi.useFakeTimers(); vi.setSystemTime(0)
  const request = vi.fn(async (): Promise<HostHistoryReply> => ({ kind: 'unsupported' }))
  const host = new HostHistoryWindow(request)
  await host.refresh()
  vi.setSystemTime(60000); await host.refresh()
  expect(request).toHaveBeenCalledTimes(1)
  expect(host.unsupported).toBe(true)
})

it('does not retry or replace the page after the reader closes', async () => {
  vi.useFakeTimers(); vi.setSystemTime(0)
  const request = vi.fn(async (): Promise<HostHistoryReply> => ({ kind: 'error', error: 'unavailable' }))
  const host = new HostHistoryWindow(request)
  await host.refresh(); await host.close()
  request.mockResolvedValue(page)
  vi.setSystemTime(60000); await host.refresh()
  expect(request).toHaveBeenCalledTimes(2)
  expect(host.visiblePage).toBeNull()
})
