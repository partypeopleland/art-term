import { afterEach, describe, expect, it, vi } from 'vitest'
import type { Terminal } from '@xterm/xterm'
import { attachMobileScroll } from '../src/mobile/scroll'

const cleanup: (() => void)[] = []
afterEach(() => { cleanup.splice(0).forEach(fn => fn()); vi.unstubAllGlobals(); document.body.replaceChildren() })
function fixture() {
  const frames = new Map<number, FrameRequestCallback>(); let serial = 0
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => { frames.set(++serial, fn); return serial })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => frames.delete(id))
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  const host = document.createElement('div'), screen = document.createElement('div'); screen.className = 'xterm-screen'; host.append(screen); document.body.append(host)
  const rect = { left: 100, top: 80, width: 360, height: 400 }
  screen.getBoundingClientRect = () => ({ ...rect, right: rect.left + rect.width, bottom: rect.top + rect.height, x: rect.left, y: rect.top, toJSON() {} })
  const event = () => ({ dispose() {} })
  const term = { rows: 20, modes: { mouseTrackingMode: 'any' }, buffer: { active: { type: 'alternate', viewportY: 0, getLine: () => undefined } }, onScroll: event, onWriteParsed: event, onResize: event, onRender: event } as unknown as Terminal
  const scrolling = attachMobileScroll(host, term); cleanup.push(scrolling.dispose)
  const wheels: WheelEvent[] = []; screen.addEventListener('wheel', e => wheels.push(e as WheelEvent))
  function touch(type: string, x = 240, y = 240, multiple = false) {
    const e = new Event(type, { bubbles: true, cancelable: true })
    Object.defineProperty(e, 'touches', { value: type === 'touchend' || type === 'touchcancel' ? [] : [{ identifier: 1, clientX: x, clientY: y }, ...(multiple ? [{ identifier: 2, clientX: x + 20, clientY: y }] : [])] }); screen.dispatchEvent(e)
  }
  function frame(time = 100) { const batch = [...frames.values()]; frames.clear(); batch.forEach(fn => fn(time)) }
  return { host, screen, rect, wheels, scrolling, touch, frame, frames }
}

describe('remote transcript wheel hit location', () => {
  it('keeps the touch-start transcript position after finger release instead of targeting the header', () => {
    const s = fixture(); s.touch('touchstart', 240, 240); s.touch('touchmove', 240, 390); s.touch('touchend'); s.frame()
    expect(s.wheels).toHaveLength(1)
    const wheel = s.wheels[0]
    // A coordinate-aware TUI accepts transcript rows and ignores its header/footer.
    const acceptsTranscript = (y: number) => y >= 180 && y < 360
    expect(acceptsTranscript(wheel.clientY)).toBe(true)
    expect(acceptsTranscript(s.rect.top + 20)).toBe(false)
    expect([wheel.clientX, wheel.clientY, wheel.deltaY]).toEqual([240, 240, -1])
  })
  it('cancels pending wheels on background/reset and uses a fresh location for the next gesture', () => {
    const s = fixture(); s.touch('touchstart', 240, 240); s.touch('touchmove', 240, 390)
    document.dispatchEvent(new Event('visibilitychange')); s.frame(); expect(s.wheels).toHaveLength(0)
    s.touch('touchstart', 300, 280); s.touch('touchmove', 300, 430); s.scrolling.reset(); s.frame(); expect(s.wheels).toHaveLength(0)
    s.touch('touchstart', 280, 260); s.touch('touchmove', 280, 410); s.frame()
    expect(s.wheels.map(e => [e.clientX, e.clientY])).toEqual([[280, 260]])
  })
  it('uses viewport center for rail scrolling and keeps queued wheel events bounded', () => {
    const s = fixture(); const track = s.host.querySelector('.mobile-scroll-track')!
    track.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })); s.frame()
    expect(s.wheels.map(e => [e.clientX, e.clientY])).toEqual([[280, 280]])
    s.scrolling.reset(); s.wheels.length = 0
    s.touch('touchstart', 240, 240); s.touch('touchmove', 240, 2000); s.touch('touchend')
    for (let i = 1; i <= 20; i++) s.frame(100 + i * 64)
    expect(s.wheels).toHaveLength(8)
    expect(s.frames.size).toBe(0)
  })
})
