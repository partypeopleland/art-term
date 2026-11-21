import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import App from '../src/desktop/App.vue'
import type { AppController } from '../src/shared/state/controller'
import { createDesktopXtermTerminal, type TerminalHandle } from '../src/shared/terminal/handle'
import {
  TERMINAL_FONT_SIZE_DEFAULT,
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  clampTerminalFontSize,
  terminalFontSize,
  terminalFontZoomKeyDirection,
  terminalFontZoomWheelDirection,
  zoomTerminalFontSize
} from '../src/shared/terminal/font-size'
import { FakeTerm, FakeTransport, makeHost, makeTarget } from './fake'

vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
window.matchMedia = (() => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia

async function flush(times = 20): Promise<void> {
  for (let i = 0; i < times; i++) {
    await Promise.resolve()
    await nextTick()
  }
}

function key(init: KeyboardEventInit): KeyboardEvent {
  return new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })
}

function wheel(init: WheelEventInit): WheelEvent {
  return new WheelEvent('wheel', { bubbles: true, cancelable: true, ...init })
}

function dispatchOnChild(element: HTMLElement, event: Event): void {
  const child = document.createElement('div')
  element.appendChild(child)
  child.dispatchEvent(event)
}

function mountApp(transport: FakeTransport) {
  const terms: FakeTerm[] = []
  const wrapper = mount(App, {
    props: {
      transport,
      terminalFactory: () => {
        const term = new FakeTerm()
        terms.push(term)
        return term
      },
      controllerOptions: { idlePollMs: 0, autoCaptureMs: 30_000 }
    }
  })
  const controller = (wrapper.vm as unknown as { controller: AppController }).controller
  return { wrapper, controller, terms }
}

beforeEach(() => {
  terminalFontSize.value = TERMINAL_FONT_SIZE_DEFAULT
  document.body.innerHTML = ''
})

afterEach(() => {
  terminalFontSize.value = TERMINAL_FONT_SIZE_DEFAULT
  document.body.innerHTML = ''
})

describe('terminal font size policy', () => {
  it('clamps discrete steps inside the supported range', () => {
    expect(clampTerminalFontSize(9)).toBe(TERMINAL_FONT_SIZE_MIN)
    expect(clampTerminalFontSize(33)).toBe(TERMINAL_FONT_SIZE_MAX)
    expect(clampTerminalFontSize(14.6)).toBe(15)
    expect(clampTerminalFontSize(Number.NaN)).toBe(TERMINAL_FONT_SIZE_DEFAULT)

    terminalFontSize.value = TERMINAL_FONT_SIZE_DEFAULT
    expect(zoomTerminalFontSize(1)).toBe(15)
    expect(zoomTerminalFontSize(-1)).toBe(14)
    terminalFontSize.value = TERMINAL_FONT_SIZE_MAX
    expect(zoomTerminalFontSize(1)).toBe(TERMINAL_FONT_SIZE_MAX)
    terminalFontSize.value = TERMINAL_FONT_SIZE_MIN
    expect(zoomTerminalFontSize(-1)).toBe(TERMINAL_FONT_SIZE_MIN)
  })

  it('matches Ctrl zoom keys by physical key and rejects other modifiers', () => {
    expect(terminalFontZoomKeyDirection(key({ code: 'Equal', key: '=', ctrlKey: true }))).toBe(1)
    expect(terminalFontZoomKeyDirection(key({ code: 'Equal', key: '+', ctrlKey: true, shiftKey: true, repeat: true }))).toBe(1)
    expect(terminalFontZoomKeyDirection(key({ code: 'Minus', key: '-', ctrlKey: true }))).toBe(-1)
    expect(terminalFontZoomKeyDirection(key({ code: 'NumpadAdd', key: '+', ctrlKey: true }))).toBe(1)
    expect(terminalFontZoomKeyDirection(key({ code: 'NumpadSubtract', key: '-', ctrlKey: true }))).toBe(-1)
    expect(terminalFontZoomKeyDirection(key({ code: 'Equal', key: '=', altKey: true, ctrlKey: true }))).toBeNull()
    expect(terminalFontZoomKeyDirection(key({ code: 'Equal', key: '=', metaKey: true, ctrlKey: true }))).toBeNull()
    expect(terminalFontZoomKeyDirection(key({ code: 'Equal', key: '=' }))).toBeNull()
    expect(terminalFontZoomKeyDirection(key({ code: 'KeyA', key: 'a', ctrlKey: true }))).toBeNull()
  })

  it('matches only Ctrl wheel with vertical delta', () => {
    expect(terminalFontZoomWheelDirection(wheel({ deltaY: -120, ctrlKey: true }))).toBe(1)
    expect(terminalFontZoomWheelDirection(wheel({ deltaY: 120, ctrlKey: true }))).toBe(-1)
    expect(terminalFontZoomWheelDirection(wheel({ deltaY: -120 }))).toBeNull()
    expect(terminalFontZoomWheelDirection(wheel({ deltaY: -120, ctrlKey: true, altKey: true }))).toBeNull()
    expect(terminalFontZoomWheelDirection(wheel({ deltaY: 0, ctrlKey: true }))).toBeNull()
  })
})

