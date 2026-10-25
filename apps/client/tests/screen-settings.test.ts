import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import ScreenSettings from '../src/shared/components/ScreenSettings.vue'
import { FakeTransport, deferred } from './fake'
import type { InstanceDesktop, InstanceDesktopInput, Transport } from '../src/shared/transport/types'
const screen: InstanceDesktop = { id: 'screen', revision: 4, name: 'Screen A', mcpUrl: 'https://screen.example.test/mcp', tokenPresent: true }
const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => { wrappers.splice(0).forEach(w => w.unmount()); vi.restoreAllMocks() })
function setup(items: InstanceDesktop[] = []) {
  const transport: Transport = new FakeTransport()
  transport.instanceDesktops = vi.fn(async () => items)
  transport.instanceSave = vi.fn(async () => screen)
  transport.instanceDelete = vi.fn(async () => {})
  transport.instanceTest = vi.fn(async () => {})
  const wrapper = mount(ScreenSettings, { props: { transport } }); wrappers.push(wrapper)
  return { wrapper, transport }
}
describe('Screen settings', () => {
  it('lists only native metadata and clears masked token immediately on submission', async () => {
    const { wrapper, transport } = setup()
    const gate = deferred<InstanceDesktop>(); let submitted!: InstanceDesktopInput
    transport.instanceSave = vi.fn(input => { submitted = { ...input }; return gate.promise })
    await flushPromises()
    await wrapper.get('[data-testid=screen-name]').setValue('New Screen')
    await wrapper.get('[data-testid=screen-url]').setValue(screen.mcpUrl)
    await wrapper.get('[data-testid=screen-token]').setValue('test-only-secret')
    await wrapper.get('form').trigger('submit')
    expect(submitted.token).toBe('test-only-secret')
    expect((wrapper.get('[data-testid=screen-token]').element as HTMLInputElement).value).toBe('')
    expect(wrapper.text()).not.toContain('test-only-secret')
    gate.resolve(screen); await flushPromises()
    expect(wrapper.emitted('changed')).toHaveLength(1)
  })
  it('keeps the stored token on unchanged URL and requires replacement after URL change', async () => {
    const { wrapper, transport } = setup([screen]); await flushPromises()
    await wrapper.findAll('li button')[1].trigger('click')
    expect(wrapper.get('[data-testid=screen-token]').attributes('required')).toBeUndefined()
    await wrapper.get('[data-testid=screen-name]').setValue('Renamed')
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(transport.instanceSave).toHaveBeenCalledWith(expect.objectContaining({ id: screen.id, expectedRevision: 4, name: 'Renamed', token: '' }))
    await wrapper.findAll('li button')[1].trigger('click')
    await wrapper.get('[data-testid=screen-url]').setValue('https://other.example.test/mcp')
    expect(wrapper.get('[data-testid=screen-token]').attributes('required')).toBeDefined()
    expect((wrapper.get('button[type=submit]').element as HTMLButtonElement).disabled).toBe(true)
  })
  it('clears unsaved token on background and refuses a late save to mutate the form', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    const gate = deferred<InstanceDesktop>(); transport.instanceSave = vi.fn(() => gate.promise)
    await wrapper.get('[data-testid=screen-name]').setValue('New Screen')
    await wrapper.get('[data-testid=screen-url]').setValue(screen.mcpUrl)
    await wrapper.get('[data-testid=screen-token]').setValue('test-only-secret')
    await wrapper.get('form').trigger('submit')
    vi.spyOn(document, 'hidden', 'get').mockReturnValue(true)
    document.dispatchEvent(new Event('visibilitychange'))
    gate.resolve(screen); await flushPromises()
    expect(wrapper.emitted('changed')).toBeUndefined()
    expect(wrapper.get('[data-testid=screen-token]').element).toHaveProperty('value', '')
  })
})
