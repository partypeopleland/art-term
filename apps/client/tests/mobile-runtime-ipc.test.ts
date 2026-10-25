// @vitest-environment node
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createMobileTauriTransport } from '../src/mobile/tauri'
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
beforeEach(() => invoke.mockReset().mockResolvedValue(undefined))
describe('Android uses the shared runtime IPC contract', () => {
  it('desktop uses the native session IPC without SSH authorization commands', async () => {
    const transport = createMobileTauriTransport()
    await transport.desktop('runtime-session-issued', true)
    expect(invoke.mock.calls).toEqual([['mobile_desktop', { sessionId: 'runtime-session-issued', enabled: true }]])
    expect(transport).not.toHaveProperty('desktopEnroll')
    expect(transport).not.toHaveProperty('desktopCancel')
  })
  it('passes exact revisions, grants, batches and local session identifiers without deriving target ids', async () => {
    const transport = createMobileTauriTransport()
    const input = { name: 'Fixture', baseUrl: 'https://runtime.example.test', credential: 'fixture-only', id: 'profile', expectedRevision: 7 }
    await transport.runtimeHosts!()
    await transport.runtimeSaveHost!(input)
    await transport.runtimeDeleteHost!('profile', 7)
    await transport.runtimeTestHost!('profile', 7)
    await transport.runtimeSessions!('profile', 7)
    await transport.runtimeCreateSession!('profile', 7, 'shell-1')
    await transport.runtimeAck!('runtime-session-issued', 17, 9)
    await transport.runtimeRecover!('runtime-session-issued')
    await transport.open('runtime-target-issued', 24, 80)
    await transport.poll('runtime-session-issued')
    await transport.write('runtime-session-issued', [3])
    await transport.resize('runtime-session-issued', 30, 90)
    await transport.close('runtime-session-issued')
    expect(invoke.mock.calls).toEqual([
      ['gui_runtime_hosts', undefined], ['gui_runtime_save_host', { input }],
      ['gui_runtime_delete_host', { id: 'profile', expectedRevision: 7 }],
      ['gui_runtime_test_host', { id: 'profile', expectedRevision: 7 }],
      ['gui_runtime_sessions', { id: 'profile', expectedRevision: 7 }],
      ['gui_runtime_create_session', { id: 'profile', expectedRevision: 7, name: 'shell-1' }],
      ['gui_runtime_ack', { sessionId: 'runtime-session-issued', batchId: 17, localGeneration: 9 }],
      ['gui_runtime_recover', { sessionId: 'runtime-session-issued' }],
      ['mobile_open', { targetId: 'runtime-target-issued', rows: 24, cols: 80 }],
      ['mobile_poll', { sessionId: 'runtime-session-issued' }],
      ['mobile_write', { sessionId: 'runtime-session-issued', bytes: [3] }],
      ['mobile_resize', { sessionId: 'runtime-session-issued', rows: 30, cols: 90 }],
      ['mobile_close', { sessionId: 'runtime-session-issued' }]
    ])
  })
  it.each(['ttlExpired', 'operatorKilled', 'superseded', 'stateChanged'])('preserves typed %s verbatim', async (kind) => {
    invoke.mockRejectedValueOnce({ kind, message: '固定安全錯誤' })
    await expect(createMobileTauriTransport().open('runtime-target-issued', 24, 80)).rejects.toMatchObject({ kind, message: '固定安全錯誤' })
    expect(invoke).toHaveBeenCalledTimes(1)
  })
})
