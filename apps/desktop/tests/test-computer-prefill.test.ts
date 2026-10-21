import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import RuntimeHosts from '../src/components/RuntimeHosts.vue'
import { getTestComputerPrefill, parseTestComputerPrefill } from '../src/test-computer-prefill'
import { hostErrorText } from '../src/transport/host-error'
import { FakeTransport, deferred } from './fake'

const state = vi.hoisted(() => ({ value: null as unknown }))
vi.mock('../src/test-computer-prefill', async (original) => ({
  ...await original<typeof import('../src/test-computer-prefill')>(),
  getTestComputerPrefill: vi.fn(() => state.value)
}))
const fixture = { name: 'Fixture', baseUrl: 'https://runtime.example.test:8443', credential: 'fixture-only-token' }
const wrappers: ReturnType<typeof mount>[] = []
function setup(list = Promise.resolve([] as unknown[])) {
  const transport = Object.assign(new FakeTransport([]), {
    runtimeHosts: vi.fn().mockReturnValue(list),
    runtimeSaveHost: vi.fn(), runtimeDeleteHost: vi.fn(), runtimeTestHost: vi.fn()
  })
  const wrapper = mount(RuntimeHosts, { props: { transport, initialView: 'create' } })
  wrappers.push(wrapper)
  return { wrapper, transport }
}
function value(wrapper: ReturnType<typeof mount>, field: string) {
  return (wrapper.get(`[data-testid="${field}"]`).element as HTMLInputElement).value
}
afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  state.value = null
  vi.mocked(getTestComputerPrefill).mockClear()
  vi.restoreAllMocks()
})

describe('private test Computer prefill', () => {
  it('fills the masked form after loading without saving or testing automatically', async () => {
    state.value = fixture
    const { wrapper, transport } = setup()
    await flushPromises()
    expect(value(wrapper, 'name')).toBe(fixture.name)
    expect(value(wrapper, 'base-url')).toBe(fixture.baseUrl)
    expect(value(wrapper, 'credential')).toBe(fixture.credential)
    expect(wrapper.get('[data-testid="credential"]').attributes('type')).toBe('password')
    expect(wrapper.text()).toContain('尚未儲存')
    expect(wrapper.text()).not.toContain(fixture.credential)
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(transport.runtimeTestHost).not.toHaveBeenCalled()
  })

  it('leaves public builds empty', async () => {
    const { wrapper } = setup()
    await flushPromises()
    expect(value(wrapper, 'name')).toBe('')
    expect(value(wrapper, 'base-url')).toBe('')
    expect(value(wrapper, 'credential')).toBe('')
  })

  it('retains private prefill convenience even when listing saved Hosts fails', async () => {
    state.value = fixture
    const { wrapper } = setup(Promise.reject({ kind: 'storage', message: 'never render this' }))
    await flushPromises()
    expect(value(wrapper, 'credential')).toBe(fixture.credential)
    expect(wrapper.text()).toContain('[STORAGE]')
  })

  it('never replaces typed input, even if it was subsequently erased', async () => {
    state.value = fixture
    const pending = deferred<unknown[]>()
    const { wrapper } = setup(pending.promise)
    await wrapper.get('[data-testid="name"]').setValue('Owner edit')
    await wrapper.get('[data-testid="name"]').setValue('')
    pending.resolve([])
    await flushPromises()
    expect(value(wrapper, 'credential')).toBe('')
  })

  it('does not refill after cancellation while the list is pending', async () => {
    state.value = fixture
    const pending = deferred<unknown[]>()
    const { wrapper } = setup(pending.promise)
    await wrapper.findAll('button').find(button => button.text() === '取消')!.trigger('click')
    pending.resolve([])
    await flushPromises()
    expect(wrapper.find('[data-testid="credential"]').exists()).toBe(false)
    expect(wrapper.find('[data-testid="runtime-host-list"]').exists()).toBe(true)
    await wrapper.get('[data-testid="runtime-create"]').trigger('click')
    expect(value(wrapper, 'credential')).toBe('')
  })

  it('clears on background and never refills when returning to foreground', async () => {
    state.value = fixture
    const { wrapper } = setup()
    await flushPromises()
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    await flushPromises()
    expect(value(wrapper, 'credential')).toBe('')
    visibility.mockReturnValue('visible')
    document.dispatchEvent(new Event('visibilitychange'))
    await flushPromises()
    expect(value(wrapper, 'credential')).toBe('')
  })

  it('does not refill on late load after a background and foreground transition', async () => {
    state.value = fixture
    const pending = deferred<unknown[]>()
    const { wrapper } = setup(pending.promise)
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    visibility.mockReturnValue('visible')
    document.dispatchEvent(new Event('visibilitychange'))
    pending.resolve([])
    await flushPromises()
    expect(value(wrapper, 'credential')).toBe('')
    expect(getTestComputerPrefill).not.toHaveBeenCalled()
  })

  it('ignores list completion after unmount', async () => {
    state.value = fixture
    const pending = deferred<unknown[]>()
    const { wrapper, transport } = setup(pending.promise)
    wrapper.unmount()
    pending.resolve([])
    await flushPromises()
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(getTestComputerPrefill).not.toHaveBeenCalled()
  })

  it('accepts bounded HTTPS origins only and rejects invalid private input', () => {
    expect(parseTestComputerPrefill(fixture)).toEqual(fixture)
    for (const bad of [null, [], {}, { ...fixture, name: 'a'.repeat(129) },
      { ...fixture, credential: 'Bearer fixture' }, { ...fixture, credential: 'a'.repeat(1025) },
      { ...fixture, baseUrl: 'http://runtime.example.test' },
      { ...fixture, baseUrl: 'https://user@runtime.example.test' },
      { ...fixture, baseUrl: fixture.baseUrl + '/admin' },
      { ...fixture, baseUrl: fixture.baseUrl + '/a/..' },
      { ...fixture, baseUrl: fixture.baseUrl + '?token=fixture' }]) {
      expect(parseTestComputerPrefill(bad)).toBeNull()
    }
  })

  it('shows a fixed gateway failure without exposing upstream text or suggesting a runtime upgrade', () => {
    const text = hostErrorText({ kind: 'gatewayUnavailable', message: fixture.credential }, 'Generic failure')
    expect(text).toContain('[GATEWAY_UNAVAILABLE]')
    expect(text).toContain('轉送')
    expect(text).not.toContain(fixture.credential)
    expect(text).not.toContain('版本')
    expect(hostErrorText({ kind: 'protocol', message: fixture.credential }, 'Generic failure')).toContain('[PROTOCOL]')
  })

  it('keeps prototype-shaped unknown error kinds on the safe generic fallback', () => {
    for (const kind of ['toString', '__proto__', 'constructor']) {
      expect(hostErrorText({ kind, message: fixture.credential }, 'Generic failure')).toBe('Generic failure')
    }
  })
})