describe('desktop terminal font zoom', () => {
  it('zooms shared terminal font by Ctrl wheel and Ctrl +/- without sending input', async () => {
    const transport = new FakeTransport()
    const target = makeTarget('host-shell', 'computers', 'host')
    const { wrapper, controller, terms } = mountApp(transport)
    try {
      await flush()
      controller.openTarget({ dto: makeHost('computers', [target]), expanded: true }, target, false)
      await flush()
      const term = terms[0]
      const host = wrapper.find('.term-host').element as HTMLElement
      const sid = controller.tabs[0].sessionId!
      const fits = term.fitCount

      const zoomIn = wheel({ deltaY: -120, ctrlKey: true })
      dispatchOnChild(host, zoomIn)
      expect(zoomIn.defaultPrevented).toBe(true)
      await flush()
      expect(terminalFontSize.value).toBe(15)
      expect(term.fontSize).toBe(15)
      expect(term.fitCount).toBeGreaterThan(fits)

      const plain = wheel({ deltaY: -120 })
      dispatchOnChild(host, plain)
      expect(plain.defaultPrevented).toBe(false)
      await nextTick()
      expect(terminalFontSize.value).toBe(15)

      const plus = key({ code: 'Equal', key: '+', ctrlKey: true, shiftKey: true })
      dispatchOnChild(host, plus)
      expect(plus.defaultPrevented).toBe(true)
      await flush()
      expect(terminalFontSize.value).toBe(16)
      expect(term.fontSize).toBe(16)

      const minus = key({ code: 'Minus', key: '-', ctrlKey: true })
      dispatchOnChild(host, minus)
      expect(minus.defaultPrevented).toBe(true)
      await flush()
      expect(terminalFontSize.value).toBe(15)
      expect(term.fontSize).toBe(15)

      const altGr = key({ code: 'Equal', key: '=', ctrlKey: true, altKey: true })
      dispatchOnChild(host, altGr)
      expect(altGr.defaultPrevented).toBe(false)
      await nextTick()
      expect(terminalFontSize.value).toBe(15)

      expect(transport.sessions.get(sid)!.writes).toEqual([])
    } finally {
      wrapper.unmount()
    }
  })

  it('applies the shared size to every mounted desktop terminal pane', async () => {
    const transport = new FakeTransport()
    const targets = [makeTarget('first', 'computers', 'host'), makeTarget('second', 'computers', 'host')]
    const { wrapper, controller, terms } = mountApp(transport)
    try {
      await flush()
      const group = { dto: makeHost('computers', targets), expanded: true }
      controller.openTarget(group, targets[0], false)
      await flush()
      controller.openTarget(group, targets[1], false)
      await flush()
      expect(terms).toHaveLength(2)
      const hosts = wrapper.findAll('.term-host')
      expect(hosts).toHaveLength(2)
      const fits = terms[1].fitCount

      const event = wheel({ deltaY: -120, ctrlKey: true })
      dispatchOnChild(hosts[1].element as HTMLElement, event)
      await flush()

      expect(terminalFontSize.value).toBe(15)
      expect(terms[0].fontSize).toBe(15)
      expect(terms[1].fontSize).toBe(15)
      expect(terms[1].fitCount).toBeGreaterThan(fits)
    } finally {
      wrapper.unmount()
    }
  })

  it('real xterm handle applies a clamped font size to the renderer', () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    const terminal: TerminalHandle = createDesktopXtermTerminal(host)
    try {
      terminal.setFontSize?.(15)
      const css = Array.from(host.querySelectorAll('style')).map((style) => style.textContent).join('\n')
      expect(css).toContain('font-size: 15px')
      terminal.setFontSize?.(99)
      const clamped = Array.from(host.querySelectorAll('style')).map((style) => style.textContent).join('\n')
      expect(clamped).toContain(`font-size: ${TERMINAL_FONT_SIZE_MAX}px`)
    } finally {
      terminal.dispose()
      host.remove()
    }
  })
})
