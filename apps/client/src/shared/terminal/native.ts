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
  | { op: 'scroll'; lines: number }
  | { op: 'follow' }
  | { op: 'older'; instance: string; epoch: number; before: number; limit: number }
  | { op: 'ack' | 'keep'; instance: string; epoch: number }
export interface HostHistoryPage {
  kind: 'page'; instance: string; epoch: number; columns: number; rows: number
  rowCount: number; start: number; end: number; hasOlder: boolean; records: NativeTerminalLine[]
}
export type HostHistoryReply = HostHistoryPage | { kind: 'unsupported' }
  | { kind: 'view'; instance: string; op: 'scroll' | 'follow'; held: boolean; offset: number; available: boolean }
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
  constructor(private request: HostHistoryRequestFn, private notice: (text: string) => void = () => {}) {}
  static key(page: Pick<HostHistoryPage, 'instance' | 'epoch'>): string { return `${page.instance}:${page.epoch}` }
  get sourceEpoch(): string | undefined { return this.page ? HostHistoryWindow.key(this.page) : undefined }
  get unavailable(): boolean { return this.unsupported || this.fallback }
  get visiblePage(): HostHistoryPage | null { return this.fallback ? null : this.page }
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
      const reply = await this.call({ op: 'older', instance: p.instance, epoch: p.epoch, before, limit })
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
    await this.call({ op: 'close' }).catch(() => {})
  }
}

/** Host controls its original terminal viewport; output stays on the original PTY. */
export class HostTerminalViewport {
  held = false
  available = true
  unsupported = false
  private active = false
  private closed = false
  private pending = 0
  private fraction = 0
  private running = false
  private issuedLines = 0
  get reading(): boolean { return this.held || this.pending !== 0 || this.issuedLines !== 0 }
  private retry = 0
  private retryTimer: ReturnType<typeof setTimeout> | undefined
  private keepTimer: ReturnType<typeof setInterval> | undefined
  private operation: Promise<void> = Promise.resolve()
  private closing: Promise<void> | undefined
  constructor(private request: HostHistoryRequestFn, private notice: (text: string) => void = () => {}) {}
  start(): void {
    if (this.closed || this.active) return
    this.active = true
    this.keepTimer = setInterval(() => { if (!this.running) void this.drain(true) }, 30000)
    void this.drain(true)
  }
  scroll(lines: number): void {
    if (!this.active || this.closed || this.unsupported || !Number.isFinite(lines)) return
    this.fraction = Math.max(-256, Math.min(256, this.fraction + lines))
    const whole = Math.trunc(this.fraction)
    this.fraction -= whole
    if (!whole) return
    this.pending = Math.max(-256, Math.min(256, this.pending + whole))
    void this.drain()
  }
  private drain(status = false): Promise<void> {
    if (this.running || !this.active || this.closed || this.unsupported || (!status && !this.pending)) return this.operation
    this.running = true
    this.operation = (async () => {
      let statusPending = status
      while (this.active && !this.closed && !this.unsupported && (statusPending || this.pending)) {
        const lines = this.pending
        this.pending = 0; this.issuedLines = lines; statusPending = false
        let reply: HostHistoryReply
        try { reply = await this.request({ op: 'scroll', lines }) }
        catch { reply = { kind: 'error', error: 'unavailable' } }
        if (this.closed || !this.active) return
        if (reply.kind === 'unsupported' || (reply.kind === 'error' && reply.error === 'unsupported')) {
          this.unsupported = true; this.pending = 0; this.fraction = 0; this.active = false; this.closed = true
          clearInterval(this.keepTimer); clearTimeout(this.retryTimer)
          const release = this.request({ op: 'close' }).catch(() => undefined)
          this.closing = Promise.allSettled([release, this.operation]).then(() => undefined)
          this.notice('主機尚未支援原視窗歷史捲動，請更新共用 helper；終端輸入仍可使用。')
          return
        }
        if (reply.kind === 'view') {
          this.held = reply.held; this.available = reply.available; this.retry = 0
          if (!reply.available) this.notice('目前程式使用 alternate 畫面，暫時無法閱讀歷史；離開後可再捲動。')
        } else {
          if (reply.kind === 'error' && reply.error === 'busy' && this.retry++ < 2) {
            this.pending = Math.max(-256, Math.min(256, lines + this.pending))
            this.retryTimer = setTimeout(() => { this.retryTimer = undefined; void this.drain(true) }, 150)
          } else this.notice('主機歷史控制暫時無法使用，請稍後重試。')
          return
        }
      }
    })().finally(() => { this.running = false; this.issuedLines = 0 })
    return this.operation
  }
  async close(): Promise<void> {
    if (this.closing) return this.closing
    this.closed = true; this.active = false; this.pending = 0; this.fraction = 0
    clearInterval(this.keepTimer); clearTimeout(this.retryTimer)
    // The native close interrupts an in-flight control operation. Do not wait
    // for that operation before issuing close, and never reopen before both settle.
    const closing = this.request({ op: 'close' }).catch(() => ({ kind: 'unsupported' } as HostHistoryReply))
    this.closing = Promise.allSettled([this.operation, closing]).then(() => undefined)
    return this.closing
  }
}
