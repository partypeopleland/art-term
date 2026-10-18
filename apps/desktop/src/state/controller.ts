import { computed, markRaw, reactive, ref } from 'vue'
import type { ComputedRef, Ref } from 'vue'
import { asConnectionError } from '../transport/types'
import { hostErrorText } from '../transport/host-error'
import type {
  CodingCli,
  InstanceDesktop,
  HostDto,
  OutputDto,
  RuntimeHostDto,
  TargetDto,
  Transport
} from '../transport/types'
import type { TerminalHandle } from '../terminal/handle'
import type { HostHistoryReply, HostHistoryRequest } from '../terminal/native'

export const MAX_TABS = 32
export const MAX_RUNTIME_PROFILES = 64
export const AUTO_CAPTURE_MS = 30_000
const WRITE_CAP = 256 * 1024
const WRITE_CHUNK = 16 * 1024
const DEFAULT_IDLE_POLL_MS = 20

/** Bound POSIX words; keep the Codex button inline and ordinary words unquoted. */
export function codingCliInput(cli: CodingCli): string | null {
  if (typeof cli.command !== 'string' || !cli.command || cli.command.length > 256 || cli.command.startsWith('-') ||
      !Array.isArray(cli.args) || cli.args.length > 32) return null
  const codex = cli.id === 'codex'
  const args = codex
    ? ['--yolo', '--no-alt-screen', ...cli.args.filter(arg => arg !== '--yolo' && arg !== '--no-alt-screen')]
    : cli.args
  const words = [cli.command, ...args]
  if (words.some(w => typeof w !== 'string' || w.length > 1024 || /[\u0000-\u001f\u007f-\u009f]/u.test(w)) || words.reduce((n, w) => n + new TextEncoder().encode(w).length, 0) > 8192) return null
  return words.map((w, index) => {
    const plain = index === 0 ? /^(?:[A-Za-z0-9_./-]+\/)?codex$/.test(w) : /^[A-Za-z0-9_./:@%+=,-]+$/.test(w)
    return codex && plain ? w : "'" + w.replaceAll("'", "'\\''") + "'"
  }).join(' ') + '\r'
}

export type TabStatus = 'connecting' | 'live' | 'exited' | 'error'

export interface ShotState {
  src: string
  source: string
  capturedAt: number
}

export interface TabState {
  /** Stable key; target identity for reuse. */
  key: string
  targetId: string
  hostId: string
  hostLabel: string
  label: string
  codingCli: CodingCli | null
  status: TabStatus
  errorMsg: string
  notice: string
  sessionId: string | null
  remoteSession: string
  runtime: boolean
  /**
   * This tab's runtime Host profile is bound to a declared runtime_agents
   * entry (backend metadata, exact configured origin match — never a
   * label guess). Lifecycle-terminal runtime kinds retire the tab instead
   * of leaving a dead reconnect target: no automatic rebind to a
   * replacement runtime, pending input/resize die with the tab.
   */
  runtimeAgent: boolean
  exitCode: number | null
  instanceDesktop?: InstanceDesktop | null
  instanceToolReady?: boolean
  desktopAvailable: boolean
  desktopEnabled: boolean
  desktopBusy: boolean
  desktopMsg: string
  desktopError: string
  previewVisible: boolean
  shot: ShotState | null
  shotError: string
  captureGen: number
  /** Internal, markRaw'd once attached. */
  term: TerminalHandle | null
  wantDesktop: boolean
  closed: boolean
  writeBuf: number[]
  writing: boolean
  writeDropped: boolean
  pendingSize: { rows: number; cols: number } | null
  resizing: boolean
  lastSize: { rows: number; cols: number } | null
  connectionGen: number
}

export interface HostUiState {
  dto: HostDto
  expanded: boolean
}

export interface ControllerOptions {
  idlePollMs?: number
  autoCaptureMs?: number
  backgroundConnection?: 'detach' | 'keep'
  /** Mobile returns to target selection when the remote Shell exits. */
  closeFinishedTabs?: boolean
}

export interface AppController {
  historyRequest(tab: TabState, request: HostHistoryRequest): Promise<HostHistoryReply>
  catalogLoading: Ref<boolean>
  /** Saved runtime Host metadata (local protected-store snapshot only —
   *  never a session target and no credential material). */
  runtimeProfiles: Ref<RuntimeHostDto[]>
  /** Safe fixed-mapping text for a failed local profile read; '' when ok. */
  runtimeProfilesError: Ref<string>
  instanceDesktops: Ref<InstanceDesktop[]>
  instanceDesktopsError: Ref<string>
  loadInstanceDesktops(): Promise<void>
  selectInstance(tab: TabState, desktop: InstanceDesktop): Promise<void>
  clearInstance(tab: TabState): Promise<void>
  tabs: TabState[]
  activeKey: Ref<string | null>
  activeTab: ComputedRef<TabState | null>
  uiNotice: Ref<string>
  /** Monotonic signal: a retired runtime tab closed and the profile list
   *  was refreshed; the mobile shell surfaces the target selection list.
   *  Increment-only — works even when other tabs stay open; no payload
   *  is ever attached. */
  selectionRequest: Ref<number>
  loadCatalog(): Promise<void>
  refreshTargets(): Promise<void>
  openTarget(host: HostUiState, target: TargetDto, withDesktop: boolean): void
  attachTerminal(tab: TabState, handle: TerminalHandle): void
  reopenTab(tab: TabState, withDesktop: boolean): void
  closeTab(tab: TabState): Promise<void>
  detachTab(tab: TabState): Promise<void>
  setHidden(hidden: boolean): Promise<void>
  setActive(key: string): void
  stepTab(delta: number): void
  isSwitchKey(e: KeyboardEvent): boolean
  handleKey(e: KeyboardEvent): boolean
  sendInput(tab: TabState, text: string): void
  launchCodingCli(tab: TabState): void
  requestResize(tab: TabState, rows: number, cols: number): void
  setDesktop(tab: TabState, enabled: boolean): Promise<void>
  setPreviewVisible(tab: TabState, visible: boolean): void
  manualCapture(): void
  dispose(): void
}

