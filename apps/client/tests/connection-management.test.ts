import { afterEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import { flushPromises, mount } from '@vue/test-utils'
import RuntimeHosts from '../src/shared/components/RuntimeHosts.vue'
import ThemeSettings from '../src/shared/components/ThemeSettings.vue'
import { FakeTransport } from './fake'

const hosts = [
  { id: 'computer', revision: 1, name: 'Computer A', baseUrl: 'https://computer.example.test', credentialPresent: true, category: 'host' as const },
  { id: 'agent-a', revision: 1, name: 'Agent A', baseUrl: 'https://agent-a.example.test', credentialPresent: true, category: 'agent' as const },
  { id: 'agent-b', revision: 1, name: 'Agent B', baseUrl: 'https://agent-b.example.test', credentialPresent: true, category: 'agent' as const }
]
const screen = { id: 'screen', revision: 1, name: 'Screen A', mcpUrl: 'https://screen.example.test/mcp', tokenPresent: true }
const wrappers: ReturnType<typeof mount>[] = []
function setup() {
  const transport = Object.assign(new FakeTransport(), {
    runtimeHosts: vi.fn(async () => hosts), instanceDesktops: vi.fn(async () => [screen]),
    runtimeSaveHost: vi.fn(async () => hosts[0]!), instanceSave: vi.fn(async () => screen),
    runtimeTestHost: vi.fn(async () => ({ sessionCount: 1, draining: false, teardownBestEffort: false, continuityVerified: false })),
    instanceTest: vi.fn(async () => {}), runtimeReorderConnections: vi.fn(async () => {})
  })
  const wrapper = mount(RuntimeHosts, { props: { transport, embedded: true } })
  wrappers.push(wrapper)
  return { wrapper, transport }
}
afterEach(() => { wrappers.splice(0).forEach(wrapper => wrapper.unmount()); vi.useRealTimers(); vi.restoreAllMocks() })
describe('unified connection management', () => {
  it('lists every type in fixed groups and only reveals the common form after add', async () => {
    const { wrapper } = setup(); await flushPromises()
    expect(wrapper.find('form').exists()).toBe(false)
    expect(wrapper.findAll('[data-connection-id]').map(row => row.attributes('data-connection-id'))).toEqual(['computer', 'agent-a', 'agent-b', 'screen'])
    await wrapper.get('[data-testid=runtime-create]').trigger('click')
    expect(wrapper.findAll('input[type=radio]').map(input => input.attributes('value'))).toEqual(['host', 'agent', 'screen'])
    await wrapper.get('[data-testid=runtime-form-cancel]').trigger('click')
    expect(wrapper.find('form').exists()).toBe(false)
  })
  it('creates Screen through the same form and clears its token before awaiting native save', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    await wrapper.get('[data-testid=runtime-create]').trigger('click')
    await wrapper.get('input[value=screen]').setValue(true)
    await wrapper.get('[data-testid=name]').setValue('桌面')
    await wrapper.get('[data-testid=base-url]').setValue(screen.mcpUrl)
    await wrapper.get('[data-testid=credential]').setValue('private-test-token')
    await wrapper.get('form').trigger('submit')
    expect(transport.instanceSave).toHaveBeenCalledWith({ name: '桌面', mcpUrl: screen.mcpUrl, token: 'private-test-token' })
    await flushPromises()
    expect(wrapper.text()).not.toContain('private-test-token')
    expect(wrapper.find('form').exists()).toBe(false)
  })
  it('validates display names by Unicode characters and saves an empty override as default', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    await wrapper.get('[data-connection-id=computer] [data-action=edit]').trigger('click')
    await wrapper.get('[data-testid=display-name]').setValue('漢'.repeat(21))
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(wrapper.get('[data-testid=form-error]').text()).toContain('20')
    await wrapper.get('[data-testid=display-name]').setValue('漢'.repeat(20))
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(transport.runtimeSaveHost).toHaveBeenLastCalledWith(expect.objectContaining({ displayName: '漢'.repeat(20) }))
    await wrapper.get('[data-connection-id=computer] [data-action=edit]').trigger('click')
    await wrapper.get('[data-testid=display-name]').setValue('')
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(transport.runtimeSaveHost).toHaveBeenLastCalledWith(expect.objectContaining({ displayName: '' }))
  })
  it('persists only a complete within-type permutation and refreshes navigation', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    await wrapper.get('[data-connection-id=agent-b] [data-action=reorder]').trigger('keydown', { key: 'ArrowUp' })
    await flushPromises()
    expect(transport.runtimeReorderConnections).toHaveBeenCalledWith('agent', ['agent-a', 'agent-b'], ['agent-b', 'agent-a'])
    expect(wrapper.findAll('[data-connection-kind=agent]').map(row => row.attributes('data-connection-id'))).toEqual(['agent-b', 'agent-a'])
    expect(wrapper.emitted('changed')).toHaveLength(1)
    await wrapper.get('[data-connection-id=agent-b] [data-action=reorder]').trigger('keydown', { key: 'ArrowUp' })
    expect(transport.runtimeReorderConnections).toHaveBeenCalledTimes(1)
  })
  it('shows test results in an expiring toast without row messages or focus changes', async () => {
    vi.useFakeTimers()
    const { wrapper } = setup(); await flushPromises()
    await wrapper.get('[data-connection-id=screen] [data-action=test]').trigger('click'); await flushPromises()
    expect(wrapper.get('[data-testid=connection-toast]').text()).toContain('連線測試成功')
    expect(wrapper.get('[data-connection-id=screen]').text()).not.toContain('成功')
    await vi.advanceTimersByTimeAsync(3100)
    expect(wrapper.find('[data-testid=connection-toast]').exists()).toBe(false)
  })
  it('accepts touch drag within a group and rejects a drop over another type', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    const handle = wrapper.get('[data-connection-id=agent-b] [data-action=reorder]')
    const row = wrapper.get('[data-connection-id=agent-a]')
    const pointer = async (type: string, id: number) => {
      const event = new MouseEvent(type, { bubbles: true, button: 0, clientX: 100, clientY: -1 })
      Object.defineProperties(event, { pointerId: { value: id }, pointerType: { value: 'touch' } })
      handle.element.dispatchEvent(event); await nextTick()
    }
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: vi.fn(() => row.element) })
    await pointer('pointerdown', 7)
    await pointer('pointermove', 7)
    expect(row.classes()).toContain('drop-before')
    await pointer('pointerup', 7); await flushPromises()
    expect(transport.runtimeReorderConnections).toHaveBeenCalledWith('agent', ['agent-a', 'agent-b'], ['agent-b', 'agent-a'])
    vi.mocked(document.elementFromPoint).mockReturnValue(wrapper.get('[data-connection-id=computer]').element)
    await pointer('pointerdown', 8)
    await pointer('pointermove', 8)
    await pointer('pointerup', 8); await flushPromises()
    expect(transport.runtimeReorderConnections).toHaveBeenCalledTimes(1)
  })
  it('preserves visible order when native persistence rejects a stale reorder', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    transport.runtimeReorderConnections.mockRejectedValueOnce({ kind: 'stateChanged' })
    await wrapper.get('[data-connection-id=agent-b] [data-action=reorder]').trigger('keydown', { key: 'ArrowUp' }); await flushPromises()
    expect(wrapper.findAll('[data-connection-kind=agent]').map(row => row.attributes('data-connection-id'))).toEqual(['agent-a', 'agent-b'])
    expect(wrapper.emitted('changed')).toBeUndefined()
    expect(wrapper.get('[data-testid=connection-toast]').text()).toContain('[STATE_CHANGED]')
    await wrapper.get('[aria-label=關閉提示]').trigger('click')
    expect(wrapper.find('[data-testid=connection-toast]').exists()).toBe(false)
  })
  it('places Screen management in connections and has no separate Screen settings page', async () => {
    const { transport } = setup()
    const wrapper = mount(ThemeSettings, { props: { embedded: true, desktop: true, initialSection: 'connections', transport } }); wrappers.push(wrapper)
    await flushPromises()
    expect(wrapper.find('[data-testid=settings-screens]').exists()).toBe(false)
    expect(wrapper.find('[data-connection-id=screen]').exists()).toBe(true)
  })
})
