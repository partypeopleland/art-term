import { afterEach, expect, it, vi } from 'vitest'

const { invoke, createApp, mount, transport, desktopApp } = vi.hoisted(() => {
  const mount = vi.fn()
  return {
    invoke: vi.fn(),
    mount,
    createApp: vi.fn(() => ({ mount })),
    transport: {},
    desktopApp: {}
  }
})
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('vue', () => ({ createApp }))
vi.mock('../src/desktop/App.vue', () => ({ default: desktopApp }))
vi.mock('../src/desktop/tauri', () => ({ createTauriTransport: () => transport }))
vi.mock('../src/shared/terminal/handle', () => ({
  createDesktopXtermTerminal: vi.fn(),
  createMobileXtermTerminal: vi.fn()
}))
afterEach(() => { vi.clearAllMocks(); vi.resetModules(); document.body.innerHTML = '' })

it.each(['linux', 'windows'])('mounts the shared desktop application for %s', async platform => {
  invoke.mockResolvedValue(platform)
  await import('../src/main')
  await vi.waitFor(() => expect(mount).toHaveBeenCalledExactlyOnceWith('#app'))
  expect(invoke).toHaveBeenCalledExactlyOnceWith('gui_platform')
  expect(createApp).toHaveBeenCalledExactlyOnceWith(desktopApp, expect.objectContaining({ platform, transport }))
})

it('shows startup failure for an unsupported platform without mounting an app', async () => {
  document.body.innerHTML = '<div id="app"></div>'
  invoke.mockResolvedValue('unsupported')
  await import('../src/main')
  await vi.waitFor(() => expect(document.getElementById('app')?.textContent).toContain('無法啟動'))
  expect(createApp).not.toHaveBeenCalled()
})
