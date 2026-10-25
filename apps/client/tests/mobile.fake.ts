import type {
  TransferPreview, TransferResult, TransferExportInput, TransferChoice,
  DesktopStateDto,
  HostDto,
  OutputDto,
  RuntimeHostDto,
  RuntimeHostTestDto,
  ScreenshotDto,
  SessionDto,
  TargetDto,
  Transport
} from '../src/shared/transport/types'

export interface FakeMobileSession {
  id: string
  targetId: string
  queue: OutputDto[]
  waiters: ((o: OutputDto) => void)[]
  writes: number[][]
  resizes: { rows: number; cols: number }[]
  closed: boolean
  pollCount: number
}

/** Build a runtime session-list group (gui_runtime_sessions reply). */
export function makeGroup(
  id: string,
  targets: TargetDto[] = [],
  label?: string
): HostDto {
  return { id, label: label ?? id, desktopAvailable: false, targets }
}

export function makeCatalogTarget(
  id: string,
  profileId: string,
  label: string,
  subtitle: string,
  kind: TargetDto['kind'] = 'host',
  desktop = false,
  phase = 'ready'
): TargetDto {
  return {
    id,
    hostId: profileId,
    label,
    subtitle,
    kind,
    running: true,
    phase,
    desktopAvailable: desktop
  }
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

/**
 * Fake implementing the runtime-only mobile contract: runtimeHosts()
 * returns saved profiles (local metadata only), runtimeSessions() returns
 * the per-profile session-list group, and open() may reject with a
 * structured ConnectionError.
 */
export class FakeMobileTransport implements Transport {
  async transferExport(_input: TransferExportInput): Promise<{ status: 'saved' | 'cancelled' }> { return { status: 'cancelled' } }
  async transferPickImport(): Promise<TransferPreview> { return { status: 'cancelled', token: null, entries: [] } }
  async transferUnlock(_token: string, _password: string): Promise<TransferPreview> { throw new Error('not implemented') }
  async transferApply(_token: string, _choices: TransferChoice[]): Promise<TransferResult> { throw new Error('not implemented') }
  async transferCancel(): Promise<{ status: 'cancelled' | 'committing' }> { return { status: 'cancelled' } }

  runtimeHostsData: RuntimeHostDto[]
  runtimeHostsError: unknown | null = null
  runtimeSessionsData = new Map<string, HostDto>()
  runtimeSessionsError: unknown | null = null
  createSessionCalls: { id: string; expectedRevision: number; name: string }[] = []
  createSessionTarget: TargetDto | null = null
  ackCalls: { sessionId: string; batchId: number; localGeneration: number }[] = []
  recoverCalls: string[] = []
  recoverError: unknown | null = null
  openCalls: { targetId: string; rows: number; cols: number }[] = []
  openGate: Promise<void> | null = null
  /** Rejection thrown by open(); ConnectionError for structured kinds. */
  openError: unknown | null = null
  /** Per-target open rejections, consulted before openError. */
  openErrors = new Map<string, unknown>()
  writeGate: Promise<void> | null = null
  closeCalls: string[] = []
  desktopCalls: { sid: string; enabled: boolean }[] = []
  screenshotCalls: string[] = []
  desktopGate: Promise<void> | null = null
  screenshotGate: Promise<void> | null = null
  desktopError: string | null = null
  shot: ScreenshotDto = {
    mime: 'image/png',
    base64: 'aGk=',
    source: 'art-term-hands',
    capturedAt: 1_760_000_000_000
  }
  sessions = new Map<string, FakeMobileSession>()
  private seq = 0

  constructor(profiles: RuntimeHostDto[] = []) {
    this.runtimeHostsData = profiles
  }

  // --- runtime profile/session surface ---
  async runtimeHosts(): Promise<RuntimeHostDto[]> {
    if (this.runtimeHostsError) throw this.runtimeHostsError
    return this.runtimeHostsData
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
  async runtimeSessions(id: string): Promise<HostDto> {
    if (this.runtimeSessionsError) throw this.runtimeSessionsError
    const group = this.runtimeSessionsData.get(id)
    if (!group) throw new Error('無法列出 Shell，請確認連線與 runtime 版本。')
    return group
  }
  async runtimeCreateSession(id: string, expectedRevision: number, name: string): Promise<TargetDto> {
    this.createSessionCalls.push({ id, expectedRevision, name })
    const target = this.createSessionTarget ?? makeCatalogTarget(`created-${name}`, id, name, '')
    const group = this.runtimeSessionsData.get(id)
    if (group) group.targets.push(target)
    return target
  }
  async runtimeAck(sessionId: string, batchId: number, localGeneration: number): Promise<void> {
    this.ackCalls.push({ sessionId, batchId, localGeneration })
  }
  async runtimeRecover(sessionId: string): Promise<void> {
    this.recoverCalls.push(sessionId)
    if (this.recoverError) throw this.recoverError
  }

  async open(targetId: string, rows: number, cols: number): Promise<SessionDto> {
    this.openCalls.push({ targetId, rows, cols })
    if (this.openGate) await this.openGate
    const perTarget = this.openErrors.get(targetId)
    if (perTarget !== undefined) throw perTarget
    if (this.openError) throw this.openError
    const id = `m${++this.seq}`
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
    return {
      id,
      targetId,
      label: targetId,
      remoteSession: `artterm_mobile-${targetId}`
    }
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
    if (this.writeGate) await this.writeGate
    this.must(sessionId).writes.push(bytes)
  }

  async resize(sessionId: string, rows: number, cols: number): Promise<void> {
    this.must(sessionId).resizes.push({ rows, cols })
  }

  async close(sessionId: string): Promise<void> {
    this.closeCalls.push(sessionId)
    const s = this.sessions.get(sessionId)
    if (s) s.closed = true
  }

  async desktop(sessionId: string, enabled: boolean): Promise<DesktopStateDto> {
    this.desktopCalls.push({ sid: sessionId, enabled })
    if (this.desktopGate) await this.desktopGate
    if (this.desktopError) throw new Error(this.desktopError)
    return { enabled, message: enabled ? 'provider ready' : '' }
  }

  async screenshot(sessionId: string): Promise<ScreenshotDto> {
    this.screenshotCalls.push(sessionId)
    if (this.screenshotGate) await this.screenshotGate
    return this.shot
  }

  private must(id: string): FakeMobileSession {
    const s = this.sessions.get(id)
    if (!s) throw new Error(`unknown session ${id}`)
    return s
  }
}
