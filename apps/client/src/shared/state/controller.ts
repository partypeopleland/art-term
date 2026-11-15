import { computed, markRaw, reactive, ref } from 'vue'
import { asConnectionError } from '../transport/types'
import { hostErrorText } from '../transport/host-error'
import { connErrText, errText, KIND_FALLBACK, RUNTIME_TERMINAL_KINDS, sleep } from './controller-errors'
import { createPreviewController } from './controller-preview'
import { createTerminalIo } from './controller-terminal-io'
import type { AppController, ControllerOptions, HostUiState, TabState } from './controller-types'
export type { AppController, ControllerOptions, HostUiState, ShotState, TabState, TabStatus } from './controller-types'
import type {
  HostDto,
  InstanceDesktop,
  OutputDto,
  RuntimeHostDto,
  TargetDto,
  Transport
} from '../transport/types'
import type { TerminalHandle } from '../terminal/handle'
import type { HostHistoryReply, HostHistoryRequest } from '../terminal/native'
import {
  findPaneLeaf,
  paneConnTabs,
  paneLeaves,
  removePaneLeaf,
  replacePaneLeaf,
  splitPaneLeaf,
  type PaneLeaf,
  type PaneNode,
  type SplitDirection
} from '../terminal/split'

export const MAX_TABS = 32
export const MAX_RUNTIME_PROFILES = 64
export const AUTO_CAPTURE_MS = 30_000
const DEFAULT_IDLE_POLL_MS = 20
const textEncoder = new TextEncoder()

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

  function newTabState(init: {
    key: string
    targetId: string
    hostId: string
    hostLabel: string
    label: string
    desktopAvailable: boolean
    paneOf: string | null
    wantDesktop?: boolean
  }): TabState {
    return {
      key: init.key,
      targetId: init.targetId,
      hostId: init.hostId,
      hostLabel: init.hostLabel,
      label: init.label,
      status: 'connecting',
      errorMsg: '',
      notice: '',
      sessionId: null,
      remoteSession: '',
      runtime: false,
      // A runtime HostDto carries the profile id — an agent-bound
      // profile marks this tab for lifecycle-terminal retirement.
      runtimeAgent: runtimeProfiles.value.some(
        (p) => p.id === init.hostId && p.category === 'agent'
      ),
      exitCode: null,
      desktopAvailable: init.desktopAvailable,
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
      mirrorTerm: null,
      paneRoot: null,
      focusedLeafId: null,
      paneSeq: 0,
      paneOf: init.paneOf,
      wantDesktop: init.wantDesktop ?? false,
      closed: false,
      writeBuf: [],
      writing: false,
      writeDropped: false,
      pendingSize: null,
      resizing: false,
      lastSize: null,
      connectionGen: 0
    }
  }

  function openTarget(host: HostUiState, target: TargetDto, withDesktop: boolean): void {
    const existing = tabs.find(
      (t) => t.targetId === target.id && t.hostId === host.dto.id && !t.closed && !t.paneOf
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
      uiNotice.value = `已達 ${MAX_TABS} 連線上限（含分割窗格）`
      return
    }
    uiNotice.value = ''
    const tab = reactive<TabState>(newTabState({
      key: `${host.dto.id}::${target.id}`,
      targetId: target.id,
      hostId: host.dto.id,
      hostLabel: host.dto.label,
      label: target.label,
      desktopAvailable: target.desktopAvailable || host.dto.desktopAvailable,
      paneOf: null,
      wantDesktop: withDesktop
    }))
    tabs.push(tab)
    activeKey.value = tab.key
    // Session opens in attachTerminal once the pane mounted and fit().
  }

  function attachTerminal(tab: TabState, handle: TerminalHandle): void {
    if (tab.closed) {
      handle.dispose()
      return
    }
    // Split/unsplit reshapes remount the pane's TerminalPane while the tab's
    // connection stays alive. Carry the viewport over and release the stale
    // handle so repeated reshapes cannot leak xterm instances.
    const previous = tab.term
    tab.term = markRaw(handle)
    handle.onData((text, terminalReply) => terminalReply ? sendTerminalReply(tab, text) : sendInput(tab, text))
    handle.setKeyInterceptor((ev) => {
      if (isSwitchKey(ev)) return false
      // Pane shortcuts belong to the owning split view: a child tab's own
      // paneRoot stays null, so check the parent's tree as well.
      const owner = tab.paneOf ? tabs.find((t) => t.key === tab.paneOf) : null
      return !(isPaneKey(ev) && !!(tab.paneRoot || owner?.paneRoot))
    })
    if (previous && previous !== handle) {
      const snapshot = previous.screenSnapshot?.()
      previous.dispose()
      if (snapshot) void handle.write(textEncoder.encode(snapshot)).catch(() => undefined)
    }
    const want = tab.wantDesktop
    tab.wantDesktop = false
    if (tab.status === 'connecting') void openSession(tab, want)
  }

  function attachMirror(tab: TabState, handle: TerminalHandle): void {
    if (tab.closed) {
      handle.dispose()
      return
    }
    if (tab.mirrorTerm && tab.mirrorTerm !== handle) tab.mirrorTerm.dispose()
    tab.mirrorTerm = markRaw(handle)
    const snapshot = tab.term?.screenSnapshot?.()
    if (snapshot) void handle.write(textEncoder.encode(snapshot)).catch(() => undefined)
  }

  function detachMirror(tab: TabState, handle: TerminalHandle): void {
    if (tab.mirrorTerm === handle) tab.mirrorTerm = null
    if (!tab.closed) handle.dispose()
  }

  function primaryLeaf(tab: TabState): PaneLeaf {
    return { kind: 'conn', id: tab.key, tab }
  }

  function paneRootOf(tab: TabState): PaneNode {
    return tab.paneRoot ?? primaryLeaf(tab)
  }

  /** Collapse a rebuilt tree back to null when only the primary conn leaf remains. */
  function settlePaneRoot(tab: TabState, root: PaneNode | null): void {
    tab.paneRoot = root && !(root.kind === 'conn' && root.tab === tab) ? root : null
  }

  function addPaneLeaf(tab: TabState, leafId: string, direction: SplitDirection, added: PaneLeaf): boolean {
    if (tab.closed || disposed || (!tab.paneRoot && tab.status !== 'live')) return false
    if (!findPaneLeaf(paneRootOf(tab), leafId)) return false
    tab.paneSeq++
    const leaf: PaneLeaf = added.kind === 'empty' ? { ...added, id: `empty:${tab.paneSeq}` } : added
    tab.paneRoot = splitPaneLeaf(paneRootOf(tab), leafId, direction, leaf, `split:${tab.paneSeq}`)
    tab.focusedLeafId = leaf.id
    return true
  }

  function splitPane(tab: TabState, leafId: string, direction: SplitDirection): boolean {
    return addPaneLeaf(tab, leafId, direction, { kind: 'empty', id: '' })
  }

  function mirrorPane(tab: TabState, leafId: string, direction: SplitDirection): boolean {
    const root = paneRootOf(tab)
    const leaf = findPaneLeaf(root, leafId)
    if (!leaf || leaf.kind !== 'conn' || tab.closed || disposed) return false
    const mirrorId = `mirror:${leaf.tab.key}`
    if (findPaneLeaf(root, mirrorId)) return false
    return addPaneLeaf(tab, leafId, direction, { kind: 'mirror', id: mirrorId, of: leaf.tab })
  }

  function openPaneTarget(tab: TabState, leafId: string, host: HostDto, target: TargetDto): void {
    if (tab.closed || disposed || !tab.paneRoot) return
    if (findPaneLeaf(tab.paneRoot, leafId)?.kind !== 'empty') return
    if (hidden || tabs.length >= MAX_TABS) {
      uiNotice.value = hidden ? '' : `已達 ${MAX_TABS} 連線上限（含分割窗格）`
      return
    }
    tab.paneSeq++
    const child = reactive<TabState>(newTabState({
      key: `${tab.key}#${tab.paneSeq}`,
      targetId: target.id,
      hostId: host.id,
      hostLabel: host.label,
      label: target.label,
      desktopAvailable: target.desktopAvailable || host.desktopAvailable,
      paneOf: tab.key
    }))
    tabs.push(child)
    tab.paneRoot = replacePaneLeaf(tab.paneRoot, leafId, { kind: 'conn', id: child.key, tab: child })
    tab.focusedLeafId = child.key
    uiNotice.value = ''
    // Session opens in attachTerminal once the pane mounted and fit().
  }

  /**
   * End the owning tab's own connection while sibling conn panes keep
   * running. The first surviving conn leaf is promoted to own the
   * top-level tab slot — its TabState, session and pane tree move over —
   * remaining children reparent to it, and the departing connection
   * closes alone. With no conn survivor the whole split view closes.
   */
  function promotePaneSurvivor(tab: TabState): void {
    const root = tab.paneRoot
    const survivor = root
      ? paneConnTabs(root).find((t) => t !== tab && !t.closed && t.paneOf === tab.key)
      : undefined
    if (!root || !survivor) {
      void closeTab(tab)
      return
    }
    // The departing conn's mirror leaf dies with it.
    let next = removePaneLeaf(root, tab.key)
    if (next) next = removePaneLeaf(next, `mirror:${tab.key}`)
    if (!next) {
      void closeTab(tab)
      return
    }
    for (const child of tabs) if (child.paneOf === tab.key) child.paneOf = survivor.key
    survivor.paneOf = null
    survivor.paneRoot = next
    survivor.paneSeq = Math.max(survivor.paneSeq, tab.paneSeq)
    survivor.focusedLeafId = tab.focusedLeafId && findPaneLeaf(next, tab.focusedLeafId)
      ? tab.focusedLeafId
      : survivor.key
    tab.paneRoot = null
    tab.focusedLeafId = null
    settlePaneRoot(survivor, survivor.paneRoot)
    void closeTab(tab)
    if (activeKey.value !== survivor.key) setActive(survivor.key)
  }

  /** Remote-side end of a tab's own connection: promote a pane survivor or close the tab. */
  function finishTabConnection(tab: TabState): void {
    if (!tab.paneOf && tab.paneRoot) promotePaneSurvivor(tab)
    else void closeTab(tab)
  }

  function closePaneLeaf(tab: TabState, leafId: string): void {
    if (tab.closed || disposed) return
    if (!tab.paneRoot) {
      // Single-pane view: closing the only pane closes the tab's connection.
      if (leafId === tab.key) void closeTab(tab)
      return
    }
    const leaf = findPaneLeaf(tab.paneRoot, leafId)
    if (!leaf) return
    if (leaf.kind === 'conn') {
      if (leaf.tab === tab) promotePaneSurvivor(tab)
      else void closeTab(leaf.tab)
      return
    }
    settlePaneRoot(tab, removePaneLeaf(tab.paneRoot, leafId))
    if (tab.focusedLeafId === leafId) tab.focusedLeafId = tab.key
  }

  function unsplitPanes(tab: TabState): void {
    if (tab.closed || !tab.paneRoot) return
    for (const leaf of paneLeaves(tab.paneRoot)) {
      if (leaf.id === tab.key) continue
      if (leaf.kind === 'conn') void closeTab(leaf.tab)
      else tab.paneRoot = removePaneLeaf(tab.paneRoot!, leaf.id)
    }
    settlePaneRoot(tab, tab.paneRoot)
    tab.focusedLeafId = tab.key
  }

  function focusPane(tab: TabState, leafId: string | null): void {
    if (tab.closed || !tab.paneRoot || leafId === null) {
      if (!tab.closed) tab.focusedLeafId = leafId
      return
    }
    const leaf = findPaneLeaf(tab.paneRoot, leafId)
    if (leaf) tab.focusedLeafId = leafId
  }

  function cyclePaneFocus(tab: TabState, delta: number): void {
    if (tab.closed || !tab.paneRoot) return
    const order = paneConnTabs(tab.paneRoot)
    if (order.length < 2) return
    const idx = order.findIndex((t) => t.key === tab.focusedLeafId)
    const next = idx < 0
      ? order[delta > 0 ? 0 : order.length - 1]
      : order[(idx + delta + order.length) % order.length]
    tab.focusedLeafId = next.key
  }

  function focusedConnTab(tab: TabState): TabState {
    const root = tab.paneRoot
    if (!root) return tab
    const leaf = tab.focusedLeafId ? findPaneLeaf(root, tab.focusedLeafId) : null
    if (leaf?.kind === 'conn') return leaf.tab
    return paneConnTabs(root)[0] ?? tab
  }

  async function writeTerminal(tab: TabState, data: Uint8Array, historical = false): Promise<void> {
    const term = tab.term
    if (!term) return
    await term.write(data, historical)
    const mirror = tab.mirrorTerm
    if (mirror && mirror !== term) void mirror.write(data, historical).catch(() => undefined)
  }

  function resetTerminals(tab: TabState): void {
    tab.term?.reset()
    tab.mirrorTerm?.reset()
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
    finishTabConnection(tab)
    return true
  }

  function reopenTab(tab: TabState, withDesktop: boolean): void {
    if (tab.closed || !tab.term || hidden || disposed || tab.status === 'live' || tab.status === 'connecting') return
    tab.writeBuf = []
    tab.writeDropped = false
    tab.pendingSize = null
    if (runtimeRecovery.has(tab.key)) { void recoverRuntime(tab, true); return }
    resetTerminals(tab)
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
              resetTerminals(tab)
              tab.notice = `部分遠端輸出已無法補送（${event.bytesDropped} 位元組）`
            } else if (event.kind === 'bytes') {
              await writeTerminal(tab, Uint8Array.from(event.bytes), event.historical)
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
          await writeTerminal(tab, data)
        } catch {
          return
        }
        if (tab.closed || tab.sessionId !== sessionId) return
      }
      if (out.exitCode !== null) {
        if (options.closeFinishedTabs) {
          finishTabConnection(tab)
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

  function stepTab(delta: number): void {
    const top = tabs.filter((t) => !t.paneOf && !t.closed)
    if (!top.length) return
    const idx = top.findIndex((t) => t.key === activeKey.value)
    const next = top[(((idx < 0 ? 0 : idx) + delta) % top.length + top.length) % top.length]
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

  /** Pane focus/close shortcuts: Ctrl+Alt+Arrow cycles conn panes, Ctrl+Shift+W closes the focused one. */
  function isPaneKey(e: KeyboardEvent): boolean {
    if (e.type !== 'keydown' || e.metaKey) return false
    if (e.ctrlKey && e.altKey && !e.shiftKey &&
        (e.code === 'ArrowLeft' || e.code === 'ArrowRight' || e.code === 'ArrowUp' || e.code === 'ArrowDown')) {
      return true
    }
    return e.ctrlKey && e.shiftKey && !e.altKey && e.code === 'KeyW'
  }

  function closeFocusedPane(tab: TabState): void {
    const leaf = tab.focusedLeafId ? findPaneLeaf(tab.paneRoot, tab.focusedLeafId) : null
    if (!leaf) {
      uiNotice.value = '請先點選要關閉的窗格。'
      return
    }
    closePaneLeaf(tab, leaf.id)
  }

  /** Global capture-phase keydown handler AND xterm key interceptor basis. */
  function handleKey(e: KeyboardEvent): boolean {
    if (isSwitchKey(e)) {
      e.preventDefault()
      e.stopPropagation()
      const back = e.code === 'BracketLeft' || e.code === 'PageUp' || e.key === '[' || e.key === 'PageUp'
      stepTab(back ? -1 : 1)
      return true
    }
    const tab = activeTab.value
    if (!tab?.paneRoot || !isPaneKey(e)) return false
    e.preventDefault()
    e.stopPropagation()
    if (e.code === 'KeyW') {
      closeFocusedPane(tab)
      return true
    }
    const back = e.code === 'ArrowLeft' || e.code === 'ArrowUp'
    cyclePaneFocus(tab, back ? -1 : 1)
    return true
  }

  function setActive(key: string): void {
    if (key === activeKey.value) return
    const target = tabs.find((t) => t.key === key)
    if (!target || target.paneOf || target.closed) return
    const prev = activeTab.value
    if (prev) invalidateCapture(prev)
    activeKey.value = key
    const next = activeTab.value
    if (next && canCapture(next)) requestCapture(next)
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
    tab.mirrorTerm?.dispose()
    tab.mirrorTerm = null
    // Pane children are independent connections — closing the owning tab
    // closes each of them; their own paneOf hook then finds a closed parent.
    for (const child of tabs.filter((t) => t.paneOf === tab.key)) void closeTab(child)
    const idx = tabs.indexOf(tab)
    if (idx >= 0) tabs.splice(idx, 1)
    if (activeKey.value === tab.key) {
      const next = tabs.filter((t) => !t.paneOf)[Math.min(idx, tabs.filter((t) => !t.paneOf).length - 1)]
      activeKey.value = next ? next.key : null
      if (next && canCapture(next)) requestCapture(next)
    }
    if (tab.paneOf) {
      const parent = tabs.find((t) => t.key === tab.paneOf && !t.closed)
      if (parent?.paneRoot) {
        // Drop the child's conn leaf and any mirror leaf watching it so no
        // orphan pane keeps pointing at the closed connection.
        let root = removePaneLeaf(parent.paneRoot, tab.key)
        if (root) root = removePaneLeaf(root, `mirror:${tab.key}`)
        settlePaneRoot(parent, root)
        if (parent.focusedLeafId === tab.key || parent.focusedLeafId === `mirror:${tab.key}`) parent.focusedLeafId = parent.key
      }
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
    attachMirror,
    detachMirror,
    splitPane,
    mirrorPane,
    openPaneTarget,
    closePaneLeaf,
    unsplitPanes,
    focusPane,
    cyclePaneFocus,
    focusedConnTab,
    reopenTab,
    closeTab,
    detachTab,
    setHidden,
    setActive,
    stepTab,
    isSwitchKey,
    isPaneKey,
    handleKey,
    sendInput,
    requestResize,
    setDesktop,
    setPreviewVisible,
    manualCapture,
    dispose
  }
}
