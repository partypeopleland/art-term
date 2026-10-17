import { afterEach, describe, expect, it, vi } from 'vitest'
import { flushPromises, mount } from '@vue/test-utils'
import RuntimeHosts from '../src/components/RuntimeHosts.vue'
import App from '../src/App.vue'
import { createTauriTransport } from '../src/transport/tauri'
import type { RuntimeHostDto, RuntimeHostInput, RuntimeHostTestDto, Transport } from '../src/transport/types'
import { FakeTerm, FakeTransport, deferred } from './fake'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

const host: RuntimeHostDto = { id: 'host-1', revision: 3, name: 'Fixture', baseUrl: 'https://runtime.example.test', credentialPresent: true }
const wrappers: ReturnType<typeof mount>[] = []
function setup(initial: RuntimeHostDto[] = []) {
  const transport: Transport = Object.assign(new FakeTransport([]), {
    runtimeHosts: vi.fn().mockResolvedValue(initial),
    runtimeSaveHost: vi.fn().mockImplementation(async (input: RuntimeHostInput) => ({ ...host, name: input.name, baseUrl: input.baseUrl, revision: 4 })),
    runtimeDeleteHost: vi.fn().mockResolvedValue(undefined),
    runtimeTestHost: vi.fn().mockResolvedValue({ sessionCount: 2, draining: false, teardownBestEffort: true, continuityVerified: false })
  })
  const wrapper = mount(RuntimeHosts, { props: { transport } })
  wrappers.push(wrapper)
  return { wrapper, transport }
}
async function fields(wrapper: ReturnType<typeof mount>, secret = 'fixture-admin') {
  if (!wrapper.find('[data-testid=name]').exists()) await button(wrapper, '新增 openab-pty 連線')
  await wrapper.get('[data-testid="name"]').setValue('New Host')
  await wrapper.get('[data-testid="base-url"]').setValue('https://runtime.example.test:443/')
  await wrapper.get('[data-testid="credential"]').setValue(secret)
}
async function button(wrapper: ReturnType<typeof mount>, label: string) {
  const found = wrapper.findAll('button').find((item) => item.text() === label)
  expect(found, label).toBeDefined()
  await found!.trigger('click')
}
function secretValue(wrapper: ReturnType<typeof mount>) {
  return wrapper.find('[data-testid="credential"]').exists() ? (wrapper.get('[data-testid="credential"]').element as HTMLInputElement).value : ''
}
afterEach(() => {
  for (const wrapper of wrappers.splice(0)) wrapper.unmount()
  vi.restoreAllMocks()
})

