import type { InjectionKey } from 'vue'
import type { NativeTerminalLine } from './native-buffer'
export interface NativeHistorySnapshot {
  foreground?: number
  background?: number
  readerId: string
  records: NativeTerminalLine[]
  visible: boolean
  followLatest?: boolean
  sourceEpoch?: string
  bounds: { x: number; y: number; width: number; height: number; viewportWidth: number }
}
export interface NativeHistoryEvent {
  readerId: string
  kind: 'tap' | 'visibility' | 'viewport' | 'position' | 'reading' | 'swipe'
  phase?: NativeSwipeEvent['phase']
  dx?: number
  sourceEpoch?: string
  visible?: boolean
  anchorId?: number
  anchorCharacter?: number
  anchorPixel?: number
  firstId?: number
  lastId?: number
  atBottom?: boolean
  nearTop?: boolean
  nearBottom?: boolean
}
export interface NativeHistoryReader {
  open(snapshot: NativeHistorySnapshot, receive: (event: NativeHistoryEvent) => void): Promise<void>
  update(snapshot: NativeHistorySnapshot): Promise<void>
  close(readerId: string): Promise<void>
}
export const NativeHistoryKey: InjectionKey<() => NativeHistoryReader> = Symbol('NativeTerminalReader')
export interface NativeSwipeEvent { phase: 'start' | 'move' | 'end' | 'cancel'; dx: number }
export const NativeSwipeKey: InjectionKey<(event: NativeSwipeEvent) => void> = Symbol('NativeWorkspaceSwipe')
export type NativeKeyboardListener = (receive: (visible: boolean) => void) => Promise<() => Promise<void>>
export const NativeKeyboardKey: InjectionKey<NativeKeyboardListener> = Symbol('NativeKeyboardVisibility')

export type HostHistoryRequest = { op: 'viewport' | 'close' }
  | { op: 'older'; instance: string; epoch: number; before: number; limit: number }
  | { op: 'ack' | 'keep'; instance: string; epoch: number }
export interface HostHistoryPage {
  kind: 'page'; instance: string; epoch: number; columns: number; rows: number
  rowCount: number; start: number; end: number; hasOlder: boolean; records: NativeTerminalLine[]
}
export type HostHistoryReply = HostHistoryPage | { kind: 'unsupported' }
  | { kind: 'ack'; instance: string; op: string; epoch?: number }
  | { kind: 'error'; error: string }
export type HostHistoryRequestFn = (request: HostHistoryRequest) => Promise<HostHistoryReply>

