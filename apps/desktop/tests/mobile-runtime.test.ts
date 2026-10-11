import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import MobileApp from '../src/mobile/MobileApp.vue'
import { createMobileInput } from '../src/mobile/input'
import type { AppController } from '../src/state/controller'
import type { RuntimeHostDto } from '../src/transport/types'
import { FakeMobileTransport, makeCatalogTarget, makeGroup } from './mobile.fake'
import { FakeTerm, deferred } from './fake'

const profile: RuntimeHostDto = { id: 'profile-safe', revision: 7, name: 'Fixture Runtime', baseUrl: 'https://runtime.example.test', credentialPresent: true }
const target = { ...makeCatalogTarget('runtime-issued-safe', profile.id, 'shell-one', '', 'host'), phase: 'attached' }
const group = makeGroup(profile.id, [target], profile.name)
class RuntimeMobile extends FakeMobileTransport {
  runtimeHosts = vi.fn(async () => [profile])
  runtimeSaveHost = vi.fn(async () => profile)
  runtimeDeleteHost = vi.fn(async () => {})
  runtimeTestHost = vi.fn()
  runtimeSessions = vi.fn(async () => group)
  runtimeCreateSession = vi.fn(async () => target)
  runtimeAck = vi.fn(async (_id: string, _batch: number, _generation: number) => {})
  runtimeRecover = vi.fn(async (_id: string) => {})
}
const wrappers: ReturnType<typeof mount>[] = []
async function flush() { for (let i = 0; i < 50; i++) { await Promise.resolve(); await nextTick() } }
function visibility(hidden: boolean) {
  Object.defineProperty(document, 'hidden', { configurable: true, value: hidden })
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: hidden ? 'hidden' : 'visible' })
  document.dispatchEvent(new Event('visibilitychange'))
}
function setup() {
  const transport = new RuntimeMobile()
  const terms: FakeTerm[] = []
  const wrapper = mount(MobileApp, { props: { transport, terminalFactory: () => { const term = new FakeTerm(); terms.push(term); return term }, controllerOptions: { idlePollMs: 0 } }, attachTo: document.body })
  wrappers.push(wrapper)
  return { wrapper, transport, terms, controller: (wrapper.vm as unknown as { controller: AppController }).controller }
}
async function button(wrapper: ReturnType<typeof mount>, label: string) {
  const found = wrapper.findAll('button').find(node => node.text() === label)
  expect(found).toBeDefined(); await found!.trigger('click'); await flush()
}
beforeEach(() => {
  visibility(false)
  vi.stubGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} })
})
afterEach(() => { for (const wrapper of wrappers.splice(0)) wrapper.unmount(); visibility(false); vi.restoreAllMocks() })

