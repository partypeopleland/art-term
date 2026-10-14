import { addPluginListener, invoke } from '@tauri-apps/api/core'
import type { NativeHistoryEvent, NativeHistoryReader } from '../terminal/native'

export function createNativeHistoryReader(): NativeHistoryReader {
  let epoch = 0
  let remove: (() => Promise<void>) | null = null
  return {
    async open(snapshot, receive) {
      const ticket = ++epoch
      const listener = await addPluginListener<NativeHistoryEvent>('mobile-history', 'history', receive)
      if (ticket !== epoch) { await listener.unregister(); return }
      remove = () => listener.unregister()
      try {
        await invoke('plugin:mobile-history|open', snapshot as unknown as Record<string, unknown>)
        if (ticket !== epoch) await invoke('plugin:mobile-history|close', { readerId: snapshot.readerId })
      } catch (error) {
        await listener.unregister(); if (ticket === epoch) remove = null
        throw error
      }
    },
    update: snapshot => invoke('plugin:mobile-history|update', snapshot as unknown as Record<string, unknown>),
    async close(readerId) {
      epoch++
      const previous = remove; remove = null
      try { await invoke('plugin:mobile-history|close', { readerId }) }
      finally { await previous?.() }
    }
  }
}
