// Transport contract — native runtime only. There is no configured
// target catalog: the only openable targets are opaque ids issued by
// gui_runtime_sessions / gui_runtime_create_session for a saved runtime
// Host profile. Target ids are never parsed by the UI. JSON field
// convention: camelCase.
// Errors: sanitized strings, or a structured {kind, message} envelope for
// session/Host failures (see ConnectionError).

export type TargetKind = 'host'
import type { HostHistoryReply, HostHistoryRequest } from '../terminal/native'

export interface CodingCli {
  id: string
  name: string
  command: string
  args: string[]
}
export interface ConnectionMetadata {
  category: 'host' | 'agent'
  desktop_transport?: 'runtime-mcp' | null
  coding_cli?: CodingCli | null
}

export interface TargetDto {
  /** Opaque backend-issued id (bound to the profile revision); never parsed. */
  id: string
  /** The runtime Host profile id this target resolves through. */
  hostId: string
  label: string
  /** Secondary line: configured tool hint. */
  subtitle: string
  kind: TargetKind
  running: boolean
  /** 'ready' (unattached) or 'attached' — drives explicit takeover UI. */
  phase: string
  desktopAvailable: boolean
  /** Frozen non-secret CLI definition from effective connection metadata. */
  codingCli?: CodingCli
}

export interface HostDto {
  /** Session-list group id (the runtime Host profile id). */
  id: string
  label: string
  desktopAvailable: boolean
  targets: TargetDto[]
}

export interface SessionDto {
  id: string
  targetId: string
  label: string
  remoteSession: string
}

export interface OutputDto {
  bytes: number[]
  exitCode: number | null
  /** Ordered native runtime output; each batch is acknowledged after parsing. */
  runtime?: {
    batchId: number
    localGeneration: number
    events: RuntimeOutputEvent[]
    ended?: string | null
    inputUncertain?: boolean
  }
}

export type RuntimeOutputEvent =
  | { kind: 'bytes'; bytes: number[]; start: number; end: number; historical: boolean }
  | { kind: 'gap'; bytesDropped: number; resumeOffset: number }
  | { kind: 'notice'; ephemeralWorkspace: boolean; teardownBestEffort: boolean }
  | { kind: 'ttlWarning'; secondsRemaining: number; absolute: boolean }

export interface ScreenshotDto {
  desktopId?: string
  desktopRevision?: number
  mime: string
  base64: string
  source: string
  capturedAt: number // Unix milliseconds
}

export interface DesktopStateDto {
  enabled: boolean
  message: string
}

/** Structured failure kinds the backend may return from runtime calls. */
export type ConnectionErrorKind =
  | 'auth'
  | 'tls'
  | 'transport'
  | 'timeout'
  | 'throttled'
  | 'capacity'
  | 'busy'
  | 'cancelled'
  | 'internal'
  | 'protocol'
  | 'missing'
  | 'sessionMissing'
  | 'sessionDead'
  | 'replacement'
  | 'upgradeRequired'
  | 'renewed'
  | 'takenOver'
  | 'operatorKilled'
  | 'ttlExpired'
  | 'exited'
  | 'inputUncertain'
  | 'slowConsumer'
  | 'retryExhausted'
  | 'stateChanged'
  | 'storage'
  | 'target'

/**
 * Typed session/Host rejection. `kind` comes verbatim from the backend's
 * structured error envelope — the frontend never infers it from message
 * text.
 */
export class ConnectionError extends Error {
  readonly kind: string
  constructor(kind: string, message: string) {
    super(message)
    this.name = 'ConnectionError'
    this.kind = kind
  }
}

/**
 * Decode a structured {kind, message} rejection from the IPC layer.
 * Accepts the object form and a JSON-encoded envelope string; anything else
 * (plain text, regex-shaped hints in messages) returns null — no string
 * inference of error semantics. The kind is passed through verbatim;
 * unknown or newer kinds still get their sanitized message.
 */
export function asConnectionError(e: unknown): ConnectionError | null {
  let v: unknown = e
  if (typeof v === 'string') {
    const s = v.trim()
    if (!s.startsWith('{') || !s.endsWith('}')) return null
    try {
      v = JSON.parse(s)
    } catch {
      return null
    }
  }
  if (v && typeof v === 'object') {
    const k = (v as { kind?: unknown }).kind
    const m = (v as { message?: unknown }).message
    if (typeof k === 'string' && k) {
      return new ConnectionError(k, typeof m === 'string' ? m : '')
    }
  }
  return null
}

