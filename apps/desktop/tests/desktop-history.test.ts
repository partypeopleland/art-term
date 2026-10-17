import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import DesktopHistoryReader from '../src/components/DesktopHistoryReader.vue'
import type { HostHistoryPage, HostHistoryRequest, HostHistoryReply } from '../src/terminal/native'
import { deferred } from './fake'

const wrappers: ReturnType<typeof mount>[] = []
const instance = 'a'.repeat(32)
function page(start: number, end: number, epoch = 1, rowCount = 1800): HostHistoryPage {
  return { kind: 'page', instance, epoch, columns: 80, rows: 24, rowCount, start, end, hasOlder: start > 0,
    records: Array.from({ length: end - start }, (_, i) => ({ id: start + i + 1, text: `ROW-${start + i + 1}`, styles: [] })) }
}
function fixture(visible = true) {
  let epoch = 1
  const request = vi.fn(async (value: HostHistoryRequest): Promise<HostHistoryReply> => {
    if (value.op === 'viewport') return page(1776, 1800, epoch)
    if (value.op === 'older') return page(Math.max(0, value.before - value.limit), value.before, value.epoch)
    return { kind: 'ack', instance, epoch, op: value.op }
  })
  const wrapper = mount(DesktopHistoryReader, { props: { request, visible } })
  wrappers.push(wrapper)
  return { wrapper, request, advanceEpoch: () => { epoch++ } }
}
async function flush() {
  await flushPromises()
  if (vi.isFakeTimers()) await vi.advanceTimersByTimeAsync(20)
  await flushPromises()
}
beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => { cb(0); return 1 })
})
afterEach(async () => {
  wrappers.splice(0).forEach(w => w.unmount())
  await flush()
  vi.useRealTimers()
})