/** Two immutable epochs mirror the Host's displayed + in-flight lease. */
export class HostHistoryWindow {
  page: HostHistoryPage | null = null
  unsupported = false
  resolved = false
  private fallback = false
  held = false
  private closed = false
  private refreshing = false
  private lastRefresh = -Infinity
  private lastKeep = -Infinity
  private acknowledged = ''
  private shown = ''
  private windows = new Map<string, HostHistoryPage>()
  private queue: Promise<unknown> = Promise.resolve()
  private prefetched = new Map<string, Promise<HostHistoryReply>>()
  constructor(private request: HostHistoryRequestFn, private notice: (text: string) => void = () => {}) {}
  static key(page: Pick<HostHistoryPage, 'instance' | 'epoch'>): string { return `${page.instance}:${page.epoch}` }
  get sourceEpoch(): string | undefined { return this.page ? HostHistoryWindow.key(this.page) : undefined }
  get visiblePage(): HostHistoryPage | null { return this.fallback ? null : this.page }
  private olderKey(p: HostHistoryPage, before: number): string { return `${HostHistoryWindow.key(p)}:${before}` }
  /** Optional desktop look-ahead. Cached pages never change the displayed epoch or hold state. */
  async prefetchOlder(allowed: () => boolean): Promise<void> {
    const p = this.page
    if (!p || this.closed || !allowed()) return
    const epoch = HostHistoryWindow.key(p)
    const prefix = `${epoch}:`
    for (const key of this.prefetched.keys()) {
      const before = Number(key.slice(prefix.length))
      if (!key.startsWith(prefix) || before > p.start || before < p.start - 512) this.prefetched.delete(key)
    }
    let before = p.start
    for (let i = 0; i < 3 && before > 0; i++) {
      if (this.closed || !allowed() || this.sourceEpoch !== epoch || this.page?.start !== p.start) return
      const key = this.olderKey(p, before)
      let pending = this.prefetched.get(key)
      if (!pending) {
        if (this.prefetched.size >= 3) return
        pending = this.call({ op: 'older', instance: p.instance, epoch: p.epoch, before, limit: 256 }).catch(() => ({ kind: 'error', error: 'unavailable' } as HostHistoryReply))
        this.prefetched.set(key, pending)
      }
      const reply = await pending
      if (reply.kind !== 'page' || HostHistoryWindow.key(reply) !== epoch || reply.rowCount !== p.rowCount || reply.end !== before || reply.start >= before) {
        this.prefetched.delete(key); return
      }
      before = reply.start
    }
  }
  private call(request: HostHistoryRequest): Promise<HostHistoryReply> {
    const next = this.queue.catch(() => {}).then(() => this.closed && request.op !== 'close'
      ? { kind: 'unsupported' } as HostHistoryReply : this.request(request))
    this.queue = next
    return next
  }
  private error(reply?: HostHistoryReply): void {
    if (reply?.kind === 'error' && reply.error === 'busy') return
    this.notice(reply?.kind === 'error' && reply.error === 'expired'
      ? '這份主機歷史已過期；目前畫面仍保留，回到底部可讀取最新內容。'
      : '主機歷史暫時無法讀取；目前畫面仍保留，請稍後重試。')
  }
  async refresh(): Promise<void> {
    if (this.closed || this.unsupported || this.refreshing) return
    if (this.held && this.page) {
      if (Date.now() - this.lastKeep < 30000) return
      this.lastKeep = Date.now()
      try { const p = this.page; const reply = await this.call({ op: 'keep', instance: p.instance, epoch: p.epoch }); if (reply.kind === 'error') this.error(reply) } catch { this.error() }
      return
    }
    if (Date.now() - this.lastRefresh < 500) return
    this.lastRefresh = Date.now(); this.refreshing = true
    try {
      const reply = await this.call({ op: 'viewport' })
      if (this.closed) return
      if (reply.kind === 'unsupported') { this.resolved = true; this.unsupported = true; this.fallback = true; return }
      if (reply.kind !== 'page') {
        if (reply.kind === 'error') {
          if (reply.error !== 'busy') this.resolved = true
          this.error(reply)
          if (reply.error === 'unsupported') { this.fallback = true; this.held = false }
          if (!this.page && reply.error === 'unavailable') { this.unsupported = true; this.fallback = true }
        }
        return
      }
      // A gesture may begin while the Host reply is in flight. Keep the
      // actual displayed epoch; never join its records with the candidate.
      const offered = HostHistoryWindow.key(reply)
      const shown = this.shown || this.sourceEpoch
      this.windows.set(offered, reply)
      for (const key of this.windows.keys()) if (key !== offered && key !== shown) this.windows.delete(key)
      this.resolved = true
      if (!this.held) { this.page = reply; this.fallback = false }
    } catch {
      if (!this.closed) {
        this.resolved = true; this.unsupported = true
        this.notice('主機歷史連線中斷；目前畫面仍保留，請關閉分頁後再接回。')
        if (!this.page) this.fallback = true
      }
    } finally { this.refreshing = false }
  }
  hold(sourceEpoch: string | undefined): void {
    const shown = sourceEpoch ? this.windows.get(sourceEpoch) : undefined
    if (!shown) return
    this.page = shown; this.held = true
  }
  async displayed(sourceEpoch: string | undefined): Promise<void> {
    if (!this.page || sourceEpoch !== this.sourceEpoch || this.closed) return
    this.shown = sourceEpoch!
    if (sourceEpoch === this.acknowledged) return
    const p = this.page
    try {
      const reply = await this.call({ op: 'ack', instance: p.instance, epoch: p.epoch })
      if (reply.kind === 'ack' && !this.closed) {
        this.acknowledged = sourceEpoch!
        // Keep a newer candidate if it arrived during the display callback.
        for (const key of this.windows.keys()) if (key !== sourceEpoch && key !== this.sourceEpoch) this.windows.delete(key)
      } else if (reply.kind === 'error') this.error(reply)
    } catch { if (!this.closed) this.error() }
  }
  async viewport(event: NativeHistoryEvent): Promise<void> {
    if (!this.page || event.sourceEpoch !== this.sourceEpoch || this.closed) return
    const p = this.page
    if (event.anchorId !== undefined && !p.records.some(r => r.id === event.anchorId)) return
    const older = event.nearTop && p.start > 0
    const newer = !older && event.nearBottom && p.end < p.rowCount
    if (!older && !newer) {
      this.held = !(event.atBottom && p.end === p.rowCount)
      return
    }
    this.held = true
    const before = older ? p.start : Math.min(p.rowCount, p.end + 256)
    const limit = older ? 256 : before - p.end
    try {
      const key = this.olderKey(p, before)
      const pending = older ? this.prefetched.get(key) : undefined
      if (older) this.prefetched.delete(key)
      const reply = await (pending ?? this.call({ op: 'older', instance: p.instance, epoch: p.epoch, before, limit }))
      if (this.closed || this.sourceEpoch !== HostHistoryWindow.key(p)) return
      if (reply.kind !== 'page') {
        this.error(reply)
        if (reply.kind === 'error' && reply.error === 'expired' && event.atBottom) this.held = false
        return
      }
      if (HostHistoryWindow.key(reply) !== HostHistoryWindow.key(p)
        || reply.rowCount !== p.rowCount || (older ? reply.end !== p.start : reply.start !== p.end)) { this.error(); return }
      const records = older ? [...reply.records, ...p.records].slice(0, 1024) : [...p.records, ...reply.records].slice(-1024)
      this.page = { ...p, start: records[0].id - 1, end: records.at(-1)!.id, hasOlder: records[0].id > 1, records }
      this.windows.set(this.sourceEpoch!, this.page)
    } catch { if (!this.closed) this.error() }
  }
  async close(): Promise<void> {
    if (this.closed) return
    this.closed = true
    this.prefetched.clear()
    await this.call({ op: 'close' }).catch(() => {})
  }
}
