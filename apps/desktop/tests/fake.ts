import type {
  TransferPreview, TransferResult, TransferExportInput, TransferChoice,
  DesktopStateDto,
  HostDto,
  OutputDto,
  RuntimeHostDto,
  RuntimeHostTestDto,
  SessionDto,
  ScreenshotDto,
  TargetDto,
  Transport
} from '../src/transport/types'
import type { TerminalHandle } from '../src/terminal/handle'

export interface Deferred<T> {
  promise: Promise<T>
  resolve: (v: T) => void
  reject: (e: unknown) => void
}

export function deferred<T>(): Deferred<T> {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

export interface FakeSession {
  id: string
  targetId: string
  queue: OutputDto[]
  waiters: ((o: OutputDto) => void)[]
  writes: number[][]
  resizes: { rows: number; cols: number }[]
  closed: boolean
  pollCount: number
}

export class FakeTransport implements Transport {
  instanceSelect?: Transport['instanceSelect']
  instanceClear?: Transport['instanceClear']
  instanceLaunch?: Transport['instanceLaunch']

  async transferExport(_input: TransferExportInput): Promise<{ status: 'saved' | 'cancelled' }> { return { status: 'cancelled' } }
  async transferPickImport(): Promise<TransferPreview> { return { status: 'cancelled', token: null, entries: [] } }
  async transferUnlock(_token: string, _password: string): Promise<TransferPreview> { throw new Error('not implemented') }
  async transferApply(_token: string, _choices: TransferChoice[]): Promise<TransferResult> { throw new Error('not implemented') }
  async transferCancel(): Promise<{ status: 'cancelled' | 'committing' }> { return { status: 'cancelled' } }

  sessions = new Map<string, FakeSession>()
  openCalls: { targetId: string; rows: number; cols: number }[] = []
  /** Thrown (verbatim) from open() when set — use ConnectionError for kinds. */
  openError: unknown | null = null
  closeCalls: string[] = []
  desktopCalls: { sid: string; enabled: boolean }[] = []
  screenshotCalls: string[] = []
  desktopGate: Promise<void> | null = null
  screenshotGate: Promise<void> | null = null
  shot: ScreenshotDto = {
    mime: 'image/png',
    base64: 'aGk=',
    source: 'art-term-hands',
    capturedAt: 1_760_000_000_000
  }
  runtimeHostsData: RuntimeHostDto[] = []
  runtimeHostsError: unknown | null = null
  runtimeSessionsData = new Map<string, HostDto>()
  private seq = 0

  constructor(profiles: RuntimeHostDto[] = []) {
    this.runtimeHostsData = profiles
  }

  async runtimeHosts(): Promise<RuntimeHostDto[]> {
    if (this.runtimeHostsError) throw this.runtimeHostsError
    return this.runtimeHostsData
  }
  async runtimeSessions(id: string): Promise<HostDto> {
    const group = this.runtimeSessionsData.get(id)
    if (!group) throw new Error('無法列出 Shell，請確認連線與 runtime 版本。')
    return group
  }
  async runtimeCreateSession(id: string, _expectedRevision: number, name: string): Promise<TargetDto> {
    const target = makeTarget(`created-${name}`, id)
    const group = this.runtimeSessionsData.get(id)
    if (group) group.targets.push(target)
    return target
  }
  async runtimeSaveHost(): Promise<RuntimeHostDto> {
    throw new Error('not implemented')
  }
  async runtimeDeleteHost(): Promise<void> {
    throw new Error('not implemented')
  }
  async runtimeTestHost(): Promise<RuntimeHostTestDto> {
    throw new Error('not implemented')
  }
  async runtimeAck(_sessionId: string, _batchId: number, _localGeneration: number): Promise<void> {}
  async runtimeRecover(_sessionId: string): Promise<void> {}

  async open(targetId: string, rows: number, cols: number): Promise<SessionDto> {
    this.openCalls.push({ targetId, rows, cols })
    if (this.openError) throw this.openError
    const id = `s${++this.seq}`
    this.sessions.set(id, {
      id,
      targetId,
      queue: [],
      waiters: [],
      writes: [],
      resizes: [],
      closed: false,
      pollCount: 0
    })
    return { id, targetId, label: targetId, remoteSession: `artterm_${targetId}_main` }
  }

  poll(sessionId: string): Promise<OutputDto> {
    const s = this.must(sessionId)
    s.pollCount++
    const next = s.queue.shift()
    if (next) return Promise.resolve(next)
    return new Promise<OutputDto>((resolve) => s.waiters.push(resolve))
  }

  feed(sessionId: string, out: OutputDto): void {
    const s = this.must(sessionId)
    const waiter = s.waiters.shift()
    if (waiter) waiter(out)
    else s.queue.push(out)
  }

  async write(sessionId: string, bytes: number[]): Promise<void> {
    this.must(sessionId).writes.push(bytes)
  }

  async resize(sessionId: string, rows: number, cols: number): Promise<void> {
    this.must(sessionId).resizes.push({ rows, cols })
  }

  async close(sessionId: string): Promise<void> {
    this.closeCalls.push(sessionId)
    this.must(sessionId).closed = true
  }

  async desktop(sessionId: string, enabled: boolean): Promise<DesktopStateDto> {
    this.desktopCalls.push({ sid: sessionId, enabled })
    if (this.desktopGate) await this.desktopGate
    return { enabled, message: enabled ? 'provider ready' : '' }
  }

  async screenshot(sessionId: string): Promise<ScreenshotDto> {
    this.screenshotCalls.push(sessionId)
    if (this.screenshotGate) await this.screenshotGate
    return this.shot
  }

  private must(id: string): FakeSession {
    const s = this.sessions.get(id)
    if (!s) throw new Error(`unknown session ${id}`)
    return s
  }
}

export class FakeTerm implements TerminalHandle {
  cols = 80
  rows = 24
  chunks: Uint8Array[] = []
  holdWrites = false
  fitCount = 0
  focusCount = 0
  resetCount = 0
  disposed = false
  private writeResolvers: (() => void)[] = []
  private dataCb: ((text: string) => void) | null = null
  keyInterceptor: ((e: KeyboardEvent) => boolean) | null = null

  write(data: Uint8Array): Promise<void> {
    this.chunks.push(data)
    if (this.holdWrites) {
      return new Promise<void>((resolve) => this.writeResolvers.push(resolve))
    }
    return Promise.resolve()
  }

  releaseWrites(): void {
    for (const r of this.writeResolvers.splice(0)) r()
  }

  onData(cb: (text: string) => void): void {
    this.dataCb = cb
  }

  emitData(text: string): void {
    this.dataCb?.(text)
  }

  setKeyInterceptor(fn: (e: KeyboardEvent) => boolean): void {
    this.keyInterceptor = fn
  }

  fit(): void {
    this.fitCount++
  }

  focus(): void {
    this.focusCount++
  }

  reset(): void {
    this.resetCount++
  }

  dispose(): void {
    this.disposed = true
  }

  text(): string {
    return new TextDecoder().decode(
      Uint8Array.from(this.chunks.flatMap((c) => Array.from(c)))
    )
  }
}

export function makeHost(id: string, targets: TargetDto[] = [], desktop = false): HostDto {
  return { id, label: id, desktopAvailable: desktop, targets }
}

export function makeTarget(id: string, hostId: string, kind: TargetDto['kind'] = 'host', running = true, desktop = false, subtitle = ''): TargetDto {
  return { id, hostId, label: id, subtitle, kind, running, phase: running ? 'ready' : 'exited', desktopAvailable: desktop }
}

export function makeProfile(id: string, name = id, extra: Partial<RuntimeHostDto> = {}): RuntimeHostDto {
  return {
    id,
    revision: 1,
    name,
    baseUrl: `https://${id}.example.test`,
    credentialPresent: true,
    ...extra
  }
}
