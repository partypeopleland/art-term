import { afterEach, describe, expect, it, vi } from 'vitest'
import { createDesktopXtermTerminal, createXtermTerminal, type TerminalHandle } from '../src/terminal/handle'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import App from '../src/App.vue'
import type { AppController } from '../src/state/controller'
import { FakeTransport, makeHost, makeTarget } from './fake'

vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
window.matchMedia = (() => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia
const opened: TerminalHandle[] = []
afterEach(() => { opened.splice(0).forEach(t => t.dispose()); document.body.replaceChildren() })

function setup(desktop = true) {
  const host = document.createElement('div')
  document.body.append(host)
  const terminal = desktop ? createDesktopXtermTerminal(host) : createXtermTerminal(host)
  opened.push(terminal)
  const input: string[] = []
  terminal.onData(text => input.push(text))
  const screen = host.querySelector('.xterm-screen')!
  const write = (text: string) => terminal.write(new TextEncoder().encode(text))
  const wheel = (deltaY: number) => screen.dispatchEvent(new WheelEvent('wheel', { deltaY, bubbles: true, cancelable: true }))
  return { host, terminal, input, write, wheel }
}

describe('desktop wheel with real xterm', () => {
  it('suppresses replayed CPR through parser callbacks, including a live suffix, but answers a new live query', async () => {
    const s = setup()
    const bytes = (value: string) => new TextEncoder().encode(value)
    await s.terminal.write(bytes('\x1b[6n'), true)
    expect(s.input).toEqual([])
    await s.terminal.write(bytes('\x1b['), true)
    await s.terminal.write(bytes('6n\x1b[6n'))
    expect(s.input).toEqual(['\x1b[1;1R'])
    s.terminal.reset()
    await s.terminal.write(bytes('\x1b[6n'))
    expect(s.input).toEqual(['\x1b[1;1R', '\x1b[1;1R'])
  })
  it('uses the desktop wheel policy through the App default factory', async () => {
    const target = makeTarget('test-host', 'computers', 'host')
    const transport = new FakeTransport()
    const wrapper = mount(App, { attachTo: document.body, props: { transport } })
    try {
      const controller = (wrapper.vm as unknown as { controller: AppController }).controller
      for (let i = 0; i < 20; i++) { await Promise.resolve(); await nextTick() }
      controller.openTarget({ dto: makeHost('computers', [target]), expanded: true }, target, false)
      for (let i = 0; i < 20; i++) { await Promise.resolve(); await nextTick() }
      const terminal = controller.tabs[0].term!
      const input: string[] = []
      terminal.onData(text => input.push(text))
      await terminal.write(new TextEncoder().encode('\x1b[?1049h'))
      wrapper.element.querySelector('.xterm-screen')!.dispatchEvent(new WheelEvent('wheel', { deltaY: -120, bubbles: true, cancelable: true }))
      expect(input).toEqual([])
      wrapper.element.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', keyCode: 38, bubbles: true, cancelable: true }))
      expect(input).toEqual(['\x1b[A'])
    } finally {
      wrapper.unmount()
    }
  })

  it('does not turn alternate-screen wheel events into command-history keys', async () => {
    const s = setup()
    await s.write('\x1b[?1049h')
    s.wheel(-120)
    s.wheel(120)
    expect(s.input).toEqual([])
    s.host.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', keyCode: 38, bubbles: true, cancelable: true }))
    expect(s.input).toEqual(['\x1b[A'])
  })

  it('retains shared/mobile alternate-screen wheel behavior', async () => {
    const s = setup(false)
    await s.write('\x1b[?1049h')
    s.wheel(-120)
    expect(s.input).toEqual(['\x1b[A'])
  })

  it('keeps normal-buffer wheel events out of shell input', async () => {
    const s = setup()
    await s.write(Array.from({ length: 100 }, (_, n) => `line${n}\r\n`).join(''))
    s.wheel(-120)
    expect(s.input).toEqual([])
  })
})