describe('runtime Host settings', () => {
  it('uses exact native IPC commands and revision arguments', async () => {
    invoke.mockClear()
    const transport = createTauriTransport()
    const input = { name: 'Fixture', baseUrl: host.baseUrl, credential: 'fixture-only' }
    await transport.runtimeHosts!()
    await transport.runtimeSaveHost!(input)
    await transport.runtimeDeleteHost!(host.id, host.revision)
    await transport.runtimeTestHost!(host.id, host.revision)
    expect(invoke.mock.calls).toEqual([
      ['gui_runtime_hosts', undefined],
      ['gui_runtime_save_host', { input }],
      ['gui_runtime_delete_host', { id: host.id, expectedRevision: host.revision }],
      ['gui_runtime_test_host', { id: host.id, expectedRevision: host.revision }]
    ])
  })

  it('opens settings from Windows App and refreshes the profile list after a saved mutation', async () => {
    const { transport } = setup()
    const hosts = vi.spyOn(transport, 'runtimeHosts')
    const wrapper = mount(App, { props: { transport, terminalFactory: () => new FakeTerm() } })
    wrappers.push(wrapper)
    await flushPromises()
    expect(hosts).toHaveBeenCalled()
    hosts.mockClear()
    await wrapper.get('[data-testid="desktop-settings"]').trigger('click')
    await flushPromises()
    await wrapper.get('[data-testid="desktop-manage-hosts"]').trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="dialog"]').text()).toContain('新增 openab-pty 連線')
    await fields(wrapper)
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(hosts).toHaveBeenCalled()
    await button(wrapper, '關閉')
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
  })

  it('separates management and form, and saves the explicit category', async () => {
    const { wrapper, transport } = setup([host]); await flushPromises()
    expect(wrapper.find('[data-testid=runtime-host-list]').exists()).toBe(true)
    expect(wrapper.find('form').exists()).toBe(false)
    await fields(wrapper)
    expect(wrapper.find('[data-testid=runtime-host-list]').exists()).toBe(false)
    await wrapper.get('[data-testid=runtime-host-category]').setValue('agent')
    await wrapper.get('form').trigger('submit'); await flushPromises()
    expect(transport.runtimeSaveHost).toHaveBeenCalledWith(expect.objectContaining({ category: 'agent' }))
    expect(wrapper.find('[data-testid=runtime-host-list]').exists()).toBe(true)
    expect(wrapper.find('form').exists()).toBe(false)
  })

  it('contains keyboard focus, closes with Escape, and restores its opener', async () => {
    const opener = document.createElement('button'); document.body.append(opener); opener.focus()
    const { transport } = setup()
    const wrapper = mount(RuntimeHosts, { props: { transport, initialView: 'create' }, attachTo: document.body }); wrappers.push(wrapper)
    await flushPromises()
    const buttons = wrapper.findAll('button'), last = buttons.at(-1)!.element as HTMLButtonElement
    last.focus(); document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true }))
    expect(document.activeElement).toBe(buttons[0].element)
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }))
    expect(wrapper.emitted('close')).toHaveLength(1)
    wrapper.unmount(); expect(document.activeElement).toBe(opener); opener.remove()
  })

  it('masks the one-shot credential and clears it before awaiting native save', async () => {
    const { wrapper, transport } = setup()
    await flushPromises()
    await fields(wrapper)
    expect(wrapper.get('[data-testid="credential"]').attributes('type')).toBe('password')
    const pending = deferred<RuntimeHostDto>()
    vi.mocked(transport.runtimeSaveHost!).mockReturnValueOnce(pending.promise)
    await wrapper.get('form').trigger('submit')
    expect(secretValue(wrapper)).toBe('')
    expect(transport.runtimeSaveHost).toHaveBeenCalledWith({ name: 'New Host', baseUrl: 'https://runtime.example.test', credential: 'fixture-admin', category: 'host' })
    expect(wrapper.text()).not.toContain('fixture-admin')
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).toHaveBeenCalledTimes(1)
    pending.resolve(host)
    await flushPromises()
    expect(wrapper.emitted('changed')).toHaveLength(1)
    expect(wrapper.text()).toContain('設定已儲存')
  })

  it('preserves a blank edit credential for the same canonical endpoint and sends revision', async () => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    await button(wrapper, '編輯')
    await wrapper.get('[data-testid="base-url"]').setValue('https://runtime.example.test:443/')
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(transport.runtimeSaveHost).toHaveBeenCalledWith({ id: host.id, expectedRevision: 3, name: host.name, baseUrl: host.baseUrl, credential: '', category: 'host' })
  })

  it('edits a credential-absent TOML record without requiring a new credential at the same origin', async () => {
    const { wrapper, transport } = setup([{ ...host, credentialPresent: false }])
    await flushPromises()
    await button(wrapper, '編輯')
    await wrapper.get('[data-testid=name]').setValue('Metadata only')
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(transport.runtimeSaveHost).toHaveBeenCalledWith(expect.objectContaining({ id: host.id, credential: '', name: 'Metadata only' }))
  })

  it('requires a fresh credential for changed endpoint, even on edit', async () => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    await button(wrapper, '編輯')
    await wrapper.get('[data-testid="base-url"]').setValue('https://other.example.test')
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(wrapper.get('[role="alert"]').text()).toContain('新的管理憑證')
  })

  it.each(['http://runtime.example.test', 'https://user:secret@runtime.example.test', 'https://@runtime.example.test', 'https://runtime.example.test/api', 'https://runtime.example.test/?token=x', 'https://runtime.example.test/?', 'https://runtime.example.test/#x', 'https://runtime.example.test/#'])('rejects unsafe root URL %s and clears submitted secret', async (url) => {
    const { wrapper, transport } = setup()
    await flushPromises()
    await fields(wrapper)
    await wrapper.get('[data-testid="base-url"]').setValue(url)
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(secretValue(wrapper)).toBe('')
  })

  it('confirms local deletion and never implies remote Shell termination', async () => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    await button(wrapper, '刪除')
    expect(wrapper.text()).toContain('現有遠端 Shell 不會被終止')
    expect(transport.runtimeDeleteHost).not.toHaveBeenCalled()
    await wrapper.get('[data-testid="confirm-delete"]').trigger('click')
    await flushPromises()
    expect(transport.runtimeDeleteHost).toHaveBeenCalledWith(host.id, 3)
    expect(wrapper.find('[data-host-id="host-1"]').exists()).toBe(false)
    expect(wrapper.emitted('changed')).toHaveLength(1)
  })

  it.each(['/./', '/../', '//', '/%2e/', '/%2e%2e/', '/\\', '\\'])('rejects raw path normalization or backslash %s', async (suffix) => {
    const { wrapper, transport } = setup()
    await flushPromises()
    await fields(wrapper)
    await wrapper.get('[data-testid="base-url"]').setValue(`https://runtime.example.test${suffix}`)
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(secretValue(wrapper)).toBe('')
  })

  it('enforces Unicode name UTF-8 bytes and allows a trimmed root URL', async () => {
    const { wrapper, transport } = setup()
    await flushPromises()
    await fields(wrapper)
    await wrapper.get('[data-testid="name"]').setValue('漢'.repeat(43))
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(wrapper.get('[role="alert"]').text()).toContain('UTF-8')
    await wrapper.get('[data-testid="name"]').setValue('漢'.repeat(42))
    await wrapper.get('[data-testid="base-url"]').setValue('  https://runtime.example.test/  ')
    await wrapper.get('[data-testid="credential"]').setValue('fixture-admin')
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(transport.runtimeSaveHost).toHaveBeenCalledWith(expect.objectContaining({ name: '漢'.repeat(42), baseUrl: host.baseUrl }))
  })

  it('enforces credential UTF-8 byte bounds and the 64 Host limit', async () => {
    const { wrapper, transport } = setup(Array.from({ length: 64 }, (_, i) => ({ ...host, id: `host-${i}` })))
    await flushPromises()
    await fields(wrapper, '漢'.repeat(342))
    expect(wrapper.get('[data-testid="credential"]').attributes('maxlength')).toBe('1024')
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(wrapper.get('[role="alert"]').text()).toContain('1024')
    await wrapper.get('[data-testid="credential"]').setValue('fixture-admin')
    await wrapper.get('form').trigger('submit')
    expect(transport.runtimeSaveHost).not.toHaveBeenCalled()
    expect(wrapper.get('[role="alert"]').text()).toContain('64')
  })

  it('tests saved Host by id/revision and displays safe native summary', async () => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    await fields(wrapper)
    await button(wrapper, '取消')
    await button(wrapper, '測試')
    await flushPromises()
    expect(secretValue(wrapper)).toBe('')
    expect(transport.runtimeTestHost).toHaveBeenCalledWith(host.id, 3)
    expect(wrapper.text()).toContain('已通過管理認證；2 個 Shell')
    expect(wrapper.text()).toContain('輸出連續性未驗證')
    expect(wrapper.text()).toContain('清理為盡力而為')
  })

  it.each([
    ['auth', '[AUTH]', 'Bearer 前綴'],
    ['tls', '[TLS]', '日期'],
    ['timeout', '[TIMEOUT]', '逾時'],
    ['transport', '[TRANSPORT]', 'Tailnet'],
    ['storage', '[STORAGE]', '受保護'],
    ['stateChanged', '[STATE_CHANGED]', '重新開啟']
  ])('maps structured HostError kind %s to fixed diagnostic %s', async (kind, code, hint) => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    vi.mocked(transport.runtimeTestHost!).mockRejectedValueOnce({ kind, message: 'fixture-admin https://evil.fixture?token=fixture-admin' })
    await button(wrapper, '測試')
    await flushPromises()
    const alert = wrapper.get('[role="alert"]').text()
    expect(alert).toContain(code)
    expect(alert).toContain(hint)
    expect(wrapper.text()).not.toContain('fixture-admin')
    expect(wrapper.text()).not.toContain('evil.fixture')
    expect(secretValue(wrapper)).toBe('')
  })

  it('maps a structured IPC rejection through the real Tauri transport', async () => {
    invoke.mockReset()
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'gui_runtime_hosts') return [host]
      throw { kind: 'auth', message: 'Bearer fixture-admin https://evil.fixture' }
    })
    const wrapper = mount(RuntimeHosts, { props: { transport: createTauriTransport() } })
    wrappers.push(wrapper)
    try {
      await flushPromises()
      await button(wrapper, '測試')
      await flushPromises()
      const alert = wrapper.get('[role="alert"]').text()
      expect(alert).toContain('[AUTH]')
      expect(alert).toContain('Bearer 前綴')
      expect(wrapper.text()).not.toContain('fixture-admin')
      expect(wrapper.text()).not.toContain('evil.fixture')
    } finally {
      invoke.mockReset().mockResolvedValue(undefined)
    }
  })

  it.each([
    ['a raw Error', new Error('Bearer fixture-admin private response')],
    ['an unknown kind', { kind: 'attackerControlled', message: 'fixture-admin https://evil.fixture' }],
    ['a non-string kind', { kind: 7, message: 'fixture-admin' }],
    ['plain text', 'fixture-admin raw failure']
  ])('keeps the generic test fallback for %s without leaking payload', async (_label, rejection) => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    vi.mocked(transport.runtimeTestHost!).mockRejectedValueOnce(rejection)
    await button(wrapper, '測試')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toBe('Host 測試失敗。請確認連線、TLS 憑證及管理憑證。')
    expect(wrapper.text()).not.toContain('fixture-admin')
    expect(wrapper.text()).not.toContain('evil.fixture')
  })

  it('maps a structured save rejection to its fixed diagnostic', async () => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    vi.mocked(transport.runtimeSaveHost!).mockRejectedValueOnce({ kind: 'stateChanged', message: 'fixture-admin' })
    await button(wrapper, '編輯')
    await wrapper.get('[data-testid="credential"]').setValue('fixture-admin')
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain('[STATE_CHANGED]')
    expect(wrapper.text()).not.toContain('fixture-admin')
    expect(secretValue(wrapper)).toBe('')
  })

  it('ignores a stale test rejection after the form was reset', async () => {
    const { wrapper, transport } = setup([host])
    await flushPromises()
    const pending = deferred<RuntimeHostTestDto>()
    vi.mocked(transport.runtimeTestHost!).mockReturnValueOnce(pending.promise)
    await button(wrapper, '測試')
    await button(wrapper, '關閉')
    pending.reject({ kind: 'auth', message: 'fixture-admin' })
    await flushPromises()
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
    expect(wrapper.text()).not.toContain('AUTH')
    expect(wrapper.text()).not.toContain('fixture-admin')
  })

  it('sanitizes rejected native errors and clears credential on failure', async () => {
    const { wrapper, transport } = setup()
    await flushPromises()
    await fields(wrapper)
    vi.mocked(transport.runtimeSaveHost!).mockRejectedValueOnce(new Error('Bearer fixture-admin private response'))
    await wrapper.get('form').trigger('submit')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).not.toContain('Bearer')
    expect(wrapper.text()).not.toContain('fixture-admin')
    expect(secretValue(wrapper)).toBe('')
    expect(wrapper.get('[role="alert"]').text()).toContain('無法儲存')
  })

  it('cancel clears credential and stale mutation cannot replace the newer form/list', async () => {
    const { wrapper, transport } = setup()
    await flushPromises()
    const pending = deferred<RuntimeHostDto>()
    vi.mocked(transport.runtimeSaveHost!).mockReturnValueOnce(pending.promise)
    await fields(wrapper)
    await wrapper.get('form').trigger('submit')
    await button(wrapper, '取消')
    await fields(wrapper, 'new-secret')
    pending.resolve(host)
    await flushPromises()
    expect(wrapper.find('[data-host-id="host-1"]').exists()).toBe(false)
    expect((wrapper.get('[data-testid="name"]').element as HTMLInputElement).value).toBe('New Host')
    expect(secretValue(wrapper)).toBe('')
    expect(wrapper.text()).not.toContain('設定已儲存')
  })

  it('reopening loads current settings without a refresh button or stale load', async () => {
    const pending = deferred<RuntimeHostDto[]>()
    const transport = Object.assign(new FakeTransport([]), {
      runtimeHosts: vi.fn().mockReturnValueOnce(pending.promise).mockResolvedValueOnce([{ ...host, name: 'Current' }]),
      runtimeSaveHost: vi.fn(), runtimeDeleteHost: vi.fn(), runtimeTestHost: vi.fn()
    })
    const wrapper = mount(RuntimeHosts, { props: { transport } })
    expect(wrapper.find('[data-testid="refresh"]').exists()).toBe(false)
    await button(wrapper, '關閉')
    wrapper.unmount()
    const reopened = mount(RuntimeHosts, { props: { transport } })
    wrappers.push(reopened)
    await flushPromises()
    pending.resolve([{ ...host, name: 'Stale' }])
    await flushPromises()
    expect(reopened.text()).toContain('Current')
    expect(reopened.text()).not.toContain('Stale')
    expect(reopened.find('[data-testid=runtime-host-list]').exists()).toBe(true)
    expect(reopened.find('form').exists()).toBe(false)
  })

  it('clears credential on cancellation, document hiding, close and unmount', async () => {
    const { wrapper } = setup()
    await flushPromises()
    await fields(wrapper)
    await button(wrapper, '取消')
    expect(secretValue(wrapper)).toBe('')
    await fields(wrapper)
    const visibility = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    document.dispatchEvent(new Event('visibilitychange'))
    await flushPromises()
    expect(secretValue(wrapper)).toBe('')
    visibility.mockRestore()
    await fields(wrapper)
    await button(wrapper, '關閉')
    expect(secretValue(wrapper)).toBe('')
    expect(wrapper.emitted('close')).toHaveLength(1)
    await fields(wrapper)
    const retained = wrapper.get('[data-testid="credential"]').element as HTMLInputElement
    wrapper.unmount()
    // No component state retains the secret; detached DOM is scrubbed as well.
    expect(retained.value).toBe('')
  })

})