describe('Windows Host history reader', () => {
  it('does not ACK the offered snapshot until its rows are committed and painted', async () => {
    let paint!: FrameRequestCallback
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { paint = callback; return 1 })
    const f = fixture(); await flushPromises()
    expect(f.wrapper.text()).toContain('ROW-1800')
    expect(f.request.mock.calls.some(([r]) => r.op === 'ack')).toBe(false)
    paint(0); await flushPromises()
    expect(f.request.mock.calls.find(([r]) => r.op === 'ack')?.[0]).toMatchObject({ epoch: 1 })
  })
  it('loads the viewport only, ACKs committed DOM, and reads older rows on demand', async () => {
    const f = fixture()
    await flush()
    expect(f.wrapper.text()).toContain('ROW-1800')
    expect(f.request.mock.calls.some(([r]) => r.op === 'older')).toBe(false)
    expect(f.request.mock.calls.find(([r]) => r.op === 'ack')?.[0]).toMatchObject({ epoch: 1 })
    const viewport = f.wrapper.get('.history-viewport').element as HTMLElement
    viewport.scrollTop = 8
    await f.wrapper.get('[data-testid=history-older]').trigger('click'); await flush()
    expect(f.request.mock.calls.find(([r]) => r.op === 'older')?.[0]).toEqual({ op: 'older', instance, epoch: 1, before: 1776, limit: 256 })
    expect(f.wrapper.findAll('.history-line')).toHaveLength(280)
    expect(viewport.scrollTop).toBe(256 * 21 + 8)
    expect(f.wrapper.text()).toContain('閱讀中')
  })

  it('bounds the DOM at 1024 rows and can page towards newer content after head navigation', async () => {
    const f = fixture(); await flush()
    for (let i = 0; i < 5; i++) { await f.wrapper.get('[data-testid=history-older]').trigger('click'); await flush() }
    expect(f.wrapper.findAll('.history-line')).toHaveLength(1024)
    expect(f.wrapper.findAll('.history-line').at(-1)!.attributes('data-line-id')).toBe('1520')
    expect(f.wrapper.get('[data-testid=history-newer]').attributes('disabled')).toBeUndefined()
    await f.wrapper.get('[data-testid=history-newer]').trigger('click'); await flush()
    expect(f.wrapper.findAll('.history-line')).toHaveLength(1024)
    expect(f.wrapper.findAll('.history-line').at(-1)!.attributes('data-line-id')).toBe('1776')
    await f.wrapper.get('[data-testid=history-newer]').trigger('click'); await flush()
    expect(f.wrapper.findAll('.history-line').at(-1)!.attributes('data-line-id')).toBe('1800')
  })

  it('holds the displayed epoch while reading, renews its lease, and resumes latest explicitly', async () => {
    vi.useFakeTimers()
    const f = fixture(); await flush()
    await f.wrapper.get('.history-viewport').trigger('pointerdown')
    f.advanceEpoch()
    await vi.advanceTimersByTimeAsync(31000); await flush()
    expect(f.request.mock.calls.some(([r]) => r.op === 'keep' && r.epoch === 1)).toBe(true)
    expect(f.request.mock.calls.filter(([r]) => r.op === 'viewport')).toHaveLength(1)
    await f.wrapper.get('[data-testid=history-latest]').trigger('click'); await flush()
    expect(f.request.mock.calls.find(([r]) => r.op === 'ack' && r.epoch === 2)).toBeTruthy()
  })

  it('keeps a displayed page when a scroll starts during an in-flight refresh', async () => {
    vi.useFakeTimers()
    const f = fixture(); await flush()
    const gate = deferred<HostHistoryReply>()
    f.request.mockImplementationOnce(() => gate.promise)
    await vi.advanceTimersByTimeAsync(500)
    await f.wrapper.get('.history-viewport').trigger('pointerdown')
    gate.resolve(page(1976, 2000, 2, 2000)); await flush()
    expect(f.wrapper.text()).toContain('ROW-1800')
    expect(f.wrapper.text()).not.toContain('ROW-2000')
    expect(f.request.mock.calls.some(([r]) => r.op === 'ack' && r.epoch === 2)).toBe(false)
  })

  it('does not request hidden history, stops hidden refreshes, and closes only its reader', async () => {
    vi.useFakeTimers()
    const f = fixture(false); await flush()
    await vi.advanceTimersByTimeAsync(1500)
    expect(f.request).not.toHaveBeenCalled()
    await f.wrapper.setProps({ visible: true }); await flush()
    await f.wrapper.setProps({ visible: false })
    const count = f.request.mock.calls.length
    await vi.advanceTimersByTimeAsync(32000)
    expect(f.request).toHaveBeenCalledTimes(count)
    f.wrapper.unmount(); wrappers.splice(wrappers.indexOf(f.wrapper), 1); await flush()
    expect(f.request.mock.calls.at(-1)?.[0]).toEqual({ op: 'close' })
  })

  it('escapes terminal text and preserves foreground/background, flags, and blank rows', async () => {
    const p = page(0, 2, 1, 2)
    p.records[0] = { id: 1, text: '<img>彩色', styles: [[5, 7, 0xff0000, 0x003366, 1 | 2 | 4]] }
    p.records[1] = { id: 2, text: '', styles: [] }
    const request = vi.fn(async (r: HostHistoryRequest): Promise<HostHistoryReply> => r.op === 'viewport' ? p : { kind: 'ack', instance, op: r.op })
    const wrapper = mount(DesktopHistoryReader, { props: { request, visible: true } }); wrappers.push(wrapper)
    await flush()
    expect(wrapper.find('img').exists()).toBe(false)
    expect(wrapper.get('[data-line-id="1"]').text()).toBe('<img>彩色')
    expect(wrapper.get('[data-line-id="1"] span:last-child').attributes('style')).toContain('color: rgb(255, 0, 0)')
    expect(wrapper.get('[data-line-id="1"] span:last-child').attributes('style')).toContain('font-weight: bold')
    expect(wrapper.findAll('.history-line')).toHaveLength(2)
  })

  it('reports unsupported shells without swallowing live terminal input or invoking write', async () => {
    const request = vi.fn(async (): Promise<HostHistoryReply> => ({ kind: 'unsupported' }))
    const wrapper = mount(DesktopHistoryReader, { props: { request, visible: true } }); wrappers.push(wrapper)
    await flush()
    expect(wrapper.text()).toContain('此 Shell 尚不支援主機歷史')
    await wrapper.get('[data-testid=history-close]').trigger('click')
    expect(wrapper.emitted('close')).toHaveLength(1)
    expect(request.mock.calls).toHaveLength(1)
  })

  it('discards a late initial page after closing and still releases the owned reader', async () => {
    const gate = deferred<HostHistoryReply>()
    const request = vi.fn(async (r: HostHistoryRequest): Promise<HostHistoryReply> => r.op === 'viewport'
      ? gate.promise : { kind: 'ack', instance, op: r.op })
    const wrapper = mount(DesktopHistoryReader, { props: { request, visible: true } }); wrappers.push(wrapper)
    await flushPromises()
    wrapper.unmount(); wrappers.splice(wrappers.indexOf(wrapper), 1)
    gate.resolve(page(1776, 1800)); await flush()
    expect(request.mock.calls.map(([r]) => r.op)).toEqual(['viewport', 'close'])
  })
})
