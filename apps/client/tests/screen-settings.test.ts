import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import RuntimeHosts from '../src/shared/components/RuntimeHosts.vue'
import { FakeTransport, deferred } from './fake'
import type { InstanceDesktop, InstanceDesktopInput, Transport } from '../src/shared/transport/types'
const screen: InstanceDesktop = { id: 'screen', revision: 4, name: 'Screen A', mcpUrl: 'https://screen.example.test/mcp', tokenPresent: true }
const wrappers: ReturnType<typeof mount>[] = []
afterEach(() => { wrappers.splice(0).forEach(w => w.unmount()); vi.restoreAllMocks() })
function setup(items: InstanceDesktop[] = []) {
  const transport: Transport = new FakeTransport()
  transport.runtimeHosts = vi.fn(async () => [])
  transport.instanceDesktops = vi.fn(async () => items)
  transport.instanceSave = vi.fn(async () => screen)
  transport.instanceDelete = vi.fn(async () => {})
  transport.instanceTest = vi.fn(async () => {})
  const wrapper = mount(RuntimeHosts, { props: { transport, initialView: 'create' } }); wrappers.push(wrapper)
  return { wrapper, transport }
}
describe('Screen settings', () => {
  it('lists only native metadata and clears masked token immediately on submission', async () => {
    const { wrapper, transport } = setup()
    const gate = deferred<InstanceDesktop>(); let submitted!: InstanceDesktopInput
    transport.instanceSave = vi.fn(input => { submitted = { ...input }; return gate.promise })
    await flushPromises()
    await wrapper.get('input[value=screen]').setValue(true)
    await wrapper.get('[data-testid=name]').setValue('New Screen')
    await wrapper.get('[data-testid=base-url]').setValue(screen.mcpUrl)
    await wrapper.get('[data-testid=credential]').setValue('test-only-secret')
    await wrapper.get('form').trigger('submit')
    expect(submitted.token).toBe('test-only-secret')
    expect((wrapper.get('[data-testid=credential]').element as HTMLInputElement).value).toBe('')
    expect(wrapper.text()).not.toContain('test-only-secret')
    gate.resolve(screen); await flushPromises()
    expect(wrapper.emitted('changed')).toHaveLength(1)
  })
  it('keeps the stored token on unchanged URL and requires replacement after URL change', async () => {
    const { wrapper, transport } = setup([screen]); await flushPromises(); await wrapper.get('[data-testid=runtime-form-cancel]').trigger('click')
    await wrapper.get('[data-connection-id=screen] [data-action=edit]').trigger('click')
    await wrapper.get('[data-testid=name]').setValue('Renamed')
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(transport.instanceSave).toHaveBeenCalledWith(expect.objectContaining({ id: screen.id, expectedRevision: 4, name: 'Renamed', token: '' }))
    await wrapper.get('[data-connection-id=screen] [data-action=edit]').trigger('click')
    await wrapper.get('[data-testid=base-url]').setValue('https://other.example.test/mcp')
    await wrapper.get('form').trigger('submit')
    expect(wrapper.get('[data-testid=form-error]').text()).toContain('Token')
    expect(transport.instanceSave).toHaveBeenCalledTimes(1)
  })
  it('clears unsaved token on background and refuses a late save to mutate the form', async () => {
    const { wrapper, transport } = setup(); await flushPromises(); await wrapper.get('input[value=screen]').setValue(true)
    const gate = deferred<InstanceDesktop>(); transport.instanceSave = vi.fn(() => gate.promise)
    await wrapper.get('[data-testid=name]').setValue('New Screen')
    await wrapper.get('[data-testid=base-url]').setValue(screen.mcpUrl)
    await wrapper.get('[data-testid=credential]').setValue('test-only-secret')
    await wrapper.get('form').trigger('submit')
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    gate.resolve(screen); await flushPromises()
    expect(wrapper.find('[data-testid=connection-toast]').exists()).toBe(false)
    expect(wrapper.get('[data-testid=name]').element).toHaveProperty('value', 'New Screen')
    expect(wrapper.get('[data-testid=credential]').element).toHaveProperty('value', '')
  })
})
