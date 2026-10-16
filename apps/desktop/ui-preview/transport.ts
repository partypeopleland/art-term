import { ConnectionError, type RuntimeHostDto, type TargetDto, type Transport } from '../src/transport/types'

export type Scenario = 'populated' | 'empty' | 'error'
const bytes = (text: string) => [...new TextEncoder().encode(text)]

/** Fictional state owned by a single iframe. No fetch, IPC, or profile persistence. */
export function createPreviewTransport(scenario: Scenario = 'populated'): Transport {
  let counter = 0
  let hosts: RuntimeHostDto[] = scenario === 'empty' ? [] : [
    { id: 'demo-host', revision: 1, name: '開發主機（示範）', baseUrl: 'https://dev.example.test', credentialPresent: true, category: 'host' },
    { id: 'demo-agent', revision: 1, name: '工作助理（示範）', baseUrl: 'https://agent.example.test', credentialPresent: true, category: 'agent' }
  ]
  const shells = new Map<string, TargetDto[]>()
  const connections = new Map<string, { queue: number[] }>()
  const target = (hostId: string, name: string): TargetDto => ({
    id: `preview-target-${++counter}`, hostId, label: name, subtitle: '畫面預覽 · 不會連線',
    kind: 'host', running: true, phase: 'ready', desktopAvailable: false
  })
  for (const host of hosts) shells.set(host.id, (host.category === 'agent' ? ['coding-assistant'] : ['nuc-1', 'nuc-2', 'nuc-3']).map(name => target(host.id, name)))

  const profile = (id: string, revision: number) => {
    const host = hosts.find(h => h.id === id)
    if (!host || host.revision !== revision) throw new ConnectionError('stateChanged', '示範設定已變更，請重新整理。')
    return host
  }
  const connection = (id: string) => {
    const found = connections.get(id)
    if (!found) throw new ConnectionError('sessionMissing', '示範分頁已關閉。')
    return found
  }

  return {
    async runtimeHosts() {
      if (scenario === 'error') throw new ConnectionError('storage', '示範：無法讀取連線設定，請切換情境後重試。')
      return structuredClone(hosts)
    },
    async runtimeSaveHost(input) {
      const previous = input.id ? profile(input.id, input.expectedRevision ?? 0) : undefined
      const saved: RuntimeHostDto = {
        id: previous?.id ?? `demo-${++counter}`, revision: (previous?.revision ?? 0) + 1,
        name: input.name, baseUrl: input.baseUrl, category: input.category ?? input.metadata?.category ?? 'host',
        metadata: input.metadata, credentialPresent: Boolean(input.credential) || Boolean(previous?.credentialPresent)
      }
      // The credential itself is deliberately never retained.
      hosts = [...hosts.filter(h => h.id !== saved.id), saved]
      if (!shells.has(saved.id)) shells.set(saved.id, [])
      return structuredClone(saved)
    },
    async runtimeDeleteHost(id, revision) {
      profile(id, revision)
      hosts = hosts.filter(h => h.id !== id)
      shells.delete(id)
    },
    async runtimeTestHost(id, revision) {
      profile(id, revision)
      return { sessionCount: shells.get(id)?.length ?? 0, draining: false, teardownBestEffort: false, continuityVerified: true }
    },
    async runtimeSessions(id, revision) {
      const host = profile(id, revision)
      return { id, label: host.name, desktopAvailable: false, targets: structuredClone(shells.get(id) ?? []) }
    },
    async runtimeCreateSession(id, revision, name) {
      profile(id, revision)
      const created = target(id, name)
      shells.set(id, [...(shells.get(id) ?? []), created])
      return structuredClone(created)
    },
    async runtimeCreateHistorySession(id, revision) {
      profile(id, revision)
      const created = target(id, `nuc-${(shells.get(id)?.length ?? 0) + 1}`)
      shells.set(id, [...(shells.get(id) ?? []), created])
      return structuredClone(created)
    },
    async open(targetId) {
      const shell = [...shells.values()].flat().find(t => t.id === targetId)
      if (!shell) throw new ConnectionError('missing', '示範 Shell 不存在。')
      const id = `preview-session-${++counter}`
      connections.set(id, { queue: bytes('\x1b[36mart-term · 手機畫面預覽\x1b[0m\r\n\r\n所有連線與資料都是示範，不會執行指令。\r\n可試用分頁、方向鍵與「輸入」編輯區。\r\n\r\npreview@demo:~$ ') })
      return { id, targetId, label: shell.label, remoteSession: `openab-pty:demo-${shell.label}` }
    },
    async poll(id) {
      // Yield while idle so the shared controller does not spin the event loop.
      await new Promise(resolve => setTimeout(resolve, 80))
      const current = connections.get(id)
      return { bytes: current?.queue.splice(0) ?? [], exitCode: current ? null : 0 }
    },
    async write(id, input) {
      const current = connection(id)
      const text = new TextDecoder().decode(Uint8Array.from(input))
      // Echo printable text; Enter produces a canned result. No command execution.
      for (const character of text) {
        if (character === '\r' || character === '\n') current.queue.push(...bytes('\r\n[示範] 輸入已接收，未執行。\r\npreview@demo:~$ '))
        else if (character >= ' ' && character !== '\x7f') current.queue.push(...bytes(character))
      }
    },
    async resize() {},
    async close(id) { connections.delete(id) },
    async runtimeAck() {},
    async runtimeRecover(id) { connection(id) },
    async desktop() { return { enabled: false, message: '瀏覽器預覽不提供遠端桌面。' } },
    async screenshot() { throw new Error('瀏覽器預覽不提供遠端截圖。') },
    async transferExport() { return { status: 'cancelled' } },
    async transferPickImport() { return { status: 'cancelled', token: null, entries: [] } },
    async transferUnlock() { return { status: 'cancelled', token: null, entries: [] } },
    async transferApply() { return { added: 0, updated: 0, skipped: 0, profiles: structuredClone(hosts) } },
    async transferCancel() { return { status: 'cancelled' } }
  }
}