describe('mobile shared runtime settings and Shell routing', () => {
  it('opens listed attached Shell only after explicit takeover, retaining shared session/terminal and historical ACK', async () => {
    const { wrapper, transport, terms, controller } = setup(); await flush()
    await wrapper.get('[data-testid="runtime-host-manage"]').trigger('click'); await flush()
    await button(wrapper, 'Shell')
    expect(transport.runtimeSessions).toHaveBeenCalledExactlyOnceWith(profile.id, 7)
    expect(transport.openCalls).toEqual([]); expect(transport.runtimeCreateSession).not.toHaveBeenCalled()
    await button(wrapper, '接管連線')
    expect(transport.openCalls).toHaveLength(1); expect(transport.openCalls[0].targetId).toBe(target.id)
    expect(wrapper.find('.runtime-hosts').exists()).toBe(false)
    const tab = controller.tabs[0], session = tab.sessionId!
    const write = vi.spyOn(terms[0], 'write')
    terms[0].holdWrites = true
    transport.feed(session, { bytes: [], exitCode: null, runtime: { batchId: 17, localGeneration: 9, events: [{ kind: 'bytes', bytes: [65], start: 0, end: 1, historical: true }] } })
    await flush(); expect(write).toHaveBeenCalledWith(Uint8Array.from([65]), true); expect(transport.runtimeAck).not.toHaveBeenCalled()
    terms[0].releaseWrites(); await flush()
    expect(transport.runtimeAck).toHaveBeenCalledExactlyOnceWith(session, 17, 9)
    visibility(true); await flush()
    transport.feed(session, { bytes: [], exitCode: null, runtime: { batchId: 0, localGeneration: 9, events: [], ended: 'transport' } }); await flush()
    expect(transport.runtimeRecover).not.toHaveBeenCalled()
    visibility(false); await flush()
    expect(transport.runtimeRecover).toHaveBeenCalledExactlyOnceWith(session)
    expect(tab.sessionId).toBe(session); expect(terms).toHaveLength(1); expect(terms[0].resetCount).toBe(0)
    expect(transport.openCalls).toHaveLength(1); expect(transport.sessions.get(session)!.writes).toEqual([])
  })

  it('masks credentials, clears the actual input before awaiting save and discards stale completion after background', async () => {
    const { wrapper, transport } = setup(); await flush()
    await wrapper.get('[data-testid="runtime-host-settings"]').trigger('click'); await flush()
    await wrapper.get('[data-testid="name"]').setValue('新主機')
    await wrapper.get('[data-testid="base-url"]').setValue('https://runtime.example.test')
    const credential = wrapper.get('[data-testid="credential"]')
    expect(credential.attributes('type')).toBe('password')
    await credential.setValue('fixture-only-secret')
    const pending = deferred<RuntimeHostDto>(); transport.runtimeSaveHost.mockReturnValueOnce(pending.promise)
    await wrapper.get('.runtime-hosts form').trigger('submit')
    expect((credential.element as HTMLInputElement).value).toBe('')
    visibility(true); await flush(); expect(wrapper.find('.runtime-hosts').exists()).toBe(false)
    visibility(false); pending.resolve(profile); await flush()
    expect(wrapper.find('.runtime-hosts').exists()).toBe(false)
    await wrapper.get('[data-testid="runtime-host-settings"]').trigger('click'); await flush()
    expect((wrapper.get('[data-testid="credential"]').element as HTMLInputElement).value).toBe('')
    expect(transport.openCalls).toEqual([]); expect(transport.runtimeCreateSession).not.toHaveBeenCalled()
  })

  it('discards successful create completion after settings closes without opening or retrying', async () => {
    const { wrapper, transport } = setup(); await flush()
    await wrapper.get('[data-testid="runtime-host-manage"]').trigger('click'); await flush(); await button(wrapper, 'Shell')
    const pending = deferred<typeof target>(); transport.runtimeCreateSession.mockReturnValueOnce(pending.promise)
    await wrapper.get('.runtime-sessions input').setValue('new-shell')
    await wrapper.get('.runtime-sessions form').trigger('submit')
    await button(wrapper, '關閉'); pending.resolve(target); await flush()
    expect(transport.runtimeCreateSession).toHaveBeenCalledExactlyOnceWith(profile.id, 7, 'new-shell')
    expect(transport.openCalls).toEqual([])
  })

  it('clears composer and the actual terminal composition on visibility changes without replay', async () => {
    const { wrapper, transport, terms, controller } = setup(); await flush()
    await wrapper.get('[data-testid="runtime-host-manage"]').trigger('click'); await flush(); await button(wrapper, 'Shell'); await button(wrapper, '接管連線')
    const terminal = wrapper.get('.term-host').element, session = controller.tabs[0].sessionId!
    terminal.dispatchEvent(new CompositionEvent('compositionstart')); terms[0].emitData('舊')
    await wrapper.get('[data-testid="k-composer"]').trigger('click')
    await wrapper.get('[data-testid="composer-text"]').setValue('未送指令')
    await wrapper.get('[data-testid="k-ctrl"]').trigger('click')
    visibility(true); await flush()
    expect(wrapper.find('[data-testid="composer"]').exists()).toBe(false)
    visibility(false); await flush()
    terminal.dispatchEvent(new CompositionEvent('compositionend', { data: '舊組字' })); terms[0].emitData('舊組字'); await flush()
    expect(transport.sessions.get(session)!.writes).toEqual([])
    expect(wrapper.get('[data-testid="k-ctrl"]').classes()).not.toContain('on')
    await wrapper.get('[data-testid="k-composer"]').trigger('click')
    expect((wrapper.get('[data-testid="composer-text"]').element as HTMLTextAreaElement).value).toBe('')
    terminal.dispatchEvent(new CompositionEvent('compositionstart')); terms[0].emitData('新')
    terminal.dispatchEvent(new CompositionEvent('compositionend', { data: '新組字' })); terms[0].emitData('新組字'); await flush()
    expect(transport.sessions.get(session)!.writes.map(bytes => new TextDecoder().decode(Uint8Array.from(bytes)))).toEqual(['新組字'])
    expect(transport.openCalls).toHaveLength(1)
  })
})

describe('background composition cancellation', () => {
  it('discards hidden fragments and late compositionend/echo after resume, then accepts a fresh composition', () => {
    const input = createMobileInput(), real = new FakeTerm(), el = document.createElement('div')
    const wrapped = input.wrap(el, real), received: string[] = []
    wrapped.onData(text => received.push(text)); input.toggleCtrl()
    el.dispatchEvent(new CompositionEvent('compositionstart')); real.emitData('舊')
    input.setSuspended(true); expect(input.ctrlLatch.value).toBe(false); real.emitData('背景')
    input.setSuspended(false)
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'x' })); real.emitData('舊 callback')
    el.dispatchEvent(new CompositionEvent('compositionend', { data: '舊輸入' })); real.emitData('舊輸入')
    expect(received).toEqual([])
    el.dispatchEvent(new CompositionEvent('compositionstart')); real.emitData('新')
    el.dispatchEvent(new CompositionEvent('compositionend', { data: '新輸入' })); real.emitData('新輸入')
    expect(received).toEqual(['新輸入']); wrapped.dispose()
  })
  it('forwards historical flag and suppresses callbacks after disposal', async () => {
    const input = createMobileInput(), real = new FakeTerm(), el = document.createElement('div')
    const wrapped = input.wrap(el, real), write = vi.spyOn(real, 'write'), callback = vi.fn()
    wrapped.onData(callback); await wrapped.write(Uint8Array.from([65]), true)
    expect(write).toHaveBeenCalledWith(Uint8Array.from([65]), true)
    wrapped.dispose(); input.setSuspended(true); input.setSuspended(false); real.emitData('late')
    expect(callback).not.toHaveBeenCalled()
  })
})