const textEncoder = new TextEncoder()

function errText(e: unknown): string {
  const msg = e instanceof Error ? e.message : String(e)
  const oneLine = msg.replace(/\s+/g, ' ').trim()
  return oneLine.length > 300 ? `${oneLine.slice(0, 300)}…` : oneLine
}

/** Kind → frontend-safe Chinese fallback when the backend message is empty. */
const KIND_FALLBACK: Record<string, string> = {
  upgradeRequired: '主機 runtime 需要升級',
  replacement: '遠端 runtime 或 Shell 已更換，請重新選擇',
  ttlExpired: '遠端 Shell 已到期',
  takenOver: '此 Shell 已由另一個連線接管',
  renewed: '此 Shell 的連線憑證已更新',
  operatorKilled: '遠端 Shell 已被管理者結束',
  exited: '遠端 Shell 已結束',
  missing: '遠端 Shell 已不存在',
  slowConsumer: '輸出佇列已滿，請重新連線',
  retryExhausted: '自動重連次數已用完，請手動重新連線',
  busy: '連線操作正在進行，請稍後再試',
  cancelled: '本機已中斷連線',
  inputUncertain: '輸入是否送達尚未確認，未自動重送',
  internal: '遠端 runtime 發生內部錯誤',
  protocol: '遠端 runtime 資料格式不符',
  auth: 'Shell 認證被拒絕',
  tls: 'WSS 憑證驗證失敗',
  capacity: '連線佇列或 runtime 容量已達上限',
  throttled: '主機暫時限制請求，請稍後重試',
  transport: '無法連線到目標，請檢查網路',
  timeout: 'Shell 連線中斷或逾時，請確認網路後重新連線',
  target: '目標無法使用或不存在',
  stateChanged: '連線設定已變更，請重新整理後再試'
}

/**
 * Runtime lifecycle-terminal kinds for a declared runtime-agent tab —
 * the backend SessionError camelCase serialization. For these kinds the
 * issued binding can never be reused and an automatic reconnect would
 * silently reach a different runtime, so the tab retires with a catalog
 * refresh and a manual-reselect notice instead.
 */
const RUNTIME_TERMINAL_KINDS = new Set([
  'replacement',
  'missing',
  'exited',
  'ttlExpired',
  'takenOver',
  'renewed',
  'operatorKilled',
  'stateChanged'
])

/**
 * Display text for any session rejection: a structured ConnectionError
 * keeps its sanitized backend message (never "[object Object]"), with a
 * per-kind Chinese fallback; anything else is flattened by errText.
 */
