import { afterEach, expect, it } from 'vitest'
window.matchMedia = (() => ({ matches: false, addListener() {}, removeListener() {}, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia
import { createXtermTerminal, type TerminalHandle } from '../src/shared/terminal/handle'
const handles: TerminalHandle[] = []
afterEach(() => { handles.splice(0).forEach(h => h.dispose()); document.body.replaceChildren() })
function setup() {
  const el = document.createElement('div'); document.body.append(el)
  const handle = createXtermTerminal(el); handles.push(handle)
  const events: { text: string; reply: boolean }[] = []
  handle.onData((text, reply) => events.push({ text, reply: !!reply }))
  return { el, handle, events, write: (s: string, replay = false) => handle.write(new TextEncoder().encode(s), replay) }
}
it('observes split CPR/DA/mode/DCS/OSC protocol replies through public parsers and preserves replay suppression', async () => {
  const s = setup()
  await s.write('\x1b['); await s.write('6n')
  await s.write('\x1b[c\x1b[>c\x1b[?25$p\x1bP$qm\x1b\\\x1b]10;?\x07\x1bZ')
  expect(s.events.length).toBeGreaterThanOrEqual(5)
  expect(s.events.every(event => event.reply)).toBe(true)
  const before = s.events.length
  await s.write('\x1b[', true); await s.write('6n')
  expect(s.events).toHaveLength(before)
})
it('does not mark keyboard input emitted while a write is queued or after an unsupported query', async () => {
  const s = setup()
  const queued = s.write('\x1b[6n')
  s.el.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowUp', keyCode: 38, bubbles: true, cancelable: true }))
  await queued
  expect(s.events.find(e => e.text === '\x1b[A')?.reply).toBe(false)
  await s.write('\x1b[999n'); await Promise.resolve()
  s.el.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', keyCode: 40, bubbles: true, cancelable: true }))
  expect(s.events.find(e => e.text === '\x1b[B')?.reply).toBe(false)
})
