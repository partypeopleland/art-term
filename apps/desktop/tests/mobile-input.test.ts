import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createMobileInput } from '../src/mobile/input'
import { FakeTerm } from './fake'

describe('mobile composition with xterm deferred commit', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  function setup() {
    const el = document.createElement('div')
    const real = new FakeTerm()
    const wrapped = createMobileInput().wrap(el, real)
    const received: string[] = []
    wrapped.onData(text => received.push(text))
    function composition(text: string, deferred = text) {
      el.dispatchEvent(new CompositionEvent('compositionstart', { data: '' }))
      real.emitData('intermediate')
      // xterm schedules its task at the textarea before compositionend
      // bubbles to the adapter on the containing element.
      setTimeout(() => real.emitData(deferred), 0)
      el.dispatchEvent(new CompositionEvent('compositionend', { data: text }))
    }
    return { real, wrapped, received, composition }
  }

  it('sends a native composition once and retains intentional identical text later', () => {
    const s = setup()
    s.composition('測試')
    vi.runAllTimers()
    expect(s.received).toEqual(['測試'])
    s.real.emitData('測試')
    expect(s.received).toEqual(['測試', '測試'])
    s.wrapped.dispose()
  })

  it('preserves Enter appended to the delayed xterm commit', () => {
    const s = setup()
    s.composition('中', '中\r')
    vi.runAllTimers()
    expect(s.received.join('')).toBe('中\r')
    s.wrapped.dispose()
  })

  it('preserves consecutive identical compositions', () => {
    const s = setup()
    s.composition('中')
    vi.runAllTimers()
    s.composition('中')
    vi.runAllTimers()
    expect(s.received).toEqual(['中', '中'])
    s.wrapped.dispose()
  })

  it('retains the duplicate guard when Enter arrives before the delayed echo', () => {
    const s = setup()
    s.composition('中')
    s.real.emitData('\r')
    vi.runAllTimers()
    expect(s.received.join('')).toBe('中\r')
    s.wrapped.dispose()
  })

  it('drops a delayed commit after disposal', () => {
    const s = setup()
    s.composition('中')
    s.wrapped.dispose()
    vi.runAllTimers()
    expect(s.received).toEqual(['中'])
  })
})

it('forwards protocol source metadata independently of mobile composition fragments', () => {
  const el = document.createElement('div'), real = new FakeTerm()
  const wrapped = createMobileInput().wrap(el, real), received = vi.fn()
  wrapped.onData(received)
  el.dispatchEvent(new CompositionEvent('compositionstart', { data: '' }))
  real.emitData('\x1b[1;1R', true)
  expect(received).toHaveBeenCalledWith('\x1b[1;1R', true)
  wrapped.dispose()
})
