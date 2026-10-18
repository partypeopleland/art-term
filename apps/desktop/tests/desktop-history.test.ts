import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import DesktopHistoryReader from '../src/components/DesktopHistoryReader.vue'
import type { HostHistoryPage, HostHistoryRequest, HostHistoryReply } from '../src/terminal/native'
import { HostHistoryWindow } from '../src/terminal/native'
import { deferred, FakeTerm } from './fake'

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
  const terminal = new FakeTerm()
  const wrapper = mount(DesktopHistoryReader, { props: { request, visible, terminal } })
  wrappers.push(wrapper)
  return { wrapper, request, terminal, advanceEpoch: () => { epoch++ } }
}
async function flush() {
  await flushPromises()
  if (vi.isFakeTimers()) await vi.advanceTimersByTimeAsync(20)
  await flushPromises()
}
beforeEach(() => {
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => { cb(0); return 1 })
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.spyOn(HTMLElement.prototype, 'offsetTop', 'get').mockImplementation(function (this: HTMLElement) {
    if (!this.classList.contains('history-line')) return 0
    return 4 + (Number(this.dataset.lineId) - Number((this.parentElement!.firstElementChild as HTMLElement).dataset.lineId)) * 17
  })
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('history-line') ? 17 : 0 })
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('history-viewport') ? 400 : 0 })
})
afterEach(async () => {
  wrappers.splice(0).forEach(w => w.unmount())
  await flush()
  vi.useRealTimers()
  vi.restoreAllMocks()
})


function wheel(wrapper: ReturnType<typeof mount>, deltaY = -34) {
  ;(wrapper.vm as unknown as { wheel: (event: WheelEvent) => void }).wheel(new WheelEvent('wheel', { deltaY, cancelable: true }))
}
describe('Windows shared snapshot adapter', () => {
  it('does not poll live and honors the first small wheel while viewport is delayed', async () => {
    const gate = deferred<HostHistoryReply>()
    const f = fixture(); await flush()
    expect(f.request).not.toHaveBeenCalled()
    f.request.mockImplementationOnce(() => gate.promise)
    wheel(f.wrapper); await flush()
    expect(f.request.mock.calls.map(([r]) => r.op)).toEqual(['viewport'])
    gate.resolve(page(1776, 1800)); await flush()
    expect(f.request.mock.calls.map(([r]) => r.op)).toEqual(['viewport', 'ack', 'older'])
    expect(f.request.mock.calls.at(-1)?.[0]).toMatchObject({ before: 1776, limit: 256 })
    expect(f.wrapper.findAll('.history-line')).toHaveLength(280)
    expect((f.wrapper.get('.history-viewport').element as HTMLElement).scrollTop).toBe(256 * 17 - 34)
  })
  it('bounds rows at 1024 and preserves the visible anchor while loading older pages', async () => {
    const f = fixture(); wheel(f.wrapper); await flush()
    for (let i = 0; i < 4; i++) { (f.wrapper.get('.history-viewport').element as HTMLElement).scrollTop = 0; await f.wrapper.get('.history-viewport').trigger('wheel', { deltaY: -34 }); await flush() }
    expect(f.wrapper.findAll('.history-line')).toHaveLength(1024)
    expect(f.wrapper.findAll('.history-line').at(-1)!.attributes('data-line-id')).toBe('1520')
    const host = f.wrapper.get('.history-viewport').element as HTMLElement
    host.scrollTop = 100000
    await f.wrapper.get('.history-viewport').trigger('wheel', { deltaY: 120 }); await flush()
    expect(f.wrapper.findAll('.history-line').at(-1)!.attributes('data-line-id')).toBe('1776')
  })
  it('keeps held output stable, renews only its snapshot, returns live and reads fresh next time', async () => {
    vi.useFakeTimers()
    const f = fixture(); wheel(f.wrapper); await flush(); f.advanceEpoch()
    const held = f.wrapper.text()
    await vi.advanceTimersByTimeAsync(31000); await flush()
    expect(f.wrapper.text()).toBe(held)
    expect(f.request.mock.calls.filter(([r]) => r.op === 'viewport')).toHaveLength(1)
    expect(f.request.mock.calls.some(([r]) => r.op === 'keep')).toBe(true)
    ;(f.wrapper.get('.history-viewport').element as HTMLElement).scrollTop = 100000
    await f.wrapper.get('.history-viewport').trigger('wheel', { deltaY: 120 }); await flush()
    expect(f.wrapper.get('[data-testid=desktop-history]').isVisible()).toBe(false)
    expect(f.request.mock.calls.at(-1)?.[0]).toEqual({ op: 'close' }); expect(f.terminal.focusCount).toBe(1)
    wheel(f.wrapper); await flush()
    expect(f.request.mock.calls.filter(([r]) => r.op === 'viewport')).toHaveLength(2)
    expect(f.request.mock.calls.some(([r]) => r.op === 'ack' && r.epoch === 2)).toBe(true)
  })
  it('closes hidden pending history, ignores stale rows, and serializes a fresh reopen after release', async () => {
    const f = fixture(); const gate = deferred<HostHistoryReply>(); f.request.mockImplementationOnce(() => gate.promise)
    wheel(f.wrapper); await flush()
    await f.wrapper.setProps({ visible: false }); await f.wrapper.setProps({ visible: true })
    wheel(f.wrapper); await flush()
    expect(f.request.mock.calls).toHaveLength(1)
    gate.resolve(page(1776, 1800)); await flush()
    expect(f.request.mock.calls.slice(0, 3).map(([r]) => r.op)).toEqual(['viewport', 'close', 'viewport'])
    expect(f.wrapper.findAll('.history-line')).toHaveLength(280)
    await f.wrapper.setProps({ visible: false }); await flush()
    expect(f.request.mock.calls.at(-1)?.[0]).toEqual({ op: 'close' })
  })
  it('leaves unsupported shells usable and releases a late initial request on unmount', async () => {
    const f = fixture(); f.request.mockImplementation(async () => ({ kind: 'unsupported' }))
    wheel(f.wrapper); await flush()
    expect(f.wrapper.get('[data-testid=desktop-history]').isVisible()).toBe(false)
    expect(f.terminal.disposed).toBe(false)
    const retry = new WheelEvent('wheel', {deltaY:-34, cancelable:true})
    ;(f.wrapper.vm as unknown as {wheel:(e:WheelEvent)=>void}).wheel(retry)
    expect(retry.defaultPrevented).toBe(false)
    expect(f.request.mock.calls.filter(([r]) => r.op === 'viewport')).toHaveLength(1)
    const gate = deferred<HostHistoryReply>(); const other = fixture(); other.request.mockImplementationOnce(() => gate.promise)
    wheel(other.wrapper); await flush(); other.wrapper.unmount(); wrappers.splice(wrappers.indexOf(other.wrapper), 1)
    gate.resolve(page(1776, 1800)); await flush()
    expect(other.request.mock.calls.map(([r]) => r.op)).toEqual(['viewport', 'close'])
  })
})

