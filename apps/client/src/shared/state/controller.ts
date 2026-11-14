import { computed, markRaw, reactive, ref } from 'vue'
import { asConnectionError } from '../transport/types'
import { hostErrorText } from '../transport/host-error'
import { connErrText, errText, KIND_FALLBACK, RUNTIME_TERMINAL_KINDS, sleep } from './controller-errors'
import { createPreviewController } from './controller-preview'
import { createTerminalIo } from './controller-terminal-io'
import type { AppController, ControllerOptions, HostUiState, TabState } from './controller-types'
export type { AppController, ControllerOptions, HostUiState, ShotState, TabState, TabStatus } from './controller-types'
import type {
  InstanceDesktop,
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
const DEFAULT_IDLE_POLL_MS = 20

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
    if (transport.instanceRestore) void restoreInstance(tab, session.id, generation)
  }

  async function restoreInstance(tab: TabState, sid: string, connectionGeneration: number): Promise<void> {
    const operation = instanceOperations.get(tab.key)
    try {
      const result = await transport.instanceRestore?.(sid)
      if (!result || disposed || tab.closed || tab.sessionId !== sid || tab.connectionGen !== connectionGeneration || instanceOperations.get(tab.key) !== operation) return
      tab.instanceDesktop = { ...result.desktop }
      tab.instanceToolReady = result.toolReady
      tab.desktopEnabled = true
      tab.previewVisible = true
      invalidateCapture(tab)
      if (canCapture(tab)) requestCapture(tab)
    } catch (error) {
      if (!disposed && !tab.closed && tab.sessionId === sid && tab.connectionGen === connectionGeneration && instanceOperations.get(tab.key) === operation) {
        const message = errText(error)
        tab.desktopError = message.startsWith('無法恢復 Shell 的既有 Screen（')
          || message === '無法恢復 Shell 的既有 Screen；指派已保留，請重新選擇 Screen'
          ? message : '無法核對 Shell 的既有 Screen；請檢查連線後重試'
      }
    }
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

  const { sendTerminalReply, sendInput, requestResize } = createTerminalIo(
    transport, () => disposed, () => hidden
  )

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

  const {
    canCapture, invalidateCapture, requestCapture, selectInstance, clearInstance,
    setDesktop, setPreviewVisible, manualCapture, disposePreview
  } = createPreviewController(
    transport, activeTab, instanceOperations, autoCaptureMs,
    () => disposed, () => hidden
  )

  function dispose(): void {
    disposed = true
    catalogGen++
    instanceCatalogGen++
    instanceOperations.clear()
    catalogLoading.value = false
    disposePreview()
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
    requestResize,
    setDesktop,
    setPreviewVisible,
    manualCapture,
    dispose
  }
}
