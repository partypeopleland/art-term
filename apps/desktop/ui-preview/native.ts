// Preview-only aliases ensure even a newly added native button cannot invoke IPC.
export const isTauri = () => false
export async function invoke<T>(_command: string, _args?: unknown): Promise<T> {
  throw new Error('手機畫面預覽不提供原生功能。')
}
export async function addPluginListener<T>(_plugin: string, _event: string, _receive: (event: T) => void) {
  return { unregister: async () => {} }
}