function connErrText(e: unknown): string {
  const conn = asConnectionError(e)
  if (conn) {
    const msg = conn.message.replace(/\s+/g, ' ').trim()
    return (msg.length > 300 ? `${msg.slice(0, 300)}…` : msg) ||
      KIND_FALLBACK[conn.kind] || '連線失敗'
  }
  return errText(e)
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

const SAFE_IMAGE_MIME = /^image\/(png|jpeg|webp|gif|bmp)$/i

export function createAppController(
  transport: Transport,
  options: ControllerOptions = {}
): AppController {
  const idlePollMs = options.idlePollMs ?? DEFAULT_IDLE_POLL_MS
  const autoCaptureMs = options.autoCaptureMs ?? AUTO_CAPTURE_MS

  const catalogLoading = ref(true)
  const runtimeProfiles = ref<RuntimeHostDto[]>([])
  const runtimeProfilesError = ref('')
  const instanceDesktops = ref<InstanceDesktop[]>([])
  const instanceDesktopsError = ref('')
  let instanceCatalogGen = 0
  const instanceOperations = new Map<string, number>()
  const tabs = reactive<TabState[]>([])
  const activeKey = ref<string | null>(null)
  const uiNotice = ref('')
  const selectionRequest = ref(0)
  let disposed = false
  let hidden = false
  const activeTab = computed(() => tabs.find((t) => t.key === activeKey.value) ?? null)

  async function loadInstanceDesktops(): Promise<void> {
    const generation = ++instanceCatalogGen
    try {
      const list = await transport.instanceDesktops?.() ?? []
      if (disposed || generation !== instanceCatalogGen) return
      instanceDesktops.value = list.slice(0, 64)
      instanceDesktopsError.value = ''
      for (const tab of tabs) {
        const selected = tab.instanceDesktop
        if (selected && !list.some(item => item.id === selected.id && item.revision === selected.revision)) void clearInstance(tab)
      }
    } catch {
      if (disposed || generation !== instanceCatalogGen) return
      instanceDesktops.value = []
      instanceDesktopsError.value = '無法讀取 Screen 設定'
    }
  }

  // The connection list is the saved runtime Host metadata
  // (transport.runtimeHosts) — local metadata only (no credential
  // readback, no remote session listing, no fan-out). Openable targets
  // come exclusively from runtimeSessions/runtimeCreateSession replies.
  // Concurrent loads are fenced by generation: only the newest snapshot
  // may land, a stale list reply can never resurrect deleted entries,
  // and a disposed controller never mutates state.
  let catalogGen = 0
  async function loadCatalog(): Promise<void> {
    if (disposed) return
    const gen = ++catalogGen
    catalogLoading.value = true
    runtimeProfilesError.value = ''
    try {
      const list = await transport.runtimeHosts()
      if (disposed || gen !== catalogGen) return
      runtimeProfiles.value = list.slice(0, MAX_RUNTIME_PROFILES)
    } catch (e) {
      if (disposed || gen !== catalogGen) return
      runtimeProfiles.value = []
      runtimeProfilesError.value = hostErrorText(
        e,
        '無法讀取已儲存的 Computer 設定，請重新整理或關閉後再試。'
      )
    } finally {
      if (!disposed && gen === catalogGen) catalogLoading.value = false
    }
  }

  function openTarget(host: HostUiState, target: TargetDto, withDesktop: boolean): void {
    const existing = tabs.find(
      (t) => t.targetId === target.id && t.hostId === host.dto.id && !t.closed
    )
    if (existing) {
      setActive(existing.key)
      if (existing.status === 'exited' || existing.status === 'error') {
        reopenTab(existing, withDesktop)
      } else if (withDesktop && !existing.desktopEnabled) {
        void setDesktop(existing, true)
      }
      return
    }
    if (hidden || disposed) return
    if (tabs.length >= MAX_TABS) {
      uiNotice.value = `已達 ${MAX_TABS} 分頁上限`
      return
    }
    uiNotice.value = ''
    const tab = reactive<TabState>({
      key: `${host.dto.id}::${target.id}`,
      targetId: target.id,
      hostId: host.dto.id,
      hostLabel: host.dto.label,
      label: target.label,
      // The coding-CLI hint is explicit backend metadata only — display
      // text is never interpreted for it.
      codingCli: target.codingCli ? { ...target.codingCli, args: [...target.codingCli.args] } : null,
      status: 'connecting',
      errorMsg: '',
      notice: '',
      sessionId: null,
      remoteSession: '',
      runtime: false,
      // A runtime HostDto carries the profile id — an agent-bound
      // profile marks this tab for lifecycle-terminal retirement.
      runtimeAgent: runtimeProfiles.value.some(
        (p) => p.id === host.dto.id && p.category === 'agent'
      ),
      exitCode: null,
      desktopAvailable: target.desktopAvailable || host.dto.desktopAvailable,
      instanceDesktop: null,
      instanceToolReady: false,
      desktopEnabled: false,
      desktopBusy: false,
      desktopMsg: '',
      desktopError: '',
      previewVisible: true,
      shot: null,
      shotError: '',
      captureGen: 0,
      term: null,
      wantDesktop: withDesktop,
      closed: false,
      writeBuf: [],
      writing: false,
      writeDropped: false,
      pendingSize: null,
      resizing: false,
      lastSize: null,
      connectionGen: 0
    })
    tabs.push(tab)
    activeKey.value = tab.key
    // Session opens in attachTerminal once the pane mounted and fit().
  }

  function attachTerminal(tab: TabState, handle: TerminalHandle): void {
    if (tab.closed) {
      handle.dispose()
      return
    }
    tab.term = markRaw(handle)
    handle.onData((text, terminalReply) => terminalReply ? sendTerminalReply(tab, text) : sendInput(tab, text))
    handle.setKeyInterceptor((ev) => !isSwitchKey(ev))
    const want = tab.wantDesktop
    tab.wantDesktop = false
    if (tab.status === 'connecting') void openSession(tab, want)
  }

  async function openSession(tab: TabState, withDesktop: boolean): Promise<void> {
    if (tab.closed || !tab.term || hidden || disposed) return
    const generation = ++tab.connectionGen
    // New generation: nothing from an older connection may replay into it.
    tab.writeBuf = []
    tab.writeDropped = false
    tab.pendingSize = null
    tab.status = 'connecting'
    tab.errorMsg = ''
    const { rows, cols } = tab.term
    tab.lastSize = { rows, cols }
    let session
    try {
      session = await transport.open(tab.targetId, rows, cols)
    } catch (e) {
      if (!tab.closed && tab.connectionGen === generation) {
        const conn = asConnectionError(e)
        if (tab.runtimeAgent && conn && RUNTIME_TERMINAL_KINDS.has(conn.kind)) {
          // Old opaque runtime target failed permanently — close the tab
          // and refresh; the issued binding is never opened automatically.
          void retireRuntimeAgentTab(tab)
        } else {
          tab.status = 'error'
          tab.errorMsg = connErrText(e)
        }
      }
      return
    }
    if (tab.closed || (hidden && options.backgroundConnection !== 'keep') || disposed || tab.connectionGen !== generation) {
      // Late open after local close — detach the new session, keep remote alive.
      transport.close(session.id).catch(() => undefined)
      return
    }
    tab.sessionId = session.id
    runtimeRecoveryAttempts.delete(tab.key)
    runtimeRecoveryHealthySince.delete(tab.key)
    runtimeRecovery.delete(tab.key)
    tab.remoteSession = session.remoteSession
    // Every openable target is runtime-issued; there is no other transport.
    tab.runtime = true
    if (session.label) tab.label = session.label
    tab.status = 'live'
    tab.exitCode = null
    void pollLoop(tab, session.id)
    if (withDesktop) void setDesktop(tab, true)
  }

  /**
   * Declared runtime-agent tab whose binding ended permanently (typed
   * replacement / missing / dead / terminated kinds). The affected local
   * tab closes — queued input and a pending resize die with it, capture
   * state is invalidated, the catalog reloads exactly once
   * (generation-fenced), and a safe notice asks for manual reselection.
   * No automatic reconnect to a replacement runtime ever happens.
   */
  async function retireRuntimeAgentTab(tab: TabState): Promise<void> {
    if (disposed || tab.closed) return
    tab.writeBuf = []
    tab.pendingSize = null
    uiNotice.value = `「${tab.label}」的遠端 runtime 或 Shell 已更換；已關閉舊分頁，請重新選擇。`
    const closing = closeTab(tab)
    const refreshing = loadCatalog()
    selectionRequest.value++
    await Promise.all([closing, refreshing])
  }

  async function refreshTargets(): Promise<void> {
    if (disposed) return
    const refreshing = loadCatalog()
    selectionRequest.value++
    await refreshing
  }

  function closeFinishedShell(tab: TabState, kind: string): boolean {
    if (!options.closeFinishedTabs || !['exited', 'missing', 'operatorKilled'].includes(kind)) return false
    void closeTab(tab)
    return true
  }

  function reopenTab(tab: TabState, withDesktop: boolean): void {
    if (tab.closed || !tab.term || hidden || disposed || tab.status === 'live' || tab.status === 'connecting') return
    tab.writeBuf = []
    tab.writeDropped = false
    tab.pendingSize = null
    if (runtimeRecovery.has(tab.key)) { void recoverRuntime(tab, true); return }
    tab.term.reset()
    tab.desktopEnabled = false
    tab.instanceDesktop = null
    tab.instanceToolReady = false
    tab.desktopMsg = ''
    tab.desktopError = ''
    tab.shot = null
    tab.shotError = ''
    tab.previewVisible = true
    invalidateCapture(tab)
    void openSession(tab, withDesktop)
  }

  async function pollLoop(tab: TabState, sessionId: string): Promise<void> {
    for (;;) {
      if (disposed || tab.closed || tab.sessionId !== sessionId) return
      let out: OutputDto
      try {
        out = await transport.poll(sessionId)
      } catch (e) {
        if (!tab.closed && tab.sessionId === sessionId) {
          const conn = asConnectionError(e)
          if (tab.runtimeAgent && conn && RUNTIME_TERMINAL_KINDS.has(conn.kind)) {
            void retireRuntimeAgentTab(tab)
            return
          }
          if (conn && closeFinishedShell(tab, conn.kind)) return
          tab.status = 'error'
          tab.errorMsg = errText(e)
          tab.sessionId = null
          invalidateCapture(tab)
          void transport.close(sessionId).catch(() => undefined)
          resumeAfterBackground(tab)
        }
        return
      }
      if (disposed || tab.closed || tab.sessionId !== sessionId) return
      if (out.runtime) {
        try {
          for (const event of out.runtime.events) {
            if (disposed || tab.closed || tab.sessionId !== sessionId) return
            if (event.kind === 'gap') {
              tab.writeBuf = []
              tab.pendingSize = null
              tab.term!.reset()
              tab.notice = `部分遠端輸出已無法補送（${event.bytesDropped} 位元組）`
            } else if (event.kind === 'bytes') {
              await tab.term!.write(Uint8Array.from(event.bytes), event.historical)
            } else if (event.kind === 'ttlWarning') {
              tab.notice = `遠端 Shell 將於 ${event.secondsRemaining} 秒後到期`
            } else if (event.ephemeralWorkspace) {
              tab.notice = '此遠端工作環境可能隨 runtime 停止而消失'
            }
          }
          if (disposed || tab.closed || tab.sessionId !== sessionId) return
          if (!transport.runtimeAck) throw new Error('此版本無法確認遠端輸出')
          if (out.runtime.batchId !== 0) await transport.runtimeAck(sessionId, out.runtime.batchId, out.runtime.localGeneration)
          const healthySince = runtimeRecoveryHealthySince.get(tab.key)
          if (out.runtime.batchId !== 0 && !out.runtime.ended && healthySince !== undefined
            && Date.now() - healthySince >= 30000
            && !out.runtime.events.some(e => e.kind === 'gap')
            && out.runtime.events.some(e => e.kind === 'bytes' && !e.historical && e.bytes.length > 0)) {
            runtimeRecoveryAttempts.delete(tab.key)
          }
        } catch (e) {
          if (!tab.closed && tab.sessionId === sessionId) {
            const conn = asConnectionError(e)
            if (tab.runtimeAgent && conn && RUNTIME_TERMINAL_KINDS.has(conn.kind)) {
              void retireRuntimeAgentTab(tab)
              return
            }
            if (conn && closeFinishedShell(tab, conn.kind)) return
            tab.status = 'error'
            tab.errorMsg = connErrText(e)
            tab.writeBuf = []
            tab.sessionId = null
            void transport.close(sessionId).catch(() => undefined)
          }
          return
        }
        if (out.runtime.inputUncertain) {
          tab.writeBuf = []
          tab.notice = '輸入是否送達尚未確認，未自動重送'
        }
        if (out.runtime.ended) {
          const reason = out.runtime.ended
          tab.writeBuf = []
          tab.pendingSize = null
          if (tab.runtimeAgent && RUNTIME_TERMINAL_KINDS.has(reason)) {
            // Agent binding ended permanently — no rebind to a new runtime.
            void retireRuntimeAgentTab(tab)
            return
          }
          if (closeFinishedShell(tab, reason)) return
          tab.status = 'error'
          tab.errorMsg = KIND_FALLBACK[reason] ?? '遠端 Shell 連線已結束'
          if (['transport', 'timeout', 'slowConsumer'].includes(reason) && (runtimeRecoveryAttempts.get(tab.key) ?? 0) < 3) {
            runtimeRecovery.set(tab.key, sessionId)
            if (!hidden) void recoverRuntime(tab)
          } else {
            runtimeRecovery.delete(tab.key)
            if (['transport', 'timeout', 'slowConsumer'].includes(reason) && (runtimeRecoveryAttempts.get(tab.key) ?? 0) >= 3) {
              tab.notice = KIND_FALLBACK.retryExhausted
            }
            tab.sessionId = null
            void transport.close(sessionId).catch(() => undefined)
          }
          return
        }
      }
      if (out.bytes.length) {
        const data = Uint8Array.from(out.bytes)
        try {
          // Await the xterm write callback BEFORE the next poll — this is the
          // pull-based backpressure required by the transport contract.
          await tab.term!.write(data)
        } catch {
          return
        }
        if (tab.closed || tab.sessionId !== sessionId) return
      }
      if (out.exitCode !== null) {
        if (options.closeFinishedTabs) {
          void closeTab(tab)
          return
        }
        tab.exitCode = out.exitCode
        tab.status = 'exited'
        tab.sessionId = null
        invalidateCapture(tab)
        void transport.close(sessionId).catch(() => undefined)
        resumeAfterBackground(tab)
        return
      }
      if (!hidden) foregroundResume.delete(tab.key)
      // Background tabs drain too — the loop is per-tab and independent of
      // visibility. Only idle polls pace themselves.
      if (!out.bytes.length && !out.runtime?.events.length && idlePollMs > 0) await sleep(idlePollMs)
    }
  }

  const replyQueues = new WeakMap<TabState, { sid: string; generation: number; bytes: number[]; writing: boolean }>()
  function sendTerminalReply(tab: TabState, text: string): void {
    if (disposed || tab.closed || tab.status !== 'live' || !tab.sessionId) return
    let queue = replyQueues.get(tab)
    if (!queue || queue.sid !== tab.sessionId || queue.generation !== tab.connectionGen) {
      queue = { sid: tab.sessionId, generation: tab.connectionGen, bytes: [], writing: false }
      replyQueues.set(tab, queue)
    }
    const bytes = Array.from(textEncoder.encode(text))
    if (queue.bytes.length + bytes.length > 16 * 1024) return
    queue.bytes.push(...bytes)
    if (queue.writing) return
    const owned = queue
    owned.writing = true
    void (async () => {
      try {
        while (owned.bytes.length && !tab.closed && !disposed && tab.sessionId === owned.sid && tab.connectionGen === owned.generation) {
          const chunk = owned.bytes.splice(0, WRITE_CHUNK)
          await transport.write(owned.sid, chunk, true)
        }
      } catch { owned.bytes = [] }
      finally { owned.bytes = []; owned.writing = false }
    })()
  }

  function sendInput(tab: TabState, text: string): void {
    // A modal password must never fall through to a previously focused terminal.
    if (hidden || tab.status !== 'live' || !tab.sessionId || tab.closed) return
    const bytes = Array.from(textEncoder.encode(text))
    if (!bytes.length) return
    if (tab.writeBuf.length + bytes.length > WRITE_CAP) {
      // Reject this whole input event; never truncate queued command prefixes
      // or retain only a pasted suffix that might contain an execution newline.
      tab.writeDropped = true
      tab.notice = '輸入佇列已滿，本次輸入未送出；請稍後重試'
      return
    } else {
      tab.writeBuf = tab.writeBuf.concat(bytes)
    }
    void flushWrite(tab)
  }

  function launchCodingCli(tab: TabState): void {
    if (!tab.codingCli || tab.status !== 'live' || hidden || tab.closed) return
    const command = codingCliInput(tab.codingCli)
    if (command === null) return
    sendInput(tab, command)
    tab.term?.focus()
  }

  async function flushWrite(tab: TabState): Promise<void> {
    if (tab.writing) return
    tab.writing = true
    const sid = tab.sessionId
    const generation = tab.connectionGen
    try {
      while (tab.writeBuf.length && tab.status === 'live' && sid && tab.sessionId === sid && tab.connectionGen === generation && !tab.closed) {
        const chunk = tab.writeBuf.splice(0, WRITE_CHUNK)
        try {
          await transport.write(sid, chunk)
        } catch (e) {
          if (tab.connectionGen === generation) {
            tab.errorMsg = errText(e)
            tab.writeBuf = []
          }
          break
        }
      }
    } finally {
      tab.writing = false
      if (!tab.writeBuf.length) tab.writeDropped = false
      // A new connection may receive fresh input while an old write settles.
      // Its own queue is safe to flush; detach already discarded old input.
      if (tab.connectionGen !== generation && tab.writeBuf.length && tab.status === 'live') {
        void flushWrite(tab)
      }
    }
  }

  function requestResize(tab: TabState, rows: number, cols: number): void {
    if (!rows || !cols || tab.closed) return
    if (tab.lastSize && tab.lastSize.rows === rows && tab.lastSize.cols === cols) return
    tab.lastSize = { rows, cols }
    if (tab.status !== 'live' || !tab.sessionId) return
    tab.pendingSize = { rows, cols }
    void flushResize(tab)
  }

  async function flushResize(tab: TabState): Promise<void> {
    if (tab.resizing) return
    tab.resizing = true
    const sid = tab.sessionId
    const generation = tab.connectionGen
    try {
      // Latest-wins coalescing keeps this bounded and serialized.
      while (tab.pendingSize && tab.status === 'live' && sid && tab.sessionId === sid && tab.connectionGen === generation && !tab.closed) {
        const { rows, cols } = tab.pendingSize
        tab.pendingSize = null
        try {
          await transport.resize(sid, rows, cols)
        } catch (e) {
          if (tab.connectionGen === generation) tab.errorMsg = errText(e)
          break
        }
      }
    } finally {
      tab.resizing = false
      if (tab.connectionGen !== generation && tab.pendingSize && tab.status === 'live') {
        void flushResize(tab)
      }
    }
  }

  function setActive(key: string): void {
    if (key === activeKey.value) return
    const prev = activeTab.value
    if (prev) invalidateCapture(prev)
    activeKey.value = key
    const next = activeTab.value
    if (next && canCapture(next)) requestCapture(next)
  }

  function stepTab(delta: number): void {
    if (!tabs.length) return
    const idx = tabs.findIndex((t) => t.key === activeKey.value)
    const next = tabs[(((idx < 0 ? 0 : idx) + delta) % tabs.length + tabs.length) % tabs.length]
    setActive(next.key)
  }

  function isSwitchKey(e: KeyboardEvent): boolean {
    if (e.type !== 'keydown') return false
    if (e.altKey && !e.ctrlKey && !e.metaKey &&
        (e.code === 'BracketLeft' || e.code === 'BracketRight' || e.key === '[' || e.key === ']')) {
      return true
    }
    if (e.ctrlKey && !e.altKey && !e.metaKey &&
        (e.code === 'PageUp' || e.code === 'PageDown' || e.key === 'PageUp' || e.key === 'PageDown')) {
      return true
    }
    return false
  }

  /** Global capture-phase keydown handler AND xterm key interceptor basis. */
  function handleKey(e: KeyboardEvent): boolean {
    if (!isSwitchKey(e)) return false
    e.preventDefault()
    e.stopPropagation()
    const back = e.code === 'BracketLeft' || e.code === 'PageUp' || e.key === '[' || e.key === 'PageUp'
    stepTab(back ? -1 : 1)
    return true
  }

  async function closeTab(tab: TabState): Promise<void> {
    foregroundResume.delete(tab.key)
    runtimeRecovery.delete(tab.key)
    runtimeRecoveryAttempts.delete(tab.key)
    runtimeRecoveryHealthySince.delete(tab.key)
    if (tab.closed) return
    instanceOperations.delete(tab.key)
    tab.closed = true
    tab.connectionGen++
    tab.writeBuf = []
    tab.pendingSize = null
    invalidateCapture(tab)
    if (capturePending === tab) capturePending = null
    const sid = tab.sessionId
    tab.sessionId = null
    tab.status = 'exited'
    // Local detach only — the backend owns bounded remote release and the
    // tmux session/desktop lease stay alive on their documented semantics.
    tab.term?.dispose()
    tab.term = null
    const idx = tabs.indexOf(tab)
    if (idx >= 0) tabs.splice(idx, 1)
    if (activeKey.value === tab.key) {
      const next = tabs[Math.min(idx, tabs.length - 1)]
      activeKey.value = next ? next.key : null
      if (next && canCapture(next)) requestCapture(next)
    }
    if (sid) await transport.close(sid).catch(() => undefined)
  }

  async function detachTab(tab: TabState): Promise<void> {
    foregroundResume.delete(tab.key)
    runtimeRecovery.delete(tab.key)
    runtimeRecoveryAttempts.delete(tab.key)
    runtimeRecoveryHealthySince.delete(tab.key)
    if (tab.closed) return
    tab.connectionGen++
    const sid = tab.sessionId
    tab.sessionId = null
    tab.status = 'exited'
    tab.writeBuf = []
    tab.pendingSize = null
    tab.desktopEnabled = false
    tab.notice = '本機已中斷；可重新連線續接遠端工作'
    invalidateCapture(tab)
    if (sid) await transport.close(sid).catch(() => undefined)
  }

  const foregroundResume = new Map<string, boolean>()
  const runtimeRecovery = new Map<string, string>()
  const runtimeRecovering = new Set<string>()
  const runtimeRecoveryAttempts = new Map<string, number>()
  const runtimeRecoveryHealthySince = new Map<string, number>()
  async function recoverRuntime(tab: TabState, explicit = false): Promise<void> {
    const sessionId = runtimeRecovery.get(tab.key)
    if (!sessionId || !transport.runtimeRecover || hidden || tab.closed || disposed || runtimeRecovering.has(tab.key)) return
    runtimeRecoveryHealthySince.delete(tab.key)
    if (explicit) runtimeRecoveryAttempts.set(tab.key, 0)
    const attempts = runtimeRecoveryAttempts.get(tab.key) ?? 0
    if (attempts >= 3) {
      runtimeRecovery.delete(tab.key)
      tab.notice = KIND_FALLBACK.retryExhausted
      tab.sessionId = null
      void transport.close(sessionId).catch(() => undefined)
      return
    }
    runtimeRecoveryAttempts.set(tab.key, attempts + 1)
    runtimeRecovering.add(tab.key)
    const generation = ++tab.connectionGen
    tab.writeBuf = []
    tab.pendingSize = null
    tab.status = 'connecting'
    try {
      await transport.runtimeRecover(sessionId)
      if (tab.closed || disposed || tab.connectionGen !== generation) {
        void transport.close(sessionId).catch(() => undefined)
        return
      }
      runtimeRecovery.delete(tab.key)
      foregroundResume.delete(tab.key)
      tab.status = 'live'
      tab.errorMsg = ''
      tab.sessionId = sessionId
      runtimeRecoveryHealthySince.set(tab.key, Date.now())
      if (tab.lastSize) await transport.resize(sessionId, tab.lastSize.rows, tab.lastSize.cols)
      void pollLoop(tab, sessionId)
      if (!hidden && tab.desktopEnabled) {
        try {
          await transport.desktop(sessionId, true)
          if (tab.sessionId === sessionId && canCapture(tab)) requestCapture(tab)
        } catch (e) {
          if (!tab.closed && tab.sessionId === sessionId) tab.desktopError = errText(e)
        }
      }
    } catch (e) {
      runtimeRecovery.delete(tab.key)
      if (!tab.closed && tab.connectionGen === generation) {
        const conn = asConnectionError(e)
        if (tab.runtimeAgent && conn && RUNTIME_TERMINAL_KINDS.has(conn.kind)) {
          // Renewal hit a permanently ended binding — retire, no rebind.
          void retireRuntimeAgentTab(tab)
        } else if (conn && closeFinishedShell(tab, conn.kind)) {
          return
        } else {
          tab.status = 'error'
          tab.errorMsg = connErrText(e)
          tab.sessionId = null
        }
      }
      void transport.close(sessionId).catch(() => undefined)
    } finally { runtimeRecovering.delete(tab.key) }
  }
  function resumeAfterBackground(tab: TabState): void {
    if (hidden || options.backgroundConnection !== 'keep' || !foregroundResume.has(tab.key)) return
    const desktop = foregroundResume.get(tab.key)!
    foregroundResume.delete(tab.key)
    if (runtimeRecovery.has(tab.key)) void recoverRuntime(tab)
    else reopenTab(tab, desktop)
  }
  async function setHidden(value: boolean): Promise<void> {
    hidden = value
    if (value) {
      if (options.backgroundConnection === 'keep') {
        for (const tab of tabs) {
          if (tab.status === 'live' || tab.status === 'connecting') foregroundResume.set(tab.key, tab.desktopEnabled || tab.wantDesktop)
          tab.writeBuf = []
          tab.pendingSize = null
          invalidateCapture(tab)
          tab.shot = null
        }
        await Promise.all(tabs.filter(t => t.sessionId && (t.desktopEnabled || t.desktopBusy)).map(t => transport.desktop(t.sessionId!, false).catch(() => undefined)))
      } else {
        await Promise.all(tabs.map((tab) => detachTab(tab)))
      }
    } else {
      if (options.backgroundConnection === 'keep') {
        for (const candidate of tabs) {
          if (candidate.status === 'error' || candidate.status === 'exited') resumeAfterBackground(candidate)
        }
      }
      await Promise.all(tabs.filter(t => t.sessionId && t.desktopEnabled && !runtimeRecovery.has(t.key) && !runtimeRecovering.has(t.key)).map(t => transport.desktop(t.sessionId!, true).catch(() => undefined)))
      const tab = activeTab.value
      if (tab && canCapture(tab)) requestCapture(tab)
    }
  }

  // --- desktop / screenshot scheduling ---

  let captureBusy = false
  let capturePending: TabState | null = null

  function canCapture(tab: TabState | null | undefined): tab is TabState {
    return (
      !!tab &&
      !disposed &&
      !hidden &&
      !tab.closed &&
      tab.status === 'live' &&
      !!tab.sessionId &&
      tab.desktopEnabled &&
      tab.previewVisible &&
      tab === activeTab.value
    )
  }

  function invalidateCapture(tab: TabState): void {
    tab.captureGen++
    if (capturePending === tab) capturePending = null
  }

  function instanceOperation(tab: TabState): number {
    const next = (instanceOperations.get(tab.key) ?? 0) + 1
    instanceOperations.set(tab.key, next)
    invalidateCapture(tab)
    tab.shot = null
    tab.shotError = ''
    return next
  }
  async function selectInstance(tab: TabState, desktop: InstanceDesktop): Promise<void> {
    if (!transport.instanceSelect || hidden || disposed || tab.closed || tab.status !== 'live' || !tab.sessionId) return
    const sid = tab.sessionId, operation = instanceOperation(tab)
    tab.desktopBusy = true; tab.desktopError = ''; tab.desktopEnabled = false
    tab.instanceDesktop = null; tab.instanceToolReady = false
    try {
      const result = await transport.instanceSelect(sid, desktop.id, desktop.revision)
      if (disposed || hidden || tab.closed || tab.sessionId !== sid || instanceOperations.get(tab.key) !== operation) return
      if (result.desktop.id !== desktop.id || result.desktop.revision !== desktop.revision) throw new Error('Screen 設定已變更，請重新選擇')
      tab.instanceDesktop = { ...result.desktop }; tab.instanceToolReady = result.toolReady
      tab.desktopBusy = false
      await setDesktop(tab, true)
      if (instanceOperations.get(tab.key) === operation) tab.desktopMsg = ''
    } catch {
      if (!tab.closed && tab.sessionId === sid && instanceOperations.get(tab.key) === operation) tab.desktopError = 'Screen 選擇失敗，請確認設定與連線後重試'
    } finally {
      if (instanceOperations.get(tab.key) === operation) tab.desktopBusy = false
    }
  }
  async function clearInstance(tab: TabState): Promise<void> {
    const sid = tab.sessionId, operation = instanceOperation(tab)
    tab.instanceDesktop = null; tab.instanceToolReady = false; tab.desktopEnabled = false
    tab.desktopMsg = ''; tab.desktopError = ''; tab.desktopBusy = true
    try { if (sid) await transport.instanceClear?.(sid) }
    catch { if (!tab.closed && tab.sessionId === sid && instanceOperations.get(tab.key) === operation) tab.desktopError = '預覽已停止；遠端工具取消尚未確認，請重試或結束 CLI' }
    finally { if (instanceOperations.get(tab.key) === operation) tab.desktopBusy = false }
  }

  /** Serialized toggles; a response landing on a closed/replaced tab is dropped. */
  async function setDesktop(tab: TabState, enabled: boolean): Promise<void> {
    if (hidden || tab.desktopBusy || tab.closed || tab.status !== 'live' || !tab.sessionId) return
    tab.desktopBusy = true
    tab.desktopError = ''
    const sid = tab.sessionId, captureVersion = tab.captureGen, operation = instanceOperations.get(tab.key)
    try {
      const st = await transport.desktop(sid, enabled)
      if (hidden || tab.closed || tab.sessionId !== sid || tab.captureGen !== captureVersion) return
      tab.desktopEnabled = st.enabled
      tab.desktopMsg = st.message
      if (st.enabled) {
        tab.previewVisible = true
        if (tab === activeTab.value) requestCapture(tab)
      } else {
        invalidateCapture(tab)
        // A disabled preview must not keep showing the last frame.
        tab.shot = null
        tab.shotError = ''
      }
    } catch (e) {
      if (!tab.closed && tab.sessionId === sid) tab.desktopError = errText(e)
    } finally {
      if (instanceOperations.get(tab.key) === operation) tab.desktopBusy = false
    }
  }

  function setPreviewVisible(tab: TabState, visible: boolean): void {
    tab.previewVisible = visible
    if (visible) {
      if (tab.sessionId && tab.desktopEnabled) void transport.desktop(tab.sessionId, true).then(() => { if (canCapture(tab)) requestCapture(tab) }).catch(() => undefined)
    } else {
      invalidateCapture(tab)
      tab.shot = null
      if (tab.sessionId && (tab.desktopEnabled || tab.desktopBusy)) void transport.desktop(tab.sessionId, false).catch(() => undefined)
    }
  }

  function requestCapture(tab: TabState): void {
    if (!canCapture(tab)) return
    if (captureBusy) {
      // One global in-flight capture; manual/auto requests coalesce latest-wins.
      capturePending = tab
      return
    }
    void runCapture(tab)
  }

  async function runCapture(tab: TabState): Promise<void> {
    captureBusy = true
    const sid = tab.sessionId!
    const gen = ++tab.captureGen
    try {
      const s = await transport.screenshot(sid)
      if (
        !tab.closed &&
        tab.sessionId === sid &&
        tab.captureGen === gen &&
        tab.desktopEnabled &&
        tab.previewVisible &&
        tab === activeTab.value &&
        (!tab.instanceDesktop || (s.desktopId === tab.instanceDesktop.id && s.desktopRevision === tab.instanceDesktop.revision)) &&
        SAFE_IMAGE_MIME.test(s.mime)
      ) {
        tab.shot = { src: `data:${s.mime};base64,${s.base64}`, source: s.source, capturedAt: s.capturedAt }
        tab.shotError = ''
      }
    } catch (e) {
      if (!tab.closed && tab.sessionId === sid && tab.captureGen === gen) {
        // Keep this Screen's last successful frame, but mark it stale.
        tab.shotError = errText(e)
      }
    } finally {
      captureBusy = false
      const pending = capturePending
      capturePending = null
      if (pending && canCapture(pending)) void runCapture(pending)
    }
  }

  function manualCapture(): void {
    const tab = activeTab.value
    if (tab && canCapture(tab)) requestCapture(tab)
  }

  // First capture is immediate (on enable / activate / show), then every
  // autoCaptureMs for the active tab only. Shell input is never blocked.
  const captureTimer = setInterval(() => {
    const tab = activeTab.value
    if (tab && canCapture(tab)) requestCapture(tab)
  }, autoCaptureMs)

  function dispose(): void {
    disposed = true
    catalogGen++
    instanceCatalogGen++
    instanceOperations.clear()
    catalogLoading.value = false
    clearInterval(captureTimer)
    capturePending = null
    for (const tab of tabs) {
      invalidateCapture(tab)
      tab.connectionGen++
      tab.writeBuf = []
      tab.pendingSize = null
      const sessionId = tab.sessionId
      tab.sessionId = null
      if (sessionId) void transport.close(sessionId).catch(() => undefined)
    }
    runtimeRecovery.clear()
    runtimeRecoveryAttempts.clear()
    runtimeRecoveryHealthySince.clear()
  }

  async function historyRequest(tab: TabState, request: HostHistoryRequest): Promise<HostHistoryReply> {
    if (!transport.runtimeHistory || !tab.runtime || tab.closed || !tab.sessionId || disposed) return { kind: 'unsupported' }
    const sid = tab.sessionId, generation = tab.connectionGen
    let result: HostHistoryReply
    try { result = await transport.runtimeHistory(sid, request) }
    catch (error) {
      if (asConnectionError(error)?.kind === 'busy') return { kind: 'error', error: 'busy' }
      throw error
    }
    if (tab.closed || disposed || tab.sessionId !== sid || tab.connectionGen !== generation) return { kind: 'unsupported' }
    return result
  }
  return {
    historyRequest,
    instanceDesktops, instanceDesktopsError, loadInstanceDesktops, selectInstance, clearInstance,
    catalogLoading,
    runtimeProfiles,
    runtimeProfilesError,
    tabs,
    activeKey,
    activeTab,
    uiNotice,
    selectionRequest,
    loadCatalog,
    refreshTargets,
    openTarget,
    attachTerminal,
    reopenTab,
    closeTab,
    detachTab,
    setHidden,
    setActive,
    stepTab,
    isSwitchKey,
    handleKey,
    sendInput,
    launchCodingCli,
    requestResize,
    setDesktop,
    setPreviewVisible,
    manualCapture,
    dispose
  }
}