export interface InstanceDesktop { id: string; revision: number; name: string; mcpUrl: string; tokenPresent: boolean }
export interface InstanceDesktopInput { id?: string; expectedRevision?: number; name: string; mcpUrl: string; token: string }
export interface InstanceSelection { desktop: InstanceDesktop; toolReady: boolean }

export interface Transport {
  instanceDesktops?(): Promise<InstanceDesktop[]>
  instanceSave?(input: InstanceDesktopInput): Promise<InstanceDesktop>
  instanceDelete?(id: string, expectedRevision: number): Promise<void>
  instanceTest?(id: string, expectedRevision: number): Promise<void>
  instanceSelect?(sessionId: string, desktopId: string, expectedRevision: number): Promise<InstanceSelection>
  instanceClear?(sessionId: string): Promise<void>
  instanceLaunch?(sessionId: string): Promise<InstanceSelection>

  transferExport(input: TransferExportInput): Promise<{ status: 'saved' | 'cancelled' }>
  transferPickImport(): Promise<TransferPreview>
  transferUnlock(token: string, password: string): Promise<TransferPreview>
  transferApply(token: string, choices: TransferChoice[]): Promise<TransferResult>
  transferCancel(): Promise<{ status: 'cancelled' | 'committing' }>

  runtimeAck(sessionId: string, batchId: number, localGeneration: number): Promise<void>
  runtimeRecover(sessionId: string): Promise<void>
  runtimeSessions(id: string, expectedRevision: number): Promise<HostDto>
  runtimeCreateSession(id: string, expectedRevision: number, name: string): Promise<TargetDto>
  runtimeCreateHistorySession?(id: string, expectedRevision: number): Promise<TargetDto>
  runtimeHistory?(sessionId: string, request: HostHistoryRequest): Promise<HostHistoryReply>
  runtimeHosts(): Promise<RuntimeHostDto[]>
  runtimeSaveHost(input: RuntimeHostInput): Promise<RuntimeHostDto>
  runtimeDeleteHost(id: string, expectedRevision: number): Promise<void>
  runtimeTestHost(id: string, expectedRevision: number): Promise<RuntimeHostTestDto>
  open(targetId: string, rows: number, cols: number): Promise<SessionDto>
  /** Pull max bounded output; raw bytes. */
  poll(sessionId: string): Promise<OutputDto>
  write(sessionId: string, bytes: number[]): Promise<void>
  resize(sessionId: string, rows: number, cols: number): Promise<void>
  /** Local detach only; remote session/lease semantics preserved by backend. */
  close(sessionId: string): Promise<void>
  desktop(sessionId: string, enabled: boolean): Promise<DesktopStateDto>
  screenshot(sessionId: string): Promise<ScreenshotDto>
}

/** Safe native runtime metadata. Credentials have no readback API. */
export interface RuntimeHostDto {
  metadata?: ConnectionMetadata
  id: string
  revision: number
  name: string
  baseUrl: string
  credentialPresent: boolean
  /** Local metadata overrides exact-origin catalog defaults. */
  category?: 'host' | 'agent'
  agentId?: string
  agentName?: string
  agentTool?: string
}

/** One-shot masked input; never retained after submission. */
export interface RuntimeHostInput {
  category?: 'host' | 'agent'
  metadata?: ConnectionMetadata
  id?: string
  expectedRevision?: number
  name: string
  baseUrl: string
  credential: string
}

export interface RuntimeHostTestDto {
  sessionCount: number
  draining: boolean
  teardownBestEffort: boolean
  continuityVerified: boolean
}

export type TransferAction = 'add' | 'update' | 'skip'
interface TransferEntryBase {
  key: string
  name: string
  matchStatus: 'new' | 'duplicate' | 'ambiguous'
}
export interface TransferHostEntry extends TransferEntryBase {
  kind?: 'host'
  origin: string
  metadata: ConnectionMetadata
  hasCredential: boolean
  target: RuntimeHostDto | null
}
export interface TransferScreenEntry extends TransferEntryBase {
  kind: 'screen'
  mcpUrl: string
  hasToken: boolean
  target: InstanceDesktop | null
}
export type TransferEntry = TransferHostEntry | TransferScreenEntry
export interface TransferPreview {
  status: 'cancelled' | 'requiresPassword' | 'preview'
  token: string | null
  entries: TransferEntry[]
}
export interface TransferExportInput {
  selected: { kind?: 'host' | 'screen'; id: string; expectedRevision: number }[]
  includeCredentials: boolean
  password: string
}
export interface TransferChoice { key: string; action: TransferAction; credential?: string }
export interface TransferResult { added: number; updated: number; skipped: number; profiles: RuntimeHostDto[]; desktops?: InstanceDesktop[] }
