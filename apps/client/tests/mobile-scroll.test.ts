import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createMobileXtermTerminal, type TerminalHandle } from '../src/shared/terminal/handle'
import { tmuxHistoryPosition } from '../src/mobile/scroll'

vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
window.matchMedia = (() => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia
const opened: TerminalHandle[] = []
// jsdom has no font layout; provide only character measurements so the real
// xterm viewport can calculate scrolling. Browser gestures are checked separately.
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('xterm-char-measure-element') ? 256 : 0 })
  vi.spyOn(HTMLElement.prototype, 'offsetHeight', 'get').mockImplementation(function (this: HTMLElement) { return this.classList.contains('xterm-char-measure-element') ? 16 : 0 })
})
afterEach(() => { opened.splice(0).forEach(t => t.dispose()); document.body.replaceChildren(); vi.restoreAllMocks() })

function touch(screen: Element, type: string, fingers: { identifier: number; clientX: number; clientY: number }[]) {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.defineProperty(event, 'touches', { value: fingers })
  screen.dispatchEvent(event)
  return event
}
async function setup() {
  const host = document.createElement('div'); document.body.append(host)
  const term = createMobileXtermTerminal(host); opened.push(term)
  const input: string[] = []; term.onData(text => input.push(text))
  await term.write(new TextEncoder().encode(Array.from({ length: 150 }, (_, i) => `ROW_${i}\r\n`).join('')))
  await new Promise(resolve => setTimeout(resolve, 180))
  const screen = host.querySelector('.xterm-screen')!
  const track = host.querySelector('.mobile-scroll-track')!
  const position = () => Number(track.getAttribute('aria-valuenow'))
  const finger = (y: number) => [{ identifier: 1, clientX: 20, clientY: y }]
  return { host, term, input, screen, track, position, finger }
}

describe('mobile terminal touch history', () => {
  it('swipes local history both ways with a persistent position rail and no shell keys', async () => {
    const s = await setup(); const bottom = s.position()
    expect(bottom).toBeGreaterThan(0)
    expect(s.host.querySelector('.mobile-scroll-thumb')).not.toBeNull()
    touch(s.screen, 'touchstart', s.finger(50))
    expect(touch(s.screen, 'touchmove', s.finger(170)).defaultPrevented).toBe(true)
    await vi.waitFor(() => expect(s.position()).toBeLessThan(bottom))
    const older = s.position()
    touch(s.screen, 'touchmove', s.finger(90))
    await vi.waitFor(() => expect(s.position()).toBeGreaterThan(older))
    touch(s.screen, 'touchend', [])
    expect(s.input).toEqual([])
  })

  it('leaves taps, horizontal movement and multiple fingers out of scrolling', async () => {
    const s = await setup(); const bottom = s.position()
    touch(s.screen, 'touchstart', s.finger(50))
    expect(touch(s.screen, 'touchmove', s.finger(54)).defaultPrevented).toBe(false)
    touch(s.screen, 'touchmove', [{ identifier: 1, clientX: 150, clientY: 80 }])
    touch(s.screen, 'touchmove', s.finger(170))
    expect(s.position()).toBe(bottom)
    touch(s.screen, 'touchstart', s.finger(50))
    const two = [...s.finger(170), { identifier: 2, clientX: 40, clientY: 180 }]
    expect(touch(s.screen, 'touchmove', two).defaultPrevented).toBe(false)
    touch(s.screen, 'touchmove', s.finger(220))
    expect(s.position()).toBe(bottom)
    expect(s.input).toEqual([])
  })

  it('stops gestures on cancellation, backgrounding, reset and disposal', async () => {
    const s = await setup(); const bottom = s.position()
    for (const cancel of [() => touch(s.screen, 'touchcancel', []), () => document.dispatchEvent(new Event('visibilitychange')), () => s.term.reset()]) {
      touch(s.screen, 'touchstart', s.finger(50)); cancel()
      expect(touch(s.screen, 'touchmove', s.finger(170)).defaultPrevented).toBe(false)
    }
    touch(s.screen, 'touchstart', s.finger(50)); s.term.dispose()
    touch(s.screen, 'touchmove', s.finger(170))
    expect(s.host.querySelector('.mobile-scrollbar')).toBeNull()
    expect(s.input).toEqual([])
    expect(bottom).toBeGreaterThan(0)
  })

  it('keeps remote history controls visible without claiming a local position', async () => {
    const s = await setup()
    await s.term.write(new TextEncoder().encode('\x1b[?1049h\x1b[?1000h\x1b[?1006h'))
    expect(s.host.querySelector('.mobile-scrollbar')?.getAttribute('data-mode')).toBe('remote')
    expect(s.track.getAttribute('aria-valuenow')).toBeNull()
    expect(s.host.querySelectorAll('.mobile-scrollbar button')).toHaveLength(0)
    expect(s.host.querySelector('.mobile-scroll-thumb')?.hasAttribute('hidden')).toBe(true)
  })

  it('links the remote thumb to tmux copy-mode output and back to the bottom', async () => {
    const s = await setup()
    await s.term.write(new TextEncoder().encode('\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[Hhistory [50/150]'))
    expect(s.track.getAttribute('aria-valuemax')).toBe('150')
    expect(s.position()).toBe(100)
    await s.term.write(new TextEncoder().encode('\x1b[H\x1b[2Khistory [80/150]'))
    expect(s.position()).toBe(70)
    await s.term.write(new TextEncoder().encode('\x1b[H\x1b[2Kshell prompt'))
    expect(s.position()).toBe(150)
    expect(tmuxHistoryPosition('text [200/150]')).toBeNull()
    expect(tmuxHistoryPosition('[0/0]')).toBeNull()
  })

  it('allows rail arrow keys to scroll history without sending keys to the shell', async () => {
    const s = await setup(); const bottom = s.position()
    const key = new KeyboardEvent('keydown', { key: 'ArrowUp', bubbles: true, cancelable: true })
    s.track.dispatchEvent(key)
    expect(key.defaultPrevented).toBe(true)
    await vi.waitFor(() => expect(s.position()).toBeLessThan(bottom))
    expect(s.input).toEqual([])
  })
})
