import { afterEach, expect, it, vi } from 'vitest'
import { deferred } from './fake'
const core = vi.hoisted(() => ({ invoke: vi.fn(), addPluginListener: vi.fn(), isTauri: () => true }))
vi.mock('@tauri-apps/api/core', () => core)
import { listenKeyboardVisibility } from '../src/mobile/history'
afterEach(() => vi.resetAllMocks())

it('keeps a newer native keyboard event when the initial status arrives late', async () => {
  const status = deferred<{ visible: boolean }>()
  let receive!: (value: { visible: boolean }) => void
  const unregister = vi.fn(async () => {})
  core.addPluginListener.mockImplementation(async (_plugin, _event, callback) => {
    receive = callback
    return { unregister }
  })
  core.invoke.mockReturnValue(status.promise)
  const visible: boolean[] = []
  const pending = listenKeyboardVisibility(value => visible.push(value))
  await Promise.resolve(); await Promise.resolve()
  receive({ visible: true })
  status.resolve({ visible: false })
  const remove = await pending
  expect(visible).toEqual([true])
  await remove()
  expect(unregister).toHaveBeenCalledOnce()
})

it('unregisters if native keyboard status cannot be read', async () => {
  const unregister = vi.fn(async () => {})
  core.addPluginListener.mockResolvedValue({ unregister })
  core.invoke.mockRejectedValue(new Error('unavailable'))
  await expect(listenKeyboardVisibility(() => {})).rejects.toThrow('unavailable')
  expect(unregister).toHaveBeenCalledOnce()
})
