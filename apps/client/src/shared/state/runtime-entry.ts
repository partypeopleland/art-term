// Shared runtime-Computer entry behaviour (Windows + Android): one explicit
// primary click may reuse an already-live local runtime tab or run ONE
// bounded session list and, only for the exact known quick state — exactly
// one target, running, phase 'ready' — connect it directly. Every other
// outcome (zero/multiple/non-running/attached/unknown phase, list failure)
// opens the existing chooser; nothing ever creates, retries, infers ids,
// or takes over implicitly.
import { ref, watch } from 'vue'
import type { Ref, WatchStopHandle } from 'vue'
import type { HostDto, RuntimeHostDto, TargetDto, Transport } from '../transport/types'
import type { AppController, TabState } from './controller'

/**
 * Backend phase contract (gui_runtime_sessions): issued targets carry
 * exactly 'ready' (unattached) or 'attached'; running is always true.
 * Only 'ready' may quick-connect — 'attached' and any unknown phase keep
 * the explicit chooser so takeover always stays a deliberate click.
 */
const QUICK_PHASE = 'ready'

/** Fixed safe text; raw backend errors never reach the list surface. */
export const RUNTIME_ENTRY_LIST_ERROR = '無法列出 Shell，請確認連線與 runtime 版本。'

/** A tab counts as a live runtime connection ONLY under the full contract. */
export function isLiveRuntimeTab(tab: TabState, hostId: string): boolean {
  return (
    tab.runtime === true &&
    tab.hostId === hostId &&
    tab.status === 'live' &&
    !tab.closed &&
    tab.sessionId !== null
  )
}

export function findLiveRuntimeTab(tabs: readonly TabState[], hostId: string): TabState | undefined {
  return tabs.find((tab) => isLiveRuntimeTab(tab, hostId))
}

/**
 * Local connected-state only: profile ids with at least one live runtime
 * tab. No heartbeat, no remote probe, and saved metadata never lights it;
 * connecting/error/exited/detach/close extinguish it reactively.
 */
export function runtimeConnectedIds(tabs: readonly TabState[]): Set<string> {
  const ids = new Set<string>()
  for (const tab of tabs) {
    if (isLiveRuntimeTab(tab, tab.hostId)) ids.add(tab.hostId)
  }
  return ids
}

export interface RuntimeEntryActions {
  /** An already-live local runtime tab for this exact host id exists. */
  activateTab(tab: TabState): void
  /** Open the chooser for the frozen profile; seed is the list just fetched. */
  openChooser(profile: RuntimeHostDto, seed: HostDto | null): void
  /** Direct-connect a quick-resolved backend group + opaque target. */
  openTarget(group: HostDto, target: TargetDto): void
  /** Fixed safe text channel for a failed list (e.g. controller.uiNotice). */
  showError(message: string): void
}

export interface RuntimeEntry {
  /** Profile id whose quick request is in flight, else null. */
  pendingId: Readonly<Ref<string | null>>
  /** Primary click: live-tab reuse → bounded quick list → chooser. */
  primary(profile: RuntimeHostDto): void
  /** Right Shell button: cancels any quick request, always opens the chooser. */
  shell(profile: RuntimeHostDto): void
  /** Retire any in-flight request without touching existing tabs. */
  cancel(): void
  dispose(): void
}

export function createRuntimeEntry(
  transport: Transport,
  controller: AppController,
  actions: RuntimeEntryActions
): RuntimeEntry {
  const pendingId = ref<string | null>(null)
  let pendingSnapshot: RuntimeHostDto | null = null
  let generation = 0
  let alive = true

  function hidden(): boolean {
    return typeof document !== 'undefined' && document.visibilityState === 'hidden'
  }

  function snapshotStillValid(snapshot: RuntimeHostDto): boolean {
    // A catalog reload that deleted the profile or bumped its revision
    // retires the in-flight quick result.
    return controller.runtimeProfiles.value.some(
      (p) => p.id === snapshot.id && p.revision === snapshot.revision
    )
  }

  function current(request: number, snapshot: RuntimeHostDto): boolean {
    return alive && request === generation && !hidden() && snapshotStillValid(snapshot)
  }

  function cancel(): void {
    generation++
    pendingId.value = null
    pendingSnapshot = null
  }

  function onVisibility(): void {
    if (hidden()) cancel()
  }

  const stopProfiles: WatchStopHandle = watch(controller.runtimeProfiles, () => {
    const snapshot = pendingSnapshot
    if (snapshot && !snapshotStillValid(snapshot)) cancel()
  })

  async function primary(profile: RuntimeHostDto): Promise<void> {
    const snapshot = { ...profile }
    if (!alive || hidden() || !snapshotStillValid(snapshot)) return
    // Local reuse first: an already-live runtime tab for the SAME host id —
    // never name matching, never a remote query, never a takeover.
    const existing = findLiveRuntimeTab(controller.tabs, snapshot.id)
    if (existing) {
      cancel()
      actions.activateTab(existing)
      return
    }
    // One bounded request in flight: same-profile double clicks are dropped,
    // selecting a DIFFERENT profile supersedes the older request.
    if (pendingSnapshot && pendingSnapshot.id !== snapshot.id) cancel()
    if (pendingSnapshot) return
    if (typeof transport.runtimeSessions !== 'function') {
      actions.openChooser(snapshot, null)
      return
    }
    const request = ++generation
    pendingId.value = snapshot.id
    pendingSnapshot = snapshot
    let group: HostDto
    try {
      group = await transport.runtimeSessions(snapshot.id, snapshot.revision)
    } catch {
      if (current(request, snapshot)) actions.showError(RUNTIME_ENTRY_LIST_ERROR)
      return
    } finally {
      if (request === generation) {
        pendingId.value = null
        pendingSnapshot = null
      }
    }
    if (!current(request, snapshot)) return
    const targets = group.id === snapshot.id ? group.targets : []
    if (
      targets.length === 1 &&
      targets[0].running &&
      targets[0].phase === QUICK_PHASE
    ) {
      // Actual backend-issued group + opaque target, unchanged.
      actions.openTarget(group, targets[0])
      return
    }
    actions.openChooser(snapshot, group.id === snapshot.id ? group : null)
  }

  function shell(profile: RuntimeHostDto): void {
    const snapshot = { ...profile }
    if (!alive || hidden() || !snapshotStillValid(snapshot)) return
    cancel()
    actions.openChooser(snapshot, null)
  }

  function dispose(): void {
    alive = false
    cancel()
    stopProfiles()
    if (typeof document !== 'undefined') {
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }

  if (typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', onVisibility)
  }

  return { pendingId, primary: (p) => void primary(p), shell, cancel, dispose }
}