describe('safe snapshot presentation', () => {
  it('escapes terminal text and preserves foreground/background, flags, and blank rows', async () => {
    const p = page(0, 2, 1, 2)
    p.records[0] = { id: 1, text: '<img>彩色', styles: [[5, 7, 0xff0000, 0x003366, 1 | 2 | 4]] }
    p.records[1] = { id: 2, text: '', styles: [] }
    const request = vi.fn(async (r: HostHistoryRequest): Promise<HostHistoryReply> => r.op === 'viewport' ? p : { kind: 'ack', instance, op: r.op })
    const wrapper = mount(DesktopHistoryReader, { props: { request, visible: true } }); wrappers.push(wrapper)
    wheel(wrapper); await flush()
    expect(wrapper.find('img').exists()).toBe(false)
    expect(wrapper.get('[data-line-id="1"]').text()).toBe('<img>彩色')
    expect(wrapper.get('[data-line-id="1"] span:last-child').attributes('style')).toContain('color: rgb(255, 0, 0)')
    expect(wrapper.get('[data-line-id="1"] span:last-child').attributes('style')).toContain('font-weight: bold')
    expect(wrapper.findAll('.history-line')).toHaveLength(2)
  })


})

describe('safe snapshot presentation', () => {
  it('copies complete styled URLs on click, preserves punctuation, and never copies a drag', async () => {
    const original = Object.getOwnPropertyDescriptor(navigator, 'clipboard')
    const writeText = vi.fn(async () => {})
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    try {
      const p = page(0, 1, 1, 1)
      p.records[0] = { id: 1, text: 'https://example.test/x.', styles: [[0, 8, 0xff0000, -1, 1]] }
      const request = async (r: HostHistoryRequest): Promise<HostHistoryReply> => r.op === 'viewport' ? p : { kind: 'ack', instance, op: r.op }
      const wrapper = mount(DesktopHistoryReader, { props: { request, visible: true } }); wrappers.push(wrapper)
      wheel(wrapper); await flush()
      const url = wrapper.findAll('[data-url]')[1]
      url.element.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, clientX: 10, clientY: 10 }))
      url.element.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 30, clientY: 10 }))
      expect(writeText).not.toHaveBeenCalled()
      url.element.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, clientX: 10, clientY: 10 }))
      url.element.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 10, clientY: 10 })); await flush()
      expect(writeText).toHaveBeenCalledExactlyOnceWith('https://example.test/x')
      expect(wrapper.get('[data-line-id="1"]').text()).toBe('https://example.test/x.')
    } finally {
      if (original) Object.defineProperty(navigator, 'clipboard', original)
      else Reflect.deleteProperty(navigator, 'clipboard')
    }
  })
})

it('retries a fresh viewport after transient busy instead of marking the Shell unsupported', async () => {
  const f = fixture()
  const notice = vi.fn(); await f.wrapper.setProps({ historyNotice: notice })
  f.request.mockImplementationOnce(async () => ({ kind: 'error', error: 'busy' }))
  wheel(f.wrapper); await flush()
  expect(f.wrapper.get('[data-testid=desktop-history]').isVisible()).toBe(false)
  expect(f.request.mock.calls.map(([r]) => r.op)).toEqual(['viewport', 'close'])
  expect(notice).not.toHaveBeenCalled()
  wheel(f.wrapper); await flush()
  expect(f.request.mock.calls.filter(([r]) => r.op === 'viewport')).toHaveLength(2)
  expect(f.wrapper.findAll('.history-line')).toHaveLength(280)
})

it('uses the shared unavailable state for an explicit unsupported error', async () => {
  const f = fixture()
  f.request.mockImplementationOnce(async () => ({ kind: 'error', error: 'unsupported' }))
  wheel(f.wrapper); await flush()
  const next = new WheelEvent('wheel', {deltaY:-34,cancelable:true})
  ;(f.wrapper.vm as unknown as {wheel:(e:WheelEvent)=>void}).wheel(next)
  expect(next.defaultPrevented).toBe(false)
  expect(f.request.mock.calls.filter(([r]) => r.op === 'viewport')).toHaveLength(1)
})
