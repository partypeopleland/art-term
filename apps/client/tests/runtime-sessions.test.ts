import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import RuntimeSessions from '../src/shared/components/RuntimeSessions.vue'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from '../src/shared/transport/types'
import { FakeTransport, deferred, makeHost, makeTarget } from './fake'

const host: RuntimeHostDto = { id: 'runtime-profile', revision: 7, name: 'Fixture Host', baseUrl: 'https://runtime.example.test', credentialPresent: true }
const target = { ...makeTarget('issued-shell', host.id, 'host'), phase: 'detached', label: 'shell-one' }
const attached = { ...target, id: 'issued-attached', phase: 'attached', label: 'shell-two' }
const dead = { ...target, id: 'issued-dead', running: false, phase: 'dead', label: 'shell-dead' }
const group = makeHost(host.id, [target, attached, dead])
const wrappers: ReturnType<typeof mount>[] = []
function setup(history = true) {
  const transport: Transport = Object.assign(new FakeTransport([]), {
    runtimeSessions: vi.fn().mockResolvedValue(group),
    runtimeCreateSession: vi.fn().mockResolvedValue(target),
    ...(history ? { runtimeCreateHistorySession: vi.fn().mockResolvedValue(target) } : {})
  })
  const wrapper = mount(RuntimeSessions, { props: { host, transport } }); wrappers.push(wrapper)
  return { wrapper, transport }
}
async function button(wrapper: ReturnType<typeof mount>, label: string) {
  const node = wrapper.findAll('button').find(item => item.text() === label)
  expect(node, label).toBeDefined(); await node!.trigger('click')
}
afterEach(() => { for (const wrapper of wrappers.splice(0)) wrapper.unmount(); vi.restoreAllMocks() })

describe('explicit runtime Shell selection and creation', () => {
  it('offers one automatic tmux create action without a name field', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    expect(wrapper.find('input').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('建立主機歷史 Shell')
    expect(wrapper.findAll('button').filter(b => b.text() === '建立並連線')).toHaveLength(1)
    expect(transport.runtimeCreateHistorySession).not.toHaveBeenCalled()
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(transport.runtimeCreateHistorySession).toHaveBeenCalledExactlyOnceWith(host.id, host.revision)
    expect(transport.runtimeCreateSession).not.toHaveBeenCalled()
    expect(wrapper.emitted('open')?.[0][1]).toEqual(target)
  })
  it('does not fall back to a plain Shell when tmux creation is unavailable', async () => {
    const { wrapper, transport } = setup(false); await flushPromises()
    const create = wrapper.findAll('button').find(b => b.text() === '建立並連線')!
    expect(create.attributes('disabled')).toBeDefined()
    expect(transport.runtimeCreateSession).not.toHaveBeenCalled()
  })
  it('lists by exact profile revision without creating anything automatically', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    expect(transport.runtimeSessions).toHaveBeenCalledExactlyOnceWith(host.id, 7)
    expect(wrapper.text()).toContain('shell-one'); expect(wrapper.text()).toContain('shell-two')
    expect(transport.runtimeCreateSession).not.toHaveBeenCalled(); expect(wrapper.emitted('open')).toBeUndefined()
    expect(wrapper.text()).toContain('接管會中斷')
  })

  it('requires an explicit takeover click and emits the issued target unchanged', async () => {
    const { wrapper } = setup(); await flushPromises()
    await button(wrapper, '接管連線')
    expect(wrapper.emitted('open')).toEqual([[group, attached]])
    expect(wrapper.findAll('li')[2].get('button').attributes('disabled')).toBeDefined()
  })

  it('creates tmux only once on explicit submit with its exact profile revision', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    const pending = deferred<TargetDto>(); vi.mocked(transport.runtimeCreateHistorySession!).mockReturnValueOnce(pending.promise)
    await wrapper.get('form').trigger('submit')
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeCreateHistorySession).toHaveBeenCalledExactlyOnceWith(host.id, 7)
    pending.resolve(target); await flushPromises()
    const emitted = wrapper.emitted('open')!
    expect(emitted).toHaveLength(1); expect(emitted[0][1]).toEqual(target)
    expect(emitted[0][0]).toEqual({ id: host.id, label: host.name, desktopAvailable: false, targets: [target] })
  })

  it('creation uncertainty never retries automatically and instructs listing first', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    vi.mocked(transport.runtimeCreateHistorySession!).mockRejectedValueOnce(new Error('private fixture upstream response'))
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(transport.runtimeCreateHistorySession).toHaveBeenCalledTimes(1)
    expect(wrapper.emitted('open')).toBeUndefined(); expect(wrapper.text()).not.toContain('private fixture')
    expect(wrapper.get('[role="alert"]').text()).toContain('重新整理確認狀態')
    await button(wrapper, '重新整理 Shell'); await flushPromises()
    expect(transport.runtimeCreateHistorySession).toHaveBeenCalledTimes(1); expect(transport.runtimeSessions).toHaveBeenCalledTimes(2)
  })

  it('does not render raw errors from failed list requests', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    vi.mocked(transport.runtimeSessions!).mockRejectedValueOnce('Bearer fixture-sensitive')
    await button(wrapper, '重新整理 Shell'); await flushPromises()
    expect(wrapper.text()).not.toContain('Bearer'); expect(wrapper.get('[role="alert"]').text()).toContain('無法列出')
  })

  it('unmount discards a late create completion without opening a Shell', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    const pending = deferred<TargetDto>(); vi.mocked(transport.runtimeCreateHistorySession!).mockReturnValueOnce(pending.promise)
    await wrapper.get('form').trigger('submit')
    wrapper.unmount(); pending.resolve(target); await flushPromises()
    expect(wrapper.emitted('open')).toBeUndefined()
  })

  it('hidden completion is discarded without automatically creating or opening again', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    const pending = deferred<TargetDto>(); vi.mocked(transport.runtimeCreateHistorySession!).mockReturnValueOnce(pending.promise)
    await wrapper.get('form').trigger('submit')
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange')); pending.resolve(target); await flushPromises()
    visibility.mockReturnValue('visible'); document.dispatchEvent(new Event('visibilitychange')); await flushPromises()
    expect(wrapper.emitted('open')).toBeUndefined(); expect(transport.runtimeCreateHistorySession).toHaveBeenCalledTimes(1)
  })

  it('hide/foreground invalidates a late create promise even when it resolves while visible again', async () => {
    const { wrapper, transport } = setup(); await flushPromises()
    const pending = deferred<TargetDto>(); vi.mocked(transport.runtimeCreateHistorySession!).mockReturnValueOnce(pending.promise)
    await wrapper.get('form').trigger('submit')
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    visibility.mockReturnValue('visible'); document.dispatchEvent(new Event('visibilitychange'))
    pending.resolve(target); await flushPromises()
    expect(wrapper.emitted('open')).toBeUndefined()
  })

  it('hide/foreground invalidates a late list response', async () => {
    const pending = deferred<HostDto>()
    const transport = Object.assign(new FakeTransport([]), { runtimeSessions: vi.fn().mockReturnValueOnce(pending.promise), runtimeCreateSession: vi.fn() })
    const wrapper = mount(RuntimeSessions, { props: { host, transport } }); wrappers.push(wrapper)
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    visibility.mockReturnValue('visible'); document.dispatchEvent(new Event('visibilitychange'))
    pending.resolve(group); await flushPromises()
    expect(wrapper.text()).not.toContain('shell-one'); expect(wrapper.emitted('open')).toBeUndefined()
  })
})
